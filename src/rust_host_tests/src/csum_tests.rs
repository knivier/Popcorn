use crate::util::csum::{add, checksum, fold};

/// Byte-wise RFC 1071 reference.
fn reference(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

#[test]
fn rfc1071_example() {
    // RFC 1071 §3: 00 01 f2 03 f4 f5 f6 f7 sums (folded) to 0xddf2.
    let data = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
    assert_eq!(fold(add(0, &data)), 0xddf2);
    assert_eq!(checksum(&data), !0xddf2);
}

#[test]
fn matches_reference_all_lengths_and_alignments() {
    let mut buf = vec![0u8; 2100];
    let mut x: u32 = 0x1234_5678;
    for b in buf.iter_mut() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *b = x as u8;
    }
    for off in 0..8 {
        for len in 0..2000 {
            let d = &buf[off..off + len];
            assert_eq!(checksum(d), reference(d), "off={off} len={len}");
        }
    }
}

#[test]
fn verifies_to_zero() {
    // An IPv4 header with its checksum filled in sums to 0xFFFF (checksum() == 0).
    let mut hdr = [0x45u8, 0, 0, 0x54, 0x12, 0x34, 0x40, 0, 64, 1, 0, 0, 10, 0, 2, 15, 10, 0, 2, 2];
    let c = checksum(&hdr);
    hdr[10..12].copy_from_slice(&c.to_be_bytes());
    assert_eq!(checksum(&hdr), 0);
}

#[test]
fn all_ones_and_empty() {
    assert_eq!(checksum(&[]), 0xFFFF);
    assert_eq!(checksum(&[0xFF; 64]), 0);
}
