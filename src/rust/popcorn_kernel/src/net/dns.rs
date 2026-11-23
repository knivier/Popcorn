//! DNS stub resolver wire format (RFC 1035): A queries and robust response
//! parsing (compression pointers with loop/bounds protection, CNAME chains).
//! Query state, retries and the cache live in `Iface`. Untrusted input never panics.

use core::fmt;

use super::be16;
use super::iface::NetError;

pub const PORT: u16 = 53;
pub const HDR_LEN: usize = 12;
/// Largest message over UDP without EDNS.
pub const MAX_MSG: usize = 512;
/// Longest name in presentation form (no trailing dot).
pub const MAX_NAME: usize = 253;
/// Longest name in wire form (labels + length bytes + root).
pub const MAX_WIRE_NAME: usize = 255;
pub const MAX_LABEL: usize = 63;
pub const TYPE_A: u16 = 1;
pub const TYPE_CNAME: u16 = 5;
pub const CLASS_IN: u16 = 1;
/// Compression pointers followed while reading one name at most.
const MAX_JUMPS: usize = 32;
/// CNAME links followed at most.
pub const MAX_CNAME_CHAIN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsError {
    /// Empty, too long, a label too long/empty, or a character outside [A-Za-z0-9-_].
    BadName,
    /// RCODE 3: the name does not exist.
    NxDomain,
    /// RCODE 2.
    ServFail,
    /// RCODE 5.
    Refused,
    /// Any other non-zero RCODE.
    Rcode(u8),
    /// NOERROR but no A record for the name (after CNAMEs).
    NoAnswer,
    /// No usable answer after every attempt.
    Timeout,
    /// No DNS server configured.
    NoServer,
    /// Every query slot is in flight.
    Busy,
    /// The query could not be sent (no address, no route, ...).
    Net(NetError),
}

impl DnsError {
    /// Short lowercase reason for log lines.
    pub fn reason(&self) -> &'static str {
        match self {
            DnsError::BadName => "badname",
            DnsError::NxDomain => "nxdomain",
            DnsError::ServFail => "servfail",
            DnsError::Refused => "refused",
            DnsError::Rcode(_) => "rcode",
            DnsError::NoAnswer => "noanswer",
            DnsError::Timeout => "timeout",
            DnsError::NoServer => "noserver",
            DnsError::Busy => "busy",
            DnsError::Net(_) => "net",
        }
    }
}

/// A name in wire form, lowercased (`\x03www\x06google\x03com\x00`).
#[derive(Clone, Copy)]
pub struct WireName {
    pub len: u16,
    pub buf: [u8; MAX_WIRE_NAME],
}

impl WireName {
    pub const EMPTY: WireName = WireName { len: 0, buf: [0; MAX_WIRE_NAME] };

    pub fn as_bytes(&self) -> &[u8] {
        self.buf.get(..self.len as usize).unwrap_or(&[])
    }
    pub fn eq_bytes(&self, other: &[u8]) -> bool {
        self.as_bytes() == other
    }
}

impl fmt::Display for WireName {
    /// Presentation form without the trailing dot (`www.google.com`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.as_bytes();
        let mut i = 0usize;
        let mut first = true;
        while let Some(&l) = b.get(i) {
            if l == 0 {
                break;
            }
            let Some(label) = b.get(i + 1..i + 1 + l as usize) else { break };
            if !first {
                f.write_str(".")?;
            }
            first = false;
            for &c in label {
                if c.is_ascii_graphic() {
                    write!(f, "{}", c as char)?;
                } else {
                    f.write_str("?")?;
                }
            }
            i += 1 + l as usize;
        }
        Ok(())
    }
}

fn valid_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_'
}

/// Validate a presentation-form name (one trailing dot allowed) and encode it in
/// lowercase wire form.
pub fn encode_name(name: &str) -> Result<WireName, DnsError> {
    let b = name.as_bytes();
    let b = b.strip_suffix(b".").unwrap_or(b);
    if b.is_empty() || b.len() > MAX_NAME {
        return Err(DnsError::BadName);
    }
    let mut w = WireName::EMPTY;
    let mut o = 0usize;
    for label in b.split(|&c| c == b'.') {
        if label.is_empty() || label.len() > MAX_LABEL || !label.iter().all(|&c| valid_char(c)) {
            return Err(DnsError::BadName);
        }
        let dst = w.buf.get_mut(o..o + 1 + label.len()).ok_or(DnsError::BadName)?;
        dst[0] = label.len() as u8;
        for (d, s) in dst[1..].iter_mut().zip(label) {
            *d = s.to_ascii_lowercase();
        }
        o += 1 + label.len();
    }
    *w.buf.get_mut(o).ok_or(DnsError::BadName)? = 0;
    w.len = (o + 1) as u16;
    Ok(w)
}

/// Parse a dotted-quad address (`8.8.8.8`): four decimal parts 0..=255, 1–3 digits.
pub fn parse_dotted_quad(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for o in out.iter_mut() {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let v: u32 = p.bytes().fold(0, |a, c| a * 10 + (c - b'0') as u32);
        *o = u8::try_from(v).ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// Build an A/IN query with RD set. Returns the message length.
pub fn build_query(buf: &mut [u8], id: u16, name: &WireName) -> Option<usize> {
    let n = name.as_bytes();
    let len = HDR_LEN + n.len() + 4;
    let m = buf.get_mut(..len)?;
    m[0..2].copy_from_slice(&id.to_be_bytes());
    m[2..4].copy_from_slice(&0x0100u16.to_be_bytes()); // RD
    m[4..6].copy_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    m[6..12].fill(0);
    m.get_mut(HDR_LEN..HDR_LEN + n.len())?.copy_from_slice(n);
    let q = HDR_LEN + n.len();
    m[q..q + 2].copy_from_slice(&TYPE_A.to_be_bytes());
    m[q + 2..q + 4].copy_from_slice(&CLASS_IN.to_be_bytes());
    Some(len)
}

/// Read the (possibly compressed) name at `off` into `out` in lowercase wire form.
/// Returns the offset just past the name in the original position. Rejects pointer
/// loops (jump limit; pointers must go strictly backwards), out-of-bounds reads,
/// reserved label types and names longer than 255 bytes.
pub fn read_name(msg: &[u8], off: usize, out: &mut WireName) -> Option<usize> {
    let mut pos = off;
    let mut end: Option<usize> = None;
    let mut jumps = 0usize;
    let mut o = 0usize;
    loop {
        let l = *msg.get(pos)?;
        match l & 0xC0 {
            0x00 => {
                if l == 0 {
                    *out.buf.get_mut(o)? = 0;
                    out.len = (o + 1) as u16;
                    return Some(end.unwrap_or(pos + 1));
                }
                let n = l as usize;
                let label = msg.get(pos + 1..pos + 1 + n)?;
                // Room for this label plus the final root byte.
                if o + 1 + n + 1 > MAX_WIRE_NAME {
                    return None;
                }
                let dst = out.buf.get_mut(o..o + 1 + n)?;
                dst[0] = l;
                for (d, s) in dst[1..].iter_mut().zip(label) {
                    *d = s.to_ascii_lowercase();
                }
                o += 1 + n;
                pos += 1 + n;
            }
            0xC0 => {
                let target = (be16(msg, pos)? & 0x3FFF) as usize;
                if end.is_none() {
                    end = Some(pos + 2);
                }
                jumps += 1;
                if jumps > MAX_JUMPS || target >= pos {
                    return None;
                }
                pos = target;
            }
            _ => return None, // 0x40 / 0x80: reserved / extended label types
        }
    }
}

/// Header fields of a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DnsHeader {
    pub id: u16,
    pub flags: u16,
    pub qdcount: u16,
    pub ancount: u16,
}

impl DnsHeader {
    pub fn is_response(&self) -> bool {
        self.flags & 0x8000 != 0
    }
    pub fn opcode(&self) -> u8 {
        ((self.flags >> 11) & 0xF) as u8
    }
    pub fn rcode(&self) -> u8 {
        (self.flags & 0xF) as u8
    }
}

pub fn parse_header(msg: &[u8]) -> Option<DnsHeader> {
    Some(DnsHeader { id: be16(msg, 0)?, flags: be16(msg, 2)?, qdcount: be16(msg, 4)?, ancount: be16(msg, 6)? })
}

/// Why a response was not accepted for a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    /// Not a response to this query (id, flags, question): ignore and keep waiting.
    Mismatch,
    /// Matching but structurally broken: ignore and keep waiting.
    Malformed,
}

/// Outcome of a matching response: the address and its TTL (seconds, the minimum
/// along the CNAME chain), or a definitive error.
pub type Answer = Result<([u8; 4], u32), DnsError>;

/// Match `msg` against the query (`id`, `qname`) and extract the answer.
pub fn parse_response(msg: &[u8], id: u16, qname: &WireName) -> Result<Answer, Reject> {
    let h = parse_header(msg).ok_or(Reject::Malformed)?;
    if h.id != id || !h.is_response() || h.opcode() != 0 || h.qdcount != 1 {
        return Err(Reject::Mismatch);
    }
    let mut name = WireName::EMPTY;
    let mut pos = read_name(msg, HDR_LEN, &mut name).ok_or(Reject::Malformed)?;
    let qtype = be16(msg, pos).ok_or(Reject::Malformed)?;
    let qclass = be16(msg, pos + 2).ok_or(Reject::Malformed)?;
    if !name.eq_bytes(qname.as_bytes()) || qtype != TYPE_A || qclass != CLASS_IN {
        return Err(Reject::Mismatch);
    }
    pos += 4;
    match h.rcode() {
        0 => {}
        2 => return Ok(Err(DnsError::ServFail)),
        3 => return Ok(Err(DnsError::NxDomain)),
        5 => return Ok(Err(DnsError::Refused)),
        r => return Ok(Err(DnsError::Rcode(r))),
    }
    let answers_start = pos;
    // Validate the whole answer section once (so the chain walk below can trust it).
    for _ in 0..h.ancount {
        pos = read_name(msg, pos, &mut name).ok_or(Reject::Malformed)?;
        let rdlen = be16(msg, pos + 8).ok_or(Reject::Malformed)? as usize;
        pos = pos.checked_add(10 + rdlen).ok_or(Reject::Malformed)?;
        if pos > msg.len() {
            return Err(Reject::Malformed);
        }
    }
    // Follow CNAMEs from the question name; the first A record for the current
    // target wins. Records may appear in any order, so rescan after each link.
    let mut target = *qname;
    let mut ttl_min = u32::MAX;
    let mut cname = WireName::EMPTY;
    for _ in 0..=MAX_CNAME_CHAIN {
        let mut p = answers_start;
        let mut next: Option<(WireName, u32)> = None;
        for _ in 0..h.ancount {
            p = read_name(msg, p, &mut name).ok_or(Reject::Malformed)?;
            let ty = be16(msg, p).ok_or(Reject::Malformed)?;
            let class = be16(msg, p + 2).ok_or(Reject::Malformed)?;
            let ttl = u32::from_be_bytes(super::arr4(msg, p + 4).ok_or(Reject::Malformed)?);
            // RFC 2181 §8: a TTL with the top bit set is treated as zero.
            let ttl = if ttl & 0x8000_0000 != 0 { 0 } else { ttl };
            let rdlen = be16(msg, p + 8).ok_or(Reject::Malformed)? as usize;
            let rd = p + 10;
            p = rd + rdlen;
            if class != CLASS_IN || !name.eq_bytes(target.as_bytes()) {
                continue;
            }
            if ty == TYPE_A && rdlen == 4 {
                let a = super::arr4(msg, rd).ok_or(Reject::Malformed)?;
                return Ok(Ok((a, ttl_min.min(ttl))));
            }
            if ty == TYPE_CNAME && next.is_none() {
                let e = read_name(msg, rd, &mut cname).ok_or(Reject::Malformed)?;
                if e != p {
                    return Err(Reject::Malformed);
                }
                next = Some((cname, ttl));
            }
        }
        match next {
            Some((n, ttl)) => {
                target = n;
                ttl_min = ttl_min.min(ttl);
            }
            None => break,
        }
    }
    Ok(Err(DnsError::NoAnswer))
}
