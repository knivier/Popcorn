//! Write-gated disk registry + boot-default picker.
//!
//! Policy:
//! - Boot never formats, installs, or unlocks NVMe
//! - Internal/NVMe writes: `disk master <name> YES` this boot only
//! - `disk install` / `disk wipe` never flip the NVMe write gate
//! - `disk install <name> YES` stages bootloader+kernel from the live boot
//!   volume into RAM, formats USB/virtio (or master-unlocked NVMe) as Popcorn FAT32
//! - ram0 always available for tests
//! - A USB disk whose first sectors are neither blank nor Popcorn-formatted
//!   (boot stick, Ventoy, NTFS/ext4 data) is `protected` until install/wipe
//! - Real NVMe (when present) is `nvme0`, Internal + locked; virtio[1] becomes `vdb`
//! - Real USB MSC (xHCI, Bulk-Only) is `usb0`; virtio[0] then becomes `vda`

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
    /// Internal/NVMe writes: only `disk master <name> YES`. Never auto, never install.
    master_unlocked: bool,
}

/// Sectors at each end of a disk that hold MBR/GPT + backup GPT.
const PROTECT_ZONE: u64 = 34;

static INITED: AtomicBool = AtomicBool::new(false);
static mut DISKS: Option<Vec<DiskEntry>> = None;
static SELECTED: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Pending install target index, or MAX if none.
static PENDING_INSTALL: AtomicUsize = AtomicUsize::new(usize::MAX);
static PENDING_WIPE: AtomicUsize = AtomicUsize::new(usize::MAX);
static PENDING_MASTER: AtomicUsize = AtomicUsize::new(usize::MAX);

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
    if is_internal_disk(d) {
        return d.master_unlocked;
    }
    match d.class {
        DiskClass::Internal => false,
        DiskClass::Usb if d.protected => d.install_unlocked,
        DiskClass::Ram | DiskClass::Virtio | DiskClass::Usb => true,
    }
}

/// Needs the `install <name>` / `install <name> YES` handshake before writes.
fn needs_unlock(d: &DiskEntry) -> bool {
    (d.class == DiskClass::Internal || d.protected) && !d.install_unlocked
}

fn is_internal_disk(d: &DiskEntry) -> bool {
    d.class == DiskClass::Internal || matches!(d.backend, Backend::Nvme(_))
}

pub fn selected_is_internal() -> bool {
    selected_entry().map(is_internal_disk).unwrap_or(false)
}

pub fn selected_master_unlocked() -> bool {
    selected_entry().map(|d| d.master_unlocked).unwrap_or(false)
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
    PENDING_WIPE.store(usize::MAX, Ordering::Release);
    PENDING_MASTER.store(usize::MAX, Ordering::Release);
    nvme::set_write_enabled(false);

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
            master_unlocked: false,
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
        /* Prefer a Popcorn volume as usb0, else blank (install target), else first. */
        let primary = (0..usb_n)
            .find(|&i| usb_msc::is_ours(i))
            .or_else(|| (0..usb_n).find(|&i| usb_msc::is_blank(i)))
            .or_else(|| (0..usb_n).find(|&i| !usb_msc::is_boot_like(i)))
            .unwrap_or(0);
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
                    /* Only a real Popcorn volume is the default boot/data disk.
                     * Blank sticks are writable but must not auto-format on boot. */
                    is_boot: usb_msc::is_ours(i),
                    install_unlocked: false,
                    protected: usb_msc::is_boot_like(i),
                    master_unlocked: false,
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
                master_unlocked: false,
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
            master_unlocked: false,
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
            master_unlocked: false,
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
        } else if is_internal_disk(d) {
            s.push_str(" MASTER-LOCKED");
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
    if matches!(t[idx].backend, Backend::Stub) {
        return Err("stub disk has no I/O");
    }
    /* Internal/NVMe may be selected for reads while MASTER-LOCKED. Writes still fail. */
    SELECTED.store(idx, Ordering::Release);
    Ok(())
}

/// Unlock Internal/NVMe writes for this boot. Never called from install, wipe, or
/// the QEMU selftest. Returns Ok(true) if unlocked, Ok(false) if armed (needs YES).
pub fn master(name: &str, confirm_yes: bool) -> Result<bool, &'static str> {
    let t = table();
    let idx = t
        .iter()
        .position(|d| d.name == name)
        .ok_or("unknown disk")?;
    if matches!(t[idx].backend, Backend::Stub | Backend::Ram) {
        return Err("master is only for internal/NVMe disks");
    }
    if !is_internal_disk(&t[idx]) {
        return Err("master is only for internal/NVMe disks");
    }
    if !confirm_yes {
        PENDING_MASTER.store(idx, Ordering::Release);
        return Ok(false);
    }
    let armed = PENDING_MASTER.load(Ordering::Acquire);
    if armed != usize::MAX && armed != idx {
        return Err("another disk is armed — master that name, or master this one without a prior arm");
    }
    t[idx].master_unlocked = true;
    if let Backend::Nvme(_) = t[idx].backend {
        nvme::set_write_enabled(true);
    }
    PENDING_MASTER.store(usize::MAX, Ordering::Release);
    crate::console_ffi::println_color(
        "master: NVMe/internal writes ENABLED until reboot",
        crate::console_ffi::COLOR_YELLOW,
    );
    Ok(true)
}

/// Start or complete a full system install onto a disk.
/// Stages `EFI/BOOT/BOOTX64.EFI` + `BOOT/KERNEL` from the live boot volume into
/// RAM, formats the target as Popcorn FAT32, writes the UEFI layout, and leaves
/// the target selected + writable (so it is both bootable and the working disk).
/// Returns Ok(true) if installed, Ok(false) if armed (needs YES).
pub fn install(name: &str, confirm_yes: bool) -> Result<bool, &'static str> {
    let t = table();
    let idx = t
        .iter()
        .position(|d| d.name == name)
        .ok_or("unknown disk")?;
    if matches!(t[idx].backend, Backend::Stub | Backend::Ram) {
        return Err("cannot install to ram/stub disks");
    }
    if is_internal_disk(&t[idx]) && !t[idx].master_unlocked {
        return Err("internal/NVMe locked — disk master <name> YES first");
    }
    if t[idx].sectors < 68_000 {
        return Err("disk too small for install (~33MiB+)");
    }
    if !confirm_yes {
        PENDING_INSTALL.store(idx, Ordering::Release);
        return Ok(false);
    }
    /* Same-line YES is confirmation enough; prior arm is optional. */
    let armed = PENDING_INSTALL.load(Ordering::Acquire);
    if armed != usize::MAX && armed != idx {
        return Err("another disk is armed — install that name, or install this one without a prior arm");
    }

    crate::console_ffi::println_color(
        "install: [1/4] staging bootloader + kernel from live boot disk...",
        crate::console_ffi::COLOR_WHITE,
    );
    let (efi, kernel) = stage_boot_payloads(idx)?;
    {
        let mut line = alloc::string::String::from("install: staged EFI=");
        append_u64(&mut line, efi.len() as u64);
        line.push_str(" kernel=");
        append_u64(&mut line, kernel.len() as u64);
        line.push_str(" bytes");
        crate::console_ffi::println_color(&line, crate::console_ffi::COLOR_LIGHT_CYAN);
    }

    /* Format target (same quick wipe as disk wipe). */
    t[idx].install_unlocked = true;
    t[idx].protected = false;
    SELECTED.store(idx, Ordering::Release);
    crate::fs::fat32::unmount();

    let sectors = t[idx].sectors;
    let ss = t[idx].sector_size as usize;
    let zero = alloc::vec![0u8; ss];
    crate::console_ffi::println_color(
        "install: [2/4] formatting target as Popcorn FAT32...",
        crate::console_ffi::COLOR_WHITE,
    );
    for lba in 0..PROTECT_ZONE.min(sectors) {
        if raw_write(&t[idx], lba, &zero) < 0 {
            return Err("install: failed clearing partition tables");
        }
    }
    let _ = raw_write(&t[idx], sectors.saturating_sub(1), &zero);

    match crate::fs::fat32::force_format() {
        Ok(()) => {}
        Err(_) => return Err("install: FAT32 format failed"),
    }

    match crate::fs::fat32::write_boot_layout(&efi, &kernel) {
        Ok(()) => {}
        Err(_) => return Err("install: failed writing/verifying boot files"),
    }

    t[idx].protected = false;
    t[idx].install_unlocked = true;
    t[idx].is_boot = true;
    PENDING_INSTALL.store(usize::MAX, Ordering::Release);
    crate::console_ffi::println_color(
        "install: complete — safe to reboot from this disk",
        crate::console_ffi::COLOR_LIGHT_GREEN,
    );
    Ok(true)
}

/// True if this ELF looks like a QEMU auto-install kernel.
/// Needles are split so this detector is not itself mistaken for that kernel.
fn kernel_is_auto_install(k: &[u8]) -> bool {
    fn has_parts(hay: &[u8], a: &[u8], b: &[u8]) -> bool {
        let n = a.len() + b.len();
        if hay.len() < n {
            return false;
        }
        for i in 0..=hay.len() - n {
            if &hay[i..i + a.len()] == a && &hay[i + a.len()..i + n] == b {
                return true;
            }
        }
        false
    }
    has_parts(k, b"rust_test_disk", b"_install") || has_parts(k, b"install-self", b"test")
}

fn ok_stage_source(d: &DiskEntry) -> bool {
    if is_internal_disk(d) || matches!(d.backend, Backend::Stub | Backend::Ram | Backend::Nvme(_)) {
        return false;
    }
    matches!(d.backend, Backend::UsbMsc(_) | Backend::Virtio(_))
}

/// Load BOOTX64.EFI + KERNEL from a FAT volume into RAM.
/// Never reads Internal/NVMe (a leftover Popcorn format there carried the
/// auto-install kernel onto the USB).
fn stage_boot_payloads(target_idx: usize) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    let t = table();
    let prev = SELECTED.load(Ordering::Acquire);
    let n = t.len();
    let mut order: Vec<usize> = Vec::new();
    if target_idx < n && ok_stage_source(&t[target_idx]) {
        order.push(target_idx);
    }
    for i in 0..n {
        if i != target_idx && t[i].is_boot && ok_stage_source(&t[i]) && !order.contains(&i) {
            order.push(i);
        }
    }
    for i in 0..n {
        if i != target_idx && t[i].class == DiskClass::Usb && ok_stage_source(&t[i]) && !order.contains(&i)
        {
            order.push(i);
        }
    }
    for i in 0..n {
        if i != target_idx && t[i].class == DiskClass::Virtio && ok_stage_source(&t[i]) && !order.contains(&i)
        {
            order.push(i);
        }
    }

    let mut found: Option<(Vec<u8>, Vec<u8>)> = None;
    for i in order {
        SELECTED.store(i, Ordering::Release);
        crate::fs::fat32::unmount();
        if crate::fs::fat32::mount_or_format().is_err() {
            continue;
        }
        let efi = match crate::fs::fat32::read_binary(&["EFI", "BOOT"], "BOOTX64.EFI") {
            Ok(v) if !v.is_empty() => v,
            _ => continue,
        };
        let kern = crate::fs::fat32::read_binary(&["BOOT"], "KERNEL")
            .or_else(|_| crate::fs::fat32::read_binary(&["EFI", "BOOT"], "KERNEL"))
            .or_else(|_| crate::fs::fat32::read_binary(&[], "KERNEL"));
        match kern {
            Ok(k) if !k.is_empty() => {
                if kernel_is_auto_install(&k) {
                    crate::console_ffi::print_color(
                        "install: refusing auto-install kernel on ",
                        crate::console_ffi::COLOR_YELLOW,
                    );
                    crate::console_ffi::println_color(t[i].name, crate::console_ffi::COLOR_YELLOW);
                    continue;
                }
                crate::console_ffi::print_color(
                    "install: staged from ",
                    crate::console_ffi::COLOR_LIGHT_CYAN,
                );
                crate::console_ffi::println_color(t[i].name, crate::console_ffi::COLOR_LIGHT_CYAN);
                found = Some((efi, k));
                break;
            }
            _ => {}
        }
    }

    SELECTED.store(prev, Ordering::Release);
    crate::fs::fat32::unmount();
    found.ok_or("no boot files found on USB (need EFI/BOOT/BOOTX64.EFI and BOOT/KERNEL; NVMe is never a source)")
}

/// Quick-wipe + Popcorn FAT32 format.
/// `disk wipe <name> YES` is enough confirmation (YES in the command is the
/// confirm). A prior bare `disk wipe <name>` still arms if preferred.
/// Returns Ok(true) if wiped+formatted, Ok(false) if armed.
pub fn wipe(name: &str, confirm_yes: bool) -> Result<bool, &'static str> {
    let t = table();
    let idx = t
        .iter()
        .position(|d| d.name == name)
        .ok_or("unknown disk")?;
    if matches!(t[idx].backend, Backend::Stub | Backend::Ram) {
        return Err("cannot wipe ram/stub disks");
    }
    if is_internal_disk(&t[idx]) && !t[idx].master_unlocked {
        return Err("internal/NVMe locked — disk master <name> YES first");
    }
    if t[idx].sectors < 68_000 {
        return Err("disk too small for FAT32 (~33MiB+)");
    }
    if !confirm_yes {
        PENDING_WIPE.store(idx, Ordering::Release);
        return Ok(false);
    }
    /* Same-line YES is confirmation enough; prior arm is optional. */
    let armed = PENDING_WIPE.load(Ordering::Acquire);
    if armed != usize::MAX && armed != idx {
        return Err("another disk is armed — wipe that name, or wipe this one without a prior arm");
    }

    /* Allow writes through the foreign-data gate for this operation. */
    t[idx].install_unlocked = true;
    t[idx].protected = false; /* wipe owns this stick for the duration */
    SELECTED.store(idx, Ordering::Release);
    crate::fs::fat32::unmount();

    let sectors = t[idx].sectors;
    let ss = t[idx].sector_size as usize;
    let zero = alloc::vec![0u8; ss];
    /* Quick wipe: destroy partition table / BPB head. Last-sector GPT backup
     * is best-effort — failing it must not abort the format (USB end-LBA
     * writes often time out on real sticks). */
    crate::console_ffi::println_color(
        "wipe: clearing partition tables...",
        crate::console_ffi::COLOR_WHITE,
    );
    for lba in 0..PROTECT_ZONE.min(sectors) {
        if raw_write(&t[idx], lba, &zero) < 0 {
            return Err("wipe write failed (USB rejected head write)");
        }
    }
    if sectors > PROTECT_ZONE {
        let _ = raw_write(&t[idx], sectors - 1, &zero);
    }

    crate::console_ffi::println_color(
        "wipe: formatting FAT32...",
        crate::console_ffi::COLOR_WHITE,
    );
    match crate::fs::fat32::force_format() {
        Ok(()) => {}
        Err(_) => return Err("FAT32 format failed after wipe"),
    }

    t[idx].protected = false;
    t[idx].install_unlocked = true;
    t[idx].is_boot = true; /* Popcorn volume — prefer for auto-select next time */
    PENDING_WIPE.store(usize::MAX, Ordering::Release);
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
        } else if is_internal_disk(d) {
            s.push_str(" MASTER-LOCKED");
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
    /* Internal/NVMe writes require `disk master <name> YES` this boot. */
    if is_internal_disk(d) && !d.master_unlocked {
        return -5;
    }
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
        /* Foreign disks (ESP, NTFS, …): block head/tail unless the user has
         * explicitly `install … YES`'d this disk — then FAT updates in the
         * reserved/FAT region must be allowed or every file write fails. */
        if holds_foreign_data(d) && !d.install_unlocked && !d.master_unlocked {
            let in_zone = lba < PROTECT_ZONE
                || (d.sectors > PROTECT_ZONE && lba >= d.sectors - PROTECT_ZONE);
            if needs_unlock(d) || in_zone {
                return -6;
            }
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
        Err(e) => {
            crate::console_ffi::print_color("install: ", crate::console_ffi::COLOR_YELLOW);
            crate::console_ffi::println_color(e, crate::console_ffi::COLOR_YELLOW);
            -1
        }
    });
    if rc == 0 {
        crate::fs::remount_selected();
    }
    rc
}

/// Returns 0 wiped+formatted, 1 need YES, -1 error.
#[no_mangle]
pub extern "C" fn rust_disk_wipe(name: *const core::ffi::c_char, yes: i32) -> i32 {
    cstr_map(name, |s| match wipe(s, yes != 0) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            crate::console_ffi::print_color("wipe: ", crate::console_ffi::COLOR_YELLOW);
            crate::console_ffi::println_color(e, crate::console_ffi::COLOR_YELLOW);
            -1
        }
    })
}

/// Returns 0 unlocked, 1 need YES, -1 error. Never invoked by boot or selftest.
#[no_mangle]
pub extern "C" fn rust_disk_master(name: *const core::ffi::c_char, yes: i32) -> i32 {
    cstr_map(name, |s| match master(s, yes != 0) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            crate::console_ffi::print_color("master: ", crate::console_ffi::COLOR_YELLOW);
            crate::console_ffi::println_color(e, crate::console_ffi::COLOR_YELLOW);
            -1
        }
    })
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

/// Automated install selftest — compiled only with `--features qemu-install-selftest`.
/// QEMU USB/virtio only; never Internal/NVMe; never linked into a flashable kernel.
#[cfg(feature = "qemu-install-selftest")]
fn running_under_hypervisor() -> bool {
    let r = unsafe { core::arch::x86_64::__cpuid(1) };
    (r.ecx & (1 << 31)) != 0
}

#[cfg(feature = "qemu-install-selftest")]
#[no_mangle]
pub extern "C" fn rust_test_disk_install() -> i32 {
    extern "C" {
        fn boot_serial_putc(c: u8);
    }
    let fail = || {
        unsafe { boot_serial_putc(b'i') };
        -1
    };
    if !running_under_hypervisor() {
        crate::console_ffi::println_color(
            "install-selftest: refused (not a VM)",
            crate::console_ffi::COLOR_YELLOW,
        );
        return fail();
    }

    let t = table();
    let n = t.len();
    let mut source = usize::MAX;
    let mut target = usize::MAX;

    /* Find a source that already has the loader (boot ESP / Popcorn stick). */
    let prev = SELECTED.load(Ordering::Acquire);
    for i in 0..n {
        if matches!(t[i].backend, Backend::Stub | Backend::Ram) {
            continue;
        }
        if is_internal_disk(&t[i]) {
            continue;
        }
        SELECTED.store(i, Ordering::Release);
        crate::fs::fat32::unmount();
        if crate::fs::fat32::mount_or_format().is_ok() && crate::fs::fat32::has_boot_loader() {
            source = i;
            break;
        }
    }
    if source == usize::MAX {
        SELECTED.store(prev, Ordering::Release);
        crate::fs::fat32::unmount();
        return fail();
    }

    /* USB/virtio only, never Internal, never the live boot source. */
    for i in 0..n {
        if i == source || is_internal_disk(&t[i]) {
            continue;
        }
        if !matches!(t[i].backend, Backend::UsbMsc(_) | Backend::Virtio(_)) {
            continue;
        }
        if t[i].sectors < 68_000 {
            continue;
        }
        target = i;
        break;
    }
    if target == usize::MAX {
        SELECTED.store(prev, Ordering::Release);
        crate::fs::fat32::unmount();
        crate::console_ffi::println_color(
            "install-selftest: no USB/virtio target (refusing NVMe fallback)",
            crate::console_ffi::COLOR_YELLOW,
        );
        return fail();
    }

    let name = t[target].name;
    SELECTED.store(prev, Ordering::Release);
    crate::fs::fat32::unmount();

    crate::console_ffi::print_color(
        "install-selftest: target=",
        crate::console_ffi::COLOR_YELLOW,
    );
    crate::console_ffi::println_color(name, crate::console_ffi::COLOR_YELLOW);

    PENDING_INSTALL.store(target, Ordering::Release);
    match install(name, true) {
        Ok(true) => {}
        _ => return fail(),
    }

    crate::fs::fat32::unmount();
    if crate::fs::fat32::mount_or_format().is_err() {
        return fail();
    }
    let efi_ok = crate::fs::fat32::read_binary(&["EFI", "BOOT"], "BOOTX64.EFI")
        .map(|v| v.len() > 1000)
        .unwrap_or(false);
    let kern_ok = crate::fs::fat32::read_binary(&["BOOT"], "KERNEL")
        .map(|v| v.len() > 100_000)
        .unwrap_or(false);
    if efi_ok && kern_ok {
        unsafe { boot_serial_putc(b'I') };
        crate::console_ffi::println_color(
            "install-selftest: PASS",
            crate::console_ffi::COLOR_LIGHT_GREEN,
        );
        0
    } else {
        crate::console_ffi::println_color(
            "install-selftest: FAIL verify",
            crate::console_ffi::COLOR_YELLOW,
        );
        fail()
    }
}
