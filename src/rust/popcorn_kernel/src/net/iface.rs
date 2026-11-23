//! One IPv4 interface: addresses, ARP cache and pending queue, ICMP echo table,
//! counters. The device is passed to every call; `Iface` owns protocol state only
//! (so it never holds driver buffers across calls: frames waiting for ARP are
//! copied into `Iface`-owned storage).

use core::fmt::Write as _;

use super::arp::{self, ArpCache, ArpPacket, Insert};
use super::eth::{self, BROADCAST, ETHERTYPE_ARP, ETHERTYPE_IPV4};
use super::icmp;
use super::ipv4::{self, Ipv4Error, Ipv4Header, MAX_PACKET, PROTO_ICMP};
use super::{Ip, LineBuf, Mac};
use crate::drivers::net::{NetDevice, RxBatch, RX_BATCH};

/// Frames that may wait for ARP resolution at once.
pub const PENDING_MAX: usize = 8;
/// Interval between ARP requests for an unresolved next hop.
pub const ARP_RETRY_MS: u64 = 1000;
/// ARP requests sent per resolution attempt (at 0, 1 s, 2 s); after one more
/// interval without a reply the waiting frames are dropped.
pub const ARP_MAX_REQUESTS: u8 = 3;
/// Outstanding echo requests remembered for RTT matching.
pub const ECHO_TABLE: usize = 16;
/// RX frames processed per `poll` call at most.
pub const POLL_RX_CAP: usize = 256;
/// TX completions reaped per `poll` call at most.
pub const REAP_BUDGET: usize = 64;
/// Largest echo payload that fits one unfragmented packet.
pub const MAX_PING_PAYLOAD: usize = MAX_PACKET - ipv4::HDR_LEN - icmp::HDR_LEN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpConfig {
    pub ip: [u8; 4],
    pub prefix: u8,
    pub gw: [u8; 4],
    pub dns: [u8; 4],
}

impl IpConfig {
    /// QEMU user networking (slirp) defaults.
    pub const SLIRP: IpConfig = IpConfig { ip: [10, 0, 2, 15], prefix: 24, gw: [10, 0, 2, 2], dns: [10, 0, 2, 3] };
    /// Unconfigured (all zero): the stack answers nothing.
    pub const NONE: IpConfig = IpConfig { ip: [0; 4], prefix: 0, gw: [0; 4], dns: [0; 4] };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    /// No IPv4 address configured.
    NotConfigured,
    /// Destination is off-link and no gateway is set.
    NoRoute,
    /// Packet would exceed the MTU (no fragmentation).
    TooBig,
    /// The device had no free TX buffer.
    TxFull,
    /// The device refused the frame (or lent a buffer smaller than `MAX_FRAME`).
    TxFailed,
    /// All `PENDING_MAX` ARP-wait slots are in use.
    PendingFull,
}

/// Stack counters. Every received frame ends in exactly one of `rx_delivered` or
/// one `drop_*` RX reason, so `rx_frames == rx_delivered + rx_dropped()`.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct StackStats {
    // RX by protocol (frames that reached that layer).
    pub rx_frames: u64,
    pub rx_arp: u64,
    pub rx_ipv4: u64,
    pub rx_icmp: u64,
    pub rx_arp_requests: u64,
    pub rx_arp_replies: u64,
    pub rx_echo_requests: u64,
    pub rx_echo_replies: u64,
    /// Frames fully handled by a protocol (the non-drop outcome).
    pub rx_delivered: u64,
    // TX by protocol (frames handed to the device).
    pub tx_frames: u64,
    pub tx_arp: u64,
    pub tx_arp_requests: u64,
    pub tx_arp_replies: u64,
    pub tx_ipv4: u64,
    pub tx_icmp: u64,
    /// Echo requests accepted by `ping_send` (sent or queued behind ARP).
    pub tx_echo_requests: u64,
    /// Echo replies generated (sent or queued behind ARP).
    pub tx_echo_replies: u64,
    // RX drops by reason.
    /// Truncated Ethernet/ARP/IPv4/ICMP header.
    pub drop_short: u64,
    /// IPv4 header or ICMP checksum wrong.
    pub drop_bad_csum: u64,
    /// Destination MAC/IP (or ARP target) is not ours.
    pub drop_not_for_us: u64,
    /// IPv4 fragment (MF or offset != 0).
    pub drop_fragment: u64,
    /// EtherType other than ARP/IPv4.
    pub drop_unknown_ethertype: u64,
    /// IPv4 protocol other than ICMP.
    pub drop_unknown_proto: u64,
    /// IPv4 version != 4 or IHL < 5; ARP not Ethernet/IPv4 or unknown opcode.
    pub drop_bad_header: u64,
    /// IPv4 total length < header or > frame.
    pub drop_bad_len: u64,
    /// Sender is broadcast/multicast/zero (never answered or learned).
    pub drop_bad_src: u64,
    /// ICMP type other than echo request/reply.
    pub drop_icmp_other: u64,
    /// Echo reply matching no outstanding request.
    pub drop_unmatched_reply: u64,
    // TX-side drops.
    /// Frames dropped after ARP gave up.
    pub drop_arp_timeout: u64,
    /// Frames refused because the ARP-wait queue was full.
    pub drop_pending_full: u64,
    /// Frames dropped because the device had no TX buffer.
    pub drop_tx_full: u64,
    /// `tx_submit` failures.
    pub tx_errors: u64,
    // Misc.
    pub arp_retries: u64,
    pub arp_timeouts: u64,
    /// `poll` calls that stopped at `POLL_RX_CAP` with frames possibly left.
    pub poll_cap_hit: u64,
}

impl StackStats {
    /// Sum of every RX drop reason.
    pub fn rx_dropped(&self) -> u64 {
        self.drop_short
            + self.drop_bad_csum
            + self.drop_not_for_us
            + self.drop_fragment
            + self.drop_unknown_ethertype
            + self.drop_unknown_proto
            + self.drop_bad_header
            + self.drop_bad_len
            + self.drop_bad_src
            + self.drop_icmp_other
            + self.drop_unmatched_reply
    }
}

#[derive(Clone, Copy)]
struct Pending {
    used: bool,
    hop: [u8; 4],
    len: u16,
    /// The complete IPv4 packet (header + payload).
    data: [u8; MAX_PACKET],
}

const PENDING_EMPTY: Pending = Pending { used: false, hop: [0; 4], len: 0, data: [0; MAX_PACKET] };

#[derive(Clone, Copy)]
struct ArpWait {
    used: bool,
    ip: [u8; 4],
    last_ms: u64,
    tries: u8,
}

const WAIT_EMPTY: ArpWait = ArpWait { used: false, ip: [0; 4], last_ms: 0, tries: 0 };

#[derive(Clone, Copy)]
struct Echo {
    used: bool,
    dst: [u8; 4],
    id: u16,
    seq: u16,
    sent_ms: u64,
    rtt_ms: Option<u64>,
}

const ECHO_EMPTY: Echo = Echo { used: false, dst: [0; 4], id: 0, seq: 0, sent_ms: 0, rtt_ms: None };

pub struct Iface {
    mac: [u8; 6],
    cfg: IpConfig,
    log: fn(&str),
    arp: ArpCache,
    pending: [Pending; PENDING_MAX],
    waits: [ArpWait; PENDING_MAX],
    echoes: [Echo; ECHO_TABLE],
    ip_id: u16,
    /// Latest time seen by `poll`/`ping_send` (for `&self` queries).
    now_ms: u64,
    stats: StackStats,
}

/// Copy a complete IPv4 packet into a fresh TX frame behind an Ethernet header.
fn emit_packet(
    dev: &mut dyn NetDevice,
    src_mac: [u8; 6],
    dst_mac: [u8; 6],
    pkt: &[u8],
    stats: &mut StackStats,
) -> Result<(), NetError> {
    let mut buf = match dev.tx_alloc() {
        Some(b) => b,
        None => return Err(NetError::TxFull),
    };
    let flen = eth::HDR_LEN + pkt.len();
    {
        let Some(f) = buf.frame_mut().get_mut(..flen) else {
            // Driver lent a buffer below MAX_FRAME (contract violation); the trait has
            // no way to hand it back unused.
            stats.tx_errors += 1;
            return Err(NetError::TxFailed);
        };
        let _ = eth::write(f, dst_mac, src_mac, ETHERTYPE_IPV4);
        if let Some(d) = f.get_mut(eth::HDR_LEN..) {
            d.copy_from_slice(pkt);
        }
    }
    if dev.tx_submit(buf, flen).is_err() {
        stats.tx_errors += 1;
        return Err(NetError::TxFailed);
    }
    stats.tx_frames += 1;
    stats.tx_ipv4 += 1;
    if pkt.get(9) == Some(&PROTO_ICMP) {
        stats.tx_icmp += 1;
    }
    Ok(())
}

impl Iface {
    pub fn new(mac: [u8; 6], cfg: IpConfig, log: fn(&str)) -> Iface {
        Iface {
            mac,
            cfg,
            log,
            arp: ArpCache::new(),
            pending: [PENDING_EMPTY; PENDING_MAX],
            waits: [WAIT_EMPTY; PENDING_MAX],
            echoes: [ECHO_EMPTY; ECHO_TABLE],
            ip_id: 1,
            now_ms: 0,
            stats: StackStats::default(),
        }
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }
    pub fn config(&self) -> IpConfig {
        self.cfg
    }
    /// Replace the address configuration. The ARP cache is kept; waiting frames
    /// keep their original next hop.
    pub fn set_config(&mut self, cfg: IpConfig) {
        self.cfg = cfg;
    }
    pub fn stats(&self) -> StackStats {
        self.stats
    }
    pub fn arp_lookup(&self, ip: [u8; 4]) -> Option<[u8; 6]> {
        self.arp.lookup(ip, self.now_ms)
    }
    /// Live ARP entries (for `net -arp`).
    pub fn arp_for_each(&self, f: impl FnMut(&arp::ArpEntry)) {
        self.arp.for_each(self.now_ms, f)
    }
    pub fn arp_flush(&mut self) {
        self.arp.clear();
    }
    /// Frames currently waiting for ARP.
    pub fn pending_frames(&self) -> usize {
        self.pending.iter().filter(|p| p.used).count()
    }
    /// RTT in ms once the echo reply for (`id`, `seq`) has arrived.
    pub fn ping_reply(&self, id: u16, seq: u16) -> Option<u64> {
        self.echoes.iter().find(|e| e.used && e.id == id && e.seq == seq).and_then(|e| e.rtt_ms)
    }

    fn logf(&self, args: core::fmt::Arguments<'_>) {
        let mut line = LineBuf::new();
        let _ = line.write_fmt(args);
        (self.log)(line.as_str());
    }

    /// Next hop for `dst`: itself when on-link (or limited broadcast), else the
    /// gateway. None when off-link without a gateway.
    fn next_hop(&self, dst: [u8; 4]) -> Option<[u8; 4]> {
        if ipv4::is_broadcast(dst) || ipv4::same_subnet(dst, self.cfg.ip, self.cfg.prefix) {
            Some(dst)
        } else if self.cfg.gw != [0; 4] {
            Some(self.cfg.gw)
        } else {
            None
        }
    }

    // ---- poll ----------------------------------------------------------------

    /// Reap TX, process received frames (up to `POLL_RX_CAP`), run ARP timers, then
    /// one `tx_flush`.
    pub fn poll(&mut self, dev: &mut dyn NetDevice, now_ms: u64) {
        self.now_ms = now_ms;
        dev.reap_tx(REAP_BUDGET);
        let mut batch = RxBatch::new();
        let mut total = 0usize;
        loop {
            let budget = RX_BATCH.min(POLL_RX_CAP - total);
            let n = dev.rx_batch(budget, &mut batch);
            if n == 0 || batch.is_empty() {
                dev.rx_recycle(&mut batch);
                break;
            }
            for i in 0..batch.len() {
                let frame = batch.frame(i);
                self.process(dev, frame, now_ms);
            }
            dev.rx_recycle(&mut batch);
            total += n;
            if total >= POLL_RX_CAP {
                self.stats.poll_cap_hit += 1;
                break;
            }
        }
        self.arp_timers(dev, now_ms);
        dev.tx_flush();
    }

    fn process(&mut self, dev: &mut dyn NetDevice, frame: &[u8], now_ms: u64) {
        self.stats.rx_frames += 1;
        let Some((h, payload)) = eth::parse(frame) else {
            self.stats.drop_short += 1;
            return;
        };
        if !eth::accepts(h.dst, self.mac) {
            self.stats.drop_not_for_us += 1;
            return;
        }
        match h.ethertype {
            ETHERTYPE_ARP => self.handle_arp(dev, payload, now_ms),
            ETHERTYPE_IPV4 => self.handle_ipv4(dev, payload, now_ms),
            _ => self.stats.drop_unknown_ethertype += 1,
        }
    }

    // ---- ARP -----------------------------------------------------------------

    fn handle_arp(&mut self, dev: &mut dyn NetDevice, p: &[u8], now_ms: u64) {
        self.stats.rx_arp += 1;
        let a = match arp::parse(p) {
            Ok(a) => a,
            Err(arp::ArpError::Short) => {
                self.stats.drop_short += 1;
                return;
            }
            Err(arp::ArpError::Unsupported) => {
                self.stats.drop_bad_header += 1;
                return;
            }
        };
        let valid_sender = !ipv4::is_bad_source(a.spa) && a.sha[0] & 1 == 0;
        let for_us = self.cfg.ip != [0; 4] && a.tpa == self.cfg.ip;
        if !for_us {
            // RFC 826 merge: refresh a sender we already know, but do not add it.
            if valid_sender && self.arp.contains(a.spa) {
                self.arp.insert(a.spa, a.sha, now_ms);
                self.flush_pending_for(dev, a.spa, a.sha);
            }
            self.stats.drop_not_for_us += 1;
            return;
        }
        if !valid_sender {
            self.stats.drop_bad_src += 1;
            return;
        }
        let kind = self.arp.insert(a.spa, a.sha, now_ms);
        self.stats.rx_delivered += 1;
        if a.op == arp::OP_REPLY {
            self.stats.rx_arp_replies += 1;
            if kind != Insert::Refreshed {
                self.logf(format_args!("net: arp {} {}", Ip(a.spa), Mac(a.sha)));
            }
        } else {
            self.stats.rx_arp_requests += 1;
            let reply = ArpPacket { op: arp::OP_REPLY, sha: self.mac, spa: self.cfg.ip, tha: a.sha, tpa: a.spa };
            let _ = self.send_arp(dev, a.sha, &reply);
        }
        self.flush_pending_for(dev, a.spa, a.sha);
    }

    fn send_arp(&mut self, dev: &mut dyn NetDevice, eth_dst: [u8; 6], a: &ArpPacket) -> Result<(), NetError> {
        let Some(mut buf) = dev.tx_alloc() else {
            self.stats.drop_tx_full += 1;
            return Err(NetError::TxFull);
        };
        let flen = eth::HDR_LEN + arp::LEN;
        {
            let Some(f) = buf.frame_mut().get_mut(..flen) else {
                self.stats.tx_errors += 1;
                return Err(NetError::TxFailed);
            };
            let _ = eth::write(f, eth_dst, self.mac, ETHERTYPE_ARP);
            if let Some(d) = f.get_mut(eth::HDR_LEN..) {
                let _ = arp::write(d, a);
            }
        }
        if dev.tx_submit(buf, flen).is_err() {
            self.stats.tx_errors += 1;
            return Err(NetError::TxFailed);
        }
        self.stats.tx_frames += 1;
        self.stats.tx_arp += 1;
        if a.op == arp::OP_REQUEST {
            self.stats.tx_arp_requests += 1;
        } else {
            self.stats.tx_arp_replies += 1;
        }
        Ok(())
    }

    fn send_arp_request(&mut self, dev: &mut dyn NetDevice, ip: [u8; 4]) {
        let req = ArpPacket { op: arp::OP_REQUEST, sha: self.mac, spa: self.cfg.ip, tha: [0; 6], tpa: ip };
        let _ = self.send_arp(dev, BROADCAST, &req);
    }

    /// Transmit every waiting frame for `ip`. Frames that hit a full TX ring stay
    /// queued and are retried by `arp_timers`.
    fn flush_pending_for(&mut self, dev: &mut dyn NetDevice, ip: [u8; 4], mac: [u8; 6]) {
        for p in self.pending.iter_mut() {
            if !p.used || p.hop != ip {
                continue;
            }
            let pkt = p.data.get(..p.len as usize).unwrap_or(&[]);
            match emit_packet(dev, self.mac, mac, pkt, &mut self.stats) {
                Err(NetError::TxFull) => {}
                _ => p.used = false,
            }
        }
    }

    /// Free wait entries whose address resolved or has no frames left.
    fn gc_waits(&mut self, now_ms: u64) {
        for w in self.waits.iter_mut() {
            if w.used
                && (self.arp.lookup(w.ip, now_ms).is_some()
                    || !self.pending.iter().any(|p| p.used && p.hop == w.ip))
            {
                w.used = false;
            }
        }
    }

    /// Start resolving `ip` unless already in progress (sends the first request).
    fn start_resolve(&mut self, dev: &mut dyn NetDevice, ip: [u8; 4], now_ms: u64) {
        if self.waits.iter().any(|w| w.used && w.ip == ip) {
            return;
        }
        self.gc_waits(now_ms);
        // At most one wait per distinct pending hop, so a slot always exists.
        if let Some(w) = self.waits.iter_mut().find(|w| !w.used) {
            *w = ArpWait { used: true, ip, last_ms: now_ms, tries: 1 };
            self.send_arp_request(dev, ip);
        }
    }

    fn arp_timers(&mut self, dev: &mut dyn NetDevice, now_ms: u64) {
        // Frames whose hop is resolved (e.g. an earlier flush hit a full TX ring).
        for i in 0..PENDING_MAX {
            let Some(p) = self.pending.get(i) else { break };
            if !p.used {
                continue;
            }
            let hop = p.hop;
            if let Some(mac) = self.arp.lookup(hop, now_ms) {
                self.flush_pending_for(dev, hop, mac);
            }
        }
        self.gc_waits(now_ms);
        // Retries and timeouts.
        for i in 0..PENDING_MAX {
            let Some(w) = self.waits.get(i).copied() else { break };
            if !w.used || now_ms.saturating_sub(w.last_ms) < ARP_RETRY_MS {
                continue;
            }
            if w.tries >= ARP_MAX_REQUESTS {
                let mut dropped = 0;
                for p in self.pending.iter_mut().filter(|p| p.used && p.hop == w.ip) {
                    p.used = false;
                    dropped += 1;
                }
                self.stats.drop_arp_timeout += dropped;
                self.stats.arp_timeouts += 1;
                if let Some(slot) = self.waits.get_mut(i) {
                    slot.used = false;
                }
                self.logf(format_args!("net: arp timeout {}", Ip(w.ip)));
            } else {
                if let Some(slot) = self.waits.get_mut(i) {
                    slot.tries += 1;
                    slot.last_ms = now_ms;
                }
                self.stats.arp_retries += 1;
                self.send_arp_request(dev, w.ip);
            }
        }
        // Waiting frames left without a resolver (cache entry expired meanwhile).
        for i in 0..PENDING_MAX {
            let Some(p) = self.pending.get(i) else { break };
            if p.used {
                let hop = p.hop;
                self.start_resolve(dev, hop, now_ms);
            }
        }
    }

    // ---- IPv4 / ICMP ---------------------------------------------------------

    /// Send an IPv4 packet with an `l4_len`-byte payload written by `fill`.
    /// Queues behind ARP when the next hop is unresolved (returns Ok).
    fn send_ipv4(
        &mut self,
        dev: &mut dyn NetDevice,
        dst: [u8; 4],
        proto: u8,
        l4_len: usize,
        now_ms: u64,
        fill: impl FnOnce(&mut [u8]),
    ) -> Result<(), NetError> {
        if self.cfg.ip == [0; 4] {
            return Err(NetError::NotConfigured);
        }
        let total = ipv4::HDR_LEN + l4_len;
        if l4_len > MAX_PACKET - ipv4::HDR_LEN {
            return Err(NetError::TooBig);
        }
        let hop = self.next_hop(dst).ok_or(NetError::NoRoute)?;
        let mac = if ipv4::is_broadcast(dst) { Some(BROADCAST) } else { self.arp.lookup(hop, now_ms) };
        let id = self.ip_id;
        self.ip_id = self.ip_id.wrapping_add(1);

        if let Some(mac) = mac {
            let Some(mut buf) = dev.tx_alloc() else {
                self.stats.drop_tx_full += 1;
                return Err(NetError::TxFull);
            };
            let flen = eth::HDR_LEN + total;
            {
                let Some(f) = buf.frame_mut().get_mut(..flen) else {
                    self.stats.tx_errors += 1;
                    return Err(NetError::TxFailed);
                };
                let (eh, rest) = f.split_at_mut(eth::HDR_LEN);
                let _ = eth::write(eh, mac, self.mac, ETHERTYPE_IPV4);
                let (ih, l4) = rest.split_at_mut(ipv4::HDR_LEN);
                let _ = ipv4::write(ih, self.cfg.ip, dst, proto, id, l4_len);
                fill(l4);
            }
            if dev.tx_submit(buf, flen).is_err() {
                self.stats.tx_errors += 1;
                return Err(NetError::TxFailed);
            }
            self.stats.tx_frames += 1;
            self.stats.tx_ipv4 += 1;
            if proto == PROTO_ICMP {
                self.stats.tx_icmp += 1;
            }
            return Ok(());
        }

        let Some(p) = self.pending.iter_mut().find(|p| !p.used) else {
            self.stats.drop_pending_full += 1;
            return Err(NetError::PendingFull);
        };
        let Some(pkt) = p.data.get_mut(..total) else {
            return Err(NetError::TooBig);
        };
        let (ih, l4) = pkt.split_at_mut(ipv4::HDR_LEN);
        let _ = ipv4::write(ih, self.cfg.ip, dst, proto, id, l4_len);
        fill(l4);
        p.used = true;
        p.hop = hop;
        p.len = total as u16;
        self.start_resolve(dev, hop, now_ms);
        Ok(())
    }

    fn handle_ipv4(&mut self, dev: &mut dyn NetDevice, p: &[u8], now_ms: u64) {
        self.stats.rx_ipv4 += 1;
        let (h, payload) = match ipv4::parse(p) {
            Ok(v) => v,
            Err(e) => {
                let s = &mut self.stats;
                match e {
                    Ipv4Error::Short => s.drop_short += 1,
                    Ipv4Error::BadHeader => s.drop_bad_header += 1,
                    Ipv4Error::BadCsum => s.drop_bad_csum += 1,
                    Ipv4Error::BadLen => s.drop_bad_len += 1,
                    Ipv4Error::Fragment => s.drop_fragment += 1,
                }
                return;
            }
        };
        if self.cfg.ip == [0; 4] || h.dst != self.cfg.ip {
            self.stats.drop_not_for_us += 1;
            return;
        }
        match h.proto {
            PROTO_ICMP => self.handle_icmp(dev, &h, payload, now_ms),
            _ => self.stats.drop_unknown_proto += 1,
        }
    }

    fn handle_icmp(&mut self, dev: &mut dyn NetDevice, ip: &Ipv4Header, msg: &[u8], now_ms: u64) {
        self.stats.rx_icmp += 1;
        let (h, payload) = match icmp::parse(msg) {
            Ok(v) => v,
            Err(icmp::IcmpError::Short) => {
                self.stats.drop_short += 1;
                return;
            }
            Err(icmp::IcmpError::BadCsum) => {
                self.stats.drop_bad_csum += 1;
                return;
            }
        };
        match (h.ty, h.code) {
            (icmp::TYPE_ECHO_REQUEST, 0) => {
                if ipv4::is_bad_source(ip.src) {
                    self.stats.drop_bad_src += 1;
                    return;
                }
                self.stats.rx_echo_requests += 1;
                self.stats.rx_delivered += 1;
                let (id, seq) = (h.id, h.seq);
                let r = self.send_ipv4(dev, ip.src, PROTO_ICMP, msg.len(), now_ms, |b| {
                    let _ = icmp::write_echo(b, icmp::TYPE_ECHO_REPLY, id, seq, payload);
                });
                if r.is_ok() {
                    self.stats.tx_echo_replies += 1;
                }
            }
            (icmp::TYPE_ECHO_REPLY, 0) => {
                let e = self.echoes.iter_mut().find(|e| {
                    e.used && e.rtt_ms.is_none() && e.id == h.id && e.seq == h.seq && e.dst == ip.src
                });
                match e {
                    Some(e) => {
                        e.rtt_ms = Some(now_ms.saturating_sub(e.sent_ms));
                        self.stats.rx_echo_replies += 1;
                        self.stats.rx_delivered += 1;
                    }
                    None => self.stats.drop_unmatched_reply += 1,
                }
            }
            _ => self.stats.drop_icmp_other += 1,
        }
    }

    /// Send an ICMP echo request with `payload_len` pattern bytes. Queues behind
    /// ARP if the next hop is unresolved. Flushes the device.
    pub fn ping_send(
        &mut self,
        dev: &mut dyn NetDevice,
        dst: [u8; 4],
        id: u16,
        seq: u16,
        payload_len: usize,
        now_ms: u64,
    ) -> Result<(), NetError> {
        self.now_ms = now_ms;
        if payload_len > MAX_PING_PAYLOAD {
            return Err(NetError::TooBig);
        }
        let r = self.send_ipv4(dev, dst, PROTO_ICMP, icmp::HDR_LEN + payload_len, now_ms, |b| {
            let _ = icmp::write_echo_pattern(b, icmp::TYPE_ECHO_REQUEST, id, seq, payload_len);
        });
        if r.is_ok() {
            self.stats.tx_echo_requests += 1;
            let entry = Echo { used: true, dst, id, seq, sent_ms: now_ms, rtt_ms: None };
            let slot = match self.echoes.iter().position(|e| e.used && e.id == id && e.seq == seq) {
                Some(i) => i,
                None => match self.echoes.iter().position(|e| !e.used) {
                    Some(i) => i,
                    None => {
                        // Evict the oldest.
                        let mut oldest = 0;
                        for (i, e) in self.echoes.iter().enumerate() {
                            if e.sent_ms < self.echoes.get(oldest).map_or(u64::MAX, |o| o.sent_ms) {
                                oldest = i;
                            }
                        }
                        oldest
                    }
                },
            };
            if let Some(e) = self.echoes.get_mut(slot) {
                *e = entry;
            }
        }
        dev.tx_flush();
        r
    }
}
