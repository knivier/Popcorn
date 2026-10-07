//! C-compatible pop registry helpers.

pub use crate::abi::PopModule;

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
        register_pop_module(core::ptr::addr_of!(super::dolphin::dolphin_module));
    }
}
