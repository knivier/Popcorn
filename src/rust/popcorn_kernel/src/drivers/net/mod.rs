//! Network device class: the one interface every NIC driver implements
//! (virtio-net now; Ethernet, USB-NCM and the Qualcomm WCN6855 Wi-Fi driver later).
//! The protocol stack (`crate::net`) sees only this trait, never a specific card.
//!
//! Pure `core` (no kernel dependencies) so the host test crate can include it.
//!
//! Buffer model (no per-packet allocation, zero-copy RX):
//! * **RX**: `rx_batch` lends up to `RX_BATCH` received frames as raw slices into
//!   the driver's buffer memory (which lives as long as the driver). The stack may
//!   transmit replies on the same device while it still holds the batch, then
//!   returns the buffers with `rx_recycle`.
//! * **TX**: `tx_alloc` hands out an empty frame buffer; the stack writes an
//!   Ethernet frame into it and calls `tx_submit`; `tx_flush` rings the doorbell
//!   once for everything submitted (one kick per pump pass).

pub mod loopback;

/// Largest Ethernet frame without FCS (14-byte header + 1500 payload).
pub const MAX_FRAME: usize = 1514;
/// Frames lent by one `rx_batch` call.
pub const RX_BATCH: usize = 32;

#[derive(Clone, Copy, Default, Debug)]
pub struct NetStats {
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub rx_errors: u64,
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub tx_full: u64,
    pub kicks: u64,
    pub kicks_suppressed: u64,
    /// Buffers currently posted to the device for RX (leak check).
    pub rx_posted: u32,
    /// TX buffers currently free (leak check).
    pub tx_free: u32,
}

/// Received frames lent by the driver. Valid until `rx_recycle`.
pub struct RxBatch {
    ids: [u16; RX_BATCH],
    ptrs: [usize; RX_BATCH],
    lens: [u16; RX_BATCH],
    n: usize,
}

impl RxBatch {
    pub const fn new() -> RxBatch {
        RxBatch { ids: [0; RX_BATCH], ptrs: [0; RX_BATCH], lens: [0; RX_BATCH], n: 0 }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    /// Ethernet frame `i` (destination MAC first; no device header).
    #[inline]
    pub fn frame(&self, i: usize) -> &[u8] {
        assert!(i < self.n);
        // Safety: the driver guarantees the memory stays valid and unchanged until
        // rx_recycle (it is not reposted to the device before that).
        unsafe { core::slice::from_raw_parts(self.ptrs[i] as *const u8, self.lens[i] as usize) }
    }
    /// Driver side: add a frame. Returns false when full.
    pub fn push(&mut self, id: u16, frame: *const u8, len: u16) -> bool {
        if self.n >= RX_BATCH {
            return false;
        }
        self.ids[self.n] = id;
        self.ptrs[self.n] = frame as usize;
        self.lens[self.n] = len;
        self.n += 1;
        true
    }
    /// Driver side: the buffer id of frame `i`.
    pub fn id(&self, i: usize) -> u16 {
        self.ids[i]
    }
    pub fn clear(&mut self) {
        self.n = 0;
    }
}

/// An empty TX frame buffer lent by the driver (up to `MAX_FRAME` bytes).
pub struct TxBuf {
    id: u16,
    ptr: usize,
    cap: u16,
}

impl TxBuf {
    /// Driver side.
    pub fn new(id: u16, frame: *mut u8, cap: u16) -> TxBuf {
        TxBuf { id, ptr: frame as usize, cap }
    }
    pub fn id(&self) -> u16 {
        self.id
    }
    /// The frame bytes to fill (destination MAC first).
    #[inline]
    pub fn frame_mut(&mut self) -> &mut [u8] {
        // Safety: the driver lent this buffer exclusively until tx_submit.
        unsafe { core::slice::from_raw_parts_mut(self.ptr as *mut u8, self.cap as usize) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TxError;

pub trait NetDevice {
    fn name(&self) -> &'static str;
    fn mac(&self) -> [u8; 6];
    fn mtu(&self) -> u16;
    fn link_up(&self) -> bool;
    /// Lend up to `budget` (≤ RX_BATCH) received frames. Returns how many.
    fn rx_batch(&mut self, budget: usize, out: &mut RxBatch) -> usize;
    /// Give the lent frames back (they are reposted to the device).
    fn rx_recycle(&mut self, batch: &mut RxBatch);
    /// An empty TX buffer, or None if all are in flight (reaps once first).
    fn tx_alloc(&mut self) -> Option<TxBuf>;
    /// Queue `len` bytes of `buf` for transmission (frames < 60 B are padded).
    fn tx_submit(&mut self, buf: TxBuf, len: usize) -> Result<(), TxError>;
    /// One doorbell for everything submitted since the last flush.
    fn tx_flush(&mut self);
    /// Return completed TX buffers to the free pool.
    fn reap_tx(&mut self, budget: usize) -> usize;
    fn stats(&self) -> NetStats;
}
