//! COM1 /dev/ttyS0

use crate::drivers::io::{inb, outb};

const COM1: u16 = 0x3F8;

pub fn probe() -> Result<(), &'static str> {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x03);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        outb(COM1 + 4, 0x0B);
    }
    Ok(())
}

fn tx_ready() -> bool {
    unsafe { (inb(COM1 + 5) & 0x20) != 0 }
}

pub fn putc(c: u8) {
    for _ in 0..100000u32 {
        if tx_ready() {
            break;
        }
    }
    unsafe {
        outb(COM1, c);
    }
}

pub fn read(_buf: &mut [u8]) -> i64 {
    0
}

pub fn write(buf: &[u8]) -> i64 {
    for &b in buf {
        if b == b'\n' {
            putc(b'\r');
        }
        putc(b);
    }
    buf.len() as i64
}

pub fn ioctl(_request: u64, _argp: *mut u8) -> i64 {
    -2
}
