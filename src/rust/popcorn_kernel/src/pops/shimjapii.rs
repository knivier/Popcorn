use crate::console_ffi::{
    console_cols, console_print_color, console_rows, console_set_cursor, CursorGuard,
    COLOR_LIGHT_GREEN,
};
use super::registry::PopModule;
use core::ffi::c_char;

static NAME: &[u8] = b"shimjapii\0";
static MESSAGE: &[u8] = b"Shimjapii popped!!!\0";
static DISPLAY: &[u8] = b"Shimjapii popped!!!!\0";

#[no_mangle]
pub extern "C" fn shimjapii_pop_func(_start_pos: u32) {
    let guard = CursorGuard::save();
    let msg_len = DISPLAY.len() - 1;
    let cols = unsafe { console_cols() };
    let rows = unsafe { console_rows() };
    let x = cols.saturating_sub(msg_len as u32).saturating_sub(1);
    let y = rows.saturating_sub(2);
    unsafe {
        console_set_cursor(x, y);
        console_print_color(DISPLAY.as_ptr(), COLOR_LIGHT_GREEN);
    }
    guard.restore();
}

#[no_mangle]
pub static shimjapii_module: PopModule = PopModule {
    name: NAME.as_ptr() as *const c_char,
    message: MESSAGE.as_ptr() as *const c_char,
    pop_function: Some(shimjapii_pop_func),
};
