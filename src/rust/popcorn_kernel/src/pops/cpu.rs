use super::registry::PopModule;
use core::ffi::c_char;

static NAME: &[u8] = b"cpu\0";
static MESSAGE: &[u8] = b"CPU detection and frequency monitoring\0";

#[no_mangle]
pub extern "C" fn cpu_pop_func(_start_pos: u32) {
    crate::drivers::backends::cpuinfo::detect_extended();
}

#[no_mangle]
pub static cpu_module: PopModule = PopModule {
    name: NAME.as_ptr() as *const c_char,
    message: MESSAGE.as_ptr() as *const c_char,
    pop_function: Some(cpu_pop_func),
};
