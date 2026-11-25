//! Copy engines (CE) for the ath11k family: DMA rings shared with the
//! firmware, on Qualcomm's SRNG ring hardware. HTC (and WMI on top of it)
//! runs over them. Stage W3b brings up the pipes HTC needs:
//!   CE0 host->target (HTC control), CE1 / CE2 target->host (HTT / WMI and
//!   HTC control), CE3 host->target (WMI).
//!
//! Each pipe has, as needed, a source ring (host fills with buffers to send),
//! a destination ring (host fills with empty buffers) and a status ring (the
//! hardware reports filled destination buffers). Pointers are in 32-bit words.
//! The host publishes its own pointers through the shadow register slots it
//! listed in WLAN_CFG; the hardware writes its pointers into `rdp`, a small
//! array in host memory. Everything is polled (no CE interrupts).

use alloc::vec::Vec;

use super::ath11k_hw as hw;
use super::qmi::SHADOW_V2;
use crate::drivers::grant::{DmaArray, DmaPool};

/// RX buffer size (the host CE table's src_sz_max for CE1/2/5).
pub const RX_BUF: usize = 2048;
/// Entries per ring this host uses (power of two).
const RX_ENTRIES: u32 = 32;
const TX_ENTRIES: u32 = 32;

pub type W32<'a> = &'a mut dyn FnMut(u32, u32);
pub type R32<'a> = &'a mut dyn FnMut(u32) -> u32;

struct Ring {
    mem: DmaArray<u32>,
    entries: u32,
    words: u32,
    entry_words: u32,
    ring_id: u32,
    /// BAR offset of this ring's shadow slot (where the host writes its pointer).
    shadow: u32,
    /// Host-owned pointer (HP for source-type rings, TP for status rings), words.
    ptr: u32,
}

impl Ring {
    fn alloc(pool: &mut DmaPool, entries: u32, entry_words: u32, ring_id: u32, shadow_target: u32) -> Result<Ring, &'static str> {
        let words = entries * entry_words;
        let mem = pool.alloc_array::<u32>(words as usize, 4096).map_err(|_| "out of DMA memory for a CE ring")?;
        let slot = hw::shadow_index(&SHADOW_V2, shadow_target).ok_or("CE ring has no shadow register slot")?;
        Ok(Ring { mem, entries, words, entry_words, ring_id, shadow: hw::shadow_slot(slot), ptr: 0 })
    }
    fn put(&self, at_words: u32, el: &[u32]) {
        for (k, v) in el.iter().enumerate() {
            self.mem.set(at_words as usize + k, *v);
        }
    }
}

pub struct Pipe {
    pub ce: u32,
    src: Option<Ring>,
    dst: Option<Ring>,
    status: Option<Ring>,
    /// RX buffers by destination slot (None while not posted).
    rx: Vec<Option<DmaArray<u8>>>,
    /// Next destination slot the status ring reports on.
    rx_next: u32,
    /// TX buffers by source slot, and the oldest not yet reaped (words).
    tx: Vec<Option<DmaArray<u8>>>,
    tx_reap: u32,
    pub rx_count: u32,
    pub tx_count: u32,
}

pub struct Ce {
    rdp: DmaArray<u32>,
    pub pipes: Vec<Pipe>,
}

fn fence() {
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}

impl Ce {
    /// Allocate rings and buffers (host memory only, no register writes).
    /// `pipes`: (CE number, has source ring, has destination ring).
    pub fn alloc(pool: &mut DmaPool, pipes: &[(u32, bool, bool)]) -> Result<Ce, &'static str> {
        let rdp = pool.alloc_array::<u32>(hw::RING_IDS, 4096).map_err(|_| "out of DMA memory for CE pointers")?;
        let mut out = Vec::new();
        for &(ce, has_src, has_dst) in pipes {
            let src = if has_src {
                Some(Ring::alloc(pool, TX_ENTRIES, hw::SRC_DESC_WORDS, hw::RING_ID_CE_SRC + ce, hw::CE_SRC_BASE + hw::CE_R2 + ce * hw::CE_STRIDE)?)
            } else {
                None
            };
            let (dst, status, rx) = if has_dst {
                let d = Ring::alloc(pool, RX_ENTRIES, hw::DST_DESC_WORDS, hw::RING_ID_CE_DST + ce, hw::CE_DST_BASE + hw::CE_R2 + ce * hw::CE_STRIDE)?;
                // The status ring's shadow mirrors its TP (R2 + 4).
                let st = Ring::alloc(
                    pool,
                    RX_ENTRIES,
                    hw::STATUS_DESC_WORDS,
                    hw::RING_ID_CE_STATUS + ce,
                    hw::CE_DST_BASE + hw::CE_STATUS_R2 + ce * hw::CE_STRIDE + 4,
                )?;
                let mut rx = Vec::new();
                for _ in 0..RX_ENTRIES {
                    rx.push(None);
                }
                (Some(d), Some(st), rx)
            } else {
                (None, None, Vec::new())
            };
            let mut tx = Vec::new();
            for _ in 0..if has_src { TX_ENTRIES } else { 0 } {
                tx.push(None);
            }
            out.push(Pipe { ce, src, dst, status, rx, rx_next: 0, tx, tx_reap: 0, rx_count: 0, tx_count: 0 });
        }
        // RX buffers: entries - 2 per destination ring, as ath11k posts.
        for p in out.iter_mut() {
            if p.dst.is_some() {
                for i in 0..(RX_ENTRIES - 2) as usize {
                    p.rx[i] = Some(pool.alloc_array::<u8>(RX_BUF, 4096).map_err(|_| "out of DMA memory for CE buffers")?);
                }
            }
        }
        Ok(Ce { rdp, pipes: out })
    }

    fn rdp_addr(&self, ring_id: u32) -> u64 {
        self.rdp.dev().get() + ring_id as u64 * 4
    }

    /// Program every ring (as ath11k's hal srng setup) and post the RX buffers.
    pub fn start(&mut self, w: W32, r: R32) {
        for p in self.pipes.iter_mut() {
            let (s0, d0, st0) = hw::ce_blocks(p.ce);
            if let Some(s) = &p.src {
                program_src(w, s, s0, self.rdp.dev().get());
                self.rdp.set(s.ring_id as usize, 0);
            }
            if let Some(d) = &p.dst {
                program_src(w, d, d0, self.rdp.dev().get());
                self.rdp.set(d.ring_id as usize, 0);
                // Largest buffer the destination ring may fill.
                let ctrl = r(d0 + hw::CE_DST_RING_CTRL);
                w(d0 + hw::CE_DST_RING_CTRL, (ctrl & !0xFFFF) | RX_BUF as u32);
            }
            if let Some(st) = &p.status {
                program_dst(w, st, st0, self.rdp.dev().get());
                self.rdp.set(st.ring_id as usize, 0);
            }
        }
        fence();
        // Post RX buffers.
        for i in 0..self.pipes.len() {
            let n = self.pipes[i].rx.iter().filter(|b| b.is_some()).count() as u32;
            let p = &mut self.pipes[i];
            if let Some(d) = &mut p.dst {
                for slot in 0..n {
                    if let Some(b) = &p.rx[slot as usize] {
                        d.put(slot * hw::DST_DESC_WORDS, &hw::ce_dst_desc(b.dev().get()));
                    }
                }
                d.ptr = (n * hw::DST_DESC_WORDS) % d.words;
                fence();
                w(d.shadow, d.ptr);
            }
        }
    }

    /// Received messages on every pipe, buffers re-posted.
    pub fn poll_rx(&mut self, w: W32, f: &mut dyn FnMut(u32, &[u8])) -> u32 {
        let mut got = 0;
        for p in self.pipes.iter_mut() {
            let (Some(st), Some(d)) = (&mut p.status, &mut p.dst) else { continue };
            fence();
            let hw_hp = self.rdp.get(st.ring_id as usize) % st.words;
            let mut seen = 0;
            while st.ptr != hw_hp && seen < st.entries {
                seen += 1;
                let w0 = st.mem.get(st.ptr as usize);
                let len = (hw::ce_status_len(w0) as usize).min(RX_BUF);
                st.ptr = (st.ptr + st.entry_words) % st.words;
                let slot = p.rx_next as usize;
                p.rx_next = (p.rx_next + 1) % d.entries;
                let Some(buf) = p.rx[slot].take() else { continue };
                if len > 0 {
                    let mut v = alloc::vec![0u8; len];
                    unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), v.as_mut_ptr(), len) };
                    f(p.ce, &v);
                    p.rx_count += 1;
                    got += 1;
                }
                // Re-post the buffer at the destination ring's next free slot.
                let at = d.ptr / hw::DST_DESC_WORDS;
                d.put(d.ptr, &hw::ce_dst_desc(buf.dev().get()));
                p.rx[at as usize] = Some(buf);
                d.ptr = (d.ptr + hw::DST_DESC_WORDS) % d.words;
            }
            if seen > 0 {
                fence();
                w(st.shadow, st.ptr);
                w(d.shadow, d.ptr);
            }
        }
        got
    }

    /// Send `data` on pipe `ce` (meta = HTC endpoint). Buffers come from `pool`.
    pub fn send(&mut self, pool: &mut DmaPool, w: W32, ce: u32, data: &[u8], meta: u32) -> Result<(), &'static str> {
        let rdp_tp = |rdp: &DmaArray<u32>, id: u32| rdp.get(id as usize);
        let Some(p) = self.pipes.iter_mut().find(|p| p.ce == ce) else { return Err("CE pipe not set up") };
        let Some(s) = &mut p.src else { return Err("CE pipe has no source ring") };
        if data.len() > RX_BUF {
            return Err("CE send larger than 2 KiB");
        }
        // Reap what the hardware consumed.
        fence();
        let tp = rdp_tp(&self.rdp, s.ring_id) % s.words;
        while p.tx_reap != tp {
            let slot = (p.tx_reap / s.entry_words) as usize;
            p.tx[slot] = None; // memory stays in the pool (no free while the device may read)
            p.tx_reap = (p.tx_reap + s.entry_words) % s.words;
        }
        let next = (s.ptr + s.entry_words) % s.words;
        if next == p.tx_reap {
            return Err("CE source ring full");
        }
        let buf = pool.alloc_array::<u8>(RX_BUF, 4096).map_err(|_| "out of DMA memory for a CE send")?;
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf.as_ptr(), data.len()) };
        s.put(s.ptr, &hw::ce_src_desc(buf.dev().get(), data.len() as u32, meta));
        p.tx[(s.ptr / s.entry_words) as usize] = Some(buf);
        s.ptr = next;
        fence();
        w(s.shadow, s.ptr);
        p.tx_count += 1;
        Ok(())
    }

    /// (pipe, rx count, tx count, hw pointer of status/source ring) for logs.
    pub fn stats(&self) -> Vec<(u32, u32, u32, u32, u32)> {
        self.pipes
            .iter()
            .map(|p| {
                let hwp = p.status.as_ref().or(p.src.as_ref()).map_or(0, |r| self.rdp.get(r.ring_id as usize));
                let hostp = p.status.as_ref().or(p.src.as_ref()).map_or(0, |r| r.ptr);
                (p.ce, p.rx_count, p.tx_count, hwp, hostp)
            })
            .collect()
    }

    pub fn rdp_base(&self) -> u64 {
        self.rdp_addr(0)
    }
}

/// Source-type ring registers (TCL1 layout): base, size, entry size, interrupt
/// thresholds (unused: polled), where the hardware writes its TP, pointers
/// zeroed, then enable.
fn program_src(w: W32, r: &Ring, r0: u32, rdp: u64) {
    let base = r.mem.dev().get();
    let tp_addr = rdp + r.ring_id as u64 * 4;
    w(r0 + hw::SRC_BASE_LSB, base as u32);
    w(r0 + hw::SRC_BASE_MSB, hw::ring_base_msb(base, r.words));
    w(r0 + hw::SRC_ID, r.entry_words);
    w(r0 + hw::SRC_IX0, r.entry_words); // batch threshold 1 entry, no timer
    w(r0 + hw::SRC_IX1, 0);
    w(r0 + hw::SRC_TP_ADDR_LSB, tp_addr as u32);
    w(r0 + hw::SRC_TP_ADDR_MSB, (tp_addr >> 32) as u32);
    // Both CE source and destination blocks keep HP at +0x400, TP at +0x404.
    let r2 = r0 + hw::CE_R2;
    w(r2, 0);
    w(r2 + 4, 0);
    w(r0 + hw::SRC_MISC, hw::MISC_LOOPCNT_DISABLE | hw::MISC_SRNG_ENABLE);
}

/// Destination-type ring registers (REO1 layout) for the status ring: base,
/// size, ring id + entry size, producer interrupt setup, where the hardware
/// writes its HP, pointers zeroed, then enable.
fn program_dst(w: W32, r: &Ring, r0: u32, rdp: u64) {
    let base = r.mem.dev().get();
    let hp_addr = rdp + r.ring_id as u64 * 4;
    w(r0 + hw::DST_BASE_LSB, base as u32);
    w(r0 + hw::DST_BASE_MSB, hw::ring_base_msb(base, r.words));
    w(r0 + hw::DST_ID, (r.ring_id << 8) | r.entry_words);
    w(r0 + hw::DST_PRODUCER_INT, ((0x1000 >> 3) << 16) | r.entry_words);
    w(r0 + hw::DST_HP_ADDR_LSB, hp_addr as u32);
    w(r0 + hw::DST_HP_ADDR_MSB, (hp_addr >> 32) as u32);
    // Status ring R2 sits at the destination block + 0x408.
    let r2 = r0 - hw::CE_STATUS_R0 + hw::CE_STATUS_R2;
    w(r2, 0);
    w(r2 + 4, 0);
    w(r0 + hw::DST_MISC, hw::MISC_SRNG_ENABLE);
}
