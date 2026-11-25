//! Plain MSI (PCI capability 0x05) for devices without MSI-X, such as the
//! Qualcomm WCN6855 Wi-Fi chip. Kernel-owned like MSI-X: drivers get an
//! [`IrqSet`] of `IrqHandle`s and never write the capability themselves.
//!
//! 1. Message Control: the device can use 2^MMC messages; we grant exactly the
//!    power of two the driver asks for (no partial grants: drivers such as
//!    ath11k have one layout for 32 messages and one for a single message).
//! 2. Vectors: one naturally aligned block (`alloc_vector_block`), because the
//!    device writes `data | index` for message `index`.
//! 3. Program with MSI off: address 0xFEE0_0000 | apic << 12, data = base
//!    vector; with per-vector masking, every granted message starts masked.
//! 4. Set Multiple Message Enable and MSI enable.
//!
//! Without per-vector masking `IrqHandle::mask` is a no-op; such devices are
//! quietened through their own registers. Call [`disable`] before dropping the
//! set, or a device without masking could still signal a freed vector.

use super::msi_math::{self, MC_64BIT, MC_ENABLE, MC_MME_MASK, MC_PVM};
use super::msix::{IrqSet, MAX_IRQS};
use super::pci::{cfg_read16, cfg_write16, cfg_write32, PciFunction, CAP_MSI, CAP_MSIX};
use crate::drivers::grant::irq::{alloc_vector_block, handle_for, IrqHandle, IrqSource, MaskCtl};

extern "C" {
    fn lapic_id() -> u32;
    fn lapic_active() -> bool;
}

/// What a function's MSI capability offers (for logs and driver decisions).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // every field is reported by Debug; drivers read what they need
pub struct MsiInfo {
    pub cap: u8,
    pub capable: u8,
    pub is_64: bool,
    pub per_vector_mask: bool,
    pub enabled: bool,
    /// Messages currently enabled (Multiple Message Enable).
    pub granted: u8,
}

fn find_cap(f: &PciFunction, id: u8) -> Option<u8> {
    let mut found = None;
    f.caps(|cid, off| {
        if cid == id && found.is_none() {
            found = Some(off);
        }
    });
    found
}

pub fn info(f: &PciFunction) -> Option<MsiInfo> {
    let cap = find_cap(f, CAP_MSI)?;
    let mc = f.cfg_r16(cap + 2);
    Some(MsiInfo {
        cap,
        capable: msi_math::capable_messages(mc),
        is_64: mc & MC_64BIT != 0,
        per_vector_mask: mc & MC_PVM != 0,
        enabled: mc & MC_ENABLE != 0,
        granted: 1u8 << ((mc & MC_MME_MASK) >> 4).min(5),
    })
}

/// Grant exactly `n` (power of two, 1..=32) MSI messages to `f`. Errors (the
/// caller then tries fewer, or polls): bad count, no LAPIC, no MSI capability,
/// the device cannot use `n` messages, MSI-X is on, or no aligned vector block.
pub fn alloc_irqs(f: &PciFunction, n: u8, owner: &'static str) -> Result<IrqSet, &'static str> {
    let mme = msi_math::mme_bits(n).ok_or("MSI message count must be a power of two up to 32")?;
    if n as usize > MAX_IRQS {
        return Err("too many MSI messages");
    }
    if !unsafe { lapic_active() } {
        return Err("no LAPIC");
    }
    let cap = find_cap(f, CAP_MSI).ok_or("no MSI capability")?;
    if let Some(xcap) = find_cap(f, CAP_MSIX) {
        if f.cfg_r16(xcap + 2) & (1 << 15) != 0 {
            return Err("MSI-X is enabled on this function");
        }
    }
    let a = f.addr();
    let mc = cfg_read16(a.bus, a.slot, a.func, cap + 2);
    if msi_math::capable_messages(mc) < n {
        return Err("device cannot use that many MSI messages");
    }
    let base = alloc_vector_block(n, owner).ok_or("no aligned block of free interrupt vectors")?;
    let lay = msi_math::layout(mc);
    let addr = msi_math::x86_address(unsafe { lapic_id() });
    let data = msi_math::x86_data(base);

    // Program with MSI off so the device never signals a half-written message.
    let off = mc & !(MC_ENABLE | MC_MME_MASK);
    cfg_write16(a.bus, a.slot, a.func, cap + 2, off);
    cfg_write32(a.bus, a.slot, a.func, cap + lay.addr_lo, addr as u32);
    if let Some(hi) = lay.addr_hi {
        cfg_write32(a.bus, a.slot, a.func, cap + hi, (addr >> 32) as u32);
    }
    cfg_write16(a.bus, a.slot, a.func, cap + lay.data, data as u16);
    if let Some(m) = lay.mask {
        // Mask every message the device has (granted ones are unmasked when armed);
        // bits above its message count are reserved, so leave them clear.
        let all = msi_math::capable_messages(mc) as u64;
        cfg_write32(a.bus, a.slot, a.func, cap + m, ((1u64 << all) - 1) as u32);
    }

    let mut handles: [IrqHandle; MAX_IRQS] = core::array::from_fn(|_| IrqHandle::polled());
    for i in 0..n {
        let ctl = match lay.mask {
            Some(m) => MaskCtl::MsiMaskBit { bus: a.bus, slot: a.slot, func: a.func, off: cap + m, bit: i },
            None => MaskCtl::None,
        };
        handles[i as usize] = handle_for(base + i, IrqSource::Msi { index: i }, ctl);
    }
    cfg_write16(a.bus, a.slot, a.func, cap + 2, off | mme | MC_ENABLE);
    Ok(IrqSet::from_msi(handles, n as usize, addr, data))
}

/// Turn MSI off on `f` (before dropping its `IrqSet`, or on driver teardown).
pub fn disable(f: &PciFunction) {
    if let Some(cap) = find_cap(f, CAP_MSI) {
        let a = f.addr();
        let mc = cfg_read16(a.bus, a.slot, a.func, cap + 2);
        cfg_write16(a.bus, a.slot, a.func, cap + 2, mc & !(MC_ENABLE | MC_MME_MASK));
    }
}
