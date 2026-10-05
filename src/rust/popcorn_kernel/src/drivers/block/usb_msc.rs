//! USB Mass Storage, Bulk-Only Transport + SCSI transparent command set,
//! on top of the polled xHCI driver. 512-byte sectors, one sector per call.
//!
//! Layout of each device's 4 KiB DMA page: CBW @0, CSW @64, data @512.

use alloc::vec::Vec;
use core::ptr::addr_of_mut;

use crate::drivers::usb::xhci;

const SECTOR: usize = 512;
const CBW_SIG: u32 = 0x4342_5355; /* "USBC" */
const CSW_SIG: u32 = 0x5342_5355; /* "USBS" */
const OFF_CBW: u64 = 0;
const OFF_CSW: u64 = 64;
const OFF_DATA: u64 = 512;

struct Disk {
    /// Index of the device inside the xHCI MSC list.
    xi: usize,
    sectors: u64,
    tag: u32,
    /// Head of the disk is neither blank nor Popcorn-formatted (ESP / MBR / GPT /
    /// NTFS / ext4 ...): someone else's data, so the block layer locks it.
    boot_like: bool,
}

/// Sectors scanned (from LBA 0) to decide whether a stick is blank.
const CLASSIFY_SECTORS: u64 = 34;

static mut DISKS: Vec<Disk> = Vec::new();

fn disks() -> &'static mut Vec<Disk> {
    unsafe { &mut *addr_of_mut!(DISKS) }
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// One BOT command. Data (if any) lives at the device's data offset.
fn bot(xi: usize, tag: u32, cdb: &[u8], data_len: u32, data_in: bool) -> Result<(), &'static str> {
    let (virt, phys) = xhci::msc_io_buf(xi).ok_or("usb msc: no io buffer")?;
    if cdb.is_empty() || cdb.len() > 16 || data_len as usize > SECTOR {
        return Err("usb msc: bad command");
    }
    unsafe {
        let p = virt as *mut u8;
        core::ptr::write_bytes(p, 0, 128);
        (p as *mut u32).write_volatile(CBW_SIG);
        (p.add(4) as *mut u32).write_volatile(tag);
        (p.add(8) as *mut u32).write_volatile(data_len);
        p.add(12).write_volatile(if data_in { 0x80 } else { 0 });
        p.add(13).write_volatile(0); /* LUN 0 */
        p.add(14).write_volatile(cdb.len() as u8);
        core::ptr::copy_nonoverlapping(cdb.as_ptr(), p.add(15), cdb.len());
    }
    xhci::msc_bulk(xi, true, phys + OFF_CBW, 31)?;
    if data_len > 0 {
        xhci::msc_bulk(xi, !data_in, phys + OFF_DATA, data_len)?;
    }
    xhci::msc_bulk(xi, false, phys + OFF_CSW, 13)?;
    unsafe {
        let c = (virt + OFF_CSW) as *const u8;
        let sig = (c as *const u32).read_volatile();
        let rtag = (c.add(4) as *const u32).read_volatile();
        let residue = (c.add(8) as *const u32).read_volatile();
        let status = c.add(12).read_volatile();
        if sig != CSW_SIG || rtag != tag {
            return Err("usb msc: bad CSW");
        }
        match status {
            /* A short data stage would leave stale bytes in the buffer. */
            0 if data_len > 0 && residue != 0 => Err("usb msc: short transfer"),
            0 => Ok(()),
            1 => Err("usb msc: scsi command failed"),
            _ => Err("usb msc: phase error"),
        }
    }
}

fn data_ptr(xi: usize) -> Option<*mut u8> {
    xhci::msc_io_buf(xi).map(|(v, _)| (v + OFF_DATA) as *mut u8)
}

fn next_tag(d: &mut Disk) -> u32 {
    d.tag = d.tag.wrapping_add(1);
    d.tag
}

fn rw10(d: &mut Disk, write: bool, lba: u64) -> Result<(), &'static str> {
    if lba > u32::MAX as u64 {
        return Err("usb msc: lba too large");
    }
    let mut cdb = [0u8; 10];
    cdb[0] = if write { 0x2A } else { 0x28 };
    cdb[2..6].copy_from_slice(&(lba as u32).to_be_bytes());
    cdb[7..9].copy_from_slice(&1u16.to_be_bytes());
    let tag = next_tag(d);
    bot(d.xi, tag, &cdb, SECTOR as u32, !write)
}

fn init_disk(xi: usize) -> Result<Disk, &'static str> {
    let mut d = Disk {
        xi,
        sectors: 0,
        tag: 0x504F_0000,
        boot_like: false,
    };

    /* INQUIRY: some devices want it first; result is not needed. */
    let tag = next_tag(&mut d);
    let _ = bot(xi, tag, &[0x12, 0, 0, 0, 36, 0], 36, true);

    /* TEST UNIT READY, clearing UNIT ATTENTION via REQUEST SENSE. */
    let mut ready = false;
    for _ in 0..10 {
        let tag = next_tag(&mut d);
        match bot(xi, tag, &[0u8; 6], 0, false) {
            Ok(()) => {
                ready = true;
                break;
            }
            Err("usb msc: scsi command failed") => {
                let tag = next_tag(&mut d);
                let _ = bot(xi, tag, &[0x03, 0, 0, 0, 18, 0], 18, true);
                xhci::delay_ms(100);
            }
            Err(e) => return Err(e),
        }
    }
    if !ready {
        return Err("usb msc: not ready");
    }

    /* READ CAPACITY(10). */
    let tag = next_tag(&mut d);
    bot(xi, tag, &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true)?;
    let mut cap = [0u8; 8];
    unsafe { core::ptr::copy_nonoverlapping(data_ptr(xi).ok_or("usb msc: no buf")?, cap.as_mut_ptr(), 8) };
    let last = be32(&cap, 0);
    let bs = be32(&cap, 4);
    if bs as usize != SECTOR {
        return Err("usb msc: sector size != 512");
    }
    if last == u32::MAX {
        return Err("usb msc: capacity > 2 TiB");
    }
    d.sectors = last as u64 + 1;

    /* Classify: blank (all-zero head) or Popcorn-formatted FAT32 = ours;
     * anything else (boot stick, NTFS, ext4, GPT, ...) is foreign. */
    let mut s = [0u8; SECTOR];
    let mut blank = true;
    for lba in 0..CLASSIFY_SECTORS.min(d.sectors) {
        rw10(&mut d, false, lba)?;
        unsafe { core::ptr::copy_nonoverlapping(data_ptr(xi).ok_or("usb msc: no buf")?, s.as_mut_ptr(), SECTOR) };
        if lba == 0 {
            let ours = s[510] == 0x55 && s[511] == 0xAA && &s[3..11] == b"POPCORN ";
            if ours {
                blank = false;
                break;
            }
        }
        if s.iter().any(|&b| b != 0) {
            blank = false;
            d.boot_like = true;
            break;
        }
    }
    if blank {
        d.boot_like = false;
    }
    Ok(d)
}

/// Bring up xHCI and every Bulk-Only MSC device on it. Returns how many
/// became usable disks. `Err("no xhci controller")` means no USB hardware.
pub fn probe_all() -> Result<usize, &'static str> {
    let n = xhci::init()?;
    let t = disks();
    t.clear();
    let mut last_err = "usb msc: no usable LUN";
    for xi in 0..n {
        match init_disk(xi) {
            Ok(d) => t.push(d),
            Err(e) => last_err = e,
        }
    }
    if t.is_empty() {
        xhci::shutdown();
        return Err(last_err);
    }
    Ok(t.len())
}

/// `(sectors, sector_size)` of usable disk `idx`.
pub fn capacity(idx: usize) -> Option<(u64, u32)> {
    disks().get(idx).map(|d| (d.sectors, SECTOR as u32))
}

pub fn is_boot_like(idx: usize) -> bool {
    disks().get(idx).map_or(false, |d| d.boot_like)
}

pub fn read(idx: usize, lba: u64, buf: &mut [u8]) -> i64 {
    if buf.len() < SECTOR {
        return -2;
    }
    let Some(d) = disks().get_mut(idx) else {
        return -1;
    };
    if lba >= d.sectors {
        return -2;
    }
    if !xhci::msc_alive(d.xi) || rw10(d, false, lba).is_err() {
        return -1;
    }
    let Some(p) = data_ptr(d.xi) else { return -1 };
    unsafe { core::ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), SECTOR) };
    SECTOR as i64
}

pub fn write(idx: usize, lba: u64, buf: &[u8]) -> i64 {
    if buf.len() < SECTOR {
        return -2;
    }
    let Some(d) = disks().get_mut(idx) else {
        return -1;
    };
    if lba >= d.sectors {
        return -2;
    }
    if !xhci::msc_alive(d.xi) {
        return -1;
    }
    let Some(p) = data_ptr(d.xi) else { return -1 };
    unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), p, SECTOR) };
    if rw10(d, true, lba).is_err() {
        return -1;
    }
    SECTOR as i64
}
