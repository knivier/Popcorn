use crate::console_ffi::{
    console_print_color, console_set_cursor, timer_get_ticks, u64_to_dec, CursorGuard,
    COLOR_LIGHT_CYAN,
};
use super::registry::PopModule;
use core::ffi::c_char;

static NAME: &[u8] = b"uptime\0";
static MESSAGE: &[u8] = b"Displays the tick counter\0";

#[no_mangle]
pub extern "C" fn uptime_pop_func(_start_pos: u32) {
    let guard = CursorGuard::save();
    let ticks = unsafe { timer_get_ticks() };
    let mut buf = [0u8; 48];
    let prefix = b"Ticks: ";
    buf[..prefix.len()].copy_from_slice(prefix);
    let avail = buf.len() - 1 - prefix.len();
    let nlen = u64_to_dec(ticks, &mut buf[prefix.len()..prefix.len() + avail]);
    let end = prefix.len() + nlen;
    buf[end] = 0;

    unsafe {
        console_set_cursor(1, 0);
        console_print_color(buf.as_ptr(), COLOR_LIGHT_CYAN);
    }
    guard.restore();
}

#[no_mangle]
pub extern "C" fn get_tick_count() -> u32 {
    unsafe { timer_get_ticks() as u32 }
}

#[no_mangle]
pub static uptime_module: PopModule = PopModule {
    name: NAME.as_ptr() as *const c_char,
    message: MESSAGE.as_ptr() as *const c_char,
    pop_function: Some(uptime_pop_func),
};
