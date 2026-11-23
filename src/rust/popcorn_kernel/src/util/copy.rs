//! Bulk copy / fill with `rep movsb` / `rep stosb`.
//!
//! The kernel crate is built without SSE, so ordinary slice copies become byte
//! loops; on modern CPUs (ERMSB) `rep movsb` is the fast path for kernel copies.

/// Copy `min(dst.len(), src.len())` bytes; returns the count.
#[inline]
pub fn copy(dst: &mut [u8], src: &[u8]) -> usize {
    let n = if dst.len() < src.len() { dst.len() } else { src.len() };
    if n == 0 {
        return 0;
    }
    // Safety: both ranges are valid for n bytes and cannot overlap (&mut vs &).
    unsafe {
        core::arch::asm!(
            "rep movsb",
            inout("rcx") n => _,
            inout("rdi") dst.as_mut_ptr() => _,
            inout("rsi") src.as_ptr() => _,
            options(nostack, preserves_flags)
        );
    }
    n
}

/// Fill `dst` with `byte`.
#[inline]
pub fn fill(dst: &mut [u8], byte: u8) {
    if dst.is_empty() {
        return;
    }
    unsafe {
        core::arch::asm!(
            "rep stosb",
            inout("rcx") dst.len() => _,
            inout("rdi") dst.as_mut_ptr() => _,
            in("al") byte,
            options(nostack, preserves_flags)
        );
    }
}
