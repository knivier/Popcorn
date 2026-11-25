//! Register facts for the Qualcomm ath11k family on PCIe (WCN6855), taken from
//! the register map the Linux ath11k driver uses. Pure `core`: the host test
//! crate includes this file.
//!
//! BAR0 is a window onto a much larger device address space. Offsets below
//! 0x80000 are read directly; anything higher goes through a 512 KiB window at
//! BAR0 + 0x80000 whose position is selected by writing WINDOW_REG.

pub const VENDOR_QCOM: u16 = 0x17CB;
pub const DEV_WCN6855: u16 = 0x1103;

// ---- register window ----
pub const WINDOW_REG: usize = 0x310C;
pub const WINDOW_ENABLE: u32 = 0x4000_0000;
pub const WINDOW_START: usize = 0x8_0000;
pub const WINDOW_RANGE_MASK: u32 = 0x7_FFFF;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    /// Read BAR0 + offset directly.
    Direct(usize),
    /// Select `window` (bits 24:19 of the device offset), then use BAR0 + bar_off.
    Windowed { window: u32, bar_off: usize },
}

pub fn access(off: u32) -> Access {
    if (off as usize) < WINDOW_START {
        Access::Direct(off as usize)
    } else {
        Access::Windowed {
            window: (off >> 19) & 0x3F,
            bar_off: WINDOW_START + (off & WINDOW_RANGE_MASK) as usize,
        }
    }
}

/// BAR0 bytes a windowed access can reach (window + its 512 KiB range).
pub const BAR_NEEDED: usize = WINDOW_START + WINDOW_RANGE_MASK as usize + 1;

// ---- chip version ----
pub const TCSR_SOC_HW_VERSION: u32 = 0x0224;
pub const TCSR_SOC_HW_SUB_VER: u32 = 0x0191_0010;

/// (major, minor) from TCSR_SOC_HW_VERSION: bits 11:8 and 7:0.
pub fn decode_version(soc: u32) -> (u32, u32) {
    ((soc >> 8) & 0xF, soc & 0xFF)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chip {
    Wcn6855Hw20,
    Wcn6855Hw21,
    /// Sibling chips that share the WCN6855 PCI ID (told apart by sub-version).
    Qca2066Hw21,
    Qca6698aqHw21,
    Unsupported { major: u32, minor: u32 },
}

/// Does this (major, minor) need TCSR_SOC_HW_SUB_VER to name the chip?
pub fn needs_sub_version(major: u32, minor: u32) -> bool {
    major == 2 && (minor == 0x10 || minor == 0x11)
}

pub fn chip(major: u32, minor: u32, sub_version: Option<u32>) -> Chip {
    match (major, minor) {
        (2, 0x00) | (2, 0x01) => Chip::Wcn6855Hw20,
        (2, 0x10) | (2, 0x11) => match sub_version {
            Some(0x1019_A0E1) | Some(0x1019_B0E1) | Some(0x1019_C0E1) | Some(0x1019_D0E1) => Chip::Qca2066Hw21,
            Some(0x001E_60E1) => Chip::Qca6698aqHw21,
            _ => Chip::Wcn6855Hw21,
        },
        _ => Chip::Unsupported { major, minor },
    }
}

impl Chip {
    pub fn name(self) -> &'static str {
        match self {
            Chip::Wcn6855Hw20 => "WCN6855 hw2.0",
            Chip::Wcn6855Hw21 => "WCN6855 hw2.1",
            Chip::Qca2066Hw21 => "QCA2066 hw2.1",
            Chip::Qca6698aqHw21 => "QCA6698AQ hw2.1",
            Chip::Unsupported { .. } => "unsupported revision",
        }
    }
    /// linux-firmware directory the later stages load from.
    pub fn fw_dir(self) -> Option<&'static str> {
        match self {
            Chip::Wcn6855Hw20 => Some("ath11k/WCN6855/hw2.0"),
            Chip::Wcn6855Hw21 => Some("ath11k/WCN6855/hw2.1"),
            Chip::Qca2066Hw21 => Some("ath11k/QCA2066/hw2.1"),
            Chip::Qca6698aqHw21 => Some("ath11k/QCA6698AQ/hw2.1"),
            Chip::Unsupported { .. } => None,
        }
    }
    pub fn supported(self) -> bool {
        !matches!(self, Chip::Unsupported { .. })
    }
}

// ---- power-on reset (ath11k "sw_reset" with power on) ----
pub const PCIE_PARF_LTSSM: u32 = 0x01E0_81B0;
pub const PARF_LTSSM_VALUE: u32 = 0x111;
pub const GCC_PCIE_HOT_RST: u32 = 0x01E4_02BC;
pub const GCC_PCIE_HOT_RST_VAL: u32 = 0x10;
pub const PCIE_INT_ALL_CLEAR: u32 = 0x01E0_8228;
pub const WLAON_QFPROM_PWR_CTRL: u32 = 0x01F8_031C;
pub const QFPROM_PWR_CTRL_VDD4BLOW: u32 = 0x4;
/// MHI vector doorbell/status registers cleared before reset.
pub const MHI_VECTOR_REGS: [u32; 4] = [0x360, 0x368, 0x394, 0x39C];
pub const PCIE_Q6_COOKIE: u32 = 0x01F8_0500;
pub const WLAON_WARM_SW_ENTRY: u32 = 0x01F8_0504;
pub const WLAON_SOC_RESET_CAUSE: u32 = 0x01F8_060C;
pub const PCIE_SOC_GLOBAL_RESET: u32 = 0x3008;
/// Keeps the SoC awake for host register access (ath11k "force wake").
pub const PCIE_SOC_WAKE: u32 = 0x3004;
pub const SOC_GLOBAL_RESET_V: u32 = 1;

// ---- MSI layout ----
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MsiUser {
    pub name: &'static str,
    pub count: u8,
    pub base: u8,
}

/// Full layout: 32 messages.
pub const MSI_32: [MsiUser; 4] = [
    MsiUser { name: "MHI", count: 3, base: 0 },
    MsiUser { name: "CE", count: 10, base: 3 },
    MsiUser { name: "WAKE", count: 1, base: 13 },
    MsiUser { name: "DP", count: 18, base: 14 },
];

/// Fallback: everything shares one message.
pub const MSI_ONE: [MsiUser; 4] = [
    MsiUser { name: "MHI", count: 3, base: 0 },
    MsiUser { name: "CE", count: 1, base: 0 },
    MsiUser { name: "WAKE", count: 1, base: 0 },
    MsiUser { name: "DP", count: 1, base: 0 },
];

// ---- copy engines on SRNG rings (stage W3b) ----
// Ring registers: CE n's source ring block at CE_SRC_BASE + n * CE_STRIDE, its
// destination (and destination-status) block at CE_DST_BASE + n * CE_STRIDE.
// Offsets inside a block follow the TCL1 layout for source-type rings (the
// CE source ring and, because the host fills it, the CE destination ring)
// and the REO1 layout for destination-type rings (the CE status ring).
pub const CE_SRC_BASE: u32 = 0x01B8_0000;
pub const CE_DST_BASE: u32 = 0x01B8_1000;
pub const CE_STRIDE: u32 = 0x2000;
/// Status ring registers start this far into the destination block.
pub const CE_STATUS_R0: u32 = 0x58;
/// Pointer registers (R2): HP, then TP at +4.
pub const CE_R2: u32 = 0x400;
pub const CE_STATUS_R2: u32 = 0x408;
/// Destination ring control: bits 15:0 = largest buffer.
pub const CE_DST_RING_CTRL: u32 = 0xB0;

// TCL1-layout offsets (source-type rings).
pub const SRC_BASE_LSB: u32 = 0x00;
pub const SRC_BASE_MSB: u32 = 0x04;
pub const SRC_ID: u32 = 0x08;
pub const SRC_MISC: u32 = 0x10;
pub const SRC_TP_ADDR_LSB: u32 = 0x1C;
pub const SRC_TP_ADDR_MSB: u32 = 0x20;
pub const SRC_IX0: u32 = 0x30;
pub const SRC_IX1: u32 = 0x34;
// REO1-layout offsets (destination-type rings).
pub const DST_BASE_LSB: u32 = 0x00;
pub const DST_BASE_MSB: u32 = 0x04;
pub const DST_ID: u32 = 0x08;
pub const DST_MISC: u32 = 0x10;
pub const DST_HP_ADDR_LSB: u32 = 0x14;
pub const DST_HP_ADDR_MSB: u32 = 0x18;
pub const DST_PRODUCER_INT: u32 = 0x24;

pub const MISC_LOOPCNT_DISABLE: u32 = 1 << 1;
pub const MISC_SRNG_ENABLE: u32 = 1 << 6;

/// Host shadow register slots (BAR0 + 0x8fc + 4 * slot) mirror the ring
/// pointer registers listed in WLAN_CFG's shadow_reg_v2, in that order.
pub const SHADOW_BASE: u32 = 0x8FC;
pub fn shadow_slot(slot: usize) -> u32 {
    SHADOW_BASE + 4 * slot as u32
}

/// SRNG ring ids (index into the pointer array the hardware writes).
pub const RING_ID_CE_SRC: u32 = 32;
pub const RING_ID_CE_DST: u32 = 56;
pub const RING_ID_CE_STATUS: u32 = 80;
/// Pointer array entries (HAL_SRNG_RING_ID_MAX).
pub const RING_IDS: usize = 172;

/// Entry sizes in 32-bit words.
pub const SRC_DESC_WORDS: u32 = 4;
pub const DST_DESC_WORDS: u32 = 2;
pub const STATUS_DESC_WORDS: u32 = 4;

/// Register block bases of CE `n`: (source R0, destination R0, status R0).
pub fn ce_blocks(n: u32) -> (u32, u32, u32) {
    let s = CE_SRC_BASE + n * CE_STRIDE;
    let d = CE_DST_BASE + n * CE_STRIDE;
    (s, d, d + CE_STATUS_R0)
}

/// BASE_MSB value: address bits 39:32 in 7:0, ring size (words) in 27:8.
pub fn ring_base_msb(addr: u64, size_words: u32) -> u32 {
    ((addr >> 32) as u32 & 0xFF) | ((size_words & 0xF_FFFF) << 8)
}

/// Source descriptor: buffer address, length (bits 31:16 of word 1), meta
/// (transfer id, bits 15:0 of word 2).
pub fn ce_src_desc(addr: u64, len: u32, meta: u32) -> [u32; 4] {
    [addr as u32, ((addr >> 32) as u32 & 0xFF) | ((len & 0xFFFF) << 16), meta & 0xFFFF, 0]
}

/// Destination descriptor: an empty buffer's address.
pub fn ce_dst_desc(addr: u64) -> [u32; 2] {
    [addr as u32, (addr >> 32) as u32 & 0xFF]
}

/// Bytes received, from a status descriptor's first word (bits 31:16).
pub fn ce_status_len(w0: u32) -> u32 {
    w0 >> 16
}

/// The shadow slot whose target is `reg` in WLAN_CFG's list.
pub fn shadow_index(list: &[u32], reg: u32) -> Option<usize> {
    list.iter().position(|&r| r == reg)
}

// ---- HTC (on top of the copy engines) ----
pub const HTC_HDR: usize = 8;
pub const HTC_MSG_READY: u16 = 1;
pub const HTC_MSG_CONNECT_SERVICE: u16 = 2;
pub const HTC_MSG_CONNECT_SERVICE_RESP: u16 = 3;
pub const HTC_MSG_SETUP_COMPLETE_EX: u16 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HtcHdr {
    pub eid: u8,
    pub flags: u8,
    pub len: u16,
    pub ctrl0: u8,
    pub ctrl1: u8,
}

pub fn htc_hdr(b: &[u8]) -> Option<HtcHdr> {
    if b.len() < HTC_HDR {
        return None;
    }
    let w0 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let w1 = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    Some(HtcHdr { eid: w0 as u8, flags: (w0 >> 8) as u8, len: (w0 >> 16) as u16, ctrl0: w1 as u8, ctrl1: (w1 >> 8) as u8 })
}

/// HTC READY from the target: (credits, credit size, max endpoints).
pub fn htc_ready(payload: &[u8]) -> Option<(u16, u16, u8)> {
    if payload.len() < 8 {
        return None;
    }
    let w0 = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
    let w1 = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if w0 as u16 != HTC_MSG_READY {
        return None;
    }
    Some(((w0 >> 16) as u16, w1 as u16, (w1 >> 16) as u8))
}
