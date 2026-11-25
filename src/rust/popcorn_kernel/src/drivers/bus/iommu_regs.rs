//! AMD IOMMU (AMD-Vi) register and table decoding, for diagnostics. Pure
//! `core`: host-tested. Layouts from the AMD I/O Virtualization Technology
//! (IOMMU) specification.
//!
//! Why Popcorn cares: firmware can leave the IOMMU on ("Kernel DMA Protection"
//! on ThinkPads) with device-table entries that stop a device's memory reads.
//! A kernel that never programs the IOMMU then sees DMA silently fail for those
//! devices (the WCN6855 boot ROM never finishing its image read).

/// PCI capability ID of the IOMMU ("secure device") capability.
pub const CAP_ID: u8 = 0x0F;

// MMIO registers.
pub const DEV_TABLE_BASE: usize = 0x0000;
pub const EVENT_LOG_BASE: usize = 0x0010;
pub const CONTROL: usize = 0x0018;
pub const EVENT_HEAD: usize = 0x2010;
pub const EVENT_TAIL: usize = 0x2018;
pub const STATUS: usize = 0x2020;
pub const MMIO_BYTES: usize = 0x4000;

pub const CTRL_IOMMU_EN: u64 = 1 << 0;
pub const CTRL_EVENT_LOG_EN: u64 = 1 << 2;
pub const CTRL_CMD_BUF_EN: u64 = 1 << 12;

/// MMIO base from the capability's base-address dwords (cap+4 low, cap+8 high):
/// bit 0 of the low dword says the base is valid; the base is 16 KiB aligned.
pub fn mmio_base(lo: u32, hi: u32) -> Option<u64> {
    if lo & 1 == 0 {
        return None;
    }
    Some(((hi as u64) << 32) | (lo as u64 & 0xFFFF_C000))
}

/// Device table: (physical base, bytes). Size field bits 8:0 = 4 KiB pages - 1.
pub fn dev_table(reg: u64) -> (u64, usize) {
    (reg & 0x000F_FFFF_FFFF_F000, ((reg & 0x1FF) as usize + 1) * 4096)
}

/// Event log: (physical base, entries). Length field bits 59:56 = log2(entries).
pub fn event_log(reg: u64) -> (u64, usize) {
    let len = ((reg >> 56) & 0xF) as u32;
    (reg & 0x000F_FFFF_FFFF_F000, 1usize << len.clamp(8, 15))
}

/// PCI requester ID (device ID in IOMMU terms).
pub fn device_id(bus: u8, slot: u8, func: u8) -> u16 {
    ((bus as u16) << 8) | ((slot as u16 & 0x1F) << 3) | (func as u16 & 7)
}

pub const DTE_BYTES: usize = 32;
pub const EVENT_BYTES: usize = 16;

/// What a device-table entry does to a device's DMA.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmaPolicy {
    /// V = 0: the IOMMU does not look at this device (untranslated).
    NotValid,
    /// V = 1, TV = 0: no translation information.
    NoTranslationInfo,
    /// V = 1, TV = 1, mode 0: translation off, reads/writes per IR/IW.
    Passthrough { read: bool, write: bool },
    /// V = 1, TV = 1, mode 1-6: page tables at `root`; only mapped pages pass.
    Translated { mode: u8, root: u64, read: bool, write: bool },
    /// Mode 7 is reserved.
    Reserved,
}

/// Decode the first 64 bits of a device-table entry.
pub fn dte_policy(q0: u64) -> DmaPolicy {
    let v = q0 & 1 != 0;
    let tv = q0 & 2 != 0;
    let mode = ((q0 >> 9) & 7) as u8;
    let read = q0 & (1 << 61) != 0;
    let write = q0 & (1 << 62) != 0;
    match (v, tv, mode) {
        (false, _, _) => DmaPolicy::NotValid,
        (true, false, _) => DmaPolicy::NoTranslationInfo,
        (true, true, 0) => DmaPolicy::Passthrough { read, write },
        (true, true, 7) => DmaPolicy::Reserved,
        (true, true, m) => DmaPolicy::Translated { mode: m, root: q0 & 0x000F_FFFF_FFFF_F000, read, write },
    }
}

/// One event log entry: (event code, device ID, address).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Event {
    pub code: u8,
    pub device: u16,
    pub flags: u16,
    pub address: u64,
}

pub fn decode_event(d: [u32; 4]) -> Event {
    Event {
        code: (d[1] >> 28) as u8,
        device: d[0] as u16,
        flags: ((d[1] >> 16) & 0xFFF) as u16,
        address: d[2] as u64 | ((d[3] as u64) << 32),
    }
}

pub fn event_name(code: u8) -> &'static str {
    match code {
        0x1 => "ILLEGAL_DEV_TABLE_ENTRY",
        0x2 => "IO_PAGE_FAULT",
        0x3 => "DEV_TAB_HARDWARE_ERROR",
        0x4 => "PAGE_TAB_HARDWARE_ERROR",
        0x5 => "ILLEGAL_COMMAND_ERROR",
        0x6 => "COMMAND_HARDWARE_ERROR",
        0x7 => "IOTLB_INV_TIMEOUT",
        0x8 => "INVALID_DEVICE_REQUEST",
        0x9 => "INVALID_PPR_REQUEST",
        0xA => "EVENT_COUNTER_ZERO",
        _ => "?",
    }
}
