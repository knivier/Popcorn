//! Rust-side IRQ claim table. PIC enable/EOI stay in C (`irq.c`); this tracks
//! which drive owns which ISA IRQ so the catalog and future IOAPIC can look it up.

use alloc::vec::Vec;

use crate::catalog;

pub type IrqHandler = fn();

pub struct IrqClaim {
    pub irq: u8,
    pub name: &'static str,
    pub handler: Option<IrqHandler>,
}

static mut CLAIMS: Option<Vec<IrqClaim>> = None;

fn table() -> &'static mut Vec<IrqClaim> {
    unsafe {
        if CLAIMS.is_none() {
            CLAIMS = Some(Vec::new());
        }
        CLAIMS.as_mut().unwrap()
    }
}

/// Record that `name` owns `irq` (does not touch the PIC — call C `irq_register` for that).
pub fn claim(irq: u8, name: &'static str, handler: Option<IrqHandler>) {
    let t = table();
    if let Some(c) = t.iter_mut().find(|c| c.irq == irq) {
        c.name = name;
        c.handler = handler;
    } else {
        t.push(IrqClaim { irq, name, handler });
    }
    catalog::publish_irq(name, irq);
}

pub fn owner(irq: u8) -> Option<&'static str> {
    table().iter().find(|c| c.irq == irq).map(|c| c.name)
}

pub fn dispatch_rust(irq: u8) {
    if let Some(h) = table().iter().find(|c| c.irq == irq).and_then(|c| c.handler) {
        h();
    }
}

/// Seed the known early IRQs (timer + keyboard).
pub fn seed_builtins() {
    claim(0, "clock", None);
    claim(1, "kbd", Some(crate::drivers::backends::kbd::irq));
}
