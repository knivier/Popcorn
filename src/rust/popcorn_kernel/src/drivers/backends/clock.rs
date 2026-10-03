//! PIT / uptime clock info drive (`clock` → `/dev/clock`).

use alloc::string::String;

use crate::console_ffi::{
    print_color, println_color, separator, timer_get_ticks, timer_get_uptime_ms, u64_to_dec,
    COLOR_LIGHT_CYAN, COLOR_LIGHT_GREEN, COLOR_LIGHT_MAGENTA, COLOR_WHITE,
};

const IOC_CLK_TICKS: u64 = (0x05 << 8) | 1;
const IOC_CLK_UPTIME: u64 = (0x05 << 8) | 2;

pub fn probe() -> Result<(), &'static str> {
    Ok(())
}

pub fn read(_buf: &mut [u8]) -> i64 {
    -2
}

pub fn write(_buf: &[u8]) -> i64 {
    -2
}

pub fn ioctl(request: u64, argp: *mut u8) -> i64 {
    if argp.is_null() {
        return -2;
    }
    unsafe {
        match request {
            IOC_CLK_TICKS => {
                *(argp as *mut u64) = timer_get_ticks();
                0
            }
            IOC_CLK_UPTIME => {
                *(argp as *mut u64) = timer_get_uptime_ms();
                0
            }
            _ => -2,
        }
    }
}

pub fn cmd(cmd: &str) -> String {
    match cmd {
        "status" | "info" => {
            let mut s = String::from("clock ready ticks=");
            append_u64(&mut s, unsafe { timer_get_ticks() });
            s.push_str(" uptime_ms=");
            append_u64(&mut s, unsafe { timer_get_uptime_ms() });
            s
        }
        "ticks" => {
            let mut s = String::new();
            append_u64(&mut s, unsafe { timer_get_ticks() });
            s
        }
        "uptime" => {
            let mut s = String::new();
            append_u64(&mut s, unsafe { timer_get_uptime_ms() });
            s.push_str(" ms");
            s
        }
        _ => String::from("error: unknown cmd (status|info|ticks|uptime)"),
    }
}

pub fn print_summary() {
    let mut num = [0u8; 24];
    console_newline_safe();
    println_color("--- Clock ---", COLOR_LIGHT_MAGENTA);
    print_color("Ticks: ", COLOR_LIGHT_CYAN);
    let n = u64_to_dec(unsafe { timer_get_ticks() }, &mut num);
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), COLOR_WHITE);
    print_color("  Uptime: ", COLOR_LIGHT_CYAN);
    let n = u64_to_dec(unsafe { timer_get_uptime_ms() }, &mut num);
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), COLOR_LIGHT_GREEN);
    println_color(" ms", COLOR_WHITE);
}

fn append_u64(s: &mut String, v: u64) {
    let mut num = [0u8; 24];
    let n = u64_to_dec(v, &mut num);
    if let Ok(t) = core::str::from_utf8(&num[..n]) {
        s.push_str(t);
    }
}

fn console_newline_safe() {
    unsafe {
        crate::console_ffi::console_newline();
    }
}

#[allow(dead_code)]
pub fn print_status() {
    print_summary();
    separator();
}
