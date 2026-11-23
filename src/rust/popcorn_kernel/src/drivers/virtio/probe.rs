//! M0 exit check (`virtio -probe`): drive a **modern-only** virtio-blk device
//! (0x1042, never touched by the legacy block driver) end to end with the new
//! framework — claim, ModernTransport, negotiate, SplitQueue on a DmaPool, one
//! 512-byte read — then reset it. Proves the transport and the virtqueue before
//! virtio-net depends on them.

use core::fmt::Write;

use super::pci::{claim, open, AnyTransport, Prefer, BLK_MODERN_ONLY};
use super::transport::{QueueAddrs, Transport};
use super::virtqueue::{ring_bytes, Desc, RingMem, SplitQueue};
use super::{negotiate, VirtioError, F_VERSION_1, STATUS_DRIVER_OK};
use crate::drivers::bus::pci;
use crate::drivers::grant::DmaPool;
use crate::drivers::queue::{Budget, Queue, Seg};

const QSIZE: usize = 16;
const BLK_T_IN: u32 = 0;

pub struct ProbeResult {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    pub capacity: u64,
    pub first16: [u8; 16],
    pub kicks: u64,
}

pub fn probe_modern_blk() -> Result<ProbeResult, &'static str> {
    let f = claim(&BLK_MODERN_ONLY, 2, 0).ok_or("no modern-only virtio-blk (0x1042)")?;
    let a = f.addr();
    let result = run(&f).map(|(capacity, first16, kicks)| ProbeResult {
        bus: a.bus,
        slot: a.slot,
        func: a.func,
        capacity,
        first16,
        kicks,
    });
    f.disable_bus_master();
    pci::release(f);
    result.map_err(|e| e.as_str())
}

fn run(f: &pci::PciFunction) -> Result<(u64, [u8; 16], u64), VirtioError> {
    f.enable(true, false, false);
    let mut any = open(f, Prefer::Modern)?;
    let t: &mut dyn Transport = match &mut any {
        AnyTransport::Modern(m) => m,
        AnyTransport::Legacy(_) => return Err(VirtioError::BadCaps),
    };
    t.reset()?; // firmware (OVMF VirtioBlkDxe) may have left it live
    f.enable(true, false, true); // bus mastering only after the reset
    negotiate(t, F_VERSION_1, F_VERSION_1)?;

    let mut cap = [0u8; 8];
    t.read_config(0, &mut cap);
    let capacity = u64::from_le_bytes(cap);

    let max = t.queue_max(0);
    if (max as usize) < QSIZE {
        return Err(VirtioError::QueueTooSmall);
    }
    let mut pool = DmaPool::new(f.grant(), 64);
    let (db, ab, ub) = ring_bytes(QSIZE);
    let desc = pool.alloc_array::<u8>(db, 4096).map_err(|_| VirtioError::NoMemory)?;
    let avail = pool.alloc_array::<u8>(ab, 4096).map_err(|_| VirtioError::NoMemory)?;
    let used = pool.alloc_array::<u8>(ub, 4096).map_err(|_| VirtioError::NoMemory)?;
    let hdr = pool.alloc_array::<u8>(16, 16).map_err(|_| VirtioError::NoMemory)?;
    let data = pool.alloc_array::<u8>(512, 512).map_err(|_| VirtioError::NoMemory)?;
    let status = pool.alloc_array::<u8>(1, 1).map_err(|_| VirtioError::NoMemory)?;
    status.set(0, 0xFF);

    let addrs = QueueAddrs { desc: desc.dev(), driver: avail.dev(), device: used.dev() };
    t.setup_queue(0, QSIZE as u16, &addrs, None)?;
    let bell = t.notifier(0)?;
    t.enable_queue(0);
    let mem = RingMem {
        desc: desc.as_ptr() as *mut Desc,
        avail: avail.as_ptr(),
        used: used.as_ptr(),
        desc_dev: desc.dev(),
        avail_dev: avail.dev(),
        used_dev: used.dev(),
    };
    let mut q: SplitQueue<u16, _, QSIZE> = SplitQueue::new(mem, bell, 0, false);
    t.add_status(STATUS_DRIVER_OK);

    // virtio_blk_req: type le32, reserved le32, sector le64 | data | status u8
    for (i, b) in BLK_T_IN.to_le_bytes().iter().enumerate() {
        hdr.set(i, *b);
    }
    let segs = [
        Seg { addr: hdr.dev(), len: 16, device_writes: false },
        Seg { addr: data.dev(), len: 512, device_writes: true },
        Seg { addr: status.dev(), len: 1, device_writes: true },
    ];
    q.submit(&segs, 7).map_err(|_| VirtioError::QueueUnavailable)?;
    q.kick();

    let mut done = false;
    for _ in 0..50_000_000u32 {
        if q.reap(Budget(4), |c| done |= c.cookie == 7) > 0 && done {
            break;
        }
        core::hint::spin_loop();
    }
    let ok = done && status.get(0) == 0;
    let mut first16 = [0u8; 16];
    for (i, b) in first16.iter_mut().enumerate() {
        *b = data.get(i);
    }
    let kicks = q.stats.kicks;
    // Stop the device before its rings and buffers go back to the allocator.
    if t.reset().is_err() {
        pool.poison();
    }
    if !ok {
        return Err(VirtioError::QueueUnavailable);
    }
    Ok((capacity, first16, kicks))
}

/// Shell output for `virtio -probe` (also printed as a COM1 `virtio:` tag).
pub fn probe_command() {
    use crate::console_ffi::{print_color, println_color, COLOR_LIGHT_GREEN, COLOR_YELLOW};
    let mut line = LineBuf::new();
    match probe_modern_blk() {
        Ok(r) => {
            let _ = write!(line, "virtio: blk modern {}:{}.{} capacity={} sectors read0=", r.bus, r.slot, r.func, r.capacity);
            for b in r.first16 {
                let _ = write!(line, "{:02x}", b);
            }
            let _ = write!(line, " kicks={} ok", r.kicks);
            println_color(line.as_str(), COLOR_LIGHT_GREEN);
        }
        Err(e) => {
            let _ = write!(line, "virtio: probe fail ({})", e);
            print_color(line.as_str(), COLOR_YELLOW);
            println_color("", COLOR_YELLOW);
        }
    }
    let mut out = crate::Com1;
    let _ = writeln!(out, "{}", line.as_str());
}

/// Fixed stack buffer for formatted lines (no heap).
pub struct LineBuf {
    buf: [u8; 200],
    len: usize,
}

impl LineBuf {
    pub const fn new() -> LineBuf {
        LineBuf { buf: [0; 200], len: 0 }
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
}

impl core::fmt::Write for LineBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}
