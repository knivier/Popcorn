//! Interrupt grants: the vector table, the vector allocator and `IrqHandle`.
//!
//! Every vector already has an IDT stub (idt.asm builds a PopFrame for all 256).
//! For vectors 0x20..0xFE (except the syscall / yield / spurious vectors) C's
//! `isr_dispatch` calls [`rust_vector_dispatch`], which counts the interrupt and
//! marks it pending; C then EOIs (PIC for 0x20-0x2F, LAPIC above). ISRs never touch
//! driver state: a driver consumes `pending` with [`IrqHandle::take`] in its pump.
//!
//! Vector map:
//!   0x00-0x1F exceptions · 0x20-0x2F 8259 PIC · 0x30-0x3F kernel (0x30 = LAPIC
//!   self-test) · 0x40-0xEF device pool (excluding 0x80 syscall and 0x81 yield) ·
//!   0xFE LAPIC error · 0xFF spurious (no EOI)

use core::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};

use crate::sync::IrqSpinLock;

pub const VEC_SELFTEST: u8 = 0x30;
pub const VEC_POOL_FIRST: u8 = 0x40;
pub const VEC_POOL_LAST: u8 = 0xEF;
const VEC_SYSCALL: u8 = 0x80;
const VEC_YIELD: u8 = 0x81;

pub struct IrqSlot {
    pending: AtomicU32,
    count: AtomicU64,
    /// 0 = free, 1 = kernel-reserved, 2 = device-owned.
    state: AtomicU8,
}

impl IrqSlot {
    const fn new() -> IrqSlot {
        IrqSlot { pending: AtomicU32::new(0), count: AtomicU64::new(0), state: AtomicU8::new(0) }
    }
}

const SLOT_INIT: IrqSlot = IrqSlot::new();
static SLOTS: [IrqSlot; 256] = [SLOT_INIT; 256];
/// Owner names for `irq` listings; written only when a vector is (un)bound.
static OWNERS: IrqSpinLock<[&'static str; 256]> = IrqSpinLock::new([""; 256]);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IrqSource {
    /// MSI-X table entry (kernel-owned table, see bus::pci).
    MsiX { entry: u16 },
    /// Plain MSI message `index` of a multi-message block (bus::msi).
    Msi { index: u8 },
    /// No interrupt: the driver polls (fallback, or MSI-X unavailable).
    Polled,
}

/// How a handle masks its source. Kernel-owned: the driver never sees the table
/// or the config register behind it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::drivers) enum MaskCtl {
    /// No per-source mask (plain MSI without per-vector masking): mask/unmask
    /// are no-ops and the driver relies on its device's own interrupt enables.
    None,
    /// VA of the MSI-X entry's vector-control dword.
    MsixEntry(usize),
    /// Plain MSI per-vector masking: bit `bit` of the config dword at `off`.
    MsiMaskBit { bus: u8, slot: u8, func: u8, off: u8, bit: u8 },
}

/// A driver's view of one interrupt source.
pub struct IrqHandle {
    vector: u8,
    source: IrqSource,
    ctl: MaskCtl,
}

impl IrqHandle {
    pub const fn polled() -> IrqHandle {
        IrqHandle { vector: 0, source: IrqSource::Polled, ctl: MaskCtl::None }
    }
    fn set_masked(&self, masked: bool) {
        match self.ctl {
            MaskCtl::None => {}
            MaskCtl::MsixEntry(va) => unsafe { core::ptr::write_volatile(va as *mut u32, masked as u32) },
            MaskCtl::MsiMaskBit { bus, slot, func, off, bit } => {
                crate::drivers::bus::pci::cfg_rmw32(bus, slot, func, off, |v| {
                    if masked {
                        v | (1 << bit)
                    } else {
                        v & !(1 << bit)
                    }
                });
            }
        }
    }
    /// Stop this source from interrupting (MSI-X vector control / MSI mask bit).
    pub fn mask(&self) {
        self.set_masked(true);
    }
    /// Allow this source to interrupt again.
    pub fn unmask(&self) {
        self.set_masked(false);
    }
    /// Whether mask/unmask actually gate this source.
    pub fn can_mask(&self) -> bool {
        self.ctl != MaskCtl::None
    }
    /// Pending without consuming it.
    pub fn is_pending(&self) -> bool {
        !self.is_polled() && SLOTS[self.vector as usize].pending.load(Ordering::Acquire) != 0
    }
    pub fn is_polled(&self) -> bool {
        self.source == IrqSource::Polled
    }
    pub fn vector(&self) -> Option<u8> {
        if self.is_polled() {
            None
        } else {
            Some(self.vector)
        }
    }
    pub fn source(&self) -> IrqSource {
        self.source
    }
    /// Consume a pending interrupt (true if one arrived since the last take).
    pub fn take(&self) -> bool {
        !self.is_polled() && SLOTS[self.vector as usize].pending.swap(0, Ordering::Acquire) != 0
    }
    pub fn count(&self) -> u64 {
        if self.is_polled() {
            0
        } else {
            SLOTS[self.vector as usize].count.load(Ordering::Relaxed)
        }
    }
}

impl Drop for IrqHandle {
    fn drop(&mut self) {
        if !self.is_polled() {
            self.mask();
            free_vector(self.vector);
        }
    }
}

fn poolable(v: u8) -> bool {
    (VEC_POOL_FIRST..=VEC_POOL_LAST).contains(&v) && v != VEC_SYSCALL && v != VEC_YIELD
}

/// Reserve a device vector. The IDT gate never changes; only ownership does.
pub fn alloc_vector(owner: &'static str) -> Option<u8> {
    let mut names = OWNERS.lock();
    for v in VEC_POOL_FIRST..=VEC_POOL_LAST {
        if poolable(v) && SLOTS[v as usize].state.compare_exchange(0, 2, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
            SLOTS[v as usize].pending.store(0, Ordering::Relaxed);
            names[v as usize] = owner;
            return Some(v);
        }
    }
    None
}

/// Reserve `n` (a power of two, 1..=32) consecutive device vectors starting at a
/// multiple of `n`: plain MSI multi-message devices put the message index in the
/// low bits of the data word, so the block must be naturally aligned.
pub fn alloc_vector_block(n: u8, owner: &'static str) -> Option<u8> {
    let mut names = OWNERS.lock();
    let base = crate::drivers::bus::msi_math::find_aligned_block(VEC_POOL_FIRST, VEC_POOL_LAST, n, |v| {
        poolable(v) && SLOTS[v as usize].state.load(Ordering::Acquire) == 0
    })?;
    for v in base..base + n {
        // Under OWNERS, so no other allocator can race us between check and claim.
        SLOTS[v as usize].state.store(2, Ordering::Release);
        SLOTS[v as usize].pending.store(0, Ordering::Relaxed);
        names[v as usize] = owner;
    }
    Some(base)
}

/// Bind an allocated vector to a source (the grant layer programs the MSI-X entry
/// or MSI capability and says how the source is masked).
pub(in crate::drivers) fn handle_for(vector: u8, source: IrqSource, ctl: MaskCtl) -> IrqHandle {
    IrqHandle { vector, source, ctl }
}

fn free_vector(v: u8) {
    let mut names = OWNERS.lock();
    SLOTS[v as usize].state.store(0, Ordering::Release);
    names[v as usize] = "";
}

/// Mark kernel vectors (self-test) so the allocator never hands them out.
pub fn reserve_kernel_vector(v: u8, owner: &'static str) {
    SLOTS[v as usize].state.store(1, Ordering::Release);
    OWNERS.lock()[v as usize] = owner;
}

/// Called by C `isr_dispatch` with interrupts off. Never touches driver state.
/// Returns 1 if the vector is owned (kernel or device), 0 if nobody claimed it.
#[no_mangle]
pub extern "C" fn rust_vector_dispatch(vector: u64) -> u32 {
    let s = &SLOTS[(vector & 0xFF) as usize];
    s.count.fetch_add(1, Ordering::Relaxed);
    s.pending.store(1, Ordering::Release);
    (s.state.load(Ordering::Relaxed) != 0) as u32
}

pub fn vector_count(v: u8) -> u64 {
    SLOTS[v as usize].count.load(Ordering::Relaxed)
}

/// (vector, owner, count) for every vector that has an owner or has fired.
pub fn for_each_active(mut f: impl FnMut(u8, &'static str, u64)) {
    let names = *OWNERS.lock();
    for v in 0x20u16..=0xFE {
        let c = SLOTS[v as usize].count.load(Ordering::Relaxed);
        let o = names[v as usize];
        if c != 0 || !o.is_empty() {
            f(v as u8, o, c);
        }
    }
}
