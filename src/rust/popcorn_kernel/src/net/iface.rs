//! One IPv4 interface: addresses, ARP cache and pending queue, ICMP echo table,
//! UDP sockets, the DHCP client, the DNS stub resolver and counters. The device is
//! passed to every call; `Iface` owns protocol state only (so it never holds driver
//! buffers across calls: frames waiting for ARP and received datagrams are copied
//! into `Iface`-owned storage).

use core::fmt::Write as _;

use super::arp::{self, ArpCache, ArpPacket, Insert};
use super::dhcp::{self, DhcpReply, DhcpRequest};
use super::dns::{self, DnsError, Reject, WireName};
use super::eth::{self, BROADCAST, ETHERTYPE_ARP, ETHERTYPE_IPV4};
use super::icmp;
use super::ipv4::{self, Ipv4Error, Ipv4Header, MAX_PACKET, PROTO_ICMP, PROTO_UDP};
use super::udp::{self, UdpError};
use super::{be16, mix32, Ip, LineBuf, Mac};
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

/// UDP sockets (bound ports) at once.
pub const UDP_SOCKETS: usize = 8;
/// Datagrams queued per socket at most.
pub const UDP_RING: usize = 4;
/// Datagram buffers shared by all sockets.
pub const UDP_POOL: usize = 8;
/// Largest UDP payload that fits one unfragmented packet.
pub const UDP_MAX_PAYLOAD: usize = MAX_PACKET - ipv4::HDR_LEN - udp::HDR_LEN;
/// Ephemeral ports are drawn from 49152..=65535.
pub const EPHEMERAL_BASE: u16 = 49152;

/// First DHCP retransmit interval; doubles per retransmit (1 s, 2 s, 4 s, ...).
pub const DHCP_RETRY_MS: u64 = 1000;
/// Retransmit interval cap.
pub const DHCP_RETRY_MAX_MS: u64 = 32_000;
/// No lease within this time after `dhcp_start` → apply the fallback (review Q12).
pub const DHCP_FALLBACK_MS: u64 = 3000;
/// Without a fallback, give up (state `Failed`) after this long.
pub const DHCP_GIVE_UP_MS: u64 = 60_000;
/// REQUESTs sent for one OFFER before starting over with DISCOVER.
pub const DHCP_REQUEST_TRIES: u8 = 4;
/// Minimum interval between renew/rebind REQUESTs (RFC 2131 §4.4.5: 60 s).
pub const DHCP_RENEW_MIN_MS: u64 = 60_000;
/// Leases shorter than this are treated as this long (avoids renew storms).
pub const DHCP_MIN_LEASE_S: u32 = 10;

/// DNS queries in flight (or finished, awaiting `dns_result`) at once.
pub const DNS_QUERIES: usize = 4;
/// Cached name → address entries.
pub const DNS_CACHE: usize = 8;
/// Per-attempt timeout.
pub const DNS_TIMEOUT_MS: u64 = 2000;
/// Attempts per query (same id and port each time).
pub const DNS_ATTEMPTS: u8 = 2;
/// Cached TTLs are capped at this many seconds.
pub const DNS_TTL_MAX_S: u32 = 3600;

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
    /// The UDP port is already bound (or used by DHCP/DNS).
    PortInUse,
    /// All `UDP_SOCKETS` sockets are bound (or no free ephemeral port).
    NoSockets,
    /// The `SockId` is not (or no longer) bound.
    BadSocket,
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
    pub rx_udp: u64,
    /// UDP datagrams to the DHCP client port while DHCP runs.
    pub rx_dhcp: u64,
    /// UDP datagrams from port 53 to a port with a query in flight.
    pub rx_dns: u64,
    /// Frames fully handled by a protocol (the non-drop outcome).
    pub rx_delivered: u64,
    // TX by protocol (frames handed to the device).
    pub tx_frames: u64,
    pub tx_arp: u64,
    pub tx_arp_requests: u64,
    pub tx_arp_replies: u64,
    pub tx_ipv4: u64,
    pub tx_icmp: u64,
    pub tx_udp: u64,
    /// Echo requests accepted by `ping_send` (sent or queued behind ARP).
    pub tx_echo_requests: u64,
    /// Echo replies generated (sent or queued behind ARP).
    pub tx_echo_replies: u64,
    /// DHCP messages sent (or queued behind ARP), retransmits included.
    pub tx_dhcp: u64,
    /// DNS queries sent (or queued behind ARP), retries included.
    pub tx_dns: u64,
    /// UDP echo-service replies sent (or queued).
    pub tx_udp_echo: u64,
    /// Gratuitous ARPs sent after a DHCP lease was applied.
    pub tx_garp: u64,
    // RX drops by reason.
    /// Truncated Ethernet/ARP/IPv4/ICMP/UDP header.
    pub drop_short: u64,
    /// IPv4 header or ICMP checksum wrong.
    pub drop_bad_csum: u64,
    /// Destination MAC/IP (or ARP target) is not ours.
    pub drop_not_for_us: u64,
    /// IPv4 fragment (MF or offset != 0).
    pub drop_fragment: u64,
    /// EtherType other than ARP/IPv4.
    pub drop_unknown_ethertype: u64,
    /// IPv4 protocol other than ICMP/UDP.
    pub drop_unknown_proto: u64,
    /// IPv4 version != 4 or IHL < 5; ARP not Ethernet/IPv4 or unknown opcode.
    pub drop_bad_header: u64,
    /// IPv4 total length < header or > frame; UDP length bad; datagram too big.
    pub drop_bad_len: u64,
    /// Sender is broadcast/multicast/zero (never answered or learned).
    pub drop_bad_src: u64,
    /// ICMP type other than echo request/reply.
    pub drop_icmp_other: u64,
    /// Echo reply matching no outstanding request.
    pub drop_unmatched_reply: u64,
    /// Non-zero UDP checksum that does not verify.
    pub drop_udp_csum: u64,
    /// UDP destination port with no socket.
    pub drop_udp_no_socket: u64,
    /// Socket ring or datagram pool full.
    pub drop_udp_rx_full: u64,
    /// DHCP message malformed, not ours (xid/chaddr) or unexpected in this state.
    pub drop_dhcp: u64,
    /// DNS response not matching the query (server, id, flags, question).
    pub drop_dns_mismatch: u64,
    /// DNS response matching but structurally broken.
    pub drop_dns_malformed: u64,
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
    pub dhcp_offers: u64,
    pub dhcp_acks: u64,
    pub dhcp_naks: u64,
    /// DHCP messages re-sent after a timeout.
    pub dhcp_retransmits: u64,
    /// Fallback configurations applied after no lease in time.
    pub dhcp_fallbacks: u64,
    pub dns_answers: u64,
    /// Queries finished with an error (including timeouts).
    pub dns_failures: u64,
    pub dns_timeouts: u64,
    pub dns_retries: u64,
    pub dns_cache_hits: u64,
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
            + self.drop_udp_csum
            + self.drop_udp_no_socket
            + self.drop_udp_rx_full
            + self.drop_dhcp
            + self.drop_dns_mismatch
            + self.drop_dns_malformed
    }
}

/// A bound UDP socket. Stale ids (after `udp_unbind`) are rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SockId {
    idx: u8,
    gen: u8,
}

impl SockId {
    pub fn index(&self) -> usize {
        self.idx as usize
    }
}

/// A DNS query started by `dns_query`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryId {
    idx: u8,
    gen: u8,
}

/// DHCP client state as seen by callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DhcpState {
    /// Never started (or stopped).
    Idle,
    /// DISCOVER sent, waiting for an OFFER.
    Selecting,
    /// REQUEST sent for an OFFER, waiting for the ACK.
    Requesting,
    /// Lease held (`lease_s` = u32::MAX for an infinite lease).
    Bound { lease_s: u32 },
    /// Past T1: unicast REQUESTs to the leasing server.
    Renewing { lease_s: u32 },
    /// Past T2: broadcast REQUESTs to any server.
    Rebinding { lease_s: u32 },
    /// No lease in time (the fallback, if any, was applied).
    Failed,
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

#[derive(Clone, Copy)]
struct UdpSock {
    used: bool,
    gen: u8,
    port: u16,
    /// Built-in echo service: every datagram is answered, nothing is queued.
    echo: bool,
    /// Pool indices, FIFO from `head`.
    ring: [u8; UDP_RING],
    head: u8,
    count: u8,
}

const SOCK_EMPTY: UdpSock = UdpSock { used: false, gen: 0, port: 0, echo: false, ring: [0; UDP_RING], head: 0, count: 0 };

#[derive(Clone, Copy)]
struct Datagram {
    used: bool,
    src_ip: [u8; 4],
    src_port: u16,
    len: u16,
    data: [u8; UDP_MAX_PAYLOAD],
}

const DGRAM_EMPTY: Datagram = Datagram { used: false, src_ip: [0; 4], src_port: 0, len: 0, data: [0; UDP_MAX_PAYLOAD] };

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Idle,
    Selecting,
    Requesting,
    Bound,
    Renewing,
    Rebinding,
    Failed,
}

#[derive(Clone, Copy)]
struct Dhcp {
    phase: Phase,
    xid: u32,
    /// Start of the current transaction (`secs` field).
    start_ms: u64,
    /// SELECTING/REQUESTING: apply the fallback (or fail) at this time.
    deadline_ms: u64,
    fallback: Option<IpConfig>,
    next_tx_ms: u64,
    interval_ms: u64,
    /// Messages sent in the current state.
    tries: u8,
    offered: [u8; 4],
    server: [u8; 4],
    lease_s: u32,
    t1_ms: u64,
    t2_ms: u64,
    expire_ms: u64,
}

const DHCP_IDLE: Dhcp = Dhcp {
    phase: Phase::Idle,
    xid: 0,
    start_ms: 0,
    deadline_ms: 0,
    fallback: None,
    next_tx_ms: 0,
    interval_ms: DHCP_RETRY_MS,
    tries: 0,
    offered: [0; 4],
    server: [0; 4],
    lease_s: 0,
    t1_ms: 0,
    t2_ms: 0,
    expire_ms: 0,
};

#[derive(Clone, Copy)]
struct DnsQuery {
    used: bool,
    gen: u8,
    name: WireName,
    id: u16,
    port: u16,
    server: [u8; 4],
    sent_ms: u64,
    attempts: u8,
    result: Option<Result<[u8; 4], DnsError>>,
    done_ms: u64,
}

impl DnsQuery {
    fn in_flight(&self) -> bool {
        self.used && self.result.is_none()
    }
}

const QUERY_EMPTY: DnsQuery = DnsQuery {
    used: false,
    gen: 0,
    name: WireName::EMPTY,
    id: 0,
    port: 0,
    server: [0; 4],
    sent_ms: 0,
    attempts: 0,
    result: None,
    done_ms: 0,
};

#[derive(Clone, Copy)]
struct CacheEntry {
    used: bool,
    name: WireName,
    ip: [u8; 4],
    expires_ms: u64,
}

const CACHE_EMPTY: CacheEntry = CacheEntry { used: false, name: WireName::EMPTY, ip: [0; 4], expires_ms: 0 };

pub struct Iface {
    mac: [u8; 6],
    cfg: IpConfig,
    log: fn(&str),
    arp: ArpCache,
    pending: [Pending; PENDING_MAX],
    waits: [ArpWait; PENDING_MAX],
    echoes: [Echo; ECHO_TABLE],
    socks: [UdpSock; UDP_SOCKETS],
    pool: [Datagram; UDP_POOL],
    dhcp: Dhcp,
    queries: [DnsQuery; DNS_QUERIES],
    cache: [CacheEntry; DNS_CACHE],
    ip_id: u16,
    /// Internal pseudo-random state (ephemeral ports).
    rand: u32,
    /// Latest time seen by `poll`/`ping_send` (for `&self` queries).
    now_ms: u64,
    /// Announce a newly leased address with one gratuitous ARP on the next poll.
    garp_pending: bool,
    stats: StackStats,
}

fn count_tx_proto(stats: &mut StackStats, proto: u8) {
    if proto == PROTO_ICMP {
        stats.tx_icmp += 1;
    } else if proto == PROTO_UDP {
        stats.tx_udp += 1;
    }
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
    count_tx_proto(stats, pkt.get(9).copied().unwrap_or(0));
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
            socks: [SOCK_EMPTY; UDP_SOCKETS],
            pool: [DGRAM_EMPTY; UDP_POOL],
            dhcp: DHCP_IDLE,
            queries: [QUERY_EMPTY; DNS_QUERIES],
            cache: [CACHE_EMPTY; DNS_CACHE],
            ip_id: 1,
            rand: mix32(u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]])),
            now_ms: 0,
            garp_pending: false,
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

    fn next_rand(&mut self) -> u32 {
        self.rand = mix32(self.rand);
        self.rand
    }

    /// Subnet-directed broadcast address of our configured subnet.
    fn is_subnet_broadcast(&self, a: [u8; 4]) -> bool {
        if self.cfg.ip == [0; 4] || self.cfg.prefix == 0 || self.cfg.prefix > 30 {
            return false;
        }
        let m = ipv4::mask(self.cfg.prefix);
        u32::from_be_bytes(a) == (u32::from_be_bytes(self.cfg.ip) | !m)
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

    /// Reap TX, process received frames (up to `POLL_RX_CAP`), run the DHCP, DNS
    /// and ARP timers, then one `tx_flush`.
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
        self.dhcp_timers(dev, now_ms);
        self.dns_timers(dev, now_ms);
        self.arp_timers(dev, now_ms);
        if self.garp_pending && self.cfg.ip != [0; 4] {
            // Gratuitous ARP (sender = target = our new address): refreshes
            // neighbours' caches and surfaces an address conflict early.
            self.garp_pending = false;
            let ip = self.cfg.ip;
            self.send_arp_request(dev, ip);
            self.stats.tx_garp += 1;
        }
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

    /// Send an IPv4 packet from our address (see `send_ipv4_from`).
    fn send_ipv4(
        &mut self,
        dev: &mut dyn NetDevice,
        dst: [u8; 4],
        proto: u8,
        l4_len: usize,
        now_ms: u64,
        fill: impl FnOnce(&mut [u8]),
    ) -> Result<(), NetError> {
        let src = self.cfg.ip;
        self.send_ipv4_from(dev, src, dst, proto, l4_len, now_ms, fill)
    }

    /// Send an IPv4 packet from `src` with an `l4_len`-byte payload written by
    /// `fill`. Broadcasts (limited or subnet-directed) go to the Ethernet broadcast
    /// MAC without ARP; `src` may be 0.0.0.0 only for limited broadcast (DHCP).
    /// Queues behind ARP when the next hop is unresolved (returns Ok).
    #[allow(clippy::too_many_arguments)]
    fn send_ipv4_from(
        &mut self,
        dev: &mut dyn NetDevice,
        src: [u8; 4],
        dst: [u8; 4],
        proto: u8,
        l4_len: usize,
        now_ms: u64,
        fill: impl FnOnce(&mut [u8]),
    ) -> Result<(), NetError> {
        let bcast = ipv4::is_broadcast(dst) || self.is_subnet_broadcast(dst);
        if src == [0; 4] && !ipv4::is_broadcast(dst) {
            return Err(NetError::NotConfigured);
        }
        let total = ipv4::HDR_LEN + l4_len;
        if l4_len > MAX_PACKET - ipv4::HDR_LEN {
            return Err(NetError::TooBig);
        }
        let hop = if bcast { dst } else { self.next_hop(dst).ok_or(NetError::NoRoute)? };
        let mac = if bcast { Some(BROADCAST) } else { self.arp.lookup(hop, now_ms) };
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
                let _ = ipv4::write(ih, src, dst, proto, id, l4_len);
                fill(l4);
            }
            if dev.tx_submit(buf, flen).is_err() {
                self.stats.tx_errors += 1;
                return Err(NetError::TxFailed);
            }
            self.stats.tx_frames += 1;
            self.stats.tx_ipv4 += 1;
            count_tx_proto(&mut self.stats, proto);
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
        let _ = ipv4::write(ih, src, dst, proto, id, l4_len);
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
        let configured = self.cfg.ip != [0; 4];
        if configured && h.dst == self.cfg.ip {
            match h.proto {
                PROTO_ICMP => self.handle_icmp(dev, &h, payload, now_ms),
                PROTO_UDP => self.handle_udp(dev, &h, payload, now_ms),
                _ => self.stats.drop_unknown_proto += 1,
            }
            return;
        }
        // Not unicast to us: only UDP to a broadcast address, or to the DHCP client
        // port while unconfigured or while DHCP runs (servers may unicast to the
        // offered address before we own it).
        let bcast = ipv4::is_broadcast(h.dst) || self.is_subnet_broadcast(h.dst);
        let dhcp_port = be16(payload, 2) == Some(dhcp::CLIENT_PORT) && (!configured || self.dhcp_active());
        if h.proto == PROTO_UDP && (bcast || dhcp_port) {
            self.handle_udp(dev, &h, payload, now_ms);
        } else {
            self.stats.drop_not_for_us += 1;
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

    // ---- UDP -----------------------------------------------------------------

    /// Send one UDP datagram (checksum always generated).
    #[allow(clippy::too_many_arguments)]
    fn send_udp(
        &mut self,
        dev: &mut dyn NetDevice,
        src: [u8; 4],
        sport: u16,
        dst: [u8; 4],
        dport: u16,
        payload: &[u8],
        now_ms: u64,
    ) -> Result<(), NetError> {
        if payload.len() > UDP_MAX_PAYLOAD {
            return Err(NetError::TooBig);
        }
        self.send_ipv4_from(dev, src, dst, PROTO_UDP, udp::HDR_LEN + payload.len(), now_ms, |b| {
            let _ = udp::write(b, src, dst, sport, dport, payload);
        })
    }

    fn handle_udp(&mut self, dev: &mut dyn NetDevice, ip: &Ipv4Header, seg: &[u8], now_ms: u64) {
        self.stats.rx_udp += 1;
        let (h, payload) = match udp::parse(ip.src, ip.dst, seg) {
            Ok(v) => v,
            Err(e) => {
                match e {
                    UdpError::Short => self.stats.drop_short += 1,
                    UdpError::BadLen => self.stats.drop_bad_len += 1,
                    UdpError::BadCsum => self.stats.drop_udp_csum += 1,
                }
                return;
            }
        };
        if ipv4::is_bad_source(ip.src) {
            self.stats.drop_bad_src += 1;
            return;
        }
        if h.dst_port == dhcp::CLIENT_PORT && self.dhcp_active() {
            self.handle_dhcp(dev, payload, now_ms);
            return;
        }
        if h.src_port == dns::PORT {
            if let Some(qi) = self.queries.iter().position(|q| q.in_flight() && q.port == h.dst_port) {
                self.handle_dns(qi, ip.src, payload, now_ms);
                return;
            }
        }
        let Some(si) = self.socks.iter().position(|s| s.used && s.port == h.dst_port) else {
            self.stats.drop_udp_no_socket += 1;
            return;
        };
        let Some(s) = self.socks.get(si).copied() else { return };
        if s.echo {
            self.stats.rx_delivered += 1;
            let src = self.cfg.ip;
            if self.send_udp(dev, src, h.dst_port, ip.src, h.src_port, payload, now_ms).is_ok() {
                self.stats.tx_udp_echo += 1;
            }
            return;
        }
        let len = payload.len();
        if len > UDP_MAX_PAYLOAD {
            self.stats.drop_bad_len += 1;
            return;
        }
        if s.count as usize >= UDP_RING {
            self.stats.drop_udp_rx_full += 1;
            return;
        }
        let Some(pi) = self.pool.iter().position(|d| !d.used) else {
            self.stats.drop_udp_rx_full += 1;
            return;
        };
        if let Some(d) = self.pool.get_mut(pi) {
            d.used = true;
            d.src_ip = ip.src;
            d.src_port = h.src_port;
            d.len = len as u16;
            if let Some(dst) = d.data.get_mut(..len) {
                dst.copy_from_slice(payload);
            }
        }
        if let Some(s) = self.socks.get_mut(si) {
            let tail = (s.head as usize + s.count as usize) % UDP_RING;
            if let Some(r) = s.ring.get_mut(tail) {
                *r = pi as u8;
            }
            s.count += 1;
        }
        self.stats.rx_delivered += 1;
    }

    fn port_busy(&self, port: u16) -> bool {
        self.socks.iter().any(|s| s.used && s.port == port)
            || (port == dhcp::CLIENT_PORT && self.dhcp_active())
            || self.queries.iter().any(|q| q.in_flight() && q.port == port)
    }

    /// A free port in 49152..=65535 derived from `seed`.
    fn ephemeral_port(&self, seed: u32) -> Option<u16> {
        let mut x = seed;
        for _ in 0..64 {
            let p = EPHEMERAL_BASE + (x % 16384) as u16;
            if !self.port_busy(p) {
                return Some(p);
            }
            x = mix32(x);
        }
        None
    }

    fn sock_index(&self, id: SockId) -> Option<usize> {
        let i = id.idx as usize;
        self.socks.get(i).filter(|s| s.used && s.gen == id.gen).map(|_| i)
    }

    fn bind_inner(&mut self, port: u16, echo: bool) -> Result<SockId, NetError> {
        let port = if port == 0 {
            let seed = self.next_rand();
            self.ephemeral_port(seed).ok_or(NetError::NoSockets)?
        } else {
            port
        };
        if self.port_busy(port) {
            return Err(NetError::PortInUse);
        }
        let i = self.socks.iter().position(|s| !s.used).ok_or(NetError::NoSockets)?;
        let s = self.socks.get_mut(i).ok_or(NetError::NoSockets)?;
        let gen = s.gen.wrapping_add(1);
        *s = UdpSock { used: true, gen, port, echo, ring: [0; UDP_RING], head: 0, count: 0 };
        Ok(SockId { idx: i as u8, gen })
    }

    /// Bind a UDP socket to `port` (0 = pick an ephemeral port). Datagrams to the
    /// port are queued (up to `UDP_RING`) for `udp_recv`.
    pub fn udp_bind(&mut self, port: u16) -> Result<SockId, NetError> {
        self.bind_inner(port, false)
    }

    /// Start the built-in echo service on `port`: `poll` answers every datagram to
    /// the port with the same payload. Stop it with `udp_unbind`.
    pub fn udp_echo(&mut self, port: u16) -> Result<SockId, NetError> {
        self.bind_inner(port, true)
    }

    /// Unbind a socket and discard its queued datagrams. False if already unbound.
    pub fn udp_unbind(&mut self, id: SockId) -> bool {
        let Some(i) = self.sock_index(id) else { return false };
        let Some(s) = self.socks.get(i).copied() else { return false };
        for k in 0..s.count as usize {
            let slot = s.ring.get((s.head as usize + k) % UDP_RING).copied().unwrap_or(0) as usize;
            if let Some(d) = self.pool.get_mut(slot) {
                d.used = false;
            }
        }
        if let Some(s) = self.socks.get_mut(i) {
            s.used = false;
            s.count = 0;
        }
        true
    }

    /// The port a socket is bound to.
    pub fn udp_port(&self, id: SockId) -> Option<u16> {
        self.sock_index(id).and_then(|i| self.socks.get(i)).map(|s| s.port)
    }

    /// Datagrams queued on a socket.
    pub fn udp_queued(&self, id: SockId) -> usize {
        self.sock_index(id).and_then(|i| self.socks.get(i)).map_or(0, |s| s.count as usize)
    }

    /// Datagram buffers in use across all sockets (leak check).
    pub fn udp_pool_used(&self) -> usize {
        self.pool.iter().filter(|d| d.used).count()
    }

    /// Dequeue the oldest datagram: copies its payload into `buf` (truncated to
    /// `buf.len()`) and returns (source ip, source port, bytes copied).
    pub fn udp_recv(&mut self, id: SockId, buf: &mut [u8]) -> Option<([u8; 4], u16, usize)> {
        let i = self.sock_index(id)?;
        let s = self.socks.get_mut(i)?;
        if s.count == 0 {
            return None;
        }
        let slot = *s.ring.get(s.head as usize)? as usize;
        s.head = ((s.head as usize + 1) % UDP_RING) as u8;
        s.count -= 1;
        let d = self.pool.get_mut(slot)?;
        d.used = false;
        let n = (d.len as usize).min(buf.len());
        if let (Some(dst), Some(src)) = (buf.get_mut(..n), d.data.get(..n)) {
            dst.copy_from_slice(src);
        }
        Some((d.src_ip, d.src_port, n))
    }

    /// Send a datagram from our address and `src_port` (any port; it need not be
    /// bound). Goes through ARP like ICMP (queued if unresolved). Flushes.
    #[allow(clippy::too_many_arguments)]
    pub fn udp_send_to(
        &mut self,
        dev: &mut dyn NetDevice,
        src_port: u16,
        dst_ip: [u8; 4],
        dst_port: u16,
        payload: &[u8],
        now_ms: u64,
    ) -> Result<(), NetError> {
        self.now_ms = now_ms;
        let src = self.cfg.ip;
        let r = self.send_udp(dev, src, src_port, dst_ip, dst_port, payload, now_ms);
        dev.tx_flush();
        r
    }

    /// `udp_send_to` from a bound socket's port.
    pub fn udp_send(
        &mut self,
        dev: &mut dyn NetDevice,
        sock: SockId,
        dst_ip: [u8; 4],
        dst_port: u16,
        payload: &[u8],
        now_ms: u64,
    ) -> Result<(), NetError> {
        let port = self.udp_port(sock).ok_or(NetError::BadSocket)?;
        self.udp_send_to(dev, port, dst_ip, dst_port, payload, now_ms)
    }

    // ---- DHCP client ---------------------------------------------------------

    fn dhcp_active(&self) -> bool {
        !matches!(self.dhcp.phase, Phase::Idle | Phase::Failed)
    }

    pub fn dhcp_state(&self) -> DhcpState {
        let lease_s = self.dhcp.lease_s;
        match self.dhcp.phase {
            Phase::Idle => DhcpState::Idle,
            Phase::Selecting => DhcpState::Selecting,
            Phase::Requesting => DhcpState::Requesting,
            Phase::Bound => DhcpState::Bound { lease_s },
            Phase::Renewing => DhcpState::Renewing { lease_s },
            Phase::Rebinding => DhcpState::Rebinding { lease_s },
            Phase::Failed => DhcpState::Failed,
        }
    }

    /// The leasing server (valid once bound).
    pub fn dhcp_server(&self) -> [u8; 4] {
        self.dhcp.server
    }

    /// The transaction id in use (tests and debugging).
    pub fn dhcp_xid(&self) -> u32 {
        self.dhcp.xid
    }

    /// Start (or restart) DHCP: DISCOVER now, then `poll` drives the exchange. The
    /// current configuration stays in place until an ACK replaces it. If no lease
    /// arrives within `DHCP_FALLBACK_MS`, `fallback` is applied (and logged) and the
    /// client stops in `Failed`; with `None` it keeps trying until
    /// `DHCP_GIVE_UP_MS`. While DHCP runs, UDP port 68 belongs to it.
    pub fn dhcp_start(&mut self, dev: &mut dyn NetDevice, now_ms: u64, xid_seed: u32, fallback: Option<IpConfig>) {
        self.now_ms = now_ms;
        let wait = if fallback.is_some() { DHCP_FALLBACK_MS } else { DHCP_GIVE_UP_MS };
        self.dhcp = Dhcp {
            phase: Phase::Selecting,
            xid: mix32(xid_seed),
            start_ms: now_ms,
            deadline_ms: now_ms.saturating_add(wait),
            fallback,
            next_tx_ms: now_ms,
            ..DHCP_IDLE
        };
        self.dhcp_timers(dev, now_ms);
        dev.tx_flush();
    }

    /// Stop the client (the configuration is kept).
    pub fn dhcp_stop(&mut self) {
        self.dhcp.phase = Phase::Idle;
    }

    /// Back to SELECTING with a fresh xid; DISCOVER goes out on the next timer run.
    fn dhcp_restart(&mut self, now_ms: u64, deadline_ms: u64, fallback: Option<IpConfig>) {
        let d = &mut self.dhcp;
        d.phase = Phase::Selecting;
        d.xid = mix32(d.xid);
        d.start_ms = now_ms;
        d.deadline_ms = deadline_ms;
        d.fallback = fallback;
        d.next_tx_ms = now_ms;
        d.interval_ms = DHCP_RETRY_MS;
        d.tries = 0;
    }

    fn dhcp_send(&mut self, dev: &mut dyn NetDevice, now_ms: u64) {
        let d = self.dhcp;
        let ip = self.cfg.ip;
        let bc = [255u8; 4];
        let (msg_type, src, dst, flags, ciaddr, requested_ip, server_id) = match d.phase {
            Phase::Selecting => (dhcp::DISCOVER, [0; 4], bc, dhcp::FLAG_BROADCAST, [0; 4], None, None),
            Phase::Requesting => {
                (dhcp::REQUEST, [0; 4], bc, dhcp::FLAG_BROADCAST, [0; 4], Some(d.offered), Some(d.server))
            }
            Phase::Renewing => (dhcp::REQUEST, ip, d.server, 0, ip, None, None),
            Phase::Rebinding => (dhcp::REQUEST, ip, bc, 0, ip, None, None),
            _ => return,
        };
        let req = DhcpRequest {
            msg_type,
            xid: d.xid,
            secs: dhcp::secs_since(d.start_ms, now_ms),
            flags,
            ciaddr,
            chaddr: self.mac,
            requested_ip,
            server_id,
        };
        let mut buf = [0u8; dhcp::MAX_LEN];
        let Some(n) = dhcp::build(&mut buf, &req) else { return };
        let msg = buf.get(..n).unwrap_or(&[]);
        if self.send_udp(dev, src, dhcp::CLIENT_PORT, dst, dhcp::SERVER_PORT, msg, now_ms).is_ok() {
            self.stats.tx_dhcp += 1;
        }
        let d = &mut self.dhcp;
        d.tries = d.tries.saturating_add(1);
        if d.tries > 1 {
            self.stats.dhcp_retransmits += 1;
        }
    }

    fn dhcp_timers(&mut self, dev: &mut dyn NetDevice, now_ms: u64) {
        let d = self.dhcp;
        match d.phase {
            Phase::Idle | Phase::Failed => {}
            Phase::Selecting | Phase::Requesting => {
                if now_ms >= d.deadline_ms {
                    self.dhcp.phase = Phase::Failed;
                    match d.fallback {
                        Some(fb) => {
                            self.cfg = fb;
                            self.stats.dhcp_fallbacks += 1;
                            self.logf(format_args!("net: dhcp timeout, static {}/{}", Ip(fb.ip), fb.prefix));
                        }
                        None => self.logf(format_args!("net: dhcp fail")),
                    }
                    return;
                }
                if now_ms < d.next_tx_ms {
                    return;
                }
                if d.phase == Phase::Requesting && d.tries >= DHCP_REQUEST_TRIES {
                    self.dhcp_restart(now_ms, d.deadline_ms, d.fallback);
                }
                self.dhcp_send(dev, now_ms);
                let d = &mut self.dhcp;
                d.next_tx_ms = now_ms.saturating_add(d.interval_ms);
                d.interval_ms = (d.interval_ms * 2).min(DHCP_RETRY_MAX_MS);
            }
            Phase::Bound | Phase::Renewing | Phase::Rebinding => {
                if d.phase == Phase::Bound && now_ms >= d.t1_ms {
                    let d = &mut self.dhcp;
                    d.phase = Phase::Renewing;
                    d.xid = mix32(d.xid);
                    d.start_ms = now_ms;
                    d.next_tx_ms = now_ms;
                    d.tries = 0;
                }
                if self.dhcp.phase == Phase::Renewing && now_ms >= self.dhcp.t2_ms {
                    let d = &mut self.dhcp;
                    d.phase = Phase::Rebinding;
                    d.next_tx_ms = now_ms;
                    d.tries = 0;
                }
                if self.dhcp.phase == Phase::Rebinding && now_ms >= self.dhcp.expire_ms {
                    self.cfg = IpConfig::NONE;
                    self.logf(format_args!("net: dhcp lease expired"));
                    self.dhcp_restart(now_ms, now_ms.saturating_add(DHCP_GIVE_UP_MS), None);
                    self.dhcp_send(dev, now_ms);
                    let d = &mut self.dhcp;
                    d.next_tx_ms = now_ms.saturating_add(d.interval_ms);
                    d.interval_ms = (d.interval_ms * 2).min(DHCP_RETRY_MAX_MS);
                    return;
                }
                let d = self.dhcp;
                if d.phase == Phase::Bound || now_ms < d.next_tx_ms {
                    return;
                }
                let limit = if d.phase == Phase::Renewing { d.t2_ms } else { d.expire_ms };
                self.dhcp_send(dev, now_ms);
                let wait = (limit.saturating_sub(now_ms) / 2).max(DHCP_RENEW_MIN_MS);
                self.dhcp.next_tx_ms = now_ms.saturating_add(wait).min(limit);
            }
        }
    }

    fn handle_dhcp(&mut self, dev: &mut dyn NetDevice, payload: &[u8], now_ms: u64) {
        self.stats.rx_dhcp += 1;
        let Ok(r) = dhcp::parse(payload) else {
            self.stats.drop_dhcp += 1;
            return;
        };
        let d = self.dhcp;
        let ours = r.op == dhcp::OP_REPLY && r.xid == d.xid && r.chaddr == self.mac;
        let server_ok = r.server_id.map_or(true, |s| s == d.server);
        let accepted = ours
            && match (d.phase, r.msg_type) {
                (Phase::Selecting, Some(dhcp::OFFER)) => {
                    match r.server_id {
                        Some(sid) if valid_lease_addr(r.yiaddr) => {
                            self.stats.dhcp_offers += 1;
                            let d = &mut self.dhcp;
                            d.phase = Phase::Requesting;
                            d.offered = r.yiaddr;
                            d.server = sid;
                            d.tries = 0;
                            d.interval_ms = DHCP_RETRY_MS;
                            self.dhcp_send(dev, now_ms);
                            let d = &mut self.dhcp;
                            d.next_tx_ms = now_ms.saturating_add(d.interval_ms);
                            d.interval_ms = (d.interval_ms * 2).min(DHCP_RETRY_MAX_MS);
                            true
                        }
                        _ => false,
                    }
                }
                (Phase::Requesting, Some(dhcp::ACK)) if server_ok && valid_lease_addr(r.yiaddr) => {
                    self.dhcp_bind(&r, now_ms);
                    true
                }
                (Phase::Renewing, Some(dhcp::ACK)) | (Phase::Rebinding, Some(dhcp::ACK))
                    if valid_lease_addr(r.yiaddr) =>
                {
                    self.dhcp_bind(&r, now_ms);
                    true
                }
                (Phase::Requesting, Some(dhcp::NAK)) if server_ok => {
                    self.stats.dhcp_naks += 1;
                    self.logf(format_args!("net: dhcp nak"));
                    self.dhcp_restart(now_ms, d.deadline_ms, d.fallback);
                    true
                }
                (Phase::Renewing, Some(dhcp::NAK)) | (Phase::Rebinding, Some(dhcp::NAK))
                    if server_ok || d.phase == Phase::Rebinding =>
                {
                    self.stats.dhcp_naks += 1;
                    self.logf(format_args!("net: dhcp nak"));
                    self.cfg = IpConfig::NONE;
                    self.dhcp_restart(now_ms, now_ms.saturating_add(DHCP_GIVE_UP_MS), None);
                    true
                }
                _ => false,
            };
        if accepted {
            self.stats.rx_delivered += 1;
        } else {
            self.stats.drop_dhcp += 1;
        }
    }

    fn dhcp_bind(&mut self, r: &DhcpReply, now_ms: u64) {
        let ip = r.yiaddr;
        let prefix = r
            .subnet_mask
            .and_then(dhcp::mask_to_prefix)
            .filter(|p| (1..=32).contains(p))
            .unwrap_or_else(|| dhcp::classful_prefix(ip));
        let cfg = IpConfig { ip, prefix, gw: r.router.unwrap_or([0; 4]), dns: r.dns.unwrap_or([0; 4]) };
        let lease_s = r.lease_s.unwrap_or(u32::MAX);
        let (t1, t2, exp) = if lease_s == u32::MAX {
            (u64::MAX, u64::MAX, u64::MAX)
        } else {
            let l = lease_s.max(DHCP_MIN_LEASE_S) as u64 * 1000;
            let t2 = r.t2_s.map(|t| t as u64 * 1000).filter(|&t| t <= l).unwrap_or(l / 8 * 7);
            let t1 = r.t1_s.map(|t| t as u64 * 1000).filter(|&t| t <= t2).unwrap_or(l / 2).min(t2);
            (now_ms.saturating_add(t1), now_ms.saturating_add(t2), now_ms.saturating_add(l))
        };
        self.stats.dhcp_acks += 1;
        self.cfg = cfg;
        let d = &mut self.dhcp;
        d.phase = Phase::Bound;
        if let Some(s) = r.server_id {
            d.server = s;
        }
        d.lease_s = lease_s;
        d.t1_ms = t1;
        d.t2_ms = t2;
        d.expire_ms = exp;
        d.tries = 0;
        self.garp_pending = true;
        self.logf(format_args!("net: dhcp {} gw {} dns {}", Ip(cfg.ip), Ip(cfg.gw), Ip(cfg.dns)));
    }

    // ---- DNS -----------------------------------------------------------------

    /// Resolve `name` to an IPv4 address. A dotted quad or a cached name completes
    /// at once (no query); otherwise an A query goes to `cfg.dns` from an ephemeral
    /// port, with the id and port derived from `seed`, and `poll` drives it (2 s
    /// timeout, 2 attempts). Read the outcome with `dns_result`.
    pub fn dns_query(
        &mut self,
        dev: &mut dyn NetDevice,
        name: &str,
        now_ms: u64,
        seed: u32,
    ) -> Result<QueryId, DnsError> {
        self.now_ms = now_ms;
        let w = match dns::encode_name(name) {
            Ok(w) => w,
            Err(e) => {
                self.logf(format_args!("net: dns {} fail {}", name, e.reason()));
                return Err(e);
            }
        };
        if let Some(ip) = dns::parse_dotted_quad(name) {
            return self.dns_complete_now(&w, ip, now_ms);
        }
        if let Some(ip) = self.cache_lookup(&w, now_ms) {
            self.stats.dns_cache_hits += 1;
            return self.dns_complete_now(&w, ip, now_ms);
        }
        let fail = |s: &Iface, e: DnsError| {
            s.logf(format_args!("net: dns {} fail {}", w, e.reason()));
            Err(e)
        };
        if self.cfg.dns == [0; 4] {
            return fail(self, DnsError::NoServer);
        }
        let Some(qi) = self.dns_alloc() else { return fail(self, DnsError::Busy) };
        let x = mix32(seed);
        let Some(port) = self.ephemeral_port(mix32(x ^ 0x5bd1_e995)) else { return fail(self, DnsError::Busy) };
        let server = self.cfg.dns;
        let Some(q) = self.queries.get_mut(qi) else { return fail(self, DnsError::Busy) };
        let gen = q.gen.wrapping_add(1);
        *q = DnsQuery {
            used: true,
            gen,
            name: w,
            id: (x ^ (x >> 16)) as u16,
            port,
            server,
            sent_ms: now_ms,
            attempts: 1,
            result: None,
            done_ms: 0,
        };
        match self.dns_send(dev, qi, now_ms) {
            Err(e @ (NetError::NotConfigured | NetError::NoRoute | NetError::TooBig)) => {
                if let Some(q) = self.queries.get_mut(qi) {
                    q.used = false;
                }
                dev.tx_flush();
                return fail(self, DnsError::Net(e));
            }
            // TX full / ARP queue full: the retry at the timeout sends it again.
            _ => {}
        }
        dev.tx_flush();
        Ok(QueryId { idx: qi as u8, gen })
    }

    /// The outcome of a query: None while in flight (or for an id whose slot was
    /// reused by a later query; finished slots are recycled oldest first).
    pub fn dns_result(&self, id: QueryId) -> Option<Result<[u8; 4], DnsError>> {
        let q = self.queries.get(id.idx as usize)?;
        if !q.used || q.gen != id.gen {
            return None;
        }
        q.result
    }

    /// Cached address for `name` (not expired).
    pub fn dns_cached(&self, name: &str) -> Option<[u8; 4]> {
        let w = dns::encode_name(name).ok()?;
        self.cache_lookup(&w, self.now_ms)
    }

    pub fn dns_cache_flush(&mut self) {
        for c in self.cache.iter_mut() {
            c.used = false;
        }
    }

    /// Queries still waiting for an answer.
    pub fn dns_in_flight(&self) -> usize {
        self.queries.iter().filter(|q| q.in_flight()).count()
    }

    /// A free slot, else the finished slot completed longest ago.
    fn dns_alloc(&self) -> Option<usize> {
        if let Some(i) = self.queries.iter().position(|q| !q.used) {
            return Some(i);
        }
        let mut best: Option<usize> = None;
        for (i, q) in self.queries.iter().enumerate() {
            if q.result.is_some() && best.map_or(true, |b| self.queries.get(b).map_or(true, |o| q.done_ms < o.done_ms)) {
                best = Some(i);
            }
        }
        best
    }

    fn dns_complete_now(&mut self, w: &WireName, ip: [u8; 4], now_ms: u64) -> Result<QueryId, DnsError> {
        let Some(qi) = self.dns_alloc() else {
            self.logf(format_args!("net: dns {} fail busy", w));
            return Err(DnsError::Busy);
        };
        let Some(q) = self.queries.get_mut(qi) else { return Err(DnsError::Busy) };
        let gen = q.gen.wrapping_add(1);
        *q = DnsQuery { used: true, gen, name: *w, result: None, ..QUERY_EMPTY };
        self.dns_finish(qi, Ok(ip), now_ms);
        Ok(QueryId { idx: qi as u8, gen })
    }

    fn dns_send(&mut self, dev: &mut dyn NetDevice, qi: usize, now_ms: u64) -> Result<(), NetError> {
        let Some(q) = self.queries.get(qi) else { return Err(NetError::TooBig) };
        let (port, server, id) = (q.port, q.server, q.id);
        let mut msg = [0u8; dns::MAX_MSG];
        let Some(n) = dns::build_query(&mut msg, id, &q.name) else { return Err(NetError::TooBig) };
        let src = self.cfg.ip;
        let r = self.send_udp(dev, src, port, server, dns::PORT, msg.get(..n).unwrap_or(&[]), now_ms);
        if r.is_ok() {
            self.stats.tx_dns += 1;
        }
        r
    }

    fn dns_finish(&mut self, qi: usize, r: Result<[u8; 4], DnsError>, now_ms: u64) {
        let Some(q) = self.queries.get_mut(qi) else { return };
        q.result = Some(r);
        q.done_ms = now_ms;
        let name = q.name;
        match r {
            Ok(ip) => {
                self.stats.dns_answers += 1;
                self.logf(format_args!("net: dns {} {}", name, Ip(ip)));
            }
            Err(e) => {
                self.stats.dns_failures += 1;
                if e == DnsError::Timeout {
                    self.stats.dns_timeouts += 1;
                }
                self.logf(format_args!("net: dns {} fail {}", name, e.reason()));
            }
        }
    }

    fn dns_timers(&mut self, dev: &mut dyn NetDevice, now_ms: u64) {
        for qi in 0..DNS_QUERIES {
            let Some(q) = self.queries.get(qi) else { break };
            if !q.in_flight() || now_ms.saturating_sub(q.sent_ms) < DNS_TIMEOUT_MS {
                continue;
            }
            if q.attempts >= DNS_ATTEMPTS {
                self.dns_finish(qi, Err(DnsError::Timeout), now_ms);
                continue;
            }
            if let Some(q) = self.queries.get_mut(qi) {
                q.attempts += 1;
                q.sent_ms = now_ms;
            }
            self.stats.dns_retries += 1;
            let _ = self.dns_send(dev, qi, now_ms);
        }
    }

    fn handle_dns(&mut self, qi: usize, src: [u8; 4], payload: &[u8], now_ms: u64) {
        self.stats.rx_dns += 1;
        let Some(q) = self.queries.get(qi) else {
            self.stats.drop_dns_mismatch += 1;
            return;
        };
        if q.server != src {
            self.stats.drop_dns_mismatch += 1;
            return;
        }
        match dns::parse_response(payload, q.id, &q.name) {
            Err(Reject::Mismatch) => self.stats.drop_dns_mismatch += 1,
            Err(Reject::Malformed) => self.stats.drop_dns_malformed += 1,
            Ok(ans) => {
                self.stats.rx_delivered += 1;
                match ans {
                    Ok((ip, ttl)) => {
                        let name = q.name;
                        self.cache_put(&name, ip, ttl, now_ms);
                        self.dns_finish(qi, Ok(ip), now_ms);
                    }
                    Err(e) => self.dns_finish(qi, Err(e), now_ms),
                }
            }
        }
    }

    fn cache_lookup(&self, w: &WireName, now_ms: u64) -> Option<[u8; 4]> {
        self.cache
            .iter()
            .find(|c| c.used && c.expires_ms > now_ms && c.name.eq_bytes(w.as_bytes()))
            .map(|c| c.ip)
    }

    fn cache_put(&mut self, w: &WireName, ip: [u8; 4], ttl_s: u32, now_ms: u64) {
        if ttl_s == 0 {
            return;
        }
        let expires_ms = now_ms.saturating_add(ttl_s.min(DNS_TTL_MAX_S) as u64 * 1000);
        let same = self.cache.iter().position(|c| c.used && c.name.eq_bytes(w.as_bytes()));
        let free = || self.cache.iter().position(|c| !c.used || c.expires_ms <= now_ms);
        let i = match same.or_else(free) {
            Some(i) => i,
            None => {
                // Evict the entry closest to expiry.
                let mut best = 0;
                for (i, c) in self.cache.iter().enumerate() {
                    if c.expires_ms < self.cache.get(best).map_or(u64::MAX, |b| b.expires_ms) {
                        best = i;
                    }
                }
                best
            }
        };
        if let Some(c) = self.cache.get_mut(i) {
            *c = CacheEntry { used: true, name: *w, ip, expires_ms };
        }
    }
}

/// An address a server may lease to us (unicast, not 0/8, multicast or class E).
fn valid_lease_addr(a: [u8; 4]) -> bool {
    !ipv4::is_bad_source(a) && a[0] != 127
}
