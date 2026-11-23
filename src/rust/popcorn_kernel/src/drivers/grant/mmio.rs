//! MMIO grant: a bounds-checked, uncached window onto device registers.

use core::ptr::NonNull;

use super::{violation, GrantError, GrantId};
use crate::drivers::dma::map_mmio_uc;

/// `Send`, `!Sync`: the owning driver keeps it inside its own lock.
pub struct MmioRegion {
    base: NonNull<u8>,
    len: usize,
    phys: u64,
    grant: GrantId,
}

// Safety: the mapping is plain device memory; exclusive use is the owner's lock.
unsafe impl Send for MmioRegion {}

impl MmioRegion {
    /// Map `[phys, phys+len)` uncached. Grant-layer only (PCI BARs, LAPIC, ...).
    pub(in crate::drivers) fn map(phys: u64, len: usize, grant: GrantId) -> Result<MmioRegion, GrantError> {
        if len == 0 {
            return Err(GrantError::OutOfRange);
        }
        let va = map_mmio_uc(phys, len as u64).map_err(|_| GrantError::NoMemory)?;
        let base = NonNull::new(va as *mut u8).ok_or(GrantError::NoMemory)?;
        Ok(MmioRegion { base, len, phys, grant })
    }

    #[inline]
    fn ok(&self, off: usize, width: usize) -> bool {
        off.checked_add(width).map_or(false, |end| end <= self.len) && off % width == 0
    }

    #[inline]
    pub fn r8(&self, off: usize) -> u8 {
        if !self.ok(off, 1) {
            violation();
            return 0xFF;
        }
        unsafe { core::ptr::read_volatile(self.base.as_ptr().add(off)) }
    }
    #[inline]
    pub fn w8(&self, off: usize, v: u8) {
        if !self.ok(off, 1) {
            return violation();
        }
        unsafe { core::ptr::write_volatile(self.base.as_ptr().add(off), v) }
    }
    #[inline]
    pub fn r16(&self, off: usize) -> u16 {
        if !self.ok(off, 2) {
            violation();
            return 0xFFFF;
        }
        unsafe { core::ptr::read_volatile(self.base.as_ptr().add(off) as *const u16) }
    }
    #[inline]
    pub fn w16(&self, off: usize, v: u16) {
        if !self.ok(off, 2) {
            return violation();
        }
        unsafe { core::ptr::write_volatile(self.base.as_ptr().add(off) as *mut u16, v) }
    }
    #[inline]
    pub fn r32(&self, off: usize) -> u32 {
        if !self.ok(off, 4) {
            violation();
            return 0xFFFF_FFFF;
        }
        unsafe { core::ptr::read_volatile(self.base.as_ptr().add(off) as *const u32) }
    }
    #[inline]
    pub fn w32(&self, off: usize, v: u32) {
        if !self.ok(off, 4) {
            return violation();
        }
        unsafe { core::ptr::write_volatile(self.base.as_ptr().add(off) as *mut u32, v) }
    }
    /// 64-bit device field written as two 32-bit halves, low first (virtio).
    #[inline]
    pub fn w64_split(&self, off: usize, v: u64) {
        self.w32(off, v as u32);
        self.w32(off + 4, (v >> 32) as u32);
    }
    #[inline]
    pub fn r64_split(&self, off: usize) -> u64 {
        self.r32(off) as u64 | ((self.r32(off + 4) as u64) << 32)
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn phys(&self) -> u64 {
        self.phys
    }
    /// Kernel VA of the window (grant layer only: MSI-X entry masks).
    pub(in crate::drivers) fn va(&self) -> usize {
        self.base.as_ptr() as usize
    }
    pub fn grant(&self) -> GrantId {
        self.grant
    }

    /// Narrow to `[off, off+len)` (never widens).
    pub fn sub(&self, off: usize, len: usize) -> Result<MmioRegion, GrantError> {
        if len == 0 || off.checked_add(len).map_or(true, |end| end > self.len) {
            return Err(GrantError::OutOfRange);
        }
        let base = unsafe { NonNull::new_unchecked(self.base.as_ptr().add(off)) };
        Ok(MmioRegion { base, len, phys: self.phys + off as u64, grant: self.grant })
    }
}
