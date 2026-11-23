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
