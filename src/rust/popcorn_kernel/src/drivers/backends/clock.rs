//! Clock drive (`clock` → `/dev/clock`): PIT uptime + CMOS RTC wall clock (local).

use alloc::string::String;

use crate::console_ffi::{
    print_color, println_color, separator, timer_get_ticks, timer_get_uptime_ms, u64_to_dec,
    COLOR_LIGHT_CYAN, COLOR_LIGHT_GREEN, COLOR_LIGHT_MAGENTA, COLOR_WHITE, COLOR_YELLOW,
};
use crate::drivers::io::{inb, outb};

const IOC_CLK_TICKS: u64 = (0x05 << 8) | 1;
const IOC_CLK_UPTIME: u64 = (0x05 << 8) | 2;
const IOC_CLK_RTC: u64 = (0x05 << 8) | 3;

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

/// Build-year floor for century guess when CMOS has no century register.
const CURRENT_YEAR: u16 = 2026;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtcDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

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
            IOC_CLK_RTC => match read_rtc() {
                Some(dt) => {
                    *(argp as *mut RtcDateTime) = dt;
                    0
                }
                None => -1,
            },
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
            if let Some(dt) = read_rtc() {
                s.push_str(" local=");
                append_datetime(&mut s, &dt);
            } else {
                s.push_str(" local=unavailable");
            }
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
        "gettime" | "-gettime" | "getime" | "-getime" | "time" | "date" => match read_rtc() {
            Some(dt) => {
                let mut s = String::from("local ");
                append_datetime(&mut s, &dt);
                s
            }
            None => String::from("error: rtc unread"),
        },
        _ => String::from("error: unknown cmd (status|ticks|uptime|gettime)"),
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
    print_rtc_line();
}

/// Shell: `cl -gettime` / `cl -getime`
#[no_mangle]
pub extern "C" fn clock_print_gettime() {
    console_newline_safe();
    println_color("--- Local time (CMOS RTC) ---", COLOR_LIGHT_MAGENTA);
    match read_rtc() {
        Some(dt) => {
            print_color("Date: ", COLOR_LIGHT_CYAN);
            print_u16_padded(dt.year, 4);
            print_color("-", COLOR_WHITE);
            print_u8_padded(dt.month);
            print_color("-", COLOR_WHITE);
            print_u8_padded(dt.day);
            console_newline_safe();
            print_color("Time: ", COLOR_LIGHT_CYAN);
            print_u8_padded(dt.hour);
            print_color(":", COLOR_WHITE);
            print_u8_padded(dt.minute);
            print_color(":", COLOR_WHITE);
            print_u8_padded(dt.second);
            println_color("  (local)", COLOR_YELLOW);
        }
        None => {
            println_color("RTC unavailable (update busy or invalid)", COLOR_YELLOW);
        }
    }
}

fn print_rtc_line() {
    print_color("Local: ", COLOR_LIGHT_CYAN);
    match read_rtc() {
        Some(dt) => {
            print_u16_padded(dt.year, 4);
            print_color("-", COLOR_WHITE);
            print_u8_padded(dt.month);
            print_color("-", COLOR_WHITE);
            print_u8_padded(dt.day);
            print_color(" ", COLOR_WHITE);
            print_u8_padded(dt.hour);
            print_color(":", COLOR_WHITE);
            print_u8_padded(dt.minute);
            print_color(":", COLOR_WHITE);
            print_u8_padded(dt.second);
            console_newline_safe();
        }
        None => println_color("unavailable", COLOR_YELLOW),
    }
}

/// Read CMOS RTC once, as local wall time (BIOS/QEMU values, no TZ convert).
pub fn read_rtc() -> Option<RtcDateTime> {
    let (mut sec, mut min, mut hour, mut day, mut month, mut year, mut century) = read_raw()?;
    let mut last = (sec, min, hour, day, month, year, century);

    // Re-read until two consecutive snapshots match (avoid update tear).
    for _ in 0..8 {
        let next = read_raw()?;
        if next == last {
            break;
        }
        last = next;
        (sec, min, hour, day, month, year, century) = next;
    }
    (sec, min, hour, day, month, year, century) = last;

    let reg_b = cmos_read(0x0B);
    let binary = (reg_b & 0x04) != 0;
    let h24 = (reg_b & 0x02) != 0;

    if !binary {
        sec = bcd_to_bin(sec);
        min = bcd_to_bin(min);
        hour = bcd_to_bin(hour & 0x7F) | (hour & 0x80);
        day = bcd_to_bin(day);
        month = bcd_to_bin(month);
        year = bcd_to_bin(year);
        if century != 0 {
            century = bcd_to_bin(century);
        }
    }

    if !h24 {
        let pm = (hour & 0x80) != 0;
        let mut h = hour & 0x7F;
        if h == 12 {
            h = 0;
        }
        if pm {
            h = h.wrapping_add(12);
        }
        hour = h % 24;
    } else {
        hour &= 0x7F;
    }

    let mut full_year = year as u16;
    if century != 0 {
        full_year = (century as u16) * 100 + (year as u16);
    } else {
        full_year += (CURRENT_YEAR / 100) * 100;
        if full_year < CURRENT_YEAR {
            full_year = full_year.saturating_add(100);
        }
    }

    if month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    if full_year < 1980 || full_year > 2099 {
        return None;
    }

    Some(RtcDateTime {
        year: full_year,
        month,
        day,
        hour,
        minute: min,
        second: sec,
    })
}

/// FAT DOS time (lo) + date (hi) for directory entries. Zeroes if RTC unread.
pub fn fat_dos_time_date() -> (u16, u16) {
    match read_rtc() {
        Some(dt) => {
            let time = ((dt.hour as u16) << 11)
                | ((dt.minute as u16) << 5)
                | ((dt.second as u16) / 2);
            let date = ((dt.year.saturating_sub(1980)) << 9)
                | ((dt.month as u16) << 5)
                | (dt.day as u16);
            (time, date)
        }
        None => (0, 0),
    }
}

/// Stamp create / access / write fields on a 32-byte short dirent.
pub fn stamp_fat_dirent(ent: &mut [u8; 32]) {
    let (time, date) = fat_dos_time_date();
    ent[13] = 0; // create tenths
    ent[14] = (time & 0xFF) as u8;
    ent[15] = (time >> 8) as u8;
    ent[16] = (date & 0xFF) as u8;
    ent[17] = (date >> 8) as u8;
    ent[18] = (date & 0xFF) as u8; // last access date
    ent[19] = (date >> 8) as u8;
    ent[22] = (time & 0xFF) as u8; // write time
    ent[23] = (time >> 8) as u8;
    ent[24] = (date & 0xFF) as u8;
    ent[25] = (date >> 8) as u8;
}

fn read_raw() -> Option<(u8, u8, u8, u8, u8, u8, u8)> {
    // Wait while update-in-progress is clear, then read (OSDev double-check style).
    for _ in 0..100_000 {
        if cmos_read(0x0A) & 0x80 == 0 {
            break;
        }
    }
    if cmos_read(0x0A) & 0x80 != 0 {
        return None;
    }
    let sec = cmos_read(0x00);
    let min = cmos_read(0x02);
    let hour = cmos_read(0x04);
    let day = cmos_read(0x07);
    let month = cmos_read(0x08);
    let year = cmos_read(0x09);
    // Common century register; zero/garbage filtered later.
    let century = cmos_read(0x32);
    Some((sec, min, hour, day, month, year, century))
}

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        // Bit 7 clear => leave NMI enabled.
        outb(CMOS_ADDR, reg & 0x7F);
        outb(0x80, 0); // IO delay
        inb(CMOS_DATA)
    }
}

fn bcd_to_bin(v: u8) -> u8 {
    ((v & 0xF0) >> 4) * 10 + (v & 0x0F)
}

fn append_u64(s: &mut String, v: u64) {
    let mut num = [0u8; 24];
    let n = u64_to_dec(v, &mut num);
    if let Ok(t) = core::str::from_utf8(&num[..n]) {
        s.push_str(t);
    }
}

fn append_datetime(s: &mut String, dt: &RtcDateTime) {
    push_u16_pad(s, dt.year, 4);
    s.push('-');
    push_u8_pad(s, dt.month);
    s.push('-');
    push_u8_pad(s, dt.day);
    s.push(' ');
    push_u8_pad(s, dt.hour);
    s.push(':');
    push_u8_pad(s, dt.minute);
    s.push(':');
    push_u8_pad(s, dt.second);
}

fn push_u8_pad(s: &mut String, v: u8) {
    if v < 10 {
        s.push('0');
    }
    let mut num = [0u8; 24];
    let n = u64_to_dec(v as u64, &mut num);
    if let Ok(t) = core::str::from_utf8(&num[..n]) {
        s.push_str(t);
    }
}

fn push_u16_pad(s: &mut String, v: u16, width: usize) {
    let mut num = [0u8; 24];
    let n = u64_to_dec(v as u64, &mut num);
    for _ in n..width {
        s.push('0');
    }
    if let Ok(t) = core::str::from_utf8(&num[..n]) {
        s.push_str(t);
    }
}

fn print_u8_padded(v: u8) {
    if v < 10 {
        print_color("0", COLOR_WHITE);
    }
    let mut num = [0u8; 24];
    let n = u64_to_dec(v as u64, &mut num);
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), COLOR_WHITE);
}

fn print_u16_padded(v: u16, width: usize) {
    let mut num = [0u8; 24];
    let n = u64_to_dec(v as u64, &mut num);
    for _ in n..width {
        print_color("0", COLOR_WHITE);
    }
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), COLOR_WHITE);
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
