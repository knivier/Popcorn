//! VGA text /dev/tty0 — CRTC + 0xB8000 cell writes.

use crate::drivers::io::outb;

const VGA_MEM: usize = 0xB8000;
const VGA_WIDTH: u32 = 80;
const VGA_HEIGHT: u32 = 25;
const VGA_CTRL: u16 = 0x3D4;
const VGA_DATA: u16 = 0x3D5;

static mut ACTIVE: bool = false;

pub fn probe() -> Result<(), &'static str> {
    unsafe {
        ACTIVE = true;
    }
    Ok(())
}

pub fn is_active() -> bool {
    unsafe { ACTIVE }
}

pub fn set_cursor(x: u32, y: u32) {
    let pos = (y * VGA_WIDTH + x) as u16;
    unsafe {
        outb(VGA_CTRL, 0x0F);
        outb(VGA_DATA, (pos & 0xFF) as u8);
        outb(VGA_CTRL, 0x0E);
        outb(VGA_DATA, (pos >> 8) as u8);
    }
}

pub fn write_cell(x: u32, y: u32, ch: u8, attr: u8) {
    if x >= VGA_WIDTH || y >= VGA_HEIGHT {
        return;
    }
    let off = ((y * VGA_WIDTH + x) * 2) as usize;
    unsafe {
        let p = VGA_MEM as *mut u8;
        *p.add(off) = ch;
        *p.add(off + 1) = attr;
    }
}

pub fn read(_buf: &mut [u8]) -> i64 {
    0
}

pub fn write(buf: &[u8]) -> i64 {
    // Stream writes go through console UX; accept and discard here.
    let _ = buf;
    buf.len() as i64
}

pub fn ioctl(request: u64, argp: *mut u8) -> i64 {
    match request {
        0x540B => {
            // TIOCGWINSZ
            if !argp.is_null() {
                unsafe {
                    let w = argp as *mut u16;
                    *w = VGA_HEIGHT as u16;
                    *w.add(1) = VGA_WIDTH as u16;
                    *w.add(2) = 0;
                    *w.add(3) = 0;
                }
            }
            0
        }
        0x5401 | 0x5402 => 0,
        _ => -2,
    }
}
