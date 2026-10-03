//! DMA helper stubs — reserved for virtio / AHCI (Phase 4+).
//!
//! No hardware programming here yet; this module exists so block drivers have a
//! stable place for bounce buffers and physical address helpers.

use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_COOKIE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
pub struct DmaBuffer {
    pub cookie: u64,
    pub phys: u64,
    pub len: usize,
}

/// Allocate a DMA-capable buffer description (identity-mapped for now).
/// Returns None until a real DMA pool exists.
pub fn dma_alloc(_len: usize) -> Option<DmaBuffer> {
    None
}

pub fn dma_free(_buf: DmaBuffer) {}

/// Mint an opaque cookie for future mapping tables.
pub fn dma_cookie() -> u64 {
    NEXT_COOKIE.fetch_add(1, Ordering::Relaxed)
}

/// Phys = virt for early identity / high-half direct map (placeholder).
pub fn virt_to_phys(virt: u64) -> u64 {
    virt
}
