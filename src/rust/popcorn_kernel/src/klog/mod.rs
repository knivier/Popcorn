//! USB-stick log: everything the console shows plus the COM1-only diagnostics
//! (driver tags, panics, CPU exception dumps), timestamped, kept in RAM from the
//! first byte of boot and written to `POPCORN.LOG` on the stick Popcorn booted
//! from. Meant for machines without a serial port (the T14s): boot, pull the stick,
//! read the file on another computer.
//!
//! Safety model (the boot stick is otherwise read-only to Popcorn): the file is
//! created at full size by the image build; `fatlog::locate` checks it is one
//! contiguous cluster run; the block layer then accepts log writes only inside
//! that run, only on the disk the UEFI loader booted from, and only while that
//! disk is not writable through the normal disk layer (an `install`/`wipe` of the
//! stick turns the log off first). FAT tables and directories are never written.
//!
//! When it is written: once right after the disks come up (that write also blanks
//! what the previous boot left), after the drivers probe, about once a second from
//! the main loop when there is new text, on `log -flush` and `stop`, and on a Rust
//! panic or CPU exception (best effort, only when no lock on the USB path is held).
//!
//! RAM holds the whole file (1 MiB). When it fills up, later output is dropped (the
//! boot is the part that matters) and counted in the header.

pub mod fatlog;

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::drivers::block;
use crate::sync::{IrqSpinLock, TaskLock};
use fatlog::{pad_byte, Extent, Header, SECTOR};

extern "C" {
    fn timer_get_uptime_ms() -> u64;
}

/// RAM text capacity: the whole file minus its header sector.
const CAP: usize = fatlog::FILE_BYTES - SECTOR;
const FULL_MARK: &[u8] = b"\n[log full: later output dropped]\n";
/// Consecutive write failures before the log turns itself off.
const MAX_FAILURES: u32 = 3;
/// Main-loop flush period.
const FLUSH_MS: u64 = 1000;

/// All fields start at zero on purpose: the 1 MiB static then lands in .bss
/// instead of being stored in the kernel image.
struct Buf {
    text: [u8; CAP],
    len: usize,
    /// First byte not yet on disk (lowered by backspace and line trimming).
    dirty_from: usize,
    /// Text limit once attached (the file's capacity); 0 = CAP.
    limit: usize,
    /// Where the current line's timestamp starts, and where its text starts.
    prefix_start: usize,
    line_start: usize,
    /// Inside a line (its timestamp has been written).
    mid_line: bool,
    /// Some text has been logged (no blank lines before the first one).
    any_text: bool,
    /// Consecutive blank lines written (collapsed to one).
    blanks: u8,
    full: bool,
    /// Lines logged before the TSC was calibrated: (timestamp offset, TSC). Their
    /// placeholder stamps are rewritten by `restamp` at attach, before anything
    /// reaches the disk.
    pending: [(u32, u64); PENDING_MAX],
    npending: usize,
}

/// Lines stamped later (boot up to the log attach is well under this; any extra
/// keep their "?" placeholder).
const PENDING_MAX: usize = 512;
/// Width of "[sssss.mmm] " (fixed, so a stamp can be rewritten in place).
const STAMP_LEN: usize = 12;

impl Buf {
    const fn new() -> Self {
        Self {
            text: [0; CAP],
            len: 0,
            dirty_from: 0,
            limit: 0,
            prefix_start: 0,
            line_start: 0,
            mid_line: false,
            any_text: false,
            blanks: 0,
            full: false,
            pending: [(0, 0); PENDING_MAX],
            npending: 0,
        }
    }

    /// Rewrite the placeholder stamps of lines logged before calibration.
    fn restamp(&mut self, hz: u64, tsc0: u64) {
        for k in 0..self.npending {
            let (off, tsc) = self.pending[k];
            let off = off as usize;
            /* The line may have been dropped (blank) and its offset reused by a
             * later line, whose entry comes later and wins. */
            if off + STAMP_LEN > self.len || self.text[off] != b'[' {
                continue;
            }
            let ms = (tsc.wrapping_sub(tsc0) as u128 * 1000 / hz as u128) as u64;
            let s = stamp(ms);
            self.text[off..off + STAMP_LEN].copy_from_slice(&s);
        }
        self.npending = 0;
    }

    fn limit(&self) -> usize {
        if self.limit == 0 {
            CAP
        } else {
            self.limit
        }
    }

    fn raw(&mut self, c: u8) {
        if self.full {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.len + FULL_MARK.len() >= self.limit() {
            self.full = true;
            self.text[self.len..self.len + FULL_MARK.len()].copy_from_slice(FULL_MARK);
            self.len += FULL_MARK.len();
            DROPPED.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.text[self.len] = c;
        self.len += 1;
    }

    fn truncate(&mut self, to: usize) {
        if to < self.len {
            self.len = to;
            self.dirty_from = self.dirty_from.min(to);
        }
    }

    fn prefix(&mut self) {
        self.prefix_start = self.len;
        let s = if TSC_HZ.load(Ordering::Relaxed) == 0 {
            if self.npending < PENDING_MAX {
                self.pending[self.npending] = (self.len as u32, rdtsc());
                self.npending += 1;
            }
            *b"[    ?.???] "
        } else {
            stamp(now_ms())
        };
        for c in s {
            self.raw(c);
        }
        self.line_start = self.len;
    }

    fn put(&mut self, c: u8) {
        match c {
            b'\r' => {}
            0x08 => {
                if self.mid_line && self.len > self.line_start && !self.full {
                    self.truncate(self.len - 1);
                }
            }
            b'\n' => {
                if self.full {
                    return;
                }
                if self.mid_line {
                    self.mid_line = false;
                    /* Trailing spaces (padded progress lines) go; an empty line
                     * loses its timestamp too and counts as blank. */
                    let mut end = self.len;
                    while end > self.line_start && self.text[end - 1] == b' ' {
                        end -= 1;
                    }
                    if end > self.line_start {
                        self.truncate(end);
                        self.raw(b'\n');
                        self.any_text = true;
                        self.blanks = 0;
                        return;
                    }
                    self.truncate(self.prefix_start);
                }
                if self.any_text && self.blanks == 0 {
                    self.raw(b'\n');
                }
                self.blanks = self.blanks.saturating_add(1);
            }
            c if c < 0x20 && c != b'\t' => {}
            c => {
                if self.full {
                    DROPPED.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if !self.mid_line {
                    self.prefix();
                    self.mid_line = true;
                }
                self.raw(c);
            }
        }
    }
}

/// The text. IrqSpinLock, and only ever `try_lock`ed by `putc`: a byte printed
/// from a path that already holds it (a panic inside the log code) is dropped
/// instead of deadlocking. Never held across disk I/O.
static BUF: IrqSpinLock<Buf> = IrqSpinLock::new(Buf::new());
static DROPPED: AtomicU64 = AtomicU64::new(0);
/// TSC at the first logged byte, and the calibrated TSC rate (set at attach).
/// Timestamps come from the TSC once calibrated: the PIT tick only starts after
/// the drivers have probed, so it would stamp all of boot 0.000.
static TSC0: AtomicU64 = AtomicU64::new(0);
static TSC_HZ: AtomicU64 = AtomicU64::new(0);

fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// "[sssss.mmm] " (seconds wrap at 100000, which keeps the width fixed).
fn stamp(ms: u64) -> [u8; STAMP_LEN] {
    let mut s = *b"[    0.000] ";
    let mut sec = (ms / 1000) % 100_000;
    let mut frac = ms % 1000;
    for i in (7..10).rev() {
        s[i] = b'0' + (frac % 10) as u8;
        frac /= 10;
    }
    let mut i = 5;
    loop {
        s[i] = b'0' + (sec % 10) as u8;
        sec /= 10;
        if sec == 0 || i == 1 {
            break;
        }
        i -= 1;
    }
    s
}

/// Milliseconds since the first logged byte (TSC), or PIT uptime before the TSC
/// is calibrated.
fn now_ms() -> u64 {
    let hz = TSC_HZ.load(Ordering::Relaxed);
    if hz == 0 {
        return unsafe { timer_get_uptime_ms() };
    }
    let d = rdtsc().wrapping_sub(TSC0.load(Ordering::Relaxed));
    (d as u128 * 1000 / hz as u128) as u64
}
static EMERGENCY: AtomicBool = AtomicBool::new(false);
static LAST_FLUSH_MS: AtomicU64 = AtomicU64::new(0);

struct Target {
    disk: usize,
    disk_name: &'static str,
    ext: Extent,
    boot: u32,
    /// Text bytes on disk that may still need blanking (the previous boot's length
    /// at first, then the length of the last flush).
    on_disk: usize,
    flushes: u64,
    failures: u32,
    /// The header on disk is behind (its write failed or has not happened yet).
    header_stale: bool,
}

struct State {
    target: Option<Target>,
    /// Why there is no target (shown by `log`).
    off: &'static str,
}

/// Disk side. TaskLock: flushes do USB I/O and run from task context; the panic
/// path only `try_lock`s it.
static STATE: TaskLock<State> = TaskLock::new(State { target: None, off: "not attached yet" });

/// Append one byte (console mirror, COM1 diagnostics, exception dumps).
pub fn putc(c: u8) {
    if TSC0.load(Ordering::Relaxed) == 0 {
        let _ = TSC0.compare_exchange(0, rdtsc(), Ordering::Relaxed, Ordering::Relaxed);
    }
    match BUF.try_lock() {
        Some(mut b) => b.put(c),
        None => {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn say(args: core::fmt::Arguments) {
    let mut l = crate::net::LineBuf::new();
    let _ = l.write_fmt(args);
    crate::console_ffi::println_color(l.as_str(), crate::console_ffi::COLOR_LIGHT_CYAN);
}

/// Find the boot stick and its `POPCORN.LOG`, then write what boot has logged so
/// far. Prints one status line either way.
pub fn attach() {
    /* ~50 ms against PIT channel 2 (cached for `cpu -hz` too). Then give the
     * lines logged so far their real times. */
    let hz = crate::drivers::backends::cpuinfo::tsc_hz();
    {
        let mut b = BUF.lock();
        TSC_HZ.store(hz, Ordering::Relaxed);
        if hz != 0 {
            b.restamp(hz, TSC0.load(Ordering::Relaxed));
        }
    }
    if let Err(why) = attach_inner() {
        STATE.lock().off = why;
        say(format_args!("log: off ({})", why));
        return;
    }
    /* The target stays on a failed first write: a later flush may still work
     * (three failures in a row turn it off). */
    if let Err(why) = flush() {
        say(format_args!("log: first write failed ({})", why));
    }
}

fn attach_inner() -> Result<(), &'static str> {
    let disk = block::booted_disk().ok_or("no boot stick (not a UEFI USB boot)")?;
    let disk_sectors = block::log_disk_check(disk)?;
    let mut read = |lba: u64, s: &mut [u8; SECTOR]| block::log_read(disk, lba, s) >= 0;
    let ext = fatlog::locate(&mut read, disk_sectors)?;
    let cap = ext.text_capacity().min(CAP);
    let mut hs = [0u8; SECTOR];
    if !read(ext.lba, &mut hs) {
        return Err("cannot read the POPCORN.LOG header");
    }
    let prev = fatlog::parse_header(&hs, ext.text_capacity());
    block::log_set_extent(disk, ext.lba, ext.sectors)?;
    {
        let mut b = BUF.lock();
        b.limit = cap;
        if b.len + FULL_MARK.len() > cap {
            /* Cannot happen with the image's 1 MiB file; keep the invariant anyway. */
            let keep = cap.saturating_sub(FULL_MARK.len());
            b.truncate(keep);
        }
    }
    let name = block::disk_name(disk).unwrap_or("?");
    STATE.lock().target = Some(Target {
        disk,
        disk_name: name,
        ext,
        boot: prev.boot.wrapping_add(1),
        on_disk: prev.used,
        flushes: 0,
        failures: 0,
        header_stale: true,
    });
    say(format_args!(
        "log: POPCORN.LOG on {} (boot {}, {} KiB, lba {})",
        name,
        prev.boot.wrapping_add(1),
        ext.sectors as usize * SECTOR / 1024,
        ext.lba
    ));
    Ok(())
}

/// Turn the log off for good (this boot).
fn disable(st: &mut State, why: &'static str) {
    st.target = None;
    st.off = why;
}

/// Write the new text (and blank whatever the previous write left beyond it), then
/// the header. Returns the bytes of text now on disk.
pub fn flush() -> Result<usize, &'static str> {
    let mut st = STATE.lock();
    let r = write_out(&mut st, false);
    if let Err(why) = r {
        if st.target.is_none() {
            /* write_out turned the log off: say so once, after the lock is gone. */
            drop(st);
            say(format_args!("log: off ({})", why));
        }
    }
    r
}

fn write_out(st: &mut State, emergency: bool) -> Result<usize, &'static str> {
    let Some(t) = st.target.as_mut() else { return Err(st.off) };
    if let Err(why) = block::log_allowed(t.disk) {
        disable(st, why);
        return Err(why);
    }
    /* Snapshot. Bytes appended from here on are past `dirty_from` (= l0); a
     * backspace during the writes lowers `dirty_from` again, so nothing is lost. */
    let (d0, l0) = {
        let Some(mut b) = BUF.try_lock() else { return Err("log buffer busy") };
        let d = b.dirty_from;
        let l = b.len;
        b.dirty_from = l;
        (d, l)
    };
    let start = if t.on_disk > l0 { d0.min(l0) } else { d0 };
    let end = l0.max(t.on_disk);
    let header_only = start >= end;
    if header_only && !t.header_stale {
        return Ok(l0);
    }
    let restore = |d: usize| {
        if let Some(mut b) = BUF.try_lock() {
            b.dirty_from = b.dirty_from.min(d);
        }
    };
    let mut sec = [0u8; SECTOR];
    if !header_only {
        for i in (start / SECTOR)..end.div_ceil(SECTOR) {
            {
                let Some(b) = BUF.try_lock() else {
                    restore(d0);
                    return Err("log buffer busy");
                };
                for (j, out) in sec.iter_mut().enumerate() {
                    let off = i * SECTOR + j;
                    *out = if off < l0 { b.text[off] } else { pad_byte(SECTOR + off) };
                }
            }
            if block::log_write(t.disk, t.ext.lba + 1 + i as u64, &sec) < 0 {
                restore(d0);
                return fail(st, emergency);
            }
        }
    }
    t.on_disk = l0;
    t.header_stale = true;
    let hdr = fatlog::header(
        Header { boot: t.boot, used: l0 },
        t.ext.text_capacity(),
        DROPPED.load(Ordering::Relaxed),
        now_ms(),
    );
    if block::log_write(t.disk, t.ext.lba, &hdr) < 0 {
        return fail(st, emergency);
    }
    t.header_stale = false;
    t.flushes += 1;
    t.failures = 0;
    LAST_FLUSH_MS.store(now_ms(), Ordering::Relaxed);
    Ok(l0)
}

fn fail(st: &mut State, emergency: bool) -> Result<usize, &'static str> {
    if let Some(t) = st.target.as_mut() {
        t.failures += 1;
        if t.failures >= MAX_FAILURES && !emergency {
            disable(st, "writes to the boot stick keep failing");
            return Err("writes to the boot stick keep failing");
        }
    }
    Err("write to the boot stick failed")
}

/// Main loop: flush about once a second when there is new text.
pub fn tick() {
    let now = now_ms();
    if now.wrapping_sub(LAST_FLUSH_MS.load(Ordering::Relaxed)) < FLUSH_MS {
        return;
    }
    let dirty = BUF.try_lock().map_or(false, |b| b.dirty_from < b.len);
    if !dirty {
        return;
    }
    match STATE.try_lock() {
        Some(st) if st.target.is_some() => drop(st),
        _ => return,
    }
    LAST_FLUSH_MS.store(now, Ordering::Relaxed);
    let _ = flush();
}

/// Rust panic / CPU exception: one last write, interrupts off, nothing may wait.
/// Skipped when any lock on the path is held (the crash may be inside it).
pub fn emergency() {
    if EMERGENCY.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(mut st) = STATE.try_lock() else { return };
    let Some(disk) = st.target.as_ref().map(|t| t.disk) else { return };
    if !block::log_emergency_ready(disk) {
        return;
    }
    let _ = write_out(&mut st, true);
}

/// `log` / `log -info`.
pub fn info() {
    let (len, dirty, full) = {
        let b = BUF.lock();
        (b.len, b.dirty_from, b.full)
    };
    let dropped = DROPPED.load(Ordering::Relaxed);
    let st = STATE.lock();
    match st.target.as_ref() {
        Some(t) => {
            let (name, ext, boot, flushes, on_disk) = (t.disk_name, t.ext, t.boot, t.flushes, t.on_disk);
            drop(st);
            say(format_args!(
                "log: POPCORN.LOG on {} lba {}..{} ({} KiB), boot {}",
                name,
                ext.lba,
                ext.lba + ext.sectors - 1,
                ext.sectors as usize * SECTOR / 1024,
                boot
            ));
            say(format_args!(
                "log: {} bytes logged, {} on disk, {} unwritten, {} flushes, dropped {}{}",
                len,
                on_disk,
                len.saturating_sub(dirty),
                flushes,
                dropped,
                if full { " (full)" } else { "" }
            ));
        }
        None => {
            let off = st.off;
            drop(st);
            say(format_args!("log: off ({}); {} bytes kept in RAM, dropped {}", off, len, dropped));
        }
    }
}

/// `log -verify`: flush, then read the file back and compare it with RAM (text,
/// padding and header). Proves the bytes really are on the stick.
pub fn verify() -> Result<usize, &'static str> {
    flush()?;
    let st = STATE.lock();
    let t = st.target.as_ref().ok_or(st.off)?;
    let (disk, ext, boot, on_disk) = (t.disk, t.ext, t.boot, t.on_disk);
    drop(st);
    let mut sec = [0u8; SECTOR];
    if block::log_read(disk, ext.lba, &mut sec) < 0 {
        return Err("cannot read the header back");
    }
    let h = fatlog::parse_header(&sec, ext.text_capacity());
    if h.boot != boot || h.used != on_disk {
        return Err("header on disk does not match");
    }
    /* Check the text plus one sector of padding after it. */
    let end = (on_disk + SECTOR).min(ext.text_capacity());
    for i in 0..end.div_ceil(SECTOR) {
        if block::log_read(disk, ext.lba + 1 + i as u64, &mut sec) < 0 {
            return Err("read back failed");
        }
        let b = BUF.lock();
        for (j, &got) in sec.iter().enumerate() {
            let off = i * SECTOR + j;
            let want = if off < on_disk { b.text[off] } else { pad_byte(SECTOR + off) };
            if got != want {
                return Err("text on disk does not match RAM");
            }
        }
    }
    Ok(on_disk)
}

/* —— C FFI —— */

#[no_mangle]
pub extern "C" fn rust_klog_putc(c: u8) {
    putc(c);
}

#[no_mangle]
pub extern "C" fn rust_klog_tick() {
    tick();
}

#[no_mangle]
pub extern "C" fn rust_klog_flush() {
    let _ = flush();
}

#[no_mangle]
pub extern "C" fn rust_klog_emergency() {
    emergency();
}

#[no_mangle]
pub extern "C" fn rust_log_cmd(args: *const core::ffi::c_char) {
    let a = crate::cstr(args);
    match a.trim() {
        "" | "-info" => info(),
        "-flush" => match flush() {
            Ok(n) => say(format_args!("log: {} bytes on disk", n)),
            Err(e) => say(format_args!("log: flush failed: {}", e)),
        },
        "-verify" => match verify() {
            Ok(n) => say(format_args!("log: verify ok ({} bytes match the stick)", n)),
            Err(e) => say(format_args!("log: verify FAILED: {}", e)),
        },
        _ => {
            say(format_args!("log            where the log goes and how much is written"));
            say(format_args!("log -flush     write the log to POPCORN.LOG on the boot stick now"));
            say(format_args!("log -verify    flush, then read POPCORN.LOG back and compare"));
        }
    }
}
