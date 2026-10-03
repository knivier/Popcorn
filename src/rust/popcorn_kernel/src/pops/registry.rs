//! C-compatible pop registry helpers.

use core::ffi::c_char;

/// Matches `PopModule` in `src/includes/pop_module.h`.
#[repr(C)]
pub struct PopModule {
    pub name: *const c_char,
    pub message: *const c_char,
    pub pop_function: Option<extern "C" fn(u32)>,
}

// SAFETY: PopModule is only ever created as immortal statics with C-string literals.
unsafe impl Sync for PopModule {}

extern "C" {
    fn register_pop_module(module: *const PopModule);
}

pub fn pop_register_all() {
    unsafe {
        register_pop_module(core::ptr::addr_of!(super::shimjapii::shimjapii_module));
        register_pop_module(core::ptr::addr_of!(super::spinner::spinner_module));
        register_pop_module(core::ptr::addr_of!(super::uptime::uptime_module));
        register_pop_module(core::ptr::addr_of!(super::memory::memory_module));
        register_pop_module(core::ptr::addr_of!(super::cpu::cpu_module));
        register_pop_module(core::ptr::addr_of!(super::sysinfo::sysinfo_module));
    }
}
