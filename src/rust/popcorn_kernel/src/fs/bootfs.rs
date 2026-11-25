//! Read-only files on the USB stick Popcorn booted from (firmware). Uses the
//! strict reader in `fs::fat_ro` over single-sector reads of the boot disk;
//! the boot stick stays closed to writes (only POPCORN.LOG is ever written).

use alloc::vec::Vec;

use super::fat_ro::Volume;
use crate::drivers::block;
use crate::klog::fatlog::SECTOR;

/// Read `path` (absolute, upper-case 8.3) into memory. Refuses files above
/// `max` bytes. `progress(done, total)` is called after every cluster.
pub fn read(path: &str, max: usize, progress: &mut dyn FnMut(usize, usize)) -> Result<Vec<u8>, &'static str> {
    let (idx, sectors) = block::boot_disk_for_read()?;
    let mut rd = |lba: u64, buf: &mut [u8; SECTOR]| block::log_read(idx, lba, buf) == SECTOR as i64;
    let mut vol = Volume::open(&mut rd, sectors)?;
    let f = vol.lookup(&mut rd, path)?;
    let size = f.size as usize;
    if size > max {
        return Err("file larger than allowed");
    }
    let mut data = Vec::new();
    data.try_reserve_exact(size).map_err(|_| "out of memory for the file")?;
    data.resize(size, 0);
    vol.read_file(&mut rd, f, &mut data, &mut |done| progress(done, size))?;
    Ok(data)
}
