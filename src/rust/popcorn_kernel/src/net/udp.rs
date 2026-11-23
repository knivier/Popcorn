//! UDP (RFC 768): header parse/verify and build with the IPv4 pseudo-header
//! checksum. Socket state lives in `Iface`; this module is wire format only.

use super::be16;
use super::ipv4::PROTO_UDP;
use crate::util::csum;

pub const HDR_LEN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    /// UDP length field (header + payload).
    pub len: usize,
    /// Checksum field as received (0 = sender computed none).
    pub csum: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UdpError {
    /// Fewer than 8 bytes.
    Short,
    /// Length field < 8 or larger than the IPv4 payload.
    BadLen,
    /// Non-zero checksum that does not verify.
    BadCsum,
}

/// One's-complement sum of the pseudo header (not folded).
fn pseudo(src: [u8; 4], dst: [u8; 4], udp_len: usize) -> u64 {
    let mut ph = [0u8; 12];
    ph[0..4].copy_from_slice(&src);
    ph[4..8].copy_from_slice(&dst);
    ph[9] = PROTO_UDP;
    ph[10..12].copy_from_slice(&(udp_len as u16).to_be_bytes());
    csum::add(0, &ph)
}

/// Checksum field value for the UDP segment `seg` (header + payload, with the
/// checksum field zeroed). Never returns 0: a computed 0 is sent as 0xFFFF.
pub fn checksum(src: [u8; 4], dst: [u8; 4], seg: &[u8]) -> u16 {
    let c = !csum::fold(csum::add(pseudo(src, dst, seg.len()), seg));
    if c == 0 {
        0xFFFF
    } else {
        c
    }
}

/// True when `seg` (with its checksum field as received) verifies.
pub fn verify(src: [u8; 4], dst: [u8; 4], seg: &[u8]) -> bool {
    !csum::fold(csum::add(pseudo(src, dst, seg.len()), seg)) == 0
}

/// Parse and verify a UDP segment (`seg` is the exact IPv4 payload). Returns the
/// header and the payload trimmed to the UDP length field. A zero checksum means
/// "none" (IPv4) and is accepted unverified.
pub fn parse(src: [u8; 4], dst: [u8; 4], seg: &[u8]) -> Result<(UdpHeader, &[u8]), UdpError> {
    if seg.len() < HDR_LEN {
        return Err(UdpError::Short);
    }
    let h = UdpHeader {
        src_port: be16(seg, 0).ok_or(UdpError::Short)?,
        dst_port: be16(seg, 2).ok_or(UdpError::Short)?,
        len: be16(seg, 4).ok_or(UdpError::Short)? as usize,
        csum: be16(seg, 6).ok_or(UdpError::Short)?,
    };
    if h.len < HDR_LEN || h.len > seg.len() {
        return Err(UdpError::BadLen);
    }
    let seg = seg.get(..h.len).ok_or(UdpError::BadLen)?;
    if h.csum != 0 && !verify(src, dst, seg) {
        return Err(UdpError::BadCsum);
    }
    Ok((h, seg.get(HDR_LEN..).ok_or(UdpError::Short)?))
}

/// Write a UDP segment carrying `payload` into exactly `buf[..8 + payload.len()]`
/// with the checksum filled. Returns the segment length, or None if `buf` is too
/// small or the length does not fit the 16-bit field.
pub fn write(buf: &mut [u8], src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16, payload: &[u8]) -> Option<usize> {
    let len = HDR_LEN.checked_add(payload.len())?;
    if len > u16::MAX as usize {
        return None;
    }
    let seg = buf.get_mut(..len)?;
    seg[0..2].copy_from_slice(&sport.to_be_bytes());
    seg[2..4].copy_from_slice(&dport.to_be_bytes());
    seg[4..6].copy_from_slice(&(len as u16).to_be_bytes());
    seg[6] = 0;
    seg[7] = 0;
    seg.get_mut(HDR_LEN..)?.copy_from_slice(payload);
    let c = checksum(src, dst, seg);
    seg[6..8].copy_from_slice(&c.to_be_bytes());
    Some(len)
}
