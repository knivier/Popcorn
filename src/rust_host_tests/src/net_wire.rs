//! Test helpers for the net stack: a "wire" NetDevice (pairs connect two ifaces),
//! a thread-local log sink, raw frame builders and a tiny PRNG.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::drivers::net::{NetDevice, NetStats, RxBatch, TxBuf, TxError, MAX_FRAME};
use crate::util::csum;

pub type Queue = Rc<RefCell<VecDeque<Vec<u8>>>>;

const TX_SLOTS: usize = 16;

/// A NetDevice whose RX is `inbox` and whose flushed TX goes to `outbox`.
pub struct Wire {
    mac: [u8; 6],
    inbox: Queue,
    outbox: Queue,
    tx_bufs: Vec<Box<[u8; MAX_FRAME]>>,
    tx_busy: [bool; TX_SLOTS],
    pending: Vec<(u16, usize)>,
    lent: Vec<Option<Vec<u8>>>,
    stats: NetStats,
    /// Every frame flushed, in order (when `record` is set).
    pub history: Vec<Vec<u8>>,
    pub record: bool,
}

fn new_queue() -> Queue {
    Rc::new(RefCell::new(VecDeque::new()))
}

impl Wire {
    fn with(mac: [u8; 6], inbox: Queue, outbox: Queue) -> Wire {
        Wire {
            mac,
            inbox,
            outbox,
            tx_bufs: (0..TX_SLOTS).map(|_| Box::new([0u8; MAX_FRAME])).collect(),
            tx_busy: [false; TX_SLOTS],
            pending: Vec::new(),
            lent: Vec::new(),
            stats: NetStats::default(),
            history: Vec::new(),
            record: true,
        }
    }

    /// A lone device: tests inject RX and inspect TX with `take_sent`.
    pub fn single(mac: [u8; 6]) -> Wire {
        Wire::with(mac, new_queue(), new_queue())
    }

    /// Two devices connected back to back.
    pub fn pair(mac_a: [u8; 6], mac_b: [u8; 6]) -> (Wire, Wire) {
        let ab = new_queue();
        let ba = new_queue();
        (Wire::with(mac_a, ba.clone(), ab.clone()), Wire::with(mac_b, ab, ba))
    }

    pub fn inject(&self, frame: &[u8]) {
        self.inbox.borrow_mut().push_back(frame.to_vec());
    }

    /// Drain frames flushed so far (single-device mode).
    pub fn take_sent(&mut self) -> Vec<Vec<u8>> {
        self.outbox.borrow_mut().drain(..).collect()
    }

    pub fn rx_queued(&self) -> usize {
        self.inbox.borrow().len()
    }

    pub fn lent_count(&self) -> usize {
        self.lent.iter().filter(|l| l.is_some()).count()
    }

    pub fn tx_busy_count(&self) -> usize {
        self.tx_busy.iter().filter(|b| **b).count()
    }
}

impl NetDevice for Wire {
    fn name(&self) -> &'static str {
        "wire"
    }
    fn mac(&self) -> [u8; 6] {
        self.mac
    }
    fn mtu(&self) -> u16 {
        1500
    }
    fn link_up(&self) -> bool {
        true
    }
    fn rx_batch(&mut self, budget: usize, out: &mut RxBatch) -> usize {
        out.clear();
        let mut n = 0;
        while n < budget {
            let Some(f) = self.inbox.borrow_mut().pop_front() else { break };
            let id = match self.lent.iter().position(|l| l.is_none()) {
                Some(i) => i,
                None => {
                    self.lent.push(None);
                    self.lent.len() - 1
                }
            };
            let (ptr, len) = (f.as_ptr(), f.len() as u16);
            self.lent[id] = Some(f);
            assert!(out.push(id as u16, ptr, len));
            self.stats.rx_packets += 1;
            n += 1;
        }
        n
    }
    fn rx_recycle(&mut self, batch: &mut RxBatch) {
        for i in 0..batch.len() {
            let id = batch.id(i) as usize;
            assert!(self.lent[id].take().is_some(), "double recycle");
        }
        batch.clear();
    }
    fn tx_alloc(&mut self) -> Option<TxBuf> {
        let id = self.tx_busy.iter().position(|b| !b)?;
        self.tx_busy[id] = true;
        Some(TxBuf::new(id as u16, self.tx_bufs[id].as_mut_ptr(), MAX_FRAME as u16))
    }
    fn tx_submit(&mut self, buf: TxBuf, len: usize) -> Result<(), TxError> {
        let id = buf.id() as usize;
        assert!(self.tx_busy[id], "submit of a free buffer");
        if len > MAX_FRAME {
            self.tx_busy[id] = false;
            return Err(TxError);
        }
        // Pad like a real driver (zeros).
        if len < 60 {
            self.tx_bufs[id][len..60].fill(0);
        }
        self.pending.push((id as u16, len.max(60)));
        self.stats.tx_packets += 1;
        Ok(())
    }
    fn tx_flush(&mut self) {
        for (id, len) in self.pending.drain(..) {
            let f = self.tx_bufs[id as usize][..len].to_vec();
            if self.record {
                self.history.push(f.clone());
            }
            self.outbox.borrow_mut().push_back(f);
            self.tx_busy[id as usize] = false;
        }
        self.stats.kicks += 1;
    }
    fn reap_tx(&mut self, _budget: usize) -> usize {
        0
    }
    fn stats(&self) -> NetStats {
        let mut s = self.stats;
        s.tx_free = self.tx_busy.iter().filter(|b| !**b).count() as u32;
        s
    }
}

// ---- log sink -------------------------------------------------------------------

thread_local! {
    static LOGS: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

pub fn log(s: &str) {
    LOGS.with(|l| l.borrow_mut().push(s.to_string()));
}

pub fn count_log(line: &str) -> usize {
    LOGS.with(|l| l.borrow().iter().filter(|s| s.as_str() == line).count())
}

// ---- raw frame builders (independent of the stack's own builders) --------------

pub const MAC_A: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
pub const MAC_GW: [u8; 6] = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02];
pub const IP_A: [u8; 4] = [10, 0, 2, 15];
pub const IP_GW: [u8; 4] = [10, 0, 2, 2];

pub fn eth_hdr(dst: [u8; 6], src: [u8; 6], ty: u16) -> Vec<u8> {
    let mut f = Vec::with_capacity(64);
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ty.to_be_bytes());
    f
}

pub fn arp_frame(eth_dst: [u8; 6], op: u16, sha: [u8; 6], spa: [u8; 4], tha: [u8; 6], tpa: [u8; 4]) -> Vec<u8> {
    let mut f = eth_hdr(eth_dst, sha, 0x0806);
    f.extend_from_slice(&[0, 1, 8, 0, 6, 4]);
    f.extend_from_slice(&op.to_be_bytes());
    f.extend_from_slice(&sha);
    f.extend_from_slice(&spa);
    f.extend_from_slice(&tha);
    f.extend_from_slice(&tpa);
    f
}

/// ICMP echo message with a correct checksum.
pub fn icmp_echo(ty: u8, id: u16, seq: u16, payload: &[u8]) -> Vec<u8> {
    let mut m = vec![ty, 0, 0, 0];
    m.extend_from_slice(&id.to_be_bytes());
    m.extend_from_slice(&seq.to_be_bytes());
    m.extend_from_slice(payload);
    let c = csum::checksum(&m);
    m[2..4].copy_from_slice(&c.to_be_bytes());
    m
}

/// IPv4 header (20 bytes, correct checksum) for `payload_len`.
pub fn ip_hdr(src: [u8; 4], dst: [u8; 4], proto: u8, payload_len: usize) -> Vec<u8> {
    let total = (20 + payload_len) as u16;
    let mut h = vec![0x45, 0];
    h.extend_from_slice(&total.to_be_bytes());
    h.extend_from_slice(&[0x12, 0x34, 0x40, 0x00, 64, proto, 0, 0]);
    h.extend_from_slice(&src);
    h.extend_from_slice(&dst);
    fix_ip_csum(&mut h);
    h
}

pub fn fix_ip_csum(h: &mut [u8]) {
    let ihl = ((h[0] & 0x0F) as usize * 4).clamp(20, h.len());
    h[10] = 0;
    h[11] = 0;
    let c = csum::checksum(&h[..ihl]);
    h[10..12].copy_from_slice(&c.to_be_bytes());
}

/// Complete Ethernet + IPv4 + ICMP echo frame.
pub fn echo_frame(
    eth_dst: [u8; 6],
    eth_src: [u8; 6],
    src: [u8; 4],
    dst: [u8; 4],
    ty: u8,
    id: u16,
    seq: u16,
    payload: &[u8],
) -> Vec<u8> {
    let msg = icmp_echo(ty, id, seq, payload);
    let mut f = eth_hdr(eth_dst, eth_src, 0x0800);
    f.extend_from_slice(&ip_hdr(src, dst, 1, msg.len()));
    f.extend_from_slice(&msg);
    f
}

// ---- UDP / DHCP / DNS builders and parsers (independent of the stack) -----------

pub const MAC_B: [u8; 6] = [0x52, 0x54, 0x00, 0xAB, 0xCD, 0xEF];
pub const MAC_DNS: [u8; 6] = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x03];
pub const IP_DNS: [u8; 4] = [10, 0, 2, 3];
pub const BCAST_MAC: [u8; 6] = [0xFF; 6];

/// Byte-wise RFC 1071 reference over the concatenation of `parts` (u32
/// accumulate, fold after every word). Returns the inverted sum: 0 means valid.
pub fn ref_csum(parts: &[&[u8]]) -> u16 {
    let bytes: Vec<u8> = parts.iter().flat_map(|p| p.iter().copied()).collect();
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let hi = bytes[i] as u32;
        let lo = if i + 1 < bytes.len() { bytes[i + 1] as u32 } else { 0 };
        sum += (hi << 8) | lo;
        while sum > 0xFFFF {
            sum = (sum & 0xFFFF) + (sum >> 16);
        }
        i += 2;
    }
    !(sum as u16)
}

pub fn udp_pseudo(src: [u8; 4], dst: [u8; 4], udp_len: usize) -> Vec<u8> {
    let mut p = Vec::with_capacity(12);
    p.extend_from_slice(&src);
    p.extend_from_slice(&dst);
    p.extend_from_slice(&[0, 17]);
    p.extend_from_slice(&(udp_len as u16).to_be_bytes());
    p
}

/// UDP segment; with `with_csum` the checksum is computed by the reference (0 sent
/// as 0xFFFF), otherwise the field is 0 ("no checksum").
pub fn udp_seg(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16, payload: &[u8], with_csum: bool) -> Vec<u8> {
    let len = 8 + payload.len();
    let mut s = Vec::with_capacity(len);
    s.extend_from_slice(&sport.to_be_bytes());
    s.extend_from_slice(&dport.to_be_bytes());
    s.extend_from_slice(&(len as u16).to_be_bytes());
    s.extend_from_slice(&[0, 0]);
    s.extend_from_slice(payload);
    if with_csum {
        let mut c = ref_csum(&[&udp_pseudo(src, dst, len), &s]);
        if c == 0 {
            c = 0xFFFF;
        }
        s[6..8].copy_from_slice(&c.to_be_bytes());
    }
    s
}

/// Ethernet + IPv4 + raw UDP segment.
pub fn udp_frame_raw(eth_dst: [u8; 6], eth_src: [u8; 6], src: [u8; 4], dst: [u8; 4], seg: &[u8]) -> Vec<u8> {
    let mut f = eth_hdr(eth_dst, eth_src, 0x0800);
    f.extend_from_slice(&ip_hdr(src, dst, 17, seg.len()));
    f.extend_from_slice(seg);
    f
}

/// Ethernet + IPv4 + UDP with a correct checksum.
#[allow(clippy::too_many_arguments)]
pub fn udp_frame(
    eth_dst: [u8; 6],
    eth_src: [u8; 6],
    src: [u8; 4],
    dst: [u8; 4],
    sport: u16,
    dport: u16,
    payload: &[u8],
) -> Vec<u8> {
    udp_frame_raw(eth_dst, eth_src, src, dst, &udp_seg(src, dst, sport, dport, payload, true))
}

#[derive(Clone, Debug)]
pub struct UdpView {
    pub eth_dst: [u8; 6],
    pub eth_src: [u8; 6],
    pub src: [u8; 4],
    pub dst: [u8; 4],
    pub ttl: u8,
    pub sport: u16,
    pub dport: u16,
    pub csum: u16,
    pub csum_ok: bool,
    pub payload: Vec<u8>,
}

/// Parse an Ethernet/IPv4/UDP frame without the stack's parsers. None if not UDP.
pub fn parse_udp_frame(f: &[u8]) -> Option<UdpView> {
    if f.len() < 14 + 20 + 8 || f[12..14] != [0x08, 0x00] {
        return None;
    }
    let ip = &f[14..];
    let ihl = (ip[0] & 0x0F) as usize * 4;
    if ip[9] != 17 || ihl < 20 {
        return None;
    }
    let total = u16::from_be_bytes([ip[2], ip[3]]) as usize;
    let ip = ip.get(..total)?;
    let seg = ip.get(ihl..)?;
    let ulen = u16::from_be_bytes([seg[4], seg[5]]) as usize;
    let seg = seg.get(..ulen)?;
    let src = [ip[12], ip[13], ip[14], ip[15]];
    let dst = [ip[16], ip[17], ip[18], ip[19]];
    let csum = u16::from_be_bytes([seg[6], seg[7]]);
    let csum_ok = csum != 0 && ref_csum(&[&udp_pseudo(src, dst, ulen), seg]) == 0;
    Some(UdpView {
        eth_dst: f[0..6].try_into().unwrap(),
        eth_src: f[6..12].try_into().unwrap(),
        src,
        dst,
        ttl: ip[8],
        sport: u16::from_be_bytes([seg[0], seg[1]]),
        dport: u16::from_be_bytes([seg[2], seg[3]]),
        csum,
        csum_ok,
        payload: seg[8..].to_vec(),
    })
}

/// Target IP of an ARP request frame.
pub fn arp_request_target(f: &[u8]) -> Option<[u8; 4]> {
    if f.len() < 42 || f[12..14] != [0x08, 0x06] || f[20..22] != [0, 1] {
        return None;
    }
    Some([f[38], f[39], f[40], f[41]])
}

// DHCP

#[derive(Clone, Debug)]
pub struct DhcpMsg {
    pub op: u8,
    pub xid: u32,
    pub secs: u16,
    pub flags: u16,
    pub ciaddr: [u8; 4],
    pub yiaddr: [u8; 4],
    pub chaddr: [u8; 6],
    pub opts: Vec<(u8, Vec<u8>)>,
}

impl DhcpMsg {
    pub fn opt(&self, code: u8) -> Option<&[u8]> {
        self.opts.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_slice())
    }
    pub fn msg_type(&self) -> Option<u8> {
        self.opt(53).and_then(|v| v.first().copied())
    }
}

/// Parse a BOOTP/DHCP message (checks htype/hlen/cookie).
pub fn parse_dhcp(p: &[u8]) -> Option<DhcpMsg> {
    if p.len() < 240 || p[1] != 1 || p[2] != 6 || p[236..240] != [99, 130, 83, 99] {
        return None;
    }
    let mut opts = Vec::new();
    let mut i = 240;
    while i < p.len() {
        match p[i] {
            0 => i += 1,
            255 => break,
            c => {
                let l = *p.get(i + 1)? as usize;
                opts.push((c, p.get(i + 2..i + 2 + l)?.to_vec()));
                i += 2 + l;
            }
        }
    }
    Some(DhcpMsg {
        op: p[0],
        xid: u32::from_be_bytes(p[4..8].try_into().unwrap()),
        secs: u16::from_be_bytes([p[8], p[9]]),
        flags: u16::from_be_bytes([p[10], p[11]]),
        ciaddr: p[12..16].try_into().unwrap(),
        yiaddr: p[16..20].try_into().unwrap(),
        chaddr: p[28..34].try_into().unwrap(),
        opts,
    })
}

/// A server reply: option 53 first, then `opts`, END, padded to 300 bytes.
pub fn dhcp_reply(msg_type: u8, xid: u32, chaddr: [u8; 6], yiaddr: [u8; 4], opts: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut m = vec![0u8; 240];
    m[0] = 2;
    m[1] = 1;
    m[2] = 6;
    m[4..8].copy_from_slice(&xid.to_be_bytes());
    m[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    m[16..20].copy_from_slice(&yiaddr);
    m[20..24].copy_from_slice(&IP_GW); // siaddr
    m[28..34].copy_from_slice(&chaddr);
    m[236..240].copy_from_slice(&[99, 130, 83, 99]);
    m.extend_from_slice(&[53, 1, msg_type]);
    for (c, v) in opts {
        m.push(*c);
        m.push(v.len() as u8);
        m.extend_from_slice(v);
    }
    m.push(255);
    if m.len() < 300 {
        m.resize(300, 0);
    }
    m
}

/// The slirp-like option set: mask /24, router 10.0.2.2, DNS 10.0.2.3, server id
/// 10.0.2.2, lease `lease_s`.
pub fn slirp_opts(lease_s: u32) -> Vec<(u8, Vec<u8>)> {
    vec![
        (54, IP_GW.to_vec()),
        (1, vec![255, 255, 255, 0]),
        (3, IP_GW.to_vec()),
        (6, IP_DNS.to_vec()),
        (51, lease_s.to_be_bytes().to_vec()),
    ]
}

/// A frame from the DHCP server (10.0.2.2:67) to the client port.
pub fn dhcp_server_frame(eth_dst: [u8; 6], ip_dst: [u8; 4], payload: &[u8]) -> Vec<u8> {
    udp_frame(eth_dst, MAC_GW, IP_GW, ip_dst, 67, 68, payload)
}

// DNS

/// Wire-form name (`www.x.com` -> 3www1x3com0).
pub fn dns_name(s: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for l in s.split('.').filter(|l| !l.is_empty()) {
        v.push(l.len() as u8);
        v.extend_from_slice(l.as_bytes());
    }
    v.push(0);
    v
}

#[derive(Clone, Debug)]
pub struct DnsQueryView {
    pub id: u16,
    pub flags: u16,
    pub qdcount: u16,
    pub qname: Vec<u8>,
    pub qtype: u16,
    pub qclass: u16,
}

/// Parse a query (uncompressed question only).
pub fn parse_dns_query(p: &[u8]) -> Option<DnsQueryView> {
    let mut i = 12;
    loop {
        let l = *p.get(i)? as usize;
        i += 1 + l;
        if l == 0 {
            break;
        }
    }
    Some(DnsQueryView {
        id: u16::from_be_bytes([p[0], p[1]]),
        flags: u16::from_be_bytes([p[2], p[3]]),
        qdcount: u16::from_be_bytes([p[4], p[5]]),
        qname: p.get(12..i)?.to_vec(),
        qtype: u16::from_be_bytes([*p.get(i)?, *p.get(i + 1)?]),
        qclass: u16::from_be_bytes([*p.get(i + 2)?, *p.get(i + 3)?]),
    })
}

/// A resource record with a raw (possibly compressed) owner name and rdata.
#[derive(Clone, Debug)]
pub struct Rr {
    pub name: Vec<u8>,
    pub ty: u16,
    pub ttl: u32,
    pub rdata: Vec<u8>,
}

impl Rr {
    pub fn a(name: Vec<u8>, ttl: u32, ip: [u8; 4]) -> Rr {
        Rr { name, ty: 1, ttl, rdata: ip.to_vec() }
    }
    pub fn cname(name: Vec<u8>, ttl: u32, target: Vec<u8>) -> Rr {
        Rr { name, ty: 5, ttl, rdata: target }
    }
}

/// Pointer to the question name (offset 12).
pub const PTR_Q: [u8; 2] = [0xC0, 0x0C];

/// A response: header (QR, RD, RA + `rcode`), the question `qname` A/IN, answers.
pub fn dns_response(id: u16, rcode: u8, qname: &[u8], answers: &[Rr]) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend_from_slice(&id.to_be_bytes());
    m.extend_from_slice(&(0x8180u16 | rcode as u16).to_be_bytes());
    m.extend_from_slice(&1u16.to_be_bytes());
    m.extend_from_slice(&(answers.len() as u16).to_be_bytes());
    m.extend_from_slice(&[0, 0, 0, 0]);
    m.extend_from_slice(qname);
    m.extend_from_slice(&[0, 1, 0, 1]);
    for r in answers {
        m.extend_from_slice(&r.name);
        m.extend_from_slice(&r.ty.to_be_bytes());
        m.extend_from_slice(&[0, 1]);
        m.extend_from_slice(&r.ttl.to_be_bytes());
        m.extend_from_slice(&(r.rdata.len() as u16).to_be_bytes());
        m.extend_from_slice(&r.rdata);
    }
    m
}

/// A frame from the resolver (10.0.2.3:53) to us at `dport`.
pub fn dns_frame(dport: u16, payload: &[u8]) -> Vec<u8> {
    udp_frame(MAC_A, MAC_DNS, IP_DNS, IP_A, 53, dport, payload)
}

// ---- PRNG -------------------------------------------------------------------------

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    pub fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}
