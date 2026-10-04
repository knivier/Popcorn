//! Minimal xHCI host driver — polled, root-hub ports only, no hubs, no MSI.
//!
//! Scope (enough to drive USB Mass Storage Bulk-Only devices):
//! - PCI class 0x0C/0x03/0x30, BAR0 through the kernel direct map
//! - BIOS handoff, controller reset, DCBAA (+scratchpads), command ring,
//!   one event ring (interrupter 0, never enabled — completions are polled)
//! - Root port reset, Enable Slot, Address Device, control transfers on EP0
//! - Config descriptor parse for MSC/SCSI/BOT (08/06/50), Configure Endpoint
//! - Synchronous bulk IN/OUT with one outstanding TD
//!
//! Failures never panic: they return `Err(&'static str)`. On a failed probe
//! the controller is halted and every DMA allocation is released.

use alloc::vec::Vec;
use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use crate::drivers::bus::pci;
use crate::drivers::dma::{self, DmaBuffer};
use crate::drivers::io::outb;

const RING_TRBS: usize = 256;
const MAX_SLOTS: u32 = 8;
pub const MAX_MSC: usize = 4;

/* Capability registers. */
const CAP_HCSPARAMS1: u64 = 0x04;
const CAP_HCSPARAMS2: u64 = 0x08;
const CAP_HCCPARAMS1: u64 = 0x10;
const CAP_DBOFF: u64 = 0x14;
const CAP_RTSOFF: u64 = 0x18;

/* Operational registers (from op base). */
const OP_USBCMD: u64 = 0x00;
const OP_USBSTS: u64 = 0x04;
const OP_PAGESIZE: u64 = 0x08;
const OP_CRCR: u64 = 0x18;
const OP_DCBAAP: u64 = 0x30;
const OP_CONFIG: u64 = 0x38;
const OP_PORTSC: u64 = 0x400;

const CMD_RS: u32 = 1;
const CMD_HCRST: u32 = 1 << 1;
const STS_HCH: u32 = 1;
const STS_CNR: u32 = 1 << 11;

/* PORTSC bits. */
const PORT_CCS: u32 = 1;
const PORT_PED: u32 = 1 << 1;
const PORT_PR: u32 = 1 << 4;
const PORT_PP: u32 = 1 << 9;
const PORT_PRC: u32 = 1 << 21;
/* Preserve only RW (non-RW1C) bits when writing PORTSC. */
const PORT_KEEP: u32 = 0x0E00_C200;

/* TRB types. */
const TRB_NORMAL: u32 = 1;
const TRB_SETUP: u32 = 2;
const TRB_DATA: u32 = 3;
const TRB_STATUS: u32 = 4;
const TRB_LINK: u32 = 6;
const TRB_ENABLE_SLOT: u32 = 9;
const TRB_ADDRESS_DEV: u32 = 11;
const TRB_CONFIG_EP: u32 = 12;
const TRB_EVAL_CTX: u32 = 13;
const TRB_EV_TRANSFER: u32 = 32;
const TRB_EV_CMD: u32 = 33;

/* Completion codes. */
const CC_SUCCESS: u32 = 1;
const CC_SHORT: u32 = 13;

/* —— MMIO / timing helpers —— */

#[inline]
fn r32(a: u64) -> u32 {
    unsafe { read_volatile(a as *const u32) }
}
#[inline]
fn w32(a: u64, v: u32) {
    unsafe { write_volatile(a as *mut u32, v) }
}
#[inline]
fn w64(a: u64, v: u64) {
    w32(a, v as u32);
    w32(a + 4, (v >> 32) as u32);
}

/// ~1 µs on real hardware (legacy POST port); also yields the vCPU in QEMU.
#[inline]
fn io_delay() {
    unsafe { outb(0x80, 0) }
}
pub fn delay_ms(ms: u32) {
    for _ in 0..(ms as u64 * 1000) {
        io_delay();
    }
}

fn wait_bits(addr: u64, mask: u32, want: u32, ms: u32) -> bool {
    for _ in 0..(ms as u64 * 1000) {
        if r32(addr) & mask == want {
            return true;
        }
        io_delay();
    }
    false
}

fn alloc(bufs: &mut Vec<DmaBuffer>, len: usize) -> Result<DmaBuffer, &'static str> {
    let b = dma::dma_alloc(len).ok_or("xhci OOM")?;
    bufs.push(b);
    Ok(b)
}

/* —— TRB ring —— */

struct Ring {
    buf: DmaBuffer,
    enq: usize,
    cycle: u32,
}

impl Ring {
    /// `buf` must be a zeroed 4 KiB page (256 TRBs); last slot is a Link TRB.
    fn new(buf: DmaBuffer) -> Ring {
        let link = (buf.virt as *mut u32).wrapping_add((RING_TRBS - 1) * 4);
        unsafe {
            write_volatile(link, buf.phys as u32);
            write_volatile(link.add(1), (buf.phys >> 32) as u32);
            write_volatile(link.add(2), 0);
            write_volatile(link.add(3), (TRB_LINK << 10) | (1 << 1)); /* TC */
        }
        Ring {
            buf,
            enq: 0,
            cycle: 1,
        }
    }

    /// Enqueue one TRB; returns its physical address.
    fn push(&mut self, ptr: u64, status: u32, ctrl: u32) -> u64 {
        let t = (self.buf.virt as *mut u32).wrapping_add(self.enq * 4);
        let phys = self.buf.phys + (self.enq * 16) as u64;
        unsafe {
            write_volatile(t, ptr as u32);
            write_volatile(t.add(1), (ptr >> 32) as u32);
            write_volatile(t.add(2), status);
            fence(Ordering::SeqCst);
            write_volatile(t.add(3), (ctrl & !1) | self.cycle);
        }
        self.enq += 1;
        if self.enq == RING_TRBS - 1 {
            let link = (self.buf.virt as *mut u32).wrapping_add((RING_TRBS - 1) * 4);
            unsafe {
                fence(Ordering::SeqCst);
                write_volatile(link.add(3), (TRB_LINK << 10) | (1 << 1) | self.cycle);
            }
            self.cycle ^= 1;
            self.enq = 0;
        }
        phys
    }
}

/* —— Devices —— */

struct MscEp {
    bin: Ring,
    bout: Ring,
    dci_in: u8,
    dci_out: u8,
    io: DmaBuffer,
    dead: bool,
}

struct Dev {
    slot: u8,
    port: u32,
    speed: u32,
    ep0: Ring,
    in_ctx: DmaBuffer,
    msc: Option<MscEp>,
}

struct Xhci {
    bar: u64,
    op: u64,
    rt: u64,
    db: u64,
    ctx: usize,
    nports: u32,
    dcbaa: DmaBuffer,
    cmd: Ring,
    ev: DmaBuffer,
    ev_idx: usize,
    ev_cycle: u32,
    scratch: DmaBuffer,
    devs: Vec<Dev>,
    msc_devs: Vec<usize>,
    bufs: Vec<DmaBuffer>,
}

static mut HC: Option<Xhci> = None;

fn hc() -> Option<&'static mut Xhci> {
    unsafe { (*addr_of_mut!(HC)).as_mut() }
}

fn halt(op: u64) {
    let c = r32(op + OP_USBCMD);
    w32(op + OP_USBCMD, c & !CMD_RS);
    if !wait_bits(op + OP_USBSTS, STS_HCH, STS_HCH, 200) {
        /* Refuses to halt: hard-reset it so it cannot DMA into pages we free next. */
        w32(op + OP_USBCMD, CMD_HCRST);
        let _ = wait_bits(op + OP_USBCMD, CMD_HCRST, 0, 1000);
    }
}

fn legacy_handoff(bar: u64, bar_size: u64, hcc1: u32) {
    let mut off = ((hcc1 >> 16) & 0xFFFF) as u64 * 4;
    if off == 0 {
        return;
    }
    for _ in 0..32 {
        if off + 8 > bar_size {
            return;
        }
        let a = bar + off;
        let cap = r32(a);
        if cap & 0xFF == 1 && cap & (1 << 16) != 0 {
            w32(a, cap | (1 << 24));
            let _ = wait_bits(a, 1 << 16, 0, 1000);
        }
        let next = ((cap >> 8) & 0xFF) as u64;
        if next == 0 {
            return;
        }
        off += next * 4;
    }
}

impl Xhci {
    /* —— event ring —— */

    fn poll_event(&mut self) -> Option<[u32; 4]> {
        let t = (self.ev.virt + (self.ev_idx * 16) as u64) as *const u32;
        let d3 = unsafe { read_volatile(t.add(3)) };
        if d3 & 1 != self.ev_cycle {
            return None;
        }
        fence(Ordering::SeqCst);
        let e = unsafe {
            [
                read_volatile(t),
                read_volatile(t.add(1)),
                read_volatile(t.add(2)),
                d3,
            ]
        };
        self.ev_idx += 1;
        if self.ev_idx == RING_TRBS {
            self.ev_idx = 0;
            self.ev_cycle ^= 1;
        }
        let erdp = self.ev.phys + (self.ev_idx * 16) as u64;
        w64(self.rt + 0x20 + 0x18, erdp | (1 << 3)); /* EHB */
        Some(e)
    }

    /// Wait for an event of `ty` that points at `trb`. Returns (code, dw3).
    fn wait_event(&mut self, ty: u32, trb: u64, ms: u32) -> Result<(u32, u32), &'static str> {
        for _ in 0..(ms as u64 * 1000) {
            while let Some(e) = self.poll_event() {
                let t = (e[3] >> 10) & 0x3F;
                let ptr = (e[0] as u64) | ((e[1] as u64) << 32);
                let code = e[2] >> 24;
                if t == ty && ptr == trb {
                    return Ok((code, e[3]));
                }
                if t == TRB_EV_TRANSFER
                    && ty == TRB_EV_TRANSFER
                    && code != CC_SUCCESS
                    && code != CC_SHORT
                {
                    return Err("xhci transfer error");
                }
            }
            io_delay();
        }
        Err("xhci timeout")
    }

    fn wait_transfer(&mut self, trb: u64, ms: u32) -> Result<(), &'static str> {
        let (code, _) = self.wait_event(TRB_EV_TRANSFER, trb, ms)?;
        if code == CC_SUCCESS || code == CC_SHORT {
            Ok(())
        } else {
            Err("xhci transfer failed")
        }
    }

    /// Run one command; returns the slot id from the completion event.
    fn command(&mut self, ptr: u64, ctrl: u32) -> Result<u32, &'static str> {
        let trb = self.cmd.push(ptr, 0, ctrl);
        fence(Ordering::SeqCst);
        w32(self.db, 0);
        let (code, dw3) = self.wait_event(TRB_EV_CMD, trb, 1000)?;
        if code != CC_SUCCESS {
            return Err("xhci command failed");
        }
        Ok(dw3 >> 24)
    }

    /* —— contexts —— */

    fn ctx_w(&self, base: &DmaBuffer, idx: usize, dw: usize, v: u32) {
        let a = base.virt + (idx * self.ctx + dw * 4) as u64;
        unsafe { write_volatile(a as *mut u32, v) }
    }

    fn zero_page(b: &DmaBuffer) {
        unsafe { core::ptr::write_bytes(b.virt as *mut u8, 0, b.len) }
    }

    fn set_ep0_ctx(&self, inc: &DmaBuffer, mps: u32, ring_phys: u64) {
        self.ctx_w(inc, 2, 1, (3 << 1) | (4 << 3) | (mps << 16));
        self.ctx_w(inc, 2, 2, (ring_phys as u32 & !0xF) | 1);
        self.ctx_w(inc, 2, 3, (ring_phys >> 32) as u32);
        self.ctx_w(inc, 2, 4, 8);
    }

    /* —— control transfers (device → host or no data) —— */

    fn control(
        &mut self,
        di: usize,
        req_type: u8,
        req: u8,
        value: u16,
        index: u16,
        len: u16,
    ) -> Result<(), &'static str> {
        let slot = self.devs[di].slot as u64;
        let sp = self.scratch.phys;
        let has_data = len != 0;
        Self::zero_page(&self.scratch);
        let setup: u64 = (req_type as u64)
            | ((req as u64) << 8)
            | ((value as u64) << 16)
            | ((index as u64) << 32)
            | ((len as u64) << 48);
        let trt: u32 = if has_data { 3 } else { 0 };
        let status;
        {
            let ring = &mut self.devs[di].ep0;
            ring.push(setup, 8, (TRB_SETUP << 10) | (1 << 6) | (trt << 16));
            if has_data {
                ring.push(sp, len as u32, (TRB_DATA << 10) | (1 << 16));
            }
            let dir = if has_data { 0 } else { 1 << 16 };
            status = ring.push(0, 0, (TRB_STATUS << 10) | (1 << 5) | dir);
        }
        fence(Ordering::SeqCst);
        w32(self.db + 4 * slot, 1);
        self.wait_transfer(status, 1000)
    }

    fn scratch_bytes(&self, n: usize) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.scratch.virt as *const u8, n) }
    }

    /* —— enumeration —— */

    fn enumerate(&mut self, port: u32, speed: u32) -> Result<(), &'static str> {
        if self.devs.len() >= MAX_SLOTS as usize {
            return Err("xhci: no free slot");
        }
        let slot = self.command(0, TRB_ENABLE_SLOT << 10)?;
        if slot == 0 || slot > MAX_SLOTS {
            return Err("xhci: bad slot id");
        }
        let out_ctx = alloc(&mut self.bufs, 4096)?;
        let in_ctx = alloc(&mut self.bufs, 4096)?;
        let ep0 = Ring::new(alloc(&mut self.bufs, 4096)?);
        unsafe {
            write_volatile((self.dcbaa.virt as *mut u64).add(slot as usize), out_ctx.phys);
        }

        let mps0: u32 = match speed {
            1 | 2 => 8,
            3 => 64,
            _ => 512,
        };
        Self::zero_page(&in_ctx);
        self.ctx_w(&in_ctx, 0, 1, 0x3); /* A0 | A1 */
        self.ctx_w(&in_ctx, 1, 0, (speed << 20) | (1 << 27));
        self.ctx_w(&in_ctx, 1, 1, port << 16);
        self.set_ep0_ctx(&in_ctx, mps0, ep0.buf.phys);
        self.command(in_ctx.phys, (TRB_ADDRESS_DEV << 10) | (slot << 24))?;

        self.devs.push(Dev {
            slot: slot as u8,
            port,
            speed,
            ep0,
            in_ctx,
            msc: None,
        });
        let di = self.devs.len() - 1;

        /* Full/low speed: EP0 max packet may differ from our guess of 8. */
        if speed <= 2 {
            self.control(di, 0x80, 6, 0x0100, 0, 8)?;
            let m = self.scratch_bytes(8)[7] as u32;
            if m != 0 && m != mps0 && matches!(m, 8 | 16 | 32 | 64) {
                let inc = self.devs[di].in_ctx;
                Self::zero_page(&inc);
                self.ctx_w(&inc, 0, 1, 0x2); /* A1 */
                let ph = self.devs[di].ep0.buf.phys;
                self.set_ep0_ctx(&inc, m, ph);
                self.command(inc.phys, (TRB_EVAL_CTX << 10) | (slot << 24))?;
            }
        }

        /* Device descriptor (sanity). */
        self.control(di, 0x80, 6, 0x0100, 0, 18)?;
        if self.scratch_bytes(2)[1] != 1 {
            return Err("usb: bad device descriptor");
        }

        /* Configuration descriptor: header, then the whole thing. */
        self.control(di, 0x80, 6, 0x0200, 0, 9)?;
        let total = {
            let d = self.scratch_bytes(9);
            if d[1] != 2 {
                return Err("usb: bad config descriptor");
            }
            u16::from_le_bytes([d[2], d[3]])
        };
        if !(9..=2048).contains(&total) {
            return Err("usb: config descriptor size");
        }
        self.control(di, 0x80, 6, 0x0200, 0, total)?;

        let cfg_val;
        let mut ep_in: Option<(u8, u32)> = None; /* (ep number, mps) */
        let mut ep_out: Option<(u8, u32)> = None;
        {
            let d = self.scratch_bytes(total as usize);
            cfg_val = d[5];
            let mut i = d[0] as usize;
            let mut in_msc = false;
            let mut seen_msc = false;
            while i + 2 <= d.len() {
                let l = d[i] as usize;
                if l < 2 || i + l > d.len() {
                    break;
                }
                match d[i + 1] {
                    4 if l >= 9 => {
                        let is_msc = d[i + 3] == 0 && d[i + 5] == 0x08 && d[i + 6] == 0x06 && d[i + 7] == 0x50;
                        in_msc = is_msc && !seen_msc;
                        if in_msc {
                            seen_msc = true;
                        }
                    }
                    5 if l >= 7 && in_msc => {
                        let addr = d[i + 2];
                        let attr = d[i + 3];
                        let mps = (u16::from_le_bytes([d[i + 4], d[i + 5]]) & 0x7FF) as u32;
                        if attr & 3 == 2 && mps != 0 {
                            let num = addr & 0x0F;
                            if addr & 0x80 != 0 {
                                if ep_in.is_none() {
                                    ep_in = Some((num, mps));
                                }
                            } else if ep_out.is_none() {
                                ep_out = Some((num, mps));
                            }
                        }
                    }
                    _ => {}
                }
                i += l;
            }
        }
        let (Some((n_in, mps_in)), Some((n_out, mps_out))) = (ep_in, ep_out) else {
            return Ok(()); /* not a BOT mass-storage device */
        };
        if self.msc_devs.len() >= MAX_MSC {
            return Ok(());
        }
        if n_in == 0 || n_out == 0 {
            return Err("usb: bad bulk endpoint");
        }

        self.control(di, 0x00, 9, cfg_val as u16, 0, 0)?;

        let dci_in = n_in * 2 + 1;
        let dci_out = n_out * 2;
        let bin = Ring::new(alloc(&mut self.bufs, 4096)?);
        let bout = Ring::new(alloc(&mut self.bufs, 4096)?);
        let io = alloc(&mut self.bufs, 4096)?;

        let inc = self.devs[di].in_ctx;
        Self::zero_page(&inc);
        self.ctx_w(&inc, 0, 1, 1 | (1 << dci_in) | (1 << dci_out));
        let maxd = dci_in.max(dci_out) as u32;
        self.ctx_w(&inc, 1, 0, (speed << 20) | (maxd << 27));
        self.ctx_w(&inc, 1, 1, port << 16);
        for (dci, ty, mps, ring) in [
            (dci_in, 6u32, mps_in, &bin),
            (dci_out, 2u32, mps_out, &bout),
        ] {
            let idx = 1 + dci as usize;
            self.ctx_w(&inc, idx, 1, (3 << 1) | (ty << 3) | (mps << 16));
            self.ctx_w(&inc, idx, 2, (ring.buf.phys as u32 & !0xF) | 1);
            self.ctx_w(&inc, idx, 3, (ring.buf.phys >> 32) as u32);
            self.ctx_w(&inc, idx, 4, mps);
        }
        self.command(inc.phys, (TRB_CONFIG_EP << 10) | (slot << 24))?;

        self.devs[di].msc = Some(MscEp {
            bin,
            bout,
            dci_in,
            dci_out,
            io,
            dead: false,
        });
        self.msc_devs.push(di);
        Ok(())
    }

    fn scan_ports(&mut self) {
        delay_ms(100); /* let USB3 links train / devices re-announce after reset */
        for p in 0..self.nports {
            let reg = self.op + OP_PORTSC + 0x10 * p as u64;
            let mut v = r32(reg);
            if v & PORT_CCS == 0 {
                continue;
            }
            if v & PORT_PP == 0 {
                w32(reg, (v & PORT_KEEP) | PORT_PP);
                delay_ms(20);
                v = r32(reg);
                if v & PORT_CCS == 0 {
                    continue;
                }
            }
            if v & PORT_PED == 0 {
                w32(reg, (v & PORT_KEEP) | PORT_PR);
                if !wait_bits(reg, PORT_PRC, PORT_PRC, 500) {
                    continue;
                }
                v = r32(reg);
                w32(reg, (v & PORT_KEEP) | PORT_PRC);
                delay_ms(20); /* reset recovery */
                v = r32(reg);
                if v & (PORT_CCS | PORT_PED) != (PORT_CCS | PORT_PED) {
                    continue;
                }
            }
            let speed = (v >> 10) & 0xF;
            if speed == 0 {
                continue;
            }
            let _ = self.enumerate(p + 1, speed);
        }
    }

    fn bulk(&mut self, di: usize, out: bool, phys: u64, len: u32) -> Result<(), &'static str> {
        let slot;
        let dci;
        let trb;
        {
            let dev = self.devs.get_mut(di).ok_or("usb: bad device")?;
            slot = dev.slot as u64;
            let m = dev.msc.as_mut().ok_or("usb: not msc")?;
            if m.dead {
                return Err("usb: device dead");
            }
            let ctrl = (TRB_NORMAL << 10) | (1 << 5); /* IOC */
            if out {
                dci = m.dci_out;
                trb = m.bout.push(phys, len, ctrl);
            } else {
                dci = m.dci_in;
                trb = m.bin.push(phys, len, ctrl);
            }
        }
        fence(Ordering::SeqCst);
        w32(self.db + 4 * slot, dci as u32);
        let r = self.wait_transfer(trb, 3000);
        if r.is_err() {
            /* No endpoint reset / BOT recovery: refuse further I/O. */
            if let Some(m) = self.devs[di].msc.as_mut() {
                m.dead = true;
            }
        }
        r
    }
}

fn build(bar: u64, bar_size: u64, bufs: &mut Vec<DmaBuffer>) -> Result<Xhci, &'static str> {
    let w0 = r32(bar);
    if w0 == 0xFFFF_FFFF {
        return Err("xhci BAR reads all-ones");
    }
    let caplen = (w0 & 0xFF) as u64;
    let hcs1 = r32(bar + CAP_HCSPARAMS1);
    let hcs2 = r32(bar + CAP_HCSPARAMS2);
    let hcc1 = r32(bar + CAP_HCCPARAMS1);
    let max_slots = hcs1 & 0xFF;
    let nports = hcs1 >> 24;
    let op = bar + caplen;
    let db = bar + (r32(bar + CAP_DBOFF) & !3) as u64;
    let rt = bar + (r32(bar + CAP_RTSOFF) & !0x1F) as u64;
    let ctx = if hcc1 & (1 << 2) != 0 { 64 } else { 32 };
    if caplen < 0x20
        || db + 4 * (MAX_SLOTS as u64 + 1) > bar + bar_size
        || rt + 0x40 > bar + bar_size
        || op + OP_PORTSC + 0x10 * nports as u64 > bar + bar_size
    {
        return Err("xhci: registers outside BAR");
    }

    legacy_handoff(bar, bar_size, hcc1);

    /* Stop, then reset. */
    let c = r32(op + OP_USBCMD);
    w32(op + OP_USBCMD, c & !CMD_RS);
    if !wait_bits(op + OP_USBSTS, STS_HCH, STS_HCH, 200) {
        return Err("xhci won't halt");
    }
    w32(op + OP_USBCMD, CMD_HCRST);
    if !wait_bits(op + OP_USBCMD, CMD_HCRST, 0, 1000)
        || !wait_bits(op + OP_USBSTS, STS_CNR, 0, 1000)
    {
        return Err("xhci reset timeout");
    }
    if r32(op + OP_PAGESIZE) & 1 == 0 {
        return Err("xhci: 4K pages unsupported");
    }
    let slots = max_slots.min(MAX_SLOTS);
    if slots == 0 || nports == 0 {
        return Err("xhci: no slots/ports");
    }
    w32(op + OP_CONFIG, slots);

    /* DCBAA + scratchpads. */
    let dcbaa = alloc(bufs, 4096)?;
    let nscratch = ((hcs2 >> 27) & 0x1F) | (((hcs2 >> 21) & 0x1F) << 5);
    if nscratch > 0 {
        if nscratch > 512 {
            return Err("xhci: too many scratchpads");
        }
        let arr = alloc(bufs, 4096)?;
        for i in 0..nscratch as usize {
            let pg = alloc(bufs, 4096)?;
            unsafe { write_volatile((arr.virt as *mut u64).add(i), pg.phys) };
        }
        unsafe { write_volatile(dcbaa.virt as *mut u64, arr.phys) };
    }
    w64(op + OP_DCBAAP, dcbaa.phys);

    /* Command ring. */
    let cmd = Ring::new(alloc(bufs, 4096)?);
    w64(op + OP_CRCR, cmd.buf.phys | 1); /* RCS=1 */

    /* Event ring: one segment of 256 TRBs on interrupter 0. */
    let ev = alloc(bufs, 4096)?;
    let erst = alloc(bufs, 4096)?;
    unsafe {
        let e = erst.virt as *mut u32;
        write_volatile(e, ev.phys as u32);
        write_volatile(e.add(1), (ev.phys >> 32) as u32);
        write_volatile(e.add(2), RING_TRBS as u32);
        write_volatile(e.add(3), 0);
    }
    fence(Ordering::SeqCst);
    w32(rt + 0x20 + 0x08, 1); /* ERSTSZ */
    w64(rt + 0x20 + 0x18, ev.phys); /* ERDP */
    w64(rt + 0x20 + 0x10, erst.phys); /* ERSTBA (low then high) */

    let scratch = alloc(bufs, 4096)?;

    /* Run (INTE stays off: polled). */
    w32(op + OP_USBCMD, CMD_RS);
    if !wait_bits(op + OP_USBSTS, STS_HCH, 0, 200) {
        return Err("xhci won't run");
    }

    Ok(Xhci {
        bar,
        op,
        rt,
        db,
        ctx,
        nports,
        dcbaa,
        cmd,
        ev,
        ev_idx: 0,
        ev_cycle: 1,
        scratch,
        devs: Vec::new(),
        msc_devs: Vec::new(),
        bufs: core::mem::take(bufs),
    })
}

/* —— Public API —— */

/// Bring up the first xHCI controller and enumerate root-port devices.
/// Returns the number of Bulk-Only mass-storage devices found.
pub fn init() -> Result<usize, &'static str> {
    if hc().is_some() {
        return Err("xhci already initialised");
    }
    let addr = pci::find_class(0x0C, 0x03, 0x30).ok_or("no xhci controller")?;
    let (phys, size) =
        pci::bar_mem(addr.bus, addr.slot, addr.func, 0).ok_or("xhci BAR0 not mmio")?;
    if size < 0x1000 || size > (1 << 20) {
        return Err("xhci BAR0 bad size");
    }
    let bar = dma::map_mmio(phys, size)?;
    pci::enable_bus_master(addr.bus, addr.slot, addr.func);

    let mut bufs: Vec<DmaBuffer> = Vec::new();
    let mut x = match build(bar, size, &mut bufs) {
        Ok(x) => x,
        Err(e) => {
            let c = core::mem::take(&mut bufs);
            let op = bar + (r32(bar) & 0xFF) as u64;
            if (r32(bar) & 0xFF) >= 0x20 {
                halt(op);
            }
            for b in c {
                dma::dma_free(b);
            }
            return Err(e);
        }
    };
    x.scan_ports();
    let n = x.msc_devs.len();
    unsafe { *addr_of_mut!(HC) = Some(x) };
    if n == 0 {
        shutdown();
        return Err("no usb mass-storage device");
    }
    Ok(n)
}

/// Halt the controller and release all DMA memory.
pub fn shutdown() {
    let Some(x) = (unsafe { (*addr_of_mut!(HC)).take() }) else {
        return;
    };
    halt(x.op);
    for b in x.bufs {
        dma::dma_free(b);
    }
    let _ = (x.bar, x.nports, x.ctx, x.rt);
}

/// Per-device DMA page used for CBW/CSW/data: `(virt, phys)`.
pub fn msc_io_buf(i: usize) -> Option<(u64, u64)> {
    let x = hc()?;
    let di = *x.msc_devs.get(i)?;
    let m = x.devs[di].msc.as_ref()?;
    Some((m.io.virt, m.io.phys))
}

/// Synchronous bulk transfer of `len` bytes at `phys` (must not cross 64 KiB).
pub fn msc_bulk(i: usize, out: bool, phys: u64, len: u32) -> Result<(), &'static str> {
    let x = hc().ok_or("xhci not running")?;
    let di = *x.msc_devs.get(i).ok_or("usb: bad msc index")?;
    x.bulk(di, out, phys, len)
}

pub fn msc_alive(i: usize) -> bool {
    let Some(x) = hc() else { return false };
    let Some(&di) = x.msc_devs.get(i) else {
        return false;
    };
    x.devs[di].msc.as_ref().map_or(false, |m| !m.dead)
}
