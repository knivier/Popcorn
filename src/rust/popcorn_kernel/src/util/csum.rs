//! RFC 1071 Internet checksum, u64 accumulate then fold. Pure Rust: the host test
//! crate includes this file directly (`#[path]`).

/// Add `data` (big-endian 16-bit words; odd tail padded with zero) to `acc`.
pub fn add(mut acc: u64, data: &[u8]) -> u64 {
    let mut chunks = data.chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
        let (s, carry) = acc.overflowing_add(w);
        acc = s + carry as u64;
    }
    let mut rest = chunks.remainder();
    while rest.len() >= 2 {
        let (s, carry) = acc.overflowing_add(u16::from_be_bytes([rest[0], rest[1]]) as u64);
        acc = s + carry as u64;
        rest = &rest[2..];
    }
    if let [b] = rest {
        let (s, carry) = acc.overflowing_add((*b as u64) << 8);
        acc = s + carry as u64;
    }
    acc
}

/// Fold a 64-bit accumulator to the 16-bit one's-complement sum (not inverted).
pub fn fold(mut acc: u64) -> u16 {
    acc = (acc & 0xFFFF_FFFF) + (acc >> 32);
    acc = (acc & 0xFFFF_FFFF) + (acc >> 32);
    acc = (acc & 0xFFFF) + (acc >> 16);
    acc = (acc & 0xFFFF) + (acc >> 16);
    acc as u16
}

/// Checksum field value for `data` (inverted fold): 0 when `data` already holds a
/// correct checksum.
pub fn checksum(data: &[u8]) -> u16 {
    !fold(add(0, data))
}
