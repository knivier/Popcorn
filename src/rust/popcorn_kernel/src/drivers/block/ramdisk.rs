//! In-RAM block disk `ram0` — always present, always writable after `disk use`.

use alloc::vec;
use alloc::vec::Vec;

use crate::sync::IrqSpinLock;

pub const SECTOR_SIZE: u32 = 512;
const SECTORS: u64 = 8192; // 4 MiB

/// IrqSpinLock: every access is a 512-byte memcpy (no I/O, no waits), so a
/// short interrupts-off section is cheaper than a yielding lock and also works
/// before the scheduler runs. The 4 MiB allocation happens outside the lock.
static DATA: IrqSpinLock<Option<Vec<u8>>> = IrqSpinLock::new(None);

pub fn init() -> Result<(u64, u32), &'static str> {
    if DATA.lock().is_none() {
        /* Leave blank — a POPCORN marker looked like "foreign data" to FAT. */
        let v = vec![0u8; (SECTORS as usize) * (SECTOR_SIZE as usize)];
        let spare = {
            let mut g = DATA.lock();
            if g.is_none() {
                *g = Some(v);
                None
            } else {
                Some(v) /* lost a race with another init: free ours unlocked */
            }
        };
        drop(spare);
    }
    Ok((SECTORS, SECTOR_SIZE))
}

/// Run `f` on the backing store (panics if `init` never ran, as before).
fn with_buf<R>(f: impl FnOnce(&mut [u8]) -> R) -> R {
    DATA.with(|d| f(d.as_mut().unwrap().as_mut_slice()))
}

pub fn read(lba: u64, out: &mut [u8]) -> i64 {
    if out.len() < SECTOR_SIZE as usize {
        return -2;
    }
    if lba >= SECTORS {
        return -2;
    }
    let off = (lba as usize) * (SECTOR_SIZE as usize);
    with_buf(|b| out[..SECTOR_SIZE as usize].copy_from_slice(&b[off..off + SECTOR_SIZE as usize]));
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
    with_buf(|b| b[off..off + SECTOR_SIZE as usize].copy_from_slice(&data[..SECTOR_SIZE as usize]));
    SECTOR_SIZE as i64
}
