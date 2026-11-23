//! Hardware grants (driver-framework decision 1).
//!
//! Drivers touch hardware only through handles granted at probe: [`MmioRegion`],
//! [`PortRange`], [`IrqHandle`] and [`DmaPool`]. This module (with `bus::pci`) is
//! the only code with raw hardware access, so a driver can later move into a
//! Ring-3 realm by swapping these implementations for capability-backed ones.
//!
//! Old drivers (NVMe, xHCI, kbd, serial, legacy virtio-blk) still use raw port
//! I/O through `drivers::io`, which is deprecated; `scripts/lib/lint.sh` keeps new
//! code (`drivers/virtio`, `drivers/net`, `net/`) free of it.

pub mod dma;
pub mod irq;
pub mod mmio;
pub mod port;

pub use dma::{BufSlab, Dma, DmaArray, DmaPool, DmaSafe};
pub use irq::{IrqHandle, IrqSource};
pub use mmio::MmioRegion;
pub use port::PortRange;

use core::sync::atomic::{AtomicU16, AtomicU64, Ordering};

/// Identifies one probe's bundle of grants (diagnostics, ownership checks).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GrantId(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GrantError {
    OutOfRange,
    Busy,
    NoVectors,
    NoMemory,
    Unsupported,
}

impl GrantError {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantError::OutOfRange => "out of range",
            GrantError::Busy => "busy",
            GrantError::NoVectors => "no vectors",
            GrantError::NoMemory => "no memory",
            GrantError::Unsupported => "unsupported",
        }
    }
}

static NEXT_GRANT: AtomicU16 = AtomicU16::new(1);
/// Out-of-bounds MMIO/port accesses (dropped writes, all-ones reads). Never panics:
/// a panic during bring-up would look like a hang.
pub static GRANT_VIOLATIONS: AtomicU64 = AtomicU64::new(0);

pub fn new_grant_id() -> GrantId {
    GrantId(NEXT_GRANT.fetch_add(1, Ordering::Relaxed))
}

#[cold]
fn violation() {
    GRANT_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
}
