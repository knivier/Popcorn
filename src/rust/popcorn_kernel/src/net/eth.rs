//! Ethernet II framing (no 802.1Q, no 802.3 length frames).

use super::{arr6, be16};

pub const HDR_LEN: usize = 14;
pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const BROADCAST: [u8; 6] = [0xFF; 6];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EthHeader {
    pub dst: [u8; 6],
    pub src: [u8; 6],
    pub ethertype: u16,
}

/// Split a frame into header and payload. None if shorter than the header.
/// The payload may include driver/wire padding; upper layers trim by their own
/// length fields.
pub fn parse(frame: &[u8]) -> Option<(EthHeader, &[u8])> {
    let hdr = EthHeader { dst: arr6(frame, 0)?, src: arr6(frame, 6)?, ethertype: be16(frame, 12)? };
    Some((hdr, frame.get(HDR_LEN..)?))
}

/// Write a header into `buf`. Returns `HDR_LEN`, or None if `buf` is too small.
pub fn write(buf: &mut [u8], dst: [u8; 6], src: [u8; 6], ethertype: u16) -> Option<usize> {
    let h = buf.get_mut(..HDR_LEN)?;
    h[0..6].copy_from_slice(&dst);
    h[6..12].copy_from_slice(&src);
    h[12..14].copy_from_slice(&ethertype.to_be_bytes());
    Some(HDR_LEN)
}

/// True when a frame addressed to `dst` should be accepted by `ours`.
#[inline]
pub fn accepts(dst: [u8; 6], ours: [u8; 6]) -> bool {
    dst == ours || dst == BROADCAST
}
