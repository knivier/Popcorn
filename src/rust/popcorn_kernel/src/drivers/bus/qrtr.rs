//! QRTR (Qualcomm IPC Router) packets, as carried on MHI's IPCR channels. QMI
//! messages ride inside QRTR DATA packets. Pure `core` + `alloc`: host-tested.
//!
//! Wire (version 1, what Linux sends): a 32-byte little-endian header
//!   version=1, type, src node, src port, confirm_rx, size, dst node, dst port
//! then `size` payload bytes, padded with zeros to a multiple of 4. Version 2
//! (16-byte header) is accepted on receive.

use alloc::vec::Vec;

pub const HDR_V1: usize = 32;
pub const HDR_V2: usize = 16;

pub const TYPE_DATA: u32 = 1;
pub const TYPE_HELLO: u32 = 2;
pub const TYPE_BYE: u32 = 3;
pub const TYPE_NEW_SERVER: u32 = 4;
pub const TYPE_DEL_SERVER: u32 = 5;
pub const TYPE_DEL_CLIENT: u32 = 6;
pub const TYPE_RESUME_TX: u32 = 7;
pub const TYPE_EXIT: u32 = 8;
pub const TYPE_PING: u32 = 9;
pub const TYPE_NEW_LOOKUP: u32 = 10;
pub const TYPE_DEL_LOOKUP: u32 = 11;

pub const NODE_BCAST: u32 = 0xFFFF_FFFF;
pub const PORT_CTRL: u32 = 0xFFFF_FFFE;
/// The node id a host uses (Linux's default local node).
pub const HOST_NODE: u32 = 1;
/// First ephemeral port.
pub const FIRST_PORT: u32 = 0x4000;
/// Sender window per destination (Linux QRTR_TX_FLOW_HIGH / LOW).
pub const FLOW_HIGH: u32 = 10;
pub const FLOW_LOW: u32 = 5;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Hdr {
    pub kind: u32,
    pub src_node: u32,
    pub src_port: u32,
    pub confirm_rx: bool,
    pub size: u32,
    pub dst_node: u32,
    pub dst_port: u32,
}

pub fn type_name(t: u32) -> &'static str {
    match t {
        TYPE_DATA => "DATA",
        TYPE_HELLO => "HELLO",
        TYPE_BYE => "BYE",
        TYPE_NEW_SERVER => "NEW_SERVER",
        TYPE_DEL_SERVER => "DEL_SERVER",
        TYPE_DEL_CLIENT => "DEL_CLIENT",
        TYPE_RESUME_TX => "RESUME_TX",
        TYPE_EXIT => "EXIT",
        TYPE_PING => "PING",
        TYPE_NEW_LOOKUP => "NEW_LOOKUP",
        TYPE_DEL_LOOKUP => "DEL_LOOKUP",
        _ => "?",
    }
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

/// Version-1 packet: header + payload + zero padding to 4 bytes.
pub fn encode(h: &Hdr, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(HDR_V1 + payload.len() + 3);
    for w in [1u32, h.kind, h.src_node, h.src_port, h.confirm_rx as u32, payload.len() as u32, h.dst_node, h.dst_port] {
        v.extend_from_slice(&w.to_le_bytes());
    }
    v.extend_from_slice(payload);
    while v.len() % 4 != 0 {
        v.push(0);
    }
    v
}

/// Parse one packet (one MHI transfer). Checks the version, that the
/// declared size fits, and Linux's padding rule.
pub fn decode(buf: &[u8]) -> Result<(Hdr, &[u8]), &'static str> {
    if buf.is_empty() {
        return Err("empty QRTR packet");
    }
    match buf[0] {
        1 => {
            if buf.len() < HDR_V1 {
                return Err("short QRTR v1 header");
            }
            let h = Hdr {
                kind: le32(buf, 4),
                src_node: le32(buf, 8),
                src_port: le32(buf, 12),
                confirm_rx: le32(buf, 16) != 0,
                size: le32(buf, 20),
                dst_node: le32(buf, 24),
                dst_port: le32(buf, 28),
            };
            let end = HDR_V1.checked_add(h.size as usize).ok_or("QRTR size overflow")?;
            if end > buf.len() {
                return Err("QRTR payload longer than the packet");
            }
            Ok((h, &buf[HDR_V1..end]))
        }
        3 => {
            if buf.len() < HDR_V2 {
                return Err("short QRTR v2 header");
            }
            let optlen = buf[3] as usize;
            let node = |v: u16| if v == 0xFFFF { NODE_BCAST } else { v as u32 };
            let port = |v: u16| if v == 0xFFFE { PORT_CTRL } else { v as u32 };
            let h = Hdr {
                kind: buf[1] as u32,
                confirm_rx: buf[2] & 1 != 0,
                size: le32(buf, 4),
                src_node: node(le16(buf, 8)),
                src_port: port(le16(buf, 10)),
                dst_node: node(le16(buf, 12)),
                dst_port: port(le16(buf, 14)),
            };
            let start = HDR_V2 + optlen;
            let end = start.checked_add(h.size as usize).ok_or("QRTR size overflow")?;
            if end > buf.len() {
                return Err("QRTR payload longer than the packet");
            }
            Ok((h, &buf[start..end]))
        }
        _ => Err("unknown QRTR version"),
    }
}

/// Control packet payload (20 bytes): cmd, then four words (server: service,
/// instance, node, port; client: node, port).
pub fn ctrl(cmd: u32, w: [u32; 4]) -> [u8; 20] {
    let mut p = [0u8; 20];
    p[..4].copy_from_slice(&cmd.to_le_bytes());
    for (i, v) in w.iter().enumerate() {
        p[4 + 4 * i..8 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    p
}

pub fn parse_ctrl(p: &[u8]) -> Option<(u32, [u32; 4])> {
    if p.len() < 4 {
        return None;
    }
    let mut w = [0u32; 4];
    for (i, x) in w.iter_mut().enumerate() {
        let o = 4 + 4 * i;
        if o + 4 <= p.len() {
            *x = le32(p, o);
        }
    }
    Some((le32(p, 0), w))
}

/// QMI service instance as QRTR carries it: version in bits 7:0, instance above.
pub fn service_instance(version: u32, instance: u32) -> u32 {
    (version & 0xFF) | (instance << 8)
}
