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
    /// No interrupt: the driver polls (fallback, or MSI-X unavailable).
    Polled,
}

/// A driver's view of one interrupt source.
pub struct IrqHandle {
    vector: u8,
    source: IrqSource,
}

impl IrqHandle {
    pub const fn polled() -> IrqHandle {
        IrqHandle { vector: 0, source: IrqSource::Polled }
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

/// Bind an allocated vector to a source (the grant layer programs the MSI-X entry).
pub(in crate::drivers) fn handle_for(vector: u8, source: IrqSource) -> IrqHandle {
    IrqHandle { vector, source }
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
