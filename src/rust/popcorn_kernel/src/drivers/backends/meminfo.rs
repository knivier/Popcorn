//! Physical memory map / stats drive (`mem` → `/dev/meminfo`).

use alloc::string::String;

use crate::sync::{irq_restore, irq_save_disable, Once};

use crate::console_ffi::{
    print, print_color, println_color, separator, u64_to_dec, COLOR_LIGHT_CYAN, COLOR_LIGHT_GREEN,
    COLOR_LIGHT_MAGENTA, COLOR_LIGHT_RED, COLOR_WHITE, COLOR_YELLOW,
};

const MEMORY_TYPE_AVAILABLE: u32 = 1;
const IOC_MEM_STATS: u64 = (0x03 << 8) | 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryStats {
    pub total_physical: u64,
    pub total_available: u64,
    pub total_reserved: u64,
    pub total_used: u64,
    pub num_regions: u32,
    pub num_available_regions: u32,
}

/// Computed once from the boot memory map; stable address for `memory_pop_get_stats`.
static STATS: Once<MemoryStats> = Once::new();

extern "C" {
    static multiboot2_info_ptr: u64;
    fn multiboot2_get_total_memory() -> u64;
}

pub fn probe() -> Result<(), &'static str> {
    calculate_stats();
    Ok(())
}

pub fn read(_buf: &mut [u8]) -> i64 {
    -2
}

pub fn write(_buf: &[u8]) -> i64 {
    -2
}

pub fn ioctl(request: u64, argp: *mut u8) -> i64 {
    if request != IOC_MEM_STATS || argp.is_null() {
        return -2;
    }
    let st = *calculate_stats();
    unsafe {
        *(argp as *mut MemoryStats) = st;
    }
    0
}

pub fn cmd(cmd: &str) -> String {
    let st = *calculate_stats();
    match cmd {
        "status" | "info" | "stats" => {
            let mut s = String::from("mem ready phys=");
            append_bytes(&mut s, st.total_physical);
            s.push_str(" avail=");
            append_bytes(&mut s, st.total_available);
            s.push_str(" reserved=");
            append_bytes(&mut s, st.total_reserved);
            s
        }
        "usage" => {
            let mut s = String::from("used=");
            append_bytes(&mut s, st.total_used);
            s.push_str(" free=");
            append_bytes(&mut s, st.total_available.saturating_sub(st.total_used));
            s
        }
        "map" => String::from("ok (see mem -map)"),
        _ => String::from("error: unknown cmd (status|info|stats|usage|map)"),
    }
}

/// Compute (first call) and return the memory stats.
pub fn calculate_stats() -> &'static MemoryStats {
    if let Some(st) = STATS.get() {
        return st;
    }
    let mut st = MemoryStats {
        total_physical: 0,
        total_available: 0,
        total_reserved: 0,
        total_used: 0,
        num_regions: 0,
        num_available_regions: 0,
    };

    let mbi_ptr = unsafe { multiboot2_info_ptr };
    if mbi_ptr != 0 {
        walk_mmap(mbi_ptr, |addr_len_type| {
            let (_addr, len, ty) = addr_len_type;
            st.num_regions += 1;
            st.total_physical = st.total_physical.saturating_add(len);
            if ty == MEMORY_TYPE_AVAILABLE {
                st.total_available = st.total_available.saturating_add(len);
                st.num_available_regions += 1;
            } else {
                st.total_reserved = st.total_reserved.saturating_add(len);
            }
        });
    }

    if st.total_available == 0 {
        let total = unsafe { multiboot2_get_total_memory() };
        if total > 0 {
            st.total_available = total;
            st.total_physical = total;
            st.num_regions = 1;
            st.num_available_regions = 1;
        }
    }

    if st.total_available > 0 {
        st.total_used = 2 * 1024 * 1024; // kernel footprint estimate
    }

    /* Racing callers compute the same value; the first to publish wins. Interrupts
     * are off across call_once so preemption cannot land inside its busy window. */
    let flags = irq_save_disable();
    let r = STATS.call_once(|| st);
    irq_restore(flags);
    r
}

#[no_mangle]
pub extern "C" fn memory_calculate_stats() {
    calculate_stats();
}

#[no_mangle]
pub extern "C" fn memory_pop_get_stats() -> *const MemoryStats {
    calculate_stats() as *const MemoryStats
}

#[no_mangle]
pub extern "C" fn memory_print_stats() {
    let st = *calculate_stats();
    unsafe { crate::console_ffi::console_newline() };
    println_color("=== MEMORY STATISTICS ===", COLOR_LIGHT_MAGENTA);
    separator();
    line_bytes("Total Physical:  ", st.total_physical, COLOR_WHITE);
    line_bytes("Total Available: ", st.total_available, COLOR_LIGHT_GREEN);
    line_bytes("Total Reserved:  ", st.total_reserved, COLOR_YELLOW);
    line_u32("Total Regions:   ", st.num_regions, COLOR_WHITE);
    line_u32("Avail Regions:   ", st.num_available_regions, COLOR_LIGHT_GREEN);
    separator();
}

#[no_mangle]
pub extern "C" fn memory_print_usage() {
    let st = *calculate_stats();
    unsafe { crate::console_ffi::console_newline() };
    println_color("=== MEMORY USAGE ===", COLOR_LIGHT_MAGENTA);
    separator();
    line_bytes("Total Available: ", st.total_available, COLOR_LIGHT_GREEN);
    line_bytes("Total Used:      ", st.total_used, COLOR_YELLOW);
    line_bytes(
        "Total Free:      ",
        st.total_available.saturating_sub(st.total_used),
        COLOR_LIGHT_GREEN,
    );
    if st.total_available > 0 {
        let pct = ((st.total_used * 100) / st.total_available) as u32;
        print_color("Usage:           ", COLOR_LIGHT_CYAN);
        let mut num = [0u8; 12];
        let n = u64_to_dec(pct as u64, &mut num);
        print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), COLOR_WHITE);
        println_color("%", COLOR_WHITE);
    }
    separator();
}

#[no_mangle]
pub extern "C" fn memory_print_map() {
    unsafe { crate::console_ffi::console_newline() };
    println_color("=== MEMORY MAP ===", COLOR_LIGHT_MAGENTA);
    separator();
    let mbi_ptr = unsafe { multiboot2_info_ptr };
    if mbi_ptr == 0 {
        println_color("No memory map available", COLOR_LIGHT_RED);
        return;
    }
    println_color("Base Address      | Length           | Type", COLOR_LIGHT_CYAN);
    separator();
    /* Print every region (this used to stop after 12, which on UEFI machines
     * hid all RAM above the first few hundred MiB). Firmware maps are split
     * into many small runs, so back-to-back regions of the same type are merged
     * into one row. */
    let mut raw = 0u32;
    let mut rows = 0u32;
    let mut run: Option<(u64, u64, u32)> = None;
    // Firmware maps are not always sorted (the T14s lists some reserved ranges
    // after the RAM above 4 GiB), so sort by base first.
    let mut entries: alloc::vec::Vec<(u64, u64, u32)> = alloc::vec::Vec::new();
    walk_mmap(mbi_ptr, |e| entries.push(e));
    entries.sort_unstable_by_key(|e| e.0);
    for &(addr, len, ty) in entries.iter() {
        raw += 1;
        if let Some((a, l, t)) = run {
            if t == ty && a.checked_add(l) == Some(addr) {
                run = Some((a, l.saturating_add(len), t));
                continue;
            }
            map_row(a, l, t);
            rows += 1;
        }
        run = Some((addr, len, ty));
    }
    if let Some((a, l, t)) = run {
        map_row(a, l, t);
        rows += 1;
    }
    separator();
    let mut num = [0u8; 12];
    let n = u64_to_dec(rows as u64, &mut num);
    print_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), COLOR_WHITE);
    print(" regions (");
    let n = u64_to_dec(raw as u64, &mut num);
    print(core::str::from_utf8(&num[..n]).unwrap_or("0"));
    println_color(" firmware entries, sorted, adjacent same-type entries merged)", COLOR_WHITE);
}

fn map_row(addr: u64, len: u64, ty: u32) {
    let mut buf = [0u8; 64];
    format_addr(addr, &mut buf);
    print_color(cstr_str(&buf), COLOR_WHITE);
    print(" | ");
    format_bytes(len, &mut buf);
    print_color(cstr_str(&buf), COLOR_WHITE);
    let pad = 16usize.saturating_sub(cstr_str(&buf).len());
    for _ in 0..pad {
        print(" ");
    }
    print(" | ");
    let (name, color) = type_name(ty);
    println_color(name, color);
}

fn walk_mmap(mbi_ptr: u64, mut f: impl FnMut((u64, u64, u32))) {
    let mbi = mbi_ptr as *const u8;
    let total_size = unsafe { *(mbi as *const u32) } as usize;
    if total_size < 8 || total_size > 0x100000 {
        return;
    }
    let mut off = 8usize;
    while off + 8 <= total_size {
        let tag = unsafe { mbi.add(off) };
        let ty = unsafe { *(tag as *const u32) };
        let size = unsafe { *(tag.add(4) as *const u32) } as usize;
        if ty == 0 || size < 8 {
            break;
        }
        if ty == 6 && size >= 16 {
            let entry_size = unsafe { *(tag.add(8) as *const u32) } as usize;
            let mut eoff = 16usize;
            while eoff + 20 <= size && entry_size >= 20 {
                let e = unsafe { tag.add(eoff) };
                let addr = unsafe { *(e as *const u64) };
                let len = unsafe { *(e.add(8) as *const u64) };
                let mem_ty = unsafe { *(e.add(16) as *const u32) };
                f((addr, len, mem_ty));
                eoff += entry_size;
            }
        }
        off += (size + 7) & !7;
        if off >= total_size {
            break;
        }
    }
}

fn type_name(ty: u32) -> (&'static str, u8) {
    match ty {
        MEMORY_TYPE_AVAILABLE => ("Available", COLOR_LIGHT_GREEN),
        2 => ("Reserved", COLOR_YELLOW),
        3 => ("ACPI Reclaimable", COLOR_YELLOW),
        4 => ("ACPI NVS", COLOR_YELLOW),
        5 => ("Bad RAM", COLOR_LIGHT_RED),
        _ => ("Unknown", COLOR_WHITE),
    }
}

fn line_bytes(label: &str, bytes: u64, color: u8) {
    print_color(label, COLOR_LIGHT_CYAN);
    let mut buf = [0u8; 64];
    format_bytes(bytes, &mut buf);
    println_color(cstr_str(&buf), color);
}

fn line_u32(label: &str, v: u32, color: u8) {
    print_color(label, COLOR_LIGHT_CYAN);
    let mut num = [0u8; 12];
    let n = u64_to_dec(v as u64, &mut num);
    println_color(core::str::from_utf8(&num[..n]).unwrap_or("0"), color);
}

fn format_bytes(bytes: u64, buf: &mut [u8]) {
    /* Whole GiB only from 16 GiB up (same rule as memory.c): truncating to
     * whole GiB below that turned ~4040 MiB into a misleading "3 GB". */
    let (val, suf): (u64, &[u8]) = if bytes >= 16 << 30 {
        (bytes >> 30, b" GiB")
    } else if bytes >= 1 << 20 {
        (bytes >> 20, b" MiB")
    } else if bytes >= 1024 {
        (bytes >> 10, b" KiB")
    } else {
        (bytes, b" B")
    };
    let n = u64_to_dec(val, buf);
    let end = (n + suf.len()).min(buf.len() - 1);
    buf[n..end].copy_from_slice(&suf[..end - n]);
    buf[end] = 0;
}

fn format_addr(addr: u64, buf: &mut [u8]) {
    const HEX: &[u8] = b"0123456789ABCDEF";
    if buf.len() < 19 {
        return;
    }
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..16 {
        let shift = (15 - i) * 4;
        buf[2 + i] = HEX[((addr >> shift) & 0xF) as usize];
    }
    buf[18] = 0;
}

fn cstr_str(buf: &[u8]) -> &str {
    let mut n = 0;
    while n < buf.len() && buf[n] != 0 {
        n += 1;
    }
    core::str::from_utf8(&buf[..n]).unwrap_or("")
}

fn append_bytes(s: &mut String, bytes: u64) {
    let mut buf = [0u8; 64];
    format_bytes(bytes, &mut buf);
    s.push_str(cstr_str(&buf));
}
