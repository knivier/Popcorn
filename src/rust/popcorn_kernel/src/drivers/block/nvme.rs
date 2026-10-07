//! Minimal NVMe 1.x driver — polled, single namespace, one outstanding command.
//!
//! Scope:
//! - PCI class 0x01/0x08, BAR0 mapped through the kernel direct map
//! - Controller reset + admin queue pair, Identify Controller / Namespace 1
//! - One I/O queue pair (qid=1), Read/Write of exactly one logical block per call
//! - Completion by polling the CQ phase bit (interrupts masked, no MSI)
//!
//! Probe failures never panic: they return `Err(&'static str)` and release
//! every DMA allocation, leaving the controller disabled.

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{fence, AtomicBool, Ordering};

use crate::drivers::bus::pci;
use crate::drivers::dma::{self, DmaBuffer};

/* Controller registers (offsets from BAR0). */
const REG_CAP: u64 = 0x00;
const REG_INTMS: u64 = 0x0C;
const REG_CC: u64 = 0x14;
const REG_CSTS: u64 = 0x1C;
const REG_AQA: u64 = 0x24;
const REG_ASQ: u64 = 0x28;
const REG_ACQ: u64 = 0x30;
const REG_DOORBELL: u64 = 0x1000;

const CC_EN: u32 = 1;
const CC_IOSQES_64: u32 = 6 << 16; /* 2^6 = 64-byte SQE */
const CC_IOCQES_16: u32 = 4 << 20; /* 2^4 = 16-byte CQE */
const CSTS_RDY: u32 = 1;
const CSTS_CFS: u32 = 1 << 1;

/* Admin opcodes. */
const ADM_CREATE_IO_SQ: u32 = 0x01;
const ADM_CREATE_IO_CQ: u32 = 0x05;
const ADM_IDENTIFY: u32 = 0x06;

/* NVM command set opcodes. */
const NVM_WRITE: u32 = 0x01;
const NVM_READ: u32 = 0x02;

const QUEUE_DEPTH: u16 = 16;
const PAGE: usize = 4096;

/// Register-poll budget (each iteration is an MMIO read).
const SPIN_READY: u32 = 5_000_000;
/// Completion-poll budget.
const SPIN_CMD: u32 = 20_000_000;

/// Off at boot. Only `disk master <name> YES` may set this. Install/wipe/selftest never do.
static WRITE_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_write_enabled(on: bool) {
    WRITE_ENABLED.store(on, Ordering::Release);
}

struct Queue {
    sq: DmaBuffer,
    cq: DmaBuffer,
    qid: u16,
    depth: u16,
    sq_tail: u16,
    cq_head: u16,
    phase: bool,
    cid: u16,
}

impl Queue {
    fn new(qid: u16, depth: u16, sq: DmaBuffer, cq: DmaBuffer) -> Self {
        Queue {
            sq,
            cq,
            qid,
            depth,
            sq_tail: 0,
            cq_head: 0,
            phase: true, /* CQ is zeroed; first valid entry has P=1 */
            cid: 0,
        }
    }
}

struct Nvme {
    bar: u64,
    stride: u64,
    /// Kept alive so the admin SQ/CQ DMA pages stay owned by the device.
    _admin: Queue,
    io: Queue,
    data: DmaBuffer,
    nsid: u32,
    sectors: u64,
    block_size: u32,
    dead: bool,
}

static mut DEV: Option<Nvme> = None;

/* —— MMIO —— */

fn mmio_r32(bar: u64, off: u64) -> u32 {
    unsafe { read_volatile((bar + off) as *const u32) }
}
fn mmio_w32(bar: u64, off: u64, v: u32) {
    unsafe { write_volatile((bar + off) as *mut u32, v) }
}

fn wait_rdy(bar: u64, want: bool) -> Result<(), &'static str> {
    for _ in 0..SPIN_READY {
        let csts = mmio_r32(bar, REG_CSTS);
        if (csts & CSTS_RDY != 0) == want {
            return Ok(());
        }
        if want && (csts & CSTS_CFS) != 0 {
            return Err("nvme fatal status");
        }
        core::hint::spin_loop();
    }
    Err("nvme ready timeout")
}

fn disable(bar: u64) -> Result<(), &'static str> {
    let cc = mmio_r32(bar, REG_CC);
    mmio_w32(bar, REG_CC, cc & !CC_EN);
    wait_rdy(bar, false)
}

/* —— Queue submit / poll —— */

fn db_off(stride: u64, qid: u16, cq: bool) -> u64 {
    REG_DOORBELL + (2 * qid as u64 + cq as u64) * stride
}

/// Submit one command and busy-wait for its completion. Returns CQE dword 0.
fn submit(bar: u64, stride: u64, q: &mut Queue, mut cmd: [u32; 16]) -> Result<u32, &'static str> {
    q.cid = q.cid.wrapping_add(1);
    if q.cid == 0 {
        q.cid = 1;
    }
    let cid = q.cid;
    cmd[0] = (cmd[0] & 0xFFFF) | ((cid as u32) << 16);

    unsafe {
        let slot = (q.sq.virt as *mut u32).add(q.sq_tail as usize * 16);
        for (i, w) in cmd.iter().enumerate() {
            write_volatile(slot.add(i), *w);
        }
    }
    fence(Ordering::SeqCst);
    q.sq_tail = (q.sq_tail + 1) % q.depth;
    mmio_w32(bar, db_off(stride, q.qid, false), q.sq_tail as u32);

    let cqe = (q.cq.virt as *const u32).wrapping_add(q.cq_head as usize * 4);
    let want_phase = q.phase as u32;
    let mut dw3 = 0u32;
    let mut done = false;
    for _ in 0..SPIN_CMD {
        dw3 = unsafe { read_volatile(cqe.add(3)) };
        if (dw3 >> 16) & 1 == want_phase {
            done = true;
            break;
        }
        core::hint::spin_loop();
    }
    if !done {
        return Err("nvme command timeout");
    }
    fence(Ordering::SeqCst);
    let dw0 = unsafe { read_volatile(cqe) };

    q.cq_head += 1;
    if q.cq_head == q.depth {
        q.cq_head = 0;
        q.phase = !q.phase;
    }
    mmio_w32(bar, db_off(stride, q.qid, true), q.cq_head as u32);

    if (dw3 & 0xFFFF) as u16 != cid {
        return Err("nvme cid mismatch");
    }
    if (dw3 >> 17) & 0x7FFF != 0 {
        return Err("nvme command failed");
    }
    Ok(dw0)
}

fn identify(
    bar: u64,
    stride: u64,
    admin: &mut Queue,
    data: &DmaBuffer,
    cns: u32,
    nsid: u32,
) -> Result<(), &'static str> {
    unsafe { core::ptr::write_bytes(data.virt as *mut u8, 0, PAGE) };
    let mut c = [0u32; 16];
    c[0] = ADM_IDENTIFY;
    c[1] = nsid;
    c[6] = data.phys as u32;
    c[7] = (data.phys >> 32) as u32;
    c[10] = cns;
    submit(bar, stride, admin, c).map(|_| ())
}

fn rd_u32(b: u64, off: usize) -> u32 {
    unsafe { read_volatile((b + off as u64) as *const u32) }
}

/* —— Probe —— */

struct Bufs {
    asq: Option<DmaBuffer>,
    acq: Option<DmaBuffer>,
    isq: Option<DmaBuffer>,
    icq: Option<DmaBuffer>,
    data: Option<DmaBuffer>,
}

impl Bufs {
    fn free_all(&mut self) {
        for b in [
            self.asq.take(),
            self.acq.take(),
            self.isq.take(),
            self.icq.take(),
            self.data.take(),
        ]
        .into_iter()
        .flatten()
        {
            dma::dma_free(b);
        }
    }
}

/// Probe the first NVMe controller. Returns `(sectors, block_size)` for NSID 1.
pub fn probe() -> Result<(u64, u32), &'static str> {
    if unsafe { (*addr_of_mut!(DEV)).is_some() } {
        return Err("nvme already probed");
    }
    let addr = pci::find_class(0x01, 0x08, 0xFF).ok_or("no nvme controller")?;
    let (phys, size) = pci::bar_mem(addr.bus, addr.slot, addr.func, 0).ok_or("nvme BAR0 not mmio")?;
    if size < 0x1008 {
        return Err("nvme BAR0 too small");
    }
    pci::enable_bus_master(addr.bus, addr.slot, addr.func);
    let bar = dma::map_mmio(phys, size)?;

    let mut bufs = Bufs {
        asq: None,
        acq: None,
        isq: None,
        icq: None,
        data: None,
    };
    match probe_inner(bar, size, &mut bufs) {
        Ok(dev) => {
            let r = (dev.sectors, dev.block_size);
            unsafe { *addr_of_mut!(DEV) = Some(dev) };
            Ok(r)
        }
        Err(e) => {
            let _ = disable(bar);
            bufs.free_all();
            Err(e)
        }
    }
}

fn probe_inner(bar: u64, bar_size: u64, bufs: &mut Bufs) -> Result<Nvme, &'static str> {
    let cap_lo = mmio_r32(bar, REG_CAP);
    let cap_hi = mmio_r32(bar, REG_CAP + 4);
    if cap_lo == 0xFFFF_FFFF && cap_hi == 0xFFFF_FFFF {
        return Err("nvme BAR reads all-ones");
    }
    let mqes = (cap_lo & 0xFFFF) as u32 + 1;
    let dstrd = cap_hi & 0xF; /* CAP[35:32] */
    let css_nvm = (cap_hi >> 5) & 1; /* CAP[37] */
    let mpsmin = (cap_hi >> 16) & 0xF; /* CAP[51:48] */
    if css_nvm == 0 {
        return Err("nvme: NVM command set unsupported");
    }
    if mpsmin != 0 {
        return Err("nvme: min page size > 4K");
    }
    if mqes < 2 {
        return Err("nvme: queue size too small");
    }
    let depth = QUEUE_DEPTH.min(mqes.min(u16::MAX as u32) as u16);
    let stride = 4u64 << dstrd;
    /* Doorbells for qid 0 and 1 (SQ+CQ each) must lie inside BAR0. */
    if bar_size < REG_DOORBELL + 4 * stride {
        return Err("nvme BAR0 too small for doorbells");
    }

    /* Reset controller, mask pin interrupts (we poll). */
    disable(bar)?;
    mmio_w32(bar, REG_INTMS, 0xFFFF_FFFF);

    bufs.asq = Some(dma::dma_alloc(PAGE).ok_or("nvme OOM")?);
    bufs.acq = Some(dma::dma_alloc(PAGE).ok_or("nvme OOM")?);
    bufs.isq = Some(dma::dma_alloc(PAGE).ok_or("nvme OOM")?);
    bufs.icq = Some(dma::dma_alloc(PAGE).ok_or("nvme OOM")?);
    bufs.data = Some(dma::dma_alloc(PAGE).ok_or("nvme OOM")?);
    let (asq, acq, isq, icq, data) = (
        bufs.asq.unwrap(),
        bufs.acq.unwrap(),
        bufs.isq.unwrap(),
        bufs.icq.unwrap(),
        bufs.data.unwrap(),
    );
    for b in [&asq, &acq, &isq, &icq] {
        if b.phys & 0xFFF != 0 {
            return Err("nvme queue not page aligned");
        }
    }
    if data.phys & 0xFFF != 0 {
        return Err("nvme data buffer not page aligned");
    }

    /* Admin queues + enable. */
    let aqa = ((depth as u32 - 1) << 16) | (depth as u32 - 1);
    mmio_w32(bar, REG_AQA, aqa);
    mmio_w32(bar, REG_ASQ, asq.phys as u32);
    mmio_w32(bar, REG_ASQ + 4, (asq.phys >> 32) as u32);
    mmio_w32(bar, REG_ACQ, acq.phys as u32);
    mmio_w32(bar, REG_ACQ + 4, (acq.phys >> 32) as u32);
    fence(Ordering::SeqCst);
    mmio_w32(bar, REG_CC, CC_EN | CC_IOSQES_64 | CC_IOCQES_16); /* MPS=0 (4K), CSS=0, AMS=0 */
    wait_rdy(bar, true)?;

    let mut admin = Queue::new(0, depth, asq, acq);

    /* Identify controller (sanity) then namespace 1. */
    identify(bar, stride, &mut admin, &data, 1, 0)?;
    let nsid = 1u32;
    identify(bar, stride, &mut admin, &data, 0, nsid)?;

    let nsze = (rd_u32(data.virt, 0) as u64) | ((rd_u32(data.virt, 4) as u64) << 32);
    let nlbaf = unsafe { read_volatile((data.virt + 25) as *const u8) } as usize; /* 0-based */
    let flbas = unsafe { read_volatile((data.virt + 26) as *const u8) } as usize;
    let fmt = flbas & 0xF;
    if nsze == 0 {
        return Err("nvme namespace 1 empty");
    }
    if fmt > nlbaf.min(15) {
        return Err("nvme bad LBA format");
    }
    let lbaf = rd_u32(data.virt, 128 + fmt * 4);
    let lbads = (lbaf >> 16) & 0xFF;
    let ms = lbaf & 0xFFFF;
    if !(9..=12).contains(&lbads) {
        return Err("nvme unsupported LBA size");
    }
    if ms != 0 {
        return Err("nvme metadata formats unsupported");
    }
    let block_size = 1u32 << lbads;

    /* I/O CQ (qid 1) first, then SQ bound to it. */
    let mut c = [0u32; 16];
    c[0] = ADM_CREATE_IO_CQ;
    c[6] = icq.phys as u32;
    c[7] = (icq.phys >> 32) as u32;
    c[10] = ((depth as u32 - 1) << 16) | 1;
    c[11] = 1; /* PC=1, IEN=0 */
    submit(bar, stride, &mut admin, c)?;

    let mut c = [0u32; 16];
    c[0] = ADM_CREATE_IO_SQ;
    c[6] = isq.phys as u32;
    c[7] = (isq.phys >> 32) as u32;
    c[10] = ((depth as u32 - 1) << 16) | 1;
    c[11] = (1 << 16) | 1; /* CQID=1, PC=1, QPRIO=0 */
    submit(bar, stride, &mut admin, c)?;

    Ok(Nvme {
        bar,
        stride,
        _admin: admin,
        io: Queue::new(1, depth, isq, icq),
        data,
        nsid,
        sectors: nsze,
        block_size,
        dead: false,
    })
}

/* —— Public block API —— */

fn do_io(idx: usize, write: bool, lba: u64, buf: *mut u8, len: usize) -> i64 {
    if idx != 0 {
        return -1;
    }
    let dev = match unsafe { (*addr_of_mut!(DEV)).as_mut() } {
        Some(d) => d,
        None => return -1,
    };
    if dev.dead {
        return -1;
    }
    if write && !WRITE_ENABLED.load(Ordering::Acquire) {
        return -5;
    }
    let bs = dev.block_size as usize;
    if buf.is_null() || len < bs || lba >= dev.sectors {
        return -2;
    }

    if write {
        unsafe { core::ptr::copy_nonoverlapping(buf as *const u8, dev.data.virt as *mut u8, bs) };
    }
    fence(Ordering::SeqCst);

    let mut c = [0u32; 16];
    c[0] = if write { NVM_WRITE } else { NVM_READ };
    c[1] = dev.nsid;
    c[6] = dev.data.phys as u32;
    c[7] = (dev.data.phys >> 32) as u32;
    c[10] = lba as u32;
    c[11] = (lba >> 32) as u32;
    c[12] = 0; /* NLB = 1 block (0-based) */

    match submit(dev.bar, dev.stride, &mut dev.io, c) {
        Ok(_) => {}
        Err("nvme command timeout") => {
            dev.dead = true;
            return -1;
        }
        Err(_) => return -1,
    }

    if !write {
        fence(Ordering::SeqCst);
        unsafe { core::ptr::copy_nonoverlapping(dev.data.virt as *const u8, buf, bs) };
    }
    bs as i64
}

pub fn read(idx: usize, lba: u64, buf: &mut [u8]) -> i64 {
    do_io(idx, false, lba, buf.as_mut_ptr(), buf.len())
}

pub fn write(idx: usize, lba: u64, buf: &[u8]) -> i64 {
    do_io(idx, true, lba, buf.as_ptr() as *mut u8, buf.len())
}
