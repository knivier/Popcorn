//! The universal device queue (driver-framework decision 2): submit → kick → reap.
//!
//! Every device class (block, net, usb, display) presents its hardware queues
//! through this one shape:
//! * **submit** stages one request and never rings the doorbell or allocates;
//! * **kick** publishes everything staged with one index store and rings the
//!   doorbell only if the device asked for notifications;
//! * **reap** hands back at most `budget` completions, then the caller may **arm**
//!   (ask for a completion interrupt and re-check) or keep polling.
//!
//! virtio: desc + avail = submit, notify = kick, used ring = reap.
//! NVMe: SQ entry = submit, SQ tail doorbell = kick, CQ phase bit = reap.
//! xHCI: TRB + cycle bit = submit, doorbell = kick, event ring = reap.
//!
//! Pure `core` (no kernel dependencies) so the host test crate can include it.

/// Address as the *device* sees it. Today equal to the physical address; an IOVA
/// once an IOMMU exists. Deliberately has no conversion to a pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(transparent)]
pub struct DeviceAddr(pub u64);

impl DeviceAddr {
    #[inline]
    pub const fn get(self) -> u64 {
        self.0
    }
    #[inline]
    pub const fn offset(self, bytes: u64) -> DeviceAddr {
        DeviceAddr(self.0 + bytes)
    }
}

/// Max completions one `reap` may deliver (bounds critical sections).
#[derive(Clone, Copy, Debug)]
pub struct Budget(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QStatus {
    Ok,
    Err(u16),
}

#[derive(Clone, Copy, Debug)]
pub struct Completion<C: Copy> {
    pub cookie: C,
    /// Bytes the device wrote (virtio used.len), 0 if not applicable.
    pub len: u32,
    pub status: QStatus,
}

#[derive(Debug)]
pub struct QueueFull;

/// One scatter segment of a request; lives on the caller's stack.
#[derive(Clone, Copy, Debug)]
pub struct Seg {
    pub addr: DeviceAddr,
    pub len: u32,
    /// true = the device writes this segment (RX buffer, block read data).
    pub device_writes: bool,
}

pub trait Queue {
    /// Per-request tag returned with its completion (slot id, request tag...).
    type Cookie: Copy;

    fn capacity(&self) -> u16;
    fn free_slots(&self) -> u16;
    /// Stage one request. Never rings the doorbell. Never allocates.
    fn submit(&mut self, segs: &[Seg], cookie: Self::Cookie) -> Result<(), QueueFull>;
    /// Publish everything staged; notify only if the device wants it.
    /// Returns true if the doorbell was written.
    fn kick(&mut self) -> bool;
    /// Deliver at most `budget` completions to `f`. Returns how many were delivered.
    fn reap<F: FnMut(Completion<Self::Cookie>)>(&mut self, budget: Budget, f: F) -> u16
    where
        Self: Sized;
    /// Ask for a completion interrupt, then re-check: true = work raced in, reap again.
    fn arm(&mut self) -> bool;
    /// Stop asking for completion interrupts (polling).
    fn disarm(&mut self);
}
