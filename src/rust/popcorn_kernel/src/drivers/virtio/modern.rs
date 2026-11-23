//! Modern (VERSION_1) virtio PCI transport over memory BARs (VIRTIO 1.2 §4.1.4).
//!
//! Vendor capabilities (ID 0x09) locate the COMMON, NOTIFY and DEVICE windows; each
//! is mapped uncached as a narrowed `MmioRegion` of its BAR.

use super::transport::{Notifier, QueueAddrs, Transport};
use super::{DeviceType, VirtioError};
use crate::drivers::bus::pci::{Bar, PciFunction, CAP_VENDOR};
use crate::drivers::grant::MmioRegion;

// virtio_pci_common_cfg offsets
const DEVICE_FEATURE_SELECT: usize = 0x00;
const DEVICE_FEATURE: usize = 0x04;
const DRIVER_FEATURE_SELECT: usize = 0x08;
const DRIVER_FEATURE: usize = 0x0C;
#[allow(dead_code)]
const CONFIG_MSIX_VECTOR: usize = 0x10;
const NUM_QUEUES: usize = 0x12;
const DEVICE_STATUS: usize = 0x14;
const CONFIG_GENERATION: usize = 0x15;
const QUEUE_SELECT: usize = 0x16;
const QUEUE_SIZE: usize = 0x18;
const QUEUE_MSIX_VECTOR: usize = 0x1A;
const QUEUE_ENABLE: usize = 0x1C;
const QUEUE_NOTIFY_OFF: usize = 0x1E;
const QUEUE_DESC: usize = 0x20;
const QUEUE_DRIVER: usize = 0x28;
const QUEUE_DEVICE: usize = 0x30;

const CFG_COMMON: u8 = 1;
const CFG_NOTIFY: u8 = 2;
const CFG_DEVICE: u8 = 4;

pub struct ModernTransport {
    common: MmioRegion,
    notify: MmioRegion,
    notify_mult: u32,
    device: Option<MmioRegion>,
    kind: DeviceType,
}

/// One parsed `virtio_pci_cap`.
#[derive(Clone, Copy)]
struct Cap {
    cfg_type: u8,
    bar: u8,
    offset: u32,
    length: u32,
    cap_off: u8,
}

impl ModernTransport {
    /// Map the COMMON / NOTIFY / DEVICE windows of `pci` (first cap of each type that
    /// sits in a memory BAR). Errors if COMMON or NOTIFY is missing.
    pub fn open(pci: &PciFunction, kind: DeviceType) -> Result<ModernTransport, VirtioError> {
        let mut caps: [Option<Cap>; 8] = [None; 8];
        let mut n = 0usize;
        pci.caps(|id, off| {
            if id == CAP_VENDOR && n < caps.len() {
                let raw = pci.vendor_cap(off);
                caps[n] = Some(Cap {
                    cfg_type: raw[3],
                    bar: raw[4],
                    offset: u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]),
                    length: u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]),
                    cap_off: off,
                });
                n += 1;
            }
        });
        let pick = |t: u8| caps.iter().flatten().copied().find(|c| c.cfg_type == t && c.bar <= 5);
        let common_cap = pick(CFG_COMMON).ok_or(VirtioError::BadCaps)?;
        let notify_cap = pick(CFG_NOTIFY).ok_or(VirtioError::BadCaps)?;
        let device_cap = pick(CFG_DEVICE);

        let window = |c: Cap| -> Result<MmioRegion, VirtioError> {
            match pci.bar(c.bar).map_err(|_| VirtioError::Grant)? {
                Bar::Mmio(r) => r.sub(c.offset as usize, c.length as usize).map_err(|_| VirtioError::BadCaps),
                Bar::Io(_) => Err(VirtioError::BadCaps),
            }
        };
        let common = window(common_cap)?;
        let notify = window(notify_cap)?;
        let notify_mult = pci.cfg_r32(notify_cap.cap_off + 16);
        let device = match device_cap {
            Some(c) => Some(window(c)?),
            None => None,
        };
        Ok(ModernTransport { common, notify, notify_mult, device, kind })
    }

    pub fn num_queues(&self) -> u16 {
        self.common.r16(NUM_QUEUES)
    }
}

impl Transport for ModernTransport {
    fn device_type(&self) -> DeviceType {
        self.kind
    }
    fn is_modern(&self) -> bool {
        true
    }
    fn reset(&mut self) -> Result<(), VirtioError> {
        self.common.w8(DEVICE_STATUS, 0);
        for _ in 0..1_000_000u32 {
            if self.common.r8(DEVICE_STATUS) == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(VirtioError::ResetTimeout)
    }
    fn status(&self) -> u8 {
        self.common.r8(DEVICE_STATUS)
    }
    fn add_status(&mut self, bits: u8) {
        let s = self.common.r8(DEVICE_STATUS);
        self.common.w8(DEVICE_STATUS, s | bits);
    }
    fn device_features(&mut self) -> u64 {
        self.common.w32(DEVICE_FEATURE_SELECT, 0);
        let lo = self.common.r32(DEVICE_FEATURE) as u64;
        self.common.w32(DEVICE_FEATURE_SELECT, 1);
        let hi = self.common.r32(DEVICE_FEATURE) as u64;
        lo | (hi << 32)
    }
    fn set_driver_features(&mut self, features: u64) {
        self.common.w32(DRIVER_FEATURE_SELECT, 0);
        self.common.w32(DRIVER_FEATURE, features as u32);
        self.common.w32(DRIVER_FEATURE_SELECT, 1);
        self.common.w32(DRIVER_FEATURE, (features >> 32) as u32);
    }
    fn queue_max(&mut self, q: u16) -> u16 {
        self.common.w16(QUEUE_SELECT, q);
        self.common.r16(QUEUE_SIZE)
    }
    fn setup_queue(&mut self, q: u16, size: u16, addrs: &QueueAddrs, vector: Option<u16>) -> Result<(), VirtioError> {
        self.common.w16(QUEUE_SELECT, q);
        let max = self.common.r16(QUEUE_SIZE);
        if max == 0 {
            return Err(VirtioError::QueueUnavailable);
        }
        if size == 0 || size > max || !size.is_power_of_two() {
            return Err(VirtioError::QueueTooSmall);
        }
        self.common.w16(QUEUE_SIZE, size);
        let v = vector.unwrap_or(0xFFFF);
        self.common.w16(QUEUE_MSIX_VECTOR, v);
        if vector.is_some() && self.common.r16(QUEUE_MSIX_VECTOR) != v {
            return Err(VirtioError::VectorRejected);
        }
        self.common.w64_split(QUEUE_DESC, addrs.desc.get());
        self.common.w64_split(QUEUE_DRIVER, addrs.driver.get());
        self.common.w64_split(QUEUE_DEVICE, addrs.device.get());
        Ok(())
    }
    fn enable_queue(&mut self, q: u16) {
        self.common.w16(QUEUE_SELECT, q);
        self.common.w16(QUEUE_ENABLE, 1);
    }
    fn notifier(&self, q: u16) -> Result<Notifier, VirtioError> {
        self.common.w16(QUEUE_SELECT, q);
        let off = self.common.r16(QUEUE_NOTIFY_OFF) as usize * self.notify_mult as usize;
        Ok(Notifier::Mmio { region: self.notify.sub(off, 2).map_err(|_| VirtioError::BadCaps)? })
    }
    fn read_config(&self, off: usize, out: &mut [u8]) {
        let Some(dev) = &self.device else {
            out.iter_mut().for_each(|b| *b = 0xFF);
            return;
        };
        // Retry until the generation counter is stable across the read.
        for _ in 0..16 {
            let g0 = self.common.r8(CONFIG_GENERATION);
            for (i, b) in out.iter_mut().enumerate() {
                *b = dev.r8(off + i);
            }
            if self.common.r8(CONFIG_GENERATION) == g0 {
                return;
            }
        }
    }
}
