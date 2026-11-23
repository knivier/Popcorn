//! PS/2 keyboard chardev (`kbd` → `/dev/kbd`).
//! Controller channels are split (keyboard vs mouse/aux); mouse stays disabled for now.

use crate::drivers::io::{inb, outb};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const DATA: u16 = 0x60;
const STATUS: u16 = 0x64;
const QUEUE_CAP: usize = 256;
/// Status bit 0: output buffer full (a byte is waiting at 0x60).
const ST_OBF: u8 = 0x01;
/// Status bit 5: the waiting byte came from the mouse/aux channel.
const ST_AUX: u8 = 0x20;
/// Upper bound on stale bytes drained at probe (the 8042 buffer is one byte deep).
const PROBE_DRAIN_MAX: u32 = 32;

/// 8042 controller helpers — keyboard and mouse/aux are separate channels.
mod kbc {
    use super::{inb, outb, DATA, STATUS, ST_OBF};

    pub fn wait_write() {
        for _ in 0..100_000u32 {
            if unsafe { (inb(STATUS) & 0x02) == 0 } {
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

    /// Read 0x60 without waiting. Only call after a status read showed OBF set;
    /// reading with OBF clear returns the stale latched byte.
    pub fn take_data() -> u8 {
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
        (status() & ST_OBF) != 0
    }
}

/// Save RFLAGS and mask interrupts. The 8042 read + queue push must not be
/// split by IRQ1, which runs the same producer path.
#[inline(always)]
fn irq_save() -> u64 {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq", "pop {}", "cli", out(reg) flags);
    }
    flags
}

/// Restore the caller's IF state (stays masked if it was masked, e.g. UEFI poll mode or IRQ context).
#[inline(always)]
fn irq_restore(flags: u64) {
    if (flags & (1 << 9)) != 0 {
        unsafe {
            core::arch::asm!("sti", options(nostack));
        }
    }
}

static READY: AtomicBool = AtomicBool::new(false);
static HEAD: AtomicU32 = AtomicU32::new(0);
static TAIL: AtomicU32 = AtomicU32::new(0);
static mut QUEUE: [u8; QUEUE_CAP] = [0; QUEUE_CAP];

/// Single producer: only called from `poll_masked`, with interrupts off.
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
    if READY.load(Ordering::Acquire) {
        return Ok(());
    }
    /* No 8042 decoding port 0x64 (UEFI-only board, no legacy emulation): the bus floats to 0xFF. */
    if kbc::status() == 0xFF {
        return Err("no 8042");
    }
    /* Mouse-ready split: disable aux, enable keyboard only, drain stale bytes. */
    kbc::disable_mouse();
    kbc::enable_keyboard();
    let mut drained = 0u32;
    while kbc::data_ready() {
        if drained >= PROBE_DRAIN_MAX {
            return Err("8042 stuck");
        }
        let _ = kbc::take_data();
        drained += 1;
    }
    /* Only mark ready once the controller is confirmed, so a failed probe can be retried. */
    READY.store(true, Ordering::Release);
    Ok(())
}

/// Move at most one waiting byte into the queue. Caller has interrupts masked.
fn poll_masked() {
    /* One status read decides both "byte waiting" and "which channel": a second
     * read could see a different byte's flags or OBF already cleared. */
    let st = kbc::status();
    if (st & ST_OBF) == 0 {
        return;
    }
    let b = kbc::take_data();
    if (st & ST_AUX) != 0 {
        return; // discard aux until mouse drive exists
    }
    push(b);
}

/// Drain keyboard-channel bytes into the queue (ignore aux if flagged).
/// Safe from both thread context (main loop / Dolphin) and IRQ1: the 8042
/// read and queue push run with interrupts masked, so there is one producer at a time.
pub fn poll() {
    if !READY.load(Ordering::Acquire) {
        return; // no controller (or probe not run yet): never read a floating 0x60
    }
    let flags = irq_save();
    poll_masked();
    irq_restore(flags);
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
