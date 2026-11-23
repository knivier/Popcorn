//! ARP for Ethernet / IPv4 (RFC 826) and a fixed-size neighbour cache.

use super::{arr4, arr6, be16};

/// ARP payload length for Ethernet/IPv4.
pub const LEN: usize = 28;
pub const OP_REQUEST: u16 = 1;
pub const OP_REPLY: u16 = 2;
const HTYPE_ETHERNET: u16 = 1;
const PTYPE_IPV4: u16 = 0x0800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArpPacket {
    pub op: u16,
    pub sha: [u8; 6],
    pub spa: [u8; 4],
    pub tha: [u8; 6],
    pub tpa: [u8; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpError {
    /// Fewer than 28 bytes.
    Short,
    /// Not Ethernet/IPv4 (htype/ptype/hlen/plen) or an unknown opcode.
    Unsupported,
}

/// Parse an ARP payload (after the Ethernet header; trailing padding ignored).
pub fn parse(p: &[u8]) -> Result<ArpPacket, ArpError> {
    if p.len() < LEN {
        return Err(ArpError::Short);
    }
    let htype = be16(p, 0).ok_or(ArpError::Short)?;
    let ptype = be16(p, 2).ok_or(ArpError::Short)?;
    let hlen = *p.get(4).ok_or(ArpError::Short)?;
    let plen = *p.get(5).ok_or(ArpError::Short)?;
    let op = be16(p, 6).ok_or(ArpError::Short)?;
    if htype != HTYPE_ETHERNET || ptype != PTYPE_IPV4 || hlen != 6 || plen != 4 {
        return Err(ArpError::Unsupported);
    }
    if op != OP_REQUEST && op != OP_REPLY {
        return Err(ArpError::Unsupported);
    }
    Ok(ArpPacket {
        op,
        sha: arr6(p, 8).ok_or(ArpError::Short)?,
        spa: arr4(p, 14).ok_or(ArpError::Short)?,
        tha: arr6(p, 18).ok_or(ArpError::Short)?,
        tpa: arr4(p, 24).ok_or(ArpError::Short)?,
    })
}

/// Write an ARP payload into `buf`. Returns `LEN`, or None if `buf` is too small.
pub fn write(buf: &mut [u8], a: &ArpPacket) -> Option<usize> {
    let b = buf.get_mut(..LEN)?;
    b[0..2].copy_from_slice(&HTYPE_ETHERNET.to_be_bytes());
    b[2..4].copy_from_slice(&PTYPE_IPV4.to_be_bytes());
    b[4] = 6;
    b[5] = 4;
    b[6..8].copy_from_slice(&a.op.to_be_bytes());
    b[8..14].copy_from_slice(&a.sha);
    b[14..18].copy_from_slice(&a.spa);
    b[18..24].copy_from_slice(&a.tha);
    b[24..28].copy_from_slice(&a.tpa);
    Some(LEN)
}

/// Neighbour cache entries.
pub const CACHE_SIZE: usize = 16;
/// Entry lifetime.
pub const CACHE_TTL_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug)]
pub struct ArpEntry {
    pub ip: [u8; 4],
    pub mac: [u8; 6],
    pub expires_ms: u64,
    pub valid: bool,
}

const EMPTY: ArpEntry = ArpEntry { ip: [0; 4], mac: [0; 6], expires_ms: 0, valid: false };

pub struct ArpCache {
    entries: [ArpEntry; CACHE_SIZE],
}

/// What `ArpCache::insert` changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Insert {
    /// Address was unknown (or expired).
    New,
    /// Known, but the MAC changed.
    Changed,
    /// Known with the same MAC; only the lifetime was refreshed.
    Refreshed,
}

impl ArpCache {
    pub const fn new() -> ArpCache {
        ArpCache { entries: [EMPTY; CACHE_SIZE] }
    }

    fn live(e: &ArpEntry, now_ms: u64) -> bool {
        e.valid && now_ms < e.expires_ms
    }

    pub fn lookup(&self, ip: [u8; 4], now_ms: u64) -> Option<[u8; 6]> {
        self.entries.iter().find(|e| Self::live(e, now_ms) && e.ip == ip).map(|e| e.mac)
    }

    /// True if `ip` has an entry (live or not); used for RFC 826 "merge" updates.
    pub fn contains(&self, ip: [u8; 4]) -> bool {
        self.entries.iter().any(|e| e.valid && e.ip == ip)
    }

    pub fn insert(&mut self, ip: [u8; 4], mac: [u8; 6], now_ms: u64) -> Insert {
        let expires_ms = now_ms.saturating_add(CACHE_TTL_MS);
        if let Some(e) = self.entries.iter_mut().find(|e| e.valid && e.ip == ip) {
            let kind = if !Self::live(e, now_ms) {
                Insert::New
            } else if e.mac != mac {
                Insert::Changed
            } else {
                Insert::Refreshed
            };
            e.mac = mac;
            e.expires_ms = expires_ms;
            return kind;
        }
        // Free or expired slot first, else evict the entry closest to expiry.
        let mut victim = 0;
        let mut best = u64::MAX;
        for (i, e) in self.entries.iter().enumerate() {
            if !Self::live(e, now_ms) {
                victim = i;
                break;
            }
            if e.expires_ms < best {
                best = e.expires_ms;
                victim = i;
            }
        }
        if let Some(e) = self.entries.get_mut(victim) {
            *e = ArpEntry { ip, mac, expires_ms, valid: true };
        }
        Insert::New
    }

    pub fn clear(&mut self) {
        self.entries = [EMPTY; CACHE_SIZE];
    }

    /// Live entries, for `net -arp`.
    pub fn for_each(&self, now_ms: u64, mut f: impl FnMut(&ArpEntry)) {
        for e in self.entries.iter().filter(|e| Self::live(e, now_ms)) {
            f(e);
        }
    }
}
