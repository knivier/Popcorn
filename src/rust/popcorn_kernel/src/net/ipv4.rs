//! IPv4 (RFC 791): header parse/verify and build. No fragmentation or reassembly.

use super::{arr4, be16};
use crate::util::csum;

pub const HDR_LEN: usize = 20;
/// Largest IPv4 packet we send (Ethernet MTU).
pub const MAX_PACKET: usize = 1500;
pub const PROTO_ICMP: u8 = 1;
pub const PROTO_UDP: u8 = 17;
pub const TTL: u8 = 64;
const FLAG_DF: u16 = 0x4000;
const FLAG_MF: u16 = 0x2000;
const FRAG_OFFSET_MASK: u16 = 0x1FFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Header {
    /// Header length in bytes (IHL * 4).
    pub hdr_len: usize,
    pub total_len: usize,
    pub id: u16,
    pub ttl: u8,
    pub proto: u8,
    pub src: [u8; 4],
    pub dst: [u8; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ipv4Error {
    /// Shorter than 20 bytes, or than IHL * 4.
    Short,
    /// Version != 4 or IHL < 5.
    BadHeader,
    /// Header checksum wrong.
    BadCsum,
    /// Total length < header length or > bytes available.
    BadLen,
    /// MF set or fragment offset != 0.
    Fragment,
}

/// Parse and verify an IPv4 packet. Returns the header and the payload trimmed to
/// the total-length field (Ethernet padding removed). Options are skipped.
pub fn parse(p: &[u8]) -> Result<(Ipv4Header, &[u8]), Ipv4Error> {
    let vihl = *p.first().ok_or(Ipv4Error::Short)?;
    if p.len() < HDR_LEN {
        return Err(Ipv4Error::Short);
    }
    let ihl = (vihl & 0x0F) as usize;
    if vihl >> 4 != 4 || ihl < 5 {
        return Err(Ipv4Error::BadHeader);
    }
    let hdr_len = ihl * 4;
    let hdr = p.get(..hdr_len).ok_or(Ipv4Error::Short)?;
    if csum::checksum(hdr) != 0 {
        return Err(Ipv4Error::BadCsum);
    }
    let total_len = be16(p, 2).ok_or(Ipv4Error::Short)? as usize;
    if total_len < hdr_len || total_len > p.len() {
        return Err(Ipv4Error::BadLen);
    }
    let frag = be16(p, 6).ok_or(Ipv4Error::Short)?;
    if frag & FLAG_MF != 0 || frag & FRAG_OFFSET_MASK != 0 {
        return Err(Ipv4Error::Fragment);
    }
    let h = Ipv4Header {
        hdr_len,
        total_len,
        id: be16(p, 4).ok_or(Ipv4Error::Short)?,
        ttl: *p.get(8).ok_or(Ipv4Error::Short)?,
        proto: *p.get(9).ok_or(Ipv4Error::Short)?,
        src: arr4(p, 12).ok_or(Ipv4Error::Short)?,
        dst: arr4(p, 16).ok_or(Ipv4Error::Short)?,
    };
    let payload = p.get(hdr_len..total_len).ok_or(Ipv4Error::BadLen)?;
    Ok((h, payload))
}

/// Write a 20-byte header (TTL 64, DF, checksum filled) for a packet carrying
/// `payload_len` bytes. Returns `HDR_LEN`, or None if `buf` is too small or the
/// packet would exceed `MAX_PACKET`.
pub fn write(buf: &mut [u8], src: [u8; 4], dst: [u8; 4], proto: u8, id: u16, payload_len: usize) -> Option<usize> {
    let total = HDR_LEN.checked_add(payload_len)?;
    if total > MAX_PACKET {
        return None;
    }
    let h = buf.get_mut(..HDR_LEN)?;
    h[0] = 0x45;
    h[1] = 0;
    h[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    h[4..6].copy_from_slice(&id.to_be_bytes());
    h[6..8].copy_from_slice(&FLAG_DF.to_be_bytes());
    h[8] = TTL;
    h[9] = proto;
    h[10] = 0;
    h[11] = 0;
    h[12..16].copy_from_slice(&src);
    h[16..20].copy_from_slice(&dst);
    let c = csum::checksum(h);
    h[10..12].copy_from_slice(&c.to_be_bytes());
    Some(HDR_LEN)
}

/// Netmask bits for a prefix length (clamped to 0..=32).
#[inline]
pub fn mask(prefix: u8) -> u32 {
    match prefix {
        0 => 0,
        p if p >= 32 => u32::MAX,
        p => u32::MAX << (32 - p as u32),
    }
}

/// True when `a` and `b` share the first `prefix` bits.
#[inline]
pub fn same_subnet(a: [u8; 4], b: [u8; 4], prefix: u8) -> bool {
    let m = mask(prefix);
    u32::from_be_bytes(a) & m == u32::from_be_bytes(b) & m
}

#[inline]
pub fn is_broadcast(a: [u8; 4]) -> bool {
    a == [255; 4]
}

/// Addresses that must never be a unicast source (0/8, multicast, class E, broadcast).
#[inline]
pub fn is_bad_source(a: [u8; 4]) -> bool {
    a[0] == 0 || a[0] >= 224
}
