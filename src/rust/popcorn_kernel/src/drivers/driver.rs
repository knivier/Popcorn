//! Driver registration — name + probe hook. Backends still live under `backends/`.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

pub type DriverId = u32;

pub type ProbeFn = fn() -> Result<(), &'static str>;

pub struct Driver {
    pub id: DriverId,
    pub name: &'static str,
    pub probe: ProbeFn,
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);
static mut DRIVERS: Option<Vec<Driver>> = None;

fn table() -> &'static mut Vec<Driver> {
    unsafe {
        if DRIVERS.is_none() {
            DRIVERS = Some(Vec::new());
        }
        DRIVERS.as_mut().unwrap()
    }
}

pub fn driver_register(name: &'static str, probe: ProbeFn) -> DriverId {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    table().push(Driver { id, name, probe });
    id
}

pub fn driver_find(name: &str) -> Option<&'static Driver> {
    table().iter().find(|d| d.name == name).map(|d| {
        // SAFETY: drivers live for the kernel lifetime in the static Vec.
        unsafe { &*(d as *const Driver) }
    })
}

pub fn driver_probe(name: &str) -> Result<(), &'static str> {
    let d = driver_find(name).ok_or("unknown driver")?;
    (d.probe)()
}
