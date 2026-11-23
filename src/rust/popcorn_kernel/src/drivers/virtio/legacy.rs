//! Legacy (pre-1.0, transitional) virtio PCI transport over an I/O BAR.
//!
//! Registers: features 0, guest features 4, queue_pfn 8 (addr >> 12), queue_size
//! 12 (read-only), queue_select 14, queue_notify 16, status 18, isr 19, and only
//! while MSI-X is enabled: config_vector 20, queue_vector 22. Device config starts
//! at 20, or at **24 when MSI-X is enabled** (`msix_enabled` tracks it, so the
//! offset is never hard-coded like the old `REG_CONFIG = 20`).

use super::transport::{Notifier, QueueAddrs, Transport};
use super::{DeviceType, VirtioError};
use crate::drivers::grant::PortRange;

const REG_FEATURES: u16 = 0;
const REG_GUEST_FEATURES: u16 = 4;
const REG_QUEUE_PFN: u16 = 8;
const REG_QUEUE_SIZE: u16 = 12;
const REG_QUEUE_SELECT: u16 = 14;
const REG_QUEUE_NOTIFY: u16 = 16;
const REG_STATUS: u16 = 18;
const REG_CONFIG_VECTOR: u16 = 20;
const REG_QUEUE_VECTOR: u16 = 22;

pub struct LegacyTransport {
    io: PortRange,
    kind: DeviceType,
    msix_enabled: bool,
}

impl LegacyTransport {
    pub fn new(io: PortRange, kind: DeviceType) -> LegacyTransport {
        LegacyTransport { io, kind, msix_enabled: false }
    }

    /// Call after the grant layer enabled MSI-X on the function (moves config to 24).
    pub fn set_msix_enabled(&mut self, on: bool) {
        self.msix_enabled = on;
    }

    fn cfg_base(&self) -> u16 {
        if self.msix_enabled {
            24
        } else {
            20
        }
    }
}

impl Transport for LegacyTransport {
    fn device_type(&self) -> DeviceType {
        self.kind
    }
    fn is_modern(&self) -> bool {
        false
    }
    fn reset(&mut self) -> Result<(), VirtioError> {
        self.io.w8(REG_STATUS, 0);
        for _ in 0..1_000_000u32 {
            if self.io.r8(REG_STATUS) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(VirtioError::ResetTimeout)
    }
    fn status(&self) -> u8 {
        self.io.r8(REG_STATUS)
    }
    fn add_status(&mut self, bits: u8) {
        let s = self.io.r8(REG_STATUS);
        self.io.w8(REG_STATUS, s | bits);
    }
    fn device_features(&mut self) -> u64 {
        self.io.r32(REG_FEATURES) as u64
    }
    fn set_driver_features(&mut self, features: u64) {
        self.io.w32(REG_GUEST_FEATURES, features as u32);
    }
    fn queue_max(&mut self, q: u16) -> u16 {
        self.io.w16(REG_QUEUE_SELECT, q);
        self.io.r16(REG_QUEUE_SIZE)
    }
    fn setup_queue(&mut self, q: u16, size: u16, addrs: &QueueAddrs, vector: Option<u16>) -> Result<(), VirtioError> {
        self.io.w16(REG_QUEUE_SELECT, q);
        let max = self.io.r16(REG_QUEUE_SIZE);
        if max == 0 {
            return Err(VirtioError::QueueUnavailable);
        }
        if size != max {
            return Err(VirtioError::QueueTooSmall); // legacy queue size is fixed by the device
        }
        let pfn = addrs.desc.get() >> 12;
        if addrs.desc.get() & 0xFFF != 0 || pfn > u32::MAX as u64 {
            return Err(VirtioError::NoMemory);
        }
        if self.msix_enabled {
            let v = vector.unwrap_or(0xFFFF);
            self.io.w16(REG_QUEUE_VECTOR, v);
            if vector.is_some() && self.io.r16(REG_QUEUE_VECTOR) != v {
                return Err(VirtioError::VectorRejected);
            }
        }
        self.io.w32(REG_QUEUE_PFN, pfn as u32);
        Ok(())
    }
    fn enable_queue(&mut self, _q: u16) {}
    fn notifier(&self, _q: u16) -> Result<Notifier, VirtioError> {
        Ok(Notifier::Port { range: self.io.sub(REG_QUEUE_NOTIFY, 2).map_err(|_| VirtioError::Grant)? })
    }
    fn set_config_vector(&mut self, entry: Option<u16>) -> Result<(), VirtioError> {
        if !self.msix_enabled {
            return Ok(());
        }
        let val = entry.unwrap_or(0xFFFF);
        self.io.w16(REG_CONFIG_VECTOR, val);
        if entry.is_some() && self.io.r16(REG_CONFIG_VECTOR) != val {
            return Err(VirtioError::VectorRejected);
        }
        Ok(())
    }
    fn read_config(&self, off: usize, out: &mut [u8]) {
        let base = self.cfg_base();
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.io.r8(base + (off + i) as u16);
        }
    }
}

