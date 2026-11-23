//! Network protocol stack (Ethernet II, ARP, IPv4, ICMP echo, UDP, DHCP client,
//! DNS stub resolver).
//!
//! Pure `core`: no heap, no `static mut`, no FFI, no asm. The only hardware view is
//! the `crate::drivers::net::NetDevice` trait, passed per call; `Iface` owns only
//! protocol state. The host test crate includes these files by `#[path]`.
//!
//! Untrusted input never panics: every parser checks lengths before reading, and
//! malformed frames are counted in `StackStats` and dropped.

pub mod arp;
pub mod dhcp;
pub mod dns;
pub mod eth;
pub mod icmp;
pub mod iface;
pub mod ipv4;
pub mod udp;

pub use dns::DnsError;
pub use iface::{DhcpState, Iface, IpConfig, NetError, QueryId, SockId, StackStats};

/// Line buffer size: fits `net: dns <253-char name> fail <reason>`.
const LINE_MAX: usize = 320;

/// 32-bit integer mixer (murmur3 finaliser) for xids, DNS ids and ephemeral ports.
/// Not cryptographic: callers supply the entropy as a seed.
#[inline]
pub fn mix32(x: u32) -> u32 {
    let mut z = x.wrapping_add(0x9E37_79B9);
    z = (z ^ (z >> 16)).wrapping_mul(0x85EB_CA6B);
    z = (z ^ (z >> 13)).wrapping_mul(0xC2B2_AE35);
    z ^ (z >> 16)
}

use core::fmt;

/// Fixed-size line buffer for log messages (never allocates; truncates silently).
pub struct LineBuf {
    buf: [u8; LINE_MAX],
    len: usize,
}

impl LineBuf {
    pub const fn new() -> LineBuf {
        LineBuf { buf: [0; LINE_MAX], len: 0 }
    }
    pub fn as_str(&self) -> &str {
        let end = self.len.min(self.buf.len());
        match core::str::from_utf8(&self.buf[..end]) {
            Ok(s) => s,
            // Truncation can only split a multi-byte char; keep the valid prefix.
            Err(e) => core::str::from_utf8(&self.buf[..e.valid_up_to()]).unwrap_or(""),
        }
    }
}

impl fmt::Write for LineBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = self.buf.len().saturating_sub(self.len);
        let n = s.len().min(room);
        if let (Some(dst), Some(src)) = (self.buf.get_mut(self.len..self.len + n), s.as_bytes().get(..n)) {
            dst.copy_from_slice(src);
            self.len += n;
        }
        Ok(())
    }
}

/// Dotted-quad IPv4 formatter (`10.0.2.2`).
pub struct Ip(pub [u8; 4]);

impl fmt::Display for Ip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let a = self.0;
        write!(f, "{}.{}.{}.{}", a[0], a[1], a[2], a[3])
    }
}

/// MAC formatter: 12 lowercase hex digits, no separators (`52550a000202`).
pub struct Mac(pub [u8; 6]);

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

/// Big-endian u16 at `off`, or None if out of bounds.
#[inline]
pub(crate) fn be16(p: &[u8], off: usize) -> Option<u16> {
    let b = p.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

#[inline]
pub(crate) fn arr4(p: &[u8], off: usize) -> Option<[u8; 4]> {
    let b = p.get(off..off.checked_add(4)?)?;
    Some([b[0], b[1], b[2], b[3]])
}

#[inline]
pub(crate) fn arr6(p: &[u8], off: usize) -> Option<[u8; 6]> {
    let b = p.get(off..off.checked_add(6)?)?;
    Some([b[0], b[1], b[2], b[3], b[4], b[5]])
}
