#![no_std]

extern crate alloc;

mod abi;
mod alloc_shim;
mod catalog;
mod console_ffi;
mod drivers;
mod fs;
mod klog;
mod pops;
mod framework_ffi;
#[allow(dead_code)]
pub mod net;
mod net_cmd;
pub mod sync;
pub mod util;

use alloc::boxed::Box;
use core::ffi::c_char;
use core::slice;

extern "C" {
    fn boot_serial_putc(c: u8);
    fn com1_putc(c: u8);
}

/// COM1-only text sink (never debugcon: its single-letter boot tags are grepped).
pub struct Com1;

impl core::fmt::Write for Com1 {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                unsafe { com1_putc(b'\r') };
            }
            unsafe { com1_putc(b) };
        }
        Ok(())
    }
}

/// A Rust panic used to spin silently (looked like a hang). Report it on COM1 and
/// halt with interrupts off so nothing else runs on top of broken state.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    unsafe { core::arch::asm!("cli", options(nostack)) };
    let mut out = Com1;
    let _ = write!(out, "\n*** RUST PANIC: {}", info.message());
    if let Some(loc) = info.location() {
        let _ = write!(out, " at {}:{}", loc.file(), loc.line());
    }
    let _ = out.write_str(" ***\n*** halt ***\n");
    /* No serial port on most laptops: one last write of the log to the boot stick. */
    klog::emergency();
    loop {
        unsafe { core::arch::asm!("cli; hlt", options(nostack)) };
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
    console_ffi::println_color("Rust active", console_ffi::COLOR_LIGHT_GREEN);
    unsafe {
        boot_serial_putc(b'r');
    }

    let b = Box::new(0xA11Cu32);
    if *b == 0xA11Cu32 {
        unsafe {
            boot_serial_putc(b'a');
        }
    }
    drop(b);

    catalog::init();
    drivers::irq::seed_builtins();
    init_drives();
    fs::init();
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
pub extern "C" fn rust_screen_idle_flush() {
    drivers::screen_idle_flush();
}

#[no_mangle]
pub extern "C" fn rust_screen_enable_shadow() -> i32 {
    drivers::screen_enable_shadow()
}

#[no_mangle]
pub extern "C" fn rust_screen_scroll_up(rows: u32, attr: u8) -> i32 {
    drivers::screen_scroll_up(rows, attr)
}

#[no_mangle]
pub extern "C" fn rust_screen_invalidate() {
    drivers::screen_invalidate();
}

#[no_mangle]
pub extern "C" fn rust_kbd_poll() {
    drivers::kbd::poll();
}

#[no_mangle]
pub extern "C" fn rust_kbd_irq() {
    drivers::kbd::irq();
}

#[no_mangle]
pub extern "C" fn rust_kbd_pop(out: *mut u8) -> i32 {
    if out.is_null() {
        return 0;
    }
    match drivers::kbd::pop() {
        Some(b) => {
            unsafe {
                *out = b;
            }
            1
        }
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn rust_serial_putc(c: u8) {
    drivers::backends::serial::putc(c);
}
