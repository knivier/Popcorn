//! virtio PCI discovery and transport selection.
//!
//! IDs: transitional 0x1000 (net, subsystem 1) / 0x1001 (blk, subsystem 2); modern
//! 0x1041 (net) / 0x1042 (blk). Modern is chosen when COMMON + NOTIFY caps map;
//! otherwise a transitional device's I/O BAR0 gives the legacy transport.

use super::legacy::LegacyTransport;
use super::modern::ModernTransport;
use super::transport::Transport;
use super::{DeviceType, VirtioError};
use crate::drivers::bus::pci::{self, Bar, PciFunction, VENDOR_VIRTIO};

pub const NET_IDS: [(u16, u16); 2] = [(VENDOR_VIRTIO, 0x1041), (VENDOR_VIRTIO, 0x1000)];
pub const BLK_IDS: [(u16, u16); 2] = [(VENDOR_VIRTIO, 0x1042), (VENDOR_VIRTIO, 0x1001)];
pub const BLK_MODERN_ONLY: [(u16, u16); 1] = [(VENDOR_VIRTIO, 0x1042)];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prefer {
    Auto,
    Modern,
    Legacy,
}

pub enum AnyTransport {
    Modern(ModernTransport),
    Legacy(LegacyTransport),
}

impl AnyTransport {
    pub fn as_dyn(&mut self) -> &mut dyn Transport {
        match self {
            AnyTransport::Modern(t) => t,
            AnyTransport::Legacy(t) => t,
        }
    }
}

pub fn device_type_of(device: u16, subsys: u16) -> DeviceType {
    match device {
        0x1041 => DeviceType::Net,
        0x1042 => DeviceType::Block,
        0x1000..=0x103F => match subsys {
            1 => DeviceType::Net,
            2 => DeviceType::Block,
            s => DeviceType::Other(s),
        },
        d => DeviceType::Other(d.wrapping_sub(0x1040)),
    }
}

/// Open the right transport for a claimed function.
pub fn open(f: &PciFunction, prefer: Prefer) -> Result<AnyTransport, VirtioError> {
    let (_, device) = f.ids();
    let kind = device_type_of(device, f.subsystem_device());
    if prefer != Prefer::Legacy {
        match ModernTransport::open(f, kind) {
            Ok(t) => return Ok(AnyTransport::Modern(t)),
            Err(e) if prefer == Prefer::Modern => return Err(e),
            Err(_) => {}
        }
    }
    if (0x1000..=0x103F).contains(&device) {
        if let Ok(Bar::Io(io)) = f.bar(0) {
            return Ok(AnyTransport::Legacy(LegacyTransport::new(io, kind)));
        }
    }
    Err(VirtioError::BadCaps)
}

/// Claim the `n`th virtio device among `ids` (subsystem-checked for transitional IDs).
pub fn claim(ids: &[(u16, u16)], subsys: u16, n: usize) -> Option<PciFunction> {
    // Modern IDs carry no subsystem convention; try them first without the check.
    let modern: &[(u16, u16)] = &ids[..1];
    pci::claim(modern, None, n).or_else(|| pci::claim(&ids[1..], Some(subsys), n))
}
