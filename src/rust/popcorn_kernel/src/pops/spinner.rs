use crate::console_ffi::{
    console_cols, console_print_color, console_set_cursor, util_delay, CursorGuard, COLOR_YELLOW,
};
use super::registry::PopModule;
use core::ffi::c_char;
use core::sync::atomic::{AtomicUsize, Ordering};

static NAME: &[u8] = b"spinner\0";
static MESSAGE: &[u8] = b"Spinning loader animation\0";
static LABEL: &[u8] = b"Running... \0";
static SPINNER: [u8; 4] = [b'|', b'/', b'-', b'\\'];
static STATE: AtomicUsize = AtomicUsize::new(0);

#[no_mangle]
pub extern "C" fn spinner_pop_func(_start_pos: u32) {
    let guard = CursorGuard::save();
    let cols = unsafe { console_cols() };
    let msg_len = (LABEL.len() - 1) as u32;
    let msg_x = cols.saturating_sub(msg_len).saturating_sub(2);
    let y = 0u32;
    let idx = STATE.load(Ordering::Relaxed) % 4;
    let mut ch = [SPINNER[idx], 0u8];

    unsafe {
        console_set_cursor(msg_x, y);
        console_print_color(LABEL.as_ptr(), COLOR_YELLOW);
        console_set_cursor(msg_x + msg_len, y);
        console_print_color(ch.as_mut_ptr(), COLOR_YELLOW);
        util_delay(10);
    }

    guard.restore();
    STATE.store((idx + 1) % 4, Ordering::Relaxed);
}

#[no_mangle]
pub static spinner_module: PopModule = PopModule {
    name: NAME.as_ptr() as *const c_char,
    message: MESSAGE.as_ptr() as *const c_char,
    pop_function: Some(spinner_pop_func),
};
