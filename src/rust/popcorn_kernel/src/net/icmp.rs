//! ICMP echo (RFC 792). Checksum covers the whole ICMP message.

use super::be16;
use crate::util::csum;

pub const HDR_LEN: usize = 8;
pub const TYPE_ECHO_REPLY: u8 = 0;
pub const TYPE_ECHO_REQUEST: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IcmpHeader {
    pub ty: u8,
    pub code: u8,
    /// Echo identifier (meaningful for echo request/reply only).
    pub id: u16,
    /// Echo sequence number (meaningful for echo request/reply only).
    pub seq: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IcmpError {
    Short,
    BadCsum,
}

/// Parse and verify an ICMP message (`msg` is the exact IPv4 payload).
pub fn parse(msg: &[u8]) -> Result<(IcmpHeader, &[u8]), IcmpError> {
    if msg.len() < HDR_LEN {
        return Err(IcmpError::Short);
    }
    if csum::checksum(msg) != 0 {
        return Err(IcmpError::BadCsum);
    }
    let h = IcmpHeader {
        ty: *msg.first().ok_or(IcmpError::Short)?,
        code: *msg.get(1).ok_or(IcmpError::Short)?,
        id: be16(msg, 4).ok_or(IcmpError::Short)?,
        seq: be16(msg, 6).ok_or(IcmpError::Short)?,
    };
    Ok((h, msg.get(HDR_LEN..).ok_or(IcmpError::Short)?))
}

/// Byte `i` of the echo-request payload pattern.
#[inline]
pub fn pattern_byte(i: usize) -> u8 {
    (i & 0xFF) as u8
}

fn finish(buf: &mut [u8], ty: u8, id: u16, seq: u16) {
    if let Some(h) = buf.get_mut(..HDR_LEN) {
        h[0] = ty;
        h[1] = 0;
        h[2] = 0;
        h[3] = 0;
        h[4..6].copy_from_slice(&id.to_be_bytes());
        h[6..8].copy_from_slice(&seq.to_be_bytes());
    }
    let c = csum::checksum(buf);
    if let Some(h) = buf.get_mut(2..4) {
        h.copy_from_slice(&c.to_be_bytes());
    }
}

/// Write an echo message carrying `payload` into exactly `buf[..8 + payload.len()]`.
/// Returns the message length, or None if `buf` is too small.
pub fn write_echo(buf: &mut [u8], ty: u8, id: u16, seq: u16, payload: &[u8]) -> Option<usize> {
    let len = HDR_LEN.checked_add(payload.len())?;
    let m = buf.get_mut(..len)?;
    m.get_mut(HDR_LEN..)?.copy_from_slice(payload);
    finish(m, ty, id, seq);
    Some(len)
}

/// Write an echo message whose payload is `payload_len` pattern bytes.
pub fn write_echo_pattern(buf: &mut [u8], ty: u8, id: u16, seq: u16, payload_len: usize) -> Option<usize> {
    let len = HDR_LEN.checked_add(payload_len)?;
    let m = buf.get_mut(..len)?;
    for (i, b) in m.get_mut(HDR_LEN..)?.iter_mut().enumerate() {
        *b = pattern_byte(i);
    }
    finish(m, ty, id, seq);
    Some(len)
}
