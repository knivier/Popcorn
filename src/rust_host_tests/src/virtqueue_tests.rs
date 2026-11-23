//! SplitQueue against a software virtio device model (virtio-net review §9.2).

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU16, Ordering};

use crate::drivers::queue::{Budget, DeviceAddr, Queue, Seg};
use crate::drivers::virtio::virtqueue::*;

const N: usize = 16;

struct Bell(Rc<Cell<u32>>);
impl Doorbell for Bell {
    fn notify(&self, _q: u16) {
        self.0.set(self.0.get() + 1);
    }
}

/// Backing memory for one queue (u64 words keep everything aligned).
struct Mem {
    desc: Vec<u64>,
    avail: Vec<u64>,
    used: Vec<u64>,
}

impl Mem {
    fn new() -> Mem {
        let (d, a, u) = ring_bytes(N);
        Mem { desc: vec![0; d / 8 + 1], avail: vec![0; a / 8 + 1], used: vec![0; u / 8 + 1] }
    }
    fn ring(&mut self) -> RingMem {
        RingMem {
            desc: self.desc.as_mut_ptr() as *mut Desc,
            avail: self.avail.as_mut_ptr() as *mut u8,
            used: self.used.as_mut_ptr() as *mut u8,
            desc_dev: DeviceAddr(0x1000),
            avail_dev: DeviceAddr(0x2000),
            used_dev: DeviceAddr(0x3000),
        }
    }
}

/// The device side: reads avail + descriptors, writes used (in any order).
struct Device {
    desc: *const Desc,
    avail: *const u8,
    used: *mut u8,
    last_avail: u16,
    used_idx: u16,
    inflight: Vec<(u16, u32)>, // (head, chain length)
    rng: u64,
}

impl Device {
    fn new(m: &mut Mem) -> Device {
        Device {
            desc: m.desc.as_ptr() as *const Desc,
            avail: m.avail.as_ptr() as *const u8,
            used: m.used.as_mut_ptr() as *mut u8,
            last_avail: 0,
            used_idx: 0,
            inflight: Vec::new(),
            rng: 0x9E37_79B9_7F4A_7C15,
        }
    }
    fn rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
    fn a16(&self, off: usize) -> u16 {
        unsafe { (*(self.avail.add(off) as *const AtomicU16)).load(Ordering::Acquire) }
    }
    fn u16_at(&self, off: usize) -> &AtomicU16 {
        unsafe { &*(self.used.add(off) as *const AtomicU16) }
    }
    /// Take every newly published request; validate its chain.
    fn consume(&mut self) {
        let idx = self.a16(2);
        while self.last_avail != idx {
            let head = self.a16(4 + 2 * (self.last_avail as usize % N));
            assert!((head as usize) < N, "avail ring holds a bad head");
            let mut d = head;
            let mut len = 1u32;
            let mut seen = HashSet::new();
            loop {
                assert!(seen.insert(d), "descriptor chain loops");
                let desc = unsafe { *self.desc.add(d as usize) };
                if desc.flags & DESC_F_NEXT == 0 {
                    break;
                }
                d = desc.next;
                len += 1;
                assert!((d as usize) < N);
            }
            self.inflight.push((head, len));
            self.last_avail = self.last_avail.wrapping_add(1);
        }
    }
    /// Complete up to `k` in-flight requests in random order.
    fn complete(&mut self, k: usize) -> usize {
        let mut done = 0;
        while done < k && !self.inflight.is_empty() {
            let i = (self.rand() as usize) % self.inflight.len();
            let (head, _) = self.inflight.swap_remove(i);
            let slot = self.used_idx as usize % N;
            let len = (self.rand() % 2000) as u32;
            unsafe {
                let p = self.used.add(4 + 8 * slot) as *mut u32;
                p.write_volatile(head as u32);
                p.add(1).write_volatile(len);
            }
            self.used_idx = self.used_idx.wrapping_add(1);
            self.u16_at(2).store(self.used_idx, Ordering::Release);
            done += 1;
        }
        done
    }
    fn set_no_notify(&self, on: bool) {
        self.u16_at(0).store(if on { USED_F_NO_NOTIFY } else { 0 }, Ordering::Relaxed);
    }
    fn avail_flags(&self) -> u16 {
        self.a16(0)
    }
}

fn seg(i: u64) -> Seg {
    Seg { addr: DeviceAddr(0x10_0000 + i * 2048), len: 2048, device_writes: i % 2 == 0 }
}

#[test]
fn need_event_edges() {
    // Device wants a kick once avail passes `event`.
    assert!(vring_need_event(5, 6, 5)); // event == old
    assert!(vring_need_event(5, 6, 0)); // crossed within the batch
    assert!(!vring_need_event(9, 6, 0)); // not reached yet
    assert!(vring_need_event(5, 6, 5) && !vring_need_event(6, 6, 5)); // event == new-1 kicks, event == new does not
    assert!(vring_need_event(0xFFFF, 0, 0xFFFE)); // across the u16 wrap
    assert!(!vring_need_event(0x8000, 0, 0xFFFF)); // distance 0x8000: far behind, no kick
}

#[test]
fn full_ring_and_free_list() {
    let mut m = Mem::new();
    let bell = Rc::new(Cell::new(0));
    let mut q: SplitQueue<u32, Bell, N> = SplitQueue::new(m.ring(), Bell(bell.clone()), 0, false);
    // 3-descriptor chains: 5 fit in 16, the 6th does not.
    for i in 0..5 {
        q.submit(&[seg(0), seg(1), seg(2)], i).unwrap();
    }
    assert!(q.submit(&[seg(0), seg(1), seg(2)], 99).is_err());
    assert!(q.submit(&[seg(0)], 5).is_ok()); // one descriptor still free
    assert!(q.submit(&[seg(0)], 6).is_err());
    assert_eq!(q.free_slots(), 0);
    assert!(q.submit(&[], 7).is_err(), "empty request must be rejected");
}

fn soak(event_idx: bool, ops: usize) {
    let mut m = Mem::new();
    let bell = Rc::new(Cell::new(0));
    let mut q: SplitQueue<u32, Bell, N> = SplitQueue::new(m.ring(), Bell(bell.clone()), 3, event_idx);
    let mut dev = Device::new(&mut m);
    let mut outstanding: HashSet<u32> = HashSet::new();
    let mut next_cookie = 0u32;
    let mut completed = 0usize;
    let mut r: u64 = 12345;
    let mut rnd = || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        r
    };
    for step in 0..ops {
        // Submit a burst of random-length chains while there is room.
        let burst = (rnd() % 6) as usize;
        for _ in 0..burst {
            let len = 1 + (rnd() % 3) as usize;
            if (q.free_slots() as usize) < len {
                break;
            }
            let segs: Vec<Seg> = (0..len as u64).map(seg).collect();
            q.submit(&segs, next_cookie).unwrap();
            assert!(outstanding.insert(next_cookie));
            next_cookie = next_cookie.wrapping_add(1);
        }
        q.kick();
        dev.consume();
        dev.set_no_notify(step % 7 == 0);
        dev.complete((rnd() % 5) as usize);
        q.reap(Budget((1 + rnd() % 8) as u16), |c| {
            assert!(outstanding.remove(&c.cookie), "cookie {} completed twice or never submitted", c.cookie);
            completed += 1;
        });
        // Descriptors in flight never exceed the ring.
        let in_flight_descs = N - q.free_slots() as usize;
        assert!(in_flight_descs <= N);
    }
    // Drain.
    loop {
        q.kick();
        dev.consume();
        dev.complete(N);
        let got = q.reap(Budget(64), |c| {
            assert!(outstanding.remove(&c.cookie));
            completed += 1;
        });
        if got == 0 && outstanding.is_empty() {
            break;
        }
    }
    assert_eq!(q.free_slots() as usize, N, "every descriptor returned to the free list");
    assert_eq!(q.stats.bad_completions, 0);
    assert!(completed > 70_000, "enough traffic to wrap the 16-bit indices: {completed}");
}

#[test]
fn random_order_soak_wraps_indices_no_event_idx() {
    soak(false, 60_000);
}

#[test]
fn random_order_soak_wraps_indices_event_idx() {
    soak(true, 60_000);
}

#[test]
fn arm_detects_race() {
    let mut m = Mem::new();
    let bell = Rc::new(Cell::new(0));
    let mut q: SplitQueue<u32, Bell, N> = SplitQueue::new(m.ring(), Bell(bell.clone()), 0, false);
    let mut dev = Device::new(&mut m);
    assert_eq!(dev.avail_flags(), AVAIL_F_NO_INTERRUPT, "starts disarmed (polled)");
    q.submit(&[seg(0)], 1).unwrap();
    q.kick();
    dev.consume();
    // Nothing completed yet: arming finds no work and clears NO_INTERRUPT.
    assert!(!q.arm());
    assert_eq!(dev.avail_flags(), 0);
    // The device completes "between" the arm and the re-check: arm must report it.
    dev.complete(1);
    assert!(q.arm(), "work that raced in must be reported");
    assert_eq!(q.reap(Budget(8), |_| {}), 1);
    q.disarm();
    assert_eq!(dev.avail_flags(), AVAIL_F_NO_INTERRUPT);
}

#[test]
fn kick_suppression_without_event_idx() {
    let mut m = Mem::new();
    let bell = Rc::new(Cell::new(0));
    let mut q: SplitQueue<u32, Bell, N> = SplitQueue::new(m.ring(), Bell(bell.clone()), 0, false);
    let dev = Device::new(&mut m);
    q.submit(&[seg(0)], 1).unwrap();
    assert!(q.kick());
    assert_eq!(bell.get(), 1);
    dev.set_no_notify(true);
    q.submit(&[seg(1)], 2).unwrap();
    assert!(!q.kick(), "NO_NOTIFY suppresses the doorbell");
    assert_eq!(bell.get(), 1);
    assert!(!q.kick(), "nothing staged: no doorbell");
    assert_eq!(q.stats.kicks_suppressed, 1);
}

#[test]
fn legacy_layout_is_page_aligned() {
    let (avail, used, total) = legacy_layout(256);
    assert_eq!(avail, 4096);
    assert_eq!(used % 4096, 0);
    assert!(used >= avail + 6 + 2 * 256);
    assert_eq!(total % 4096, 0);
}
