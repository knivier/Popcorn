//! C entry points for the driver framework (M0): IRQ self-test and listings,
//! `pci -caps`, `virtio -probe`.

use core::fmt::Write;

use crate::console_ffi::{println_color, COLOR_LIGHT_CYAN, COLOR_WHITE};
use crate::drivers::grant::irq::{for_each_active, reserve_kernel_vector, vector_count, VEC_SELFTEST};
use crate::drivers::virtio::probe::LineBuf;

extern "C" {
    fn lapic_self_ipi(vector: u8) -> i32;
    fn isr_unexpected_vector_count() -> u32;
    fn isr_lapic_spurious_count() -> u32;
}

/// Send a self-IPI on the kernel self-test vector and wait for the ISR to count it.
/// Needs IF=1. Returns 1 = delivered exactly once, 0 = not delivered, -1 = no LAPIC.
#[no_mangle]
pub extern "C" fn rust_irq_selftest() -> i32 {
    reserve_kernel_vector(VEC_SELFTEST, "lapic-selftest");
    let before = vector_count(VEC_SELFTEST);
    if unsafe { lapic_self_ipi(VEC_SELFTEST) } != 0 {
        return -1;
    }
    for _ in 0..10_000_000u32 {
        if vector_count(VEC_SELFTEST) != before {
            break;
        }
        core::hint::spin_loop();
    }
    // Let a (wrong) duplicate delivery show up before judging "exactly once".
    for _ in 0..100_000u32 {
        core::hint::spin_loop();
    }
    let got = vector_count(VEC_SELFTEST) - before;
    let mut out = crate::Com1;
    let _ = writeln!(out, "irq: selftest vector 0x30 count={}", got);
    (got == 1) as i32
}

/// `irq`: every vector that has an owner or has fired.
#[no_mangle]
pub extern "C" fn rust_irq_list() {
    println_color("Vector  Source     Owner            Count", COLOR_LIGHT_CYAN);
    for_each_active(|v, owner, count| {
        let mut line = LineBuf::new();
        let source = match v {
            0x20..=0x2F => "pic",
            0x30..=0x3F => "kernel",
            _ => "lapic/msi",
        };
        let owner = if owner.is_empty() {
            match v {
                0x20 => "timer",
                0x21 => "kbd",
                _ => "-",
            }
        } else {
            owner
        };
        let _ = write!(line, "0x{:02x}    {:<10} {:<16} {}", v, source, owner, count);
        println_color(line.as_str(), COLOR_WHITE);
    });
    let mut line = LineBuf::new();
    let _ = write!(
        line,
        "unexpected={} spurious={} grant-violations={}",
        unsafe { isr_unexpected_vector_count() },
        unsafe { isr_lapic_spurious_count() },
        crate::drivers::grant::GRANT_VIOLATIONS.load(core::sync::atomic::Ordering::Relaxed)
    );
    println_color(line.as_str(), COLOR_LIGHT_CYAN);
}

/// `pci -caps`
#[no_mangle]
pub extern "C" fn rust_pci_caps() {
    crate::drivers::bus::pci::scan_caps_print();
}

/// `virtio -blk`
#[no_mangle]
pub extern "C" fn rust_virtio_blk_info() {
    let mut any = false;
    crate::drivers::virtio::blk::info_lines(|l| {
        any = true;
        println_color(l, COLOR_WHITE);
    });
    if !any {
        println_color("virtio-blk: no disks bound", COLOR_WHITE);
    }
}

/// `virtio -blk -verify`
#[no_mangle]
pub extern "C" fn rust_virtio_blk_verify() {
    let mut l = LineBuf::new();
    match crate::drivers::virtio::blk::verify(0, 128) {
        Ok(depth) => {
            let _ = write!(l, "virtio-blk: verify slot 0 128 sectors ok depth {}", depth);
        }
        Err(e) => {
            let _ = write!(l, "virtio-blk: verify slot 0 fail ({})", e);
        }
    }
    println_color(l.as_str(), COLOR_WHITE);
    let mut out = crate::Com1;
    let _ = writeln!(out, "{}", l.as_str());
}

/// `virtio -probe`
#[no_mangle]
pub extern "C" fn rust_virtio_probe() {
    crate::drivers::virtio::probe::probe_command();
}

/// `irq -msitest`: prove plain MSI end to end on QEMU's `edu` test device
/// (PCI 1234:11E8, one 64-bit MSI message, no per-vector masking). Checks that
/// a 32-message request is refused, grants one message, makes the device raise
/// it (edu register 0x60) and waits for the vector to fire.
#[no_mangle]
pub extern "C" fn rust_irq_msitest() {
    use crate::drivers::bus::{msi, pci};
    use crate::console_ffi::{COLOR_LIGHT_GREEN, COLOR_LIGHT_RED, COLOR_YELLOW};
    let say = |color: u8, args: core::fmt::Arguments| {
        let mut l = LineBuf::new();
        let _ = l.write_str("msi: ");
        let _ = l.write_fmt(args);
        println_color(l.as_str(), color);
    };
    let Some(f) = pci::claim(&[(0x1234, 0x11E8)], None, 0) else {
        say(COLOR_YELLOW, format_args!("no QEMU edu test device (start QEMU with -device edu)"));
        return;
    };
    let bar = match f.bar(0) {
        Ok(pci::Bar::Mmio(r)) => r,
        _ => {
            say(COLOR_LIGHT_RED, format_args!("edu BAR0 unusable"));
            pci::release(f);
            return;
        }
    };
    f.enable(true, false, true);
    let ident = bar.r32(0x00);
    if ident & 0xFF != 0xED {
        say(COLOR_LIGHT_RED, format_args!("edu identification {:#x} is wrong", ident));
        pci::release(f);
        return;
    }
    let refused = msi::alloc_irqs(&f, 32, "msi-test").err().unwrap_or("granted (wrong)");
    let set = match msi::alloc_irqs(&f, 1, "msi-test") {
        Ok(s) => s,
        Err(e) => {
            say(COLOR_LIGHT_RED, format_args!("could not grant 1 message: {}", e));
            pci::release(f);
            return;
        }
    };
    let h = &set.handles[0];
    let v = h.vector().unwrap_or(0);
    let before = h.count();
    bar.w32(0x60, 1); // raise
    let got = crate::drivers::delay::poll_ms(500, || h.count() != before);
    let status = bar.r32(0x24);
    bar.w32(0x64, status); // acknowledge
    let fired = h.count() - before;
    let pending = h.take();
    let info = msi::info(&f);
    msi::disable(&f);
    drop(set);
    f.disable_bus_master();
    pci::release(f);
    match got {
        Some(ms) if fired == 1 && pending => say(
            COLOR_LIGHT_GREEN,
            format_args!(
                "edu selftest ok: vector {:#04x} fired once in {} ms ({}-bit address, 32 messages refused: {})",
                v,
                ms,
                if info.map_or(false, |i| i.is_64) { 64 } else { 32 },
                refused
            ),
        ),
        _ => say(
            COLOR_LIGHT_RED,
            format_args!("edu selftest FAILED: vector {:#04x} fired {} time(s), edu status {:#x}", v, fired, status),
        ),
    }
}
