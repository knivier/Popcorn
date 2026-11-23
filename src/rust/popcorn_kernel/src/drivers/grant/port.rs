//! Port-I/O grant: a bounds-checked I/O port range.

use super::{violation, GrantError, GrantId};

/// Raw x86 port instructions. Visible to the grant layer and `bus::pci` (kernel-
/// owned config access) only; `drivers::io` keeps deprecated wrappers for the old
/// drivers that have not moved to grants yet.
pub(in crate::drivers) mod raw {
    #[inline]
    pub unsafe fn outb(port: u16, val: u8) {
        core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nostack, preserves_flags));
    }
    #[inline]
    pub unsafe fn inb(port: u16) -> u8 {
        let val: u8;
        core::arch::asm!("in al, dx", in("dx") port, out("al") val, options(nostack, preserves_flags));
        val
    }
    #[inline]
    pub unsafe fn outw(port: u16, val: u16) {
        core::arch::asm!("out dx, ax", in("dx") port, in("ax") val, options(nostack, preserves_flags));
    }
    #[inline]
    pub unsafe fn inw(port: u16) -> u16 {
        let val: u16;
        core::arch::asm!("in ax, dx", in("dx") port, out("ax") val, options(nostack, preserves_flags));
        val
    }
    #[inline]
    pub unsafe fn outl(port: u16, val: u32) {
        core::arch::asm!("out dx, eax", in("dx") port, in("eax") val, options(nostack, preserves_flags));
    }
    #[inline]
    pub unsafe fn inl(port: u16) -> u32 {
        let val: u32;
        core::arch::asm!("in eax, dx", in("dx") port, out("eax") val, options(nostack, preserves_flags));
        val
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PortRange {
    base: u16,
    len: u16,
    grant: GrantId,
}

impl PortRange {
    pub(in crate::drivers) fn new(base: u16, len: u16, grant: GrantId) -> PortRange {
        PortRange { base, len, grant }
    }

    #[inline]
    fn ok(&self, off: u16, width: u16) -> bool {
        off.checked_add(width).map_or(false, |end| end <= self.len)
    }

    #[inline]
    pub fn r8(&self, off: u16) -> u8 {
        if !self.ok(off, 1) {
            violation();
            return 0xFF;
        }
        unsafe { raw::inb(self.base + off) }
    }
    #[inline]
    pub fn w8(&self, off: u16, v: u8) {
        if !self.ok(off, 1) {
            return violation();
        }
        unsafe { raw::outb(self.base + off, v) }
    }
    #[inline]
    pub fn r16(&self, off: u16) -> u16 {
        if !self.ok(off, 2) {
            violation();
            return 0xFFFF;
        }
        unsafe { raw::inw(self.base + off) }
    }
    #[inline]
    pub fn w16(&self, off: u16, v: u16) {
        if !self.ok(off, 2) {
            return violation();
        }
        unsafe { raw::outw(self.base + off, v) }
    }
    #[inline]
    pub fn r32(&self, off: u16) -> u32 {
        if !self.ok(off, 4) {
            violation();
            return 0xFFFF_FFFF;
        }
        unsafe { raw::inl(self.base + off) }
    }
    #[inline]
    pub fn w32(&self, off: u16, v: u32) {
        if !self.ok(off, 4) {
            return violation();
        }
        unsafe { raw::outl(self.base + off, v) }
    }

    pub fn base(&self) -> u16 {
        self.base
    }
    pub fn len(&self) -> u16 {
        self.len
    }
    pub fn grant(&self) -> GrantId {
        self.grant
    }

    pub fn sub(&self, off: u16, len: u16) -> Result<PortRange, GrantError> {
        if len == 0 || off.checked_add(len).map_or(true, |end| end > self.len) {
            return Err(GrantError::OutOfRange);
        }
        Ok(PortRange { base: self.base + off, len, grant: self.grant })
    }
}
