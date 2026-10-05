//! Legacy virtio-blk (PCI transitional) — up to 4 devices.

use alloc::boxed::Box;
use core::sync::atomic::Ordering;

use crate::drivers::bus::pci;
use crate::drivers::dma;
use crate::drivers::io::{inb, inl, inw, outb, outl, outw};

extern "C" {
    fn alloc_pages(num_pages: usize, flags: u32) -> *mut u8;
}
const MEM_ALLOC_ZERO: u32 = 0x01;

const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
const VIRTIO_STATUS_FAILED: u8 = 128;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;

pub const SECTOR_SIZE: u32 = 512;
const MAX_QUEUE: u16 = 256;
pub const MAX_VIRTIO_DISKS: usize = 4;

const REG_DEVICE_FEATURES: u16 = 0;
const REG_GUEST_FEATURES: u16 = 4;
const REG_QUEUE_PFN: u16 = 8;
const REG_QUEUE_SIZE: u16 = 12;
const REG_QUEUE_SEL: u16 = 14;
const REG_QUEUE_NOTIFY: u16 = 16;
const REG_STATUS: u16 = 18;
const REG_ISR: u16 = 19;
const REG_CONFIG: u16 = 20;

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;

#[repr(C)]
struct BlkReqHdr {
    type_: u32,
    reserved: u32,
    sector: u64,
}

struct VirtioBlk {
    iobase: u16,
    capacity: u64,
    queue_size: u16,
    queue_pages: *mut u8,
    desc_off: usize,
    avail_off: usize,
    used_off: usize,
    avail_idx: u16,
    last_used_idx: u16,
}

static mut DEVS: [Option<VirtioBlk>; MAX_VIRTIO_DISKS] = [None, None, None, None];

fn io_read32(base: u16, off: u16) -> u32 {
    unsafe { inl(base + off) }
}
fn io_write32(base: u16, off: u16, v: u32) {
    unsafe { outl(base + off, v) }
}
fn io_read16(base: u16, off: u16) -> u16 {
    unsafe { inw(base + off) }
}
fn io_write16(base: u16, off: u16, v: u16) {
    unsafe { outw(base + off, v) }
}
fn io_read8(base: u16, off: u16) -> u8 {
    unsafe { inb(base + off) }
}
fn io_write8(base: u16, off: u16, v: u8) {
    unsafe { outb(base + off, v) }
}

fn align_up(n: usize, a: usize) -> usize {
    (n + a - 1) & !(a - 1)
}

fn layout(qsz: u16) -> (usize, usize, usize) {
    let q = qsz as usize;
    let desc = core::mem::size_of::<VirtqDesc>() * q;
    let avail = 4 + 2 * q + 2;
    let used = 4 + 8 * q + 2;
    let avail_off = desc;
    let used_off = align_up(desc + avail, 4096);
    let total = used_off + align_up(used, 4096);
    (avail_off, used_off, total)
}

fn probe_one(pci_n: usize, slot: usize) -> Result<(u64, u32), &'static str> {
    let addr = pci::find_virtio_blk_n(pci_n).ok_or("no virtio-blk")?;
    pci::enable_bus_master(addr.bus, addr.slot, addr.func);
    let iobase = pci::bar_io(addr.bus, addr.slot, addr.func, 0).ok_or("virtio BAR0 not I/O")?;

    io_write8(iobase, REG_STATUS, 0);
    io_write8(iobase, REG_STATUS, VIRTIO_STATUS_ACKNOWLEDGE);
    io_write8(iobase, REG_STATUS, VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);
    let _host = io_read32(iobase, REG_DEVICE_FEATURES);
    io_write32(iobase, REG_GUEST_FEATURES, 0);

    let capacity = (io_read32(iobase, REG_CONFIG) as u64)
        | ((io_read32(iobase, REG_CONFIG + 4) as u64) << 32);
    if capacity == 0 {
        io_write8(iobase, REG_STATUS, VIRTIO_STATUS_FAILED);
        return Err("virtio capacity 0");
    }

    io_write16(iobase, REG_QUEUE_SEL, 0);
    let qsz = io_read16(iobase, REG_QUEUE_SIZE);
    if qsz == 0 || qsz > MAX_QUEUE {
        io_write8(iobase, REG_STATUS, VIRTIO_STATUS_FAILED);
        return Err("virtio queue size unsupported");
    }

    let (avail_off, used_off, nbytes) = layout(qsz);
    let npages = (nbytes + 4095) / 4096;
    let pages = unsafe { alloc_pages(npages, MEM_ALLOC_ZERO) };
    if pages.is_null() {
        io_write8(iobase, REG_STATUS, VIRTIO_STATUS_FAILED);
        return Err("virtio queue OOM");
    }
    let phys = dma::virt_to_phys(pages as u64);
    if (phys & 0xFFF) != 0 {
        return Err("queue not page aligned");
    }
    io_write32(iobase, REG_QUEUE_PFN, (phys >> 12) as u32);
    io_write8(
        iobase,
        REG_STATUS,
        VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK,
    );

    unsafe {
        DEVS[slot] = Some(VirtioBlk {
            iobase,
            capacity,
            queue_size: qsz,
            queue_pages: pages,
            desc_off: 0,
            avail_off,
            used_off,
            avail_idx: 0,
            last_used_idx: 0,
        });
    }
    Ok((capacity, SECTOR_SIZE))
}

/// Probe all legacy virtio-blk devices; returns count.
pub fn probe_all() -> usize {
    let mut n = 0usize;
    for i in 0..MAX_VIRTIO_DISKS {
        if probe_one(i, i).is_ok() {
            n += 1;
        } else {
            break;
        }
    }
    n
}

pub fn capacity(idx: usize) -> Option<(u64, u32)> {
    unsafe { DEVS.get(idx)?.as_ref().map(|d| (d.capacity, SECTOR_SIZE)) }
}

fn do_io(idx: usize, is_write: bool, lba: u64, buf: &mut [u8]) -> i64 {
    if buf.len() < SECTOR_SIZE as usize {
        return -2;
    }
    let dev = unsafe {
        match DEVS.get_mut(idx).and_then(|s| s.as_mut()) {
            Some(d) => d,
            None => return -1,
        }
    };
    if lba >= dev.capacity {
        return -2;
    }

    #[repr(C)]
    struct ReqBuf {
        hdr: BlkReqHdr,
        data: [u8; 512],
        status: u8,
    }
    let mut req = Box::new(ReqBuf {
        hdr: BlkReqHdr {
            type_: if is_write {
                VIRTIO_BLK_T_OUT
            } else {
                VIRTIO_BLK_T_IN
            },
            reserved: 0,
            sector: lba,
        },
        data: [0u8; 512],
        status: 0xFF,
    });
    if is_write {
        req.data.copy_from_slice(&buf[..512]);
    }

    let req_phys = dma::virt_to_phys(req.as_ref() as *const ReqBuf as u64);
    let data_phys = req_phys + 16;
    let status_phys = data_phys + 512;

    unsafe {
        let base = dev.queue_pages as usize;
        let desc = (base + dev.desc_off) as *mut VirtqDesc;
        (*desc.add(0)).addr = req_phys;
        (*desc.add(0)).len = 16;
        (*desc.add(0)).flags = VIRTQ_DESC_F_NEXT;
        (*desc.add(0)).next = 1;
        (*desc.add(1)).addr = data_phys;
        (*desc.add(1)).len = 512;
        (*desc.add(1)).flags = VIRTQ_DESC_F_NEXT
            | if is_write {
                0
            } else {
                VIRTQ_DESC_F_WRITE
            };
        (*desc.add(1)).next = 2;
        (*desc.add(2)).addr = status_phys;
        (*desc.add(2)).len = 1;
        (*desc.add(2)).flags = VIRTQ_DESC_F_WRITE;
        (*desc.add(2)).next = 0;

        let avail = (base + dev.avail_off) as *mut u16;
        let slot = (dev.avail_idx % dev.queue_size) as usize;
        *avail.add(2 + slot) = 0;
        core::sync::atomic::fence(Ordering::SeqCst);
        dev.avail_idx = dev.avail_idx.wrapping_add(1);
        *avail.add(1) = dev.avail_idx;
        core::sync::atomic::fence(Ordering::SeqCst);
        io_write16(dev.iobase, REG_QUEUE_NOTIFY, 0);

        let used = (base + dev.used_off) as *mut u16;
        let mut done = false;
        for _ in 0..10_000_000u32 {
            core::sync::atomic::fence(Ordering::SeqCst);
            let uidx = core::ptr::read_volatile(used.add(1));
            if uidx != dev.last_used_idx {
                dev.last_used_idx = uidx;
                let _ = io_read8(dev.iobase, REG_ISR);
                done = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !done {
            /* The device may still DMA into `req` later: leak it rather than
             * hand the page back to the allocator. */
            core::mem::forget(req);
            return -1;
        }
    }

    /* Written by the device via DMA — read volatile, never cache. */
    if unsafe { core::ptr::read_volatile(&req.status) } != 0 {
        return -1;
    }
    if !is_write {
        buf[..512].copy_from_slice(&req.data);
    }
    SECTOR_SIZE as i64
}

pub fn read(idx: usize, lba: u64, buf: &mut [u8]) -> i64 {
    do_io(idx, false, lba, buf)
}

pub fn write(idx: usize, lba: u64, buf: &[u8]) -> i64 {
    /* A short buffer used to panic (slice length mismatch) = kernel hang. */
    if buf.len() < SECTOR_SIZE as usize {
        return -2;
    }
    let mut tmp = [0u8; 512];
    tmp.copy_from_slice(&buf[..512]);
    do_io(idx, true, lba, &mut tmp)
}
