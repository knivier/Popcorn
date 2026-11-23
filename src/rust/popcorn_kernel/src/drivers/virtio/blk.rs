//! virtio-blk on the driver framework (virtio-net review §8 M4).
//!
//! * Slot N = the Nth transitional virtio-blk function (0x1001, subsystem 2), the
//!   same numbering as the old driver, so disk names (vda, ...) do not move. The
//!   modern-only 0x1042 test disk stays free for `virtio -probe`.
//! * Transport per device: modern (VERSION_1) when the virtio caps map, else the
//!   legacy I/O BAR (config offset 20, or 24 once MSI-X is enabled).
//! * One 256-entry `SplitQueue`; up to `MAX_INFLIGHT` requests in flight, each a
//!   header + up to `CHUNK_SECTORS` sectors through a 4 KiB bounce slot + status.
//!   Multi-sector reads/writes (`read_many` / `write_many`) keep several requests
//!   in flight (queue depth > 1).
//! * Timeouts are tick-based; on timeout the device is reset and its DMA pool
//!   poisoned (never freed while the device might still write it), and the disk
//!   is marked dead.
//! * MSI-X: entry 0 = config, entry 1 = requests. While waiting the task sleeps on
//!   `hlt` with the request interrupt armed; without MSI-X it polls.
//! * Features: accept VERSION_1 (modern), SEG_MAX, RO (honoured: writes fail),
//!   BLK_SIZE, FLUSH (used after install/wipe); refuse everything else.

use core::fmt::Write;

use super::pci::{open, AnyTransport, Prefer};
use super::transport::{Notifier, QueueAddrs};
use super::virtqueue::{legacy_layout, ring_bytes, Desc, RingMem, SplitQueue};
use super::{negotiate, STATUS_DRIVER_OK, STATUS_FAILED, F_VERSION_1};
use crate::drivers::bus::msix::{self, IrqSet};
use crate::drivers::bus::pci::{self, PciFunction, VENDOR_VIRTIO};
use crate::drivers::grant::{BufSlab, DmaArray, DmaPool, IrqHandle};
use crate::drivers::queue::{Budget, QStatus, Queue, Seg};
use crate::sync::TaskLock;

pub const SECTOR_SIZE: u32 = 512;
pub const MAX_VIRTIO_DISKS: usize = 4;

const QN: usize = 256;
const MAX_INFLIGHT: usize = 16;
const CHUNK_SECTORS: usize = 8;
const CHUNK_BYTES: usize = CHUNK_SECTORS * SECTOR_SIZE as usize;
const TIMEOUT_MS: u64 = 5000;

const F_BLK_SEG_MAX: u64 = 1 << 2;
const F_BLK_RO: u64 = 1 << 5;
const F_BLK_BLK_SIZE: u64 = 1 << 6;
const F_BLK_FLUSH: u64 = 1 << 9;

const T_IN: u32 = 0;
const T_OUT: u32 = 1;
const T_FLUSH: u32 = 4;

const TRANSITIONAL_BLK: [(u16, u16); 1] = [(VENDOR_VIRTIO, pci::DEV_VIRTIO_BLK_LEGACY)];

extern "C" {
    fn timer_get_uptime_ms() -> u64;
    fn timer_poll();
    fn timer_is_poll_mode() -> bool;
}

type BlkQueue = SplitQueue<u16, Notifier, QN>;

#[derive(Clone, Copy, Default)]
pub struct BlkStats {
    pub requests: u64,
    pub sectors: u64,
    pub max_depth: u16,
    pub errors: u64,
    pub irq_wakes: u64,
    pub flushes: u64,
}

struct Blk {
    pci: PciFunction,
    transport: AnyTransport,
    q: BlkQueue,
    hdrs: DmaArray<u8>,
    status: DmaArray<u8>,
    bounce: BufSlab,
    capacity: u64,
    read_only: bool,
    flush_ok: bool,
    seg_max: u32,
    blk_size: u32,
    acked: u64,
    irq: IrqHandle,
    _cfg_irq: IrqHandle,
    _irqs: IrqSet,
    dead: bool,
    stats: BlkStats,
    // Last: freed only after the device was reset (or leaked if poisoned).
    pool: DmaPool,
}

// Safety: owned device state, serialised by the per-slot TaskLock.
unsafe impl Send for Blk {}

/// One TaskLock per slot: held across the request wait (interrupts stay on).
/// Lock order: the block table lock (block/mod.rs) is never held while taking one.
static DEVS: [TaskLock<Option<Blk>>; MAX_VIRTIO_DISKS] =
    [TaskLock::new(None), TaskLock::new(None), TaskLock::new(None), TaskLock::new(None)];

fn now_ms() -> u64 {
    unsafe { timer_get_uptime_ms() }
}

fn com1(args: core::fmt::Arguments) {
    let mut out = crate::Com1;
    let _ = out.write_fmt(args);
}

/// Probe every transitional virtio-blk (slot N = Nth device). Returns how many came up.
pub fn probe_all() -> usize {
    let mut up = 0;
    for slot in 0..MAX_VIRTIO_DISKS {
        let Some(f) = pci::claim(&TRANSITIONAL_BLK, Some(2), slot) else { break };
        let a = f.addr();
        match bring_up(f) {
            Ok(dev) => {
                com1(format_args!(
                    "virtio-blk: slot {} {}:{}.{} {} capacity={} irq {}{}\n",
                    slot,
                    a.bus,
                    a.slot,
                    a.func,
                    dev.transport_name(),
                    dev.capacity,
                    if dev.irq.is_polled() { "poll" } else { "msix" },
                    if dev.read_only { " ro" } else { "" }
                ));
                *DEVS[slot].lock() = Some(dev);
                up += 1;
            }
            Err((f, why)) => {
                com1(format_args!("virtio-blk: slot {} probe failed: {}\n", slot, why));
                f.disable_bus_master();
                pci::release(f);
            }
        }
    }
    up
}

fn bring_up(f: PciFunction) -> Result<Blk, (PciFunction, &'static str)> {
    f.enable(true, true, false);
    let mut transport = match open(&f, Prefer::Auto) {
        Ok(t) => t,
        Err(e) => return Err((f, e.as_str())),
    };
    let modern = matches!(transport, AnyTransport::Modern(_));
    if let Err(e) = transport.as_dyn().reset() {
        return Err((f, e.as_str()));
    }
    f.enable(true, true, true);

    // MSI-X before any config read: on legacy it moves device config from 20 to 24.
    let mut irqs = msix::alloc_irqs(&f, 2, "virtio-blk").unwrap_or_else(|_| IrqSet::polled());
    if !irqs.is_polled() {
        if let AnyTransport::Legacy(l) = &mut transport {
            l.set_msix_enabled(true);
        }
    }
    let t = transport.as_dyn();

    let wanted = (if modern { F_VERSION_1 } else { 0 }) | F_BLK_SEG_MAX | F_BLK_RO | F_BLK_BLK_SIZE | F_BLK_FLUSH;
    let required = if modern { F_VERSION_1 } else { 0 };
    let acked = match negotiate(t, wanted, required) {
        Ok(a) => a,
        Err(e) => return Err((f, e.as_str())),
    };

    let mut entries = (None, None);
    if !irqs.is_polled() {
        if t.set_config_vector(Some(0)).is_ok() {
            entries = (Some(0u16), Some(1u16));
        } else {
            let _ = t.set_config_vector(None);
            if let AnyTransport::Legacy(l) = &mut transport {
                l.set_msix_enabled(false);
            }
            msix::disable(&f);
            irqs = IrqSet::polled();
        }
    }
    let t = transport.as_dyn();

    // Config: capacity le64 @0, seg_max le32 @12, blk_size le32 @20.
    let mut cfg = [0u8; 24];
    t.read_config(0, &mut cfg);
    let capacity = u64::from_le_bytes([cfg[0], cfg[1], cfg[2], cfg[3], cfg[4], cfg[5], cfg[6], cfg[7]]);
    let seg_max = if acked & F_BLK_SEG_MAX != 0 { u32::from_le_bytes([cfg[12], cfg[13], cfg[14], cfg[15]]) } else { 0 };
    let blk_size = if acked & F_BLK_BLK_SIZE != 0 { u32::from_le_bytes([cfg[20], cfg[21], cfg[22], cfg[23]]) } else { 512 };
    if capacity == 0 {
        t.add_status(STATUS_FAILED);
        return Err((f, "capacity 0"));
    }
    if seg_max != 0 && seg_max < 3 {
        // We use 3 descriptors per request (header, data, status) — always within
        // the 1-data-segment minimum, but refuse devices that report nonsense.
        t.add_status(STATUS_FAILED);
        return Err((f, "seg_max too small"));
    }

    let max = t.queue_max(0);
    if max == 0 {
        t.add_status(STATUS_FAILED);
        return Err((f, "no request queue"));
    }
    if (modern && (max as usize) < QN) || (!modern && max as usize != QN) {
        t.add_status(STATUS_FAILED);
        return Err((f, "queue size not 256"));
    }

    let mut pool = DmaPool::new(f.grant(), 64);
    // Modern: desc / avail / used each on their own page. Legacy: one contiguous
    // run in the fixed legacy layout (the device only learns the PFN of `desc`).
    let ring = if modern {
        let (db, ab, ub) = ring_bytes(QN);
        match (pool.alloc_array::<u8>(db, 4096), pool.alloc_array::<u8>(ab, 4096), pool.alloc_array::<u8>(ub, 4096)) {
            (Ok(d), Ok(av), Ok(u)) => RingMem {
                desc: d.as_ptr() as *mut Desc,
                avail: av.as_ptr(),
                used: u.as_ptr(),
                desc_dev: d.dev(),
                avail_dev: av.dev(),
                used_dev: u.dev(),
            },
            _ => return Err((f, "no DMA memory for rings")),
        }
    } else {
        let (avail_off, used_off, total) = legacy_layout(QN);
        let Ok(r) = pool.alloc_array::<u8>(total, 4096) else {
            return Err((f, "no DMA memory for rings"));
        };
        let base = r.as_ptr();
        let dev = r.dev();
        RingMem {
            desc: base as *mut Desc,
            avail: unsafe { base.add(avail_off) },
            used: unsafe { base.add(used_off) },
            desc_dev: dev,
            avail_dev: dev.offset(avail_off as u64),
            used_dev: dev.offset(used_off as u64),
        }
    };
    let (hdrs, status, bounce) = match (
        pool.alloc_array::<u8>(16 * MAX_INFLIGHT, 16),
        pool.alloc_array::<u8>(MAX_INFLIGHT, 16),
        pool.slab(CHUNK_BYTES, MAX_INFLIGHT as u16),
    ) {
        (Ok(h), Ok(s), Ok(b)) => (h, s, b),
        _ => return Err((f, "no DMA memory for requests")),
    };

    let addrs = QueueAddrs { desc: ring.desc_dev, driver: ring.avail_dev, device: ring.used_dev };
    if let Err(e) = t.setup_queue(0, QN as u16, &addrs, entries.1) {
        t.add_status(STATUS_FAILED);
        return Err((f, e.as_str()));
    }
    let bell = match t.notifier(0) {
        Ok(b) => b,
        Err(e) => {
            t.add_status(STATUS_FAILED);
            pool.poison();
            return Err((f, e.as_str()));
        }
    };
    t.enable_queue(0);
    let q: BlkQueue = SplitQueue::new(ring, bell, 0, false);
    t.add_status(STATUS_DRIVER_OK);

    let cfg_irq = irqs.take(0);
    let irq = irqs.take(1);
    Ok(Blk {
        pci: f,
        transport,
        q,
        hdrs,
        status,
        bounce,
        capacity,
        read_only: acked & F_BLK_RO != 0,
        flush_ok: acked & F_BLK_FLUSH != 0,
        seg_max,
        blk_size,
        acked,
        irq,
        _cfg_irq: cfg_irq,
        _irqs: irqs,
        dead: false,
        stats: BlkStats::default(),
        pool,
    })
}

impl Blk {
    fn transport_name(&self) -> &'static str {
        match self.transport {
            AnyTransport::Modern(_) => "modern",
            AnyTransport::Legacy(_) => "legacy",
        }
    }

    fn write_hdr(&self, slot: usize, kind: u32, sector: u64) {
        let b = slot * 16;
        for (i, v) in kind.to_le_bytes().iter().enumerate() {
            self.hdrs.set(b + i, *v);
        }
        for i in 4..8 {
            self.hdrs.set(b + i, 0);
        }
        for (i, v) in sector.to_le_bytes().iter().enumerate() {
            self.hdrs.set(b + 8 + i, *v);
        }
    }

    /// Wait for at least one completion. Sleeps on the request interrupt when there
    /// is one and interrupts are enabled; otherwise polls. false = timed out.
    fn wait_some(&mut self, deadline: u64) -> bool {
        loop {
            if self.q.pending() > 0 {
                return true;
            }
            if now_ms() >= deadline {
                return false;
            }
            if self.irq.is_polled() {
                // No interrupt source: poll (keeping the PIT alive in poll mode).
                if unsafe { timer_is_poll_mode() } {
                    unsafe { timer_poll() };
                }
                core::hint::spin_loop();
                continue;
            }
            let (irq, q) = (&self.irq, &mut self.q);
            crate::sync::wait_for_interrupt(|| {
                irq.unmask();
                q.arm()
            });
            self.irq.mask();
            self.q.disarm();
            if self.irq.take() {
                self.stats.irq_wakes += 1;
            }
        }
    }

    /// Reset the device and quarantine its memory after a lost request.
    fn fail_dead(&mut self) {
        self.dead = true;
        let _ = self.transport.as_dyn().reset();
        self.pool.poison();
        crate::console_ffi::println_color("virtio-blk: request timeout, device reset and disabled", crate::console_ffi::COLOR_YELLOW);
    }

    /// Read (`out = Some`) or write (`inp = Some`) `count` sectors from `lba`.
    fn rw(&mut self, lba: u64, count: usize, mut out: Option<&mut [u8]>, inp: Option<&[u8]>) -> i64 {
        if self.dead {
            return -1;
        }
        if count == 0 || lba.checked_add(count as u64).map_or(true, |end| end > self.capacity) {
            return -2;
        }
        let is_write = inp.is_some();
        if is_write && self.read_only {
            return -5;
        }
        let chunks = (count + CHUNK_SECTORS - 1) / CHUNK_SECTORS;
        let mut slot_chunk = [usize::MAX; MAX_INFLIGHT];
        let mut next = 0usize;
        let mut inflight = 0usize;
        let mut done = 0usize;
        let mut failed = false;
        while done < chunks {
            // Fill the queue with as many chunks as there are free request slots.
            let mut staged = 0;
            while next < chunks && inflight < MAX_INFLIGHT {
                let slot = slot_chunk.iter().position(|c| *c == usize::MAX).unwrap_or(0);
                let first = next * CHUNK_SECTORS;
                let n = (count - first).min(CHUNK_SECTORS);
                let bytes = n * SECTOR_SIZE as usize;
                self.write_hdr(slot, if is_write { T_OUT } else { T_IN }, lba + first as u64);
                if let Some(src) = inp {
                    let off = first * SECTOR_SIZE as usize;
                    crate::util::copy::copy(&mut self.bounce.bytes_mut(slot as u16)[..bytes], &src[off..off + bytes]);
                }
                self.status.set(slot, 0xFF);
                let segs = [
                    Seg { addr: self.hdrs.dev().offset((slot * 16) as u64), len: 16, device_writes: false },
                    Seg { addr: self.bounce.addr(slot as u16), len: bytes as u32, device_writes: !is_write },
                    Seg { addr: self.status.dev().offset(slot as u64), len: 1, device_writes: true },
                ];
                if self.q.submit(&segs, slot as u16).is_err() {
                    break;
                }
                slot_chunk[slot] = next;
                next += 1;
                inflight += 1;
                staged += 1;
            }
            if staged > 0 {
                self.q.kick();
                self.stats.max_depth = self.stats.max_depth.max(inflight as u16);
            }
            if !self.wait_some(now_ms() + TIMEOUT_MS) {
                self.fail_dead();
                return -1;
            }
            let status = &self.status;
            let bounce = &self.bounce;
            let mut finished = [usize::MAX; MAX_INFLIGHT];
            let mut nfin = 0;
            self.q.reap(Budget(MAX_INFLIGHT as u16), |c| {
                if c.status != QStatus::Ok {
                    failed = true;
                }
                if (c.cookie as usize) < MAX_INFLIGHT && nfin < finished.len() {
                    finished[nfin] = c.cookie as usize;
                    nfin += 1;
                }
            });
            for &slot in &finished[..nfin] {
                let chunk = slot_chunk[slot];
                slot_chunk[slot] = usize::MAX;
                inflight -= 1;
                done += 1;
                if chunk == usize::MAX {
                    failed = true;
                    continue;
                }
                if status.get(slot) != 0 {
                    failed = true;
                    continue;
                }
                if let Some(dst) = out.as_deref_mut() {
                    let first = chunk * CHUNK_SECTORS;
                    let n = (count - first).min(CHUNK_SECTORS);
                    let off = first * SECTOR_SIZE as usize;
                    let bytes = n * SECTOR_SIZE as usize;
                    crate::util::copy::copy(&mut dst[off..off + bytes], &bounce.bytes(slot as u16)[..bytes]);
                }
            }
        }
        self.stats.requests += chunks as u64;
        self.stats.sectors += count as u64;
        if failed {
            self.stats.errors += 1;
            return -1;
        }
        (count * SECTOR_SIZE as usize) as i64
    }

    fn flush(&mut self) -> i64 {
        if self.dead {
            return -1;
        }
        if !self.flush_ok {
            return 0; // write-through device: nothing to flush
        }
        let slot = 0usize;
        self.write_hdr(slot, T_FLUSH, 0);
        self.status.set(slot, 0xFF);
        let segs = [
            Seg { addr: self.hdrs.dev(), len: 16, device_writes: false },
            Seg { addr: self.status.dev(), len: 1, device_writes: true },
        ];
        if self.q.submit(&segs, slot as u16).is_err() {
            return -1;
        }
        self.q.kick();
        if !self.wait_some(now_ms() + TIMEOUT_MS) {
            self.fail_dead();
            return -1;
        }
        self.q.reap(Budget(4), |_| {});
        self.stats.flushes += 1;
        if self.status.get(slot) == 0 {
            0
        } else {
            -1
        }
    }
}

pub fn capacity(idx: usize) -> Option<(u64, u32)> {
    DEVS.get(idx)?.lock().as_ref().map(|d| (d.capacity, SECTOR_SIZE))
}

/// One sector (`buf` >= 512 bytes). Returns 512, or <0 (-2 bad args, -5 read-only).
pub fn read(idx: usize, lba: u64, buf: &mut [u8]) -> i64 {
    if buf.len() < SECTOR_SIZE as usize {
        return -2;
    }
    read_many(idx, lba, &mut buf[..SECTOR_SIZE as usize])
}

pub fn write(idx: usize, lba: u64, buf: &[u8]) -> i64 {
    if buf.len() < SECTOR_SIZE as usize {
        return -2;
    }
    write_many(idx, lba, &buf[..SECTOR_SIZE as usize])
}

/// `buf.len() / 512` sectors from `lba`, several requests in flight. Returns bytes.
pub fn read_many(idx: usize, lba: u64, buf: &mut [u8]) -> i64 {
    let count = buf.len() / SECTOR_SIZE as usize;
    let Some(slot) = DEVS.get(idx) else { return -1 };
    let mut g = slot.lock();
    match g.as_mut() {
        Some(d) => d.rw(lba, count, Some(buf), None),
        None => -1,
    }
}

pub fn write_many(idx: usize, lba: u64, buf: &[u8]) -> i64 {
    let count = buf.len() / SECTOR_SIZE as usize;
    let Some(slot) = DEVS.get(idx) else { return -1 };
    let mut g = slot.lock();
    match g.as_mut() {
        Some(d) => d.rw(lba, count, None, Some(buf)),
        None => -1,
    }
}

/// VIRTIO_BLK_T_FLUSH when the device offers it (after install / wipe).
pub fn flush(idx: usize) -> i64 {
    let Some(slot) = DEVS.get(idx) else { return -1 };
    let mut g = slot.lock();
    match g.as_mut() {
        Some(d) => d.flush(),
        None => -1,
    }
}

/// `virtio -blk -verify`: read `sectors` sectors from LBA 0 of `idx` with one
/// multi-request call (several requests in flight) and compare every byte with
/// single-sector reads. Read-only. Returns the queue depth the batch reached.
pub fn verify(idx: usize, sectors: usize) -> Result<u16, &'static str> {
    let bytes = sectors * SECTOR_SIZE as usize;
    let mut many = alloc::vec![0u8; bytes];
    let before = DEVS.get(idx).ok_or("no such slot")?.lock().as_ref().ok_or("slot empty")?.stats.max_depth;
    {
        let mut g = DEVS[idx].lock();
        let d = g.as_mut().ok_or("slot empty")?;
        d.stats.max_depth = 0;
    }
    if read_many(idx, 0, &mut many) != bytes as i64 {
        return Err("multi-sector read failed");
    }
    let depth = DEVS[idx].lock().as_ref().map_or(0, |d| d.stats.max_depth);
    {
        let mut g = DEVS[idx].lock();
        if let Some(d) = g.as_mut() {
            d.stats.max_depth = d.stats.max_depth.max(before);
        }
    }
    let mut one = [0u8; 512];
    for s in 0..sectors {
        if read(idx, s as u64, &mut one) != SECTOR_SIZE as i64 {
            return Err("single-sector read failed");
        }
        if one[..] != many[s * 512..(s + 1) * 512] {
            return Err("data mismatch between batched and single reads");
        }
    }
    Ok(depth)
}

/// `virtio -blk`: one line per bound disk.
pub fn info_lines(mut f: impl FnMut(&str)) {
    for (i, slot) in DEVS.iter().enumerate() {
        let g = slot.lock();
        let Some(d) = g.as_ref() else { continue };
        let a = d.pci.addr();
        let mut l = super::probe::LineBuf::new();
        let _ = write!(
            l,
            "virtio-blk slot {} @ {}:{}.{} {} irq {} cap {} ro {} flush {} seg_max {} blk {} feat {:#x} | req {} sect {} depth-max {} irq-wakes {} flushes {} err {}{}",
            i, a.bus, a.slot, a.func, d.transport_name(),
            if d.irq.is_polled() { "poll" } else { "msix" },
            d.capacity, d.read_only as u8, d.flush_ok as u8, d.seg_max, d.blk_size, d.acked,
            d.stats.requests, d.stats.sectors, d.stats.max_depth, d.stats.irq_wakes, d.stats.flushes, d.stats.errors,
            if d.dead { " DEAD" } else { "" }
        );
        f(l.as_str());
    }
}
