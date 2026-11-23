//! Split virtqueue (VIRTIO 1.2 §2.7), implementing the universal [`Queue`].
//!
//! Pure `core`: ring memory comes in as [`RingMem`] (a DMA pool in the kernel, a
//! `Vec` in the host tests) and the doorbell as a [`Doorbell`], so the host test
//! crate drives this exact file against a software device model.
//!
//! * The free list lives in driver-private `state[i].next_free`; the driver never
//!   reads back device-shared descriptor memory (hardening for a Ring-3 realm).
//! * Fences (x86 TSO, coherent DMA): Release store of `avail.idx`; one full fence
//!   before the kick decision; Acquire load of `used.idx`; one full fence in `arm`.
//! * Without EVENT_IDX: kick unless `used.flags & NO_NOTIFY`; disarm sets
//!   `avail.flags = NO_INTERRUPT`. With EVENT_IDX: `vring_need_event` decides the
//!   kick; arm writes `used_event`; disarm parks it half a ring behind.

use core::sync::atomic::{fence, AtomicU16, Ordering};

use crate::drivers::queue::{Budget, Completion, DeviceAddr, QStatus, Queue, QueueFull, Seg};

pub const DESC_F_NEXT: u16 = 1;
pub const DESC_F_WRITE: u16 = 2;
pub const AVAIL_F_NO_INTERRUPT: u16 = 1;
pub const USED_F_NO_NOTIFY: u16 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Desc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

/// Bytes of each ring part for `n` entries: (desc, avail, used).
pub const fn ring_bytes(n: usize) -> (usize, usize, usize) {
    (16 * n, 6 + 2 * n, 6 + 8 * n)
}

/// Legacy (pre-1.0) contiguous layout: (avail offset, used offset, total bytes).
pub const fn legacy_layout(n: usize) -> (usize, usize, usize) {
    let avail = 16 * n;
    let used = (avail + 6 + 2 * n + 4095) & !4095;
    (avail, used, used + ((6 + 8 * n + 4095) & !4095))
}

/// `true` if the device asked to be notified for the batch (old, new].
#[inline]
pub fn vring_need_event(event: u16, new: u16, old: u16) -> bool {
    new.wrapping_sub(event).wrapping_sub(1) < new.wrapping_sub(old)
}

pub trait Doorbell {
    fn notify(&self, queue: u16);
}

/// Ring memory, zeroed by the allocator. Pointers stay valid for the queue's life.
pub struct RingMem {
    pub desc: *mut Desc,
    pub avail: *mut u8,
    pub used: *mut u8,
    pub desc_dev: DeviceAddr,
    pub avail_dev: DeviceAddr,
    pub used_dev: DeviceAddr,
}

#[derive(Clone, Copy)]
struct DescState<C: Copy> {
    cookie: Option<C>,
    next_free: u16,
    chain_next: u16,
    chain_len: u16,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct QueueStats {
    pub submits: u64,
    pub kicks: u64,
    pub kicks_suppressed: u64,
    pub completions: u64,
    pub bad_completions: u64,
}

pub struct SplitQueue<C: Copy, D: Doorbell, const N: usize> {
    mem: RingMem,
    bell: D,
    qidx: u16,
    free_head: u16,
    num_free: u16,
    avail_shadow: u16,
    kicked_at: u16,
    staged: u16,
    last_used: u16,
    state: [DescState<C>; N],
    event_idx: bool,
    pub stats: QueueStats,
}

// Safety: the ring memory is owned by this queue (the owner serialises access).
unsafe impl<C: Copy + Send, D: Doorbell + Send, const N: usize> Send for SplitQueue<C, D, N> {}

impl<C: Copy, D: Doorbell, const N: usize> SplitQueue<C, D, N> {
    const SIZE_OK: () = assert!(N > 0 && N <= 32768 && N.is_power_of_two(), "virtqueue size must be a power of two");

    /// `mem` must be zeroed. Starts disarmed (polled): `avail.flags = NO_INTERRUPT`.
    pub fn new(mem: RingMem, bell: D, qidx: u16, event_idx: bool) -> Self {
        let () = Self::SIZE_OK;
        let mut state = [DescState { cookie: None, next_free: 0, chain_next: 0, chain_len: 0 }; N];
        for (i, s) in state.iter_mut().enumerate() {
            s.next_free = if i + 1 < N { (i + 1) as u16 } else { 0 };
        }
        let mut q = SplitQueue {
            mem,
            bell,
            qidx,
            free_head: 0,
            num_free: N as u16,
            avail_shadow: 0,
            kicked_at: 0,
            staged: 0,
            last_used: 0,
            state,
            event_idx,
            stats: QueueStats::default(),
        };
        q.disarm();
        q
    }

    pub fn addrs(&self) -> (DeviceAddr, DeviceAddr, DeviceAddr) {
        (self.mem.desc_dev, self.mem.avail_dev, self.mem.used_dev)
    }

    #[inline]
    fn avail_u16(&self, off: usize) -> &AtomicU16 {
        unsafe { &*(self.mem.avail.add(off) as *const AtomicU16) }
    }
    #[inline]
    fn used_u16(&self, off: usize) -> &AtomicU16 {
        unsafe { &*(self.mem.used.add(off) as *const AtomicU16) }
    }
    #[inline]
    fn used_elem(&self, slot: usize) -> (u32, u32) {
        unsafe {
            let p = self.mem.used.add(4 + 8 * slot) as *const u32;
            (core::ptr::read_volatile(p), core::ptr::read_volatile(p.add(1)))
        }
    }
    #[inline]
    fn used_event(&self) -> &AtomicU16 {
        self.avail_u16(4 + 2 * N)
    }
    #[inline]
    fn avail_event(&self) -> &AtomicU16 {
        self.used_u16(4 + 8 * N)
    }

    /// Completions waiting in the used ring (one acquire load).
    pub fn pending(&self) -> u16 {
        self.used_u16(2).load(Ordering::Acquire).wrapping_sub(self.last_used)
    }
}

impl<C: Copy, D: Doorbell, const N: usize> Queue for SplitQueue<C, D, N> {
    type Cookie = C;

    fn capacity(&self) -> u16 {
        N as u16
    }

    fn free_slots(&self) -> u16 {
        self.num_free
    }

    fn submit(&mut self, segs: &[Seg], cookie: C) -> Result<(), QueueFull> {
        let n = segs.len();
        if n == 0 || n > self.num_free as usize {
            return Err(QueueFull);
        }
        let head = self.free_head;
        let mut d = head;
        for (i, seg) in segs.iter().enumerate() {
            let next_free = self.state[d as usize].next_free;
            let last = i + 1 == n;
            let mut flags = if seg.device_writes { DESC_F_WRITE } else { 0 };
            if !last {
                flags |= DESC_F_NEXT;
            }
            let desc = Desc { addr: seg.addr.get(), len: seg.len, flags, next: if last { 0 } else { next_free } };
            unsafe { core::ptr::write_volatile(self.mem.desc.add(d as usize), desc) };
            self.state[d as usize].chain_next = next_free;
            d = next_free;
        }
        self.free_head = d;
        self.num_free -= n as u16;
        let st = &mut self.state[head as usize];
        st.cookie = Some(cookie);
        st.chain_len = n as u16;
        let slot = (self.avail_shadow as usize) % N;
        self.avail_u16(4 + 2 * slot).store(head, Ordering::Relaxed);
        self.avail_shadow = self.avail_shadow.wrapping_add(1);
        self.staged += 1;
        self.stats.submits += 1;
        Ok(())
    }

    fn kick(&mut self) -> bool {
        if self.staged == 0 {
            return false;
        }
        // Descriptor + ring entries before the index the device polls.
        self.avail_u16(2).store(self.avail_shadow, Ordering::Release);
        // The index store must be visible before we read the device's suppression state.
        fence(Ordering::SeqCst);
        let need = if self.event_idx {
            vring_need_event(self.avail_event().load(Ordering::Relaxed), self.avail_shadow, self.kicked_at)
        } else {
            self.used_u16(0).load(Ordering::Relaxed) & USED_F_NO_NOTIFY == 0
        };
        self.kicked_at = self.avail_shadow;
        self.staged = 0;
        if need {
            self.bell.notify(self.qidx);
            self.stats.kicks += 1;
        } else {
            self.stats.kicks_suppressed += 1;
        }
        need
    }

    fn reap<F: FnMut(Completion<C>)>(&mut self, budget: Budget, mut f: F) -> u16 {
        let snapshot = self.used_u16(2).load(Ordering::Acquire);
        let ready = snapshot.wrapping_sub(self.last_used);
        let n = if ready < budget.0 { ready } else { budget.0 };
        for _ in 0..n {
            let (id, len) = self.used_elem((self.last_used as usize) % N);
            self.last_used = self.last_used.wrapping_add(1);
            let idx = id as usize;
            let cookie = if idx < N { self.state[idx].cookie.take() } else { None };
            let Some(cookie) = cookie else {
                // Device returned a descriptor we never submitted (or twice).
                self.stats.bad_completions += 1;
                continue;
            };
            // Return the chain to the free list using only driver-private state.
            let chain = self.state[idx].chain_len;
            let mut d = idx as u16;
            for _ in 0..chain {
                let next = self.state[d as usize].chain_next;
                self.state[d as usize].next_free = self.free_head;
                self.free_head = d;
                d = next;
            }
            self.num_free += chain;
            self.stats.completions += 1;
            f(Completion { cookie, len, status: QStatus::Ok });
        }
        n
    }

    fn arm(&mut self) -> bool {
        if self.event_idx {
            self.used_event().store(self.last_used, Ordering::Relaxed);
        } else {
            self.avail_u16(0).store(0, Ordering::Relaxed);
        }
        fence(Ordering::SeqCst);
        self.used_u16(2).load(Ordering::Acquire) != self.last_used
    }

    fn disarm(&mut self) {
        if self.event_idx {
            // Park the event index half a ring behind: interrupts become one-shot.
            let park = self.last_used.wrapping_sub(1).wrapping_sub((N / 2) as u16);
            self.used_event().store(park, Ordering::Relaxed);
        } else {
            self.avail_u16(0).store(AVAIL_F_NO_INTERRUPT, Ordering::Relaxed);
        }
    }
}
