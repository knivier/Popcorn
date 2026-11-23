//! The transport trait both virtio PCI flavours implement.

use super::{DeviceType, VirtioError};
use crate::drivers::grant::{MmioRegion, PortRange};
use crate::drivers::queue::DeviceAddr;
use crate::drivers::virtio::virtqueue::Doorbell;

pub struct QueueAddrs {
    pub desc: DeviceAddr,
    pub driver: DeviceAddr,
    pub device: DeviceAddr,
}

/// A queue's precomputed doorbell.
pub enum Notifier {
    /// Modern: 16-bit write of the queue index at notify_base + off * multiplier.
    Mmio { region: MmioRegion },
    /// Legacy: 16-bit write of the queue index to register 16.
    Port { range: PortRange },
}

impl Doorbell for Notifier {
    #[inline]
    fn notify(&self, queue: u16) {
        match self {
            Notifier::Mmio { region } => region.w16(0, queue),
            Notifier::Port { range } => range.w16(0, queue),
        }
    }
}

pub trait Transport: Send {
    fn device_type(&self) -> DeviceType;
    fn is_modern(&self) -> bool;
    /// Write 0 and wait (bounded) until the device reads back 0.
    fn reset(&mut self) -> Result<(), VirtioError>;
    fn status(&self) -> u8;
    fn add_status(&mut self, bits: u8);
    /// Legacy: low 32 bits only.
    fn device_features(&mut self) -> u64;
    fn set_driver_features(&mut self, features: u64);
    /// 0 = no such queue.
    fn queue_max(&mut self, q: u16) -> u16;
    /// Legacy: `size` must equal `queue_max` and the rings must be the contiguous
    /// legacy layout starting at `addrs.desc`.
    fn setup_queue(&mut self, q: u16, size: u16, addrs: &QueueAddrs, vector: Option<u16>) -> Result<(), VirtioError>;
    /// Modern: queue_enable = 1. Legacy: no-op (writing the PFN enabled it).
    fn enable_queue(&mut self, q: u16);
    fn notifier(&self, q: u16) -> Result<Notifier, VirtioError>;
    /// Read device-specific config (modern: generation-counter retry loop).
    fn read_config(&self, off: usize, out: &mut [u8]);
}
