//! DMA helpers — page-backed buffers + phys translation via direct map.

use core::sync::atomic::{AtomicU64, Ordering};

extern "C" {
    fn alloc_pages(num_pages: usize, flags: u32) -> *mut u8;
    fn free_pages(p: *mut u8, num_pages: usize);
}
const MEM_ALLOC_ZERO: u32 = 0x01;

static NEXT_COOKIE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
pub struct DmaBuffer {
    pub cookie: u64,
    pub virt: u64,
    pub phys: u64,
    pub len: usize,
    pages: usize,
}

const DIRECT_MAP_BASE: u64 = 0xFFFF_8000_0000_0000;
const IDENTITY_BYTES: u64 = 64u64 << 30;

pub fn virt_to_phys(virt: u64) -> u64 {
    if virt >= DIRECT_MAP_BASE && virt < DIRECT_MAP_BASE + IDENTITY_BYTES {
        virt - DIRECT_MAP_BASE
    } else {
        virt
    }
}

pub fn phys_to_virt(phys: u64) -> u64 {
    DIRECT_MAP_BASE + phys
}

extern "C" {
    fn vmm_map_4k(pml4_phys: u64, vaddr: u64, paddr: u64, flags: u64) -> i32;
    fn vmm_get_cr3() -> u64;
}

const PTE_P_RW_UC: u64 = 1 | (1 << 1) | (1 << 3) | (1 << 4); /* P|RW|PWT|PCD */
/// High MMIO window: PML4[256] (shared by every address space), PDPT slots
/// 256..384 — above the 64 GiB direct map, so phys addresses past it (QEMU/OVMF
/// put 64-bit BARs at >= 512 GiB) can still be reached.
const MMIO_WIN_BASE: u64 = DIRECT_MAP_BASE + (256u64 << 30);
const MMIO_WIN_BYTES: u64 = 128u64 << 30;
static MMIO_NEXT: AtomicU64 = AtomicU64::new(0);

/// Return a kernel VA for the PCI MMIO range `[phys, phys+size)`.
/// Inside the 64 GiB direct map this is just `phys_to_virt`; above it the range
/// is mapped uncached into the high MMIO window (mappings are never torn down).
pub fn map_mmio(phys: u64, size: u64) -> Result<u64, &'static str> {
    if size == 0 || size > MMIO_WIN_BYTES {
        return Err("mmio size out of range");
    }
    let end = phys.checked_add(size).ok_or("mmio range overflow")?;
    if end <= IDENTITY_BYTES {
        return Ok(phys_to_virt(phys));
    }
    let off_in_page = phys & 0xFFF;
    let pages = (off_in_page + size + 0xFFF) >> 12;
    let bytes = pages << 12;
    let start = MMIO_NEXT.fetch_add(bytes, Ordering::Relaxed);
    if start + bytes > MMIO_WIN_BYTES {
        return Err("mmio window exhausted");
    }
    let cr3 = unsafe { vmm_get_cr3() } & 0x000f_ffff_ffff_f000;
    let pbase = phys & !0xFFF;
    for i in 0..pages {
        let va = MMIO_WIN_BASE + start + (i << 12);
        let rc = unsafe { vmm_map_4k(cr3, va, pbase + (i << 12), PTE_P_RW_UC) };
        if rc != 0 {
            return Err("mmio map failed");
        }
    }
    Ok(MMIO_WIN_BASE + start + off_in_page)
}

/// Allocate zeroed contiguous pages suitable for DMA (identity / low phys).
pub fn dma_alloc(len: usize) -> Option<DmaBuffer> {
    /* Reject 0 and anything absurd (also keeps `len + 4095` from overflowing). */
    if len == 0 || len > (256usize << 20) {
        return None;
    }
    let pages = (len + 4095) / 4096;
    let ptr = unsafe { alloc_pages(pages, MEM_ALLOC_ZERO) };
    if ptr.is_null() {
        return None;
    }
    let virt = ptr as u64;
    let phys = virt_to_phys(virt);
    Some(DmaBuffer {
        cookie: NEXT_COOKIE.fetch_add(1, Ordering::Relaxed),
        virt,
        phys,
        len: pages * 4096,
        pages,
    })
}

pub fn dma_free(buf: DmaBuffer) {
    if buf.virt != 0 {
        unsafe {
            free_pages(buf.virt as *mut u8, buf.pages);
        }
    }
}

pub fn dma_cookie() -> u64 {
    NEXT_COOKIE.fetch_add(1, Ordering::Relaxed)
}
