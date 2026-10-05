//! Sysinfo pop — aggregates mem / cpu / clock drive info.

use super::registry::PopModule;
use crate::console_ffi::{
    print_color, println_color, separator, u64_to_dec, COLOR_LIGHT_CYAN, COLOR_LIGHT_GREEN,
    COLOR_LIGHT_MAGENTA, COLOR_WHITE,
};
use crate::drivers::backends::{clock, cpuinfo, meminfo};
use core::ffi::c_char;

static NAME: &[u8] = b"sysinfo\0";
static MESSAGE: &[u8] = b"System information detection\0";

extern "C" {
    fn multiboot2_get_bootloader_name() -> *const u8;
    fn multiboot2_get_total_memory() -> u64;
    fn multiboot2_get_memory_lower() -> u32;
    fn multiboot2_get_memory_upper() -> u32;
    fn rust_screen_cols() -> u32;
    fn rust_screen_rows() -> u32;
    fn rust_screen_backend() -> i32;
}

#[no_mangle]
pub extern "C" fn sysinfo_print_full() {
    let _ = crate::drivers::init_drive("mem");
    let _ = crate::drivers::init_drive("cpu");
    let _ = crate::drivers::init_drive("clock");

    unsafe { crate::console_ffi::console_newline() };
    println_color("=== SYSTEM INFORMATION ===", COLOR_LIGHT_MAGENTA);
    separator();

    print_color("Kernel: ", COLOR_LIGHT_CYAN);
    print_color("Popcorn v0.5", COLOR_LIGHT_GREEN);
    print_color("  Architecture: ", COLOR_LIGHT_CYAN);
    println_color("x86_64 (64-bit long mode)", COLOR_WHITE);

    print_color("Bootloader: ", COLOR_LIGHT_CYAN);
    println_color(bootloader_name(), COLOR_WHITE);

    unsafe { crate::console_ffi::console_newline() };
    cpuinfo::print_compact_for_sysinfo();

    /* Prefer conventional RAM (available). total_physical includes MMIO reserved. */
    meminfo::calculate_stats();
    let st = unsafe { *meminfo::memory_pop_get_stats() };
    unsafe { crate::console_ffi::console_newline() };
    println_color("--- Memory Information ---", COLOR_LIGHT_MAGENTA);
    print_color("RAM (available): ", COLOR_LIGHT_CYAN);
    let mut buf = [0u8; 64];
    format_mb(st.total_available, &mut buf);
    println_color(cstr(&buf), COLOR_WHITE);
    print_color("Boot handoff: ", COLOR_LIGHT_CYAN);
    let total = unsafe { multiboot2_get_total_memory() };
    format_mb(total, &mut buf);
    println_color(cstr(&buf), COLOR_WHITE);

    unsafe { crate::console_ffi::console_newline() };
    clock::print_summary();

    unsafe { crate::console_ffi::console_newline() };
    println_color("--- Display Information ---", COLOR_LIGHT_MAGENTA);
    let be = unsafe { rust_screen_backend() };
    print_color("Mode: ", COLOR_LIGHT_CYAN);
    println_color(
        if be == 2 {
            "Framebuffer (GOP)"
        } else if be == 1 {
            "VGA Text"
        } else {
            "None"
        },
        COLOR_WHITE,
    );
    print_color("Grid: ", COLOR_LIGHT_CYAN);
    print_u32(unsafe { rust_screen_cols() }, COLOR_WHITE);
    print_color("x", COLOR_WHITE);
    print_u32(unsafe { rust_screen_rows() }, COLOR_WHITE);
    unsafe { crate::console_ffi::console_newline() };

    separator();
}

#[no_mangle]
pub extern "C" fn sysinfo_pop_func(_start_pos: u32) {
    cpuinfo::detect_extended();
}

#[no_mangle]
pub static sysinfo_module: PopModule = PopModule {
    name: NAME.as_ptr() as *const c_char,
    message: MESSAGE.as_ptr() as *const c_char,
    pop_function: Some(sysinfo_pop_func),
};

fn bootloader_name() -> &'static str {
    let p = unsafe { multiboot2_get_bootloader_name() };
    if p.is_null() {
        return "unknown";
    }
    unsafe {
        let mut len = 0usize;
        while *p.add(len) != 0 && len < 64 {
            len += 1;
        }
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len))
    }
}

fn print_u32(v: u32, color: u8) {
    let mut num = [0u8; 12];
    let n = u64_to_dec(v as u64, &mut num);
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), color);
}

fn format_mb(bytes: u64, buf: &mut [u8]) {
    let mb = bytes / (1024 * 1024);
    let n = u64_to_dec(if mb == 0 { bytes / 1024 } else { mb }, buf);
    let suf: &[u8] = if mb == 0 { b" KB" } else { b" MB" };
    let end = (n + suf.len()).min(buf.len() - 1);
    buf[n..end].copy_from_slice(&suf[..end - n]);
    buf[end] = 0;
}

fn cstr(buf: &[u8]) -> &str {
    let mut n = 0;
    while n < buf.len() && buf[n] != 0 {
        n += 1;
    }
    core::str::from_utf8(&buf[..n]).unwrap_or("")
}
