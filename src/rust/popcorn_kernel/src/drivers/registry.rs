//! Drive catalog: init_drive → /dev nodes + commands.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use super::backends::{fb, null, serial, vga, zero};

extern "C" {
    fn device_register_rust(name: *const u8);
    fn console_fb_active() -> i32;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Char,
    Screen,
}

pub struct DriveEntry {
    pub name: &'static str,
    /// Short plain-English label for listings.
    pub title: &'static str,
    pub kind: DriveKind,
    pub ready: bool,
    pub dev_name: &'static str,
}

static mut DRIVES: Option<Vec<DriveEntry>> = None;
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
    ]
}

fn table() -> &'static mut Vec<DriveEntry> {
    unsafe {
        if DRIVES.is_none() {
            DRIVES = Some(catalog());
        }
        DRIVES.as_mut().unwrap()
    }
}

fn register_dev(name: &str) {
    let mut buf = [0u8; 32];
    let b = name.as_bytes();
    let n = b.len().min(buf.len() - 1);
    buf[..n].copy_from_slice(&b[..n]);
    buf[n] = 0;
    unsafe {
        device_register_rust(buf.as_ptr());
    }
}

fn probe(name: &str) -> Result<(), &'static str> {
    match name {
        "null" => null::probe(),
        "zero" => zero::probe(),
        "ttyS0" => serial::probe(),
        "tty0" => vga::probe(),
        "fb0" => fb::probe(),
        _ => Err("unknown drive"),
    }
}

/// Start the built-in drives at boot.
pub fn init_drives() {
    if INIT.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = table();

    let _ = init_drive("null");
    let _ = init_drive("zero");
    let _ = init_drive("ttyS0");

    let use_fb = unsafe { console_fb_active() != 0 };
    if use_fb {
        let _ = init_drive("fb0");
        let _ = init_drive("tty0");
    } else {
        let _ = init_drive("tty0");
    }
}

/// Start one drive by name and publish its /dev node.
pub fn init_drive(name: &str) -> Result<(), &'static str> {
    let t = table();
    let idx = t
        .iter()
        .position(|d| d.name == name || d.dev_name == name || d.title == name)
        .ok_or("unknown drive")?;
    if t[idx].ready {
        return Ok(());
    }
    probe(t[idx].name)?;
    t[idx].ready = true;
    register_dev(t[idx].dev_name);
    Ok(())
}

pub fn list_drives(buf: &mut [u8]) -> usize {
    let t = table();
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
    let t = table();
    let mut s = String::new();
    for d in t.iter().filter(|d| d.ready) {
        s.push_str("/dev/");
        s.push_str(d.dev_name);
        s.push(' ');
    }
    write_cstr(&s, buf)
}

pub fn drive_cmd(target: &str, cmd: &str, buf: &mut [u8]) -> usize {
    let t = table();
    let d = t
        .iter()
        .find(|d| d.name == target || d.dev_name == target || d.title == target);
    let reply = match (d, cmd) {
        (None, _) => String::from("error: unknown target"),
        (Some(d), "status") | (Some(d), "info") => {
            let mut s = String::from(d.name);
            s.push(' ');
            s.push_str(if d.ready { "ready" } else { "idle" });
            s.push_str(" node=/dev/");
            s.push_str(d.dev_name);
            s
        }
        (Some(d), "init") | (Some(d), "load") => match init_drive(d.name) {
            Ok(()) => String::from("ok"),
            Err(e) => {
                let mut s = String::from("error: ");
                s.push_str(e);
                s
            }
        },
        (Some(_), _) => String::from("error: unknown cmd (status|info|init)"),
    };
    write_cstr(&reply, buf)
}

pub fn device_read(name: &str, buf: &mut [u8]) -> i64 {
    match name {
        "null" => null::read(buf),
        "zero" => zero::read(buf),
        "ttyS0" => serial::read(buf),
        "tty0" => vga::read(buf),
        "fb0" => fb::read(buf),
        _ => -2,
    }
}

pub fn device_write(name: &str, buf: &[u8]) -> i64 {
    match name {
        "null" => null::write(buf),
        "zero" => zero::write(buf),
        "ttyS0" => serial::write(buf),
        "tty0" => vga::write(buf),
        "fb0" => fb::write(buf),
        _ => -2,
    }
}

pub fn device_ioctl(name: &str, request: u64, argp: *mut u8) -> i64 {
    match name {
        "null" => null::ioctl(request, argp),
        "zero" => zero::ioctl(request, argp),
        "ttyS0" => serial::ioctl(request, argp),
        "tty0" => vga::ioctl(request, argp),
        "fb0" => fb::ioctl(request, argp),
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
