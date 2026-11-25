//! Qualcomm WCN6855 Wi-Fi (ath11k family on PCIe), stages W0 and W1.
//!
//! * Boot probe (`probe`, from the PCI driver table) only identifies: map BAR0,
//!   read the chip version, note the MSI capability. Nothing is reset, so a
//!   card that misbehaves cannot hurt boot.
//! * W0 (`wifi -up`, `wifi -reset`): the power-on reset the chip needs before
//!   firmware can be loaded, MSI, and the MHI / BHI state. A healthy card ends
//!   in its boot ROM (PBL) waiting for firmware.
//! * W1 (`wifi -up`): read AMSS.BIN from /FW/WCN6855 on the boot stick, check it
//!   against MANIFEST.TXT (size + SHA-256), and boot it over MHI (`bus::mhi_host`):
//!   SBL through BHI, READY, M0, the full image through BHIe, then the main
//!   firmware (AMSS). Every step is printed and flushed to POPCORN.LOG.
//!
//! Every hardware access goes through grants: BAR0 as an `MmioRegion`, MSI as an
//! `IrqSet`, ASPM through `PciFunction`. Register facts live in `ath11k_hw`, the
//! MHI register map in `bus::mhi` (shared with future Qualcomm drivers).

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use super::ath11k_hw::{self as hw, Access, Chip};
use crate::console_ffi::{println_color, COLOR_LIGHT_CYAN, COLOR_LIGHT_GREEN, COLOR_LIGHT_RED, COLOR_WHITE, COLOR_YELLOW};
use crate::drivers::bus::mhi;
use crate::drivers::bus::mhi_host::{self, ChCfg, EvCfg, Host, Level};
use crate::drivers::bus::msi::{self, MsiInfo};
use crate::drivers::bus::msix::IrqSet;
use crate::drivers::bus::pci::{Bar, PciFunction};
use crate::drivers::delay::{delay_ms, poll_ms};
use crate::drivers::grant::MmioRegion;
use crate::drivers::virtio::probe::LineBuf;
use crate::sync::TaskLock;

pub struct Ath11k {
    pci: PciFunction,
    bar: MmioRegion,
    /// Window currently selected in WINDOW_REG (None: unknown, select on next use).
    window: Option<u32>,
    soc_version: u32,
    sub_version: Option<u32>,
    chip: Chip,
    msi_cap: Option<MsiInfo>,
    irqs: Option<IrqSet>,
    /// Which layout the granted MSI messages follow (32 or 1), 0 = none yet.
    msi_messages: u8,
    last_mhi: Option<mhi::Snapshot>,
    up_runs: u32,
    up_ok: bool,
    /// AMSS.BIN, verified against the manifest (kept so a retry skips the read).
    fw: Option<Vec<u8>>,
    /// Host state of a booted device (owns memory the device uses).
    mhi: Option<Host>,
    w1: W1,
    /// W2: verified M3.BIN and BOARD-2.BIN, the QMI link, its result.
    m3: Option<Vec<u8>>,
    board2: Option<Vec<u8>>,
    qmi: Option<super::qmi_link::Link>,
    w2: W1,
    cap: Option<super::qmi::Cap>,
    /// How the firmware's memory request is answered (see `MemMode`).
    mem_mode: MemMode,
    /// W3a: firmware WLAN start (WLAN_INI, WLAN_CFG, WLAN_MODE).
    w3: W1,
    /// W3b: copy engines and the target's HTC READY (credits, credit size, endpoints).
    ce: Option<super::ath11k_ce::Ce>,
    w3b: W1,
    htc_ready: Option<(u16, u16, u8)>,
}

/// Answering the firmware's REQUEST_MEM. `Aligned`: one naturally aligned
/// block per segment it asks for. `Delayed`: first answer with no memory, so
/// the firmware asks again in smaller segments (the path Linux takes when a
/// big block cannot be allocated). After a firmware crash right after an
/// `Aligned` answer the next `wifi -up` switches to `Delayed`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MemMode {
    Aligned,
    Delayed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum W1 {
    NotRun,
    Skipped(&'static str),
    Failed(&'static str),
    Running,
}

/// MHI controller layout for WCN6855 (as ath11k configures it): 128 channel
/// contexts, IPCR channels 20 (to device) / 21 (from device) on event ring 1,
/// a control event ring on MSI message 1 and a data ring on message 2.
static WCN6855_EVENTS: [EvCfg; 2] = [
    EvCfg { elements: 32, msivec: 1, intmod_ms: 0 },
    EvCfg { elements: 256, msivec: 2, intmod_ms: 1 },
];
static WCN6855_CHANNELS: [ChCfg; 2] = [
    ChCfg { id: 20, chtype: mhi::CH_TYPE_OUTBOUND, erindex: 1 },
    ChCfg { id: 21, chtype: mhi::CH_TYPE_INBOUND, erindex: 1 },
];
static WCN6855_MHI: mhi_host::Config = mhi_host::Config {
    max_channels: 128,
    events: &WCN6855_EVENTS,
    channels: &WCN6855_CHANNELS,
    cmd_elements: 128,
    timeout_ms: 20_000,
    sbl_size: 512 * 1024,
    seg_len: 512 * 1024,
    iova_stop: (1u64 << 36) - 1,
};

/// Firmware directory on the boot stick for this chip (hw2.0 and hw2.1 share files).
fn fw_dir(chip: Chip) -> Option<&'static str> {
    match chip {
        Chip::Wcn6855Hw20 | Chip::Wcn6855Hw21 => Some("/FW/WCN6855"),
        _ => None,
    }
}

/// The first (only) supported card.
pub static WIFI0: TaskLock<Option<Ath11k>> = TaskLock::new(None);

impl Ath11k {
    fn select(&mut self, window: u32) {
        if self.window != Some(window) {
            self.bar.w32(hw::WINDOW_REG, hw::WINDOW_ENABLE | window);
            let _ = self.bar.r32(hw::WINDOW_REG); // flush the posted write
            self.window = Some(window);
        }
    }
    /// Read a device register (direct or through the window).
    pub fn r32(&mut self, off: u32) -> u32 {
        match hw::access(off) {
            Access::Direct(o) => self.bar.r32(o),
            Access::Windowed { window, bar_off } => {
                self.select(window);
                self.bar.r32(bar_off)
            }
        }
    }
    pub fn w32(&mut self, off: u32, v: u32) {
        match hw::access(off) {
            Access::Direct(o) => self.bar.w32(o, v),
            Access::Windowed { window, bar_off } => {
                self.select(window);
                self.bar.w32(bar_off, v)
            }
        }
    }
    fn mhi_snapshot(&self) -> mhi::Snapshot {
        let bar = &self.bar;
        mhi::snapshot(bar.len(), |o| bar.r32(o))
    }
}

fn com1(args: core::fmt::Arguments) {
    let _ = crate::Com1.write_fmt(args);
}

/// Boot probe: identify only.
pub fn probe(f: PciFunction) -> Result<(), &'static str> {
    if WIFI0.lock().is_some() {
        return Err("a Wi-Fi card is already bound (one supported for now)");
    }
    let a = f.addr();
    let bar = match f.bar(0) {
        Ok(Bar::Mmio(r)) => r,
        Ok(Bar::Io(_)) => return Err("BAR0 is I/O space"),
        Err(e) => return Err(e.as_str()),
    };
    // Memory decode only: no DMA and no interrupts until `wifi -up`.
    f.enable(true, false, false);
    let pm = f.set_power_d0();
    if matches!(pm, Some(s) if s != 0) {
        delay_ms(10); // D3hot -> D0 recovery time
    }
    com1(format_args!(
        "wifi: Qualcomm 17CB:1103 at {}:{}.{}, BAR0 {:#x} ({} KiB), power state {}\n",
        a.bus,
        a.slot,
        a.func,
        bar.phys(),
        bar.len() / 1024,
        match pm {
            Some(0) => "D0",
            Some(1) => "D1 -> D0",
            Some(2) => "D2 -> D0",
            Some(_) => "D3hot -> D0",
            None => "unknown (no PM capability)",
        }
    ));
    if bar.len() < hw::BAR_NEEDED {
        com1(format_args!("wifi: BAR0 smaller than the register window needs ({} KiB)\n", hw::BAR_NEEDED / 1024));
    }
    let msi_cap = msi::info(&f);
    let mut dev = Ath11k {
        pci: f,
        bar,
        window: None,
        soc_version: 0,
        sub_version: None,
        chip: Chip::Unsupported { major: 0, minor: 0 },
        msi_cap,
        irqs: None,
        msi_messages: 0,
        last_mhi: None,
        up_runs: 0,
        up_ok: false,
        fw: None,
        mhi: None,
        w1: W1::NotRun,
        m3: None,
        board2: None,
        qmi: None,
        w2: W1::NotRun,
        cap: None,
        mem_mode: MemMode::Aligned,
        w3: W1::NotRun,
        ce: None,
        w3b: W1::NotRun,
        htc_ready: None,
    };
    let soc = dev.r32(hw::TCSR_SOC_HW_VERSION);
    dev.soc_version = soc;
    if soc == 0xFFFF_FFFF {
        com1(format_args!("wifi: chip version reads all-ones (card powered off or link down)\n"));
    } else {
        let (major, minor) = hw::decode_version(soc);
        if hw::needs_sub_version(major, minor) {
            dev.sub_version = Some(dev.r32(hw::TCSR_SOC_HW_SUB_VER));
        }
        dev.chip = hw::chip(major, minor, dev.sub_version);
        com1(format_args!(
            "wifi: chip {} (soc version {:#010x}, major {} minor {:#x}{})\n",
            dev.chip.name(),
            soc,
            major,
            minor,
            match dev.sub_version {
                Some(s) => SubVer(s),
                None => SubVer(u32::MAX),
            }
        ));
    }
    match &dev.msi_cap {
        Some(m) => com1(format_args!(
            "wifi: MSI capability: up to {} messages, {}-bit address, per-vector mask {}\n",
            m.capable,
            if m.is_64 { 64 } else { 32 },
            if m.per_vector_mask { "yes" } else { "no" }
        )),
        None => com1(format_args!("wifi: no MSI capability (will poll)\n")),
    }
    let s = dev.mhi_snapshot();
    com1(format_args!(
        "wifi: MHI state {} ({}), EE {}\n",
        mhi::state_name(s.state()),
        if s.dead() { "not answering" } else { "before reset" },
        s.ee.map_or("?", mhi::ee_name)
    ));
    dev.last_mhi = Some(s);
    // Whether firmware left the IOMMU in the way of this card's DMA.
    let devs = crate::drivers::bus::iommu::interesting_devices(&[(dev.pci.addr(), "wifi")]);
    crate::drivers::bus::iommu::report(&devs, &mut |args| {
        let _ = crate::Com1.write_fmt(args);
        let _ = crate::Com1.write_str("\n");
    });
    *WIFI0.lock() = Some(dev);
    Ok(())
}

/// MHIVER, or "n/a" for the placeholders the boot ROM returns (0xdeadbeef)
/// and an unanswered read (all-ones).
struct MhiVer(u32);
impl core::fmt::Display for MhiVer {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self.0 {
            0xDEAD_BEEF | 0xFFFF_FFFF => write!(f, "n/a"),
            v => write!(f, "{:#x}", v),
        }
    }
}

/// ", sub version 0x..." when present.
struct SubVer(u32);
impl core::fmt::Display for SubVer {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        if self.0 == u32::MAX {
            Ok(())
        } else {
            write!(f, ", sub version {:#010x}", self.0)
        }
    }
}

/// One progress line on screen (and so in POPCORN.LOG), flushed to the boot
/// stick right away: if the card hangs the bus, the log shows the last step.
fn step(color: u8, args: core::fmt::Arguments) {
    let mut line = String::new();
    let _ = line.write_str("wifi: ");
    let _ = line.write_fmt(args);
    println_color(&line, color);
    let _ = crate::klog::flush();
}

fn level_color(l: Level) -> u8 {
    match l {
        Level::Info => COLOR_WHITE,
        Level::Ok => COLOR_LIGHT_GREEN,
        Level::Warn => COLOR_YELLOW,
        Level::Err => COLOR_LIGHT_RED,
    }
}

fn ms_since(t0: u64) -> u64 {
    let hz = crate::drivers::backends::cpuinfo::tsc_hz().max(1000);
    unsafe { core::arch::x86_64::_rdtsc() }.wrapping_sub(t0) / (hz / 1000)
}

/// `wifi -up` (W0 then, if the reset went well, W1) or `wifi -reset` (W0 only).
/// Safe to run more than once: every run starts from a fresh reset.
pub fn up(load_firmware: bool) {
    let mut g = WIFI0.lock();
    let Some(dev) = g.as_mut() else {
        println_color("wifi: no supported Wi-Fi card found (Qualcomm WCN6855, PCI 17CB:1103)", COLOR_YELLOW);
        return;
    };
    w0(dev);
    if load_firmware && dev.up_ok {
        w1(dev);
        if dev.w1 == W1::Running {
            w2(dev);
        }
        if dev.w2 == W1::Running {
            w3a(dev);
        }
        if dev.w3 == W1::Running {
            w3b(dev);
        }
    }
}

/// W2: QMI handshake with the firmware's WLFW service: indications, host
/// capabilities, the memory it asks for, its capabilities, regulatory
/// database and board data, M3, then FW_READY.
fn w2(dev: &mut Ath11k) {
    const T: u32 = 10_000; // ath11k's per-request timeout
    let Some(dir) = fw_dir(dev.chip) else { return };
    step(COLOR_LIGHT_CYAN, format_args!("W2: QMI handshake with the firmware"));
    for (name, max, slot) in [("M3.BIN", 4usize << 20, 0u8), ("BOARD-2.BIN", 32 << 20, 1)] {
        let have = if slot == 0 { dev.m3.is_some() } else { dev.board2.is_some() };
        if have {
            continue;
        }
        match load_file(dir, name, max) {
            Ok(v) => {
                if slot == 0 {
                    dev.m3 = Some(v)
                } else {
                    dev.board2 = Some(v)
                }
            }
            Err(why) => {
                dev.w2 = W1::Failed(why);
                step(COLOR_LIGHT_RED, format_args!("W2 stopped: {}: {}", name, why));
                return;
            }
        }
    }
    let (sub_vendor, sub_device) = (dev.pci.cfg_r16(0x2C), dev.pci.cfg_r16(0x2E));
    step(COLOR_WHITE, format_args!("card subsystem id {:04x}:{:04x}", sub_vendor, sub_device));
    let r = w2_inner(dev, sub_vendor, sub_device, T);
    match r {
        Ok(()) => {
            dev.w2 = W1::Running;
            let fw = dev.cap.as_ref().map(|c| c.fw_build_id.clone()).unwrap_or_default();
            step(
                COLOR_LIGHT_GREEN,
                format_args!("W2 done: firmware {} is ready (next: W3, WMI and scanning)", if fw.is_empty() { "?" } else { &fw }),
            );
        }
        Err(why) => {
            dev.w2 = W1::Failed(why);
            if let Some(l) = &dev.qmi {
                step(COLOR_WHITE, format_args!("QRTR: {} packets received, {} sent; firmware node {:?}, WLFW at {:?}", l.rx_packets, l.tx_packets, l.remote_node, l.svc));
            }
            if let (Some(h), Some(_)) = (&dev.mhi, &dev.irqs) {
                let mut say = |l: Level, args: core::fmt::Arguments| step(level_color(l), args);
                h.dump(&dev.bar, &mut say);
            }
            step(COLOR_LIGHT_RED, format_args!("W2 stopped: {} (wifi -up retries from a fresh reset)", why));
        }
    }
}

fn w2_inner(dev: &mut Ath11k, sub_vendor: u16, sub_device: u16, t: u32) -> Result<(), &'static str> {
    use super::board2;
    use super::qmi;
    use super::qmi_link::Link;
    let bar = &dev.bar;
    let host = dev.mhi.as_mut().ok_or("no MHI host (W1 not done)")?;
    let mut say = |l: Level, args: core::fmt::Arguments| step(level_color(l), args);

    // ---- transport: MHI channels, QRTR hello, WLFW service ----
    let mut link = Link::open(host, bar, &WCN6855_MHI, &mut say)?;
    link.hello(host, bar, &mut say)?;
    let found = link.wait(host, bar, 5000, &mut say, |l| l.svc.is_some())?;
    if !found {
        say(Level::Warn, format_args!("QRTR: WLFW not announced within 5 s; asking the firmware's name server"));
        link.lookup(host, bar, &mut say)?;
        if !link.wait(host, bar, 5000, &mut say, |l| l.svc.is_some())? {
            dev.qmi = Some(link);
            return Err("firmware never offered its WLFW service");
        }
    }
    let (node, port) = link.svc.unwrap_or((0, 0));
    say(Level::Ok, format_args!("QMI: WLFW service at node {} port {:#x}", node, port));

    // ---- registration and host capabilities ----
    let txn = link.next_txn();
    link.request(host, bar, qmi::ind_register_req(txn), t, &mut say)?;
    say(Level::Info, format_args!("QMI: indications registered"));
    let txn = link.next_txn();
    link.request(host, bar, qmi::host_cap_req(txn, false), t, &mut say)?;
    say(Level::Info, format_args!("QMI: host capabilities accepted"));

    // ---- memory the firmware asks for ----
    let mode = dev.mem_mode;
    let body = link.wait_ind(host, bar, qmi::REQUEST_MEM_IND, t, &mut say)?;
    let mut segs = qmi::request_mem_ind(&body)?;
    let total: u64 = segs.iter().map(|s| s.size as u64).sum();
    say(Level::Info, format_args!("QMI: firmware asks for {} memory segment(s), {} KiB in all", segs.len(), total / 1024));
    if mode == MemMode::Delayed {
        // No memory now: the firmware then asks again in smaller segments.
        say(Level::Info, format_args!("QMI: memory mode delayed: answering with no memory so the firmware asks for smaller segments"));
        let txn = link.next_txn();
        match link.request(host, bar, qmi::respond_mem_req(txn, &[]), t, &mut say) {
            Ok(_) | Err("QMI request refused by the firmware") => {}
            Err(e) => {
                dev.qmi = Some(link);
                return Err(e);
            }
        }
        let body = link.wait_ind(host, bar, qmi::REQUEST_MEM_IND, t, &mut say)?;
        segs = qmi::request_mem_ind(&body)?;
        let total: u64 = segs.iter().map(|s| s.size as u64).sum();
        say(Level::Info, format_args!("QMI: firmware asks again: {} segment(s), {} KiB in all", segs.len(), total / 1024));
    }
    let mut granted = Vec::with_capacity(segs.len());
    for s in &segs {
        if s.size == 0 {
            granted.push((0u64, 0u32, s.kind));
            continue;
        }
        // Naturally aligned, as Linux's allocator / IOMMU would give it.
        let align = (s.size as usize).next_power_of_two().clamp(4096, 32 << 20);
        let m = host
            .pool()
            .alloc_large_aligned(s.size as usize, align)
            .map_err(|_| "out of DMA memory for the firmware's request")?;
        let a = m.dev().get();
        say(Level::Info, format_args!("QMI: segment type {} {} KiB at {:#x} ({} KiB aligned)", s.kind, s.size / 1024, a, align / 1024));
        granted.push((a, s.size, s.kind));
        core::mem::forget(m); // owned by the host pool for the device's lifetime
    }
    let txn = link.next_txn();
    let r = link.request(host, bar, qmi::respond_mem_req(txn, &granted), t, &mut say);
    if r == Err("device entered SYS_ERR") || r == Err("firmware crashed (RDDM)") {
        if mode == MemMode::Aligned {
            dev.mem_mode = MemMode::Delayed;
            say(Level::Warn, format_args!("QMI: firmware crashed right after the memory answer; the next wifi -up tries the delayed (smaller segments) path"));
        } else {
            dev.mem_mode = MemMode::Aligned;
            say(Level::Warn, format_args!("QMI: firmware crashed after the delayed memory answer too; the next wifi -up goes back to aligned blocks"));
        }
    }
    r?;
    link.wait_ind(host, bar, qmi::FW_MEM_READY_IND, t, &mut say)?;
    say(Level::Ok, format_args!("QMI: firmware memory ready ({} mode)", if mode == MemMode::Aligned { "aligned" } else { "delayed" }));

    // ---- capabilities ----
    let txn = link.next_txn();
    let body = link.request(host, bar, qmi::cap_req(txn), t, &mut say)?;
    let cap = qmi::cap_resp(&body);
    let chip_id = cap.chip_id.unwrap_or(0);
    let board_id = cap.board_id.unwrap_or(0xFF);
    say(
        Level::Ok,
        format_args!(
            "QMI: chip id {} family {:?}, board id {}, soc id {:#x}, fw version {:#x} ({}), build {}",
            chip_id,
            cap.chip_family,
            board_id,
            cap.soc_id.unwrap_or(0),
            cap.fw_version.unwrap_or(0),
            cap.fw_build_timestamp,
            cap.fw_build_id
        ),
    );

    // ---- regulatory database, then board data ----
    let b2 = dev.board2.as_deref().ok_or("BOARD-2.BIN not loaded")?;
    let names = board2::board_names(0x17CB, 0x1103, sub_vendor, sub_device, chip_id, board_id);
    let regdb = board2::regdb_names(&names[0]).iter().find_map(|n| board2::find(b2, board2::IE_REGDB, n).ok().map(|d| (n.clone(), d)));
    match regdb {
        Some((n, data)) => {
            say(Level::Info, format_args!("board: regulatory database '{}' ({} bytes)", n, data.len()));
            download(&mut link, host, bar, board_id, data, qmi::BDF_TYPE_REGDB, t, &mut say)?;
        }
        None => say(Level::Warn, format_args!("board: no regulatory database in BOARD-2.BIN (continuing, as ath11k does)")),
    }
    let board = names.iter().find_map(|n| board2::find(b2, board2::IE_BOARD, n).ok().map(|d| (n.clone(), d)));
    let Some((n, data)) = board else {
        say(Level::Err, format_args!("board: none of these names is in BOARD-2.BIN: '{}', '{}'", names[0], names[1]));
        dev.qmi = Some(link);
        return Err("no board data for this card");
    };
    let data = &data[..data.len().min(qmi::BOARD_MAX)];
    let kind = qmi::bdf_type_of(data);
    say(Level::Info, format_args!("board: '{}' ({} bytes, {})", n, data.len(), if kind == qmi::BDF_TYPE_ELF { "ELF" } else { "BIN" }));
    download(&mut link, host, bar, board_id, data, kind, t, &mut say)?;

    // ---- M3 ----
    let m3 = dev.m3.as_deref().ok_or("M3.BIN not loaded")?;
    let mb = host.pool().alloc_large(m3.len()).map_err(|_| "out of DMA memory for M3")?;
    unsafe { core::ptr::copy_nonoverlapping(m3.as_ptr(), mb.as_ptr(), m3.len()) };
    let addr = mb.dev().get();
    core::mem::forget(mb);
    let txn = link.next_txn();
    link.request(host, bar, qmi::m3_info_req(txn, addr, m3.len() as u32), t, &mut say)?;
    say(Level::Info, format_args!("QMI: M3 ({} bytes at {:#x}) accepted", m3.len(), addr));

    // ---- ready ----
    // ath11k treats FW_READY and FW_INIT_DONE alike (whichever arrives first
    // starts the firmware); WCN6855 sends FW_INIT_DONE, here right after the
    // board data (T14s: before the M3 answer), and no FW_READY.
    let ready = link.wait(host, bar, t, &mut say, |l| l.has_ind(qmi::FW_READY_IND) || l.has_ind(qmi::FW_INIT_DONE_IND))?;
    if !ready {
        say(Level::Err, format_args!("QMI: neither FW_READY nor FW_INIT_DONE in {} ms", t));
        dev.qmi = Some(link);
        return Err("firmware never reported ready");
    }
    say(
        Level::Ok,
        format_args!(
            "QMI: firmware initialized ({}{})",
            if link.has_ind(qmi::FW_INIT_DONE_IND) { "FW_INIT_DONE" } else { "FW_READY" },
            if link.has_ind(qmi::FW_INIT_DONE_IND) && link.has_ind(qmi::FW_READY_IND) { " and FW_READY" } else { "" }
        ),
    );
    dev.cap = Some(cap);
    dev.qmi = Some(link);
    Ok(())
}

/// W3a: start the firmware's WLAN subsystem, as ath11k does right after
/// FW_INIT_DONE: WLAN_INI (firmware log on), WLAN_CFG (the copy-engine,
/// service-to-pipe and shadow-register tables the host will use), WLAN_MODE
/// (normal). The firmware answers each with success or an error code.
fn w3a(dev: &mut Ath11k) {
    use super::qmi;
    const T: u32 = 10_000;
    dev.w3 = W1::NotRun;
    step(COLOR_LIGHT_CYAN, format_args!("W3a: starting the firmware's WLAN subsystem (WLAN_INI, WLAN_CFG, WLAN_MODE)"));
    let r = (|| -> Result<(), &'static str> {
        let bar = &dev.bar;
        let host = dev.mhi.as_mut().ok_or("no MHI host")?;
        let link = dev.qmi.as_mut().ok_or("no QMI link (W2 not done)")?;
        let mut say = |l: Level, args: core::fmt::Arguments| step(level_color(l), args);
        let txn = link.next_txn();
        link.request(host, bar, qmi::wlan_ini_req(txn, true), T, &mut say)?;
        say(Level::Info, format_args!("QMI: WLAN_INI accepted (firmware log on)"));
        let txn = link.next_txn();
        let msg = qmi::wlan_cfg_req(txn);
        let len = msg.len();
        link.request(host, bar, msg, T, &mut say)?;
        say(
            Level::Info,
            format_args!(
                "QMI: WLAN_CFG accepted ({} copy engines, {} service routes, {} shadow registers, {} bytes)",
                qmi::TARGET_CE.len(),
                qmi::SVC_MAP.len(),
                qmi::SHADOW_V2.len(),
                len
            ),
        );
        let txn = link.next_txn();
        link.request(host, bar, qmi::wlan_mode_req(txn, qmi::FIRMWARE_MODE_NORMAL), T, &mut say)?;
        say(Level::Info, format_args!("QMI: WLAN_MODE normal accepted"));
        Ok(())
    })();
    match r {
        Ok(()) => {
            dev.w3 = W1::Running;
            step(COLOR_LIGHT_GREEN, format_args!("W3a done: the firmware's WLAN subsystem is started (next: copy engines, HTC, WMI)"));
        }
        Err(why) => {
            dev.w3 = W1::Failed(why);
            if let (Some(h), Some(l)) = (&dev.mhi, &dev.qmi) {
                step(COLOR_WHITE, format_args!("QRTR: {} packets received, {} sent", l.rx_packets, l.tx_packets));
                let mut say = |l: Level, args: core::fmt::Arguments| step(level_color(l), args);
                h.dump(&dev.bar, &mut say);
            }
            step(COLOR_LIGHT_RED, format_args!("W3a stopped: {} (wifi -up retries from a fresh reset)", why));
        }
    }
}

/// W3b: copy engines. Allocate and program the rings HTC needs (CE0 and CE3
/// host->target, CE1 and CE2 target->host), post receive buffers, then wait
/// for the target's HTC READY message, which arrives on CE2.
fn w3b(dev: &mut Ath11k) {
    use super::ath11k_ce::Ce;
    dev.w3b = W1::NotRun;
    dev.htc_ready = None;
    // A previous run's rings: the device was reset since; leak, never free.
    if let Some(old) = dev.ce.take() {
        core::mem::forget(old);
    }
    step(COLOR_LIGHT_CYAN, format_args!("W3b: copy engines (CE0/CE3 send, CE1/CE2 receive), waiting for HTC READY"));
    let pipes = [(0u32, true, false), (1, false, true), (2, false, true), (3, true, false)];
    let alloc = match dev.mhi.as_mut() {
        Some(h) => Ce::alloc(h.pool(), &pipes),
        None => Err("no MHI host"),
    };
    let mut ce = match alloc {
        Ok(c) => c,
        Err(why) => {
            dev.w3b = W1::Failed(why);
            step(COLOR_LIGHT_RED, format_args!("W3b stopped: {}", why));
            return;
        }
    };
    {
        // Ring registers sit above the window start: go through the window.
        let devp: *mut Ath11k = dev;
        let mut w = |off: u32, v: u32| unsafe { (*devp).w32(off, v) };
        let mut r = |off: u32| unsafe { (*devp).r32(off) };
        ce.start(&mut w, &mut r);
    }
    step(COLOR_WHITE, format_args!("CE: rings programmed, pointer array at {:#x}, receive buffers posted", ce.rdp_base()));

    // Poll for HTC READY (ath11k waits 1 s; allow 5).
    let mut ready: Option<(u16, u16, u8)> = None;
    let mut other = 0u32;
    let bar = &dev.bar;
    let mut w = |off: u32, v: u32| bar.w32(off as usize, v); // shadow slots: direct
    let _ = poll_ms(5000, || {
        ce.poll_rx(&mut w, &mut |pipe, data| {
            let h = hw::htc_hdr(data);
            match h {
                Some(h) if h.eid == 0 => {
                    if let Some(r) = hw::htc_ready(&data[hw::HTC_HDR..]) {
                        ready = Some(r);
                    } else {
                        other += 1;
                    }
                }
                Some(h) => {
                    other += 1;
                    let _ = (pipe, h);
                }
                None => other += 1,
            }
        });
        ready.is_some()
    });
    for (p, rx, tx, hwp, hostp) in ce.stats() {
        step(COLOR_WHITE, format_args!("CE{}: {} received, {} sent, hw pointer {:#x}, host pointer {:#x}", p, rx, tx, hwp, hostp));
    }
    dev.ce = Some(ce);
    match ready {
        Some((credits, size, eps)) => {
            dev.htc_ready = ready;
            dev.w3b = W1::Running;
            step(
                COLOR_LIGHT_GREEN,
                format_args!("W3b done: HTC READY from the firmware: {} credit(s) of {} bytes, {} endpoints (next: HTC connect, WMI)", credits, size, eps),
            );
        }
        None => {
            dev.w3b = W1::Failed("no HTC READY from the firmware");
            step(COLOR_LIGHT_RED, format_args!("W3b stopped: no HTC READY in 5 s ({} other message(s) received)", other));
        }
    }
}

/// BDF_DOWNLOAD in 6 KiB chunks (total = bytes remaining, as ath11k sends it).
#[allow(clippy::too_many_arguments)]
fn download(
    link: &mut super::qmi_link::Link,
    host: &mut Host,
    bar: &MmioRegion,
    file_id: u32,
    data: &[u8],
    kind: u8,
    t: u32,
    say: &mut dyn FnMut(Level, core::fmt::Arguments),
) -> Result<(), &'static str> {
    use super::qmi;
    let mut off = 0usize;
    let mut seg = 0u32;
    while off < data.len() {
        let n = (data.len() - off).min(qmi::BDF_CHUNK);
        let end = off + n == data.len();
        let txn = link.next_txn();
        let msg = qmi::bdf_req(txn, file_id, (data.len() - off) as u32, seg, &data[off..off + n], end, kind);
        link.request(host, bar, msg, t, say)?;
        off += n;
        seg += 1;
    }
    say(Level::Info, format_args!("QMI: {} bytes downloaded in {} chunk(s) (type {})", data.len(), seg, kind));
    Ok(())
}

/// PCI/PCIe error status of the card and every bridge above it (what a failed
/// DMA left behind: received master abort, unsupported request, ...).
fn err_dump(me: crate::drivers::bus::pci::PciAddr) {
    use crate::drivers::bus::pci;
    let mut all = alloc::vec![me];
    all.extend(pci::bridge_path(me));
    for a in all {
        let e = pci::err_status(a);
        step(
            COLOR_WHITE,
            format_args!(
                "pci {}:{}.{}: command {:#06x} status {:#06x}{}{} pcie devsta {:#x}{}",
                a.bus,
                a.slot,
                a.func,
                e.command,
                e.status,
                if e.status & (1 << 13) != 0 { " (received master abort)" } else { "" },
                match e.sec_status {
                    Some(s) => alloc::format!(" secondary {:#06x}{}", s, if s & (1 << 13) != 0 { " (master abort below)" } else { "" }),
                    None => String::new(),
                },
                e.dev_status.unwrap_or(0),
                match e.dev_status {
                    Some(d) if d & 0x8 != 0 => " (unsupported request seen)",
                    Some(d) if d & 0x7 != 0 => " (errors seen)",
                    _ => "",
                }
            ),
        );
    }
}

/// Read AMSS.BIN and check it against MANIFEST.TXT.
fn load_firmware(dir: &str) -> Result<Vec<u8>, &'static str> {
    load_file(dir, "AMSS.BIN", 32 << 20)
}

/// Read `name` from the firmware directory and check it against MANIFEST.TXT.
fn load_file(dir: &str, name: &str, max: usize) -> Result<Vec<u8>, &'static str> {
    let t0 = unsafe { core::arch::x86_64::_rdtsc() };
    let mut path = String::new();
    let _ = write!(path, "{}/MANIFEST.TXT", dir);
    let man = crate::fs::bootfs::read(&path, 64 * 1024, &mut |_, _| {})?;
    let e = super::fw_manifest::entry(&man, name)?;
    let hex = crate::util::sha256::to_hex(&e.sha256);
    step(
        COLOR_WHITE,
        format_args!(
            "firmware: {}/{}, {} bytes, sha256 {}...",
            dir,
            name,
            e.size,
            core::str::from_utf8(&hex[..16]).unwrap_or("?")
        ),
    );
    path.clear();
    let _ = write!(path, "{}/{}", dir, name);
    let mut next = 1usize << 20;
    let data = crate::fs::bootfs::read(&path, max, &mut |done, total| {
        if done >= next || (done == total && total >= 1 << 20) {
            step(COLOR_WHITE, format_args!("firmware: {} {} / {} KiB read", name, done / 1024, total / 1024));
            while next <= done {
                next += 1 << 20;
            }
        }
    })?;
    super::fw_manifest::verify(&data, &e)?;
    step(
        COLOR_LIGHT_GREEN,
        format_args!("firmware: {} verified (size and SHA-256 match the manifest) in {} ms", name, ms_since(t0)),
    );
    Ok(data)
}

/// W1: boot the main firmware over MHI.
fn w1(dev: &mut Ath11k) {
    let Some(dir) = fw_dir(dev.chip) else {
        dev.w1 = W1::Skipped("no firmware known for this chip");
        step(COLOR_YELLOW, format_args!("W1 skipped: no firmware known for {}", dev.chip.name()));
        return;
    };
    step(COLOR_LIGHT_CYAN, format_args!("W1: loading firmware into {} over MHI", dev.chip.name()));
    if dev.fw.is_none() {
        match load_firmware(dir) {
            Ok(v) => dev.fw = Some(v),
            Err(why) => {
                dev.w1 = W1::Failed(why);
                step(COLOR_LIGHT_RED, format_args!("W1 stopped: firmware: {} (rebuild the image with core.sh firmware)", why));
                return;
            }
        }
    } else {
        step(COLOR_WHITE, format_args!("firmware: AMSS.BIN already read and verified this boot"));
    }
    // MHI's three messages (BHI, control ring, data ring) may interrupt now:
    // the host polls, the counts show whether MSI from the card works.
    if let Some(set) = &dev.irqs {
        for h in set.handles[..set.count.min(3)].iter() {
            h.unmask();
        }
    }
    // Fresh error bits, so a failure dump shows only what this attempt caused.
    crate::drivers::bus::pci::clear_err_status(dev.pci.addr());
    for b in crate::drivers::bus::pci::bridge_path(dev.pci.addr()) {
        crate::drivers::bus::pci::clear_err_status(b);
    }
    let Some(fw) = dev.fw.as_deref() else { return };
    let irqs = &dev.irqs;
    let irq_count = || irqs.as_ref().map_or(0, |s| s.handles[..s.count].iter().map(|h| h.count()).sum());
    let mut say = |l: Level, args: core::fmt::Arguments| step(level_color(l), args);
    let r = mhi_host::boot(&WCN6855_MHI, &dev.bar, dev.pci.grant(), fw, &irq_count, &mut say);
    match r {
        Ok(h) => {
            dev.mhi = Some(h);
            dev.w1 = W1::Running;
            step(
                COLOR_LIGHT_GREEN,
                format_args!("W1 done: {} is running its main firmware (next: W2, QMI handshake)", dev.chip.name()),
            );
        }
        Err(f) => {
            // A failed host's memory is poisoned: dropping it leaks, never frees.
            drop(f.host);
            dev.w1 = W1::Failed(f.why);
            // Interrupts per MHI message, and whether the IOMMU is in the way of
            // the card's DMA (firmware "Kernel DMA Protection").
            if let Some(set) = &dev.irqs {
                let c: [u64; 3] = core::array::from_fn(|i| set.handles.get(i).map_or(0, |h| h.count()));
                step(COLOR_WHITE, format_args!("MSI so far: BHI {} / control ring {} / data ring {}", c[0], c[1], c[2]));
            }
            let me = dev.pci.addr();
            err_dump(me);
            let devs = crate::drivers::bus::iommu::interesting_devices(&[(me, "wifi")]);
            crate::drivers::bus::iommu::report(&devs, &mut |args| step(COLOR_WHITE, args));
            step(COLOR_LIGHT_RED, format_args!("W1 stopped: {} (wifi -up retries from a fresh reset)", f.why));
        }
    }
    dev.last_mhi = Some(dev.mhi_snapshot());
}

/// W0: power-on reset, MSI, MHI state.
fn w0(dev: &mut Ath11k) {
    // A previous run's MHI host: the reset below stops the device, but never
    // free memory a device may have been writing, so leak it instead.
    if let Some(h) = dev.mhi.take() {
        h.abandon();
    }
    dev.w1 = W1::NotRun;
    dev.up_runs += 1;
    dev.up_ok = false;
    if !dev.chip.supported() {
        step(COLOR_LIGHT_RED, format_args!("{} (soc version {:#010x}): not bringing it up", dev.chip.name(), dev.soc_version));
        return;
    }
    step(COLOR_LIGHT_CYAN, format_args!("bringing up {} (W0: reset, MSI, MHI state)", dev.chip.name()));
    // Bus mastering now: MSI messages are memory writes from the device. The
    // bridges above the card must forward them too.
    dev.pci.enable(true, false, true);
    for (b, before, after) in dev.pci.enable_upstream() {
        step(
            if after & 0x6 == 0x6 { COLOR_WHITE } else { COLOR_YELLOW },
            format_args!(
                "upstream bridge {}:{}.{}: command {:#06x} -> {:#06x}{}",
                b.bus,
                b.slot,
                b.func,
                before,
                after,
                if before & 0x4 == 0 { " (bus mastering was OFF)" } else { "" }
            ),
        );
    }
    let me = dev.pci.addr();
    let card = crate::drivers::bus::pci::err_status(me);
    step(COLOR_WHITE, format_args!("card command {:#06x} status {:#06x} pcie devsta {:#x}", card.command, card.status, card.dev_status.unwrap_or(0)));
    // Old grants first, so a second run starts from MSI off.
    if dev.irqs.take().is_some() {
        msi::disable(&dev.pci);
    }
    dev.msi_messages = 0;

    // ---- stop firmware that is still running (a second `wifi -up`) ----
    // As Linux's ath11k power-down: wake the SoC, then ask the running firmware
    // to leave MHI through an MHI reset and wait for it, before the hard reset.
    // Skipping this left state behind that made the next SBL fail (T14s:
    // SBL accepted, then never READY, BHI ERRDBG2/3 = 0x143b578/0x2c).
    dev.w32(hw::PCIE_SOC_WAKE, 1);
    delay_ms(5);
    let st = dev.bar.r32(mhi::MHISTATUS);
    let ee = dev.last_mhi.and_then(|s| if s.bhi_off != 0xFFFF_FFFF { Some(s.bhi_off as usize) } else { None })
        .map(|b| dev.bar.r32(b + mhi::BHI_EXECENV));
    if st != 0xFFFF_FFFF && (mhi::state_of(st) != mhi::STATE_RESET || ee.map_or(false, |e| e != mhi::EE_PBL)) {
        step(
            COLOR_WHITE,
            format_args!(
                "firmware still running (MHI {}, EE {}): asking it to stop (MHI reset)",
                mhi::state_name(mhi::state_of(st)),
                ee.map_or("?", mhi::ee_name)
            ),
        );
        let ctrl = dev.bar.r32(mhi::MHICTRL);
        dev.bar.w32(mhi::MHICTRL, mhi::mhictrl_with_state(ctrl, mhi::STATE_RESET) | mhi::MHICTRL_RESET);
        let bar = &dev.bar;
        match poll_ms(2000, || {
            mhi::state_of(bar.r32(mhi::MHISTATUS)) == mhi::STATE_RESET && bar.r32(mhi::MHICTRL) & mhi::MHICTRL_RESET == 0
        }) {
            Some(ms) => step(COLOR_WHITE, format_args!("firmware left MHI after {} ms", ms)),
            None => step(
                COLOR_YELLOW,
                format_args!("firmware did not leave MHI within 2 s (status {:#x}); resetting anyway", bar.r32(mhi::MHISTATUS)),
            ),
        }
    }

    // ---- power-on reset ----
    delay_ms(100);
    let mut v = dev.r32(hw::PCIE_PARF_LTSSM);
    let mut tries = 0;
    while v != hw::PARF_LTSSM_VALUE && tries < 5 {
        if v == 0xFFFF_FFFF {
            delay_ms(5);
        }
        dev.w32(hw::PCIE_PARF_LTSSM, hw::PARF_LTSSM_VALUE);
        v = dev.r32(hw::PCIE_PARF_LTSSM);
        tries += 1;
    }
    step(
        if v == hw::PARF_LTSSM_VALUE { COLOR_WHITE } else { COLOR_YELLOW },
        format_args!("LTSSM {:#x} after {} write(s){}", v, tries, if v == hw::PARF_LTSSM_VALUE { "" } else { " (expected 0x111)" }),
    );
    let v = dev.r32(hw::GCC_PCIE_HOT_RST) | hw::GCC_PCIE_HOT_RST_VAL;
    dev.w32(hw::GCC_PCIE_HOT_RST, v);
    let _ = dev.r32(hw::GCC_PCIE_HOT_RST);
    delay_ms(5);
    dev.w32(hw::PCIE_INT_ALL_CLEAR, 0xFFFF_FFFF);
    let v = dev.r32(hw::WLAON_QFPROM_PWR_CTRL) & !hw::QFPROM_PWR_CTRL_VDD4BLOW;
    dev.w32(hw::WLAON_QFPROM_PWR_CTRL, v);
    for r in hw::MHI_VECTOR_REGS {
        dev.w32(r, 0);
    }
    let cookie = dev.r32(hw::PCIE_Q6_COOKIE);
    let warm = dev.r32(hw::WLAON_WARM_SW_ENTRY);
    delay_ms(10);
    dev.w32(hw::WLAON_WARM_SW_ENTRY, 0);
    delay_ms(10);
    let _ = dev.r32(hw::WLAON_WARM_SW_ENTRY);
    let cause = dev.r32(hw::WLAON_SOC_RESET_CAUSE); // read-to-clear
    step(COLOR_WHITE, format_args!("before reset: q6 cookie {:#x}, warm entry {:#x}, reset cause {:#x}", cookie, warm, cause));

    let v = dev.r32(hw::PCIE_SOC_GLOBAL_RESET);
    dev.w32(hw::PCIE_SOC_GLOBAL_RESET, v | hw::SOC_GLOBAL_RESET_V);
    let _ = dev.r32(hw::PCIE_SOC_GLOBAL_RESET);
    delay_ms(10);
    dev.w32(hw::PCIE_SOC_GLOBAL_RESET, v & !hw::SOC_GLOBAL_RESET_V);
    let _ = dev.r32(hw::PCIE_SOC_GLOBAL_RESET);
    delay_ms(10);
    if dev.r32(hw::PCIE_SOC_GLOBAL_RESET) == 0xFFFF_FFFF {
        step(COLOR_LIGHT_RED, format_args!("SoC global reset: link down afterwards (register reads all-ones); stopping"));
        return;
    }
    dev.window = None; // the reset may have cleared the window register
    step(COLOR_WHITE, format_args!("SoC global reset done"));

    let status = dev.r32(mhi::MHISTATUS as u32);
    dev.w32(mhi::MHICTRL as u32, mhi::MHICTRL_RESET);
    delay_ms(10);
    let cleared = poll_ms(1000, || dev.bar.r32(mhi::MHICTRL) & mhi::MHICTRL_RESET == 0);
    match cleared {
        Some(ms) => step(COLOR_WHITE, format_args!("MHI reset (status before {:#x}) cleared after {} ms", status, ms + 10)),
        None => step(COLOR_YELLOW, format_args!("MHI reset (status before {:#x}) still set after 1 s", status)),
    }

    // ---- link power management and interrupts ----
    match dev.pci.pcie_aspm_disable() {
        Some(old) => step(COLOR_WHITE, format_args!("ASPM off (link control was {:#06x})", old)),
        None => step(COLOR_YELLOW, format_args!("no PCIe capability: ASPM left alone")),
    }
    let mut why32 = "";
    for n in [32u8, 1] {
        match msi::alloc_irqs(&dev.pci, n, "wifi0") {
            Ok(set) => {
                dev.msi_messages = n;
                dev.irqs = Some(set);
                break;
            }
            Err(e) => {
                if n == 32 {
                    why32 = e;
                } else {
                    step(COLOR_YELLOW, format_args!("MSI unavailable ({}); later stages will poll", e));
                }
            }
        }
    }
    if let Some(set) = &dev.irqs {
        let (addr, data) = set.msi_message().unwrap_or((0, 0));
        let first = set.handles[0].vector().unwrap_or(0);
        let layout = if dev.msi_messages == 32 { "MHI 0-2, CE 3-12, WAKE 13, DP 14-31" } else { "all users share it" };
        step(
            COLOR_WHITE,
            format_args!(
                "MSI: {} message(s), vectors {:#04x}-{:#04x}, address {:#x} data {:#x} ({}){}{}",
                dev.msi_messages,
                first,
                first as u32 + dev.msi_messages as u32 - 1,
                addr,
                data,
                layout,
                if why32.is_empty() { "" } else { "; 32 refused: " },
                why32
            ),
        );
    }

    // ---- where the card stands ----
    let s = dev.mhi_snapshot();
    dev.last_mhi = Some(s);
    if s.dead() {
        step(COLOR_LIGHT_RED, format_args!("MHI registers read all-ones: card not answering after reset"));
        return;
    }
    let bhi_ver = if s.ee.is_some() {
        let b = s.bhi_off as usize;
        (dev.bar.r32(b + mhi::BHI_VERSION_MAJOR), dev.bar.r32(b + mhi::BHI_VERSION_MINOR))
    } else {
        (0, 0)
    };
    step(
        COLOR_WHITE,
        format_args!(
            "MHI version {}, state {}{}{}, BHI at {:#x} (v{}.{})",
            MhiVer(s.version),
            mhi::state_name(s.state()),
            if s.ready() { ", ready" } else { "" },
            if s.syserr() { ", SYSERR" } else { "" },
            s.bhi_off,
            bhi_ver.0,
            bhi_ver.1
        ),
    );
    match (s.ee, s.bhi_status) {
        (Some(ee), Some(bs)) => step(
            COLOR_WHITE,
            format_args!("execution environment {} = {}, BHI status {}", ee, mhi::ee_name(ee), mhi::bhi_status_name(bs)),
        ),
        _ => step(COLOR_YELLOW, format_args!("BHI offset {:#x} is outside BAR0: execution environment unknown", s.bhi_off)),
    }
    let pbl = s.ee == Some(0);
    dev.up_ok = !s.syserr() && pbl;
    step(
        if dev.up_ok { COLOR_LIGHT_GREEN } else { COLOR_YELLOW },
        format_args!(
            "W0 {}: {} {}",
            if dev.up_ok { "done" } else { "finished with warnings" },
            dev.chip.name(),
            if dev.up_ok {
                "is reset and waiting in its boot ROM for firmware (next: W1, MHI firmware load)"
            } else {
                "is not in the expected boot-ROM state (see the lines above)"
            }
        ),
    );
}

/// `wifi` / `wifi -info`.
pub fn info() {
    let g = WIFI0.lock();
    let Some(dev) = g.as_ref() else {
        println_color("wifi: no supported Wi-Fi card found (Qualcomm WCN6855, PCI 17CB:1103)", COLOR_YELLOW);
        return;
    };
    let a = dev.pci.addr();
    let mut l = LineBuf::new();
    let _ = write!(
        l,
        "wifi0: {} at {}:{}.{}, BAR0 {:#x} ({} KiB), soc version {:#010x}",
        dev.chip.name(),
        a.bus,
        a.slot,
        a.func,
        dev.bar.phys(),
        dev.bar.len() / 1024,
        dev.soc_version
    );
    println_color(l.as_str(), COLOR_LIGHT_CYAN);
    let mut l = LineBuf::new();
    match dev.chip.fw_dir() {
        Some(d) => {
            let _ = write!(l, "wifi0: firmware directory {} ({})", d, if dev.w1 == W1::Running { "loaded" } else { "not loaded yet" });
        }
        None => {
            let _ = write!(l, "wifi0: no firmware known for this revision");
        }
    }
    println_color(l.as_str(), COLOR_WHITE);
    let mut l = LineBuf::new();
    match (&dev.msi_cap, &dev.irqs) {
        (_, Some(set)) => {
            let first = set.handles[0].vector().unwrap_or(0);
            let total: u64 = set.handles[..set.count].iter().map(|h| h.count()).sum();
            let _ = write!(l, "wifi0: irq msi {} message(s) from vector {:#04x}, {} interrupts", dev.msi_messages, first, total);
        }
        (Some(m), None) => {
            let _ = write!(l, "wifi0: irq none yet (MSI capable of {} messages; run wifi -up)", m.capable);
        }
        (None, None) => {
            let _ = write!(l, "wifi0: irq none (no MSI capability)");
        }
    }
    println_color(l.as_str(), COLOR_WHITE);
    let mut l = LineBuf::new();
    match dev.last_mhi {
        Some(s) if !s.dead() => {
            let _ = write!(
                l,
                "wifi0: MHI state {}, EE {}, {}",
                mhi::state_name(s.state()),
                s.ee.map_or("?", mhi::ee_name),
                if dev.up_runs == 0 {
                    "W0 not run yet (wifi -up)"
                } else if dev.up_ok {
                    "W0 done"
                } else {
                    "W0 had warnings"
                }
            );
        }
        _ => {
            let _ = write!(l, "wifi0: MHI not answering");
        }
    }
    println_color(l.as_str(), COLOR_WHITE);
    let mut l = LineBuf::new();
    match dev.w1 {
        W1::NotRun => {
            let _ = write!(l, "wifi0: W1 (firmware) not run yet{}", if fw_dir(dev.chip).is_some() { " (wifi -up)" } else { "" });
        }
        W1::Skipped(why) => {
            let _ = write!(l, "wifi0: W1 skipped: {}", why);
        }
        W1::Failed(why) => {
            let _ = write!(l, "wifi0: W1 failed: {}", why);
        }
        W1::Running => {
            let ev = dev.mhi.as_ref().map_or(0, |h| h.events_seen);
            let _ = write!(l, "wifi0: W1 done: main firmware (AMSS) running, {} MHI event(s)", ev);
        }
    }
    println_color(l.as_str(), if dev.w1 == W1::Running { COLOR_LIGHT_GREEN } else { COLOR_WHITE });
    let mut l = LineBuf::new();
    match dev.w2 {
        W1::NotRun => {
            let _ = write!(l, "wifi0: W2 (QMI handshake) not run yet");
        }
        W1::Skipped(why) => {
            let _ = write!(l, "wifi0: W2 skipped: {}", why);
        }
        W1::Failed(why) => {
            let _ = write!(l, "wifi0: W2 failed: {}", why);
        }
        W1::Running => match &dev.cap {
            Some(c) => {
                let _ = write!(l, "wifi0: W2 done: chip id {} board id {}, firmware ready", c.chip_id.unwrap_or(0), c.board_id.unwrap_or(0xFF));
            }
            None => {
                let _ = write!(l, "wifi0: W2 done");
            }
        },
    }
    println_color(l.as_str(), if dev.w2 == W1::Running { COLOR_LIGHT_GREEN } else { COLOR_WHITE });
    if let Some(c) = &dev.cap {
        let mut l = LineBuf::new();
        let _ = write!(l, "wifi0: firmware build {}", c.fw_build_id);
        println_color(l.as_str(), COLOR_WHITE);
    }
    let mut l = LineBuf::new();
    match dev.w3 {
        W1::NotRun => {
            let _ = write!(l, "wifi0: W3a (WLAN start) not run yet");
        }
        W1::Skipped(why) => {
            let _ = write!(l, "wifi0: W3a skipped: {}", why);
        }
        W1::Failed(why) => {
            let _ = write!(l, "wifi0: W3a failed: {}", why);
        }
        W1::Running => {
            let _ = write!(l, "wifi0: W3a done: WLAN subsystem started");
        }
    }
    println_color(l.as_str(), if dev.w3 == W1::Running { COLOR_LIGHT_GREEN } else { COLOR_WHITE });
    let mut l = LineBuf::new();
    match (dev.w3b, dev.htc_ready) {
        (W1::Running, Some((c, s, e))) => {
            let _ = write!(l, "wifi0: W3b done: HTC ready, {} credit(s) of {} bytes, {} endpoints", c, s, e);
        }
        (W1::Failed(why), _) => {
            let _ = write!(l, "wifi0: W3b failed: {}", why);
        }
        _ => {
            let _ = write!(l, "wifi0: W3b (copy engines) not run yet");
        }
    }
    println_color(l.as_str(), if dev.w3b == W1::Running { COLOR_LIGHT_GREEN } else { COLOR_WHITE });
}
