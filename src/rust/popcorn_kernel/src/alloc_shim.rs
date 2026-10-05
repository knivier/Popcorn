//! Heap via C `kmalloc` / `kfree` (page-granularity PMM).

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::ptr;

extern "C" {
    fn kmalloc(size: usize, flags: u32) -> *mut c_void;
    fn kfree(ptr: *mut c_void);
}

/// Matches `MEM_ALLOC_NORMAL` in `src/includes/memory.h`.
const MEM_ALLOC_NORMAL: u32 = 0;

/// C `kmalloc` rounds to 4 KiB pages; reject larger alignment requests.
const PAGE_SIZE: usize = 4096;

pub struct KmallocAlloc;

unsafe impl GlobalAlloc for KmallocAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return ptr::null_mut();
        }
        if layout.align() > PAGE_SIZE {
            return ptr::null_mut();
        }
        // Safety: kmalloc is the kernel heap; size > 0; flags are NORMAL.
        kmalloc(layout.size(), MEM_ALLOC_NORMAL) as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }
        // Safety: ptr came from alloc() / kmalloc above.
        kfree(ptr as *mut c_void);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Avoid rustc's default realloc (needs memcpy from compiler-rt).
        let new_layout = match Layout::from_size_align(new_size, layout.align()) {
            Ok(l) => l,
            Err(_) => return ptr::null_mut(),
        };
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
