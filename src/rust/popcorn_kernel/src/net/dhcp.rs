//! DHCP client wire format (RFC 2131 / RFC 2132). The client state machine lives in
//! `Iface` (`dhcp_start`, driven by `poll`); this module builds requests and
//! parses server replies. Untrusted input never panics.

use super::{arr4, be16};

pub const CLIENT_PORT: u16 = 68;
pub const SERVER_PORT: u16 = 67;
pub const MAGIC: [u8; 4] = [99, 130, 83, 99];
/// BOOTP flags: ask the server to broadcast its replies (we have no address yet).
pub const FLAG_BROADCAST: u16 = 0x8000;

pub const OP_REQUEST: u8 = 1;
pub const OP_REPLY: u8 = 2;

// Message types (option 53).
pub const DISCOVER: u8 = 1;
pub const OFFER: u8 = 2;
pub const REQUEST: u8 = 3;
pub const DECLINE: u8 = 4;
pub const ACK: u8 = 5;
pub const NAK: u8 = 6;
pub const RELEASE: u8 = 7;

// Options.
pub const OPT_PAD: u8 = 0;
pub const OPT_SUBNET: u8 = 1;
pub const OPT_ROUTER: u8 = 3;
pub const OPT_DNS: u8 = 6;
pub const OPT_REQUESTED_IP: u8 = 50;
pub const OPT_LEASE: u8 = 51;
pub const OPT_MSG_TYPE: u8 = 53;
pub const OPT_SERVER_ID: u8 = 54;
pub const OPT_PARAM_LIST: u8 = 55;
pub const OPT_T1: u8 = 58;
pub const OPT_T2: u8 = 59;
pub const OPT_END: u8 = 255;

/// Offset of the magic cookie (end of the fixed BOOTP header).
pub const COOKIE_OFF: usize = 236;
/// Offset of the first option.
pub const OPTIONS_OFF: usize = 240;
/// Requests are padded to the classic BOOTP minimum.
pub const MIN_LEN: usize = 300;
/// Largest message we build (fits the RFC 2131 576-byte guarantee).
pub const MAX_LEN: usize = 576 - 28;

/// What the client puts in a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DhcpRequest {
    pub msg_type: u8,
    pub xid: u32,
    pub secs: u16,
    pub flags: u16,
    pub ciaddr: [u8; 4],
    pub chaddr: [u8; 6],
    /// Option 50 (only in SELECTING/INIT-REBOOT REQUESTs).
    pub requested_ip: Option<[u8; 4]>,
    /// Option 54 (only in SELECTING REQUESTs).
    pub server_id: Option<[u8; 4]>,
}

/// The fields of a server reply that the client uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DhcpReply {
    pub op: u8,
    pub xid: u32,
    pub yiaddr: [u8; 4],
    pub chaddr: [u8; 6],
    pub msg_type: Option<u8>,
    pub server_id: Option<[u8; 4]>,
    pub subnet_mask: Option<[u8; 4]>,
    pub router: Option<[u8; 4]>,
    pub dns: Option<[u8; 4]>,
    pub lease_s: Option<u32>,
    pub t1_s: Option<u32>,
    pub t2_s: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DhcpError {
    /// Shorter than the fixed header + cookie.
    Short,
    /// Not Ethernet (htype 1 / hlen 6) or the magic cookie is wrong.
    BadHeader,
    /// An option runs past the end of the message.
    BadOption,
}

/// Build a client message into `buf`. Returns its length (≥ `MIN_LEN`), or None if
/// `buf` is too small.
pub fn build(buf: &mut [u8], r: &DhcpRequest) -> Option<usize> {
    let m = buf.get_mut(..MIN_LEN)?;
    m.fill(0);
    m[0] = OP_REQUEST;
    m[1] = 1; // htype Ethernet
    m[2] = 6; // hlen
    m[3] = 0; // hops
    m[4..8].copy_from_slice(&r.xid.to_be_bytes());
    m[8..10].copy_from_slice(&r.secs.to_be_bytes());
    m[10..12].copy_from_slice(&r.flags.to_be_bytes());
    m[12..16].copy_from_slice(&r.ciaddr);
    // yiaddr, siaddr, giaddr zero
    m[28..34].copy_from_slice(&r.chaddr);
    m[COOKIE_OFF..OPTIONS_OFF].copy_from_slice(&MAGIC);
    let mut o = OPTIONS_OFF;
    let mut put = |m: &mut [u8], bytes: &[u8]| -> Option<()> {
        m.get_mut(o..o + bytes.len())?.copy_from_slice(bytes);
        o += bytes.len();
        Some(())
    };
    put(m, &[OPT_MSG_TYPE, 1, r.msg_type])?;
    if let Some(ip) = r.requested_ip {
        put(m, &[OPT_REQUESTED_IP, 4, ip[0], ip[1], ip[2], ip[3]])?;
    }
    if let Some(ip) = r.server_id {
        put(m, &[OPT_SERVER_ID, 4, ip[0], ip[1], ip[2], ip[3]])?;
    }
    if r.msg_type == DISCOVER || r.msg_type == REQUEST {
        put(m, &[OPT_PARAM_LIST, 4, OPT_SUBNET, OPT_ROUTER, OPT_DNS, OPT_LEASE])?;
    }
    put(m, &[OPT_END])?;
    Some(MIN_LEN)
}

fn opt_u32(v: &[u8]) -> Option<u32> {
    let b = v.get(..4)?;
    if v.len() != 4 {
        return None;
    }
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// First address of an address-list option (length a non-zero multiple of 4).
fn opt_addr(v: &[u8]) -> Option<[u8; 4]> {
    if v.is_empty() || v.len() % 4 != 0 {
        return None;
    }
    arr4(v, 0)
}

/// Parse a DHCP/BOOTP message (`p` is the UDP payload). Options with a malformed
/// length are ignored; an option running past the end of the message is an error.
/// Option overload (52) is not honoured (`sname`/`file` are never parsed).
pub fn parse(p: &[u8]) -> Result<DhcpReply, DhcpError> {
    if p.len() < OPTIONS_OFF {
        return Err(DhcpError::Short);
    }
    if p.get(1) != Some(&1) || p.get(2) != Some(&6) || p.get(COOKIE_OFF..OPTIONS_OFF) != Some(&MAGIC[..]) {
        return Err(DhcpError::BadHeader);
    }
    let mut r = DhcpReply {
        op: *p.first().ok_or(DhcpError::Short)?,
        xid: u32::from_be_bytes(arr4(p, 4).ok_or(DhcpError::Short)?),
        yiaddr: arr4(p, 16).ok_or(DhcpError::Short)?,
        ..DhcpReply::default()
    };
    if let Some(c) = p.get(28..34) {
        r.chaddr.copy_from_slice(c);
    }
    let mut o = OPTIONS_OFF;
    loop {
        let Some(&code) = p.get(o) else { break }; // missing END is tolerated
        if code == OPT_END {
            break;
        }
        if code == OPT_PAD {
            o += 1;
            continue;
        }
        let len = *p.get(o + 1).ok_or(DhcpError::BadOption)? as usize;
        let v = p.get(o + 2..o + 2 + len).ok_or(DhcpError::BadOption)?;
        match code {
            OPT_MSG_TYPE if len == 1 => r.msg_type = v.first().copied(),
            OPT_SERVER_ID if len == 4 => r.server_id = arr4(v, 0),
            OPT_SUBNET if len == 4 => r.subnet_mask = arr4(v, 0),
            OPT_ROUTER => r.router = opt_addr(v).or(r.router),
            OPT_DNS => r.dns = opt_addr(v).or(r.dns),
            OPT_LEASE => r.lease_s = opt_u32(v).or(r.lease_s),
            OPT_T1 => r.t1_s = opt_u32(v).or(r.t1_s),
            OPT_T2 => r.t2_s = opt_u32(v).or(r.t2_s),
            _ => {}
        }
        o += 2 + len;
    }
    Ok(r)
}

/// Prefix length of a contiguous netmask, or None if it is not contiguous.
pub fn mask_to_prefix(mask: [u8; 4]) -> Option<u8> {
    let m = u32::from_be_bytes(mask);
    let ones = m.leading_ones();
    if m.checked_shl(ones).unwrap_or(0) != 0 {
        return None;
    }
    Some(ones as u8)
}

/// Classful default prefix when the server sends no subnet mask.
pub fn classful_prefix(ip: [u8; 4]) -> u8 {
    match ip[0] {
        0..=127 => 8,
        128..=191 => 16,
        _ => 24,
    }
}

/// `secs` field helper: whole seconds since `start_ms`, saturated to u16.
pub fn secs_since(start_ms: u64, now_ms: u64) -> u16 {
    (now_ms.saturating_sub(start_ms) / 1000).min(u16::MAX as u64) as u16
}

/// Read the `secs`/`flags` of a client message (tests and debugging).
pub fn flags_of(p: &[u8]) -> Option<u16> {
    be16(p, 10)
}
