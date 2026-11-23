//! Drive catalog: init_drive → /dev nodes + commands.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::catalog;
use crate::sync::IrqSpinLock;

use super::backends::{clock, cpuinfo, fb, kbd, meminfo, null, serial, vga, zero};
use super::bus::pci;

extern "C" {
    fn device_register_rust(name: *const u8);
    /// C `bool console_fb_active(void)`; Rust `bool` matches C `_Bool` (only AL defined).
    fn console_fb_active() -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Char,
    Screen,
    Info,
}

#[derive(Clone, Copy)]
pub struct DriveEntry {
    pub name: &'static str,
    /// Short plain-English label for listings.
    pub title: &'static str,
    pub kind: DriveKind,
    pub ready: bool,
    pub dev_name: &'static str,
}

/// Leaf lock: probes, device registration and the catalog are called only
/// after it is dropped (entries are copied out first).
static DRIVES: IrqSpinLock<Vec<DriveEntry>> = IrqSpinLock::new(Vec::new());
static INIT: AtomicBool = AtomicBool::new(false);

fn catalog() -> Vec<DriveEntry> {
    alloc::vec![
        DriveEntry {
            name: "null",
            title: "trash",
            kind: DriveKind::Char,
            ready: false,
            dev_name: "null",
        },
        DriveEntry {
            name: "zero",
            title: "zeros",
            kind: DriveKind::Char,
            ready: false,
            dev_name: "zero",
        },
        DriveEntry {
            name: "ttyS0",
            title: "serial",
            kind: DriveKind::Char,
            ready: false,
            dev_name: "ttyS0",
        },
        DriveEntry {
            name: "kbd",
            title: "keyboard",
            kind: DriveKind::Char,
            ready: false,
            dev_name: "kbd",
        },
        DriveEntry {
            name: "tty0",
            title: "text-screen",
            kind: DriveKind::Screen,
            ready: false,
            dev_name: "tty0",
        },
        DriveEntry {
            name: "fb0",
            title: "picture-screen",
            kind: DriveKind::Screen,
            ready: false,
            dev_name: "fb0",
        },
        DriveEntry {
            name: "mem",
            title: "memory",
            kind: DriveKind::Info,
            ready: false,
            dev_name: "meminfo",
        },
        DriveEntry {
            name: "cpu",
            title: "processor",
            kind: DriveKind::Info,
            ready: false,
            dev_name: "cpu",
        },
        DriveEntry {
            name: "clock",
            title: "pit",
            kind: DriveKind::Info,
            ready: false,
            dev_name: "clock",
        },
    ]
}

/// Run `f` on the drive table (seeded on first use). Keep `f` short and
/// self-contained: it runs with interrupts off and must not call out.
fn with_table<R>(f: impl FnOnce(&mut Vec<DriveEntry>) -> R) -> R {
    let mut t = DRIVES.lock();
    if t.is_empty() {
        *t = catalog();
    }
    f(&mut t)
}

/// Copy of the whole table, for formatting without the lock.
fn snapshot() -> Vec<DriveEntry> {
    with_table(|t| t.clone())
}

fn register_dev(name: &str, kind: DriveKind, id: u32) {
    let mut buf = [0u8; 32];
    let b = name.as_bytes();
    let n = b.len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&b[..n]);
    buf[n] = 0;
    unsafe {
        device_register_rust(buf.as_ptr());
    }
    catalog::publish_device(name, class_for(kind), id);
}

fn class_for(kind: DriveKind) -> &'static str {
    match kind {
        DriveKind::Char => "char",
        DriveKind::Screen => "fb",
        DriveKind::Info => "info",
    }
}

fn probe(name: &str) -> Result<(), &'static str> {
    match name {
        "null" => null::probe(),
        "zero" => zero::probe(),
        "ttyS0" => serial::probe(),
        "kbd" => kbd::probe(),
        "tty0" => vga::probe(),
        "fb0" => fb::probe(),
        "mem" => meminfo::probe(),
        "cpu" => cpuinfo::probe(),
        "clock" => clock::probe(),
        _ => Err("unknown drive"),
    }
}

/// Start the built-in drives at boot.
pub fn init_drives() {
    if INIT.swap(true, Ordering::SeqCst) {
        return;
    }
    for (i, d) in snapshot().iter().enumerate() {
        catalog::publish_drive(d.name, class_for(d.kind), false, i as u32);
    }

    let _ = init_drive("null");
    let _ = init_drive("zero");
    let _ = init_drive("ttyS0");
    let _ = init_drive("kbd");
    let _ = init_drive("mem");
    let _ = init_drive("cpu");
    let _ = init_drive("clock");

    let use_fb = unsafe { console_fb_active() };
    if use_fb {
        let _ = init_drive("fb0");
        let _ = init_drive("tty0");
    } else {
        let _ = init_drive("tty0");
    }

    pci::scan_print();
    /* Block disks: ram0 always; vda if virtio-blk present. Write-gated. */
    super::block::init();
    /* Framework drivers (registry table): NICs today, Wi-Fi later. */
    let bound = super::pci_drivers::probe_all();
    if super::virtio::net::NET0.lock().is_none() {
        let mut out = crate::Com1;
        let _ = core::fmt::Write::write_str(&mut out, "net: none\n");
    }
    let _ = bound;
}

/// Start one drive by name and publish its /dev node.
pub fn init_drive(name: &str) -> Result<(), &'static str> {
    let (idx, d) = with_table(|t| {
        t.iter()
            .position(|d| d.name == name || d.dev_name == name || d.title == name)
            .map(|i| (i, t[i]))
    })
    .ok_or("unknown drive")?;
    if d.ready {
        return Ok(());
    }
    // Probe with the table unlocked (probes may publish to the catalog, etc.).
    probe(d.name)?;
    // Mark ready; if another caller won the race, it already registered the node.
    let already = with_table(|t| core::mem::replace(&mut t[idx].ready, true));
    if already {
        return Ok(());
    }
    let name = d.name;
    let kind = d.kind;
    let dev_name = d.dev_name;
    register_dev(dev_name, kind, idx as u32);
    catalog::publish_drive(name, class_for(kind), true, idx as u32);
    Ok(())
}

pub fn list_drives(buf: &mut [u8]) -> usize {
    let t = snapshot();
    let mut s = String::new();
    for d in t.iter() {
        s.push_str(d.name);
        s.push('(');
        s.push_str(d.title);
        s.push(')');
        s.push(':');
        s.push_str(if d.ready { "ready" } else { "idle" });
        s.push(' ');
    }
    write_cstr(&s, buf)
}

pub fn list_devices(buf: &mut [u8]) -> usize {
    let t = snapshot();
    let mut s = String::new();
    for d in t.iter().filter(|d| d.ready) {
        s.push_str("/dev/");
        s.push_str(d.dev_name);
        s.push(' ');
    }
    write_cstr(&s, buf)
}

pub fn drive_cmd(target: &str, cmd: &str, buf: &mut [u8]) -> usize {
    let found = with_table(|t| {
        t.iter()
            .find(|d| d.name == target || d.dev_name == target || d.title == target)
            .copied()
    });
    let (name, title, dev_name, ready, kind) = match found {
        None => {
            return write_cstr("error: unknown target", buf);
        }
        Some(d) => (d.name, d.title, d.dev_name, d.ready, d.kind),
    };

    let reply = if cmd == "init" || cmd == "load" {
        match init_drive(name) {
            Ok(()) => String::from("ok"),
            Err(e) => {
                let mut s = String::from("error: ");
                s.push_str(e);
                s
            }
        }
    } else if !ready && (cmd == "status" || cmd == "info") && kind != DriveKind::Info {
        let mut s = String::from(name);
        s.push_str(" idle node=/dev/");
        s.push_str(dev_name);
        s
    } else {
        if !ready {
            let _ = init_drive(name);
        }
            match name {
            "mem" => meminfo::cmd(cmd),
            "cpu" => cpuinfo::cmd(cmd),
            "clock" => clock::cmd(cmd),
            "kbd" => kbd::cmd(cmd),
            _ if cmd == "status" || cmd == "info" => {
                let mut s = String::from(name);
                s.push(' ');
                let now_ready = with_table(|t| {
                    t.iter()
                        .find(|d| d.name == name)
                        .map(|d| d.ready)
                        .unwrap_or(false)
                });
                s.push_str(if now_ready { "ready" } else { "idle" });
                s.push_str(" node=/dev/");
                s.push_str(dev_name);
                let _ = title;
                s
            }
            _ => String::from("error: unknown cmd (status|info|init)"),
        }
    };
    write_cstr(&reply, buf)
}

pub fn device_read(name: &str, buf: &mut [u8]) -> i64 {
    match name {
        "null" => null::read(buf),
        "zero" => zero::read(buf),
        "ttyS0" => serial::read(buf),
        "kbd" => kbd::read(buf),
        "tty0" => vga::read(buf),
        "fb0" => fb::read(buf),
        "meminfo" => meminfo::read(buf),
        "cpu" => cpuinfo::read(buf),
        "clock" => clock::read(buf),
        _ => -2,
    }
}

pub fn device_write(name: &str, buf: &[u8]) -> i64 {
    match name {
        "null" => null::write(buf),
        "zero" => zero::write(buf),
        "ttyS0" => serial::write(buf),
        "kbd" => kbd::write(buf),
        "tty0" => vga::write(buf),
        "fb0" => fb::write(buf),
        "meminfo" => meminfo::write(buf),
        "cpu" => cpuinfo::write(buf),
        "clock" => clock::write(buf),
        _ => -2,
    }
}

pub fn device_ioctl(name: &str, request: u64, argp: *mut u8) -> i64 {
    match name {
        "null" => null::ioctl(request, argp),
        "zero" => zero::ioctl(request, argp),
        "ttyS0" => serial::ioctl(request, argp),
        "kbd" => kbd::ioctl(request, argp),
        "tty0" => vga::ioctl(request, argp),
        "fb0" => fb::ioctl(request, argp),
        "meminfo" => meminfo::ioctl(request, argp),
        "cpu" => cpuinfo::ioctl(request, argp),
        "clock" => clock::ioctl(request, argp),
        _ => -2,
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
