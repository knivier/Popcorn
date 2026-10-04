//! In-RAM block disk `ram0` — always present, always writable after `disk use`.

use alloc::vec;
use alloc::vec::Vec;

pub const SECTOR_SIZE: u32 = 512;
const SECTORS: u64 = 8192; // 4 MiB

static mut DATA: Option<Vec<u8>> = None;

pub fn init() -> Result<(u64, u32), &'static str> {
    unsafe {
        if DATA.is_none() {
            let mut v = vec![0u8; (SECTORS as usize) * (SECTOR_SIZE as usize)];
            // Marker so reads aren't all-zero before first write.
            if let Some(b) = v.get_mut(0..8) {
                b.copy_from_slice(b"POPCORN\0");
            }
            DATA = Some(v);
        }
    }
    Ok((SECTORS, SECTOR_SIZE))
}

fn buf() -> &'static mut [u8] {
    unsafe { DATA.as_mut().unwrap().as_mut_slice() }
}

pub fn read(lba: u64, out: &mut [u8]) -> i64 {
    if out.len() < SECTOR_SIZE as usize {
        return -2;
    }
    if lba >= SECTORS {
        return -2;
    }
    let off = (lba as usize) * (SECTOR_SIZE as usize);
    out[..SECTOR_SIZE as usize].copy_from_slice(&buf()[off..off + SECTOR_SIZE as usize]);
    SECTOR_SIZE as i64
}

pub fn write(lba: u64, data: &[u8]) -> i64 {
    if data.len() < SECTOR_SIZE as usize {
        return -2;
    }
    if lba >= SECTORS {
        return -2;
    }
    let off = (lba as usize) * (SECTOR_SIZE as usize);
    buf()[off..off + SECTOR_SIZE as usize].copy_from_slice(&data[..SECTOR_SIZE as usize]);
    SECTOR_SIZE as i64
}
