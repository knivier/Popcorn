//! QMI wire format and the WLFW (WLAN firmware) messages the ath11k family
//! uses. Pure `core` + `alloc`: host-tested.
//!
//! Message: 7-byte header (type u8, txn u16 LE, msg id u16 LE, length u16 LE)
//! then TLVs (type u8, length u16 LE, value). Integers little-endian; arrays
//! are a count (u8 or u16, per field) then the elements; structs are their
//! members back to back. Every response carries TLV 0x02 = (result u16,
//! error u16); result 0 is success.

use alloc::string::String;
use alloc::vec::Vec;

pub const HDR: usize = 7;
pub const REQUEST: u8 = 0;
pub const RESPONSE: u8 = 2;
pub const INDICATION: u8 = 4;

/// WLFW service on QRTR: id 0x45, version 1, instance 1 (WCN6855 / QCA6390).
pub const WLFW_SERVICE: u32 = 0x45;
pub const WLFW_VERSION: u32 = 1;
pub const WLFW_INSTANCE: u32 = 1;

// Message ids.
pub const IND_REGISTER: u16 = 0x0020;
pub const FW_READY_IND: u16 = 0x0021;
pub const WLAN_MODE: u16 = 0x0022;
pub const WLAN_CFG: u16 = 0x0023;
pub const CAP: u16 = 0x0024;
pub const BDF_DOWNLOAD: u16 = 0x0025;
pub const WLAN_INI: u16 = 0x002F;
pub const HOST_CAP: u16 = 0x0034;
pub const REQUEST_MEM_IND: u16 = 0x0035;
pub const RESPOND_MEM: u16 = 0x0036;
pub const FW_MEM_READY_IND: u16 = 0x0037;
pub const FW_INIT_DONE_IND: u16 = 0x0038;
pub const M3_INFO: u16 = 0x003C;
pub const COLD_BOOT_CAL_DONE_IND: u16 = 0x003E;

pub const CLIENT_ID: u32 = 0x4b4e_454c;
/// Largest BDF chunk per BDF_DOWNLOAD request.
pub const BDF_CHUNK: usize = 6144;
/// Board data is capped at this (hw_params board_size).
pub const BOARD_MAX: usize = 256 * 1024;
pub const BDF_TYPE_BIN: u8 = 0;
pub const BDF_TYPE_ELF: u8 = 1;
pub const BDF_TYPE_REGDB: u8 = 4;
pub const MAX_MEM_SEGS: usize = 52;

pub fn msg_name(id: u16) -> &'static str {
    match id {
        IND_REGISTER => "IND_REGISTER",
        FW_READY_IND => "FW_READY",
        WLAN_MODE => "WLAN_MODE",
        WLAN_CFG => "WLAN_CFG",
        CAP => "CAP",
        BDF_DOWNLOAD => "BDF_DOWNLOAD",
        WLAN_INI => "WLAN_INI",
        HOST_CAP => "HOST_CAP",
        REQUEST_MEM_IND => "REQUEST_MEM",
        RESPOND_MEM => "RESPOND_MEM",
        FW_MEM_READY_IND => "FW_MEM_READY",
        FW_INIT_DONE_IND => "FW_INIT_DONE",
        M3_INFO => "M3_INFO",
        COLD_BOOT_CAL_DONE_IND => "COLD_BOOT_CAL_DONE",
        _ => "?",
    }
}

/// Builds one message.
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new(kind: u8, txn: u16, id: u16) -> Writer {
        let mut buf = Vec::with_capacity(64);
        buf.push(kind);
        buf.extend_from_slice(&txn.to_le_bytes());
        buf.extend_from_slice(&id.to_le_bytes());
        buf.extend_from_slice(&[0, 0]);
        Writer { buf }
    }
    pub fn tlv(mut self, t: u8, v: &[u8]) -> Writer {
        self.buf.push(t);
        self.buf.extend_from_slice(&(v.len() as u16).to_le_bytes());
        self.buf.extend_from_slice(v);
        self
    }
    pub fn u8(self, t: u8, v: u8) -> Writer {
        self.tlv(t, &[v])
    }
    pub fn u32(self, t: u8, v: u32) -> Writer {
        self.tlv(t, &v.to_le_bytes())
    }
    pub fn u64(self, t: u8, v: u64) -> Writer {
        self.tlv(t, &v.to_le_bytes())
    }
    pub fn finish(mut self) -> Vec<u8> {
        let len = (self.buf.len() - HDR) as u16;
        self.buf[5..7].copy_from_slice(&len.to_le_bytes());
        self.buf
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Msg<'a> {
    pub kind: u8,
    pub txn: u16,
    pub id: u16,
    pub body: &'a [u8],
}

pub fn parse(b: &[u8]) -> Result<Msg<'_>, &'static str> {
    if b.len() < HDR {
        return Err("short QMI header");
    }
    let len = u16::from_le_bytes([b[5], b[6]]) as usize;
    if HDR + len > b.len() {
        return Err("QMI length past the packet");
    }
    Ok(Msg { kind: b[0], txn: u16::from_le_bytes([b[1], b[2]]), id: u16::from_le_bytes([b[3], b[4]]), body: &b[HDR..HDR + len] })
}

/// Value of TLV `t`, if present (bounds-checked walk; a malformed TLV ends it).
pub fn tlv(body: &[u8], t: u8) -> Option<&[u8]> {
    let mut o = 0;
    while o + 3 <= body.len() {
        let ty = body[o];
        let len = u16::from_le_bytes([body[o + 1], body[o + 2]]) as usize;
        let end = o + 3 + len;
        if end > body.len() {
            return None;
        }
        if ty == t {
            return Some(&body[o + 3..end]);
        }
        o = end;
    }
    None
}

/// (result, error) from the response TLV 0x02.
pub fn result(body: &[u8]) -> Option<(u16, u16)> {
    let v = tlv(body, 0x02)?;
    if v.len() < 4 {
        return None;
    }
    Some((u16::from_le_bytes([v[0], v[1]]), u16::from_le_bytes([v[2], v[3]])))
}

fn r32(v: &[u8], o: usize) -> Option<u32> {
    v.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

// ---------------------------------------------------------------- requests

pub fn ind_register_req(txn: u16) -> Vec<u8> {
    Writer::new(REQUEST, txn, IND_REGISTER)
        .u8(0x10, 1) // fw_ready_enable
        .u32(0x15, CLIENT_ID) // client_id
        .u8(0x16, 1) // request_mem_enable
        .u8(0x17, 1) // fw_mem_ready_enable
        .u8(0x18, 1) // fw_init_done_enable
        .u8(0x1B, 1) // cal_done_enable
        .finish()
}

/// nm_modem bits for WCN6855: host C-state, internal sleep clock, PCIe global
/// reset, PME from D3cold.
pub const NM_MODEM_WCN6855: u8 = 0x04 | 0x02 | 0x08 | 0x10;

pub fn host_cap_req(txn: u16, cal_done: bool) -> Vec<u8> {
    Writer::new(REQUEST, txn, HOST_CAP)
        .u32(0x10, 1) // num_clients
        .u8(0x13, NM_MODEM_WCN6855)
        .u8(0x14, 1) // bdf_support
        .u8(0x16, 1) // m3_support
        .u8(0x17, 1) // m3_cache_support
        .u8(0x1A, cal_done as u8)
        .u8(0x1C, 0) // mem_cfg_mode
        .finish()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MemSeg {
    pub size: u32,
    pub kind: u32,
}

/// Segments the firmware asks for (REQUEST_MEM indication, TLV 0x01).
pub fn request_mem_ind(body: &[u8]) -> Result<Vec<MemSeg>, &'static str> {
    let v = tlv(body, 0x01).ok_or("REQUEST_MEM without segments")?;
    let n = *v.first().ok_or("REQUEST_MEM empty")? as usize;
    if n > MAX_MEM_SEGS {
        return Err("REQUEST_MEM: too many segments");
    }
    let mut o = 1;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let size = r32(v, o).ok_or("REQUEST_MEM truncated")?;
        let kind = r32(v, o + 4).ok_or("REQUEST_MEM truncated")?;
        let cfgs = *v.get(o + 8).ok_or("REQUEST_MEM truncated")? as usize;
        if cfgs > 2 {
            return Err("REQUEST_MEM: bad mem_cfg count");
        }
        o += 9 + cfgs * 13; // mem_cfg: offset u64, size u32, secure u8
        if o > v.len() {
            return Err("REQUEST_MEM truncated");
        }
        out.push(MemSeg { size, kind });
    }
    Ok(out)
}

/// RESPOND_MEM: one (addr, size, type, restore=0) entry per segment.
pub fn respond_mem_req(txn: u16, segs: &[(u64, u32, u32)]) -> Vec<u8> {
    let mut v = Vec::with_capacity(1 + segs.len() * 17);
    v.push(segs.len() as u8);
    for &(addr, size, kind) in segs {
        v.extend_from_slice(&addr.to_le_bytes());
        v.extend_from_slice(&size.to_le_bytes());
        v.extend_from_slice(&kind.to_le_bytes());
        v.push(0);
    }
    Writer::new(REQUEST, txn, RESPOND_MEM).tlv(0x01, &v).finish()
}

pub fn cap_req(txn: u16) -> Vec<u8> {
    Writer::new(REQUEST, txn, CAP).finish()
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Cap {
    pub chip_id: Option<u32>,
    pub chip_family: Option<u32>,
    pub board_id: Option<u32>,
    pub soc_id: Option<u32>,
    pub fw_version: Option<u32>,
    pub fw_build_timestamp: String,
    pub fw_build_id: String,
    pub num_macs: Option<u8>,
    pub otp_version: Option<u32>,
    pub eeprom_read_timeout: Option<u32>,
}

fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end].iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '?' }).collect()
}

pub fn cap_resp(body: &[u8]) -> Cap {
    let mut c = Cap::default();
    if let Some(v) = tlv(body, 0x10) {
        c.chip_id = r32(v, 0);
        c.chip_family = r32(v, 4);
    }
    c.board_id = tlv(body, 0x11).and_then(|v| r32(v, 0));
    c.soc_id = tlv(body, 0x12).and_then(|v| r32(v, 0));
    if let Some(v) = tlv(body, 0x13) {
        c.fw_version = r32(v, 0);
        if let Some(&n) = v.get(4) {
            let end = (5 + n as usize).min(v.len());
            c.fw_build_timestamp = text(&v[5.min(v.len())..end]);
        }
    }
    if let Some(v) = tlv(body, 0x14) {
        c.fw_build_id = text(v);
    }
    c.num_macs = tlv(body, 0x15).and_then(|v| v.first().copied());
    c.otp_version = tlv(body, 0x18).and_then(|v| r32(v, 0));
    c.eeprom_read_timeout = tlv(body, 0x19).and_then(|v| r32(v, 0));
    c
}

/// One BDF_DOWNLOAD chunk.
pub fn bdf_req(txn: u16, file_id: u32, total: u32, seg: u32, data: &[u8], end: bool, kind: u8) -> Vec<u8> {
    let mut d = Vec::with_capacity(2 + data.len());
    d.extend_from_slice(&(data.len() as u16).to_le_bytes());
    d.extend_from_slice(data);
    Writer::new(REQUEST, txn, BDF_DOWNLOAD)
        .u8(0x01, 1) // valid
        .u32(0x10, file_id)
        .u32(0x11, total)
        .u32(0x12, seg)
        .tlv(0x13, &d)
        .u8(0x14, end as u8)
        .u8(0x15, kind)
        .finish()
}

pub fn m3_info_req(txn: u16, addr: u64, size: u32) -> Vec<u8> {
    Writer::new(REQUEST, txn, M3_INFO).u64(0x01, addr).u32(0x02, size).finish()
}

/// Board data type ath11k sends: ELF when the data is an ELF image.
pub fn bdf_type_of(data: &[u8]) -> u8 {
    if data.starts_with(b"\x7fELF") {
        BDF_TYPE_ELF
    } else {
        BDF_TYPE_BIN
    }
}

// ---------------------------------------------------------------- firmware start (W3a)

/// Target copy-engine table for WCN6855 / QCA6390 (ath11k
/// target_ce_config_wlan_qca6390): pipe, direction (1 in, 2 out, 3 in/out,
/// 4 in/out host-to-host), entries, max bytes, flags (8 = no interrupts).
pub const TARGET_CE: [[u32; 5]; 9] = [
    [0, 2, 32, 2048, 0],
    [1, 1, 32, 2048, 0],
    [2, 1, 32, 2048, 0],
    [3, 2, 32, 2048, 0],
    [4, 2, 256, 256, 8],
    [5, 1, 32, 2048, 0],
    [6, 3, 32, 16384, 0],
    [7, 4, 0, 0, 8],
    [8, 3, 32, 16384, 0],
];

/// HTC service ids.
pub const SVC_RSVD_CTRL: u32 = 0x0001;
pub const SVC_WMI_CONTROL: u32 = 0x0100;
pub const SVC_WMI_DATA_BE: u32 = 0x0101;
pub const SVC_WMI_DATA_BK: u32 = 0x0102;
pub const SVC_WMI_DATA_VI: u32 = 0x0103;
pub const SVC_WMI_DATA_VO: u32 = 0x0104;
pub const SVC_HTT_DATA_MSG: u32 = 0x0300;

/// Service-to-pipe map (ath11k target_service_to_ce_map_wlan_qca6390):
/// service, direction (2 host->target, 1 target->host), pipe.
pub const SVC_MAP: [[u32; 3]; 14] = [
    [SVC_WMI_DATA_VO, 2, 3],
    [SVC_WMI_DATA_VO, 1, 2],
    [SVC_WMI_DATA_BK, 2, 3],
    [SVC_WMI_DATA_BK, 1, 2],
    [SVC_WMI_DATA_BE, 2, 3],
    [SVC_WMI_DATA_BE, 1, 2],
    [SVC_WMI_DATA_VI, 2, 3],
    [SVC_WMI_DATA_VI, 1, 2],
    [SVC_WMI_CONTROL, 2, 3],
    [SVC_WMI_CONTROL, 1, 2],
    [SVC_RSVD_CTRL, 2, 0],
    [SVC_RSVD_CTRL, 1, 2],
    [SVC_HTT_DATA_MSG, 2, 4],
    [SVC_HTT_DATA_MSG, 1, 1],
];

/// Shadow register targets (shadow_reg_v2) ath11k computes for WCN6855: the
/// ring pointer register each host shadow slot i (BAR0 + 0x8fc + 4*i)
/// mirrors. 20 data-path rings, then the copy-engine rings.
pub const SHADOW_V2: [u32; 30] = [
    0x00a3_b034, 0x00a3_b03c, 0x00a3_b044, 0x00a3_b04c, // REO_DST 0-3
    0x00a3_b064, // REO_EXCEPTION
    0x00a3_b028, // REO_REINJECT
    0x00a3_b020, // REO_CMD
    0x00a3_b07c, // REO_STATUS
    0x00a4_6000, 0x00a4_6008, 0x00a4_6010, // TCL_DATA 0-2
    0x00a4_6018, // TCL_CMD
    0x00a4_6034, // TCL_STATUS
    0x00a3_70b0, // WBM_IDLE_LINK
    0x00a3_7018, // SW2WBM_RELEASE
    0x00a3_70c4, 0x00a3_70cc, 0x00a3_70d4, 0x00a3_70dc, 0x00a3_70e4, // WBM2SW 0-4
    0x01b8_0400, // CE0 SRC
    0x01b8_3400, 0x01b8_340c, // CE1 DST, DST_STATUS
    0x01b8_5400, 0x01b8_540c, // CE2 DST, DST_STATUS
    0x01b8_6400, // CE3 SRC
    0x01b8_8400, // CE4 SRC
    0x01b8_b400, 0x01b8_b40c, // CE5 DST, DST_STATUS
    0x01b8_e400, // CE7 SRC
];

pub const FIRMWARE_MODE_NORMAL: u32 = 0;
pub const FIRMWARE_MODE_OFF: u32 = 4;
pub const HOST_VERSION: &str = "WIN";

pub fn wlan_ini_req(txn: u16, fw_log: bool) -> Vec<u8> {
    Writer::new(REQUEST, txn, WLAN_INI).u8(0x10, fw_log as u8).finish()
}

pub fn wlan_cfg_req(txn: u16) -> Vec<u8> {
    let mut ce = Vec::with_capacity(1 + TARGET_CE.len() * 20);
    ce.push(TARGET_CE.len() as u8);
    for row in TARGET_CE {
        for v in row {
            ce.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut svc = Vec::with_capacity(1 + SVC_MAP.len() * 12);
    svc.push(SVC_MAP.len() as u8);
    for row in SVC_MAP {
        for v in row {
            svc.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut sh = Vec::with_capacity(1 + SHADOW_V2.len() * 4);
    sh.push(SHADOW_V2.len() as u8);
    for v in SHADOW_V2 {
        sh.extend_from_slice(&v.to_le_bytes());
    }
    Writer::new(REQUEST, txn, WLAN_CFG)
        .tlv(0x10, HOST_VERSION.as_bytes())
        .tlv(0x11, &ce)
        .tlv(0x12, &svc)
        .tlv(0x14, &sh)
        .finish()
}

pub fn wlan_mode_req(txn: u16, mode: u32) -> Vec<u8> {
    Writer::new(REQUEST, txn, WLAN_MODE).u32(0x01, mode).u8(0x10, 0).finish()
}
