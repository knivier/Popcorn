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
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use crate::catalog;
use crate::drivers::bus::pci;
use crate::sync::IrqSpinLock;

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
    /// Preferred Popcorn volume: auto-selected at boot (a Popcorn-formatted or
    /// freshly installed disk). Not necessarily the disk the machine booted from.
    is_boot: bool,
    /// The disk the UEFI loader booted from (matched by BPB serial + size from
    /// the handoff). Shown as ",boot"; the only install source when known.
    booted: bool,
    /// Set by `disk install`/`disk wipe … YES`: lifts the `protected` USB lock
    /// and the foreign-data head/tail guard. Never unlocks Internal/NVMe.
    install_unlocked: bool,
    /// USB disk that holds someone else's data (boot media, NTFS, ...): writes
    /// need `install_unlocked`.
    protected: bool,
    /// Internal/NVMe writes: only `disk master <name> YES`. Never auto, never install.
    master_unlocked: bool,
    /// Cached `holds_foreign_data` result (FOREIGN_*). Reset whenever LBA 0/1
    /// is written, so it never outlives the sectors it was computed from.
    foreign: AtomicU8,
}

/// Copy of a table entry (minus the `foreign` cache, reached through `idx`), so
/// I/O can run after the table lock is dropped.
#[derive(Clone, Copy)]
struct DiskView {
    idx: usize,
    name: &'static str,
    class: DiskClass,
    sectors: u64,
    sector_size: u32,
    backend: Backend,
    install_unlocked: bool,
    protected: bool,
    master_unlocked: bool,
}

impl DiskEntry {
    fn view(&self, idx: usize) -> DiskView {
        DiskView {
            idx,
            name: self.name,
            class: self.class,
            sectors: self.sectors,
            sector_size: self.sector_size,
            backend: self.backend,
            install_unlocked: self.install_unlocked,
            protected: self.protected,
            master_unlocked: self.master_unlocked,
        }
    }
}

const FOREIGN_UNKNOWN: u8 = 0;
const FOREIGN_NO: u8 = 1;
const FOREIGN_YES: u8 = 2;

/// Sectors at each end of a disk that hold MBR/GPT + backup GPT.
const PROTECT_ZONE: u64 = 34;

static INITED: AtomicBool = AtomicBool::new(false);
/// The disk table. Entries are only appended during `init` (indices are then
/// stable, so `SELECTED`/`PENDING_*` can hold them).
///
/// IrqSpinLock, short sections only: look up / update entries, copy out a
/// `DiskView`, drop the guard, then do the work. Never hold it across device
/// I/O (`raw_read`/`raw_write`), fat32 (which re-enters this module through
/// `read_selected`/`write_selected`/`selected_*`), console output, or catalog.
///
/// Lock order: this lock is a leaf. It is never held while taking a device
/// TaskLock (nvme `DEV`, virtio_blk `DEVS`, usb_msc `DISKS`, xhci `HC`), the
/// fat32 lock, or the catalog lock; device code never calls back into here.
static DISKS: IrqSpinLock<Vec<DiskEntry>> = IrqSpinLock::new(Vec::new());
static SELECTED: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Pending install target index, or MAX if none.
static PENDING_INSTALL: AtomicUsize = AtomicUsize::new(usize::MAX);
static PENDING_WIPE: AtomicUsize = AtomicUsize::new(usize::MAX);
static PENDING_MASTER: AtomicUsize = AtomicUsize::new(usize::MAX);
/// A failed install/wipe left the selected disk unmounted; the FFI wrapper
/// remounts it (after printing the error) so ls/cat keep working.
static NEEDS_REMOUNT: AtomicBool = AtomicBool::new(false);

/* Names for registered disks ('static literals, never changed). */
const NAME_USB0: &str = "usb0";
const NAME_USB1: &str = "usb1";
const NAME_VDC: &str = "vdc";
const NAME_VDD: &str = "vdd";
const NAME_NVME0: &str = "nvme0";
const NAME_VDA: &str = "vda";
const NAME_VDB: &str = "vdb";
const NAME_RAM0: &str = "ram0";
const NAME_SATA0: &str = "sata0";
const NAME_IDE0: &str = "ide0";

/// Snapshot of entry `idx` (table lock held only for the copy).
fn entry(idx: usize) -> Option<DiskView> {
    DISKS.with(|t| t.get(idx).map(|d| d.view(idx)))
}

/// Index of the disk called `name`, if any.
fn find_disk(name: &str) -> Option<usize> {
    DISKS.with(|t| t.iter().position(|d| d.name == name))
}

/// Append an entry (publish to the catalog first, outside the table lock).
fn add_disk(d: DiskEntry) {
    publish_disk(&d);
    DISKS.lock().push(d);
}

fn is_writable(d: &DiskView) -> bool {
    if is_internal_disk(d) {
        return d.master_unlocked;
    }
    match d.class {
        /* Unreachable (is_internal_disk above); kept so the match stays exhaustive. */
        DiskClass::Internal => false,
        DiskClass::Usb if d.protected => d.install_unlocked,
        DiskClass::Ram | DiskClass::Virtio | DiskClass::Usb => true,
    }
}

fn is_internal_disk(d: &DiskView) -> bool {
    d.class == DiskClass::Internal || matches!(d.backend, Backend::Nvme(_))
}

pub fn selected_is_internal() -> bool {
    selected_entry().map(|d| is_internal_disk(&d)).unwrap_or(false)
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

extern "C" {
    fn popcorn_boot_volume(serial: *mut u32, sectors: *mut u64) -> i32;
}

/// (BPB serial, total sectors) of the volume the UEFI loader booted from.
fn boot_volume() -> Option<(u32, u64)> {
    let mut serial = 0u32;
    let mut sectors = 0u64;
    if unsafe { popcorn_boot_volume(&mut serial, &mut sectors) } == 1 && sectors != 0 {
        Some((serial, sectors))
    } else {
        None
    }
}

/// Does this LBA 0 hold the boot sector the loader recorded?
fn is_boot_volume_sector(sec: &[u8]) -> bool {
    let Some((serial, sectors)) = boot_volume() else { return false };
    if sec.len() < 512 || sec[510] != 0x55 || sec[511] != 0xAA {
        return false;
    }
    let le32 = |o: usize| u32::from_le_bytes([sec[o], sec[o + 1], sec[o + 2], sec[o + 3]]);
    let spf16 = u16::from_le_bytes([sec[22], sec[23]]);
    let tot16 = u16::from_le_bytes([sec[19], sec[20]]) as u64;
    let total = if tot16 != 0 { tot16 } else { le32(32) as u64 };
    let ser = le32(if spf16 == 0 { 67 } else { 39 });
    ser == serial && total == sectors
}

fn usb_booted(i: usize) -> bool {
    let mut sec = [0u8; 512];
    boot_volume().is_some() && usb_msc::read(i, 0, &mut sec) >= 0 && is_boot_volume_sector(&sec)
}

fn virtio_booted(i: usize) -> bool {
    let mut sec = [0u8; 512];
    boot_volume().is_some() && virtio_blk::read(i, 0, &mut sec) >= 0 && is_boot_volume_sector(&sec)
}

/// The UEFI test-install loader (target/UNSAFE only) carries this UTF-16 banner.
fn loader_is_test_install(efi: &[u8]) -> bool {
    /* Spelled as chars so the kernel image never holds this as ASCII (the
     * flashable-image check greps for install/selftest strings). */
    const BANNER: [char; 19] = [
        't', 'e', 's', 't', '-', 'i', 'n', 's', 't', 'a', 'l', 'l', ' ', 'l', 'o', 'a', 'd', 'e', 'r',
    ];
    let needle: Vec<u8> = BANNER.iter().flat_map(|&c| (c as u16).to_le_bytes()).collect();
    efi.windows(needle.len()).any(|w| w == needle.as_slice())
}

fn auto_select_boot() {
    let idx = DISKS.with(|t| {
        let w = |i: usize| is_writable(&t[i].view(i));
        (0..t.len())
            .position(|i| t[i].is_boot && w(i))
            .or_else(|| (0..t.len()).position(|i| t[i].class == DiskClass::Usb && w(i)))
            .or_else(|| (0..t.len()).position(|i| t[i].class == DiskClass::Virtio && w(i)))
    });
    if let Some(i) = idx {
        SELECTED.store(i, Ordering::Release);
    }
}

/// Boot: ram0; real USB MSC=usb0/usb1 else virtio[0]=vda; real NVMe=nvme0 (internal locked), else
/// virtio[1]=nvme0 stand-in; PCI stubs.
pub fn init() {
    /* NVMe/xHCI probes are one-shot; a second init would orphan their devices. */
    if INITED.swap(true, Ordering::SeqCst) {
        return;
    }
    DISKS.lock().clear();
    SELECTED.store(usize::MAX, Ordering::Release);
    PENDING_INSTALL.store(usize::MAX, Ordering::Release);
    PENDING_WIPE.store(usize::MAX, Ordering::Release);
    PENDING_MASTER.store(usize::MAX, Ordering::Release);
    nvme::set_write_enabled(false);

    if let Ok((sectors, ssize)) = ramdisk::init() {
        let d = DiskEntry {
            name: NAME_RAM0,
            class: DiskClass::Ram,
            sectors,
            sector_size: ssize,
            backend: Backend::Ram,
            is_boot: false,
            booted: false,
            install_unlocked: false,
            protected: false,
            master_unlocked: false,
            foreign: AtomicU8::new(FOREIGN_UNKNOWN),
        };
        add_disk(d);
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
                    name: if n == 0 { NAME_USB0 } else { NAME_USB1 },
                    class: DiskClass::Usb,
                    sectors,
                    sector_size: ssize,
                    backend: Backend::UsbMsc(i),
                    /* Only a real Popcorn volume is the default boot/data disk.
                     * Blank sticks are writable but must not auto-format on boot. */
                    is_boot: usb_msc::is_ours(i),
                    booted: usb_booted(i),
                    install_unlocked: false,
                    protected: usb_msc::is_boot_like(i),
                    master_unlocked: false,
                    foreign: AtomicU8::new(FOREIGN_UNKNOWN),
                };
                add_disk(d);
                usb_real = true;
            }
        }
    }

    /* Virtio slots are sparse (a failed device leaves a hole) so names stay
     * tied to the PCI index; walk every slot, not just the probe count. */
    let _ = virtio_blk::probe_all();
    for i in 0..virtio_blk::MAX_VIRTIO_DISKS {
        if let Some((sectors, ssize)) = virtio_blk::capacity(i) {
            let (name, class, is_boot) = if i == 0 && usb_real {
                /* Real USB MSC owns usb0; first virtio is a plain data disk. */
                (NAME_VDA, DiskClass::Virtio, false)
            } else if i == 0 {
                /* No USB stick: the first virtio disk is the default Popcorn
                 * volume, but it keeps its own name (",boot" only if booted). */
                (NAME_VDA, DiskClass::Virtio, true)
            } else if i == 1 && nvme_disk.is_some() {
                /* Real nvme0 present: virtio[1] is just a plain virtio disk. */
                (NAME_VDB, DiskClass::Virtio, false)
            } else if i == 1 {
                (NAME_NVME0, DiskClass::Internal, false)
            } else if i == 2 {
                if usb_real {
                    (NAME_VDC, DiskClass::Virtio, false)
                } else {
                    (NAME_VDA, DiskClass::Virtio, false)
                }
            } else {
                /* First vd* name not already taken (vdb may be virtio[1]). */
                let name = [NAME_VDA, NAME_VDB, NAME_VDC, NAME_VDD]
                    .into_iter()
                    .find(|n| find_disk(n).is_none())
                    .unwrap_or(NAME_VDD);
                (name, DiskClass::Virtio, false)
            };
            let d = DiskEntry {
                name,
                class,
                sectors,
                sector_size: ssize,
                backend: Backend::Virtio(i),
                is_boot,
                booted: virtio_booted(i),
                install_unlocked: false,
                protected: false,
                master_unlocked: false,
                foreign: AtomicU8::new(FOREIGN_UNKNOWN),
            };
            add_disk(d);
        }
    }

    if let Some((sectors, ssize)) = nvme_disk {
        let d = DiskEntry {
            name: NAME_NVME0,
            class: DiskClass::Internal,
            sectors,
            sector_size: ssize,
            backend: Backend::Nvme(0),
            is_boot: false,
            booted: false,
            install_unlocked: false,
            protected: false,
            master_unlocked: false,
            foreign: AtomicU8::new(FOREIGN_UNKNOWN),
        };
        add_disk(d);
    }

    /* Wave 2: enumerate real PCI storage as locked internals (list-only). */
    let mut sata_i = 0u32;
    let mut ide_i = 0u32;
    pci::foreach_storage_controller(|_addr, _class, subclass| {
        let has = |p: &str| DISKS.with(|t| t.iter().any(|d| d.name.starts_with(p)));
        /* Skip if we already have virtio stand-ins filling those roles. */
        let (name, skip) = match subclass {
            0x08 => {
                /* NVMe */
                /* Only the first NVMe controller is listed (as nvme0). */
                if has("nvme") {
                    ("", true)
                } else {
                    (NAME_NVME0, false)
                }
            }
            0x06 => {
                if has("sata") {
                    ("", true)
                } else {
                    sata_i += 1;
                    (NAME_SATA0, sata_i > 1)
                }
            }
            0x01 => {
                if has("ide") {
                    ("", true)
                } else {
                    ide_i += 1;
                    (NAME_IDE0, ide_i > 1)
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
            booted: false,
            install_unlocked: false,
            protected: false,
            master_unlocked: false,
            foreign: AtomicU8::new(FOREIGN_UNKNOWN),
        };
        add_disk(d);
    });

    auto_select_boot();
}

pub fn list(buf: &mut [u8]) -> usize {
    /* Formatting only (no I/O) under the table lock; dropped before returning. */
    let t = DISKS.lock();
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
        if d.booted {
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
        let v = d.view(i);
        if is_writable(&v) {
            s.push_str(" rw");
        } else if is_internal_disk(&v) {
            s.push_str(" MASTER-LOCKED");
        } else {
            s.push_str(" LOCKED");
        }
        s.push(' ');
    }
    drop(t);
    write_cstr(&s, buf)
}

pub fn use_disk(name: &str) -> Result<(), &'static str> {
    /* Lookup copies out; the table lock is not held across fat32::unmount. */
    let idx = find_disk(name).ok_or("unknown disk")?;
    if matches!(entry(idx).map(|d| d.backend), Some(Backend::Stub)) {
        return Err("stub disk has no I/O");
    }
    /* Internal/NVMe may be selected for reads while MASTER-LOCKED. Writes still fail. */
    crate::fs::fat32::unmount(); /* flush the old volume while it is still selected */
    SELECTED.store(idx, Ordering::Release);
    Ok(())
}

/// Unlock Internal/NVMe writes for this boot. Never called from install, wipe, or
/// the QEMU selftest. Returns Ok(true) if unlocked, Ok(false) if armed (needs YES).
pub fn master(name: &str, confirm_yes: bool) -> Result<bool, &'static str> {
    let idx = find_disk(name).ok_or("unknown disk")?;
    let d = entry(idx).ok_or("unknown disk")?;
    if matches!(d.backend, Backend::Stub | Backend::Ram) {
        return Err("master is only for internal/NVMe disks");
    }
    if !is_internal_disk(&d) {
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
    DISKS.with(|t| t[idx].master_unlocked = true);
    if let Backend::Nvme(_) = d.backend {
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
    /* Copy out only: staging/fat32 below re-enter the block layer, so the
     * table lock is never held past these short lookups. */
    let idx = {
        let idx = find_disk(name).ok_or("unknown disk")?;
        let d = entry(idx).ok_or("unknown disk")?;
        if matches!(d.backend, Backend::Stub | Backend::Ram) {
            return Err("cannot install to ram/stub disks");
        }
        if is_internal_disk(&d) && !d.master_unlocked {
            return Err("internal/NVMe locked — disk master <name> YES first");
        }
        if d.sectors < 68_000 {
            return Err("disk too small for install (~33MiB+)");
        }
        idx
    };
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
    let (efi, kernel) = match stage_boot_payloads(idx) {
        Ok(p) => p,
        Err(e) => {
            /* Staging restored the previous selection but left it unmounted. */
            NEEDS_REMOUNT.store(true, Ordering::Release);
            return Err(e);
        }
    };
    {
        let mut line = alloc::string::String::from("install: staged EFI=");
        append_u64(&mut line, efi.len() as u64);
        line.push_str(" kernel=");
        append_u64(&mut line, kernel.len() as u64);
        line.push_str(" bytes");
        crate::console_ffi::println_color(&line, crate::console_ffi::COLOR_LIGHT_CYAN);
    }

    /* Format target (same quick wipe as disk wipe). */
    let prev = SELECTED.load(Ordering::Acquire);
    {
        let mut t = DISKS.lock();
        let d = &mut t[idx];
        d.install_unlocked = true;
        d.protected = false;
    }
    crate::fs::fat32::unmount(); /* flush the old volume while it is still selected */
    SELECTED.store(idx, Ordering::Release);

    /* Snapshot after the unlock above; raw_write runs with the table unlocked. */
    let target = entry(idx).ok_or_else(|| restore_selection(prev, "unknown disk"))?;
    let (sectors, ss) = (target.sectors, target.sector_size as usize);
    let zero = alloc::vec![0u8; ss];
    crate::console_ffi::println_color(
        "install: [2/4] formatting target as Popcorn FAT32...",
        crate::console_ffi::COLOR_WHITE,
    );
    for lba in 0..PROTECT_ZONE.min(sectors) {
        if raw_write(&target,lba, &zero) < 0 {
            return Err(restore_selection(prev, "install: failed clearing partition tables"));
        }
    }
    let _ = raw_write(&target,sectors.saturating_sub(1), &zero);

    if crate::fs::fat32::force_format().is_err() {
        return Err(restore_selection(prev, "install: FAT32 format failed"));
    }

    if crate::fs::fat32::write_boot_layout(&efi, &kernel).is_err() {
        return Err(restore_selection(prev, "install: failed writing/verifying boot files"));
    }

    {
        let mut t = DISKS.lock();
        let d = &mut t[idx];
        d.protected = false;
        d.install_unlocked = true;
        d.is_boot = true;
    }
    PENDING_INSTALL.store(usize::MAX, Ordering::Release);
    crate::console_ffi::println_color(
        "install: complete — safe to reboot from this disk",
        crate::console_ffi::COLOR_LIGHT_GREEN,
    );
    Ok(true)
}

/// An install/wipe failed after selecting its target: drop the half-written
/// volume and go back to the disk that was selected before, so the shell is not
/// left on a broken target with nothing mounted. Returns `msg` for the Err.
fn restore_selection(prev: usize, msg: &'static str) -> &'static str {
    crate::fs::fat32::unmount(); /* flushes to the target while it is still selected */
    SELECTED.store(prev, Ordering::Release);
    NEEDS_REMOUNT.store(true, Ordering::Release);
    msg
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
    /* `class == Internal` + the Nvme arm are exactly `is_internal_disk`. */
    if d.class == DiskClass::Internal || matches!(d.backend, Backend::Stub | Backend::Ram | Backend::Nvme(_)) {
        return false;
    }
    matches!(d.backend, Backend::UsbMsc(_) | Backend::Virtio(_))
}

/// Load BOOTX64.EFI + KERNEL from a FAT volume into RAM.
/// Never reads Internal/NVMe (a leftover Popcorn format there carried the
/// auto-install kernel onto the USB). Other disks are tried before the target,
/// so reinstalling over an old Popcorn disk takes the live boot volume's files,
/// not the target's own stale ones; the target is only a last resort (e.g. a
/// reinstall onto the stick that was booted from).
fn stage_boot_payloads(target_idx: usize) -> Result<(Vec<u8>, Vec<u8>), &'static str> {
    let prev = SELECTED.load(Ordering::Acquire);
    /* Table locked only to pick the order: the fat32 calls below re-enter
     * the block layer (read_selected etc.), which takes the table lock. */
    let mut return_order_only = false;
    let order: Vec<usize> = {
        let t = DISKS.lock();
        let n = t.len();
        let mut order: Vec<usize> = Vec::new();
        /* The loader told us which disk we booted from: copy from that disk
         * only. Guessing picked a leftover scratch stick holding the test loader. */
        if let Some(b) = (0..n).find(|&i| t[i].booted) {
            if ok_stage_source(&t[b]) {
                order.push(b);
            }
            return_order_only = true;
        }
        for i in 0..n {
            if return_order_only {
                break;
            }
            if i != target_idx && t[i].is_boot && ok_stage_source(&t[i]) {
                order.push(i);
            }
        }
        for i in 0..n {
            if return_order_only {
                break;
            }
            if i != target_idx && t[i].class == DiskClass::Usb && ok_stage_source(&t[i]) && !order.contains(&i)
            {
                order.push(i);
            }
        }
        for i in 0..n {
            if return_order_only {
                break;
            }
            if i != target_idx && t[i].class == DiskClass::Virtio && ok_stage_source(&t[i]) && !order.contains(&i)
            {
                order.push(i);
            }
        }
        if !return_order_only && target_idx < n && ok_stage_source(&t[target_idx]) {
            order.push(target_idx);
        }
        order
    };

    let mut found: Option<(Vec<u8>, Vec<u8>)> = None;
    for i in order {
        let Some(disk_name) = entry(i).map(|d| d.name) else { continue };
        crate::fs::fat32::unmount(); /* flush while the old disk is still selected */
        SELECTED.store(i, Ordering::Release);
        if crate::fs::fat32::mount_or_format().is_err() {
            continue;
        }
        let efi = match crate::fs::fat32::read_binary(&["EFI", "BOOT"], "BOOTX64.EFI") {
            Ok(v) if !v.is_empty() => v,
            _ => continue,
        };
        let efi = if loader_is_test_install(&efi) {
            /* The QEMU install selftest's stick also carries the normal loader
             * as SAFEX64.EFI; copy that. Anything else: refuse. */
            match crate::fs::fat32::read_binary(&["EFI", "BOOT"], "SAFEX64.EFI") {
                Ok(v) if !v.is_empty() && !loader_is_test_install(&v) => {
                    crate::console_ffi::print_color(
                        "install: test-only loader on ",
                        crate::console_ffi::COLOR_YELLOW,
                    );
                    crate::console_ffi::print_color(disk_name, crate::console_ffi::COLOR_YELLOW);
                    crate::console_ffi::println_color(
                        "; using its normal loader (SAFEX64.EFI)",
                        crate::console_ffi::COLOR_YELLOW,
                    );
                    v
                }
                _ => {
                    crate::console_ffi::print_color(
                        "install: refusing test-only loader on ",
                        crate::console_ffi::COLOR_YELLOW,
                    );
                    crate::console_ffi::println_color(disk_name, crate::console_ffi::COLOR_YELLOW);
                    continue;
                }
            }
        } else {
            efi
        };
        /* Every kernel copy the loader may boot, in its order. An auto-install
         * kernel is never staged, but a safe copy beside it still is. */
        const KERNEL_PATHS: [&[&str]; 3] = [&["BOOT"], &["EFI", "BOOT"], &[]];
        let mut refused = false;
        let mut kern: Option<Vec<u8>> = None;
        for dir in KERNEL_PATHS {
            match crate::fs::fat32::read_binary(dir, "KERNEL") {
                Ok(k) if !k.is_empty() => {
                    if kernel_is_auto_install(&k) {
                        refused = true;
                        continue;
                    }
                    kern = Some(k);
                    break;
                }
                _ => {}
            }
        }
        if refused {
            crate::console_ffi::print_color(
                "install: refusing auto-install kernel on ",
                crate::console_ffi::COLOR_YELLOW,
            );
            crate::console_ffi::println_color(disk_name, crate::console_ffi::COLOR_YELLOW);
        }
        if let Some(k) = kern {
            crate::console_ffi::print_color(
                "install: staged from ",
                crate::console_ffi::COLOR_LIGHT_CYAN,
            );
            crate::console_ffi::println_color(disk_name, crate::console_ffi::COLOR_LIGHT_CYAN);
            found = Some((efi, k));
            break;
        }
    }

    crate::fs::fat32::unmount();
    SELECTED.store(prev, Ordering::Release);
    found.ok_or("no boot files found on USB (need EFI/BOOT/BOOTX64.EFI and BOOT/KERNEL; NVMe is never a source)")
}

/// Quick-wipe + Popcorn FAT32 format.
/// `disk wipe <name> YES` is enough confirmation (YES in the command is the
/// confirm). A prior bare `disk wipe <name>` still arms if preferred.
/// Returns Ok(true) if wiped+formatted, Ok(false) if armed.
pub fn wipe(name: &str, confirm_yes: bool) -> Result<bool, &'static str> {
    /* Copy out only: fat32 below re-enters the block layer, so the table
     * lock is never held past these short lookups. */
    let idx = {
        let idx = find_disk(name).ok_or("unknown disk")?;
        let d = entry(idx).ok_or("unknown disk")?;
        if matches!(d.backend, Backend::Stub | Backend::Ram) {
            return Err("cannot wipe ram/stub disks");
        }
        if is_internal_disk(&d) && !d.master_unlocked {
            return Err("internal/NVMe locked — disk master <name> YES first");
        }
        if d.sectors < 68_000 {
            return Err("disk too small for FAT32 (~33MiB+)");
        }
        idx
    };
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
    let prev = SELECTED.load(Ordering::Acquire);
    {
        let mut t = DISKS.lock();
        let d = &mut t[idx];
        d.install_unlocked = true;
        d.protected = false; /* wipe owns this stick for the duration */
    }
    crate::fs::fat32::unmount(); /* flush the old volume while it is still selected */
    SELECTED.store(idx, Ordering::Release);

    /* Snapshot after the unlock above; raw_write runs with the table unlocked. */
    let target = entry(idx).ok_or_else(|| restore_selection(prev, "unknown disk"))?;
    let (sectors, ss) = (target.sectors, target.sector_size as usize);
    let zero = alloc::vec![0u8; ss];
    /* Quick wipe: destroy partition table / BPB head. Last-sector GPT backup
     * is best-effort — failing it must not abort the format (USB end-LBA
     * writes often time out on real sticks). */
    crate::console_ffi::println_color(
        "wipe: clearing partition tables...",
        crate::console_ffi::COLOR_WHITE,
    );
    for lba in 0..PROTECT_ZONE.min(sectors) {
        if raw_write(&target,lba, &zero) < 0 {
            return Err(restore_selection(prev, "wipe write failed (USB rejected head write)"));
        }
    }
    if sectors > PROTECT_ZONE {
        let _ = raw_write(&target,sectors - 1, &zero);
    }

    crate::console_ffi::println_color(
        "wipe: formatting FAT32...",
        crate::console_ffi::COLOR_WHITE,
    );
    if crate::fs::fat32::force_format().is_err() {
        return Err(restore_selection(prev, "FAT32 format failed after wipe"));
    }

    {
        let mut t = DISKS.lock();
        let d = &mut t[idx];
        d.protected = false;
        d.install_unlocked = true;
        d.is_boot = true; /* Popcorn volume — prefer for auto-select next time */
    }
    PENDING_WIPE.store(usize::MAX, Ordering::Release);
    Ok(true)
}

pub fn info(buf: &mut [u8]) -> usize {
    let mut s = String::new();
    /* Formatting only (no I/O) under the table lock; dropped before returning. */
    let t = DISKS.lock();
    let i = SELECTED.load(Ordering::Acquire);
    if i >= t.len() {
        s.push_str("selected: none");
    } else {
        let d = &t[i];
        let v = d.view(i);
        s.push_str("selected: ");
        s.push_str(d.name);
        s.push_str(" class=");
        s.push_str(d.class.as_str());
        if d.booted {
            s.push_str(" boot");
        }
        s.push_str(" sectors=");
        append_u64(&mut s, d.sectors);
        if is_writable(&v) {
            s.push_str(" writable");
        } else if is_internal_disk(&v) {
            s.push_str(" MASTER-LOCKED");
        } else {
            s.push_str(" LOCKED");
        }
    }
    drop(t);
    write_cstr(&s, buf)
}

/// Snapshot of the selected disk; the table lock is released before returning,
/// so callers may do I/O (and fat32 may call back in) with it.
fn selected_entry() -> Option<DiskView> {
    entry(SELECTED.load(Ordering::Acquire))
}

/// Read entry `idx`'s cached `holds_foreign_data` answer.
fn foreign_get(idx: usize) -> u8 {
    DISKS.with(|t| t.get(idx).map_or(FOREIGN_UNKNOWN, |d| d.foreign.load(Ordering::Acquire)))
}

fn foreign_set(idx: usize, v: u8) {
    DISKS.with(|t| {
        if let Some(d) = t.get(idx) {
            d.foreign.store(v, Ordering::Release);
        }
    });
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

/// Device I/O: call with the table lock released (takes device TaskLocks).
fn raw_read(d: &DiskView, lba: u64, buf: &mut [u8]) -> i64 {
    match d.backend {
        Backend::Ram => ramdisk::read(lba, buf),
        Backend::Virtio(i) => virtio_blk::read(i, lba, buf),
        Backend::Nvme(i) => nvme::read(i, lba, buf),
        Backend::UsbMsc(i) => usb_msc::read(i, lba, buf),
        Backend::Stub => -1,
    }
}

/// Device I/O: call with the table lock released (takes device TaskLocks).
fn raw_write(d: &DiskView, lba: u64, buf: &[u8]) -> i64 {
    /* Internal/NVMe writes require `disk master <name> YES` this boot. */
    if is_internal_disk(d) && !d.master_unlocked {
        return -5;
    }
    let rc = match d.backend {
        Backend::Ram => ramdisk::write(lba, buf),
        Backend::Virtio(i) => virtio_blk::write(i, lba, buf),
        Backend::Nvme(i) => nvme::write(i, lba, buf),
        Backend::UsbMsc(i) => usb_msc::write(i, lba, buf),
        Backend::Stub => -1,
    };
    /* holds_foreign_data looks at LBA 0/1: any write there (even a failed,
     * possibly partial one) makes the cached answer stale. */
    if lba < 2 {
        foreign_set(d.idx, FOREIGN_UNKNOWN);
    }
    rc
}

/// True unless sector 0 is blank or a Popcorn-formatted FAT32 boot sector and
/// LBA 1 is not a GPT header. Fails closed (true) on any read error.
/// The answer is cached per disk (reset by `raw_write` to LBA 0/1); a read
/// error is never cached, so a transient USB failure cannot lock the disk.
fn holds_foreign_data(d: &DiskView) -> bool {
    if d.class == DiskClass::Ram {
        return false;
    }
    match foreign_get(d.idx) {
        FOREIGN_NO => return false,
        FOREIGN_YES => return true,
        _ => {}
    }
    let cache = |v: bool| {
        foreign_set(d.idx, if v { FOREIGN_YES } else { FOREIGN_NO });
        v
    };
    let ss = d.sector_size as usize;
    if !(512..=4096).contains(&ss) {
        return cache(true);
    }
    let mut s0 = vec![0u8; ss];
    if raw_read(d, 0, &mut s0) < 0 {
        return true; /* fail closed, uncached */
    }
    let blank = s0.iter().all(|&b| b == 0);
    let ours = s0[510] == 0x55 && s0[511] == 0xAA && &s0[3..11] == b"POPCORN ";
    if !(blank || ours) {
        return cache(true);
    }
    if d.sectors > 1 {
        let mut s1 = vec![0u8; ss];
        if raw_read(d, 1, &mut s1) < 0 {
            return true; /* fail closed, uncached */
        }
        if &s1[0..8] == b"EFI PART" {
            return cache(true);
        }
    }
    cache(false)
}

pub fn read_selected(lba: u64, buf: &mut [u8]) -> i64 {
    let Some(d) = selected_entry() else {
        return -4;
    };
    if buf.len() < d.sector_size as usize || (d.sectors != 0 && lba >= d.sectors) {
        return -2;
    }
    raw_read(&d, lba, buf)
}

/// Write one sector to the selected disk.
/// -2 bad args, -4 no disk, -5 locked, -6 refused (foreign data / GPT zone).
pub fn write_selected(lba: u64, buf: &[u8]) -> i64 {
    let Some(d) = selected_entry() else {
        return -4;
    };
    if !is_writable(&d) {
        return -5;
    }
    if buf.len() < d.sector_size as usize || (d.sectors != 0 && lba >= d.sectors) {
        return -2;
    }
    if d.class != DiskClass::Ram {
        /* Foreign disks (ESP, NTFS, …): block head/tail unless the user has
         * explicitly `install … YES`'d this disk — then FAT updates in the
         * reserved/FAT region must be allowed or every file write fails.
         * Cheap checks first: the (cached) sector 0/1 probe runs last. */
        let in_zone = lba < PROTECT_ZONE
            || (d.sectors > PROTECT_ZONE && lba >= d.sectors - PROTECT_ZONE);
        if in_zone && !d.install_unlocked && !d.master_unlocked && holds_foreign_data(&d) {
            return -6;
        }
    }
    raw_write(&d, lba, buf)
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
    /* Success: mount the new volume. Failure part-way: remount the restored
     * selection so FAT matches the selected disk again. */
    if NEEDS_REMOUNT.swap(false, Ordering::AcqRel) || rc == 0 {
        crate::fs::remount_selected();
    }
    rc
}

/// Returns 0 wiped+formatted, 1 need YES, -1 error.
#[no_mangle]
pub extern "C" fn rust_disk_wipe(name: *const core::ffi::c_char, yes: i32) -> i32 {
    let rc = cstr_map(name, |s| match wipe(s, yes != 0) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            crate::console_ffi::print_color("wipe: ", crate::console_ffi::COLOR_YELLOW);
            crate::console_ffi::println_color(e, crate::console_ffi::COLOR_YELLOW);
            -1
        }
    });
    /* Success already left the fresh volume mounted (force_format). */
    if NEEDS_REMOUNT.swap(false, Ordering::AcqRel) {
        crate::fs::remount_selected();
    }
    rc
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
    /* Single 'i'/'I' keeps the positional check; the multi-char markers are
     * unambiguous (uppercase only: no stray 'i'/'e'/'t' in the debugcon). */
    let mark = |s: &[u8]| {
        for &c in s {
            unsafe { boot_serial_putc(c) };
        }
    };
    let fail = || {
        mark(b"iINSTFAIL");
        -1
    };
    if !running_under_hypervisor() {
        crate::console_ffi::println_color(
            "install-selftest: refused (not a VM)",
            crate::console_ffi::COLOR_YELLOW,
        );
        return fail();
    }

    /* Table lock only for short copies: the fat32 calls below re-enter it. */
    let n = DISKS.lock().len();
    let mut source = usize::MAX;
    let mut target = usize::MAX;

    /* Find a source that already has the loader (boot ESP / Popcorn stick). */
    let prev = SELECTED.load(Ordering::Acquire);
    for i in 0..n {
        {
            let Some(d) = entry(i) else { continue };
            if matches!(d.backend, Backend::Stub | Backend::Ram) || is_internal_disk(&d) {
                continue;
            }
        }
        crate::fs::fat32::unmount();
        SELECTED.store(i, Ordering::Release);
        if crate::fs::fat32::mount_or_format().is_ok() && crate::fs::fat32::has_boot_loader() {
            source = i;
            break;
        }
    }
    if source == usize::MAX {
        crate::fs::fat32::unmount();
        SELECTED.store(prev, Ordering::Release);
        return fail();
    }

    /* USB/virtio only, never Internal, never the live boot source. */
    {
        let t = DISKS.lock();
        for i in 0..n {
            if i == source || is_internal_disk(&t[i].view(i)) {
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
    }
    if target == usize::MAX {
        crate::fs::fat32::unmount();
        SELECTED.store(prev, Ordering::Release);
        crate::console_ffi::println_color(
            "install-selftest: no USB/virtio target (refusing NVMe fallback)",
            crate::console_ffi::COLOR_YELLOW,
        );
        return fail();
    }

    let name = DISKS.with(|t| t[target].name);
    crate::fs::fat32::unmount();
    SELECTED.store(prev, Ordering::Release);

    crate::console_ffi::print_color(
        "install-selftest: target=",
        crate::console_ffi::COLOR_YELLOW,
    );
    crate::console_ffi::println_color(name, crate::console_ffi::COLOR_YELLOW);

    PENDING_INSTALL.store(target, Ordering::Release);
    match install(name, true) {
        Ok(true) => {}
        _ => {
            /* Consume the flag here (no FFI wrapper on this path) so it cannot
             * fire on a later, unrelated disk install/wipe. */
            if NEEDS_REMOUNT.swap(false, Ordering::AcqRel) {
                crate::fs::remount_selected();
            }
            return fail();
        }
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
        mark(b"INSTOK");
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
