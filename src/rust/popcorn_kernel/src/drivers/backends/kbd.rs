//! PS/2 keyboard chardev (`kbd` → `/dev/kbd`).
//! Controller channels are split (keyboard vs mouse/aux); mouse stays disabled for now.

use crate::drivers::io::{inb, outb};
use crate::sync::IrqSpinLock;
use core::sync::atomic::{AtomicBool, Ordering};

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

/// Scancode ring. Head == tail means empty; one slot stays unused.
struct Ring {
    buf: [u8; QUEUE_CAP],
    head: usize,
    tail: usize,
}

impl Ring {
    /// Drops the byte when full.
    fn push(&mut self, scancode: u8) {
        let next = (self.tail + 1) % QUEUE_CAP;
        if next == self.head {
            return;
        }
        self.buf[self.tail] = scancode;
        self.tail = next;
    }

    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            return None;
        }
        let b = self.buf[self.head];
        self.head = (self.head + 1) % QUEUE_CAP;
        Some(b)
    }
}

static READY: AtomicBool = AtomicBool::new(false);
/// Producers (IRQ1, thread-context polls) and consumers (shell, Dolphin, /dev/kbd)
/// all take this lock, which masks interrupts: the 8042 read + push cannot be
/// split by IRQ1, and a reader cannot be interleaved with the IRQ producer.
static QUEUE: IrqSpinLock<Ring> = IrqSpinLock::new(Ring { buf: [0; QUEUE_CAP], head: 0, tail: 0 });

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

/// Move at most one waiting byte into the queue. Caller holds the queue lock (interrupts masked).
fn poll_masked(q: &mut Ring) {
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
    q.push(b);
}

/// Drain keyboard-channel bytes into the queue (ignore aux if flagged).
/// Safe from both thread context (main loop / Dolphin) and IRQ1: the 8042
/// read and queue push run under the queue lock with interrupts masked, so
/// there is one producer at a time.
pub fn poll() {
    if !READY.load(Ordering::Acquire) {
        return; // no controller (or probe not run yet): never read a floating 0x60
    }
    QUEUE.with(poll_masked);
}

pub fn irq() {
    poll();
}

pub fn pop() -> Option<u8> {
    QUEUE.lock().pop()
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
