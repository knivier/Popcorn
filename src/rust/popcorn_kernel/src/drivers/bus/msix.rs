//! MSI-X programming (virtio-net review §3.3.3). Kernel-owned: drivers never see
//! the table, only `IrqHandle`s (mask / unmask / take).
//!
//! 1. cap 0x11: table size = (mc & 0x7FF) + 1; table at BIR/offset (cap+4), PBA at cap+8.
//! 2. enable with the function masked (mc |= ENABLE | FUNCTION_MASK), INTx off.
//! 3. per entry: addr = 0xFEE0_0000 | (apic_id << 12) (physical, RH = 0), data =
//!    vector (fixed, edge), vector control = 1 (masked).
//! 4. clear FUNCTION_MASK. Each entry stays masked until its queue arms, so a
//!    polling driver costs QEMU no interrupt work at all.

use super::pci::{cfg_read16, cfg_write16, for_each_cap, Bar, PciFunction, CAP_MSIX};
use crate::drivers::grant::irq::{alloc_vector, handle_for, IrqHandle, IrqSource};
use crate::drivers::grant::MmioRegion;

extern "C" {
    fn lapic_id() -> u32;
    fn lapic_active() -> bool;
}

const MC_ENABLE: u16 = 1 << 15;
const MC_FUNCTION_MASK: u16 = 1 << 14;
const ENTRY_BYTES: usize = 16;
pub const MAX_IRQS: usize = 4;

/// Interrupt sources granted to one driver. `handles[i]` = MSI-X entry `i`.
pub struct IrqSet {
    pub handles: [IrqHandle; MAX_IRQS],
    pub count: usize,
    /// Kept alive so entry masks stay reachable; never exposed to the driver.
    _table: Option<MmioRegion>,
}

impl IrqSet {
    pub fn polled() -> IrqSet {
        IrqSet {
            handles: [IrqHandle::polled(), IrqHandle::polled(), IrqHandle::polled(), IrqHandle::polled()],
            count: 0,
            _table: None,
        }
    }
    pub fn is_polled(&self) -> bool {
        self.count == 0
    }
    /// Take entry `i` out of the set (the driver keeps it with its queue).
    pub fn take(&mut self, i: usize) -> IrqHandle {
        core::mem::replace(&mut self.handles[i], IrqHandle::polled())
    }
}

fn msix_cap(f: &PciFunction) -> Option<u8> {
    let mut found = None;
    let a = f.addr();
    for_each_cap(a, |id, off| {
        if id == CAP_MSIX && found.is_none() {
            found = Some(off);
        }
    });
    found
}

/// Allocate up to `want` MSI-X vectors for `f`. Errors (the caller then polls):
/// no LAPIC, no MSI-X capability, too few entries, no free vectors, or the table
/// cannot be mapped. Entries start masked.
pub fn alloc_irqs(f: &PciFunction, want: usize, owner: &'static str) -> Result<IrqSet, &'static str> {
    let want = want.min(MAX_IRQS);
    if want == 0 {
        return Err("no vectors requested");
    }
    if !unsafe { lapic_active() } {
        return Err("no LAPIC");
    }
    let Some(cap) = msix_cap(f) else { return Err("no MSI-X capability") };
    let a = f.addr();
    let mc = cfg_read16(a.bus, a.slot, a.func, cap + 2);
    let table_size = ((mc & 0x7FF) + 1) as usize;
    if table_size < want {
        return Err("MSI-X table too small");
    }
    let tbl = f.cfg_r32(cap + 4);
    let bir = (tbl & 7) as u8;
    let off = (tbl & !7) as usize;
    let table = match f.bar(bir) {
        Ok(Bar::Mmio(r)) => match r.sub(off, table_size * ENTRY_BYTES) {
            Ok(t) => t,
            Err(_) => return Err("MSI-X table outside its BAR"),
        },
        Ok(Bar::Io(_)) => return Err("MSI-X table BAR is I/O"),
        Err(e) => return Err(e.as_str()),
    };

    // Vectors first, so a shortage leaves the device untouched.
    let mut vectors = [0u8; MAX_IRQS];
    for i in 0..want {
        match alloc_vector(owner) {
            Some(v) => vectors[i] = v,
            None => {
                for &v in &vectors[..i] {
                    drop(handle_for(v, IrqSource::MsiX { entry: 0 }, 0)); // frees the vector
                }
                return Err("no free interrupt vectors");
            }
        }
    }

    // Enable with every entry masked by the function mask.
    cfg_write16(a.bus, a.slot, a.func, cap + 2, mc | MC_ENABLE | MC_FUNCTION_MASK);
    let apic = unsafe { lapic_id() };
    let mut set = IrqSet::polled();
    for i in 0..table_size {
        let e = i * ENTRY_BYTES;
        table.w32(e + 12, 1); // mask every entry (including ones we do not use)
        if i < want {
            table.w32(e, 0xFEE0_0000 | (apic << 12));
            table.w32(e + 4, 0);
            table.w32(e + 8, vectors[i] as u32);
        }
    }
    for i in 0..want {
        set.handles[i] = handle_for(vectors[i], IrqSource::MsiX { entry: i as u16 }, entry_ctrl_va(&table, i));
    }
    set.count = want;
    // Unmask the function; entries stay individually masked until armed.
    cfg_write16(a.bus, a.slot, a.func, cap + 2, (mc | MC_ENABLE) & !MC_FUNCTION_MASK);
    set._table = Some(table);
    Ok(set)
}

/// Disable MSI-X on `f` (driver teardown, after the device was reset).
pub fn disable(f: &PciFunction) {
    if let Some(cap) = msix_cap(f) {
        let a = f.addr();
        let mc = cfg_read16(a.bus, a.slot, a.func, cap + 2);
        cfg_write16(a.bus, a.slot, a.func, cap + 2, mc & !MC_ENABLE);
    }
}

fn entry_ctrl_va(table: &MmioRegion, i: usize) -> usize {
    table.va() + i * ENTRY_BYTES + 12
}
