//! Qualcomm MHI (Modem Host Interface): the PCIe transport Qualcomm modems and
//! Wi-Fi chips (ath11k / ath12k class) use to load firmware and carry control
//! messages. Shared by every Qualcomm driver; nothing here is WCN6855-specific.
//!
//! W0 scope: the register map and decoding the device's state before any
//! firmware is loaded. Pure `core` (register reads come in through a closure),
//! so the host test crate includes this file.
//!
//! MHI registers sit at the start of the device's BAR (offset 0 on ath11k).
//! The BHI block (boot host interface, used to push the first firmware image)
//! is found through BHIOFF.

pub const MHIREGLEN: usize = 0x00;
pub const MHIVER: usize = 0x08;
pub const MHICFG: usize = 0x10;
pub const CHDBOFF: usize = 0x18;
pub const ERDBOFF: usize = 0x20;
pub const BHIOFF: usize = 0x28;
pub const BHIEOFF: usize = 0x2C;
pub const MHICTRL: usize = 0x38;
pub const MHISTATUS: usize = 0x48;

/// MHICTRL bit 1: host requests an MHI reset; the device clears it when done.
pub const MHICTRL_RESET: u32 = 1 << 1;
pub const MHISTATUS_READY: u32 = 1 << 0;
pub const MHISTATUS_SYSERR: u32 = 1 << 2;

/// BHI registers, relative to the BHI base (BAR + BHIOFF).
pub const BHI_VERSION_MINOR: usize = 0x00;
pub const BHI_VERSION_MAJOR: usize = 0x04;
pub const BHI_EXECENV: usize = 0x28;
pub const BHI_STATUS: usize = 0x2C;
pub const BHI_ERRCODE: usize = 0x30;

/// MHI state field (bits 15:8 of MHISTATUS and MHICTRL).
pub fn state_of(reg: u32) -> u32 {
    (reg >> 8) & 0xFF
}

pub fn state_name(s: u32) -> &'static str {
    match s {
        0x0 => "RESET",
        0x1 => "READY",
        0x2 => "M0",
        0x3 => "M1",
        0x4 => "M2",
        0x5 => "M3",
        0x6 => "M3_FAST",
        0x7 => "BHI",
        0xFF => "SYS_ERR",
        _ => "?",
    }
}

/// Execution environment: which firmware stage the device is running.
pub fn ee_name(ee: u32) -> &'static str {
    match ee {
        0 => "PBL (boot ROM, waiting for firmware)",
        1 => "SBL",
        2 => "AMSS (main firmware)",
        3 => "RDDM (crash dump)",
        4 => "WFW",
        5 => "PTHRU",
        6 => "EDL (emergency download)",
        7 => "FP",
        _ => "?",
    }
}

/// BHI_STATUS bits 31:30.
pub fn bhi_status_name(reg: u32) -> &'static str {
    match reg >> 30 {
        0 => "reset",
        2 => "success",
        3 => "error",
        _ => "?",
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Snapshot {
    pub reglen: u32,
    pub version: u32,
    pub ctrl: u32,
    pub status: u32,
    pub bhi_off: u32,
    /// None when BHIOFF points outside `bar_len` (or the link is down).
    pub ee: Option<u32>,
    pub bhi_status: Option<u32>,
}

impl Snapshot {
    pub fn state(&self) -> u32 {
        state_of(self.status)
    }
    pub fn ready(&self) -> bool {
        self.status & MHISTATUS_READY != 0
    }
    pub fn syserr(&self) -> bool {
        self.status & MHISTATUS_SYSERR != 0
    }
    /// All-ones everywhere: the device is not answering (link down / powered off).
    pub fn dead(&self) -> bool {
        self.version == 0xFFFF_FFFF && self.status == 0xFFFF_FFFF
    }
}

/// Read the MHI and BHI state through `r32(offset)` on a BAR of `bar_len` bytes.
pub fn snapshot(bar_len: usize, r32: impl Fn(usize) -> u32) -> Snapshot {
    let bhi_off = r32(BHIOFF);
    let bhi = bhi_off as usize;
    let bhi_ok = bhi_off != 0xFFFF_FFFF && bhi % 4 == 0 && bhi.checked_add(BHI_ERRCODE + 4).map_or(false, |e| e <= bar_len);
    Snapshot {
        reglen: r32(MHIREGLEN),
        version: r32(MHIVER),
        ctrl: r32(MHICTRL),
        status: r32(MHISTATUS),
        bhi_off,
        ee: if bhi_ok { Some(r32(bhi + BHI_EXECENV)) } else { None },
        bhi_status: if bhi_ok { Some(r32(bhi + BHI_STATUS)) } else { None },
    }
}

// ======================================================================
// Firmware load and MHI bring-up (stage W1). Layouts follow the MHI host
// protocol as the Linux MHI core implements it (drivers/bus/mhi).
// ======================================================================

// ---- more MHI registers (relative to the MHI base) ----
pub const CCABAP_LOWER: usize = 0x58;
pub const CCABAP_HIGHER: usize = 0x5C;
pub const ECABAP_LOWER: usize = 0x60;
pub const ECABAP_HIGHER: usize = 0x64;
pub const CRCBAP_LOWER: usize = 0x68;
pub const CRCBAP_HIGHER: usize = 0x6C;
pub const CRDB_LOWER: usize = 0x70;
pub const CRDB_HIGHER: usize = 0x74;
pub const MHICTRLBASE_LOWER: usize = 0x80;
pub const MHICTRLBASE_HIGHER: usize = 0x84;
pub const MHICTRLLIMIT_LOWER: usize = 0x88;
pub const MHICTRLLIMIT_HIGHER: usize = 0x8C;
pub const MHIDATABASE_LOWER: usize = 0x98;
pub const MHIDATABASE_HIGHER: usize = 0x9C;
pub const MHIDATALIMIT_LOWER: usize = 0xA0;
pub const MHIDATALIMIT_HIGHER: usize = 0xA4;

/// MHICFG: number of event rings (bits 23:16) and hardware event rings (31:24).
pub fn mhicfg_with_rings(cfg: u32, ner: u8, nhwer: u8) -> u32 {
    (cfg & 0x0000_FFFF) | ((ner as u32) << 16) | ((nhwer as u32) << 24)
}

/// MHICTRL with the MHI state field (bits 15:8) set to `state`.
pub fn mhictrl_with_state(ctrl: u32, state: u32) -> u32 {
    (ctrl & !0xFF00) | ((state & 0xFF) << 8)
}

pub const STATE_RESET: u32 = 0x0;
pub const STATE_READY: u32 = 0x1;
pub const STATE_M0: u32 = 0x2;
pub const STATE_SYS_ERR: u32 = 0xFF;

pub const EE_PBL: u32 = 0;
pub const EE_SBL: u32 = 1;
pub const EE_AMSS: u32 = 2;
pub const EE_RDDM: u32 = 3;

// ---- BHI (boot ROM image push), relative to BHIOFF ----
pub const BHI_IMGADDR_LOW: usize = 0x08;
pub const BHI_IMGADDR_HIGH: usize = 0x0C;
pub const BHI_IMGSIZE: usize = 0x10;
pub const BHI_IMGTXDB: usize = 0x18;
pub const BHI_INTVEC: usize = 0x20;
pub const BHI_ERRDBG1: usize = 0x34;
pub const BHI_ERRDBG2: usize = 0x38;
pub const BHI_ERRDBG3: usize = 0x3C;
pub const BHI_SERIALNU: usize = 0x40;

/// BHI_STATUS / BHIE_TXVECSTATUS field (bits 31:30).
pub fn xfer_status(reg: u32) -> u32 {
    reg >> 30
}
pub const XFER_RESET: u32 = 0;
pub const XFER_DONE: u32 = 2;
pub const XFER_ERROR: u32 = 3;

// ---- BHIe (vector-table image push), relative to BHIEOFF ----
pub const BHIE_TXVECADDR_LOW: usize = 0x2C;
pub const BHIE_TXVECADDR_HIGH: usize = 0x30;
pub const BHIE_TXVECSIZE: usize = 0x34;
pub const BHIE_TXVECDB: usize = 0x3C;
pub const BHIE_TXVECSTATUS: usize = 0x44;

/// Doorbell sequence numbers (BHI_IMGTXDB, BHIE_TXVECDB) live in bits 29:0 and
/// must be non-zero. Derived from a free-running counter (the TSC).
pub fn seq_number(seed: u64) -> u32 {
    let v = (seed ^ (seed >> 29) ^ (seed >> 47)) as u32 & 0x3FFF_FFFF;
    if v == 0 {
        1
    } else {
        v
    }
}

/// TXVECDB with the sequence field (bits 29:0) replaced.
pub fn txvecdb_with_seq(reg: u32, seq: u32) -> u32 {
    (reg & !0x3FFF_FFFF) | (seq & 0x3FFF_FFFF)
}

// ---- BHIe vector table ----
/// One entry: little-endian (device address, size), 16 bytes.
pub const VEC_ENTRY_BYTES: usize = 16;

/// How an image of `image` bytes splits into `seg`-byte segments: (segments,
/// bytes in the last one). The table itself is a separate buffer.
pub fn bhie_segments(image: usize, seg: usize) -> (usize, usize) {
    if image == 0 || seg == 0 {
        return (0, 0);
    }
    let n = image.div_ceil(seg);
    (n, image - (n - 1) * seg)
}

/// Vector table entry bytes for a segment at `addr` holding `len` bytes.
pub fn vec_entry(addr: u64, len: u64) -> [u8; VEC_ENTRY_BYTES] {
    let mut e = [0u8; VEC_ENTRY_BYTES];
    e[..8].copy_from_slice(&addr.to_le_bytes());
    e[8..].copy_from_slice(&len.to_le_bytes());
    e
}

// ---- contexts (44 bytes each, little-endian, 4-byte aligned fields) ----
pub const CTX_WORDS: usize = 11;
pub const CTX_BYTES: usize = CTX_WORDS * 4;

/// Channel direction (chtype): host to device / device to host.
pub const CH_TYPE_OUTBOUND: u32 = 1;
pub const CH_TYPE_INBOUND: u32 = 2;
/// Doorbell burst mode "disabled" (MHI_DB_BRST_DISABLE), in chcfg bits 9:8.
pub const DB_BRST_DISABLE: u32 = 2;
pub const EV_TYPE_VALID: u32 = 1;

fn put64(w: &mut [u32; CTX_WORDS], at: usize, v: u64) {
    w[at] = v as u32;
    w[at + 1] = (v >> 32) as u32;
}

/// Channel context: chcfg (state DISABLED, burst mode, poll cfg 0), chtype,
/// erindex; ring base/len/rp/wp are filled when the channel is started.
pub fn chan_ctx(chtype: u32, erindex: u32, brst: u32) -> [u32; CTX_WORDS] {
    let mut w = [0u32; CTX_WORDS];
    w[0] = (brst & 3) << 8; // CHSTATE = 0 (disabled)
    w[1] = chtype;
    w[2] = erindex;
    w
}

/// Event ring context: interrupt moderation (ms, bits 31:16), type valid,
/// MSI vector index, ring base/len, rp = wp = base.
pub fn event_ctx(intmod_ms: u16, msivec: u32, base: u64, len: u64) -> [u32; CTX_WORDS] {
    let mut w = [0u32; CTX_WORDS];
    w[0] = (intmod_ms as u32) << 16;
    w[1] = EV_TYPE_VALID;
    w[2] = msivec;
    put64(&mut w, 3, base);
    put64(&mut w, 5, len);
    put64(&mut w, 7, base);
    put64(&mut w, 9, base);
    w
}

/// Command ring context: ring base/len, rp = wp = base.
pub fn cmd_ctx(base: u64, len: u64) -> [u32; CTX_WORDS] {
    let mut w = [0u32; CTX_WORDS];
    put64(&mut w, 3, base);
    put64(&mut w, 5, len);
    put64(&mut w, 7, base);
    put64(&mut w, 9, base);
    w
}

/// Channel state in chcfg bits 7:0.
pub const CH_STATE_ENABLED: u32 = 1;

pub const PKT_TRANSFER: u32 = 0x02;
pub const PKT_RESET_CHAN: u32 = 0x10;
pub const PKT_STOP_CHAN: u32 = 0x11;
pub const PKT_START_CHAN: u32 = 0x12;

/// Command ring element: channel in dword1 bits 31:24, type in 23:16.
pub fn cmd_el(kind: u32, chan: u32) -> [u32; 4] {
    [0, 0, 0, ((chan & 0xFF) << 24) | ((kind & 0xFF) << 16)]
}

/// Transfer ring element for one buffer: ptr, length (bits 15:0), type
/// TRANSFER, interrupt on end of transfer (bit 9) when `ieot`.
pub fn transfer_el(addr: u64, len: u32, ieot: bool) -> [u32; 4] {
    [addr as u32, (addr >> 32) as u32, len & 0xFFFF, (PKT_TRANSFER << 16) | if ieot { 1 << 9 } else { 0 }]
}

/// Word index of rp / wp inside any context.
pub const CTX_RP: usize = 7;
pub const CTX_WP: usize = 9;

// ---- ring elements and events (16 bytes: ptr, dword0, dword1) ----
pub const EL_BYTES: usize = 16;

pub const PKT_STATE_CHANGE: u32 = 0x20;
pub const PKT_CMD_COMPLETION: u32 = 0x21;
pub const PKT_TX_EVENT: u32 = 0x22;
pub const PKT_EE_EVENT: u32 = 0x40;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    StateChange(u32),
    ExecEnv(u32),
    CmdCompletion { code: u32, ptr: u64 },
    Tx { chan: u32, code: u32, len: u32, ptr: u64 },
    Other { kind: u32, dword0: u32, dword1: u32 },
}

/// Decode one event ring element (as four little-endian dwords).
pub fn decode_event(el: [u32; 4]) -> Event {
    let ptr = el[0] as u64 | ((el[1] as u64) << 32);
    let (d0, d1) = (el[2], el[3]);
    let kind = (d1 >> 16) & 0xFF;
    match kind {
        PKT_STATE_CHANGE => Event::StateChange(d0 >> 24),
        PKT_EE_EVENT => Event::ExecEnv(d0 >> 24),
        PKT_CMD_COMPLETION => Event::CmdCompletion { code: d0 >> 24, ptr },
        PKT_TX_EVENT => Event::Tx { chan: d1 >> 24, code: d0 >> 24, len: d0 & 0xFFFF, ptr },
        _ => Event::Other { kind, dword0: d0, dword1: d1 },
    }
}

/// Ring of `n` 16-byte elements at device address `base`: the next element
/// address after `addr` (wrapping), or None if `addr` is not an element of it.
pub fn ring_next(base: u64, n: u64, addr: u64) -> Option<u64> {
    let len = n.checked_mul(EL_BYTES as u64)?;
    if addr < base || addr >= base + len || (addr - base) % EL_BYTES as u64 != 0 {
        return None;
    }
    let next = addr + EL_BYTES as u64;
    Some(if next == base + len { base } else { next })
}

/// Element index of `addr` in a ring at `base` (None when outside / unaligned).
pub fn ring_index(base: u64, n: u64, addr: u64) -> Option<usize> {
    let len = n.checked_mul(EL_BYTES as u64)?;
    if addr < base || addr >= base + len || (addr - base) % EL_BYTES as u64 != 0 {
        return None;
    }
    Some(((addr - base) / EL_BYTES as u64) as usize)
}
