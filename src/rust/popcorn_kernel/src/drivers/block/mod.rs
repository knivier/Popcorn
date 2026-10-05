//! Write-gated disk registry + boot-default picker.
//!
//! Policy:
//! - Boot/USB (or first virtio in QEMU) auto-selected and writable
//! - Internal (NVMe/SATA / second virtio) locked until `install <name> YES`
//! - ram0 always available for tests
//! - `install <name>` must be typed first (arm), then `install <name> YES` (unlock);
//!   a bare `install <name> YES` is refused
//! - A USB disk whose first sectors are neither blank nor Popcorn-formatted
//!   (boot stick, Ventoy, NTFS/ext4 data) is `protected`: locked like Internal
//! - Even an unlocked Internal/protected disk refuses writes while it holds
//!   foreign data (GPT header, or a non-blank non-Popcorn sector 0), and every
//!   disk refuses writes to the GPT/MBR head and backup-GPT tail zones then
//! - Real NVMe (when present) is `nvme0`, Internal + locked; virtio[1] becomes `vdb`
//! - Real USB MSC (xHCI, Bulk-Only) is `usb0` (boot, writable); virtio[0] then
//!   becomes `vda`. Without a working MSC device virtio[0] stays the `usb0` stand-in.

pub mod nvme;
pub mod ramdisk;
pub mod usb_msc;
pub mod virtio_blk;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

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
    Nvme(usize),
    /// Real USB mass storage (index into `usb_msc`).
    UsbMsc(usize),
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
    /// USB disk that holds someone else's data (boot media, NTFS, ...): needs unlock too.
    protected: bool,
}

/// Sectors at each end of a disk that hold MBR/GPT + backup GPT.
const PROTECT_ZONE: u64 = 34;

static INITED: AtomicBool = AtomicBool::new(false);
static mut DISKS: Option<Vec<DiskEntry>> = None;
static SELECTED: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Pending install target index, or MAX if none.
static PENDING_INSTALL: AtomicUsize = AtomicUsize::new(usize::MAX);

/* Static names for registered disks (immortal). */
static mut NAME_USB0: &str = "usb0";
static mut NAME_USB1: &str = "usb1";
static mut NAME_VDC: &str = "vdc";
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
        DiskClass::Usb if d.protected => d.install_unlocked,
        DiskClass::Ram | DiskClass::Virtio | DiskClass::Usb => true,
    }
}

/// Needs the `install <name>` / `install <name> YES` handshake before writes.
fn needs_unlock(d: &DiskEntry) -> bool {
    d.class == DiskClass::Internal || d.protected
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

/// Boot: ram0; real USB MSC=usb0 (boot) else virtio[0]=usb0; real NVMe=nvme0 (internal locked), else
/// virtio[1]=nvme0 stand-in; PCI stubs.
pub fn init() {
    /* NVMe/xHCI probes are one-shot; a second init would orphan their devices. */
    if INITED.swap(true, Ordering::SeqCst) {
        return;
    }
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
            protected: false,
        };
        publish_disk(&d);
        t.push(d);
    }

    /* Real NVMe first so virtio[1] can yield the nvme0 name. Failure is non-fatal. */
    let nvme_disk = match nvme::probe() {
        Ok((sectors, ssize)) => Some((sectors, ssize)),
        Err(_) => None,
    };

    /* Real USB MSC first: it claims usb0, so virtio[0] must not. Failure is
     * non-fatal and leaves virtio[0] as the usb0 stand-in. */
    let usb_n = match usb_msc::probe_all() {
        Ok(n) => {
            crate::console_ffi::println_color(
                "USB MSC: ok",
                crate::console_ffi::COLOR_LIGHT_CYAN,
            );
            n
        }
        Err(e) => {
            /* Always surface this on real hardware — silent miss looks like "only ram0". */
            crate::console_ffi::print_color(
                "USB MSC: ",
                crate::console_ffi::COLOR_YELLOW,
            );
            crate::console_ffi::println_color(e, crate::console_ffi::COLOR_YELLOW);
            0
        }
    };
    let mut usb_real = false;
    if usb_n > 0 {
        /* usb0 = first non-foreign disk (blank / Popcorn data), else the first
         * (which then registers as `protected`, i.e. locked until install YES). */
        let primary = (0..usb_n).find(|&i| !usb_msc::is_boot_like(i)).unwrap_or(0);
        let mut order: Vec<usize> = Vec::new();
        order.push(primary);
        for i in 0..usb_n {
            if i != primary {
                order.push(i);
            }
        }
        for (n, &i) in order.iter().enumerate().take(2) {
            if let Some((sectors, ssize)) = usb_msc::capacity(i) {
                let d = DiskEntry {
                    name: if n == 0 {
                        unsafe { NAME_USB0 }
                    } else {
                        unsafe { NAME_USB1 }
                    },
                    class: DiskClass::Usb,
                    sectors,
                    sector_size: ssize,
                    backend: Backend::UsbMsc(i),
                    is_boot: n == 0,
                    install_unlocked: false,
                    protected: usb_msc::is_boot_like(i),
                };
                publish_disk(&d);
                t.push(d);
                usb_real = true;
            }
        }
    }

    let nvirt = virtio_blk::probe_all();
    for i in 0..nvirt {
        if let Some((sectors, ssize)) = virtio_blk::capacity(i) {
            let (name, class, is_boot) = if i == 0 && usb_real {
                /* Real USB MSC owns usb0; first virtio is a plain data disk. */
                (unsafe { NAME_VDA }, DiskClass::Virtio, false)
            } else if i == 0 {
                /* QEMU stand-in for "OS lives on USB / boot medium". */
                (unsafe { NAME_USB0 }, DiskClass::Usb, true)
            } else if i == 1 && nvme_disk.is_some() {
                /* Real nvme0 present: virtio[1] is just a plain virtio disk. */
                (unsafe { NAME_VDB }, DiskClass::Virtio, false)
            } else if i == 1 {
                (unsafe { NAME_NVME0 }, DiskClass::Internal, false)
            } else if i == 2 {
                if usb_real {
                    (unsafe { NAME_VDC }, DiskClass::Virtio, false)
                } else {
                    (unsafe { NAME_VDA }, DiskClass::Virtio, false)
                }
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
                protected: false,
            };
            publish_disk(&d);
            t.push(d);
        }
    }

    if let Some((sectors, ssize)) = nvme_disk {
        let d = DiskEntry {
            name: unsafe { NAME_NVME0 },
            class: DiskClass::Internal,
            sectors,
            sector_size: ssize,
            backend: Backend::Nvme(0),
            is_boot: false,
            install_unlocked: false,
            protected: false,
        };
        publish_disk(&d);
        t.push(d);
    }

    /* Wave 2: enumerate real PCI storage as locked internals (list-only). */
    let mut sata_i = 0u32;
    let mut ide_i = 0u32;
    pci::foreach_storage_controller(|_addr, _class, subclass| {
        let t = table();
        /* Skip if we already have virtio stand-ins filling those roles. */
        let (name, skip) = match subclass {
            0x08 => {
                /* NVMe */
                /* Only the first NVMe controller is listed (as nvme0). */
                if t.iter().any(|d| d.name.starts_with("nvme")) {
                    ("", true)
                } else {
                    (unsafe { NAME_NVME0 }, false)
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
            protected: false,
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
        return Err("locked — use: install <name>, then install <name> YES");
    }
    SELECTED.store(idx, Ordering::Release);
    Ok(())
}

/// Start or complete install unlock for an internal (or protected USB) disk.
/// `install nvme0` arms; `install nvme0 YES` unlocks + selects — but only for the
/// disk that was armed by the immediately preceding `install` (a bare `... YES`
/// is refused).
/// Returns Ok(true) if unlocked, Ok(false) if armed (needs YES).
pub fn install(name: &str, confirm_yes: bool) -> Result<bool, &'static str> {
    let t = table();
    let idx = t
        .iter()
        .position(|d| d.name == name)
        .ok_or("unknown disk")?;
    if !needs_unlock(&t[idx]) {
        return Err("install only for internal/protected disks");
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
    if PENDING_INSTALL.load(Ordering::Acquire) != idx {
        /* YES without an arm for *this* disk: refuse (never self-arm). */
        return Err("not armed: run `install <name>` first, then `install <name> YES`");
    }
    t[idx].install_unlocked = true;
    if let Backend::Nvme(_) = t[idx].backend {
        nvme::set_write_enabled(true);
    }
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

pub fn selected_capacity() -> Option<(u64, u32)> {
    let d = selected_entry()?;
    if d.sectors == 0 {
        None
    } else {
        Some((d.sectors, d.sector_size))
    }
}

/// Index of the selected disk (`usize::MAX` = none). Lets the filesystem notice
/// that the disk it mounted is no longer the one block I/O would hit.
pub fn selected_id() -> usize {
    SELECTED.load(Ordering::Acquire)
}

fn raw_read(d: &DiskEntry, lba: u64, buf: &mut [u8]) -> i64 {
    match d.backend {
        Backend::Ram => ramdisk::read(lba, buf),
        Backend::Virtio(i) => virtio_blk::read(i, lba, buf),
        Backend::Nvme(i) => nvme::read(i, lba, buf),
        Backend::UsbMsc(i) => usb_msc::read(i, lba, buf),
        Backend::Stub => -1,
    }
}

fn raw_write(d: &DiskEntry, lba: u64, buf: &[u8]) -> i64 {
    match d.backend {
        Backend::Ram => ramdisk::write(lba, buf),
        Backend::Virtio(i) => virtio_blk::write(i, lba, buf),
        Backend::Nvme(i) => nvme::write(i, lba, buf),
        Backend::UsbMsc(i) => usb_msc::write(i, lba, buf),
        Backend::Stub => -1,
    }
}

/// True unless sector 0 is blank or a Popcorn-formatted FAT32 boot sector and
/// LBA 1 is not a GPT header. Fails closed (true) on any read error.
fn holds_foreign_data(d: &DiskEntry) -> bool {
    if d.class == DiskClass::Ram {
        return false;
    }
    let ss = d.sector_size as usize;
    if !(512..=4096).contains(&ss) {
        return true;
    }
    let mut s0 = vec![0u8; ss];
    if raw_read(d, 0, &mut s0) < 0 {
        return true;
    }
    let blank = s0.iter().all(|&b| b == 0);
    let ours = s0[510] == 0x55 && s0[511] == 0xAA && &s0[3..11] == b"POPCORN ";
    if !(blank || ours) {
        return true;
    }
    if d.sectors > 1 {
        let mut s1 = vec![0u8; ss];
        if raw_read(d, 1, &mut s1) < 0 || &s1[0..8] == b"EFI PART" {
            return true;
        }
    }
    false
}

/// True only if the selected disk uses 512 B sectors and its MBR/GPT head
/// (first `PROTECT_ZONE` sectors) and last sector are all zero. This is the one
/// and only condition under which FAT32 may auto-format a disk.
pub fn selected_is_blank() -> bool {
    let Some(d) = selected_entry() else {
        return false;
    };
    if d.sector_size != 512 || d.sectors == 0 {
        return false;
    }
    let mut b = vec![0u8; 512];
    for lba in 0..PROTECT_ZONE.min(d.sectors) {
        if raw_read(d, lba, &mut b) < 0 || b.iter().any(|&x| x != 0) {
            return false;
        }
    }
    if d.sectors > PROTECT_ZONE {
        if raw_read(d, d.sectors - 1, &mut b) < 0 || b.iter().any(|&x| x != 0) {
            return false;
        }
    }
    true
}

pub fn read_selected(lba: u64, buf: &mut [u8]) -> i64 {
    let Some(d) = selected_entry() else {
        return -4;
    };
    if buf.len() < d.sector_size as usize || (d.sectors != 0 && lba >= d.sectors) {
        return -2;
    }
    raw_read(d, lba, buf)
}

/// Write one sector to the selected disk.
/// -2 bad args, -4 no disk, -5 locked, -6 refused (foreign data / GPT zone).
pub fn write_selected(lba: u64, buf: &[u8]) -> i64 {
    let Some(d) = selected_entry() else {
        return -4;
    };
    if !is_writable(d) {
        return -5;
    }
    if buf.len() < d.sector_size as usize || (d.sectors != 0 && lba >= d.sectors) {
        return -2;
    }
    if d.class != DiskClass::Ram {
        let in_zone = lba < PROTECT_ZONE
            || (d.sectors > PROTECT_ZONE && lba >= d.sectors - PROTECT_ZONE);
        /* Unlocked Internal/protected disks and the MBR/GPT head+tail of any
         * disk are off limits while the disk holds data that is not ours. */
        if (needs_unlock(d) || in_zone) && holds_foreign_data(d) {
            return -6;
        }
    }
    raw_write(d, lba, buf)
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
    let rc = cstr_map(name, |s| use_disk(s).map(|_| 0).unwrap_or(-1));
    if rc == 0 {
        crate::fs::remount_selected();
    }
    rc
}

/// Returns 0 unlocked, 1 need YES, -1 error.
#[no_mangle]
pub extern "C" fn rust_disk_install(name: *const core::ffi::c_char, yes: i32) -> i32 {
    let rc = cstr_map(name, |s| match install(s, yes != 0) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(_) => -1,
    });
    if rc == 0 {
        crate::fs::remount_selected();
    }
    rc
}

/// Run `f` on the NUL-terminated disk name (max 31 bytes, valid UTF-8).
fn cstr_map(name: *const core::ffi::c_char, f: impl FnOnce(&str) -> i32) -> i32 {
    if name.is_null() {
        return -1;
    }
    let mut len = 0usize;
    unsafe {
        while len < 32 && *name.add(len) != 0 {
            len += 1;
        }
        if len == 32 {
            return -1; /* not NUL-terminated within the name limit */
        }
        match core::str::from_utf8(core::slice::from_raw_parts(name as *const u8, len)) {
            Ok(s) => f(s),
            Err(_) => -1,
        }
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
