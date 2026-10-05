use super::registry::PopModule;
use core::ffi::c_char;

static NAME: &[u8] = b"memory\0";
static MESSAGE: &[u8] = b"Memory management and statistics\0";

#[no_mangle]
pub extern "C" fn memory_pop_func(_start_pos: u32) {
    // Ensure mem drive stats are warm; shell uses memory_print_*.
    crate::drivers::backends::meminfo::calculate_stats();
}

#[no_mangle]
pub static memory_module: PopModule = PopModule {
    name: NAME.as_ptr() as *const c_char,
    message: MESSAGE.as_ptr() as *const c_char,
    pop_function: Some(memory_pop_func),
};
