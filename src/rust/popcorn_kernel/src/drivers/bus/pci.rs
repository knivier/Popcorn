//! PCI config space — multi-bus scan (bridge recursion), BAR read, virtio find.
//!
//! ThinkPads put xHCI/NVMe behind PCIe root ports (secondary buses). Bus-0-only
//! scans miss them; we walk type-1 bridges so real hardware can see USB/NVMe.

use crate::console_ffi::{print_color, println_color, u64_to_dec, COLOR_LIGHT_CYAN, COLOR_WHITE};
use crate::drivers::grant::port::raw::{inl, outl};
use crate::drivers::grant::{GrantError, GrantId, MmioRegion, PortRange};
use crate::sync::IrqSpinLock;

/// CF8/CFC is a two-step access: an ISR or a preempting task between the address
/// write and the data access would read/write the wrong register. One lock
/// serialises every config access (held for one dword, or one read-modify-write).
static CFG_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

const CONFIG_ADDR: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

pub const VENDOR_VIRTIO: u16 = 0x1AF4;
pub const DEV_VIRTIO_BLK_LEGACY: u16 = 0x1001;
pub const DEV_VIRTIO_BLK_MODERN: u16 = 0x1042;

#[inline]
fn cfg_addr(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    (1u32 << 31) | ((bus as u32) << 16) | ((slot as u32) << 11) | ((func as u32) << 8) | ((offset as u32) & 0xFC)
}

pub fn cfg_read32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let _g = CFG_LOCK.lock();
    unsafe {
        outl(CONFIG_ADDR, cfg_addr(bus, slot, func, offset));
        inl(CONFIG_DATA)
    }
}

pub fn cfg_write32(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let _g = CFG_LOCK.lock();
    unsafe {
        outl(CONFIG_ADDR, cfg_addr(bus, slot, func, offset));
        outl(CONFIG_DATA, value);
    }
}

/// Atomic read-modify-write of one config dword.
pub(in crate::drivers) fn cfg_rmw32(bus: u8, slot: u8, func: u8, offset: u8, f: impl FnOnce(u32) -> u32) {
    let _g = CFG_LOCK.lock();
    unsafe {
        outl(CONFIG_ADDR, cfg_addr(bus, slot, func, offset));
        let v = inl(CONFIG_DATA);
        outl(CONFIG_ADDR, cfg_addr(bus, slot, func, offset));
        outl(CONFIG_DATA, f(v));
    }
}

pub fn cfg_read16(bus: u8, slot: u8, func: u8, offset: u8) -> u16 {
    let v = cfg_read32(bus, slot, func, offset & !3);
    if (offset & 2) != 0 {
        (v >> 16) as u16
    } else {
        v as u16
    }
}

/// 16-bit write via a 32-bit read-modify-write of the containing dword.
///
/// The dword at 0x04 is Command (low) + Status (high). Status error bits are
/// write-1-to-clear, so writing back what we read would wipe latched errors.
/// For Command writes we zero the Status half (0 is a no-op for RW1C / RO bits);
/// for Status writes the caller's value is the intended clear mask.
pub fn cfg_write16(bus: u8, slot: u8, func: u8, offset: u8, value: u16) {
    let aligned = offset & !3;
    cfg_rmw32(bus, slot, func, aligned, |v| {
        if aligned == 0x04 {
            if (offset & 2) != 0 {
                (v & 0x0000_FFFF) | ((value as u32) << 16)
            } else {
                value as u32
            }
        } else if (offset & 2) != 0 {
            (v & 0x0000_FFFF) | ((value as u32) << 16)
        } else {
            (v & 0xFFFF_0000) | (value as u32)
        }
    });
}

pub fn cfg_read8(bus: u8, slot: u8, func: u8, offset: u8) -> u8 {
    (cfg_read32(bus, slot, func, offset & !3) >> ((offset & 3) * 8)) as u8
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PciAddr {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
}

/// Walk every present function on `bus`, then recurse into PCI-PCI bridges.
/// `visited` is a 256-bit bus bitmap (4×u64) to stop cycles.
fn scan_bus(bus: u8, visited: &mut [u64; 4], f: &mut impl FnMut(PciAddr, u32, u32) -> bool) -> bool {
    let vi = (bus >> 6) as usize;
    let bit = 1u64 << (bus & 63);
    if visited[vi] & bit != 0 {
        return true;
    }
    visited[vi] |= bit;

    for slot in 0u8..32 {
        let mut multi = false;
        for func in 0u8..8 {
            let id = cfg_read32(bus, slot, func, 0);
            /* 0xFFFF: nothing there. 0x0000 is never a valid vendor either; the
             * T14s Gen 4 (AMD Phoenix) answers it for unused functions 0:3.2-3.4. */
            let vendor = id & 0xFFFF;
            if vendor == 0xFFFF || vendor == 0 {
                if func == 0 {
                    break;
                }
                continue;
            }
            if func == 0 {
                multi = (cfg_read32(bus, slot, 0, 0x0C) >> 16) & 0x80 != 0;
            }
            let class_reg = cfg_read32(bus, slot, func, 0x08);
            let addr = PciAddr { bus, slot, func };
            if !f(addr, id, class_reg) {
                return false;
            }
            /* Type-1 PCI-PCI bridge → walk secondary bus. */
            let class = (class_reg >> 24) as u8;
            let subclass = (class_reg >> 16) as u8;
            if class == 0x06 && subclass == 0x04 {
                let buses = cfg_read32(bus, slot, func, 0x18);
                let secondary = ((buses >> 8) & 0xFF) as u8;
                if secondary != 0 && secondary != bus {
                    if !scan_bus(secondary, visited, f) {
                        return false;
                    }
                }
            }
            if func == 0 && !multi {
                break;
            }
        }
    }
    true
}

/// Walk all reachable PCI buses (bus 0 + bridges). `f` returns `false` to stop.
fn scan(mut f: impl FnMut(PciAddr, u32, u32) -> bool) {
    let mut visited = [0u64; 4];
    let _ = scan_bus(0, &mut visited, &mut f);
}

/// I/O BAR base (legacy virtio), or `None` if the BAR is not I/O space.
pub fn bar_io(bus: u8, slot: u8, func: u8, bar: u8) -> Option<u16> {
    if bar > 5 {
        return None;
    }
    let off = 0x10 + bar * 4;
    let lo = cfg_read32(bus, slot, func, off);
    if (lo & 1) == 0 {
        return None;
    }
    Some((lo & !0x3) as u16)
}

/// I/O BAR (base, size). Size probe with decode off: write all-ones, read back,
/// `size = (!(v & !3) + 1) & 0xFFFF`, restore.
pub fn bar_io_sized(bus: u8, slot: u8, func: u8, bar: u8) -> Option<(u16, u16)> {
    if bar > 5 {
        return None;
    }
    let off = 0x10 + bar * 4;
    let lo = cfg_read32(bus, slot, func, off);
    if (lo & 1) == 0 {
        return None;
    }
    let cmd = cfg_read16(bus, slot, func, 0x04);
    cfg_write16(bus, slot, func, 0x04, cmd & !0x3);
    cfg_write32(bus, slot, func, off, 0xFFFF_FFFF);
    let probe = cfg_read32(bus, slot, func, off);
    cfg_write32(bus, slot, func, off, lo);
    cfg_write16(bus, slot, func, 0x04, cmd);
    let size = ((!(probe & !0x3)).wrapping_add(1) & 0xFFFF) as u16;
    if size == 0 {
        return None;
    }
    Some(((lo & !0x3) as u16, size))
}

/// Memory BAR (phys base, size). Uses size probe (write all-1s) with I/O + memory
/// decode switched off meanwhile, so the device never decodes a bogus address.
pub fn bar_mem(bus: u8, slot: u8, func: u8, bar: u8) -> Option<(u64, u64)> {
    if bar > 5 {
        return None;
    }
    let off = 0x10 + bar * 4;
    let lo = cfg_read32(bus, slot, func, off);
    if (lo & 1) != 0 {
        return None; /* I/O */
    }
    let mem_type = (lo >> 1) & 3;
    if mem_type == 2 && bar == 5 {
        return None; /* 64-bit BAR would need the upper half past BAR5 */
    }
    let mut base = (lo & !0xF) as u64;
    let hi = if mem_type == 2 {
        let hi = cfg_read32(bus, slot, func, off + 4);
        base |= (hi as u64) << 32;
        hi
    } else {
        0
    };
    if base == 0 {
        return None;
    }

    let cmd = cfg_read16(bus, slot, func, 0x04);
    cfg_write16(bus, slot, func, 0x04, cmd & !0x3);

    cfg_write32(bus, slot, func, off, 0xFFFF_FFFF);
    let size_lo = cfg_read32(bus, slot, func, off);
    cfg_write32(bus, slot, func, off, lo);
    let mut size_bits = (size_lo & !0xF) as u64;
    if mem_type == 2 {
        cfg_write32(bus, slot, func, off + 4, 0xFFFF_FFFF);
        let size_hi = cfg_read32(bus, slot, func, off + 4);
        cfg_write32(bus, slot, func, off + 4, hi);
        size_bits |= (size_hi as u64) << 32;
    }

    cfg_write16(bus, slot, func, 0x04, cmd);

    let size = if mem_type == 2 {
        (!size_bits).wrapping_add(1)
    } else {
        (!(size_bits as u32)).wrapping_add(1) as u64
    };
    if size == 0 {
        return None;
    }
    Some((base, size))
}

/// Find first PCI device by class/subclass/prog-if (0xFF = wild).
pub fn find_class(class: u8, subclass: u8, prog_if: u8) -> Option<PciAddr> {
    let mut found = None;
    scan(|addr, _id, class_reg| {
        let c = (class_reg >> 24) as u8;
        let s = (class_reg >> 16) as u8;
        let p = (class_reg >> 8) as u8;
        if c == class && s == subclass && (prog_if == 0xFF || p == prog_if) {
            found = Some(addr);
            return false;
        }
        true
    });
    found
}

/// Enable I/O + memory + bus master.
pub fn enable_bus_master(bus: u8, slot: u8, func: u8) {
    let mut cmd = cfg_read16(bus, slot, func, 0x04);
    cmd |= 0x0007; // IO | MEM | BUS_MASTER
    cfg_write16(bus, slot, func, 0x04, cmd);
}

/// Any virtio-blk function (transitional 0x1001 or modern-only 0x1042).
fn is_virtio_blk(id: u32) -> bool {
    let v = (id & 0xFFFF) as u16;
    let d = ((id >> 16) & 0xFFFF) as u16;
    v == VENDOR_VIRTIO && (d == DEV_VIRTIO_BLK_LEGACY || d == DEV_VIRTIO_BLK_MODERN)
}

/// Transitional virtio-blk only (0x1001): the one ID with a legacy I/O BAR0.
/// Modern-only 0x1042 needs the capability/MMIO transport, which we lack.
fn is_virtio_blk_legacy(id: u32) -> bool {
    let v = (id & 0xFFFF) as u16;
    let d = ((id >> 16) & 0xFFFF) as u16;
    v == VENDOR_VIRTIO && d == DEV_VIRTIO_BLK_LEGACY
}

/// Nth transitional virtio-blk on any reachable PCI bus (bus 0 + bridges;
/// 0 = first). Modern-only devices are not counted: we drive the legacy
/// I/O-BAR path only, so they would fail to probe and shift the index.
pub fn find_virtio_blk_n(n: usize) -> Option<PciAddr> {
    let mut seen = 0usize;
    let mut found = None;
    scan(|addr, id, _class_reg| {
        if is_virtio_blk_legacy(id) {
            if seen == n {
                found = Some(addr);
                return false;
            }
            seen += 1;
        }
        true
    });
    found
}

/// Mass-storage class codes for internal-disk enumeration (listing only; no I/O).
pub fn foreach_storage_controller(mut f: impl FnMut(PciAddr, u8, u8)) {
    scan(|addr, id, class_reg| {
        /* virtio-blk (legacy or modern) is never an internal disk controller. */
        if is_virtio_blk(id) {
            return true;
        }
        let class_code = (class_reg >> 24) as u8;
        let subclass = (class_reg >> 16) as u8;
        if class_code == 0x01 {
            /* IDE / ATA / SATA / NVMe / etc. */
            f(addr, class_code, subclass);
        }
        true
    });
}

fn print_hex16(v: u16) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut buf = [0u8; 5];
    buf[0] = HEX[((v >> 12) & 0xF) as usize];
    buf[1] = HEX[((v >> 8) & 0xF) as usize];
    buf[2] = HEX[((v >> 4) & 0xF) as usize];
    buf[3] = HEX[(v & 0xF) as usize];
    buf[4] = 0;
    print_color(core::str::from_utf8(&buf[..4]).unwrap_or("????"), COLOR_WHITE);
}

/// Scan all reachable PCI buses and print present devices.
pub fn scan_print() {
    println_color("PCI:", COLOR_LIGHT_CYAN);
    let mut found = 0u32;
    scan(|addr, id, class_reg| {
        let vendor = (id & 0xFFFF) as u16;
        let device = ((id >> 16) & 0xFFFF) as u16;
        let class_code = (class_reg >> 24) as u8;
        let subclass = (class_reg >> 16) as u8;

        print_color("  ", COLOR_WHITE);
        print_hex16(vendor);
        print_color(":", COLOR_WHITE);
        print_hex16(device);
        print_color(" @ ", COLOR_WHITE);
        let mut num = [0u8; 4];
        let n = u64_to_dec(addr.bus as u64, &mut num);
        print_color(core::str::from_utf8(&num[..n]).unwrap_or("?"), COLOR_WHITE);
        print_color(":", COLOR_WHITE);
        let n = u64_to_dec(addr.slot as u64, &mut num);
        print_color(core::str::from_utf8(&num[..n]).unwrap_or("?"), COLOR_WHITE);
        print_color(".", COLOR_WHITE);
        let n = u64_to_dec(addr.func as u64, &mut num);
        print_color(core::str::from_utf8(&num[..n]).unwrap_or("?"), COLOR_WHITE);
        print_color(" class ", COLOR_LIGHT_CYAN);
        print_hex16(((class_code as u16) << 8) | subclass as u16);
        unsafe {
            crate::console_ffi::console_newline();
        }
        found += 1;
        true
    });
    if found == 0 {
        println_color("  (none)", COLOR_WHITE);
    }
}

// ------------------------------------------------------------- capabilities

pub const CAP_PM: u8 = 0x01;
pub const CAP_MSI: u8 = 0x05;
pub const CAP_VENDOR: u8 = 0x09;
pub const CAP_PCIE: u8 = 0x10;
pub const CAP_MSIX: u8 = 0x11;

/// Walk the capability list: (cap id, config offset). Status bit 4 gates it; the
/// walk stops after 48 entries so a looped list cannot hang the kernel.
pub fn for_each_cap(a: PciAddr, mut f: impl FnMut(u8, u8)) {
    let status = cfg_read16(a.bus, a.slot, a.func, 0x06);
    if status & (1 << 4) == 0 {
        return;
    }
    let mut ptr = cfg_read8(a.bus, a.slot, a.func, 0x34) & 0xFC;
    let mut guard = 0;
    while ptr >= 0x40 && guard < 48 {
        let id = cfg_read8(a.bus, a.slot, a.func, ptr);
        let next = cfg_read8(a.bus, a.slot, a.func, ptr + 1) & 0xFC;
        f(id, ptr);
        ptr = next;
        guard += 1;
    }
}

/// MSI-X capability: (table size, table BIR, table offset, PBA BIR, PBA offset).
pub fn msix_info(a: PciAddr) -> Option<(u16, u8, u32, u8, u32)> {
    let mut found = None;
    for_each_cap(a, |id, off| {
        if id == CAP_MSIX && found.is_none() {
            let mc = cfg_read16(a.bus, a.slot, a.func, off + 2);
            let table = cfg_read32(a.bus, a.slot, a.func, off + 4);
            let pba = cfg_read32(a.bus, a.slot, a.func, off + 8);
            found = Some(((mc & 0x7FF) + 1, (table & 7) as u8, table & !7, (pba & 7) as u8, pba & !7));
        }
    });
    found
}

// ------------------------------------------------------------- PciFunction

pub const DEV_VIRTIO_NET_LEGACY: u16 = 0x1000;
pub const DEV_VIRTIO_NET_MODERN: u16 = 0x1041;

/// The mediated grant source for one PCI function. Drivers get BARs, capability
/// reads and `enable` through it; they cannot write BARs, MSI-X entries or
/// arbitrary config space (the line that later stops a Ring-3 driver injecting
/// interrupts).
pub struct PciFunction {
    addr: PciAddr,
    vendor: u16,
    device: u16,
    subsys_dev: u16,
    grant: GrantId,
}

pub enum Bar {
    Mmio(MmioRegion),
    Io(PortRange),
}

impl PciFunction {
    pub fn addr(&self) -> PciAddr {
        self.addr
    }
    pub fn ids(&self) -> (u16, u16) {
        (self.vendor, self.device)
    }
    pub fn subsystem_device(&self) -> u16 {
        self.subsys_dev
    }
    pub fn grant(&self) -> GrantId {
        self.grant
    }
    pub fn cfg_r8(&self, off: u8) -> u8 {
        cfg_read8(self.addr.bus, self.addr.slot, self.addr.func, off)
    }
    pub fn cfg_r16(&self, off: u8) -> u16 {
        cfg_read16(self.addr.bus, self.addr.slot, self.addr.func, off)
    }
    pub fn cfg_r32(&self, off: u8) -> u32 {
        cfg_read32(self.addr.bus, self.addr.slot, self.addr.func, off)
    }
    pub fn caps(&self, f: impl FnMut(u8, u8)) {
        for_each_cap(self.addr, f)
    }
    /// Raw `virtio_pci_cap` (or any vendor cap) bytes at `cap_off`.
    pub fn vendor_cap(&self, cap_off: u8) -> [u8; 20] {
        let mut out = [0u8; 20];
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.cfg_r8(cap_off.wrapping_add(i as u8));
        }
        out
    }
    /// Map BAR `i`: memory BARs uncached through `map_mmio_uc`, I/O BARs sized.
    pub fn bar(&self, i: u8) -> Result<Bar, GrantError> {
        let a = self.addr;
        if let Some((base, size)) = bar_io_sized(a.bus, a.slot, a.func, i) {
            return Ok(Bar::Io(PortRange::new(base, size, self.grant)));
        }
        let (phys, size) = bar_mem(a.bus, a.slot, a.func, i).ok_or(GrantError::Unsupported)?;
        if size > (1 << 30) {
            return Err(GrantError::OutOfRange);
        }
        Ok(Bar::Mmio(MmioRegion::map(phys, size as usize, self.grant)?))
    }
    /// Enable decoding / bus mastering. Always sets INTx-disable (bit 10): PCI
    /// devices never use legacy PIC INTx in this kernel.
    pub fn enable(&self, mem: bool, io: bool, bus_master: bool) {
        let a = self.addr;
        let mut cmd = cfg_read16(a.bus, a.slot, a.func, 0x04);
        cmd |= 1 << 10;
        if io {
            cmd |= 1;
        }
        if mem {
            cmd |= 2;
        }
        if bus_master {
            cmd |= 4;
        }
        cfg_write16(a.bus, a.slot, a.func, 0x04, cmd);
    }
    pub fn disable_bus_master(&self) {
        let a = self.addr;
        let cmd = cfg_read16(a.bus, a.slot, a.func, 0x04);
        cfg_write16(a.bus, a.slot, a.func, 0x04, cmd & !4);
    }
    /// Memory decode + bus mastering on every bridge between this function and
    /// the CPU: a device's DMA and MSI writes only travel upstream through
    /// bridges whose own Bus Master Enable is set, and firmware often clears it
    /// on ports to devices it did not use. Returns (bridge, command before,
    /// command after), nearest first.
    pub fn enable_upstream(&self) -> alloc::vec::Vec<(PciAddr, u16, u16)> {
        let mut out = alloc::vec::Vec::new();
        for b in bridge_path(self.addr) {
            let before = cfg_read16(b.bus, b.slot, b.func, 0x04);
            if before & 0x6 != 0x6 {
                cfg_write16(b.bus, b.slot, b.func, 0x04, before | 0x6);
            }
            out.push((b, before, cfg_read16(b.bus, b.slot, b.func, 0x04)));
        }
        out
    }
    /// Put the function in power state D0 (PCI PM capability 0x01, PMCSR bits
    /// 1:0) if firmware left it in D1-D3hot, where its BARs read all-ones.
    /// Returns the state it was in, or None without a PM capability. PME_Status
    /// (RW1C) is written as 0, so it is left alone. The caller waits the 10 ms a
    /// D3hot -> D0 transition needs.
    pub fn set_power_d0(&self) -> Option<u8> {
        let mut cap = None;
        for_each_cap(self.addr, |id, off| {
            if id == CAP_PM && cap.is_none() {
                cap = Some(off);
            }
        });
        let off = cap? + 4;
        let a = self.addr;
        let old = (cfg_read16(a.bus, a.slot, a.func, off) & 0b11) as u8;
        if old != 0 {
            cfg_rmw32(a.bus, a.slot, a.func, off, |v| v & 0x0000_7FFC);
        }
        Some(old)
    }
    /// Turn off ASPM (L0s/L1 link power states, Link Control bits 1:0) on this
    /// PCIe function, as chip bring-up sequences require. Returns the old Link
    /// Control value, or None without a PCIe capability. Link Status (the upper
    /// half of the dword) is written as 0, a no-op for its RW1C bits.
    pub fn pcie_aspm_disable(&self) -> Option<u16> {
        let mut cap = None;
        for_each_cap(self.addr, |id, off| {
            if id == CAP_PCIE && cap.is_none() {
                cap = Some(off);
            }
        });
        let off = cap? + 0x10;
        let a = self.addr;
        let old = cfg_read16(a.bus, a.slot, a.func, off);
        cfg_rmw32(a.bus, a.slot, a.func, off, |v| (v & 0xFFFF) & !0b11);
        Some(old)
    }
}

/// Functions handed out by `claim` (bus/slot/func), so two drivers never share one.
static CLAIMED: IrqSpinLock<[Option<PciAddr>; 32]> = IrqSpinLock::new([None; 32]);

/// Claim the `n`th function matching `ids` (and the subsystem device, if given:
/// transitional virtio IDs are shared by every virtio type) that nobody claimed.
pub fn claim(ids: &[(u16, u16)], subsys: Option<u16>, n: usize) -> Option<PciFunction> {
    let mut seen = 0usize;
    let mut found: Option<(PciAddr, u16, u16, u16)> = None;
    scan(|addr, id, _class| {
        let v = (id & 0xFFFF) as u16;
        let d = (id >> 16) as u16;
        if !ids.iter().any(|&(iv, idv)| iv == v && idv == d) {
            return true;
        }
        let sub = cfg_read16(addr.bus, addr.slot, addr.func, 0x2E);
        if subsys.map_or(false, |want| want != sub) {
            return true;
        }
        if seen == n {
            found = Some((addr, v, d, sub));
            return false;
        }
        seen += 1;
        true
    });
    let (addr, vendor, device, subsys_dev) = found?;
    let mut claimed = CLAIMED.lock();
    if claimed.iter().flatten().any(|c| *c == addr) {
        return None;
    }
    let slot = claimed.iter_mut().find(|c| c.is_none())?;
    *slot = Some(addr);
    Some(PciFunction { addr, vendor, device, subsys_dev, grant: crate::drivers::grant::new_grant_id() })
}

/// Give a claimed function back (driver teardown).
pub fn release(f: PciFunction) {
    let mut claimed = CLAIMED.lock();
    for c in claimed.iter_mut() {
        if *c == Some(f.addr) {
            *c = None;
        }
    }
}

/// All functions matching (vendor, device) pairs, any claim state: (addr, device id).
pub fn find_all(ids: &[(u16, u16)], mut f: impl FnMut(PciAddr, u16)) {
    scan(|addr, id, _| {
        let v = (id & 0xFFFF) as u16;
        let d = (id >> 16) as u16;
        if ids.iter().any(|&(iv, idv)| iv == v && idv == d) {
            f(addr, d);
        }
        true
    });
}

/// PCI-PCI bridges between `a` and the host bridge, nearest first: each one
/// whose secondary..subordinate bus range holds the bus below it.
pub fn bridge_path(a: PciAddr) -> alloc::vec::Vec<PciAddr> {
    let mut path = alloc::vec::Vec::new();
    let mut bus = a.bus;
    for _ in 0..8 {
        if bus == 0 {
            break;
        }
        let mut up = None;
        scan(|b, _id, class_reg| {
            if (class_reg >> 16) == 0x0604 {
                let buses = cfg_read32(b.bus, b.slot, b.func, 0x18);
                let sec = ((buses >> 8) & 0xFF) as u8;
                let sub = ((buses >> 16) & 0xFF) as u8;
                if sec <= bus && bus <= sub && b.bus < bus {
                    up = Some(b);
                    return false;
                }
            }
            true
        });
        match up {
            Some(b) => {
                path.push(b);
                bus = b.bus;
            }
            None => break,
        }
    }
    path
}

/// PCIe capability offset of `a`, if any.
pub fn pcie_cap(a: PciAddr) -> Option<u8> {
    let mut cap = None;
    for_each_cap(a, |id, off| {
        if id == CAP_PCIE && cap.is_none() {
            cap = Some(off);
        }
    });
    cap
}

/// Error-relevant status of one function: (command, status, secondary status
/// for bridges, PCIe device status).
#[derive(Clone, Copy, Debug)]
pub struct ErrStatus {
    pub command: u16,
    pub status: u16,
    pub sec_status: Option<u16>,
    pub dev_status: Option<u16>,
}

pub fn err_status(a: PciAddr) -> ErrStatus {
    let header = cfg_read8(a.bus, a.slot, a.func, 0x0E) & 0x7F;
    ErrStatus {
        command: cfg_read16(a.bus, a.slot, a.func, 0x04),
        status: cfg_read16(a.bus, a.slot, a.func, 0x06),
        sec_status: if header == 1 { Some(cfg_read16(a.bus, a.slot, a.func, 0x1E)) } else { None },
        dev_status: pcie_cap(a).map(|c| cfg_read16(a.bus, a.slot, a.func, c + 0x0A)),
    }
}

/// Clear the write-1-to-clear error bits of `a` (status, bridge secondary
/// status, PCIe device status) so a later `err_status` shows only new errors.
pub fn clear_err_status(a: PciAddr) {
    cfg_write16(a.bus, a.slot, a.func, 0x06, 0xF900);
    if cfg_read8(a.bus, a.slot, a.func, 0x0E) & 0x7F == 1 {
        cfg_write16(a.bus, a.slot, a.func, 0x1E, 0xF900);
    }
    if let Some(c) = pcie_cap(a) {
        cfg_write16(a.bus, a.slot, a.func, c + 0x0A, 0x000F);
    }
}

/// Every present function: (address, vendor, device, class register).
pub fn for_each_function(mut f: impl FnMut(PciAddr, u16, u16, u32)) {
    scan(|addr, id, class_reg| {
        f(addr, (id & 0xFFFF) as u16, (id >> 16) as u16, class_reg);
        true
    });
}

fn cap_name(id: u8) -> &'static str {
    match id {
        0x01 => "pm",
        CAP_MSI => "msi",
        CAP_VENDOR => "vendor",
        CAP_PCIE => "pcie",
        CAP_MSIX => "msix",
        0x12 => "sata",
        0x13 => "af",
        _ => "other",
    }
}

fn virtio_cfg_name(t: u8) -> &'static str {
    match t {
        1 => "common",
        2 => "notify",
        3 => "isr",
        4 => "device",
        5 => "pci-cfg",
        8 => "shm",
        _ => "?",
    }
}

fn print_dec(v: u64) {
    let mut num = [0u8; 20];
    let n = u64_to_dec(v, &mut num);
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("?"), COLOR_WHITE);
}

/// `pci -caps`: every function with its capability list (MSI-X table size,
/// virtio cfg types).
pub fn scan_caps_print() {
    println_color("PCI capabilities:", COLOR_LIGHT_CYAN);
    scan(|addr, id, class_reg| {
        let vendor = (id & 0xFFFF) as u16;
        let device = (id >> 16) as u16;
        print_color("  ", COLOR_WHITE);
        print_hex16(vendor);
        print_color(":", COLOR_WHITE);
        print_hex16(device);
        print_color(" @ ", COLOR_WHITE);
        print_dec(addr.bus as u64);
        print_color(":", COLOR_WHITE);
        print_dec(addr.slot as u64);
        print_color(".", COLOR_WHITE);
        print_dec(addr.func as u64);
        print_color(" class ", COLOR_LIGHT_CYAN);
        print_hex16((class_reg >> 16) as u16);
        print_color(" caps:", COLOR_LIGHT_CYAN);
        let mut any = false;
        for_each_cap(addr, |cid, off| {
            any = true;
            print_color(" ", COLOR_WHITE);
            print_color(cap_name(cid), COLOR_WHITE);
            if cid == CAP_MSIX {
                let mc = cfg_read16(addr.bus, addr.slot, addr.func, off + 2);
                print_color("(", COLOR_WHITE);
                print_dec(((mc & 0x7FF) + 1) as u64);
                print_color(")", COLOR_WHITE);
            } else if cid == CAP_VENDOR && vendor == VENDOR_VIRTIO {
                let t = cfg_read8(addr.bus, addr.slot, addr.func, off + 3);
                print_color("(", COLOR_WHITE);
                print_color(virtio_cfg_name(t), COLOR_WHITE);
                print_color(")", COLOR_WHITE);
            }
        });
        if !any {
            print_color(" none", COLOR_WHITE);
        }
        unsafe {
            crate::console_ffi::console_newline();
        }
        true
    });
}
