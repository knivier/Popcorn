//! Write-gated disk registry + boot-default picker.
//!
//! Policy:
//! - Boot/USB (or first virtio in QEMU) auto-selected and writable
//! - Internal (NVMe/SATA / second virtio) locked until `install <name> YES`
//! - ram0 always available for tests

pub mod ramdisk;
pub mod virtio_blk;

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::catalog;
use crate::drivers::bus::pci;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DiskClass {
    Ram = 1,
    Virtio = 2,
    Usb = 3,
    Internal = 4,
}

impl DiskClass {
    pub fn as_str(self) -> &'static str {
        match self {
            DiskClass::Ram => "ram",
            DiskClass::Virtio => "virtio",
            DiskClass::Usb => "usb",
            DiskClass::Internal => "internal",
        }
    }
}

#[derive(Clone, Copy)]
enum Backend {
    Ram,
    Virtio(usize),
    /// Listed only (PCI storage controller); no sector I/O yet.
    Stub,
}

struct DiskEntry {
    name: &'static str,
    class: DiskClass,
    sectors: u64,
    sector_size: u32,
    backend: Backend,
    is_boot: bool,
    /// Internal disks need install unlock before writes.
    install_unlocked: bool,
}

static mut DISKS: Option<Vec<DiskEntry>> = None;
static SELECTED: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Pending install target index, or MAX if none.
static PENDING_INSTALL: AtomicUsize = AtomicUsize::new(usize::MAX);

/* Static names for registered disks (immortal). */
static mut NAME_USB0: &str = "usb0";
static mut NAME_NVME0: &str = "nvme0";
static mut NAME_VDA: &str = "vda";
static mut NAME_VDB: &str = "vdb";
static mut NAME_RAM0: &str = "ram0";
static mut NAME_SATA0: &str = "sata0";
static mut NAME_IDE0: &str = "ide0";

fn table() -> &'static mut Vec<DiskEntry> {
    unsafe {
        if DISKS.is_none() {
            DISKS = Some(Vec::new());
        }
        DISKS.as_mut().unwrap()
    }
}

fn is_writable(d: &DiskEntry) -> bool {
    match d.class {
        DiskClass::Internal => d.install_unlocked,
        DiskClass::Ram | DiskClass::Virtio | DiskClass::Usb => true,
    }
}

fn publish_disk(d: &DiskEntry) {
    catalog::publish(
        crate::abi::CATALOG_KIND_DISK,
        d.name,
        d.class.as_str(),
        crate::abi::CATALOG_STATE_READY,
        d.sectors as u32,
    );
}

fn auto_select_boot() {
    let t = table();
    let idx = t
        .iter()
        .position(|d| d.is_boot && is_writable(d))
        .or_else(|| t.iter().position(|d| d.class == DiskClass::Usb && is_writable(d)))
        .or_else(|| {
            t.iter()
                .position(|d| d.class == DiskClass::Virtio && is_writable(d))
        });
    if let Some(i) = idx {
        SELECTED.store(i, Ordering::Release);
    }
}

/// Boot: ram0; virtio[0]=usb0 (boot); virtio[1]=nvme0 (internal locked); PCI stubs.
pub fn init() {
    let t = table();
    t.clear();
    SELECTED.store(usize::MAX, Ordering::Release);
    PENDING_INSTALL.store(usize::MAX, Ordering::Release);

    if let Ok((sectors, ssize)) = ramdisk::init() {
        let d = DiskEntry {
            name: unsafe { NAME_RAM0 },
            class: DiskClass::Ram,
            sectors,
            sector_size: ssize,
            backend: Backend::Ram,
            is_boot: false,
            install_unlocked: false,
        };
        publish_disk(&d);
        t.push(d);
    }

    let nvirt = virtio_blk::probe_all();
    for i in 0..nvirt {
        if let Some((sectors, ssize)) = virtio_blk::capacity(i) {
            let (name, class, is_boot) = if i == 0 {
                /* QEMU stand-in for "OS lives on USB / boot medium". */
                (unsafe { NAME_USB0 }, DiskClass::Usb, true)
            } else if i == 1 {
                (unsafe { NAME_NVME0 }, DiskClass::Internal, false)
            } else if i == 2 {
                (unsafe { NAME_VDA }, DiskClass::Virtio, false)
            } else {
                (unsafe { NAME_VDB }, DiskClass::Virtio, false)
            };
            let d = DiskEntry {
                name,
                class,
                sectors,
                sector_size: ssize,
                backend: Backend::Virtio(i),
                is_boot,
                install_unlocked: false,
            };
            publish_disk(&d);
            t.push(d);
        }
    }

    /* Wave 2: enumerate real PCI storage as locked internals (list-only). */
    let mut sata_i = 0u32;
    let mut nvme_i = 0u32;
    let mut ide_i = 0u32;
    pci::foreach_storage_controller(|_addr, _class, subclass| {
        let t = table();
        /* Skip if we already have virtio stand-ins filling those roles. */
        let (name, skip) = match subclass {
            0x08 => {
                /* NVMe */
                if t.iter().any(|d| d.name.starts_with("nvme")) {
                    ("", true)
                } else {
                    let n = if nvme_i == 0 {
                        unsafe { NAME_NVME0 }
                    } else {
                        unsafe { NAME_NVME0 }
                    };
                    nvme_i += 1;
                    (n, nvme_i > 1)
                }
            }
            0x06 => {
                if t.iter().any(|d| d.name.starts_with("sata")) {
                    ("", true)
                } else {
                    sata_i += 1;
                    (unsafe { NAME_SATA0 }, sata_i > 1)
                }
            }
            0x01 => {
                if t.iter().any(|d| d.name.starts_with("ide")) {
                    ("", true)
                } else {
                    ide_i += 1;
                    (unsafe { NAME_IDE0 }, ide_i > 1)
                }
            }
            _ => ("", true),
        };
        if skip || name.is_empty() {
            return;
        }
        let d = DiskEntry {
            name,
            class: DiskClass::Internal,
            sectors: 0,
            sector_size: 512,
            backend: Backend::Stub,
            is_boot: false,
            install_unlocked: false,
        };
        publish_disk(&d);
        t.push(d);
    });

    auto_select_boot();
}

pub fn list(buf: &mut [u8]) -> usize {
    let t = table();
    let sel = SELECTED.load(Ordering::Acquire);
    let mut s = String::new();
    if t.is_empty() {
        s.push_str("(no disks)");
    }
    for (i, d) in t.iter().enumerate() {
        if i == sel {
            s.push('*');
        } else {
            s.push(' ');
        }
        s.push_str(d.name);
        s.push('(');
        s.push_str(d.class.as_str());
        if d.is_boot {
            s.push_str(",boot");
        }
        s.push(')');
        s.push(':');
        if d.sectors == 0 {
            s.push_str("?");
        } else {
            let mib = (d.sectors * d.sector_size as u64) / (1024 * 1024);
            append_u64(&mut s, mib);
            s.push_str("MiB");
        }
        if is_writable(d) {
            s.push_str(" rw");
        } else {
            s.push_str(" LOCKED");
        }
        s.push(' ');
    }
    write_cstr(&s, buf)
}

pub fn use_disk(name: &str) -> Result<(), &'static str> {
    let t = table();
    let idx = t
        .iter()
        .position(|d| d.name == name)
        .ok_or("unknown disk")?;
    if !is_writable(&t[idx]) {
        return Err("locked — use: install <name> YES");
    }
    SELECTED.store(idx, Ordering::Release);
    Ok(())
}

/// Start or complete install unlock for an internal disk.
/// `install nvme0` arms; `install nvme0 YES` unlocks + selects.
/// Returns Ok(true) if unlocked, Ok(false) if armed (needs YES).
pub fn install(name: &str, confirm_yes: bool) -> Result<bool, &'static str> {
    let t = table();
    let idx = t
        .iter()
        .position(|d| d.name == name)
        .ok_or("unknown disk")?;
    if t[idx].class != DiskClass::Internal {
        return Err("install only for internal disks");
    }
    if matches!(t[idx].backend, Backend::Stub) {
        return Err("no driver yet (list-only stub)");
    }
    if t[idx].install_unlocked {
        SELECTED.store(idx, Ordering::Release);
        return Ok(true);
    }
    if !confirm_yes {
        PENDING_INSTALL.store(idx, Ordering::Release);
        return Ok(false);
    }
    let _ = PENDING_INSTALL.load(Ordering::Acquire);
    t[idx].install_unlocked = true;
    SELECTED.store(idx, Ordering::Release);
    PENDING_INSTALL.store(usize::MAX, Ordering::Release);
    Ok(true)
}

pub fn info(buf: &mut [u8]) -> usize {
    let mut s = String::new();
    let t = table();
    let i = SELECTED.load(Ordering::Acquire);
    if i >= t.len() {
        s.push_str("selected: none");
    } else {
        let d = &t[i];
        s.push_str("selected: ");
        s.push_str(d.name);
        s.push_str(" class=");
        s.push_str(d.class.as_str());
        if d.is_boot {
            s.push_str(" boot");
        }
        s.push_str(" sectors=");
        append_u64(&mut s, d.sectors);
        if is_writable(d) {
            s.push_str(" writable");
        } else {
            s.push_str(" LOCKED");
        }
    }
    write_cstr(&s, buf)
}

fn selected_entry() -> Option<&'static DiskEntry> {
    let i = SELECTED.load(Ordering::Acquire);
    let t = table();
    if i >= t.len() {
        None
    } else {
        Some(&t[i])
    }
}

pub fn read_selected(lba: u64, buf: &mut [u8]) -> i64 {
    let Some(d) = selected_entry() else {
        return -4;
    };
    match d.backend {
        Backend::Ram => ramdisk::read(lba, buf),
        Backend::Virtio(i) => virtio_blk::read(i, lba, buf),
        Backend::Stub => -1,
    }
}

pub fn write_selected(lba: u64, buf: &[u8]) -> i64 {
    let Some(d) = selected_entry() else {
        return -4;
    };
    if !is_writable(d) {
        return -5;
    }
    match d.backend {
        Backend::Ram => ramdisk::write(lba, buf),
        Backend::Virtio(i) => virtio_blk::write(i, lba, buf),
        Backend::Stub => -1,
    }
}

fn append_u64(s: &mut String, mut v: u64) {
    if v == 0 {
        s.push('0');
        return;
    }
    let mut tmp = [0u8; 20];
    let mut i = 0;
    while v > 0 {
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        s.push(tmp[i] as char);
    }
}

fn write_cstr(s: &str, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let n = s.as_bytes().len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf[n] = 0;
    n
}

/* —— C FFI —— */

#[no_mangle]
pub extern "C" fn rust_disk_init() {
    init();
}

#[no_mangle]
pub extern "C" fn rust_disk_list(buf: *mut u8, buflen: usize) -> i32 {
    if buf.is_null() || buflen == 0 {
        return 0;
    }
    list(unsafe { core::slice::from_raw_parts_mut(buf, buflen) }) as i32
}

#[no_mangle]
pub extern "C" fn rust_disk_use(name: *const core::ffi::c_char) -> i32 {
    cstr_map(name, |s| use_disk(s).map(|_| 0).unwrap_or(-1))
}

/// Returns 0 unlocked, 1 need YES, -1 error.
#[no_mangle]
pub extern "C" fn rust_disk_install(name: *const core::ffi::c_char, yes: i32) -> i32 {
    cstr_map(name, |s| match install(s, yes != 0) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(_) => -1,
    })
}

fn cstr_map(name: *const core::ffi::c_char, f: impl FnOnce(&str) -> i32) -> i32 {
    if name.is_null() {
        return -1;
    }
    let mut len = 0usize;
    unsafe {
        while *name.add(len) != 0 && len < 32 {
            len += 1;
        }
        let s = core::str::from_utf8_unchecked(core::slice::from_raw_parts(name as *const u8, len));
        f(s)
    }
}

#[no_mangle]
pub extern "C" fn rust_disk_info(buf: *mut u8, buflen: usize) -> i32 {
    if buf.is_null() || buflen == 0 {
        return 0;
    }
    info(unsafe { core::slice::from_raw_parts_mut(buf, buflen) }) as i32
}

#[no_mangle]
pub extern "C" fn rust_disk_read(lba: u64, buf: *mut u8, buflen: usize) -> i32 {
    if buf.is_null() || buflen == 0 {
        return -2;
    }
    read_selected(lba, unsafe { core::slice::from_raw_parts_mut(buf, buflen) }) as i32
}

#[no_mangle]
pub extern "C" fn rust_disk_write(lba: u64, buf: *const u8, buflen: usize) -> i32 {
    if buf.is_null() || buflen == 0 {
        return -2;
    }
    write_selected(lba, unsafe { core::slice::from_raw_parts(buf, buflen) }) as i32
}
