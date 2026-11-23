//! Driver registration — name + probe hook. Backends still live under `backends/`.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::sync::IrqSpinLock;

pub type DriverId = u32;

pub type ProbeFn = fn() -> Result<(), &'static str>;

#[derive(Clone, Copy)]
pub struct Driver {
    pub id: DriverId,
    pub name: &'static str,
    pub probe: ProbeFn,
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
/// Leaf lock: probe functions are copied out and called after it is dropped.
static DRIVERS: IrqSpinLock<Vec<Driver>> = IrqSpinLock::new(Vec::new());

pub fn driver_register(name: &'static str, probe: ProbeFn) -> DriverId {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    DRIVERS.lock().push(Driver { id, name, probe });
    id
}

/// Copy of the registered driver named `name`.
pub fn driver_find(name: &str) -> Option<Driver> {
    DRIVERS.lock().iter().find(|d| d.name == name).copied()
}

pub fn driver_probe(name: &str) -> Result<(), &'static str> {
    // Lock is released before the probe runs (probes may re-enter registries).
    let d = driver_find(name).ok_or("unknown driver")?;
    (d.probe)()
}
