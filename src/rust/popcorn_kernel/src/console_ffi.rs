//! Console writing for Rust kernel code.
//!
//! Prefer the safe helpers (`print`, `println_color`, `error`, …). Raw `extern "C"`
//! bindings stay available for NUL-terminated byte literals and CursorGuard.

#![allow(dead_code)]

/* VGA attribute colors (match console.h). */
pub const COLOR_WHITE: u8 = 0x0F;
pub const COLOR_LIGHT_GREEN: u8 = 0x0A;
pub const COLOR_LIGHT_CYAN: u8 = 0x0B;
pub const COLOR_YELLOW: u8 = 0x0E;
pub const COLOR_LIGHT_RED: u8 = 0x0C;
pub const COLOR_LIGHT_MAGENTA: u8 = 0x0D;

pub const COLOR_ERROR: u8 = COLOR_LIGHT_RED;
pub const COLOR_SUCCESS: u8 = COLOR_LIGHT_GREEN;
pub const COLOR_INFO: u8 = COLOR_LIGHT_CYAN;
pub const COLOR_WARNING: u8 = COLOR_YELLOW;
pub const COLOR_HEADER: u8 = COLOR_LIGHT_MAGENTA;
pub const COLOR_PROMPT: u8 = COLOR_LIGHT_GREEN;

extern "C" {
    pub fn console_print(s: *const u8);
    pub fn console_print_color(s: *const u8, color: u8);
    pub fn console_println(s: *const u8);
    pub fn console_println_color(s: *const u8, color: u8);
    pub fn console_newline();
    pub fn console_putchar(c: core::ffi::c_char);
    pub fn console_backspace();
    pub fn console_clear();
    pub fn console_draw_separator(y: u32, color: u8);
    pub fn console_draw_header(title: *const u8);
    pub fn console_draw_prompt_with_path(path: *const u8);
    pub fn console_print_error(message: *const u8);
    pub fn console_print_success(message: *const u8);
    pub fn console_print_info(message: *const u8);
    pub fn console_print_warning(message: *const u8);
    pub fn console_set_color(color: u8);
    pub fn console_get_color() -> u8;
    pub fn console_set_cursor(x: u32, y: u32);
    pub fn console_get_cursor(x: *mut u32, y: *mut u32);
    pub fn console_cols() -> u32;
    pub fn console_rows() -> u32;
    pub fn util_delay(ms: u32);
    pub fn timer_get_ticks() -> u64;
    pub fn timer_get_uptime_ms() -> u64;
}

const CSTR_CAP: usize = 256;

/// Copy `s` into a stack NUL buffer and call `f` with the pointer.
fn with_cstr<R>(s: &str, f: impl FnOnce(*const u8) -> R) -> R {
    let mut buf = [0u8; CSTR_CAP];
    let n = s.as_bytes().len().min(CSTR_CAP - 1);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf[n] = 0;
    f(buf.as_ptr())
}

pub fn cstr(s: &[u8]) -> *const u8 {
    s.as_ptr()
}

pub fn print(s: &str) {
    with_cstr(s, |p| unsafe { console_print(p) });
}

pub fn print_color(s: &str, color: u8) {
    with_cstr(s, |p| unsafe { console_print_color(p, color) });
}

pub fn println(s: &str) {
    with_cstr(s, |p| unsafe { console_println(p) });
}

pub fn println_color(s: &str, color: u8) {
    with_cstr(s, |p| unsafe { console_println_color(p, color) });
}

pub fn newline() {
    unsafe { console_newline() }
}

pub fn putchar(c: char) {
    unsafe { console_putchar(c as core::ffi::c_char) }
}

pub fn backspace() {
    unsafe { console_backspace() }
}

pub fn clear() {
    unsafe { console_clear() }
}

pub fn set_cursor(x: u32, y: u32) {
    unsafe { console_set_cursor(x, y) }
}

pub fn get_cursor() -> (u32, u32) {
    let mut x = 0u32;
    let mut y = 0u32;
    unsafe { console_get_cursor(&mut x, &mut y) };
    (x, y)
}

pub fn cols() -> u32 {
    unsafe { console_cols() }
}

pub fn rows() -> u32 {
    unsafe { console_rows() }
}

pub fn separator() {
    let (_, y) = get_cursor();
    unsafe { console_draw_separator(y, COLOR_WHITE) };
}

pub fn separator_at(y: u32) {
    unsafe { console_draw_separator(y, COLOR_WHITE) };
}

pub fn header(title: &str) {
    with_cstr(title, |p| unsafe { console_draw_header(p) });
}

pub fn prompt_with_path(path: &str) {
    with_cstr(path, |p| unsafe { console_draw_prompt_with_path(p) });
}

pub fn error(message: &str) {
    with_cstr(message, |p| unsafe { console_print_error(p) });
}

pub fn success(message: &str) {
    with_cstr(message, |p| unsafe { console_print_success(p) });
}

pub fn info(message: &str) {
    with_cstr(message, |p| unsafe { console_print_info(p) });
}

pub fn warning(message: &str) {
    with_cstr(message, |p| unsafe { console_print_warning(p) });
}

pub fn u64_to_dec(mut n: u64, out: &mut [u8]) -> usize {
    if out.is_empty() {
        return 0;
    }
    if n == 0 {
        out[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut i = 0usize;
    while n > 0 && i < tmp.len() {
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    let mut o = 0usize;
    while i > 0 && o < out.len() {
        i -= 1;
        out[o] = tmp[i];
        o += 1;
    }
    o
}

pub fn print_u32(v: u32, color: u8) {
    let mut num = [0u8; 12];
    let n = u64_to_dec(v as u64, &mut num);
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), color);
}

/// Saves cursor + color; restores on `restore()`.
pub struct CursorGuard {
    x: u32,
    y: u32,
    color: u8,
}

impl CursorGuard {
    pub fn save() -> Self {
        let (x, y) = get_cursor();
        Self {
            x,
            y,
            color: unsafe { console_get_color() },
        }
    }

    pub fn restore(self) {
        unsafe {
            console_set_color(self.color);
            console_set_cursor(self.x, self.y);
        }
    }
}
