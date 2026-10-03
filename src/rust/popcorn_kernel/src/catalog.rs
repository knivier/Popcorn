//! In-kernel registry database — one lookup table for drives, /dev nodes, pops, IRQs, syscalls.
//!
//! This is not a disk database. It is a RAM catalog the kernel (and later VFS/userspace
//! tooling) can query by name or kind without walking separate registries.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::abi::{
    CatalogEntryC, CATALOG_KIND_DEVICE, CATALOG_KIND_DRIVE, CATALOG_KIND_IRQ, CATALOG_KIND_POP,
    CATALOG_KIND_SYSCALL, CATALOG_STATE_BOUND, CATALOG_STATE_IDLE, CATALOG_STATE_READY,
};

const MAX_ENTRIES: usize = 96;

static mut TABLE: Option<Vec<CatalogEntryC>> = None;
static INIT: AtomicBool = AtomicBool::new(false);

fn table() -> &'static mut Vec<CatalogEntryC> {
    unsafe {
        if TABLE.is_none() {
            TABLE = Some(Vec::with_capacity(MAX_ENTRIES));
        }
        TABLE.as_mut().unwrap()
    }
}

fn fill_name(dst: &mut [u8], src: &str) {
    dst.fill(0);
    let b = src.as_bytes();
    let n = b.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&b[..n]);
}

fn same_name(entry: &CatalogEntryC, name: &str) -> bool {
    let b = name.as_bytes();
    let mut i = 0usize;
    while i < entry.name.len() && entry.name[i] != 0 {
        if i >= b.len() || entry.name[i] != b[i] {
            return false;
        }
        i += 1;
    }
    i == b.len()
}

/// Insert or update an entry by (kind, name).
pub fn publish(kind: u8, name: &str, class_name: &str, state: u8, id: u32) {
    if name.is_empty() {
        return;
    }
    let t = table();
    if let Some(e) = t.iter_mut().find(|e| e.kind == kind && same_name(e, name)) {
        e.state = state;
        e.id = id;
        fill_name(&mut e.class_name, class_name);
        return;
    }
    if t.len() >= MAX_ENTRIES {
        return;
    }
    let mut e = CatalogEntryC {
        kind,
        state,
        reserved: 0,
        id,
        name: [0; 32],
        class_name: [0; 16],
    };
    fill_name(&mut e.name, name);
    fill_name(&mut e.class_name, class_name);
    t.push(e);
    INIT.store(true, Ordering::Release);
}

pub fn publish_drive(name: &str, class_name: &str, ready: bool, id: u32) {
    publish(
        CATALOG_KIND_DRIVE,
        name,
        class_name,
        if ready {
            CATALOG_STATE_READY
        } else {
            CATALOG_STATE_IDLE
        },
        id,
    );
}

pub fn publish_device(name: &str, class_name: &str, id: u32) {
    publish(CATALOG_KIND_DEVICE, name, class_name, CATALOG_STATE_READY, id);
}

pub fn publish_pop(name: &str, id: u32) {
    publish(CATALOG_KIND_POP, name, "pop", CATALOG_STATE_READY, id);
}

pub fn publish_irq(name: &str, irq: u8) {
    publish(CATALOG_KIND_IRQ, name, "irq", CATALOG_STATE_BOUND, irq as u32);
}

pub fn publish_syscall(name: &str, num: u32) {
    publish(CATALOG_KIND_SYSCALL, name, "syscall", CATALOG_STATE_READY, num);
}

pub fn lookup(name: &str) -> Option<CatalogEntryC> {
    table().iter().find(|e| same_name(e, name)).copied()
}

pub fn lookup_kind(kind: u8, name: &str) -> Option<CatalogEntryC> {
    table()
        .iter()
        .find(|e| e.kind == kind && same_name(e, name))
        .copied()
}

pub fn count() -> usize {
    table().len()
}

/// Write a human listing into `buf` (NUL-terminated). Optional kind filter (0 = all).
pub fn list(kind_filter: u8, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let mut out = alloc::string::String::new();
    for e in table().iter() {
        if kind_filter != 0 && e.kind != kind_filter {
            continue;
        }
        let name_end = e.name.iter().position(|&c| c == 0).unwrap_or(e.name.len());
        let class_end = e
            .class_name
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(e.class_name.len());
        let kind = match e.kind {
            CATALOG_KIND_DRIVE => "drive",
            CATALOG_KIND_DEVICE => "dev",
            CATALOG_KIND_POP => "pop",
            CATALOG_KIND_IRQ => "irq",
            CATALOG_KIND_SYSCALL => "sys",
            _ => "?",
        };
        let state = match e.state {
            CATALOG_STATE_IDLE => "idle",
            CATALOG_STATE_READY => "ready",
            _ => "bound",
        };
        out.push_str(kind);
        out.push(':');
        out.push_str(core::str::from_utf8(&e.name[..name_end]).unwrap_or("?"));
        out.push('(');
        out.push_str(core::str::from_utf8(&e.class_name[..class_end]).unwrap_or("?"));
        out.push(')');
        out.push(':');
        out.push_str(state);
        out.push(' ');
    }
    let n = out.as_bytes().len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&out.as_bytes()[..n]);
    buf[n] = 0;
    n
}

/// Seed builtin syscall numbers (matches registered set in `syscall.c`).
pub fn seed_syscalls() {
    const SYSCALLS: &[(&str, u32)] = &[
        ("exit", 0x01),
        ("read", 0x02),
        ("write", 0x03),
        ("open", 0x04),
        ("close", 0x05),
        ("getpid", 0x07),
        ("malloc", 0x0B),
        ("free", 0x0C),
        ("gettime", 0x0F),
        ("sleep", 0x10),
        ("yield", 0x11),
        ("getcwd", 0x12),
        ("chdir", 0x13),
        ("ioctl", 0x15),
    ];
    for (name, num) in SYSCALLS {
        publish_syscall(name, *num);
    }
}

/// Seed known pop names (Rust + C).
pub fn seed_pops() {
    const POPS: &[&str] = &[
        "Shimjapii",
        "Spinner",
        "Uptime",
        "Memory",
        "CPU",
        "Sysinfo",
        "Filesystem",
        "Dolphin",
    ];
    for (i, name) in POPS.iter().enumerate() {
        publish_pop(name, i as u32);
    }
}

pub fn init() {
    if INIT.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = table();
    seed_syscalls();
    seed_pops();
}

/* —— C FFI —— */

#[no_mangle]
pub extern "C" fn rust_catalog_init() {
    init();
}

#[no_mangle]
pub extern "C" fn rust_catalog_count() -> u32 {
    count() as u32
}

#[no_mangle]
pub extern "C" fn rust_catalog_list(kind: u8, buf: *mut u8, buflen: usize) -> i32 {
    if buf.is_null() || buflen == 0 {
        return 0;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, buflen) };
    list(kind, slice) as i32
}

#[no_mangle]
pub extern "C" fn rust_catalog_lookup(name: *const core::ffi::c_char, out: *mut CatalogEntryC) -> i32 {
    if name.is_null() || out.is_null() {
        return 0;
    }
    let mut len = 0usize;
    unsafe {
        while *name.add(len) != 0 && len < 64 {
            len += 1;
        }
        let s = core::str::from_utf8_unchecked(core::slice::from_raw_parts(name as *const u8, len));
        match lookup(s) {
            Some(e) => {
                *out = e;
                1
            }
            None => 0,
        }
    }
}
