//! virtio-net driver (modern / VERSION_1, polled in M1) — virtio-net review §5.
//!
//! * Queue 0 = receiveq1, queue 1 = transmitq1, 256 entries each (`SplitQueue`).
//! * One `DmaPool`: rings (3 pages per queue), RX slab 256 x 2 KiB, TX slab 256 x 2 KiB.
//! * RX slot layout: buffer posted at +38 (len 2010), so the 12-byte virtio-net
//!   header lands at [38, 50), Ethernet at [50, 64) and the IPv4 header at 64,
//!   cache-line aligned. Frames are lent zero-copy to the stack (`RxBatch`).
//! * TX slot layout: frame written at +64 by the stack, header prepended at +52;
//!   one descriptor of 12 + len bytes. One doorbell per `tx_flush`.
//! * Interrupts (M2, hybrid): MSI-X entry 0 = config, 1 = RX, 2 = TX. The pump
//!   polls the used rings every pass (one RAM load, no MMIO); RX interrupts only
//!   wake the CPU from `hlt`: `idle_arm` unmasks + arms before sleeping,
//!   `wake_disarm` masks again while the pump drains. TX interrupts stay off (TX
//!   is reaped by polling). With EVENT_IDX, kicks and interrupts are suppressed
//!   by index. Without MSI-X (QEMU `vectors=0`) everything is polled.

use core::fmt::Write;

use super::pci::{open, AnyTransport, Prefer};
use super::transport::{Notifier, QueueAddrs, Transport};
use super::virtqueue::{ring_bytes, Desc, RingMem, SplitQueue};
use super::{negotiate, F_RING_EVENT_IDX, STATUS_DRIVER_OK, STATUS_FAILED, F_VERSION_1};
use crate::drivers::bus::msix::{self, IrqSet};
use crate::drivers::bus::pci::{self, PciFunction};
use crate::drivers::grant::{BufSlab, DmaPool, IrqHandle};
use crate::drivers::net::{NetDevice, NetStats, RxBatch, TxBuf, TxError, MAX_FRAME};
use crate::drivers::queue::{Budget, Queue, Seg};
use crate::sync::TaskLock;

pub const F_NET_MAC: u64 = 1 << 5;
pub const F_NET_MTU: u64 = 1 << 3;
pub const F_NET_STATUS: u64 = 1 << 16;
const NET_WANTED: u64 = F_VERSION_1 | F_NET_MAC | F_NET_MTU | F_NET_STATUS | F_RING_EVENT_IDX;
const NET_REQUIRED: u64 = F_VERSION_1;

const QN: usize = 256;
const SLOT: usize = 2048;
const HDR_LEN: usize = 12; // virtio_net_hdr with VERSION_1
const RX_OFF: usize = 38;
const RX_LEN: usize = SLOT - RX_OFF; // 2010
const TX_FRAME_OFF: usize = 64;
const TX_HDR_OFF: usize = TX_FRAME_OFF - HDR_LEN;

type NetQueue = SplitQueue<u16, Notifier, QN>;

pub struct VirtioNet {
    pci: PciFunction,
    transport: AnyTransport,
    rxq: NetQueue,
    txq: NetQueue,
    rx_slab: BufSlab,
    tx_slab: BufSlab,
    tx_free: [u16; QN],
    tx_free_n: usize,
    rx_posted: u32,
    mac: [u8; 6],
    mtu: u16,
    status_feature: bool,
    features_offered: u64,
    features_acked: u64,
    stats: NetStats,
    rx_irq: IrqHandle,
    tx_irq: IrqHandle,
    cfg_irq: IrqHandle,
    irq_wakes: u64,
    irq_redundant: u64,
    _irqs: IrqSet,
    // Last in the struct: dropped after the device is reset in `shutdown`.
    pool: DmaPool,
}

// Safety: all fields are owned device state; access is serialised by NET0's lock.
unsafe impl Send for VirtioNet {}

/// The (single, for now) virtio NIC. Lock order: stack iface lock → NET0.
pub static NET0: TaskLock<Option<VirtioNet>> = TaskLock::new(None);

fn com1(args: core::fmt::Arguments) {
    let mut out = crate::Com1;
    let _ = out.write_fmt(args);
}

/// Registry probe entry (`pci_drivers.rs`). On failure the function is released.
pub fn probe(f: PciFunction) -> Result<(), &'static str> {
    let mut slot = NET0.lock();
    if slot.is_some() {
        pci::release(f);
        return Err("net0 already bound (one NIC supported for now)");
    }
    match bring_up(f) {
        Ok(dev) => {
            com1(format_args!(
                "net: probe ok mac {} mtu {} feat dev={:#x} acked={:#x} irq {}\n",
                MacHex(dev.mac),
                dev.mtu,
                dev.features_offered,
                dev.features_acked,
                dev.irq_mode()
            ));
            match dev.rx_irq.vector() {
                Some(v) => com1(format_args!("net: irq msix v={:#04x}\n", v)),
                None => com1(format_args!("net: irq poll\n")),
            }
            crate::catalog::publish_device("net0", "net", 0);
            *slot = Some(dev);
            Ok(())
        }
        Err((f, why)) => {
            com1(format_args!("net: probe fail {}\n", why));
            f.disable_bus_master();
            pci::release(f);
            Err(why)
        }
    }
}

fn bring_up(f: PciFunction) -> Result<VirtioNet, (PciFunction, &'static str)> {
    f.enable(true, false, false);
    let mut transport = match open(&f, Prefer::Modern) {
        Ok(t @ AnyTransport::Modern(_)) => t,
        Ok(_) => return Err((f, "legacy-only device (net is modern-only)")),
        Err(e) => return Err((f, e.as_str())),
    };
    let t = transport.as_dyn();
    // OVMF's VirtioNetDxe may have left the device live: reset before bus mastering.
    if let Err(e) = t.reset() {
        return Err((f, e.as_str()));
    }
    f.enable(true, false, true);
    let offered = t.device_features();
    let acked = match negotiate(t, NET_WANTED, NET_REQUIRED) {
        Ok(a) => a,
        Err(e) => return Err((f, e.as_str())),
    };

    // Device config: mac @0, status @6, mtu @10 (generation-checked reads).
    let mut cfg = [0u8; 12];
    t.read_config(0, &mut cfg);
    let mac = if acked & F_NET_MAC != 0 {
        [cfg[0], cfg[1], cfg[2], cfg[3], cfg[4], cfg[5]]
    } else {
        [0x52, 0x54, 0x00, 0x00, 0x00, 0x01] // locally administered fallback
    };
    let mtu = if acked & F_NET_MTU != 0 { u16::from_le_bytes([cfg[10], cfg[11]]) } else { 1500 };
    if mtu > 1500 {
        t.add_status(STATUS_FAILED);
        return Err((f, "mtu above 1500 (no jumbo support yet)"));
    }

    // MSI-X before any device-config-dependent work (legacy would move config 20 -> 24).
    let mut irqs = match msix::alloc_irqs(&f, 3, "net0") {
        Ok(s) => s,
        Err(why) => {
            com1(format_args!("net: msix unavailable ({}), polling\n", why));
            IrqSet::polled()
        }
    };
    let mut entries: [Option<u16>; 3] = [None; 3];
    if !irqs.is_polled() {
        if t.set_config_vector(Some(0)).is_ok() {
            entries = [Some(0), Some(1), Some(2)];
        } else {
            // Device refused the vector: run polled, give the vectors back.
            com1(format_args!("net: device refused MSI-X config vector, polling\n"));
            let _ = t.set_config_vector(None);
            msix::disable(&f);
            irqs = IrqSet::polled();
        }
    }
    let event_idx = acked & F_RING_EVENT_IDX != 0;

    let mut pool = DmaPool::new(f.grant(), 320);
    let (rxq, rx_slab) = match setup_queue(t, &mut pool, 0, entries[1], event_idx) {
        Ok(q) => q,
        Err(e) => {
            t.add_status(STATUS_FAILED);
            return Err((f, e));
        }
    };
    let (txq, tx_slab) = match setup_queue(t, &mut pool, 1, entries[2], event_idx) {
        Ok(q) => q,
        Err(e) => {
            t.add_status(STATUS_FAILED);
            pool.poison(); // queue 0 is already live in the device
            return Err((f, e));
        }
    };

    let cfg_irq = irqs.take(0);
    let rx_irq = irqs.take(1);
    let tx_irq = irqs.take(2);
    let mut dev = VirtioNet {
        rx_irq,
        tx_irq,
        cfg_irq,
        irq_wakes: 0,
        irq_redundant: 0,
        _irqs: irqs,
        pci: f,
        transport,
        rxq,
        txq,
        rx_slab,
        tx_slab,
        tx_free: [0; QN],
        tx_free_n: QN,
        rx_posted: 0,
        mac,
        mtu,
        status_feature: acked & F_NET_STATUS != 0,
        features_offered: offered,
        features_acked: acked,
        stats: NetStats::default(),
        pool,
    };
    for i in 0..QN {
        dev.tx_free[i] = i as u16;
    }
    // Post every RX buffer (no notify before DRIVER_OK: spec §3.1.1).
    for id in 0..QN as u16 {
        dev.post_rx(id);
    }
    dev.transport.as_dyn().add_status(STATUS_DRIVER_OK);
    dev.rxq.kick();
    Ok(dev)
}

fn setup_queue(
    t: &mut dyn Transport,
    pool: &mut DmaPool,
    q: u16,
    entry: Option<u16>,
    event_idx: bool,
) -> Result<(NetQueue, BufSlab), &'static str> {
    if (t.queue_max(q) as usize) < QN {
        return Err("queue smaller than 256");
    }
    let (db, ab, ub) = ring_bytes(QN);
    let desc = pool.alloc_array::<u8>(db, 4096).map_err(|e| e.as_str())?;
    let avail = pool.alloc_array::<u8>(ab, 4096).map_err(|e| e.as_str())?;
    let used = pool.alloc_array::<u8>(ub, 4096).map_err(|e| e.as_str())?;
    let slab = pool.slab(SLOT, QN as u16).map_err(|e| e.as_str())?;
    let addrs = QueueAddrs { desc: desc.dev(), driver: avail.dev(), device: used.dev() };
    t.setup_queue(q, QN as u16, &addrs, entry).map_err(|e| e.as_str())?;
    let bell = t.notifier(q).map_err(|e| e.as_str())?;
    t.enable_queue(q);
    let mem = RingMem {
        desc: desc.as_ptr() as *mut Desc,
        avail: avail.as_ptr(),
        used: used.as_ptr(),
        desc_dev: desc.dev(),
        avail_dev: avail.dev(),
        used_dev: used.dev(),
    };
    Ok((SplitQueue::new(mem, bell, q, event_idx), slab))
}

impl VirtioNet {
    fn post_rx(&mut self, id: u16) {
        let seg = [Seg { addr: self.rx_slab.addr(id).offset(RX_OFF as u64), len: RX_LEN as u32, device_writes: true }];
        if self.rxq.submit(&seg, id).is_ok() {
            self.rx_posted += 1;
        }
    }

    pub fn pci_addr(&self) -> pci::PciAddr {
        self.pci.addr()
    }
    pub fn features(&self) -> (u64, u64) {
        (self.features_offered, self.features_acked)
    }
    pub fn dma_pages(&self) -> u32 {
        self.pool.outstanding_pages()
    }
    /// (rx vector or 0, interrupts that woke the pump, wakes that found nothing).
    pub fn irq_info(&self) -> (u8, u64, u64) {
        (self.rx_irq.vector().unwrap_or(0), self.irq_wakes, self.irq_redundant)
    }

    /// Reset the device (stops all DMA), then release the PCI function. The pool
    /// is freed when the struct drops, which is only safe after the reset.
    pub fn shutdown(mut self) {
        self.rx_irq.mask();
        if self.transport.as_dyn().reset().is_err() {
            self.pool.poison();
        }
        msix::disable(&self.pci);
        self.pci.disable_bus_master();
        let VirtioNet { pci: f, .. } = self;
        pci::release(f);
    }
}

impl NetDevice for VirtioNet {
    fn name(&self) -> &'static str {
        "net0"
    }
    fn mac(&self) -> [u8; 6] {
        self.mac
    }
    fn mtu(&self) -> u16 {
        self.mtu
    }
    fn link_up(&self) -> bool {
        if !self.status_feature {
            return true;
        }
        let mut st = [0u8; 2];
        self.transport_ref_read(6, &mut st);
        u16::from_le_bytes(st) & 1 != 0
    }

    fn rx_batch(&mut self, budget: usize, out: &mut RxBatch) -> usize {
        out.clear();
        let budget = budget.min(crate::drivers::net::RX_BATCH);
        let mut bad: [u16; 32] = [0; 32];
        let mut nbad = 0usize;
        let slab = &self.rx_slab;
        let stats = &mut self.stats;
        let posted = &mut self.rx_posted;
        self.rxq.reap(Budget(budget as u16), |c| {
            *posted -= 1;
            let id = c.cookie;
            let len = c.len as usize;
            if len < HDR_LEN + 14 || len > HDR_LEN + MAX_FRAME + 4 {
                stats.rx_errors += 1;
                if nbad < bad.len() {
                    bad[nbad] = id;
                    nbad += 1;
                }
                return;
            }
            let frame = slab.bytes(id)[RX_OFF + HDR_LEN..].as_ptr();
            let flen = (len - HDR_LEN).min(MAX_FRAME) as u16;
            out.push(id, frame, flen);
            stats.rx_packets += 1;
            stats.rx_bytes += flen as u64;
        });
        // Malformed completions go straight back to the device.
        for &id in &bad[..nbad] {
            self.post_rx(id);
        }
        if nbad > 0 {
            self.rxq.kick();
        }
        out.len()
    }

    fn rx_recycle(&mut self, batch: &mut RxBatch) {
        for i in 0..batch.len() {
            self.post_rx(batch.id(i));
        }
        batch.clear();
        if self.rxq.kick() {
            self.stats.kicks += 1;
        }
    }

    fn tx_alloc(&mut self) -> Option<TxBuf> {
        if self.tx_free_n == 0 {
            self.reap_tx(64);
        }
        if self.tx_free_n == 0 {
            self.stats.tx_full += 1;
            return None;
        }
        self.tx_free_n -= 1;
        let id = self.tx_free[self.tx_free_n];
        let frame = self.tx_slab.bytes_mut(id)[TX_FRAME_OFF..].as_mut_ptr();
        Some(TxBuf::new(id, frame, MAX_FRAME as u16))
    }

    fn tx_submit(&mut self, buf: TxBuf, len: usize) -> Result<(), TxError> {
        let id = buf.id();
        if len > MAX_FRAME {
            self.give_back_tx(id);
            return Err(TxError);
        }
        let bytes = self.tx_slab.bytes_mut(id);
        // Zero virtio_net_hdr (GSO_NONE, no checksum offload) and pad runts to 60.
        crate::util::copy::fill(&mut bytes[TX_HDR_OFF..TX_FRAME_OFF], 0);
        let len = if len < 60 {
            crate::util::copy::fill(&mut bytes[TX_FRAME_OFF + len..TX_FRAME_OFF + 60], 0);
            60
        } else {
            len
        };
        let seg = [Seg { addr: self.tx_slab.addr(id).offset(TX_HDR_OFF as u64), len: (HDR_LEN + len) as u32, device_writes: false }];
        if self.txq.submit(&seg, id).is_err() {
            self.give_back_tx(id);
            self.stats.tx_full += 1;
            return Err(TxError);
        }
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += len as u64;
        Ok(())
    }

    fn tx_flush(&mut self) {
        if self.txq.kick() {
            self.stats.kicks += 1;
        }
    }

    fn reap_tx(&mut self, budget: usize) -> usize {
        let free = &mut self.tx_free;
        let n = &mut self.tx_free_n;
        self.txq.reap(Budget(budget as u16), |c| {
            if *n < QN {
                free[*n] = c.cookie;
                *n += 1;
            }
        }) as usize
    }

    fn irq_mode(&self) -> &'static str {
        if self.rx_irq.is_polled() {
            "poll"
        } else {
            "msix"
        }
    }

    fn idle_arm(&mut self) -> bool {
        if self.rx_irq.is_polled() {
            return false;
        }
        self.rx_irq.unmask();
        // arm(): request an interrupt (used_event / flags), full fence, re-check.
        self.rxq.arm()
    }

    fn wake_disarm(&mut self) -> bool {
        if self.rx_irq.is_polled() {
            return false;
        }
        let fired = self.rx_irq.take();
        self.rx_irq.mask();
        self.rxq.disarm();
        if fired {
            self.irq_wakes += 1;
            if self.rxq.pending() == 0 {
                self.irq_redundant += 1;
            }
        }
        fired
    }

    fn irq_count(&self) -> u64 {
        self.rx_irq.count() + self.tx_irq.count() + self.cfg_irq.count()
    }

    fn stats(&self) -> NetStats {
        let mut s = self.stats;
        s.kicks = self.rxq.stats.kicks + self.txq.stats.kicks;
        s.kicks_suppressed = self.rxq.stats.kicks_suppressed + self.txq.stats.kicks_suppressed;
        s.rx_posted = self.rx_posted;
        s.tx_free = self.tx_free_n as u32;
        s
    }
}

impl VirtioNet {
    fn give_back_tx(&mut self, id: u16) {
        if self.tx_free_n < QN {
            self.tx_free[self.tx_free_n] = id;
            self.tx_free_n += 1;
        }
    }
    fn transport_ref_read(&self, off: usize, out: &mut [u8]) {
        match &self.transport {
            AnyTransport::Modern(m) => m.read_config(off, out),
            AnyTransport::Legacy(l) => l.read_config(off, out),
        }
    }
}

/// `52540012345` style lowercase MAC for log tags.
pub struct MacHex(pub [u8; 6]);

impl core::fmt::Display for MacHex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}
