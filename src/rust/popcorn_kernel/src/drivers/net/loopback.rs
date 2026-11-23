//! In-memory NetDevice: every transmitted frame comes back as received. Used by
//! the stack's self-tests (`net -selftest`) and by the host unit tests. Pure core.

use super::{NetDevice, NetStats, RxBatch, TxBuf, TxError, MAX_FRAME};

const SLOTS: usize = 16;

pub struct Loopback {
    mac: [u8; 6],
    tx_bufs: [[u8; MAX_FRAME]; SLOTS],
    tx_busy: [bool; SLOTS],
    rx_bufs: [[u8; MAX_FRAME]; SLOTS],
    rx_len: [u16; SLOTS],
    rx_lent: [bool; SLOTS],
    rx_head: usize,
    rx_count: usize,
    pending_tx: [(u16, u16); SLOTS],
    pending_n: usize,
    stats: NetStats,
}

impl Loopback {
    pub fn new(mac: [u8; 6]) -> Loopback {
        Loopback {
            mac,
            tx_bufs: [[0; MAX_FRAME]; SLOTS],
            tx_busy: [false; SLOTS],
            rx_bufs: [[0; MAX_FRAME]; SLOTS],
            rx_len: [0; SLOTS],
            rx_lent: [false; SLOTS],
            rx_head: 0,
            rx_count: 0,
            pending_tx: [(0, 0); SLOTS],
            pending_n: 0,
            stats: NetStats::default(),
        }
    }

    /// Inject a frame as if it arrived from the wire (tests).
    pub fn inject(&mut self, frame: &[u8]) -> bool {
        if self.rx_count >= SLOTS || frame.len() > MAX_FRAME {
            return false;
        }
        let slot = (self.rx_head + self.rx_count) % SLOTS;
        if self.rx_lent[slot] {
            return false;
        }
        self.rx_bufs[slot][..frame.len()].copy_from_slice(frame);
        self.rx_len[slot] = frame.len() as u16;
        self.rx_count += 1;
        true
    }

    /// Frames transmitted but not yet looped back (tests can inspect them).
    pub fn take_sent(&mut self, mut f: impl FnMut(&[u8])) {
        for i in 0..self.pending_n {
            let (id, len) = self.pending_tx[i];
            f(&self.tx_bufs[id as usize][..len as usize]);
            self.tx_busy[id as usize] = false;
        }
        self.pending_n = 0;
    }
}

impl NetDevice for Loopback {
    fn name(&self) -> &'static str {
        "lo"
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
        while n < budget && self.rx_count > 0 {
            let slot = self.rx_head;
            self.rx_lent[slot] = true;
            if !out.push(slot as u16, self.rx_bufs[slot].as_ptr(), self.rx_len[slot]) {
                self.rx_lent[slot] = false;
                break;
            }
            self.rx_head = (self.rx_head + 1) % SLOTS;
            self.rx_count -= 1;
            self.stats.rx_packets += 1;
            self.stats.rx_bytes += self.rx_len[slot] as u64;
            n += 1;
        }
        n
    }
    fn rx_recycle(&mut self, batch: &mut RxBatch) {
        for i in 0..batch.len() {
            self.rx_lent[batch.id(i) as usize] = false;
        }
        batch.clear();
    }
    fn tx_alloc(&mut self) -> Option<TxBuf> {
        let id = self.tx_busy.iter().position(|b| !b)?;
        self.tx_busy[id] = true;
        Some(TxBuf::new(id as u16, self.tx_bufs[id].as_mut_ptr(), MAX_FRAME as u16))
    }
    fn tx_submit(&mut self, buf: TxBuf, len: usize) -> Result<(), TxError> {
        if len > MAX_FRAME || self.pending_n >= SLOTS {
            return Err(TxError);
        }
        let len = len.max(60);
        self.pending_tx[self.pending_n] = (buf.id(), len as u16);
        self.pending_n += 1;
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += len as u64;
        Ok(())
    }
    fn tx_flush(&mut self) {
        // Loop every pending frame back to RX.
        for i in 0..self.pending_n {
            let (id, len) = self.pending_tx[i];
            let frame = self.tx_bufs[id as usize];
            let _ = self.inject(&frame[..len as usize]);
            self.tx_busy[id as usize] = false;
        }
        self.pending_n = 0;
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
