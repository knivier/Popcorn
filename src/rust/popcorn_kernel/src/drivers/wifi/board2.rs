//! `board-2.bin`: the container holding per-board calibration data for every
//! laptop model using the chip. Pure `core` + `alloc`: host-tested against the
//! real file.
//!
//! "QCA-ATH11K-BOARD\0" padded to 20 bytes, then IEs: id u32 LE, len u32 LE,
//! data padded to 4. Top-level IE 0 = board, 1 = regulatory database; inside
//! one, sub-IE 0 = a name it matches, 1 = its data. Names compare exactly.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const MAGIC: &[u8] = b"QCA-ATH11K-BOARD\0";
pub const IE_BOARD: u32 = 0;
pub const IE_REGDB: u32 = 1;
const SUB_NAME: u32 = 0;
const SUB_DATA: u32 = 1;

fn ie(b: &[u8], o: usize) -> Option<(u32, &[u8], usize)> {
    let h = b.get(o..o + 8)?;
    let id = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
    let len = u32::from_le_bytes([h[4], h[5], h[6], h[7]]) as usize;
    let data = b.get(o + 8..o.checked_add(8)?.checked_add(len)?)?;
    let next = o + 8 + ((len + 3) & !3);
    Some((id, data, next))
}

/// The data of the `kind` IE whose name list contains `name` exactly.
pub fn find<'a>(file: &'a [u8], kind: u32, name: &str) -> Result<&'a [u8], &'static str> {
    if !file.starts_with(MAGIC) {
        return Err("not a board-2.bin (bad magic)");
    }
    let mut o = (MAGIC.len() + 3) & !3;
    let mut guard = 0;
    while o < file.len() && guard < 10_000 {
        guard += 1;
        let Some((id, body, next)) = ie(file, o) else { return Err("board-2.bin truncated") };
        if id == kind {
            let mut s = 0;
            let mut matched = false;
            let mut sg = 0;
            while s < body.len() && sg < 10_000 {
                sg += 1;
                let Some((sid, sb, snext)) = ie(body, s) else { break };
                if sid == SUB_NAME && sb == name.as_bytes() {
                    matched = true;
                } else if sid == SUB_DATA && matched {
                    return Ok(sb);
                }
                s = snext;
            }
        }
        o = next;
    }
    Err("no entry with that name")
}

/// Board names ath11k tries for a PCI card, best first (no SMBIOS variant):
/// full name, then the chip-id-only name.
pub fn board_names(vendor: u16, device: u16, sub_vendor: u16, sub_device: u16, chip_id: u32, board_id: u32) -> [String; 2] {
    [
        format!(
            "bus=pci,vendor={:04x},device={:04x},subsystem-vendor={:04x},subsystem-device={:04x},qmi-chip-id={},qmi-board-id={}",
            vendor, device, sub_vendor, sub_device, chip_id, board_id
        ),
        format!("bus=pci,qmi-chip-id={}", chip_id),
    ]
}

/// Regulatory database names ath11k tries for PCI: full name, then "bus=pci".
pub fn regdb_names(full: &str) -> [String; 2] {
    [String::from(full), String::from("bus=pci")]
}

/// Every name in IEs of `kind` (diagnostics).
pub fn names(file: &[u8], kind: u32) -> Vec<String> {
    let mut out = Vec::new();
    if !file.starts_with(MAGIC) {
        return out;
    }
    let mut o = (MAGIC.len() + 3) & !3;
    while let Some((id, body, next)) = ie(file, o) {
        if id == kind {
            let mut s = 0;
            while let Some((sid, sb, snext)) = ie(body, s) {
                if sid == SUB_NAME {
                    out.push(sb.iter().map(|&c| c as char).collect());
                }
                s = snext;
            }
        }
        o = next;
    }
    out
}
