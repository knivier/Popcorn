//! Shared virtio layer: transports (modern MMIO, legacy PIO), feature negotiation
//! and the split virtqueue. Used by virtio-net from M1 and virtio-blk from M4
//! (`block/virtio_blk.rs` keeps its own legacy code until then).
//!
//! Lint: no raw port I/O, `phys_to_virt` or `map_mmio` in this directory; all
//! hardware access goes through grants (`scripts/lib/lint.sh`).

pub mod legacy;
pub mod modern;
pub mod net;
pub mod pci;
pub mod probe;
pub mod transport;
pub mod virtqueue;

pub use transport::{Notifier, QueueAddrs, Transport};

pub const STATUS_ACKNOWLEDGE: u8 = 1;
pub const STATUS_DRIVER: u8 = 2;
pub const STATUS_DRIVER_OK: u8 = 4;
pub const STATUS_FEATURES_OK: u8 = 8;
pub const STATUS_NEEDS_RESET: u8 = 64;
pub const STATUS_FAILED: u8 = 128;

/// Reserved (device-independent) feature bits.
pub const F_RING_INDIRECT_DESC: u64 = 1 << 28;
pub const F_RING_EVENT_IDX: u64 = 1 << 29;
pub const F_VERSION_1: u64 = 1 << 32;
pub const F_ACCESS_PLATFORM: u64 = 1 << 33;
pub const F_RING_PACKED: u64 = 1 << 34;
pub const F_IN_ORDER: u64 = 1 << 35;
pub const F_ORDER_PLATFORM: u64 = 1 << 36;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceType {
    Net,
    Block,
    Other(u16),
}

impl DeviceType {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceType::Net => "net",
            DeviceType::Block => "blk",
            DeviceType::Other(_) => "other",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VirtioError {
    NoDevice,
    BadCaps,
    Grant,
    ResetTimeout,
    FeaturesRejected,
    MissingFeature,
    QueueUnavailable,
    QueueTooSmall,
    VectorRejected,
    NoMemory,
}

impl VirtioError {
    pub fn as_str(self) -> &'static str {
        match self {
            VirtioError::NoDevice => "no device",
            VirtioError::BadCaps => "bad capabilities",
            VirtioError::Grant => "grant failed",
            VirtioError::ResetTimeout => "reset timeout",
            VirtioError::FeaturesRejected => "FEATURES_OK not accepted",
            VirtioError::MissingFeature => "required feature missing",
            VirtioError::QueueUnavailable => "queue unavailable",
            VirtioError::QueueTooSmall => "queue too small",
            VirtioError::VectorRejected => "MSI-X vector rejected",
            VirtioError::NoMemory => "no DMA memory",
        }
    }
}

/// reset → ACKNOWLEDGE → DRIVER → features = device ∩ wanted (must include
/// `required`) → write → modern: FEATURES_OK must stick. Returns the acked set.
/// On failure the device is left with FAILED set.
pub fn negotiate(t: &mut dyn Transport, wanted: u64, required: u64) -> Result<u64, VirtioError> {
    t.reset()?;
    t.add_status(STATUS_ACKNOWLEDGE);
    t.add_status(STATUS_DRIVER);
    let offered = t.device_features();
    let acked = offered & wanted;
    if acked & required != required {
        t.add_status(STATUS_FAILED);
        return Err(VirtioError::MissingFeature);
    }
    t.set_driver_features(acked);
    if t.is_modern() {
        t.add_status(STATUS_FEATURES_OK);
        if t.status() & STATUS_FEATURES_OK == 0 {
            t.add_status(STATUS_FAILED);
            return Err(VirtioError::FeaturesRejected);
        }
    }
    Ok(acked)
}
