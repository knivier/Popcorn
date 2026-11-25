//! Wi-Fi drivers. Each chip family gets its own module and an entry in the PCI
//! driver table (`drivers::pci_drivers`); what every Wi-Fi driver will share
//! (the `WirelessDevice` scan / connect / keys interface, WPA2) arrives with
//! stage W3, when there is a second user for it besides the shell.
//!
//! Families: `ath11k` (Qualcomm WCN6855, the T14s Gen 4 card). Qualcomm's PCIe
//! transport (MHI) lives in `bus::mhi` so ath12k-class chips can reuse it.

pub mod ath11k;
#[allow(dead_code)] // the send path is first used by HTC connect (W3c)
pub mod ath11k_ce;
#[allow(dead_code)] // MSI layout tables are used from stage W3 on
pub mod ath11k_hw;
pub mod fw_manifest;
#[allow(dead_code)]
pub mod board2;
#[allow(dead_code)] // the full WLFW message set; W3 uses the rest
pub mod qmi;
pub mod qmi_link;

use alloc::string::String;
use core::ffi::{c_char, CStr};
use core::fmt::Write;

use crate::console_ffi::{println_color, COLOR_LIGHT_GREEN, COLOR_LIGHT_RED, COLOR_WHITE, COLOR_YELLOW};

fn say(color: u8, args: core::fmt::Arguments) {
    let mut s = String::new();
    let _ = s.write_str("fw: ");
    let _ = s.write_fmt(args);
    println_color(&s, color);
}

/// `wifi -fwcheck`: read every file listed in /FW/WCN6855/MANIFEST.TXT from the
/// boot stick and check its size and SHA-256. Needs no Wi-Fi card (QEMU runs it).
fn fwcheck() {
    const DIR: &str = "/FW/WCN6855";
    let t0 = unsafe { core::arch::x86_64::_rdtsc() };
    let hz = crate::drivers::backends::cpuinfo::tsc_hz().max(1000);
    let man = match crate::fs::bootfs::read("/FW/WCN6855/MANIFEST.TXT", 64 * 1024, &mut |_, _| {}) {
        Ok(m) => m,
        Err(why) => {
            say(COLOR_YELLOW, format_args!("no Wi-Fi firmware on the boot stick ({}); build with: core.sh firmware", why));
            return;
        }
    };
    let (mut ok, mut total) = (0usize, 0usize);
    let r = fw_manifest::for_each(&man, |name, e| {
        total += 1;
        let mut path = String::new();
        let _ = write!(path, "{}/{}", DIR, name);
        let res = crate::fs::bootfs::read(&path, 64 << 20, &mut |_, _| {})
            .and_then(|data| fw_manifest::verify(&data, &e).map(|_| data.len()));
        match res {
            Ok(n) => {
                ok += 1;
                let hex = crate::util::sha256::to_hex(&e.sha256);
                say(COLOR_WHITE, format_args!("{} {} bytes ok (sha256 {}...)", name, n, core::str::from_utf8(&hex[..12]).unwrap_or("?")));
            }
            Err(why) => say(COLOR_LIGHT_RED, format_args!("{} FAILED: {}", name, why)),
        }
    });
    if let Err(why) = r {
        say(COLOR_LIGHT_RED, format_args!("manifest unusable: {}", why));
        return;
    }
    let ms = unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(t0) / (hz / 1000);
    say(
        if ok == total && total > 0 { COLOR_LIGHT_GREEN } else { COLOR_LIGHT_RED },
        format_args!("{} of {} files verified in {} ms ({})", ok, total, ms, DIR),
    );
}

/// `wifi [-info | -up | -help]`
#[no_mangle]
pub extern "C" fn rust_wifi_cmd(args: *const c_char) {
    let a = if args.is_null() {
        ""
    } else {
        unsafe { CStr::from_ptr(args) }.to_str().unwrap_or("?").trim()
    };
    match a {
        "" | "-info" => ath11k::info(),
        "-up" => ath11k::up(true),
        "-reset" => ath11k::up(false),
        "-fwcheck" => fwcheck(),
        _ => {
            println_color("wifi            show the Wi-Fi card and its bring-up state", COLOR_WHITE);
            println_color("wifi -up        reset the card (W0), load its firmware over MHI (W1), QMI handshake (W2), start WLAN (W3a)", COLOR_WHITE);
            println_color("wifi -reset     only the reset and firmware-loader state (W0)", COLOR_WHITE);
            println_color("wifi -fwcheck   check the firmware files on the boot stick (size, SHA-256)", COLOR_WHITE);
        }
    }
}
