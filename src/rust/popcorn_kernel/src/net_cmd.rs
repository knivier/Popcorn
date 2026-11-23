//! Kernel glue for the network stack: the pump called from the kmain loop and the
//! `net` / `ping` shell commands. The stack (`crate::net`) owns protocol state;
//! the NIC (`drivers::virtio::net::NET0`) is passed to it per call.
//!
//! Lock order: IFACE → NET0 (never the reverse). The pump only try-locks, so the
//! shell loop never blocks on the network.

use core::ffi::c_char;
use core::fmt::Write;

use crate::console_ffi::{print_color, println_color, COLOR_LIGHT_CYAN, COLOR_LIGHT_GREEN, COLOR_WHITE, COLOR_YELLOW};
use crate::drivers::net::NetDevice;
use crate::drivers::virtio::net::{MacHex, NET0};
use crate::drivers::virtio::probe::LineBuf;
use crate::net::iface::{Iface, IpConfig};
use crate::sync::TaskLock;

extern "C" {
    fn timer_get_uptime_ms() -> u64;
    fn timer_poll();
    fn timer_is_poll_mode() -> bool;
}

static IFACE: TaskLock<Option<Iface>> = TaskLock::new(None);

const DEFAULT_CFG: IpConfig = IpConfig { ip: [10, 0, 2, 15], prefix: 24, gw: [10, 0, 2, 2], dns: [10, 0, 2, 3] };

fn now_ms() -> u64 {
    unsafe { timer_get_uptime_ms() }
}

fn com1_log(s: &str) {
    let mut out = crate::Com1;
    let _ = out.write_str(s);
    let _ = out.write_str("\n");
}

fn say(args: core::fmt::Arguments, color: u8) {
    let mut l = LineBuf::new();
    let _ = l.write_fmt(args);
    println_color(l.as_str(), color);
}

/// Wait for the next timer tick (IRQ mode: hlt; poll fallback: poll the PIT).
fn wait_tick() {
    unsafe {
        if timer_is_poll_mode() {
            timer_poll();
            core::arch::asm!("pause", options(nomem, nostack));
        } else if crate::sync::irqs_enabled() {
            core::arch::asm!("hlt", options(nomem, nostack));
        } else {
            core::arch::asm!("pause", options(nomem, nostack));
        }
    }
}

/// One stack pass (RX, ARP timers, TX flush). Never blocks.
fn pump_once() {
    let Some(mut ig) = IFACE.try_lock() else { return };
    let Some(iface) = ig.as_mut() else { return };
    let Some(mut ng) = NET0.try_lock() else { return };
    let Some(dev) = ng.as_mut() else { return };
    iface.poll(dev, now_ms());
}

/// kmain loop hook (next to timer_poll).
#[no_mangle]
pub extern "C" fn rust_net_pump() {
    pump_once();
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for b in out.iter_mut() {
        *b = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

struct Ip([u8; 4]);
impl core::fmt::Display for Ip {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

/// `net -up [A.B.C.D/NN gw G.G.G.G]`
fn net_up(args: &str) {
    let mut cfg = DEFAULT_CFG;
    let mut it = args.split_whitespace();
    if let Some(addr) = it.next() {
        let (ip, prefix) = match addr.split_once('/') {
            Some((ip, p)) => (ip, p.parse::<u8>().ok()),
            None => (addr, Some(24)),
        };
        match (parse_ip(ip), prefix) {
            (Some(ip), Some(p)) if p <= 32 => {
                cfg.ip = ip;
                cfg.prefix = p;
            }
            _ => {
                println_color("Usage: net -up [A.B.C.D/NN gw G.G.G.G]", COLOR_YELLOW);
                return;
            }
        }
        if it.next() == Some("gw") {
            match it.next().and_then(parse_ip) {
                Some(gw) => cfg.gw = gw,
                None => {
                    println_color("Usage: net -up [A.B.C.D/NN gw G.G.G.G]", COLOR_YELLOW);
                    return;
                }
            }
        }
    }
    let mac = match NET0.lock().as_ref() {
        Some(d) => d.mac(),
        None => {
            println_color("net: no network device (net: none)", COLOR_YELLOW);
            return;
        }
    };
    let mut ig = IFACE.lock();
    match ig.as_mut() {
        Some(iface) => iface.set_config(cfg),
        None => *ig = Some(Iface::new(mac, cfg, com1_log)),
    }
    drop(ig);
    say(format_args!("net0 up: {}/{} gw {} dns {}", Ip(cfg.ip), cfg.prefix, Ip(cfg.gw), Ip(cfg.dns)), COLOR_LIGHT_GREEN);
    let mut out = crate::Com1;
    let _ = writeln!(out, "net: up {}/{} gw {}", Ip(cfg.ip), cfg.prefix, Ip(cfg.gw));
}

fn net_info() {
    let ng = NET0.lock();
    let Some(dev) = ng.as_ref() else {
        println_color("net: no network device", COLOR_YELLOW);
        return;
    };
    let a = dev.pci_addr();
    let (offered, acked) = dev.features();
    say(format_args!("net0: virtio-net (modern) at {}:{}.{}  mac {}  mtu {}  link {}", a.bus, a.slot, a.func, MacHex(dev.mac()), dev.mtu(), if dev.link_up() { "up" } else { "down" }), COLOR_WHITE);
    say(format_args!("  features offered={:#x} acked={:#x}  queues rx/tx 256/256  irq poll  dma {} pages", offered, acked, dev.dma_pages()), COLOR_WHITE);
    drop(ng);
    match IFACE.lock().as_ref() {
        Some(i) => {
            let c = i.config();
            say(format_args!("  ip {}/{} gw {} dns {}", Ip(c.ip), c.prefix, Ip(c.gw), Ip(c.dns)), COLOR_WHITE);
        }
        None => println_color("  ip: not configured (net -up)", COLOR_WHITE),
    }
}

fn net_stats() {
    if let Some(dev) = NET0.lock().as_ref() {
        let s = dev.stats();
        say(format_args!("net0: rx {} pkts {} B, errors {} | tx {} pkts {} B, full {} | kicks {} (suppressed {}) | rx_posted {} tx_free {}", s.rx_packets, s.rx_bytes, s.rx_errors, s.tx_packets, s.tx_bytes, s.tx_full, s.kicks, s.kicks_suppressed, s.rx_posted, s.tx_free), COLOR_WHITE);
    } else {
        println_color("net: no network device", COLOR_YELLOW);
    }
    if let Some(i) = IFACE.lock().as_ref() {
        let mut l = LineBuf::new();
        let _ = write!(l, "stack: {:?}", i.stats());
        println_color(l.as_str(), COLOR_WHITE);
    }
}

fn net_down() {
    *IFACE.lock() = None;
    println_color("net0 down (interface unconfigured; device stays bound)", COLOR_WHITE);
}

fn net_arp() {
    let ig = IFACE.lock();
    let Some(i) = ig.as_ref() else {
        println_color("net: not configured (net -up)", COLOR_YELLOW);
        return;
    };
    let c = i.config();
    for ip in [c.gw, c.dns] {
        match i.arp_lookup(ip) {
            Some(mac) => say(format_args!("  {} -> {}", Ip(ip), MacHex(mac)), COLOR_WHITE),
            None => say(format_args!("  {} -> (unresolved)", Ip(ip)), COLOR_WHITE),
        }
    }
}

fn net_help() {
    println_color("net -info               device, features, IP configuration", COLOR_WHITE);
    println_color("net -stats              driver + stack counters", COLOR_WHITE);
    println_color("net -up [A.B.C.D/NN gw G]  configure (default 10.0.2.15/24 gw 10.0.2.2)", COLOR_WHITE);
    println_color("net -down               unconfigure", COLOR_WHITE);
    println_color("net -arp                ARP entries for gateway / DNS", COLOR_WHITE);
    println_color("ping <A.B.C.D> [count] [size]   ICMP echo (default 4 x 56 bytes)", COLOR_WHITE);
}

#[no_mangle]
pub extern "C" fn rust_net_cmd(args: *const c_char) {
    let a = crate::cstr(args).trim();
    match a.split_once(' ').map(|(c, r)| (c, r.trim())).unwrap_or((a, "")) {
        ("-info", _) => net_info(),
        ("-stats", _) => net_stats(),
        ("-up", rest) => net_up(rest),
        ("-down", _) => net_down(),
        ("-arp", _) => net_arp(),
        _ => net_help(),
    }
}

/// `ping <A.B.C.D> [count] [size]`: pumps the stack itself while waiting (the
/// kmain loop is busy running this command).
#[no_mangle]
pub extern "C" fn rust_ping(args: *const c_char) {
    let mut it = crate::cstr(args).split_whitespace();
    let Some(dst) = it.next().and_then(parse_ip) else {
        println_color("Usage: ping <A.B.C.D> [count] [size]", COLOR_YELLOW);
        return;
    };
    let count: u16 = it.next().and_then(|s| s.parse().ok()).unwrap_or(4).clamp(1, 100);
    let size: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(56).min(1472);
    if IFACE.lock().is_none() {
        println_color("net: not configured (run net -up)", COLOR_YELLOW);
        return;
    }
    say(format_args!("PING {}: {} data bytes", Ip(dst), size), COLOR_LIGHT_CYAN);
    let id: u16 = 0x5043; // "PC"
    let mut received = 0u16;
    for seq in 1..=count {
        let sent_at = now_ms();
        let sent = {
            let mut ig = IFACE.lock();
            let mut ng = NET0.lock();
            match (ig.as_mut(), ng.as_mut()) {
                (Some(i), Some(d)) => i.ping_send(d, dst, id, seq, size, sent_at).is_ok(),
                _ => false,
            }
        };
        if !sent {
            say(format_args!("seq={}: send failed", seq), COLOR_YELLOW);
            continue;
        }
        let mut rtt = None;
        while now_ms().saturating_sub(sent_at) < 1000 {
            pump_once();
            if let Some(r) = IFACE.lock().as_ref().and_then(|i| i.ping_reply(id, seq)) {
                rtt = Some(r);
                break;
            }
            wait_tick();
        }
        match rtt {
            Some(ms) => {
                received += 1;
                say(format_args!("Reply from {}: seq={} time={} ms", Ip(dst), seq, ms), COLOR_WHITE);
            }
            None => say(format_args!("Request timed out (seq={})", seq), COLOR_YELLOW),
        }
        // Pace like ping(8): about one request per second.
        while now_ms().saturating_sub(sent_at) < 1000 && seq < count {
            pump_once();
            wait_tick();
        }
    }
    let loss = (count - received) as u32 * 100 / count as u32;
    say(format_args!("{} packets transmitted, {} received, {}% loss", count, received, loss), if received == count { COLOR_LIGHT_GREEN } else { COLOR_YELLOW });
    let mut out = crate::Com1;
    let _ = writeln!(out, "net: ping {}/{}", received, count);
    let _ = print_color;
}
