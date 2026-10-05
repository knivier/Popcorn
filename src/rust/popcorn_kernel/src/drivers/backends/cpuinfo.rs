//! CPUID / frequency info drive (`cpu` → `/dev/cpu`).

use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::console_ffi::{
    print, print_color, println_color, separator, u64_to_dec, util_delay, COLOR_LIGHT_CYAN,
    COLOR_LIGHT_GREEN, COLOR_LIGHT_MAGENTA, COLOR_WHITE, COLOR_YELLOW,
};

const IOC_CPU_INFO: u64 = (0x04 << 8) | 1;
const IOC_CPU_FREQ: u64 = (0x04 << 8) | 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CpuFrequency {
    pub tsc_hz: u64,
    pub mhz: u32,
    pub frequency_detected: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtendedCpuInfo {
    pub vendor: [u8; 13],
    pub brand_string: [u8; 49],
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
    pub cores: u32,
    pub has_fpu: u8,
    pub has_sse: u8,
    pub has_sse2: u8,
    pub has_sse3: u8,
    pub has_ssse3: u8,
    pub has_sse41: u8,
    pub has_sse42: u8,
    pub has_avx: u8,
    pub has_avx2: u8,
    pub has_apic: u8,
    pub has_tsc: u8,
    pub has_msr: u8,
}

/// Compact CPUInfo for sysinfo_get_cpu_info ABI.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CpuInfo {
    pub vendor: [u8; 13],
    pub family: u32,
    pub model: u32,
    pub stepping: u32,
    pub has_fpu: u8,
    pub has_sse: u8,
    pub has_sse2: u8,
    pub has_sse3: u8,
    pub has_avx: u8,
    pub has_apic: u8,
}

static mut EXT: ExtendedCpuInfo = ExtendedCpuInfo {
    vendor: [0; 13],
    brand_string: [0; 49],
    family: 0,
    model: 0,
    stepping: 0,
    cores: 1,
    has_fpu: 0,
    has_sse: 0,
    has_sse2: 0,
    has_sse3: 0,
    has_ssse3: 0,
    has_sse41: 0,
    has_sse42: 0,
    has_avx: 0,
    has_avx2: 0,
    has_apic: 0,
    has_tsc: 0,
    has_msr: 0,
};
static mut FREQ: CpuFrequency = CpuFrequency {
    tsc_hz: 0,
    mhz: 0,
    frequency_detected: 0,
};
static mut COMPACT: CpuInfo = CpuInfo {
    vendor: [0; 13],
    family: 0,
    model: 0,
    stepping: 0,
    has_fpu: 0,
    has_sse: 0,
    has_sse2: 0,
    has_sse3: 0,
    has_avx: 0,
    has_apic: 0,
};
static EXT_READY: AtomicBool = AtomicBool::new(false);
static FREQ_READY: AtomicBool = AtomicBool::new(false);

extern "C" {
    fn cpuid_get_vendor(vendor_string: *mut u8);
    fn cpuid_get_features(output: *mut u32);
    fn cpuid_extended_brand(leaf: u32, output: *mut u32);
    fn rdtsc() -> u64;
}

pub fn probe() -> Result<(), &'static str> {
    detect_extended();
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
    match request {
        IOC_CPU_INFO => {
            detect_extended();
            unsafe {
                *(argp as *mut ExtendedCpuInfo) = EXT;
            }
            0
        }
        IOC_CPU_FREQ => {
            detect_frequency();
            unsafe {
                *(argp as *mut CpuFrequency) = FREQ;
            }
            0
        }
        _ => -2,
    }
}

pub fn cmd(cmd: &str) -> String {
    detect_extended();
    match cmd {
        "status" | "info" => {
            let e = unsafe { EXT };
            let mut s = String::from("cpu ready vendor=");
            push_cstr(&mut s, &e.vendor);
            s.push_str(" fam=");
            append_u32(&mut s, e.family);
            s.push_str(" model=");
            append_u32(&mut s, e.model);
            s
        }
        "hz" | "freq" => {
            detect_frequency();
            let f = unsafe { FREQ };
            if f.frequency_detected == 0 {
                String::from("tsc unavailable")
            } else {
                let mut s = String::new();
                append_u32(&mut s, f.mhz);
                s.push_str(" MHz");
                s
            }
        }
        _ => String::from("error: unknown cmd (status|info|hz|freq)"),
    }
}

pub fn detect_extended() {
    if EXT_READY.load(Ordering::Acquire) {
        return;
    }
    let mut e = ExtendedCpuInfo {
        vendor: [0; 13],
        brand_string: [0; 49],
        family: 0,
        model: 0,
        stepping: 0,
        cores: 1,
        has_fpu: 0,
        has_sse: 0,
        has_sse2: 0,
        has_sse3: 0,
        has_ssse3: 0,
        has_sse41: 0,
        has_sse42: 0,
        has_avx: 0,
        has_avx2: 0,
        has_apic: 0,
        has_tsc: 0,
        has_msr: 0,
    };
    unsafe {
        cpuid_get_vendor(e.vendor.as_mut_ptr());
        e.vendor[12] = 0;
        let mut data = [0u32; 4];
        cpuid_get_features(data.as_mut_ptr());
        let eax = data[0];
        let ebx = data[1];
        let ecx = data[2];
        let edx = data[3];
        e.stepping = eax & 0xF;
        e.model = (eax >> 4) & 0xF;
        e.family = (eax >> 8) & 0xF;
        if e.family == 0xF {
            e.family += (eax >> 20) & 0xFF;
        }
        if e.family == 0x6 || e.family == 0xF {
            e.model += ((eax >> 16) & 0xF) << 4;
        }
        e.has_fpu = u8::from((edx & (1 << 0)) != 0);
        e.has_tsc = u8::from((edx & (1 << 4)) != 0);
        e.has_msr = u8::from((edx & (1 << 5)) != 0);
        e.has_apic = u8::from((edx & (1 << 9)) != 0);
        e.has_sse = u8::from((edx & (1 << 25)) != 0);
        e.has_sse2 = u8::from((edx & (1 << 26)) != 0);
        e.has_sse3 = u8::from((ecx & (1 << 0)) != 0);
        e.has_ssse3 = u8::from((ecx & (1 << 9)) != 0);
        e.has_sse41 = u8::from((ecx & (1 << 19)) != 0);
        e.has_sse42 = u8::from((ecx & (1 << 20)) != 0);
        e.has_avx = u8::from((ecx & (1 << 28)) != 0);
        e.cores = (ebx >> 16) & 0xFF;
        if e.cores == 0 {
            e.cores = 1;
        }
        let mut ext_check = [0u32; 4];
        cpuid_extended_brand(0x8000_0000, ext_check.as_mut_ptr());
        if ext_check[0] >= 0x8000_0004 {
            let brand = e.brand_string.as_mut_ptr() as *mut u32;
            cpuid_extended_brand(0x8000_0002, brand);
            cpuid_extended_brand(0x8000_0003, brand.add(4));
            cpuid_extended_brand(0x8000_0004, brand.add(8));
            e.brand_string[48] = 0;
            trim_leading_spaces(&mut e.brand_string);
        }
        if e.has_avx != 0 {
            let mut cpuid7 = [0u32; 4];
            cpuid_extended_brand(7, cpuid7.as_mut_ptr());
            e.has_avx2 = u8::from((cpuid7[1] & (1 << 5)) != 0);
        }
        EXT = e;
        COMPACT = CpuInfo {
            vendor: e.vendor,
            family: e.family,
            model: e.model,
            stepping: e.stepping,
            has_fpu: e.has_fpu,
            has_sse: e.has_sse,
            has_sse2: e.has_sse2,
            has_sse3: e.has_sse3,
            has_avx: e.has_avx,
            has_apic: e.has_apic,
        };
    }
    EXT_READY.store(true, Ordering::Release);
}

pub fn detect_frequency() {
    if FREQ_READY.load(Ordering::Acquire) {
        return;
    }
    detect_extended();
    let mut f = CpuFrequency {
        tsc_hz: 0,
        mhz: 0,
        frequency_detected: 0,
    };
    if unsafe { EXT.has_tsc } == 0 {
        unsafe {
            FREQ = f;
        }
        FREQ_READY.store(true, Ordering::Release);
        return;
    }
    unsafe {
        let t0 = rdtsc();
        util_delay(100);
        let t1 = rdtsc();
        let delta = t1.wrapping_sub(t0);
        f.tsc_hz = delta.saturating_mul(10);
        f.mhz = (f.tsc_hz / 1_000_000) as u32;
        f.frequency_detected = 1;
        FREQ = f;
    }
    FREQ_READY.store(true, Ordering::Release);
}

#[no_mangle]
pub extern "C" fn cpu_detect_extended() {
    detect_extended();
}

#[no_mangle]
pub extern "C" fn cpu_detect_frequency() {
    detect_frequency();
}

#[no_mangle]
pub extern "C" fn cpu_get_extended_info() -> *const ExtendedCpuInfo {
    detect_extended();
    core::ptr::addr_of!(EXT)
}

#[no_mangle]
pub extern "C" fn cpu_get_frequency() -> *const CpuFrequency {
    detect_frequency();
    core::ptr::addr_of!(FREQ)
}

#[no_mangle]
pub extern "C" fn sysinfo_detect_cpu() {
    detect_extended();
}

#[no_mangle]
pub extern "C" fn sysinfo_get_cpu_info() -> *const CpuInfo {
    detect_extended();
    core::ptr::addr_of!(COMPACT)
}

#[no_mangle]
pub extern "C" fn cpu_print_info() {
    detect_extended();
    let e = unsafe { EXT };
    unsafe { crate::console_ffi::console_newline() };
    println_color("=== CPU INFORMATION ===", COLOR_LIGHT_MAGENTA);
    separator();
    print_color("Vendor: ", COLOR_LIGHT_CYAN);
    println_color(bytes_str(&e.vendor), COLOR_LIGHT_GREEN);
    if e.brand_string[0] != 0 {
        print_color("Brand:  ", COLOR_LIGHT_CYAN);
        println_color(bytes_str(&e.brand_string), COLOR_WHITE);
    }
    print_color("Family: ", COLOR_LIGHT_CYAN);
    print_u32(e.family, COLOR_WHITE);
    print_color("  Model: ", COLOR_LIGHT_CYAN);
    print_u32(e.model, COLOR_WHITE);
    print_color("  Stepping: ", COLOR_LIGHT_CYAN);
    print_u32_ln(e.stepping, COLOR_WHITE);
    print_color("Cores:  ", COLOR_LIGHT_CYAN);
    print_u32_ln(e.cores, COLOR_WHITE);
    unsafe { crate::console_ffi::console_newline() };
    println_color("Features:", COLOR_LIGHT_CYAN);
    print("  ");
    feat("FPU ", e.has_fpu);
    feat("TSC ", e.has_tsc);
    feat("MSR ", e.has_msr);
    feat("APIC ", e.has_apic);
    unsafe { crate::console_ffi::console_newline() };
    print("  ");
    feat("SSE ", e.has_sse);
    feat("SSE2 ", e.has_sse2);
    feat("SSE3 ", e.has_sse3);
    feat("SSSE3 ", e.has_ssse3);
    unsafe { crate::console_ffi::console_newline() };
    print("  ");
    feat("SSE4.1 ", e.has_sse41);
    feat("SSE4.2 ", e.has_sse42);
    feat("AVX ", e.has_avx);
    feat("AVX2", e.has_avx2);
    unsafe { crate::console_ffi::console_newline() };
    separator();
}

#[no_mangle]
pub extern "C" fn cpu_print_frequency() {
    detect_frequency();
    let f = unsafe { FREQ };
    unsafe { crate::console_ffi::console_newline() };
    println_color("=== CPU FREQUENCY ===", COLOR_LIGHT_MAGENTA);
    separator();
    if f.frequency_detected == 0 {
        println_color("TSC not available - cannot measure frequency", COLOR_YELLOW);
    } else {
        print_color("Estimated: ", COLOR_LIGHT_CYAN);
        print_u32(f.mhz, COLOR_LIGHT_GREEN);
        println_color(" MHz", COLOR_WHITE);
        print_color("TSC Rate:  ", COLOR_LIGHT_CYAN);
        print_u32((f.tsc_hz / 1_000_000) as u32, COLOR_WHITE);
        println_color(" MHz", COLOR_WHITE);
        unsafe { crate::console_ffi::console_newline() };
        println_color("Note: Frequency measured via TSC sampling", COLOR_LIGHT_CYAN);
    }
    separator();
}

pub fn print_compact_for_sysinfo() {
    detect_extended();
    let e = unsafe { EXT };
    println_color("--- CPU Information ---", COLOR_LIGHT_MAGENTA);
    print_color("Vendor: ", COLOR_LIGHT_CYAN);
    print_color(bytes_str(&e.vendor), COLOR_WHITE);
    print_color("  Family: ", COLOR_LIGHT_CYAN);
    print_u32(e.family, COLOR_WHITE);
    print_color("  Model: ", COLOR_LIGHT_CYAN);
    print_u32(e.model, COLOR_WHITE);
    print_color("  Stepping: ", COLOR_LIGHT_CYAN);
    print_u32_ln(e.stepping, COLOR_WHITE);
    print_color("Features: ", COLOR_LIGHT_CYAN);
    feat("FPU ", e.has_fpu);
    feat("APIC ", e.has_apic);
    feat("SSE ", e.has_sse);
    feat("SSE2 ", e.has_sse2);
    feat("SSE3 ", e.has_sse3);
    feat("AVX", e.has_avx);
    unsafe { crate::console_ffi::console_newline() };
}

fn feat(name: &str, on: u8) {
    if on != 0 {
        print_color(name, COLOR_LIGHT_GREEN);
    }
}

fn print_u32(v: u32, color: u8) {
    let mut num = [0u8; 12];
    let n = u64_to_dec(v as u64, &mut num);
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), color);
}

fn print_u32_ln(v: u32, color: u8) {
    print_u32(v, color);
    println_color("", color);
}

fn bytes_str(b: &[u8]) -> &str {
    let mut n = 0;
    while n < b.len() && b[n] != 0 {
        n += 1;
    }
    core::str::from_utf8(&b[..n]).unwrap_or("")
}

fn push_cstr(s: &mut String, b: &[u8]) {
    s.push_str(bytes_str(b));
}

fn append_u32(s: &mut String, v: u32) {
    let mut num = [0u8; 12];
    let n = u64_to_dec(v as u64, &mut num);
    if let Ok(t) = core::str::from_utf8(&num[..n]) {
        s.push_str(t);
    }
}

fn trim_leading_spaces(s: &mut [u8]) {
    let mut start = 0usize;
    while start < s.len() && s[start] == b' ' {
        start += 1;
    }
    if start == 0 {
        return;
    }
    let mut i = 0usize;
    while start + i < s.len() {
        s[i] = s[start + i];
        if s[i] == 0 {
            break;
        }
        i += 1;
    }
    if i < s.len() {
        s[i] = 0;
    }
}
