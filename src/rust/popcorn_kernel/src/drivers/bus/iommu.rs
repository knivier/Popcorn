//! AMD IOMMU diagnostics (read-only): is the IOMMU on, what does the device
//! table do to a device's DMA, and what blocked accesses has it logged.
//! `iommu` prints the report; the Wi-Fi driver includes it when a DMA step
//! fails. Nothing here writes to the IOMMU.

use alloc::string::String;
use core::fmt::Write;

use super::iommu_regs::{self as regs, DmaPolicy};
use super::pci::{self, cfg_read32};
use crate::drivers::grant::{self, MmioRegion};

/// Direct map of physical memory (64 GiB): the device table and event log are
/// firmware-owned RAM, read through it.
const DIRECT_MAP: u64 = 0xFFFF_8000_0000_0000;
const DIRECT_BYTES: u64 = 64 << 30;

fn ram_u64(phys: u64) -> Option<u64> {
    if phys % 8 != 0 || phys.checked_add(8)? > DIRECT_BYTES {
        return None;
    }
    Some(unsafe { core::ptr::read_volatile((DIRECT_MAP + phys) as *const u64) })
}

fn ram_u32(phys: u64) -> Option<u32> {
    if phys % 4 != 0 || phys.checked_add(4)? > DIRECT_BYTES {
        return None;
    }
    Some(unsafe { core::ptr::read_volatile((DIRECT_MAP + phys) as *const u32) })
}

struct Iommu {
    at: pci::PciAddr,
    mmio: MmioRegion,
}

fn find() -> Result<Iommu, &'static str> {
    let at = pci::find_class(0x08, 0x06, 0xFF).ok_or("no AMD IOMMU on the PCI bus")?;
    let mut cap = None;
    pci::for_each_cap(at, |id, off| {
        if id == regs::CAP_ID && cap.is_none() {
            cap = Some(off);
        }
    });
    let cap = cap.ok_or("IOMMU function has no IOMMU capability")?;
    let lo = cfg_read32(at.bus, at.slot, at.func, cap + 4);
    let hi = cfg_read32(at.bus, at.slot, at.func, cap + 8);
    let base = regs::mmio_base(lo, hi).ok_or("IOMMU MMIO base not enabled")?;
    let mmio = MmioRegion::map(base, regs::MMIO_BYTES, grant::new_grant_id()).map_err(|e| e.as_str())?;
    Ok(Iommu { at, mmio })
}

fn r64(m: &MmioRegion, off: usize) -> u64 {
    m.r64_split(off)
}

fn policy_text(p: DmaPolicy) -> String {
    let mut s = String::new();
    let _ = match p {
        DmaPolicy::NotValid => write!(s, "entry not valid (IOMMU ignores the device: DMA untranslated)"),
        DmaPolicy::NoTranslationInfo => write!(s, "valid, no translation info"),
        DmaPolicy::Passthrough { read, write } => {
            write!(s, "passthrough, read {} write {}", if read { "allowed" } else { "BLOCKED" }, if write { "allowed" } else { "BLOCKED" })
        }
        DmaPolicy::Translated { mode, root, read, write } => write!(
            s,
            "TRANSLATED (mode {}, page tables at {:#x}, IR {} IW {}): only pages firmware mapped are reachable",
            mode, root, read as u8, write as u8
        ),
        DmaPolicy::Reserved => write!(s, "reserved mode"),
    };
    s
}

/// Report lines through `say`. `focus` = devices to show the device-table entry of.
pub fn report(focus: &[(pci::PciAddr, &str)], say: &mut dyn FnMut(core::fmt::Arguments)) {
    let iommu = match find() {
        Ok(i) => i,
        Err(why) => {
            say(format_args!("iommu: {}", why));
            return;
        }
    };
    let m = &iommu.mmio;
    let ctrl = r64(m, regs::CONTROL);
    let status = r64(m, regs::STATUS);
    let (dt, dt_bytes) = regs::dev_table(r64(m, regs::DEV_TABLE_BASE));
    say(format_args!(
        "iommu: AMD IOMMU at {}:{}.{}, MMIO {:#x}: {} (control {:#x}, status {:#x}), device table {:#x} ({} entries)",
        iommu.at.bus,
        iommu.at.slot,
        iommu.at.func,
        m.phys(),
        if ctrl & regs::CTRL_IOMMU_EN != 0 { "ENABLED" } else { "disabled" },
        ctrl,
        status,
        dt,
        dt_bytes / regs::DTE_BYTES
    ));
    if ctrl & regs::CTRL_IOMMU_EN == 0 {
        say(format_args!("iommu: off, so it cannot block device DMA"));
    }
    for (a, name) in focus {
        let id = regs::device_id(a.bus, a.slot, a.func);
        let off = id as usize * regs::DTE_BYTES;
        if dt == 0 || off + regs::DTE_BYTES > dt_bytes {
            say(format_args!("iommu: {} ({}:{}.{}, id {:#06x}): outside the device table", name, a.bus, a.slot, a.func, id));
            continue;
        }
        let q = [ram_u64(dt + off as u64), ram_u64(dt + off as u64 + 8)];
        match q {
            [Some(q0), Some(q1)] => say(format_args!(
                "iommu: {} ({}:{}.{}, id {:#06x}): DTE {:#018x} {:#018x}: {}",
                name,
                a.bus,
                a.slot,
                a.func,
                id,
                q0,
                q1,
                policy_text(regs::dte_policy(q0))
            )),
            _ => say(format_args!("iommu: {}: device table not readable", name)),
        }
    }
    // Event log: newest first, at most 12 entries.
    let el_reg = r64(m, regs::EVENT_LOG_BASE);
    let (el, entries) = regs::event_log(el_reg);
    let head = (r64(m, regs::EVENT_HEAD) & 0x7FFF0) as usize / regs::EVENT_BYTES;
    let tail = (r64(m, regs::EVENT_TAIL) & 0x7FFF0) as usize / regs::EVENT_BYTES;
    if el == 0 || ctrl & regs::CTRL_EVENT_LOG_EN == 0 {
        say(format_args!("iommu: event log not enabled"));
        return;
    }
    let pending = (tail + entries - head) % entries;
    say(format_args!("iommu: event log {:#x}, {} entries, {} unread", el, entries, pending));
    let show = pending.min(12);
    for k in 0..show {
        let idx = (tail + entries - 1 - k) % entries;
        let p = el + (idx * regs::EVENT_BYTES) as u64;
        let d = [ram_u32(p), ram_u32(p + 4), ram_u32(p + 8), ram_u32(p + 12)];
        let [Some(a), Some(b), Some(c), Some(e)] = d else { continue };
        let ev = regs::decode_event([a, b, c, e]);
        say(format_args!(
            "iommu: event {}: {} device {:02x}:{:02x}.{} address {:#x} flags {:#x}",
            k,
            regs::event_name(ev.code),
            ev.device >> 8,
            (ev.device >> 3) & 0x1F,
            ev.device & 7,
            ev.address,
            ev.flags
        ));
    }
    if pending > show {
        say(format_args!("iommu: {} older event(s) not shown", pending - show));
    }
}

/// Every USB controller (class 0C03) plus `extra`, for the report.
pub fn interesting_devices(extra: &[(pci::PciAddr, &'static str)]) -> alloc::vec::Vec<(pci::PciAddr, &'static str)> {
    let mut v: alloc::vec::Vec<(pci::PciAddr, &'static str)> = extra.to_vec();
    pci::for_each_function(|a, _vendor, _device, class_reg| {
        if (class_reg >> 16) == 0x0C03 {
            v.push((a, "usb controller"));
        }
    });
    v
}

/// `iommu` shell command.
#[no_mangle]
pub extern "C" fn rust_iommu_cmd() {
    let mut extra = alloc::vec::Vec::new();
    pci::find_all(&[(0x17CB, 0x1103)], |a, _| extra.push((a, "wifi")));
    let devs = interesting_devices(&extra);
    report(&devs, &mut |args| {
        let mut s = String::new();
        let _ = s.write_fmt(args);
        crate::console_ffi::println_color(&s, crate::console_ffi::COLOR_WHITE);
    });
}
