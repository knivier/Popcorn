//! Shared console / timer FFI for drivers and pops.

#![allow(dead_code)]

pub const COLOR_WHITE: u8 = 0x0F;
pub const COLOR_LIGHT_GREEN: u8 = 0x0A;
pub const COLOR_LIGHT_CYAN: u8 = 0x0B;
pub const COLOR_YELLOW: u8 = 0x0E;
pub const COLOR_LIGHT_RED: u8 = 0x0C;
pub const COLOR_LIGHT_MAGENTA: u8 = 0x0D;

extern "C" {
    pub fn console_print(s: *const u8);
    pub fn console_print_color(s: *const u8, color: u8);
    pub fn console_println(s: *const u8);
    pub fn console_println_color(s: *const u8, color: u8);
    pub fn console_newline();
    pub fn console_draw_separator(y: u32, color: u8);
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

pub fn cstr(s: &[u8]) -> *const u8 {
    s.as_ptr()
}

pub fn print(s: &str) {
    let mut buf = [0u8; 160];
    let n = s.as_bytes().len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf[n] = 0;
    unsafe { console_print(buf.as_ptr()) }
}

pub fn print_color(s: &str, color: u8) {
    let mut buf = [0u8; 160];
    let n = s.as_bytes().len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf[n] = 0;
    unsafe { console_print_color(buf.as_ptr(), color) }
}

pub fn println_color(s: &str, color: u8) {
    let mut buf = [0u8; 160];
    let n = s.as_bytes().len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf[n] = 0;
    unsafe { console_println_color(buf.as_ptr(), color) }
}

pub fn separator() {
    let mut y = 0u32;
    let mut x = 0u32;
    unsafe {
        console_get_cursor(&mut x, &mut y);
        console_draw_separator(y, COLOR_WHITE);
    }
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

pub struct CursorGuard {
    x: u32,
    y: u32,
    color: u8,
}

impl CursorGuard {
    pub fn save() -> Self {
        let mut x = 0u32;
        let mut y = 0u32;
        unsafe {
            console_get_cursor(&mut x, &mut y);
            Self {
                x,
                y,
                color: console_get_color(),
            }
        }
    }

    pub fn restore(self) {
        unsafe {
            console_set_color(self.color);
            console_set_cursor(self.x, self.y);
        }
    }
}
