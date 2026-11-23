//! Locks for Popcorn's single-CPU, preemptive kernel (shapes that stay valid on SMP).
//!
//! * [`IrqSpinLock`]: short critical sections. Interrupts are off while it is held,
//!   so nothing can preempt the holder. On one CPU that means a lock found already
//!   held can only be *re-entry* by the same code path: it panics (naming both call
//!   sites) instead of spinning forever with interrupts off. Never hold it across
//!   device I/O waits; the timer would stop ticking.
//! * [`TaskLock`]: long sections (filesystem, device I/O). Interrupts stay on; a
//!   contender yields to the scheduler until the holder (a task) releases it.
//!   Re-entry by the holder panics; a holder that was killed is detected and the
//!   lock is reclaimed.
//! * [`Once`]: one-time initialisation.
//!
//! New code must not add `static mut`.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::panic::Location;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU8, Ordering};

extern "C" {
    fn scheduler_yield();
    fn scheduler_current_pid() -> u32;
    fn scheduler_pid_alive(pid: u32) -> i32;
}

/// Save RFLAGS and disable interrupts (no `nomem`: it orders memory accesses too).
#[inline]
pub fn irq_save_disable() -> u64 {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq", "pop {}", "cli", out(reg) flags, options(preserves_flags));
    }
    flags
}

/// Re-enable interrupts only if they were on when `flags` was saved.
#[inline]
pub fn irq_restore(flags: u64) {
    if flags & 0x200 != 0 {
        unsafe { core::arch::asm!("sti", options(nostack)) };
    }
}

/// True when interrupts are enabled (RFLAGS.IF).
#[inline]
pub fn irqs_enabled() -> bool {
    let flags: u64;
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(preserves_flags)) };
    flags & 0x200 != 0
}

extern "C" {
    fn timer_poll();
    fn timer_is_poll_mode() -> bool;
}

/// Kernel service for drivers waiting on a device: sleep until the next interrupt.
///
/// `arm` runs with interrupts disabled; it should request a completion interrupt
/// and return true if work already raced in (then we do not sleep). Otherwise the
/// CPU executes `sti; hlt` as one step (sti's shadow), so an interrupt arriving
/// right after arming still wakes it. Falls back to a short poll when interrupts
/// are disabled or the PIT is in poll mode. Drivers never issue cli/hlt themselves.
pub fn wait_for_interrupt(arm: impl FnOnce() -> bool) {
    unsafe {
        if timer_is_poll_mode() {
            timer_poll();
            core::hint::spin_loop();
            return;
        }
        if !irqs_enabled() {
            core::hint::spin_loop();
            return;
        }
        core::arch::asm!("cli", options(nostack));
        if arm() {
            core::arch::asm!("sti", options(nostack));
        } else {
            core::arch::asm!("sti; hlt", options(nostack));
        }
    }
}

// ---------------------------------------------------------------- IrqSpinLock

pub struct IrqSpinLock<T> {
    locked: AtomicBool,
    holder: AtomicPtr<Location<'static>>,
    value: UnsafeCell<T>,
}

// Safety: access to `value` is serialised by `locked` with interrupts off.
unsafe impl<T: Send> Sync for IrqSpinLock<T> {}
unsafe impl<T: Send> Send for IrqSpinLock<T> {}

impl<T> IrqSpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self { locked: AtomicBool::new(false), holder: AtomicPtr::new(ptr::null_mut()), value: UnsafeCell::new(value) }
    }

    #[track_caller]
    #[inline]
    pub fn lock(&self) -> IrqGuard<'_, T> {
        let flags = irq_save_disable();
        if self.locked.swap(true, Ordering::Acquire) {
            // One CPU + IRQs off: only the current code path can be holding it.
            let held = self.holder.load(Ordering::Relaxed);
            reentry_panic("IrqSpinLock", Location::caller(), unsafe { held.as_ref() });
        }
        self.holder.store(Location::caller() as *const _ as *mut _, Ordering::Relaxed);
        IrqGuard { lock: self, flags }
    }

    /// Non-blocking: None if held (e.g. from a path that may already hold it).
    #[track_caller]
    pub fn try_lock(&self) -> Option<IrqGuard<'_, T>> {
        let flags = irq_save_disable();
        if self.locked.swap(true, Ordering::Acquire) {
            irq_restore(flags);
            return None;
        }
        self.holder.store(Location::caller() as *const _ as *mut _, Ordering::Relaxed);
        Some(IrqGuard { lock: self, flags })
    }

    /// Run `f` with the value locked.
    #[track_caller]
    #[inline]
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut g = self.lock();
        f(&mut g)
    }

    /// Bypass the lock. Only for the panic path, where the holder will never run again.
    ///
    /// # Safety
    /// Nothing else may be using the value.
    pub unsafe fn force_get(&self) -> &mut T {
        &mut *self.value.get()
    }

    /// Nobody holds it right now (panic path: with interrupts off on one CPU the
    /// answer cannot change before the caller takes it).
    pub fn is_free(&self) -> bool {
        !self.locked.load(Ordering::Acquire)
    }
}

pub struct IrqGuard<'a, T> {
    lock: &'a IrqSpinLock<T>,
    flags: u64,
}

impl<T> Deref for IrqGuard<'_, T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for IrqGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for IrqGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.holder.store(ptr::null_mut(), Ordering::Relaxed);
        self.lock.locked.store(false, Ordering::Release);
        irq_restore(self.flags);
    }
}

// ------------------------------------------------------------------- TaskLock

/// Owner encoding: 0 = free, otherwise pid + 1.
pub struct TaskLock<T> {
    owner: AtomicU32,
    holder: AtomicPtr<Location<'static>>,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for TaskLock<T> {}
unsafe impl<T: Send> Send for TaskLock<T> {}

impl<T> TaskLock<T> {
    pub const fn new(value: T) -> Self {
        Self { owner: AtomicU32::new(0), holder: AtomicPtr::new(ptr::null_mut()), value: UnsafeCell::new(value) }
    }

    #[track_caller]
    pub fn lock(&self) -> TaskGuard<'_, T> {
        let pid = unsafe { scheduler_current_pid() };
        let me = pid.wrapping_add(1);
        loop {
            match self.owner.compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => break,
                Err(o) if o == me => {
                    let held = self.holder.load(Ordering::Relaxed);
                    reentry_panic("TaskLock", Location::caller(), unsafe { held.as_ref() });
                }
                Err(o) => {
                    // A killed holder never releases: reclaim its lock.
                    if unsafe { scheduler_pid_alive(o.wrapping_sub(1)) } == 0
                        && self.owner.compare_exchange(o, me, Ordering::Acquire, Ordering::Relaxed).is_ok()
                    {
                        break;
                    }
                    unsafe { scheduler_yield() };
                }
            }
        }
        self.holder.store(Location::caller() as *const _ as *mut _, Ordering::Relaxed);
        TaskGuard { lock: self }
    }

    #[track_caller]
    pub fn try_lock(&self) -> Option<TaskGuard<'_, T>> {
        let me = unsafe { scheduler_current_pid() }.wrapping_add(1);
        if self.owner.compare_exchange(0, me, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return None;
        }
        self.holder.store(Location::caller() as *const _ as *mut _, Ordering::Relaxed);
        Some(TaskGuard { lock: self })
    }

    #[track_caller]
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut g = self.lock();
        f(&mut g)
    }

    /// # Safety
    /// Panic path only: nothing else may be using the value.
    pub unsafe fn force_get(&self) -> &mut T {
        &mut *self.value.get()
    }

    /// Nobody holds it right now (see `IrqSpinLock::is_free`).
    pub fn is_free(&self) -> bool {
        self.owner.load(Ordering::Acquire) == 0
    }
}

pub struct TaskGuard<'a, T> {
    lock: &'a TaskLock<T>,
}

impl<T> Deref for TaskGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for TaskGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for TaskGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.holder.store(ptr::null_mut(), Ordering::Relaxed);
        self.lock.owner.store(0, Ordering::Release);
    }
}

// ----------------------------------------------------------------------- Once

const ONCE_EMPTY: u8 = 0;
const ONCE_BUSY: u8 = 1;
const ONCE_READY: u8 = 2;

pub struct Once<T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Send + Sync> Sync for Once<T> {}
unsafe impl<T: Send> Send for Once<T> {}

impl<T> Once<T> {
    pub const fn new() -> Self {
        Self { state: AtomicU8::new(ONCE_EMPTY), value: UnsafeCell::new(MaybeUninit::uninit()) }
    }

    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == ONCE_READY {
            Some(unsafe { (*self.value.get()).assume_init_ref() })
        } else {
            None
        }
    }

    /// Initialise once; later calls return the stored value. Re-entry from `f` panics.
    #[track_caller]
    pub fn call_once(&self, f: impl FnOnce() -> T) -> &T {
        match self.state.compare_exchange(ONCE_EMPTY, ONCE_BUSY, Ordering::Acquire, Ordering::Acquire) {
            Ok(_) => {
                unsafe { (*self.value.get()).write(f()) };
                self.state.store(ONCE_READY, Ordering::Release);
            }
            Err(ONCE_BUSY) => reentry_panic("Once", Location::caller(), None),
            Err(_) => {}
        }
        unsafe { (*self.value.get()).assume_init_ref() }
    }
}

#[cold]
#[inline(never)]
fn reentry_panic(kind: &str, at: &Location<'_>, held_by: Option<&Location<'_>>) -> ! {
    match held_by {
        Some(h) => panic!("{} re-entered at {}:{} (held since {}:{})", kind, at.file(), at.line(), h.file(), h.line()),
        None => panic!("{} re-entered at {}:{}", kind, at.file(), at.line()),
    }
}
