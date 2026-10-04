//! PCI config space — scan, BAR read, virtio find.

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

/// I/O BAR base (legacy virtio) or 0 if not I/O.
pub fn bar_io(bus: u8, slot: u8, func: u8, bar: u8) -> Option<u16> {
    let off = 0x10 + bar * 4;
    let lo = cfg_read32(bus, slot, func, off);
    if (lo & 1) == 0 {
        return None;
    }
    Some((lo & !0x3) as u16)
}

/// Enable I/O + memory + bus master.
pub fn enable_bus_master(bus: u8, slot: u8, func: u8) {
    let mut cmd = cfg_read16(bus, slot, func, 0x04);
    cmd |= 0x0007; // IO | MEM | BUS_MASTER
    cfg_write16(bus, slot, func, 0x04, cmd);
}

/// Find first matching vendor:device on bus 0.
pub fn find_device(vendor: u16, device: u16) -> Option<PciAddr> {
    for slot in 0u8..32 {
        for func in 0u8..8 {
            let id = cfg_read32(0, slot, func, 0);
            let v = (id & 0xFFFF) as u16;
            if v == 0xFFFF {
                if func == 0 {
                    break;
                }
                continue;
            }
            let d = ((id >> 16) & 0xFFFF) as u16;
            if v == vendor && d == device {
                return Some(PciAddr {
                    bus: 0,
                    slot,
                    func,
                });
            }
            if func == 0 {
                let header = cfg_read32(0, slot, 0, 0x0C);
                let hdr_type = ((header >> 16) & 0xFF) as u8;
                if (hdr_type & 0x80) == 0 {
                    break;
                }
            }
        }
    }
    None
}

/// Find virtio-blk (legacy preferred, then modern id — we use legacy BAR path).
pub fn find_virtio_blk() -> Option<PciAddr> {
    find_virtio_blk_n(0)
}

/// Nth virtio-blk on bus 0 (0 = first).
pub fn find_virtio_blk_n(n: usize) -> Option<PciAddr> {
    let mut seen = 0usize;
    for slot in 0u8..32 {
        for func in 0u8..8 {
            let id = cfg_read32(0, slot, func, 0);
            let v = (id & 0xFFFF) as u16;
            if v == 0xFFFF {
                if func == 0 {
                    break;
                }
                continue;
            }
            let d = ((id >> 16) & 0xFFFF) as u16;
            if v == VENDOR_VIRTIO && (d == DEV_VIRTIO_BLK_LEGACY || d == DEV_VIRTIO_BLK_MODERN) {
                if seen == n {
                    return Some(PciAddr {
                        bus: 0,
                        slot,
                        func,
                    });
                }
                seen += 1;
            }
            if func == 0 {
                let header = cfg_read32(0, slot, 0, 0x0C);
                let hdr_type = ((header >> 16) & 0xFF) as u8;
                if (hdr_type & 0x80) == 0 {
                    break;
                }
            }
        }
    }
    None
}

/// Mass-storage class codes for internal-disk enumeration (no I/O yet).
pub fn foreach_storage_controller(mut f: impl FnMut(PciAddr, u8, u8)) {
    for slot in 0u8..32 {
        for func in 0u8..8 {
            let id = cfg_read32(0, slot, func, 0);
            let v = (id & 0xFFFF) as u16;
            if v == 0xFFFF {
                if func == 0 {
                    break;
                }
                continue;
            }
            /* Skip virtio-blk — handled as usb/virtio disks. */
            let d = ((id >> 16) & 0xFFFF) as u16;
            if v == VENDOR_VIRTIO && (d == DEV_VIRTIO_BLK_LEGACY || d == DEV_VIRTIO_BLK_MODERN) {
                continue;
            }
            let class_reg = cfg_read32(0, slot, func, 0x08);
            let class_code = (class_reg >> 24) as u8;
            let subclass = (class_reg >> 16) as u8;
            if class_code == 0x01 {
                /* IDE / ATA / SATA / NVMe / etc. */
                f(
                    PciAddr {
                        bus: 0,
                        slot,
                        func,
                    },
                    class_code,
                    subclass,
                );
            }
            if func == 0 {
                let header = cfg_read32(0, slot, 0, 0x0C);
                let hdr_type = ((header >> 16) & 0xFF) as u8;
                if (hdr_type & 0x80) == 0 {
                    break;
                }
            }
        }
    }
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
    for slot in 0u8..32 {
        for func in 0u8..8 {
            let id = cfg_read32(0, slot, func, 0);
            let vendor = (id & 0xFFFF) as u16;
            if vendor == 0xFFFF {
                if func == 0 {
                    break;
                }
                continue;
            }
            let device = ((id >> 16) & 0xFFFF) as u16;
            let class_reg = cfg_read32(0, slot, func, 0x08);
            let class_code = (class_reg >> 24) as u8;
            let subclass = (class_reg >> 16) as u8;

            print_color("  ", COLOR_WHITE);
            print_hex16(vendor);
            print_color(":", COLOR_WHITE);
            print_hex16(device);
            print_color(" @ 0:", COLOR_WHITE);
            let mut num = [0u8; 4];
            let n = u64_to_dec(slot as u64, &mut num);
            print_color(core::str::from_utf8(&num[..n]).unwrap_or("?"), COLOR_WHITE);
            print_color(".", COLOR_WHITE);
            let n = u64_to_dec(func as u64, &mut num);
            print_color(core::str::from_utf8(&num[..n]).unwrap_or("?"), COLOR_WHITE);
            print_color(" class ", COLOR_LIGHT_CYAN);
            print_hex16(((class_code as u16) << 8) | subclass as u16);
            unsafe {
                crate::console_ffi::console_newline();
            }
            found += 1;

            if func == 0 {
                let header = cfg_read32(0, slot, 0, 0x0C);
                let hdr_type = ((header >> 16) & 0xFF) as u8;
                if (hdr_type & 0x80) == 0 {
                    break;
                }
            }
        }
    }
    if found == 0 {
        println_color("  (none)", COLOR_WHITE);
    }
}
