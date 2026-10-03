#![no_std]

extern crate alloc;

mod alloc_shim;
mod drivers;

use alloc::boxed::Box;
use core::ffi::c_char;
use core::slice;

extern "C" {
    fn console_println_color(s: *const u8, color: u8);
    fn boot_serial_putc(c: u8);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

fn cstr<'a>(p: *const c_char) -> &'a str {
    if p.is_null() {
        return "";
    }
    unsafe {
        let mut len = 0usize;
        while *p.add(len) != 0 {
            len += 1;
            if len > 256 {
                break;
            }
        }
        core::str::from_utf8_unchecked(slice::from_raw_parts(p as *const u8, len))
    }
}

#[no_mangle]
pub extern "C" fn rust_init() {
    const MSG: &[u8] = b"Rust active\0";
    const COLOR_LIGHT_GREEN: u8 = 0x0A;
    unsafe {
        console_println_color(MSG.as_ptr(), COLOR_LIGHT_GREEN);
        boot_serial_putc(b'r');
    }

    let b = Box::new(0xA11Cu32);
    if *b == 0xA11Cu32 {
        unsafe {
            boot_serial_putc(b'a');
        }
    }
    drop(b);

    init_drives();
}

#[no_mangle]
pub extern "C" fn init_drives() {
    drivers::init_drives();
}

#[no_mangle]
pub extern "C" fn init_drive(name: *const c_char) -> i32 {
    match drivers::init_drive(cstr(name)) {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn list_drives(buf: *mut u8, buflen: usize) -> i32 {
    if buf.is_null() || buflen == 0 {
        return 0;
    }
    let slice = unsafe { slice::from_raw_parts_mut(buf, buflen) };
    drivers::list_drives(slice) as i32
}

#[no_mangle]
pub extern "C" fn list_devices(buf: *mut u8, buflen: usize) -> i32 {
    if buf.is_null() || buflen == 0 {
        return 0;
    }
    let slice = unsafe { slice::from_raw_parts_mut(buf, buflen) };
    drivers::list_devices(slice) as i32
}

#[no_mangle]
pub extern "C" fn drive_cmd(
    target: *const c_char,
    cmd: *const c_char,
    buf: *mut u8,
    buflen: usize,
) -> i32 {
    if buf.is_null() || buflen == 0 {
        return 0;
    }
    let slice = unsafe { slice::from_raw_parts_mut(buf, buflen) };
    drivers::drive_cmd(cstr(target), cstr(cmd), slice) as i32
}

#[no_mangle]
pub extern "C" fn rust_device_read(name: *const c_char, buf: *mut u8, count: usize) -> i64 {
    if buf.is_null() {
        return -2;
    }
    let slice = unsafe { slice::from_raw_parts_mut(buf, count) };
    drivers::device_read(cstr(name), slice)
}

#[no_mangle]
pub extern "C" fn rust_device_write(name: *const c_char, buf: *const u8, count: usize) -> i64 {
    if buf.is_null() {
        return -2;
    }
    let slice = unsafe { slice::from_raw_parts(buf, count) };
    drivers::device_write(cstr(name), slice)
}

#[no_mangle]
pub extern "C" fn rust_device_ioctl(name: *const c_char, request: u64, argp: *mut u8) -> i64 {
    drivers::device_ioctl(cstr(name), request, argp)
}

#[no_mangle]
pub extern "C" fn rust_screen_set_cursor(x: u32, y: u32) {
    drivers::screen_set_cursor(x, y);
}

#[no_mangle]
pub extern "C" fn rust_screen_set_cursor_visible(visible: i32) {
    drivers::screen_set_cursor_visible(visible);
}

#[no_mangle]
pub extern "C" fn rust_screen_write_cell(x: u32, y: u32, ch: u8, attr: u8) {
    drivers::screen_write_cell(x, y, ch, attr);
}

#[no_mangle]
pub extern "C" fn rust_screen_backend() -> i32 {
    drivers::screen_backend()
}

#[no_mangle]
pub extern "C" fn rust_screen_cols() -> u32 {
    drivers::screen_cols()
}

#[no_mangle]
pub extern "C" fn rust_screen_rows() -> u32 {
    drivers::screen_rows()
}

#[no_mangle]
pub extern "C" fn rust_screen_cells() -> *mut u8 {
    drivers::cells_ptr()
}

#[no_mangle]
pub extern "C" fn rust_screen_init_vga() {
    drivers::init_vga();
}

#[no_mangle]
pub extern "C" fn rust_screen_init_fb(
    addr: u64,
    pitch: u32,
    width: u32,
    height: u32,
    bpp: u8,
    ty: u8,
    red_pos: u8,
    red_size: u8,
    green_pos: u8,
    green_size: u8,
    blue_pos: u8,
    blue_size: u8,
) -> i32 {
    drivers::init_fb(
        addr, pitch, width, height, bpp, ty, red_pos, red_size, green_pos, green_size, blue_pos,
        blue_size,
    )
}

#[no_mangle]
pub extern "C" fn rust_screen_present() {
    drivers::screen_present();
}

#[no_mangle]
pub extern "C" fn rust_screen_sync_begin() {
    drivers::screen_sync_begin();
}

#[no_mangle]
pub extern "C" fn rust_screen_sync_end() {
    drivers::screen_sync_end();
}

#[no_mangle]
pub extern "C" fn rust_screen_mark_row(y: u32) {
    drivers::screen_mark_row(y);
}

#[no_mangle]
pub extern "C" fn rust_screen_clear(attr: u8) {
    drivers::screen_clear(attr);
}

#[no_mangle]
pub extern "C" fn rust_screen_paint_bg(rgb: u32) {
    drivers::screen_paint_bg(rgb);
}

#[no_mangle]
pub extern "C" fn rust_screen_relayout() {
    drivers::screen_relayout();
}

#[no_mangle]
pub extern "C" fn rust_screen_fill_panel() {
    drivers::screen_fill_panel();
}

#[no_mangle]
pub extern "C" fn rust_screen_invalidate() {
    drivers::screen_invalidate();
}
