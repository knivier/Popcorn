//! Plain MSI arithmetic (PCI capability 0x05). Pure `core`: the host test crate
//! includes this file.
//!
//! Message Control (cap + 2):
//!   bit 0     MSI enable
//!   bits 3:1  Multiple Message Capable: the device can use 2^n messages
//!   bits 6:4  Multiple Message Enable: how many the OS granted (2^n)
//!   bit 7     64-bit message address
//!   bit 8     per-vector masking (mask + pending dwords follow the data word)
//! A device sending message `i` of a multi-message block writes `data | i`, so
//! the vectors behind one block must be consecutive and aligned to its size.

pub const MC_ENABLE: u16 = 1 << 0;
pub const MC_MME_MASK: u16 = 0b111 << 4;
pub const MC_64BIT: u16 = 1 << 7;
pub const MC_PVM: u16 = 1 << 8;
/// Largest block the capability can describe.
pub const MAX_MESSAGES: u8 = 32;

/// How many messages the device can use (Multiple Message Capable). The
/// reserved encodings 6 and 7 are treated as 32, the most the field allows.
pub fn capable_messages(mc: u16) -> u8 {
    let e = (mc >> 1) & 0b111;
    1u8 << e.min(5)
}

/// Multiple Message Enable bits for `n` messages, or None if `n` is not a power
/// of two in 1..=32.
pub fn mme_bits(n: u8) -> Option<u16> {
    if n == 0 || n > MAX_MESSAGES || !n.is_power_of_two() {
        return None;
    }
    Some((n.trailing_zeros() as u16) << 4)
}

/// Register offsets relative to the capability, which depend on the 64-bit and
/// per-vector-masking bits of Message Control.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Layout {
    pub addr_lo: u8,
    pub addr_hi: Option<u8>,
    pub data: u8,
    pub mask: Option<u8>,
    pub pending: Option<u8>,
}

pub fn layout(mc: u16) -> Layout {
    let is_64 = mc & MC_64BIT != 0;
    let pvm = mc & MC_PVM != 0;
    let data = if is_64 { 12 } else { 8 };
    Layout {
        addr_lo: 4,
        addr_hi: if is_64 { Some(8) } else { None },
        data,
        mask: if pvm { Some(data + 4) } else { None },
        pending: if pvm { Some(data + 8) } else { None },
    }
}

/// x86 MSI address for a fixed, physical-destination message to LAPIC `apic`
/// (xAPIC IDs only: 8 bits).
pub fn x86_address(apic: u32) -> u64 {
    0xFEE0_0000 | (((apic & 0xFF) as u64) << 12)
}

/// x86 MSI data for vector `v`: fixed delivery, edge triggered.
pub fn x86_data(v: u8) -> u32 {
    v as u32
}

/// First `n`-aligned run of `n` vectors inside `first..=last` for which `free`
/// holds for every vector. `n` must be a power of two in 1..=32.
pub fn find_aligned_block(first: u8, last: u8, n: u8, free: impl Fn(u8) -> bool) -> Option<u8> {
    mme_bits(n)?;
    let n16 = n as u16;
    let mut start = (first as u16 + n16 - 1) & !(n16 - 1);
    while start + n16 - 1 <= last as u16 {
        if (start..start + n16).all(|v| free(v as u8)) {
            return Some(start as u8);
        }
        start += n16;
    }
    None
}
