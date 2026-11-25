//! MHI host: boots a Qualcomm MHI device from its boot ROM to main firmware.
//! Generic (config-driven) so other Qualcomm chips can reuse it; the register
//! map and wire formats are in `bus::mhi` (pure, host-tested).
//!
//! Sequence (full boot chain, as the Linux MHI core does it):
//!  1. Host memory: channel / event / command contexts and event + command rings.
//!  2. BHI: the first `sbl_size` bytes of the image go to the boot ROM as one
//!     buffer; it starts the secondary boot loader (SBL).
//!  3. READY: wait for MHICTRL reset clear and MHISTATUS READY, program the
//!     context base addresses and MHICFG, hand every event ring to the device,
//!     set MHI state M0.
//!  4. Control event ring: STATE_CHANGE(M0), then EE_EVENT(SBL) -> BHIe: the
//!     whole image as 512 KiB segments described by a vector table.
//!  5. EE_EVENT(AMSS): main firmware runs. Done.
//!
//! Bulletproofing: every wait is bounded; every pointer the device gives back
//! is range-checked; all-ones register reads stop the sequence ("link down");
//! SYS_ERR and RDDM (firmware crash) stop it with a register dump. On any
//! failure the DMA pool is poisoned (leaked, never freed) because the device
//! may still write into it. Interrupts are counted but never relied on: the
//! host polls registers and the event rings.

use alloc::vec::Vec;

use super::mhi::{self, Event};
use crate::drivers::delay::{delay_ms, poll_ms};
use crate::drivers::grant::{DmaArray, DmaPool, GrantId, MmioRegion};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Info,
    Ok,
    Warn,
    Err,
}

pub type Say<'a> = &'a mut dyn FnMut(Level, core::fmt::Arguments);

#[derive(Clone, Copy, Debug)]
pub struct EvCfg {
    pub elements: u16,
    /// MSI message index the device signals this ring on.
    pub msivec: u32,
    pub intmod_ms: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct ChCfg {
    pub id: u16,
    pub chtype: u32,
    pub erindex: u32,
}

pub struct Config {
    pub max_channels: usize,
    pub events: &'static [EvCfg],
    pub channels: &'static [ChCfg],
    pub cmd_elements: u16,
    /// Per-step wait limit.
    pub timeout_ms: u32,
    /// Bytes of the image sent through BHI (the SBL).
    pub sbl_size: usize,
    /// BHIe segment size.
    pub seg_len: usize,
    /// Highest device address the controller may use (MHICTRLLIMIT / DATALIMIT).
    pub iova_stop: u64,
}

struct EvRing {
    ring: DmaArray<[u32; 4]>,
    n: u64,
    /// Next element the host reads (device address).
    rd: u64,
    /// Host write pointer handed to the device (device address).
    wp: u64,
}

/// A booted (or booting) device's host-side state. Keep it alive while the
/// device runs: it owns the memory the device reads and writes.
pub struct Host {
    pool: DmaPool,
    chan_ctx: DmaArray<u32>,
    ev_ctx: DmaArray<u32>,
    cmd_ctx: DmaArray<u32>,
    ev: Vec<EvRing>,
    _cmd_ring: DmaArray<[u32; 4]>,
    _bhi_buf: Option<DmaArray<u8>>,
    _segments: Vec<DmaArray<u8>>,
    _table: Option<DmaArray<u8>>,
    bhi: usize,
    bhie: usize,
    erdb: usize,
    chdb: usize,
    cmd_n: u64,
    /// Command ring write pointer (device address).
    cmd_wp: u64,
    /// Last command completion seen: (command TRE address, completion code).
    cmd_done: Option<(u64, u32)>,
    chans: Vec<Chan>,
    pub events_seen: u32,
    pub m0: bool,
    pub ee: u32,
}

/// A started transfer channel (stage W2: IPCR 20/21).
struct Chan {
    id: u16,
    ring: DmaArray<[u32; 4]>,
    n: u64,
    /// Host write pointer (next free element, device address).
    wp: u64,
    /// Oldest element not yet completed (device address).
    rp: u64,
    /// Per element: (tag, posted length).
    slots: Vec<(u32, u32)>,
}

/// A finished transfer: the caller's tag, bytes moved, completion code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Done {
    pub chan: u16,
    pub tag: u32,
    pub len: u32,
    pub code: u32,
}

/// TX event completion codes (MHI spec / Linux enum mhi_ev_ccs).
pub const CC_SUCCESS: u32 = 0x1;
pub const CC_EOT: u32 = 0x2;
pub const CC_OVERFLOW: u32 = 0x3;
pub const CC_EOB: u32 = 0x4;

/// Why a boot stopped, plus the host state that must not be freed.
pub struct Failure {
    pub why: &'static str,
    pub host: Option<Host>,
}

fn dead(v: u32) -> bool {
    v == 0xFFFF_FFFF
}

fn tsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

impl Host {
    /// Memory for a driver's own device-visible buffers (RX/TX buffers, the
    /// firmware's requested memory, M3): it lives exactly as long as the host.
    pub fn pool(&mut self) -> &mut DmaPool {
        &mut self.pool
    }

    /// Keep the device in M0: assert the MHI device-wake doorbell
    /// (channel doorbell 127), as the Linux MHI core does in mission mode.
    pub fn assert_wake(&self, bar: &MmioRegion) {
        let db = self.chdb + 8 * 127;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        bar.w32(db + 4, 0);
        bar.w32(db, 1);
    }

    fn ring_db(bar: &MmioRegion, db: usize, v: u64) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        bar.w32(db + 4, (v >> 32) as u32);
        bar.w32(db, v as u32);
    }

    /// Put one command on the command ring and ring CRDB. Returns the command
    /// element's device address (matched against the completion's pointer).
    fn send_cmd(&mut self, bar: &MmioRegion, el: [u32; 4]) -> Result<u64, &'static str> {
        let base = self._cmd_ring.dev().get();
        let at = self.cmd_wp;
        let idx = mhi::ring_index(base, self.cmd_n, at).ok_or("command pointer outside its ring")?;
        let next = mhi::ring_next(base, self.cmd_n, at).ok_or("command pointer outside its ring")?;
        self._cmd_ring.set(idx, el);
        self.cmd_wp = next;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.cmd_ctx.set(mhi::CTX_WP, next as u32);
        self.cmd_ctx.set(mhi::CTX_WP + 1, (next >> 32) as u32);
        Self::ring_db(bar, mhi::CRDB_LOWER, next);
        Ok(at)
    }

    /// Start channel `id` with a ring of `elements` (power of two, <= 256):
    /// fill its context, send START_CHAN, wait for the completion on the
    /// control event ring. `inbound` = device-to-host.
    pub fn start_channel(&mut self, bar: &MmioRegion, cfg: &Config, id: u16, elements: u16, say: Say) -> Result<(), &'static str> {
        if (id as usize) >= cfg.max_channels || id >= 127 || !elements.is_power_of_two() || elements > 256 {
            return Err("bad channel parameters");
        }
        if self.chans.iter().any(|c| c.id == id) {
            return Err("channel already started");
        }
        let n = elements as usize;
        let ring = self
            .pool
            .alloc_array::<[u32; 4]>(n, (n * mhi::EL_BYTES).min(4096))
            .map_err(|_| "out of DMA memory for a channel ring")?;
        let base = ring.dev().get();
        let b = id as usize * mhi::CTX_WORDS;
        let chcfg = self.chan_ctx.get(b);
        let len = (n * mhi::EL_BYTES) as u64;
        self.chan_ctx.set(b, (chcfg & !0xFF) | mhi::CH_STATE_ENABLED);
        for (k, v) in [(3usize, base), (5, len), (7, base), (9, base)] {
            self.chan_ctx.set(b + k, v as u32);
            self.chan_ctx.set(b + k + 1, (v >> 32) as u32);
        }
        self.chans.push(Chan { id, ring, n: n as u64, wp: base, rp: base, slots: alloc::vec![(0, 0); n] });
        self.cmd_done = None;
        let at = self.send_cmd(bar, mhi::cmd_el(mhi::PKT_START_CHAN, id as u32))?;
        let mut done = None;
        let took = poll_ms(cfg.timeout_ms, || {
            let _ = self.service(bar, &mut |_| {});
            done = self.cmd_done;
            done.is_some()
        });
        match (took, done) {
            (Some(ms), Some((ptr, code))) if ptr == at && code == CC_SUCCESS => {
                say(Level::Ok, format_args!("MHI: channel {} started ({} elements at {:#x}) in {} ms", id, n, base, ms));
                Ok(())
            }
            (_, Some((ptr, code))) => {
                say(Level::Err, format_args!("MHI: START channel {}: completion code {:#x} for {:#x} (sent {:#x})", id, code, ptr, at));
                Err("channel start refused")
            }
            _ => {
                say(Level::Err, format_args!("MHI: START channel {}: no completion in {} ms", id, cfg.timeout_ms));
                Err("channel start timed out")
            }
        }
    }

    /// Queue a buffer on channel `id`. `tag` comes back in its `Done`.
    pub fn queue(&mut self, bar: &MmioRegion, id: u16, addr: u64, len: u32, tag: u32) -> Result<(), &'static str> {
        if len == 0 || len > 0xFFFF {
            return Err("transfer length out of range");
        }
        let Some(c) = self.chans.iter_mut().find(|c| c.id == id) else { return Err("channel not started") };
        let base = c.ring.dev().get();
        let next = mhi::ring_next(base, c.n, c.wp).ok_or("channel pointer outside its ring")?;
        if next == c.rp {
            return Err("channel ring full");
        }
        let idx = mhi::ring_index(base, c.n, c.wp).ok_or("channel pointer outside its ring")?;
        c.ring.set(idx, mhi::transfer_el(addr, len, true));
        c.slots[idx] = (tag, len);
        c.wp = next;
        let b = id as usize * mhi::CTX_WORDS + mhi::CTX_WP;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.chan_ctx.set(b, next as u32);
        self.chan_ctx.set(b + 1, (next >> 32) as u32);
        Self::ring_db(bar, self.chdb + 8 * id as usize, next);
        Ok(())
    }

    /// Process every event ring once: command completions are remembered,
    /// transfer completions go to `f`, state changes are tracked. Errors on a
    /// device pointer outside its ring or SYS_ERR.
    pub fn service(&mut self, bar: &MmioRegion, f: &mut dyn FnMut(Done)) -> Result<u32, &'static str> {
        let mut total = 0;
        for i in 0..self.ev.len() {
            let mut evs: Vec<Event> = Vec::new();
            total += self.drain(bar, i, &mut |e| evs.push(e))?;
            for e in evs {
                match e {
                    Event::CmdCompletion { code, ptr } => self.cmd_done = Some((ptr, code)),
                    Event::StateChange(s) => {
                        if s == mhi::STATE_M0 {
                            self.m0 = true;
                        } else if s == mhi::STATE_SYS_ERR {
                            return Err("device entered SYS_ERR");
                        }
                    }
                    Event::ExecEnv(x) => {
                        self.ee = x;
                        if x == mhi::EE_RDDM {
                            return Err("firmware crashed (RDDM)");
                        }
                    }
                    Event::Tx { chan, code, len, ptr } => {
                        let Some(c) = self.chans.iter_mut().find(|c| c.id as u32 == chan) else { continue };
                        let base = c.ring.dev().get();
                        let Some(_) = mhi::ring_index(base, c.n, ptr) else {
                            return Err("transfer event points outside its ring");
                        };
                        // Complete every element from rp through the one the event names.
                        for _ in 0..c.n {
                            let idx = mhi::ring_index(base, c.n, c.rp).ok_or("channel rp outside its ring")?;
                            let last = c.rp == ptr;
                            let (tag, posted) = c.slots[idx];
                            f(Done { chan: c.id, tag, len: if last { len.min(posted) } else { posted }, code });
                            c.rp = mhi::ring_next(base, c.n, c.rp).ok_or("channel rp outside its ring")?;
                            if last {
                                break;
                            }
                        }
                    }
                    Event::Other { .. } => {}
                }
            }
        }
        Ok(total)
    }

    /// Give up on this host without freeing anything the device may still
    /// write (the pool is poisoned: dropped memory is leaked).
    pub fn abandon(mut self) {
        self.pool.poison();
    }

    fn fail(mut self, why: &'static str) -> Failure {
        self.pool.poison();
        Failure { why, host: Some(self) }
    }

    fn ctx_set(arr: &DmaArray<u32>, idx: usize, w: &[u32; mhi::CTX_WORDS]) {
        for (k, v) in w.iter().enumerate() {
            arr.set(idx * mhi::CTX_WORDS + k, *v);
        }
    }

    fn ev_ctx_rp(&self, i: usize) -> u64 {
        let b = i * mhi::CTX_WORDS + mhi::CTX_RP;
        self.ev_ctx.get(b) as u64 | ((self.ev_ctx.get(b + 1) as u64) << 32)
    }

    fn ev_set_wp(&mut self, i: usize, wp: u64) {
        let b = i * mhi::CTX_WORDS + mhi::CTX_WP;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.ev_ctx.set(b, wp as u32);
        self.ev_ctx.set(b + 1, (wp >> 32) as u32);
        self.ev[i].wp = wp;
    }

    fn ring_ev_db(&self, bar: &MmioRegion, i: usize) {
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let db = self.erdb + 8 * i;
        let wp = self.ev[i].wp;
        bar.w32(db + 4, (wp >> 32) as u32);
        bar.w32(db, wp as u32);
    }

    /// Registers worth seeing when something goes wrong.
    pub fn dump(&self, bar: &MmioRegion, say: Say) {
        let st = bar.r32(mhi::MHISTATUS);
        say(
            Level::Info,
            format_args!(
                "regs: MHISTATUS {:#x} ({}) MHICTRL {:#x} EE {} BHI status {:#x} err {:#x} dbg {:#x}/{:#x}/{:#x} BHIe status {:#x}",
                st,
                mhi::state_name(mhi::state_of(st)),
                bar.r32(mhi::MHICTRL),
                bar.r32(self.bhi + mhi::BHI_EXECENV),
                bar.r32(self.bhi + mhi::BHI_STATUS),
                bar.r32(self.bhi + mhi::BHI_ERRCODE),
                bar.r32(self.bhi + mhi::BHI_ERRDBG1),
                bar.r32(self.bhi + mhi::BHI_ERRDBG2),
                bar.r32(self.bhi + mhi::BHI_ERRDBG3),
                bar.r32(self.bhie + mhi::BHIE_TXVECSTATUS),
            ),
        );
        for i in 0..self.ev.len() {
            let r = &self.ev[i];
            say(
                Level::Info,
                format_args!(
                    "event ring {}: device rp {:#x}, host rd {:#x}, wp {:#x}, base {:#x}, {} elements",
                    i,
                    self.ev_ctx_rp(i),
                    r.rd,
                    r.wp,
                    r.ring.dev().get(),
                    r.n
                ),
            );
        }
    }

    /// Drain event ring `i`, handing each event to `f`. Returns events read, or
    /// an error if the device's pointer is outside the ring.
    fn drain(&mut self, bar: &MmioRegion, i: usize, f: &mut dyn FnMut(Event)) -> Result<u32, &'static str> {
        let base = self.ev[i].ring.dev().get();
        let n = self.ev[i].n;
        let dev_rp = self.ev_ctx_rp(i);
        if mhi::ring_index(base, n, dev_rp).is_none() {
            return Err("device event pointer outside its ring");
        }
        let mut count = 0u32;
        while self.ev[i].rd != dev_rp && (count as u64) < n {
            let idx = mhi::ring_index(base, n, self.ev[i].rd).ok_or("host event pointer outside its ring")?;
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
            let el = self.ev[i].ring.get(idx);
            f(mhi::decode_event(el));
            count += 1;
            // Recycle: give the element back to the device, move on.
            let wp = mhi::ring_next(base, n, self.ev[i].wp).ok_or("event write pointer outside its ring")?;
            self.ev_set_wp(i, wp);
            self.ev[i].rd = mhi::ring_next(base, n, self.ev[i].rd).ok_or("event read pointer outside its ring")?;
        }
        if count > 0 {
            self.ring_ev_db(bar, i);
        }
        self.events_seen += count;
        Ok(count)
    }

    fn send_bhie(&self, bar: &MmioRegion, cfg: &Config, segments: usize, say: Say) -> Result<(), &'static str> {
        let Some(t) = &self._table else { return Err("no BHIe table") };
        let addr = t.dev().get();
        let size = (segments * mhi::VEC_ENTRY_BYTES) as u32;
        bar.w32(self.bhie + mhi::BHIE_TXVECADDR_HIGH, (addr >> 32) as u32);
        bar.w32(self.bhie + mhi::BHIE_TXVECADDR_LOW, addr as u32);
        bar.w32(self.bhie + mhi::BHIE_TXVECSIZE, size);
        let seq = mhi::seq_number(tsc());
        let db = mhi::txvecdb_with_seq(bar.r32(self.bhie + mhi::BHIE_TXVECDB), seq);
        bar.w32(self.bhie + mhi::BHIE_TXVECDB, db);
        say(
            Level::Info,
            format_args!("BHIe: vector table {:#x} ({} segments, {} bytes), sequence {:#x}; waiting", addr, segments, size, seq),
        );
        let mut last = 0u32;
        let took = poll_ms(cfg.timeout_ms, || {
            last = bar.r32(self.bhie + mhi::BHIE_TXVECSTATUS);
            dead(last) || mhi::xfer_status(last) != mhi::XFER_RESET
        });
        match took {
            _ if dead(last) => Err("link down during BHIe"),
            Some(ms) if mhi::xfer_status(last) == mhi::XFER_DONE => {
                say(Level::Ok, format_args!("BHIe: full image transferred in {} ms (status {:#x})", ms, last));
                Ok(())
            }
            Some(_) => {
                say(Level::Err, format_args!("BHIe: transfer error (status {:#x})", last));
                Err("BHIe transfer error")
            }
            None => {
                say(Level::Err, format_args!("BHIe: no completion after {} ms (status {:#x})", cfg.timeout_ms, last));
                Err("BHIe timed out")
            }
        }
    }
}

/// Boot the device from PBL to AMSS with `image` (the whole firmware file).
/// `irq_count` reports interrupts seen so far (for the log only).
pub fn boot(
    cfg: &Config,
    bar: &MmioRegion,
    grant: GrantId,
    image: &[u8],
    irq_count: &dyn Fn() -> u64,
    say: Say,
) -> Result<Host, Failure> {
    let nofree = |why| Failure { why, host: None };
    if image.is_empty() {
        return Err(nofree("empty firmware image"));
    }
    if cfg.events.is_empty() || cfg.events.len() > 8 {
        return Err(nofree("bad event ring config"));
    }

    // ---- where the device stands ----
    let bhi = bar.r32(mhi::BHIOFF);
    let bhie = bar.r32(mhi::BHIEOFF);
    if dead(bhi) || dead(bhie) {
        return Err(nofree("MHI registers read all-ones (link down)"));
    }
    let (bhi, bhie) = (bhi as usize, bhie as usize);
    if bhi % 4 != 0 || bhi + 0x48 > bar.len() || bhie % 4 != 0 || bhie + 0x48 > bar.len() {
        return Err(nofree("BHI/BHIe offsets outside BAR0"));
    }
    let ee = bar.r32(bhi + mhi::BHI_EXECENV);
    let st = mhi::state_of(bar.r32(mhi::MHISTATUS));
    say(
        Level::Info,
        format_args!(
            "MHI: BHI at {:#x}, BHIe at {:#x}, EE {} ({}), state {}, serial {:#x}",
            bhi,
            bhie,
            ee,
            mhi::ee_name(ee),
            mhi::state_name(st),
            bar.r32(bhi + mhi::BHI_SERIALNU)
        ),
    );
    if ee != mhi::EE_PBL {
        return Err(nofree("device is not in its boot ROM (run the W0 reset first)"));
    }
    if st == mhi::STATE_SYS_ERR {
        bar.w32(mhi::MHICTRL, bar.r32(mhi::MHICTRL) | mhi::MHICTRL_RESET);
        if poll_ms(cfg.timeout_ms, || bar.r32(mhi::MHICTRL) & mhi::MHICTRL_RESET == 0).is_none() {
            return Err(nofree("MHI reset from SYS_ERR did not clear"));
        }
    }
    bar.w32(bhi + mhi::BHI_INTVEC, 0);

    // ---- host memory ----
    // Room for the firmware's own memory request in W2 (several MiB) too.
    let mut pool = DmaPool::new(grant, 32768);
    let ctx_err = |e: crate::drivers::grant::GrantError| -> &'static str {
        let _ = e;
        "out of DMA memory for MHI contexts/rings"
    };
    let chan_ctx = pool.alloc_array::<u32>(cfg.max_channels * mhi::CTX_WORDS, 4096).map_err(|e| nofree(ctx_err(e)))?;
    let ev_ctx = pool.alloc_array::<u32>(cfg.events.len() * mhi::CTX_WORDS, 64).map_err(|e| nofree(ctx_err(e)))?;
    let cmd_ctx = pool.alloc_array::<u32>(mhi::CTX_WORDS, 64).map_err(|e| nofree(ctx_err(e)))?;
    let cmd_n = cfg.cmd_elements as usize;
    let cmd_ring = pool.alloc_array::<[u32; 4]>(cmd_n, (cmd_n * mhi::EL_BYTES).min(4096)).map_err(|e| nofree(ctx_err(e)))?;
    let mut ev = Vec::new();
    for e in cfg.events {
        let n = e.elements as usize;
        let ring = pool.alloc_array::<[u32; 4]>(n, (n * mhi::EL_BYTES).min(4096)).map_err(|e| nofree(ctx_err(e)))?;
        let base = ring.dev().get();
        ev.push(EvRing { ring, n: n as u64, rd: base, wp: base });
    }
    for c in cfg.channels {
        if (c.id as usize) < cfg.max_channels {
            Host::ctx_set(&chan_ctx, c.id as usize, &mhi::chan_ctx(c.chtype, c.erindex, mhi::DB_BRST_DISABLE));
        }
    }
    for (i, e) in cfg.events.iter().enumerate() {
        let base = ev[i].ring.dev().get();
        Host::ctx_set(&ev_ctx, i, &mhi::event_ctx(e.intmod_ms, e.msivec, base, (e.elements as usize * mhi::EL_BYTES) as u64));
    }
    Host::ctx_set(&cmd_ctx, 0, &mhi::cmd_ctx(cmd_ring.dev().get(), (cmd_n * mhi::EL_BYTES) as u64));
    let mut host = Host {
        pool,
        chan_ctx,
        ev_ctx,
        cmd_ctx,
        ev,
        _cmd_ring: cmd_ring,
        _bhi_buf: None,
        _segments: Vec::new(),
        _table: None,
        bhi,
        bhie,
        erdb: 0,
        chdb: 0,
        cmd_n: cmd_n as u64,
        cmd_wp: 0,
        cmd_done: None,
        chans: Vec::new(),
        events_seen: 0,
        m0: false,
        ee,
    };
    host.cmd_wp = host._cmd_ring.dev().get();
    let hi = [host.chan_ctx.dev().get(), host.ev_ctx.dev().get(), host.cmd_ctx.dev().get()];
    if hi.iter().any(|a| *a > cfg.iova_stop) {
        return Err(host.fail("DMA memory above the device's address limit"));
    }
    say(
        Level::Info,
        format_args!(
            "MHI: contexts at {:#x} (channels) {:#x} (events) {:#x} (command); {} event ring(s)",
            hi[0],
            hi[1],
            hi[2],
            host.ev.len()
        ),
    );

    // ---- BHI: secondary boot loader ----
    let sbl = cfg.sbl_size.min(image.len());
    let buf = match host.pool.alloc_large(sbl) {
        Ok(b) => b,
        Err(_) => return Err(host.fail("out of DMA memory for the SBL image")),
    };
    unsafe { core::ptr::copy_nonoverlapping(image.as_ptr(), buf.as_ptr(), sbl) };
    let addr = buf.dev().get();
    host._bhi_buf = Some(buf);
    let seq = mhi::seq_number(tsc());
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    bar.w32(bhi + mhi::BHI_STATUS, 0);
    bar.w32(bhi + mhi::BHI_IMGADDR_HIGH, (addr >> 32) as u32);
    bar.w32(bhi + mhi::BHI_IMGADDR_LOW, addr as u32);
    bar.w32(bhi + mhi::BHI_IMGSIZE, sbl as u32);
    bar.w32(bhi + mhi::BHI_IMGTXDB, seq);
    say(Level::Info, format_args!("BHI: SBL {} bytes at {:#x}, sequence {:#x}; waiting for the boot ROM", sbl, addr, seq));
    let mut last = 0u32;
    let took = poll_ms(cfg.timeout_ms, || {
        last = bar.r32(bhi + mhi::BHI_STATUS);
        dead(last) || mhi::xfer_status(last) != mhi::XFER_RESET
    });
    if dead(last) {
        return Err(host.fail("link down during BHI"));
    }
    match took {
        Some(ms) if mhi::xfer_status(last) != mhi::XFER_ERROR => say(
            Level::Ok,
            format_args!(
                "BHI: SBL accepted in {} ms (status {:#x}, EE now {}, {} interrupt(s) so far)",
                ms,
                last,
                bar.r32(bhi + mhi::BHI_EXECENV),
                irq_count()
            ),
        ),
        Some(_) => {
            say(Level::Err, format_args!("BHI: boot ROM reported an error (status {:#x})", last));
            host.dump(bar, say);
            return Err(host.fail("BHI transfer error"));
        }
        None => {
            say(Level::Err, format_args!("BHI: no answer after {} ms (status {:#x})", cfg.timeout_ms, last));
            host.dump(bar, say);
            return Err(host.fail("BHI timed out"));
        }
    }

    // ---- BHIe table for the full image (sent once SBL asks for it) ----
    let (segments, _last_len) = mhi::bhie_segments(image.len(), cfg.seg_len);
    let table = match host.pool.alloc_array::<u8>(segments * mhi::VEC_ENTRY_BYTES, 64) {
        Ok(t) => t,
        Err(_) => return Err(host.fail("out of DMA memory for the BHIe table")),
    };
    for s in 0..segments {
        let off = s * cfg.seg_len;
        let len = (image.len() - off).min(cfg.seg_len);
        let seg = match host.pool.alloc_large(len) {
            Ok(b) => b,
            Err(_) => return Err(host.fail("out of DMA memory for the firmware image")),
        };
        unsafe { core::ptr::copy_nonoverlapping(image.as_ptr().add(off), seg.as_ptr(), len) };
        let e = mhi::vec_entry(seg.dev().get(), len as u64);
        for (k, b) in e.iter().enumerate() {
            table.set(s * mhi::VEC_ENTRY_BYTES + k, *b);
        }
        if seg.dev().get() + len as u64 > cfg.iova_stop {
            return Err(host.fail("DMA memory above the device's address limit"));
        }
        host._segments.push(seg);
    }
    host._table = Some(table);
    say(Level::Info, format_args!("BHIe: {} bytes staged in {} segment(s) of up to {} KiB", image.len(), segments, cfg.seg_len / 1024));

    // ---- READY, program MMIO, M0 ----
    if poll_ms(cfg.timeout_ms, || bar.r32(mhi::MHICTRL) & mhi::MHICTRL_RESET == 0).is_none() {
        host.dump(bar, say);
        return Err(host.fail("MHICTRL reset never cleared"));
    }
    let mut status = 0u32;
    let took = poll_ms(cfg.timeout_ms, || {
        status = bar.r32(mhi::MHISTATUS);
        dead(status) || status & mhi::MHISTATUS_READY != 0 || status & mhi::MHISTATUS_SYSERR != 0
    });
    if dead(status) {
        return Err(host.fail("link down waiting for READY"));
    }
    if status & mhi::MHISTATUS_SYSERR != 0 {
        say(Level::Err, format_args!("MHI: SYS_ERR while waiting for READY (status {:#x})", status));
        host.dump(bar, say);
        return Err(host.fail("device reported SYS_ERR"));
    }
    match took {
        Some(ms) => say(Level::Ok, format_args!("MHI: READY after {} ms (status {:#x})", ms, status)),
        None => {
            host.dump(bar, say);
            return Err(host.fail("MHI never became READY"));
        }
    }
    let chdb = bar.r32(mhi::CHDBOFF) as usize;
    let erdb = bar.r32(mhi::ERDBOFF) as usize;
    if chdb % 4 != 0 || chdb + 8 * cfg.max_channels > bar.len() || erdb % 4 != 0 || erdb + 8 * host.ev.len() > bar.len() {
        say(Level::Err, format_args!("MHI: doorbell offsets out of range (channels {:#x}, events {:#x})", chdb, erdb));
        return Err(host.fail("doorbell offsets outside BAR0"));
    }
    host.erdb = erdb;
    host.chdb = chdb;
    let w64 = |lo: usize, hi_: usize, v: u64| {
        bar.w32(hi_, (v >> 32) as u32);
        bar.w32(lo, v as u32);
    };
    w64(mhi::CCABAP_LOWER, mhi::CCABAP_HIGHER, hi[0]);
    w64(mhi::ECABAP_LOWER, mhi::ECABAP_HIGHER, hi[1]);
    w64(mhi::CRCBAP_LOWER, mhi::CRCBAP_HIGHER, hi[2]);
    w64(mhi::MHICTRLBASE_LOWER, mhi::MHICTRLBASE_HIGHER, 0);
    w64(mhi::MHIDATABASE_LOWER, mhi::MHIDATABASE_HIGHER, 0);
    w64(mhi::MHICTRLLIMIT_LOWER, mhi::MHICTRLLIMIT_HIGHER, cfg.iova_stop);
    w64(mhi::MHIDATALIMIT_LOWER, mhi::MHIDATALIMIT_HIGHER, cfg.iova_stop);
    let mhicfg = bar.r32(mhi::MHICFG);
    bar.w32(mhi::MHICFG, mhi::mhicfg_with_rings(mhicfg, host.ev.len() as u8, 0));
    for i in 0..host.ev.len() {
        let base = host.ev[i].ring.dev().get();
        let last_el = base + (host.ev[i].n - 1) * mhi::EL_BYTES as u64;
        host.ev_set_wp(i, last_el);
        host.ring_ev_db(bar, i);
    }
    let ctrl = bar.r32(mhi::MHICTRL);
    bar.w32(mhi::MHICTRL, mhi::mhictrl_with_state(ctrl, mhi::STATE_M0));
    say(
        Level::Info,
        format_args!(
            "MHI: contexts programmed (MHICFG {:#x} -> {:#x}, doorbells ch {:#x} ev {:#x}); M0 requested",
            mhicfg,
            bar.r32(mhi::MHICFG),
            chdb,
            erdb
        ),
    );

    // ---- events: M0, SBL -> BHIe, AMSS ----
    let start = tsc();
    let hz = crate::drivers::backends::cpuinfo::tsc_hz().max(1);
    let elapsed_ms = |t0: u64| (tsc().wrapping_sub(t0) / (hz / 1000).max(1)) as u32;
    let mut bhie_sent = false;
    let mut sbl_seen_reg: Option<u64> = None;
    let mut amss = false;
    let mut fatal: Option<&'static str> = None;
    let budget = cfg.timeout_ms * 2;
    loop {
        // Control and data rings.
        for i in 0..host.ev.len() {
            let mut evs: [Option<Event>; 16] = [None; 16];
            let mut k = 0usize;
            let r = host.drain(bar, i, &mut |e| {
                if k < evs.len() {
                    evs[k] = Some(e);
                }
                k += 1;
            });
            if let Err(why) = r {
                say(Level::Err, format_args!("MHI: {}", why));
                host.dump(bar, say);
                return Err(host.fail(why));
            }
            for e in evs.iter().flatten() {
                match *e {
                    Event::StateChange(s) => {
                        say(Level::Info, format_args!("event: MHI state -> {} (ring {})", mhi::state_name(s), i));
                        if s == mhi::STATE_M0 {
                            host.m0 = true;
                        } else if s == mhi::STATE_SYS_ERR {
                            fatal = Some("device entered SYS_ERR");
                        }
                    }
                    Event::ExecEnv(x) => {
                        say(Level::Info, format_args!("event: execution environment -> {} ({})", x, mhi::ee_name(x)));
                        host.ee = x;
                        match x {
                            mhi::EE_SBL if !bhie_sent => {
                                bhie_sent = true;
                                if let Err(why) = host.send_bhie(bar, cfg, segments, say) {
                                    host.dump(bar, say);
                                    return Err(host.fail(why));
                                }
                            }
                            mhi::EE_AMSS => amss = true,
                            mhi::EE_RDDM => fatal = Some("firmware crashed (RDDM)"),
                            _ => {}
                        }
                    }
                    other => say(Level::Info, format_args!("event: {:?} (ring {})", other, i)),
                }
            }
            if k > evs.len() {
                say(Level::Warn, format_args!("event ring {}: {} more event(s) not shown", i, k - evs.len()));
            }
        }
        if let Some(why) = fatal {
            say(Level::Err, format_args!("MHI: {}", why));
            host.dump(bar, say);
            return Err(host.fail(why));
        }
        // Register view, as a fallback for a lost event.
        let st = bar.r32(mhi::MHISTATUS);
        let ee = bar.r32(bhi + mhi::BHI_EXECENV);
        if dead(st) || dead(ee) {
            return Err(host.fail("link down while booting"));
        }
        if st & mhi::MHISTATUS_SYSERR != 0 || mhi::state_of(st) == mhi::STATE_SYS_ERR {
            say(Level::Err, format_args!("MHI: SYS_ERR (status {:#x}, EE {})", st, ee));
            host.dump(bar, say);
            return Err(host.fail("device reported SYS_ERR"));
        }
        if mhi::state_of(st) == mhi::STATE_M0 && !host.m0 {
            host.m0 = true;
            say(Level::Warn, format_args!("MHI: M0 seen in MHISTATUS without its event"));
        }
        if ee == mhi::EE_SBL && !bhie_sent {
            let t0 = *sbl_seen_reg.get_or_insert(tsc());
            if elapsed_ms(t0) > 2000 {
                say(Level::Warn, format_args!("MHI: EE register says SBL but no EE event after 2 s; sending BHIe anyway"));
                bhie_sent = true;
                if let Err(why) = host.send_bhie(bar, cfg, segments, say) {
                    host.dump(bar, say);
                    return Err(host.fail(why));
                }
            }
        }
        if ee == mhi::EE_AMSS && !amss {
            amss = true;
            say(Level::Warn, format_args!("MHI: AMSS seen in the EE register without its event"));
        }
        if amss && host.m0 {
            host.ee = mhi::EE_AMSS;
            say(
                Level::Ok,
                format_args!(
                    "MHI: main firmware (AMSS) running, state M0, {} event(s), {} interrupt(s), {} ms after M0 request",
                    host.events_seen,
                    irq_count(),
                    elapsed_ms(start)
                ),
            );
            return Ok(host);
        }
        if elapsed_ms(start) > budget {
            say(
                Level::Err,
                format_args!(
                    "MHI: no AMSS after {} ms (M0 {}, BHIe sent {}, EE {}, state {})",
                    budget,
                    host.m0,
                    bhie_sent,
                    ee,
                    mhi::state_name(mhi::state_of(st))
                ),
            );
            host.dump(bar, say);
            return Err(host.fail("timed out waiting for AMSS"));
        }
        delay_ms(5);
    }
}
