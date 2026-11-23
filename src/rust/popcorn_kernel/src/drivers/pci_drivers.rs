//! PCI driver registry: one table of (driver, device class, supported IDs, probe).
//!
//! Adding a device family = adding one entry here plus its driver module. The
//! matcher walks the bus at boot and hands every matching, unclaimed function to
//! its driver's probe as a `PciFunction` grant source (`bus::pci::claim`, so two
//! drivers can never own one function).
//!
//! Drivers that predate the framework (legacy virtio-blk, NVMe, xHCI) still find
//! their own hardware; they move onto this table as they move onto grants (M4+).

use core::fmt::Write;

use super::bus::pci::{self, PciFunction};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DriverClass {
    Net,
    Block,
    Usb,
    Display,
}

impl DriverClass {
    pub fn as_str(self) -> &'static str {
        match self {
            DriverClass::Net => "net",
            DriverClass::Block => "block",
            DriverClass::Usb => "usb",
            DriverClass::Display => "display",
        }
    }
}

pub struct PciDriver {
    pub name: &'static str,
    pub class: DriverClass,
    /// (vendor, device) pairs this driver supports.
    pub ids: &'static [(u16, u16)],
    /// Required PCI subsystem device id (transitional virtio IDs are shared by
    /// every virtio type), or None.
    pub subsys: Option<u16>,
    /// Bring up one device. Errors are logged; the function is released.
    pub probe: fn(PciFunction) -> Result<(), &'static str>,
}

/// The table. Order matters only when two entries could match one function.
pub static PCI_DRIVERS: &[PciDriver] = &[
    PciDriver {
        name: "virtio-net",
        class: DriverClass::Net,
        ids: &[(pci::VENDOR_VIRTIO, pci::DEV_VIRTIO_NET_MODERN)],
        subsys: None,
        probe: super::virtio::net::probe,
    },
    PciDriver {
        name: "virtio-net",
        class: DriverClass::Net,
        ids: &[(pci::VENDOR_VIRTIO, pci::DEV_VIRTIO_NET_LEGACY)],
        subsys: Some(1),
        probe: super::virtio::net::probe,
    },
];

const MAX_PER_DRIVER: usize = 4;

/// Match every table entry against the bus and probe what it claims.
/// Returns the number of devices brought up.
pub fn probe_all() -> usize {
    let mut up = 0;
    for d in PCI_DRIVERS {
        for n in 0..MAX_PER_DRIVER {
            let Some(f) = pci::claim(d.ids, d.subsys, n) else {
                continue;
            };
            let a = f.addr();
            let mut out = crate::Com1;
            match (d.probe)(f) {
                Ok(()) => {
                    up += 1;
                    let _ = writeln!(out, "drv: {} ({}) bound to {}:{}.{}", d.name, d.class.as_str(), a.bus, a.slot, a.func);
                }
                Err(e) => {
                    let _ = writeln!(out, "drv: {} probe failed on {}:{}.{}: {}", d.name, a.bus, a.slot, a.func, e);
                }
            }
        }
    }
    up
}
