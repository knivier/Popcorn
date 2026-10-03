//! PS/2 keyboard chardev (`kbd` → `/dev/kbd`).
//! Controller channels are split (keyboard vs mouse/aux); mouse stays disabled for now.

use crate::drivers::io::{inb, outb};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;
const QUEUE_CAP: usize = 256;

/// 8042 controller helpers — keyboard and mouse/aux are separate channels.
mod kbc {
    use super::{inb, outb, DATA, STATUS};

    pub fn wait_write() {
        for _ in 0..100_000u32 {
            if unsafe { (inb(STATUS) & 0x02) == 0 } {
                return;
            }
        }
    }

    pub fn wait_read() {
        for _ in 0..100_000u32 {
            if unsafe { (inb(STATUS) & 0x01) != 0 } {
                return;
            }
        }
    }

    pub fn write_cmd(cmd: u8) {
        wait_write();
        unsafe { outb(STATUS, cmd) }
    }

    pub fn write_data(data: u8) {
        wait_write();
        unsafe { outb(DATA, data) }
    }

    pub fn read_data() -> u8 {
        wait_read();
        unsafe { inb(DATA) }
    }

    pub fn status() -> u8 {
        unsafe { inb(STATUS) }
    }

    /// First PS/2 port (keyboard).
    pub fn enable_keyboard() {
        write_cmd(0xAE);
    }

    #[allow(dead_code)]
    pub fn disable_keyboard() {
        write_cmd(0xAD);
    }

    /// Second PS/2 port (mouse/aux) — kept off until a mouse drive exists.
    #[allow(dead_code)]
    pub fn enable_mouse() {
        write_cmd(0xA8);
    }

    pub fn disable_mouse() {
        write_cmd(0xA7);
    }

    #[allow(dead_code)]
    pub fn write_mouse_data(data: u8) {
        write_cmd(0xD4); // next byte → aux
        write_data(data);
    }

    pub fn data_ready() -> bool {
        (status() & 0x01) != 0
    }

    /// Bit 5 set ⇒ byte came from mouse/aux (when dual-channel is live).
    pub fn from_mouse() -> bool {
        (status() & 0x20) != 0
    }
}

static READY: AtomicBool = AtomicBool::new(false);
static HEAD: AtomicU32 = AtomicU32::new(0);
static TAIL: AtomicU32 = AtomicU32::new(0);
static mut QUEUE: [u8; QUEUE_CAP] = [0; QUEUE_CAP];

fn push(scancode: u8) {
    let tail = TAIL.load(Ordering::Relaxed);
    let next = (tail + 1) % QUEUE_CAP as u32;
    let head = HEAD.load(Ordering::Relaxed);
    if next == head {
        return;
    }
    unsafe {
        QUEUE[tail as usize] = scancode;
    }
    TAIL.store(next, Ordering::Release);
}

pub fn probe() -> Result<(), &'static str> {
    if READY.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    /* Mouse-ready split: disable aux, enable keyboard only, drain stale bytes. */
    kbc::disable_mouse();
    kbc::enable_keyboard();
    while kbc::data_ready() {
        let _ = kbc::read_data();
    }
    Ok(())
}

/// Drain keyboard-channel bytes into the queue (ignore aux if flagged).
pub fn poll() {
    if !kbc::data_ready() {
        return;
    }
    if kbc::from_mouse() {
        let _ = kbc::read_data(); // discard aux until mouse drive exists
        return;
    }
    push(kbc::read_data());
}

pub fn irq() {
    poll();
}

pub fn pop() -> Option<u8> {
    let head = HEAD.load(Ordering::Acquire);
    let tail = TAIL.load(Ordering::Acquire);
    if head == tail {
        return None;
    }
    let b = unsafe { QUEUE[head as usize] };
    HEAD.store((head + 1) % QUEUE_CAP as u32, Ordering::Release);
    Some(b)
}

pub fn read(buf: &mut [u8]) -> i64 {
    if buf.is_empty() {
        return 0;
    }
    let mut n = 0i64;
    while (n as usize) < buf.len() {
        match pop() {
            Some(b) => {
                buf[n as usize] = b;
                n += 1;
            }
            None => break,
        }
    }
    n
}

pub fn write(_buf: &[u8]) -> i64 {
    -2
}

pub fn ioctl(_request: u64, _argp: *mut u8) -> i64 {
    -2
}

pub fn cmd(cmd: &str) -> alloc::string::String {
    match cmd {
        "status" | "info" => {
            if READY.load(Ordering::Acquire) {
                alloc::string::String::from("kbd ready node=/dev/kbd (mouse aux off)")
            } else {
                alloc::string::String::from("kbd idle")
            }
        }
        _ => alloc::string::String::from("error: unknown cmd (status|info)"),
    }
}
