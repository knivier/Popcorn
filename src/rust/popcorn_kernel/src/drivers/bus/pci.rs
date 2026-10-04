//! PCI config space — bus-0 scan, BAR read, virtio find.
//!
//! Only bus 0 is walked (no bridge recursion): devices behind PCIe root ports
//! (e.g. a laptop's NVMe) are not seen yet, which is the safe direction.

use crate::console_ffi::{print_color, println_color, u64_to_dec, COLOR_LIGHT_CYAN, COLOR_WHITE};
use crate::drivers::io::{inl, outl};

const CONFIG_ADDR: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

pub const VENDOR_VIRTIO: u16 = 0x1AF4;
pub const DEV_VIRTIO_BLK_LEGACY: u16 = 0x1001;
pub const DEV_VIRTIO_BLK_MODERN: u16 = 0x1042;

pub fn cfg_read32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let addr = (1u32 << 31)
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);
    unsafe {
        outl(CONFIG_ADDR, addr);
        inl(CONFIG_DATA)
    }
}

pub fn cfg_write32(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let addr = (1u32 << 31)
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);
    unsafe {
        outl(CONFIG_ADDR, addr);
        outl(CONFIG_DATA, value);
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

pub fn cfg_write16(bus: u8, slot: u8, func: u8, offset: u8, value: u16) {
    let aligned = offset & !3;
    let mut v = cfg_read32(bus, slot, func, aligned);
    if (offset & 2) != 0 {
        v = (v & 0x0000_FFFF) | ((value as u32) << 16);
    } else {
        v = (v & 0xFFFF_0000) | (value as u32);
    }
    cfg_write32(bus, slot, func, aligned, v);
}

#[derive(Clone, Copy)]
pub struct PciAddr {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
}

/// Walk every present function on bus 0. `f(addr, id_reg, class_reg)` returns
/// `false` to stop. Single-function slots are not probed past function 0.
fn scan(mut f: impl FnMut(PciAddr, u32, u32) -> bool) {
    for slot in 0u8..32 {
        let mut multi = false;
        for func in 0u8..8 {
            let id = cfg_read32(0, slot, func, 0);
            if id & 0xFFFF == 0xFFFF {
                if func == 0 {
                    break;
                }
                continue;
            }
            if func == 0 {
                multi = (cfg_read32(0, slot, 0, 0x0C) >> 16) & 0x80 != 0;
            }
            let class_reg = cfg_read32(0, slot, func, 0x08);
            let addr = PciAddr {
                bus: 0,
                slot,
                func,
            };
            if !f(addr, id, class_reg) {
                return;
            }
            if func == 0 && !multi {
                break;
            }
        }
    }
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

fn is_virtio_blk(id: u32) -> bool {
    let v = (id & 0xFFFF) as u16;
    let d = ((id >> 16) & 0xFFFF) as u16;
    v == VENDOR_VIRTIO && (d == DEV_VIRTIO_BLK_LEGACY || d == DEV_VIRTIO_BLK_MODERN)
}

/// Nth virtio-blk on bus 0 (0 = first); we drive the legacy I/O-BAR path.
pub fn find_virtio_blk_n(n: usize) -> Option<PciAddr> {
    let mut seen = 0usize;
    let mut found = None;
    scan(|addr, id, _class_reg| {
        if is_virtio_blk(id) {
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
        /* virtio-blk is handled as usb/virtio disks. */
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

/// Scan PCI bus 0 and print present devices (vendor:device @ bus:slot.func).
pub fn scan_bus0_print() {
    println_color("PCI bus 0:", COLOR_LIGHT_CYAN);
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
        print_color(" @ 0:", COLOR_WHITE);
        let mut num = [0u8; 4];
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
