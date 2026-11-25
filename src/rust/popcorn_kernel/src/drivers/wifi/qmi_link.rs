//! QMI over QRTR over MHI IPCR channels (20 host->device, 21 device->host):
//! the transport Qualcomm Wi-Fi firmware's control service (WLFW) is reached
//! through. Generic over any MHI host; the ath11k W2 flow drives it.
//!
//! Receive: 32 buffers of 8 KiB stay queued on channel 21; every completion is
//! one QRTR packet, handled and the buffer re-queued. Send: 4 buffers on
//! channel 20, reused when their completion arrives. Every wait is bounded.

use alloc::vec::Vec;

use crate::drivers::bus::mhi_host::{Config, Done, Host, Level, Say, CC_EOB, CC_EOT, CC_OVERFLOW};
use crate::drivers::bus::qrtr::{self, Hdr};
use crate::drivers::delay::poll_ms;
use crate::drivers::grant::{DmaArray, MmioRegion};

use super::qmi;

pub const CH_UL: u16 = 20;
pub const CH_DL: u16 = 21;
const RING: u16 = 64;
const BUF: usize = 8192;
const RX_BUFS: usize = 32;
const TX_BUFS: usize = 4;
const TAG_TX: u32 = 0x1000;

pub struct Link {
    rx: Vec<DmaArray<u8>>,
    tx: Vec<DmaArray<u8>>,
    tx_busy: Vec<bool>,
    pub remote_node: Option<u32>,
    /// WLFW service address (node, port) once announced.
    pub svc: Option<(u32, u32)>,
    port: u32,
    txn: u16,
    flow_pending: u32,
    responses: Vec<(u16, u16, Vec<u8>)>,
    inds: Vec<(u16, Vec<u8>)>,
    pub rx_packets: u32,
    pub tx_packets: u32,
}

fn copy_out(b: &DmaArray<u8>, len: usize) -> Vec<u8> {
    let mut v = alloc::vec![0u8; len];
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), v.as_mut_ptr(), len) };
    v
}

impl Link {
    /// Start both channels and fill the receive ring.
    pub fn open(host: &mut Host, bar: &MmioRegion, cfg: &Config, say: Say) -> Result<Link, &'static str> {
        host.assert_wake(bar);
        host.start_channel(bar, cfg, CH_UL, RING, say)?;
        host.start_channel(bar, cfg, CH_DL, RING, say)?;
        let mut rx = Vec::new();
        for i in 0..RX_BUFS {
            let b = host.pool().alloc_array::<u8>(BUF, 4096).map_err(|_| "out of DMA memory for QRTR buffers")?;
            host.queue(bar, CH_DL, b.dev().get(), BUF as u32, i as u32)?;
            rx.push(b);
        }
        let mut tx = Vec::new();
        for _ in 0..TX_BUFS {
            tx.push(host.pool().alloc_array::<u8>(BUF, 4096).map_err(|_| "out of DMA memory for QRTR buffers")?);
        }
        say(Level::Info, format_args!("QRTR: {} receive buffers of {} KiB queued on channel {}", RX_BUFS, BUF / 1024, CH_DL));
        Ok(Link {
            rx,
            tx,
            tx_busy: alloc::vec![false; TX_BUFS],
            remote_node: None,
            svc: None,
            port: qrtr::FIRST_PORT,
            txn: 0,
            flow_pending: 0,
            responses: Vec::new(),
            inds: Vec::new(),
            rx_packets: 0,
            tx_packets: 0,
        })
    }

    /// Service MHI once and handle whatever arrived.
    pub fn pump(&mut self, host: &mut Host, bar: &MmioRegion, say: Say) -> Result<(), &'static str> {
        let mut done: Vec<Done> = Vec::new();
        host.service(bar, &mut |d| done.push(d))?;
        for d in done {
            if d.chan == CH_UL {
                if let Some(b) = d.tag.checked_sub(TAG_TX).and_then(|i| self.tx_busy.get_mut(i as usize)) {
                    *b = false;
                }
            } else if d.chan == CH_DL {
                let i = d.tag as usize;
                if i >= self.rx.len() {
                    continue;
                }
                let pkt = if d.code == CC_EOT || d.code == CC_EOB {
                    Some(copy_out(&self.rx[i], (d.len as usize).min(BUF)))
                } else {
                    if d.code == CC_OVERFLOW {
                        say(Level::Warn, format_args!("QRTR: receive overflow ({} bytes), packet dropped", d.len));
                    }
                    None
                };
                host.queue(bar, CH_DL, self.rx[i].dev().get(), BUF as u32, d.tag)?;
                if let Some(p) = pkt {
                    self.rx_packets += 1;
                    self.handle(host, bar, &p, say)?;
                }
            }
        }
        Ok(())
    }

    fn handle(&mut self, host: &mut Host, bar: &MmioRegion, p: &[u8], say: Say) -> Result<(), &'static str> {
        let (h, payload) = match qrtr::decode(p) {
            Ok(x) => x,
            Err(why) => {
                say(Level::Warn, format_args!("QRTR: bad packet ({}, {} bytes)", why, p.len()));
                return Ok(());
            }
        };
        if h.kind != qrtr::TYPE_DATA {
            let (cmd, w) = qrtr::parse_ctrl(payload).unwrap_or((h.kind, [0; 4]));
            match h.kind {
                qrtr::TYPE_HELLO => {
                    self.remote_node = Some(h.src_node);
                    say(Level::Info, format_args!("QRTR: HELLO from firmware node {}", h.src_node));
                }
                qrtr::TYPE_NEW_SERVER => {
                    if w[0] == 0 && w[3] == 0 {
                        say(Level::Info, format_args!("QRTR: end of the firmware's service list"));
                    } else {
                        say(
                            Level::Info,
                            format_args!(
                                "QRTR: service {:#x} instance {:#x} (version {}, instance {}) at node {} port {:#x}",
                                w[0],
                                w[1],
                                w[1] & 0xFF,
                                w[1] >> 8,
                                w[2],
                                w[3]
                            ),
                        );
                        if w[0] == qmi::WLFW_SERVICE && w[1] == qrtr::service_instance(qmi::WLFW_VERSION, qmi::WLFW_INSTANCE) {
                            self.svc = Some((w[2], w[3]));
                        }
                    }
                }
                qrtr::TYPE_RESUME_TX => self.flow_pending = 0,
                _ => say(
                    Level::Info,
                    format_args!("QRTR: {} (cmd {}) from {}:{:#x}", qrtr::type_name(h.kind), cmd, h.src_node, h.src_port),
                ),
            }
            return Ok(());
        }
        if h.dst_port != self.port {
            say(Level::Warn, format_args!("QRTR: data for port {:#x} (ours is {:#x}) dropped", h.dst_port, self.port));
            return Ok(());
        }
        if h.confirm_rx {
            // The sender stops after its window until we confirm.
            let resume = qrtr::ctrl(qrtr::TYPE_RESUME_TX, [qrtr::HOST_NODE, self.port, 0, 0]);
            let hdr = Hdr {
                kind: qrtr::TYPE_RESUME_TX,
                src_node: qrtr::HOST_NODE,
                src_port: self.port,
                confirm_rx: false,
                size: 0,
                dst_node: h.src_node,
                dst_port: h.src_port,
            };
            self.send_raw(host, bar, &hdr, &resume, say)?;
        }
        match qmi::parse(payload) {
            Ok(m) if m.kind == qmi::RESPONSE => self.responses.push((m.txn, m.id, m.body.to_vec())),
            Ok(m) if m.kind == qmi::INDICATION => {
                say(Level::Info, format_args!("QMI: indication {} ({:#06x}, {} bytes)", qmi::msg_name(m.id), m.id, m.body.len()));
                self.inds.push((m.id, m.body.to_vec()));
            }
            Ok(m) => say(Level::Warn, format_args!("QMI: unexpected message type {} id {:#06x}", m.kind, m.id)),
            Err(why) => say(Level::Warn, format_args!("QMI: bad message ({})", why)),
        }
        Ok(())
    }

    fn send_raw(&mut self, host: &mut Host, bar: &MmioRegion, h: &Hdr, payload: &[u8], say: Say) -> Result<(), &'static str> {
        let pkt = qrtr::encode(h, payload);
        if pkt.len() > BUF {
            return Err("QRTR packet larger than a buffer");
        }
        let mut slot = self.tx_busy.iter().position(|b| !b);
        if slot.is_none() {
            let _ = poll_ms(2000, || {
                let _ = self.pump(host, bar, say);
                slot = self.tx_busy.iter().position(|b| !b);
                slot.is_some()
            });
        }
        let i = slot.ok_or("no free QRTR send buffer (channel 20 not completing)")?;
        unsafe { core::ptr::copy_nonoverlapping(pkt.as_ptr(), self.tx[i].as_ptr(), pkt.len()) };
        self.tx_busy[i] = true;
        host.queue(bar, CH_UL, self.tx[i].dev().get(), pkt.len() as u32, TAG_TX + i as u32)?;
        self.tx_packets += 1;
        Ok(())
    }

    /// Send our HELLO to the firmware's control port.
    pub fn hello(&mut self, host: &mut Host, bar: &MmioRegion, say: Say) -> Result<(), &'static str> {
        let h = Hdr {
            kind: qrtr::TYPE_HELLO,
            src_node: qrtr::HOST_NODE,
            src_port: qrtr::PORT_CTRL,
            confirm_rx: false,
            size: 0,
            dst_node: self.remote_node.unwrap_or(qrtr::NODE_BCAST),
            dst_port: qrtr::PORT_CTRL,
        };
        self.send_raw(host, bar, &h, &qrtr::ctrl(qrtr::TYPE_HELLO, [0; 4]), say)
    }

    /// Ask the firmware's name server for WLFW (fallback if it never announces it).
    pub fn lookup(&mut self, host: &mut Host, bar: &MmioRegion, say: Say) -> Result<(), &'static str> {
        let inst = qrtr::service_instance(qmi::WLFW_VERSION, qmi::WLFW_INSTANCE);
        let h = Hdr {
            kind: qrtr::TYPE_NEW_LOOKUP,
            src_node: qrtr::HOST_NODE,
            src_port: qrtr::PORT_CTRL,
            confirm_rx: false,
            size: 0,
            dst_node: self.remote_node.unwrap_or(qrtr::NODE_BCAST),
            dst_port: qrtr::PORT_CTRL,
        };
        self.send_raw(host, bar, &h, &qrtr::ctrl(qrtr::TYPE_NEW_LOOKUP, [qmi::WLFW_SERVICE, inst, 0, 0]), say)
    }

    /// Wait until `done(self)` holds, pumping meanwhile.
    pub fn wait(&mut self, host: &mut Host, bar: &MmioRegion, ms: u32, say: Say, mut done: impl FnMut(&Link) -> bool) -> Result<bool, &'static str> {
        let mut err = None;
        let r = poll_ms(ms, || {
            if let Err(e) = self.pump(host, bar, say) {
                err = Some(e);
                return true;
            }
            done(self)
        });
        if let Some(e) = err {
            return Err(e);
        }
        Ok(r.is_some())
    }

    pub fn next_txn(&mut self) -> u16 {
        self.txn = self.txn.wrapping_add(1);
        if self.txn == 0 {
            self.txn = 1;
        }
        self.txn
    }

    /// Send one QMI request (already encoded with `txn`) and wait for its
    /// response. Ok(body) only when the response's result is success.
    pub fn request(&mut self, host: &mut Host, bar: &MmioRegion, msg: Vec<u8>, ms: u32, say: Say) -> Result<Vec<u8>, &'static str> {
        let (txn, id) = match qmi::parse(&msg) {
            Ok(m) => (m.txn, m.id),
            Err(_) => return Err("bad QMI request"),
        };
        let (node, port) = self.svc.ok_or("WLFW service not found")?;
        if self.flow_pending >= qrtr::FLOW_HIGH {
            let ok = self.wait(host, bar, ms, say, |l| l.flow_pending < qrtr::FLOW_HIGH)?;
            if !ok {
                return Err("QRTR flow control: no RESUME_TX from the firmware");
            }
        }
        self.flow_pending += 1;
        let h = Hdr {
            kind: qrtr::TYPE_DATA,
            src_node: qrtr::HOST_NODE,
            src_port: self.port,
            confirm_rx: self.flow_pending == qrtr::FLOW_LOW,
            size: 0,
            dst_node: node,
            dst_port: port,
        };
        self.send_raw(host, bar, &h, &msg, say)?;
        let got = self.wait(host, bar, ms, say, |l| l.responses.iter().any(|r| r.0 == txn && r.1 == id))?;
        if !got {
            say(Level::Err, format_args!("QMI: no {} response in {} ms", qmi::msg_name(id), ms));
            return Err("QMI request timed out");
        }
        let at = self.responses.iter().position(|r| r.0 == txn && r.1 == id).unwrap_or(0);
        let (_, _, body) = self.responses.swap_remove(at);
        match qmi::result(&body) {
            Some((0, _)) => Ok(body),
            Some((r, e)) => {
                say(Level::Err, format_args!("QMI: {} failed (result {}, error {})", qmi::msg_name(id), r, e));
                Err("QMI request refused by the firmware")
            }
            None => Err("QMI response without a result"),
        }
    }

    /// Wait for indication `id` (or take it if it already arrived).
    pub fn wait_ind(&mut self, host: &mut Host, bar: &MmioRegion, id: u16, ms: u32, say: Say) -> Result<Vec<u8>, &'static str> {
        if !self.inds.iter().any(|i| i.0 == id) {
            let got = self.wait(host, bar, ms, say, |l| l.inds.iter().any(|i| i.0 == id))?;
            if !got {
                say(Level::Err, format_args!("QMI: no {} indication in {} ms", qmi::msg_name(id), ms));
                return Err("QMI indication never arrived");
            }
        }
        let at = self.inds.iter().position(|i| i.0 == id).unwrap_or(0);
        Ok(self.inds.remove(at).1)
    }

    pub fn has_ind(&self, id: u16) -> bool {
        self.inds.iter().any(|i| i.0 == id)
    }
}
