//! DMA grant: pools of device-visible memory, carved from 64 KiB runs.
//!
//! * Runs come from `dma_alloc` (zeroed, contiguous, below 4 GiB). 64 KiB keeps
//!   `pmm_alloc_contig` happy (16 frames) and costs one kmalloc block per run.
//! * [`BufSlab`] slots are 2 KiB by default and never cross a page, so each buffer
//!   is one physically contiguous descriptor.
//! * x86 DMA is cache-coherent: `sync_for_device` / `sync_for_cpu` are no-ops that
//!   keep the API honest for a future IOMMU.
//! * Teardown: free a pool only after the device was reset or lost bus mastering;
//!   otherwise call [`DmaPool::poison`] and it is leaked on drop instead (the device
//!   may still DMA into it).
//!
//! Handles ([`Dma`], [`DmaArray`], [`BufSlab`]) hold raw pointers into the pool's
//! runs: the driver keeps the pool alive (and in the same lock) as long as them.

use core::marker::PhantomData;
use core::ptr::NonNull;

use super::{GrantError, GrantId};
use crate::drivers::dma::{dma_alloc, dma_free, DmaBuffer};
use crate::drivers::queue::DeviceAddr;

pub const RUN_BYTES: usize = 64 * 1024;
const MAX_RUNS: usize = 96;
const PAGE: usize = 4096;

/// Types whose every bit pattern is valid and that are safe to share with a device.
///
/// # Safety
/// `repr(C)`/primitive, no pointers, no padding the device could leak or rely on.
pub unsafe trait DmaSafe: Copy + 'static {}
unsafe impl DmaSafe for u8 {}
unsafe impl DmaSafe for u16 {}
unsafe impl DmaSafe for u32 {}
unsafe impl DmaSafe for u64 {}
unsafe impl<T: DmaSafe, const N: usize> DmaSafe for [T; N] {}

struct Run {
    buf: DmaBuffer,
    used: usize,
}

pub struct DmaPool {
    grant: GrantId,
    runs: [Option<Run>; MAX_RUNS],
    pages: u32,
    page_limit: u32,
    poisoned: bool,
}

// Safety: runs are plain memory owned by the pool; the driver serialises access.
unsafe impl Send for DmaPool {}

impl DmaPool {
    pub fn new(grant: GrantId, page_limit: u32) -> DmaPool {
        const NONE: Option<Run> = None;
        DmaPool { grant, runs: [NONE; MAX_RUNS], pages: 0, page_limit, poisoned: false }
    }

    fn new_run(&mut self) -> Result<usize, GrantError> {
        let slot = self.runs.iter().position(|r| r.is_none()).ok_or(GrantError::NoMemory)?;
        let pages = (RUN_BYTES / PAGE) as u32;
        if self.pages + pages > self.page_limit {
            return Err(GrantError::NoMemory);
        }
        let buf = dma_alloc(RUN_BYTES).ok_or(GrantError::NoMemory)?;
        self.pages += pages;
        self.runs[slot] = Some(Run { buf, used: 0 });
        Ok(slot)
    }

    /// Carve `len` zeroed bytes aligned to `align` (power of two, <= 4096).
    /// Allocations of a page or more start on a page boundary.
    fn carve(&mut self, len: usize, align: usize) -> Result<(NonNull<u8>, DeviceAddr), GrantError> {
        if len == 0 || len > RUN_BYTES || !align.is_power_of_two() || align > PAGE {
            return Err(GrantError::OutOfRange);
        }
        let align = if len >= PAGE { PAGE } else { align.max(8) };
        for pass in 0..2 {
            for r in self.runs.iter_mut().flatten() {
                let start = (r.used + align - 1) & !(align - 1);
                // Small objects must not straddle a page (one contiguous descriptor).
                let start = if len < PAGE && (start / PAGE) != ((start + len - 1) / PAGE) {
                    (start + PAGE - 1) & !(PAGE - 1)
                } else {
                    start
                };
                if start + len <= r.buf.len {
                    r.used = start + len;
                    let va = (r.buf.virt as usize + start) as *mut u8;
                    let dev = DeviceAddr(r.buf.phys + start as u64);
                    return Ok((NonNull::new(va).ok_or(GrantError::NoMemory)?, dev));
                }
            }
            if pass == 0 {
                self.new_run()?;
            }
        }
        Err(GrantError::NoMemory)
    }

    /// One zeroed `T`.
    pub fn alloc<T: DmaSafe>(&mut self) -> Result<Dma<T>, GrantError> {
        let (p, dev) = self.carve(core::mem::size_of::<T>(), core::mem::align_of::<T>())?;
        Ok(Dma { ptr: p.cast(), dev })
    }

    /// `n` zeroed `T`s, contiguous, aligned to `align`.
    pub fn alloc_array<T: DmaSafe>(&mut self, n: usize, align: usize) -> Result<DmaArray<T>, GrantError> {
        let bytes = n.checked_mul(core::mem::size_of::<T>()).ok_or(GrantError::OutOfRange)?;
        let (p, dev) = self.carve(bytes, align.max(core::mem::align_of::<T>()))?;
        Ok(DmaArray { ptr: p.cast(), dev, n })
    }

    /// `count` buffers of `slot` bytes (power of two, 64..=4096); none crosses a page.
    pub fn slab(&mut self, slot: usize, count: u16) -> Result<BufSlab, GrantError> {
        if !slot.is_power_of_two() || !(64..=PAGE).contains(&slot) || count == 0 {
            return Err(GrantError::OutOfRange);
        }
        let per_run = RUN_BYTES / slot;
        let runs_needed = (count as usize + per_run - 1) / per_run;
        if runs_needed > MAX_SLAB_RUNS {
            return Err(GrantError::OutOfRange);
        }
        let mut s = BufSlab { slot, per_run, count, virt: [0; MAX_SLAB_RUNS], dev: [DeviceAddr(0); MAX_SLAB_RUNS] };
        for i in 0..runs_needed {
            let (p, dev) = self.carve(RUN_BYTES, PAGE)?;
            s.virt[i] = p.as_ptr() as usize;
            s.dev[i] = dev;
        }
        Ok(s)
    }

    /// One zeroed, physically contiguous, page-aligned buffer of `len` bytes
    /// below 4 GiB, bigger than a run allows (firmware images: 512 KiB BHI /
    /// BHIe segments). Takes a run slot of its own; freed with the pool.
    pub fn alloc_large(&mut self, len: usize) -> Result<DmaArray<u8>, GrantError> {
        if len == 0 {
            return Err(GrantError::OutOfRange);
        }
        let slot = self.runs.iter().position(|r| r.is_none()).ok_or(GrantError::NoMemory)?;
        let pages = len.div_ceil(PAGE) as u32;
        if self.pages + pages > self.page_limit {
            return Err(GrantError::NoMemory);
        }
        let buf = dma_alloc(len).ok_or(GrantError::NoMemory)?;
        let ptr = NonNull::new(buf.virt as *mut u8).ok_or(GrantError::NoMemory)?;
        let dev = DeviceAddr(buf.phys);
        self.pages += pages;
        // `used` = the whole buffer, so `carve` never hands out parts of it.
        let used = buf.len;
        self.runs[slot] = Some(Run { buf, used });
        Ok(DmaArray { ptr, dev, n: len })
    }

    /// Like `alloc_large`, but the buffer starts on a multiple of `align` (a
    /// power of two). Over-allocates by `align` and uses the aligned part:
    /// devices that map host memory with large pages (Qualcomm WLFW firmware)
    /// expect what Linux's buddy allocator / IOMMU always give, a block
    /// aligned to its own (power-of-two) size.
    pub fn alloc_large_aligned(&mut self, len: usize, align: usize) -> Result<DmaArray<u8>, GrantError> {
        if len == 0 || !align.is_power_of_two() || align > (64 << 20) {
            return Err(GrantError::OutOfRange);
        }
        if align <= PAGE {
            return self.alloc_large(len);
        }
        let total = len.checked_add(align).ok_or(GrantError::OutOfRange)?;
        let slot = self.runs.iter().position(|r| r.is_none()).ok_or(GrantError::NoMemory)?;
        let pages = total.div_ceil(PAGE) as u32;
        if self.pages + pages > self.page_limit {
            return Err(GrantError::NoMemory);
        }
        let buf = dma_alloc(total).ok_or(GrantError::NoMemory)?;
        let off = ((buf.phys + align as u64 - 1) & !(align as u64 - 1)) - buf.phys;
        let ptr = NonNull::new((buf.virt + off) as *mut u8).ok_or(GrantError::NoMemory)?;
        let dev = DeviceAddr(buf.phys + off);
        self.pages += pages;
        let used = buf.len;
        self.runs[slot] = Some(Run { buf, used });
        Ok(DmaArray { ptr, dev, n: len })
    }

    /// The device may still DMA into this pool: leak it on drop instead of freeing.
    pub fn poison(&mut self) {
        self.poisoned = true;
    }

    pub fn outstanding_pages(&self) -> u32 {
        self.pages
    }

    pub fn grant(&self) -> GrantId {
        self.grant
    }
}

impl Drop for DmaPool {
    fn drop(&mut self) {
        if self.poisoned {
            return;
        }
        for r in self.runs.iter_mut() {
            if let Some(run) = r.take() {
                dma_free(run.buf);
            }
        }
        self.pages = 0;
    }
}

/// One device-shared `T`.
pub struct Dma<T: DmaSafe> {
    ptr: NonNull<T>,
    dev: DeviceAddr,
}

unsafe impl<T: DmaSafe> Send for Dma<T> {}

impl<T: DmaSafe> Dma<T> {
    #[inline]
    pub fn read(&self) -> T {
        unsafe { core::ptr::read_volatile(self.ptr.as_ptr()) }
    }
    #[inline]
    pub fn write(&self, v: T) {
        unsafe { core::ptr::write_volatile(self.ptr.as_ptr(), v) }
    }
    #[inline]
    pub fn dev(&self) -> DeviceAddr {
        self.dev
    }
    #[inline]
    pub fn as_ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }
}

/// `n` device-shared `T`s.
pub struct DmaArray<T: DmaSafe> {
    ptr: NonNull<T>,
    dev: DeviceAddr,
    n: usize,
}

unsafe impl<T: DmaSafe> Send for DmaArray<T> {}

impl<T: DmaSafe> DmaArray<T> {
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }
    #[inline]
    pub fn get(&self, i: usize) -> T {
        assert!(i < self.n);
        unsafe { core::ptr::read_volatile(self.ptr.as_ptr().add(i)) }
    }
    #[inline]
    pub fn set(&self, i: usize, v: T) {
        assert!(i < self.n);
        unsafe { core::ptr::write_volatile(self.ptr.as_ptr().add(i), v) }
    }
    #[inline]
    pub fn dev(&self) -> DeviceAddr {
        self.dev
    }
    #[inline]
    pub fn as_ptr(&self) -> *mut T {
        self.ptr.as_ptr()
    }
    #[inline]
    pub fn bytes(&self) -> usize {
        self.n * core::mem::size_of::<T>()
    }
}

const MAX_SLAB_RUNS: usize = 16;

/// Fixed-size I/O buffers addressed by a u16 id (no per-packet allocation).
pub struct BufSlab {
    slot: usize,
    per_run: usize,
    count: u16,
    virt: [usize; MAX_SLAB_RUNS],
    dev: [DeviceAddr; MAX_SLAB_RUNS],
}

unsafe impl Send for BufSlab {}

impl BufSlab {
    #[inline]
    pub fn count(&self) -> u16 {
        self.count
    }
    #[inline]
    pub fn slot_size(&self) -> usize {
        self.slot
    }
    #[inline]
    pub fn addr(&self, id: u16) -> DeviceAddr {
        let i = id as usize;
        assert!(id < self.count);
        self.dev[i / self.per_run].offset(((i % self.per_run) * self.slot) as u64)
    }
    #[inline]
    pub fn bytes(&self, id: u16) -> &[u8] {
        let i = id as usize;
        assert!(id < self.count);
        let va = self.virt[i / self.per_run] + (i % self.per_run) * self.slot;
        unsafe { core::slice::from_raw_parts(va as *const u8, self.slot) }
    }
    #[inline]
    pub fn bytes_mut(&mut self, id: u16) -> &mut [u8] {
        let i = id as usize;
        assert!(id < self.count);
        let va = self.virt[i / self.per_run] + (i % self.per_run) * self.slot;
        unsafe { core::slice::from_raw_parts_mut(va as *mut u8, self.slot) }
    }
}

/// No-ops on coherent x86; kept so drivers already call them where an IOMMU will need it.
#[inline]
pub fn sync_for_device() {}
#[inline]
pub fn sync_for_cpu() {}

#[allow(dead_code)]
struct _AssertSend(PhantomData<DmaPool>);
