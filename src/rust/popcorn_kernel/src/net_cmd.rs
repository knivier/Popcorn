//! Kernel glue for the network stack: the pump called from the kmain loop and the
//! `net` / `ping` / `nslookup` / `udp` shell commands. The stack (`crate::net`) owns protocol state;
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
use crate::net::{DhcpState, Iface, IpConfig};
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

/// Sleep until the next interrupt: the timer tick, or the NIC's RX interrupt once
/// armed. Race-free: interrupts are disabled while arming, and `sti; hlt` makes the
/// re-enable and the halt one step, so a packet that lands in between still wakes us.
fn wait_tick() {
    crate::sync::wait_for_interrupt(idle_prepare);
}

/// Arm the NIC's RX interrupt before the CPU idles. true = work already waiting
/// (or the locks are busy): do not sleep.
fn idle_prepare() -> bool {
    let Some(mut ng) = NET0.try_lock() else { return true };
    match ng.as_mut() {
        Some(dev) => dev.idle_arm(),
        None => false,
    }
}

/// One stack pass (RX, ARP/DHCP/DNS timers, TX flush). Never blocks. On an
/// interrupt-capable NIC, first mask the RX interrupt again (the pump drains by
/// polling; the interrupt only exists to wake the CPU).
fn pump_once() {
    let Some(mut ng) = NET0.try_lock() else { return };
    let Some(dev) = ng.as_mut() else { return };
    dev.wake_disarm();
    drop(ng);
    let Some(mut ig) = IFACE.try_lock() else { return };
    let Some(iface) = ig.as_mut() else { return };
    let Some(mut ng) = NET0.try_lock() else { return };
    let Some(dev) = ng.as_mut() else { return };
    iface.poll(dev, now_ms());
}

/// kmain idle hook, called with interrupts disabled right before `sti; hlt`.
/// Returns 1 if the network has work waiting (the loop should not sleep).
#[no_mangle]
pub extern "C" fn rust_net_idle_prepare() -> i32 {
    idle_prepare() as i32
}

/// Run `f` with the interface and the NIC locked (IFACE → NET0). None if either
/// is missing.
fn with_net<R>(f: impl FnOnce(&mut Iface, &mut dyn NetDevice) -> R) -> Option<R> {
    let mut ig = IFACE.lock();
    let iface = ig.as_mut()?;
    let mut ng = NET0.lock();
    let dev = ng.as_mut()?;
    Some(f(iface, dev))
}

/// Pump the stack until `check` yields a value or `timeout_ms` passes.
fn wait_for<T>(timeout_ms: u64, mut check: impl FnMut(&Iface) -> Option<T>) -> Option<T> {
    let start = now_ms();
    loop {
        pump_once();
        if let Some(v) = IFACE.lock().as_ref().and_then(&mut check) {
            return Some(v);
        }
        if now_ms().saturating_sub(start) >= timeout_ms {
            return None;
        }
        wait_tick();
    }
}

/// Per-call randomness for DHCP xids / DNS ids / ports (TSC-derived).
fn seed() -> u32 {
    let t = unsafe { core::arch::x86_64::_rdtsc() };
    crate::net::mix32((t ^ (t >> 32)) as u32)
}

/// Resolve a dotted quad or a DNS name (pumping while the query runs).
fn resolve(name: &str) -> Result<[u8; 4], &'static str> {
    if let Some(ip) = parse_ip(name) {
        return Ok(ip);
    }
    let id = match with_net(|i, d| i.dns_query(d, name, now_ms(), seed())) {
        None => return Err("not configured (run net -up or net -dhcp)"),
        Some(Err(e)) => return Err(e.reason()),
        Some(Ok(id)) => id,
    };
    match wait_for(6000, |i| i.dns_result(id)) {
        Some(Ok(ip)) => Ok(ip),
        Some(Err(e)) => Err(e.reason()),
        None => Err("timeout"),
    }
}

/// `net -dhcp`: lease an address (falls back to 10.0.2.15/24 after 3 s).
fn net_dhcp() {
    let mac = match NET0.lock().as_ref() {
        Some(d) => d.mac(),
        None => {
            println_color("net: no network device (net: none)", COLOR_YELLOW);
            return;
        }
    };
    {
        let mut ig = IFACE.lock();
        if ig.is_none() {
            *ig = Some(Iface::new(mac, IpConfig::NONE, com1_log));
        }
    }
    println_color("DHCP: discovering...", COLOR_LIGHT_CYAN);
    let started = with_net(|i, d| i.dhcp_start(d, now_ms(), seed(), Some(DEFAULT_CFG)));
    if started.is_none() {
        println_color("net: no network device", COLOR_YELLOW);
        return;
    }
    let st = wait_for(8000, |i| match i.dhcp_state() {
        s @ (DhcpState::Bound { .. } | DhcpState::Failed) => Some(s),
        _ => None,
    });
    let cfg = IFACE.lock().as_ref().map(|i| i.config()).unwrap_or(IpConfig::NONE);
    match st {
        Some(DhcpState::Bound { lease_s }) => say(
            format_args!("DHCP: bound {}/{} gw {} dns {} lease {} s", Ip(cfg.ip), cfg.prefix, Ip(cfg.gw), Ip(cfg.dns), lease_s),
            COLOR_LIGHT_GREEN,
        ),
        Some(_) => say(format_args!("DHCP: no lease; using static {}/{} gw {}", Ip(cfg.ip), cfg.prefix, Ip(cfg.gw)), COLOR_YELLOW),
        None => println_color("DHCP: still waiting (see net -info)", COLOR_YELLOW),
    }
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
    let (vec, wakes, redundant) = dev.irq_info();
    if dev.irq_mode() == "msix" {
        say(format_args!("  features offered={:#x} acked={:#x}  queues rx/tx 256/256  irq msix v={:#04x} (wakes {}, redundant {})  dma {} pages", offered, acked, vec, wakes, redundant, dev.dma_pages()), COLOR_WHITE);
    } else {
        say(format_args!("  features offered={:#x} acked={:#x}  queues rx/tx 256/256  irq poll  dma {} pages", offered, acked, dev.dma_pages()), COLOR_WHITE);
    }
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
        say(format_args!("net0: irq {} irqs {}", dev.irq_mode(), dev.irq_count()), COLOR_WHITE);
    } else {
        println_color("net: no network device", COLOR_YELLOW);
    }
    if let Some(i) = IFACE.lock().as_ref() {
        let s = i.stats();
        say(format_args!("stack rx: frames {} delivered {} dropped {} | arp {} ipv4 {} icmp {} udp {} dhcp {} dns {}",
            s.rx_frames, s.rx_delivered, s.rx_dropped(), s.rx_arp, s.rx_ipv4, s.rx_icmp, s.rx_udp, s.rx_dhcp, s.rx_dns), COLOR_WHITE);
        say(format_args!("stack tx: frames {} arp {} ipv4 {} icmp {} udp {} dhcp {} dns {} | tx_udp_echo: {} tx_garp: {}",
            s.tx_frames, s.tx_arp, s.tx_ipv4, s.tx_icmp, s.tx_udp, s.tx_dhcp, s.tx_dns, s.tx_udp_echo, s.tx_garp), COLOR_WHITE);
        say(format_args!("stack drops: short {} csum {} udp_csum {} not_for_us {} fragment {} no_socket {} dns_mismatch {} dns_malformed {} arp_timeout {}",
            s.drop_short, s.drop_bad_csum, s.drop_udp_csum, s.drop_not_for_us, s.drop_fragment, s.drop_udp_no_socket, s.drop_dns_mismatch, s.drop_dns_malformed, s.drop_arp_timeout), COLOR_WHITE);
        say(format_args!("dhcp: offers {} dhcp_acks: {} naks {} retransmits {} fallbacks {} | dns: answers {} failures {} timeouts {} retries {} cache_hits {}",
            s.dhcp_offers, s.dhcp_acks, s.dhcp_naks, s.dhcp_retransmits, s.dhcp_fallbacks, s.dns_answers, s.dns_failures, s.dns_timeouts, s.dns_retries, s.dns_cache_hits), COLOR_WHITE);
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
    println_color("net -dhcp               lease an address (static 10.0.2.15/24 fallback after 3 s)", COLOR_WHITE);
    println_color("ping <host> [count] [size]      ICMP echo to an address or name", COLOR_WHITE);
    println_color("nslookup <name>         DNS A lookup", COLOR_WHITE);
    println_color("udp -send <host> <port> <text>  send one datagram", COLOR_WHITE);
    println_color("udp -echo <port>        echo every datagram received on <port>", COLOR_WHITE);
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
        ("-dhcp", _) => net_dhcp(),
        _ => net_help(),
    }
}

/// `ping <A.B.C.D> [count] [size]`: pumps the stack itself while waiting (the
/// kmain loop is busy running this command).
#[no_mangle]
pub extern "C" fn rust_ping(args: *const c_char) {
    let mut it = crate::cstr(args).split_whitespace();
    let Some(target) = it.next() else {
        println_color("Usage: ping <host> [count] [size]", COLOR_YELLOW);
        return;
    };
    let count: u16 = it.next().and_then(|s| s.parse().ok()).unwrap_or(4).clamp(1, 100);
    let size: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(56).min(1472);
    if IFACE.lock().is_none() {
        println_color("net: not configured (run net -up)", COLOR_YELLOW);
        return;
    }
    let dst = match resolve(target) {
        Ok(ip) => ip,
        Err(why) => {
            say(format_args!("ping: cannot resolve {}: {}", target, why), COLOR_YELLOW);
            return;
        }
    };
    if parse_ip(target).is_some() {
        say(format_args!("PING {}: {} data bytes", Ip(dst), size), COLOR_LIGHT_CYAN);
    } else {
        say(format_args!("PING {} ({}): {} data bytes", target, Ip(dst), size), COLOR_LIGHT_CYAN);
    }
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

/// `nslookup <name>`
#[no_mangle]
pub extern "C" fn rust_nslookup(args: *const c_char) {
    let name = crate::cstr(args).trim();
    if name.is_empty() {
        println_color("Usage: nslookup <name>", COLOR_YELLOW);
        return;
    }
    match resolve(name) {
        Ok(ip) => say(format_args!("{} -> {}", name, Ip(ip)), COLOR_LIGHT_GREEN),
        Err(why) => say(format_args!("nslookup: {}: {}", name, why), COLOR_YELLOW),
    }
}

/// `udp -send <host> <port> <text>` / `udp -echo <port>`
#[no_mangle]
pub extern "C" fn rust_udp_cmd(args: *const c_char) {
    let a = crate::cstr(args).trim();
    let usage = || println_color("Usage: udp -send <host> <port> <text> | udp -echo <port>", COLOR_YELLOW);
    if let Some(rest) = a.strip_prefix("-echo ") {
        let Ok(port) = rest.trim().parse::<u16>() else { return usage() };
        match IFACE.lock().as_mut().map(|i| i.udp_echo(port)) {
            Some(Ok(_)) => say(format_args!("udp: echo service on port {}", port), COLOR_LIGHT_GREEN),
            Some(Err(e)) => say(format_args!("udp: cannot listen on {}: {:?}", port, e), COLOR_YELLOW),
            None => println_color("net: not configured (run net -up)", COLOR_YELLOW),
        }
        return;
    }
    if let Some(rest) = a.strip_prefix("-send ") {
        let mut it = rest.splitn(3, ' ');
        let (Some(host), Some(port), Some(text)) = (it.next(), it.next(), it.next()) else { return usage() };
        let Ok(port) = port.parse::<u16>() else { return usage() };
        let dst = match resolve(host) {
            Ok(ip) => ip,
            Err(why) => {
                say(format_args!("udp: cannot resolve {}: {}", host, why), COLOR_YELLOW);
                return;
            }
        };
        let src = 49152 + (seed() % 16384) as u16;
        match with_net(|i, d| i.udp_send_to(d, src, dst, port, text.as_bytes(), now_ms())) {
            Some(Ok(())) => say(format_args!("udp: sent {} bytes to {}:{}", text.len(), Ip(dst), port), COLOR_LIGHT_GREEN),
            Some(Err(e)) => say(format_args!("udp: send failed: {:?}", e), COLOR_YELLOW),
            None => println_color("net: not configured (run net -up)", COLOR_YELLOW),
        }
        return;
    }
    usage();
}
