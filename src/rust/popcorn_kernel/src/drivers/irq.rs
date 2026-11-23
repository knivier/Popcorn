//! Rust-side IRQ claim table. PIC enable/EOI stay in C (`irq.c`); this tracks
//! which drive owns which ISA IRQ so the catalog and future IOAPIC can look it up.

use alloc::vec::Vec;

use crate::catalog;
use crate::sync::IrqSpinLock;

pub type IrqHandler = fn();

#[derive(Clone, Copy)]
pub struct IrqClaim {
    pub irq: u8,
    pub name: &'static str,
    pub handler: Option<IrqHandler>,
}

/// Leaf lock: the catalog and handlers are called only after it is dropped.
static CLAIMS: IrqSpinLock<Vec<IrqClaim>> = IrqSpinLock::new(Vec::new());

/// Record that `name` owns `irq` (does not touch the PIC — call C `irq_register` for that).
pub fn claim(irq: u8, name: &'static str, handler: Option<IrqHandler>) {
    {
        let mut t = CLAIMS.lock();
        if let Some(c) = t.iter_mut().find(|c| c.irq == irq) {
            c.name = name;
            c.handler = handler;
        } else {
            t.push(IrqClaim { irq, name, handler });
        }
    }
    catalog::publish_irq(name, irq);
}

pub fn owner(irq: u8) -> Option<&'static str> {
    CLAIMS.lock().iter().find(|c| c.irq == irq).map(|c| c.name)
}

pub fn dispatch_rust(irq: u8) {
    // Copy the handler out; call it with the table unlocked.
    let h = CLAIMS.lock().iter().find(|c| c.irq == irq).and_then(|c| c.handler);
    if let Some(h) = h {
        h();
    }
}

/// Seed the known early IRQs (timer + keyboard).
pub fn seed_builtins() {
    claim(0, "clock", None);
    claim(1, "kbd", Some(crate::drivers::backends::kbd::irq));
}
