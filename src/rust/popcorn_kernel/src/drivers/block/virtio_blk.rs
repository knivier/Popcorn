//! Legacy virtio-blk (PCI transitional) — up to 4 devices.
//! Slot N is always PCI virtio-blk #N; a device that fails to probe leaves its
//! slot empty (`capacity(N) == None`) so the other disks keep stable names.

use core::sync::atomic::Ordering;

use crate::drivers::bus::pci;
use crate::drivers::dma::{self, DmaBuffer};
use crate::drivers::io::{inb, inl, inw, outb, outl, outw};

extern "C" {
    fn alloc_pages(num_pages: usize, flags: u32) -> *mut u8;
    fn free_pages(p: *mut u8, num_pages: usize);
}
const MEM_ALLOC_ZERO: u32 = 0x01;
/// `MEM_ALLOC_DMA` in `src/includes/memory.h`: frames below 4 GiB.
const MEM_ALLOC_DMA: u32 = 0x02;

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
    /// One DMA page reused by every request: header, data, status (REQ_*_OFF).
    req: DmaBuffer,
    /// A request timed out: the device may still own descriptors 0-2 and the
    /// request page, so no further I/O is issued (same policy as nvme).
    dead: bool,
}

/* Request page layout (one request in flight at a time). */
const REQ_HDR_OFF: usize = 0;
const REQ_DATA_OFF: usize = 16;
const REQ_STATUS_OFF: usize = 16 + SECTOR_SIZE as usize;

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
    /* Check BAR0 first: a device we cannot drive is left with decode/BME off. */
    let iobase = pci::bar_io(addr.bus, addr.slot, addr.func, 0).ok_or("virtio BAR0 not I/O")?;
    pci::enable_bus_master(addr.bus, addr.slot, addr.func);

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
    /* Below 4 GiB like dma_alloc: the queue is device-visible memory. */
    let pages = unsafe { alloc_pages(npages, MEM_ALLOC_ZERO | MEM_ALLOC_DMA) };
    if pages.is_null() {
        io_write8(iobase, REG_STATUS, VIRTIO_STATUS_FAILED);
        return Err("virtio queue OOM");
    }
    let phys = dma::virt_to_phys(pages as u64);
    if (phys & 0xFFF) != 0 {
        io_write8(iobase, REG_STATUS, VIRTIO_STATUS_FAILED);
        unsafe { free_pages(pages, npages) };
        return Err("queue not page aligned");
    }
    /* Per-device request page, allocated before the device learns the queue. */
    let req = match dma::dma_alloc(4096) {
        Some(b) => b,
        None => {
            io_write8(iobase, REG_STATUS, VIRTIO_STATUS_FAILED);
            unsafe { free_pages(pages, npages) };
            return Err("virtio request buffer OOM");
        }
    };
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
            req,
            dead: false,
        });
    }
    Ok((capacity, SECTOR_SIZE))
}

/// Probe all legacy virtio-blk devices; returns how many came up.
/// Slots may be sparse (a bad device does not hide the ones after it), so
/// callers must walk `0..MAX_VIRTIO_DISKS` and check `capacity(i)`.
pub fn probe_all() -> usize {
    let mut n = 0usize;
    for i in 0..MAX_VIRTIO_DISKS {
        if pci::find_virtio_blk_n(i).is_none() {
            break; /* no more virtio-blk functions on the bus */
        }
        if probe_one(i, i).is_ok() {
            n += 1;
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
    if dev.dead {
        return -1;
    }
    if lba >= dev.capacity {
        return -2;
    }

    let req_virt = dev.req.virt as usize;
    let req_phys = dev.req.phys + REQ_HDR_OFF as u64;
    let data_phys = dev.req.phys + REQ_DATA_OFF as u64;
    let status_phys = dev.req.phys + REQ_STATUS_OFF as u64;
    let data_ptr = (req_virt + REQ_DATA_OFF) as *mut u8;
    let status_ptr = (req_virt + REQ_STATUS_OFF) as *mut u8;

    unsafe {
        core::ptr::write_volatile(
            (req_virt + REQ_HDR_OFF) as *mut BlkReqHdr,
            BlkReqHdr {
                type_: if is_write {
                    VIRTIO_BLK_T_OUT
                } else {
                    VIRTIO_BLK_T_IN
                },
                reserved: 0,
                sector: lba,
            },
        );
        if is_write {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), data_ptr, SECTOR_SIZE as usize);
        }
        core::ptr::write_volatile(status_ptr, 0xFF);
    }

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
            /* The device still owns descriptors 0-2 and may DMA into the request
             * page later. Reusing either would hand it a half-rewritten request
             * and shift every later completion by one, so stop all I/O. The
             * request page is never freed. */
            dev.dead = true;
            crate::console_ffi::println_color(
                "virtio-blk: request timeout, disk disabled",
                crate::console_ffi::COLOR_YELLOW,
            );
            return -1;
        }
    }

    /* Written by the device via DMA — read volatile, never cache. */
    if unsafe { core::ptr::read_volatile(status_ptr) } != 0 {
        return -1;
    }
    if !is_write {
        unsafe {
            core::ptr::copy_nonoverlapping(data_ptr, buf.as_mut_ptr(), SECTOR_SIZE as usize);
        }
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
