//! Rust heap: power-of-two slab classes (16..2048 bytes) carved from 4 KiB
//! `kmalloc` pages; anything larger (or more strictly aligned) goes straight to
//! C `kmalloc` / `kfree` (page-granularity PMM).
//!
//! Each class keeps an intrusive free list behind an `IrqSpinLock` (allocation
//! may happen with interrupts on or off). The lock is never held across
//! `kmalloc`. Slab pages are never returned to `kmalloc`.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::sync::IrqSpinLock;

extern "C" {
    fn kmalloc(size: usize, flags: u32) -> *mut c_void;
    fn kfree(ptr: *mut c_void);
}

/// Matches `MEM_ALLOC_NORMAL` in `src/includes/memory.h`.
const MEM_ALLOC_NORMAL: u32 = 0;

/// C `kmalloc` rounds to 4 KiB pages (page-aligned); reject larger alignment requests.
const PAGE_SIZE: usize = 4096;

/// Smallest class 16 bytes (holds the free-list link), largest 2048.
const MIN_SHIFT: u32 = 4;
const MAX_SHIFT: u32 = 11;
const NUM_CLASSES: usize = (MAX_SHIFT - MIN_SHIFT + 1) as usize;
const MAX_SLAB: usize = 1 << MAX_SHIFT;

/// Free object header, written into the free object itself.
struct FreeObj {
    next: *mut FreeObj,
}

struct FreeList {
    head: *mut FreeObj,
}

// Safety: the list is only touched under its IrqSpinLock.
unsafe impl Send for FreeList {}

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY_CLASS: IrqSpinLock<FreeList> = IrqSpinLock::new(FreeList { head: ptr::null_mut() });
static CLASSES: [IrqSpinLock<FreeList>; NUM_CLASSES] = [EMPTY_CLASS; NUM_CLASSES];

static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
static SLAB_HITS: AtomicU64 = AtomicU64::new(0);
static LARGE_ALLOCS: AtomicU64 = AtomicU64::new(0);
static PAGES_CARVED: AtomicU64 = AtomicU64::new(0);

/// (total alloc calls, slab-served allocations, large kmalloc allocations, slab pages carved).
#[allow(dead_code)] // for later diagnostics
pub fn stats() -> (u64, u64, u64, u64) {
    (
        ALLOC_CALLS.load(Ordering::Relaxed),
        SLAB_HITS.load(Ordering::Relaxed),
        LARGE_ALLOCS.load(Ordering::Relaxed),
        PAGES_CARVED.load(Ordering::Relaxed),
    )
}

/// Class index for a layout, or None for the large (direct kmalloc) path.
/// Class size = next power of two >= max(size, align, 16); objects sit at
/// multiples of the class size inside page-aligned pages, so they are
/// naturally aligned to it.
#[inline]
fn class_of(layout: &Layout) -> Option<usize> {
    let need = if layout.size() > layout.align() { layout.size() } else { layout.align() };
    if need > MAX_SLAB {
        return None;
    }
    let sz = need.next_power_of_two().max(1 << MIN_SHIFT);
    Some((sz.trailing_zeros() - MIN_SHIFT) as usize)
}

#[inline]
fn class_size(idx: usize) -> usize {
    1usize << (idx as u32 + MIN_SHIFT)
}

/// Pop one object from class `idx`, refilling from a fresh kmalloc page when empty.
/// Returns null on OOM.
unsafe fn slab_alloc(idx: usize) -> *mut u8 {
    loop {
        {
            let mut list = CLASSES[idx].lock();
            let obj = list.head;
            if !obj.is_null() {
                list.head = (*obj).next;
                SLAB_HITS.fetch_add(1, Ordering::Relaxed);
                return obj as *mut u8;
            }
        }
        // Lock dropped: kmalloc runs outside it.
        let page = kmalloc(PAGE_SIZE, MEM_ALLOC_NORMAL) as *mut u8;
        if page.is_null() {
            return ptr::null_mut();
        }
        PAGES_CARVED.fetch_add(1, Ordering::Relaxed);
        let sz = class_size(idx);
        let mut list = CLASSES[idx].lock();
        let mut off = PAGE_SIZE;
        while off >= sz {
            off -= sz;
            let obj = page.add(off) as *mut FreeObj;
            (*obj).next = list.head;
            list.head = obj;
        }
    }
}

unsafe fn slab_free(idx: usize, p: *mut u8) {
    let obj = p as *mut FreeObj;
    let mut list = CLASSES[idx].lock();
    (*obj).next = list.head;
    list.head = obj;
}

pub struct KmallocAlloc;

unsafe impl GlobalAlloc for KmallocAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
        if layout.size() == 0 {
            return ptr::null_mut();
        }
        if layout.align() > PAGE_SIZE {
            return ptr::null_mut();
        }
        match class_of(&layout) {
            Some(idx) => slab_alloc(idx),
            None => {
                LARGE_ALLOCS.fetch_add(1, Ordering::Relaxed);
                // Safety: kmalloc is the kernel heap; size > 0; flags are NORMAL.
                kmalloc(layout.size(), MEM_ALLOC_NORMAL) as *mut u8
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }
        // Same Layout as alloc(), so the class recomputes identically.
        match class_of(&layout) {
            Some(idx) => slab_free(idx, ptr),
            // Safety: ptr came from kmalloc in alloc() above.
            None => kfree(ptr as *mut c_void),
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Avoid rustc's default realloc (needs memcpy from compiler-rt).
        let new_layout = match Layout::from_size_align(new_size, layout.align()) {
            Ok(l) => l,
            Err(_) => return ptr::null_mut(),
        };
        // Same slab class: the object already has room.
        if !ptr.is_null() && layout.size() > 0 && new_size > 0 {
            if let (Some(a), Some(b)) = (class_of(&layout), class_of(&new_layout)) {
                if a == b {
                    return ptr;
                }
            }
        }
        let new_ptr = self.alloc(new_layout);
        if new_ptr.is_null() {
            return ptr::null_mut();
        }
        if !ptr.is_null() && layout.size() > 0 {
            let n = if layout.size() < new_size {
                layout.size()
            } else {
                new_size
            };
            // Byte loop — do not use copy_nonoverlapping (lowers to memcpy).
            let src = ptr;
            let dst = new_ptr;
            for i in 0..n {
                *dst.add(i) = *src.add(i);
            }
            self.dealloc(ptr, layout);
        }
        new_ptr
    }
}

#[global_allocator]
static HEAP: KmallocAlloc = KmallocAlloc;
