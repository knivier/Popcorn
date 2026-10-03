//! PCI config space walk (bus 0) — foundation for virtio later.

use crate::console_ffi::{print_color, println_color, u64_to_dec, COLOR_LIGHT_CYAN, COLOR_WHITE};
use crate::drivers::io::{inl, outl};

const CONFIG_ADDR: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

fn cfg_read32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
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
                    break; // empty slot
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

            // Only walk other functions if multifunction.
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
