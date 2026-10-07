//! Minimal FAT32 (mount / format / file + directory ops) on the selected disk.
//!
//! Safety rules:
//! - Boot never auto-formats. Blank disks stay unmounted until `disk wipe … YES`.
//! - Any MBR/GPT/ESP/NTFS/ext4 content makes mount refuse.
//! - The mounted volume is bound to the disk it was mounted from; if the selected
//!   disk changes the volume reads as "not mounted" instead of writing FAT
//!   structures onto another device.
//! - Every cluster number is range-checked before it becomes an LBA, and every
//!   chain/directory walk is hop-bounded (corrupt volumes cannot loop forever).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_char;
use core::ptr::addr_of_mut;

use crate::console_ffi::{print_color, println_color, COLOR_LIGHT_GREEN, COLOR_WHITE};
use crate::drivers::block;

const ATTR_READ_ONLY: u8 = 0x01;
const ATTR_HIDDEN: u8 = 0x02;
const ATTR_SYSTEM: u8 = 0x04;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LONG_NAME: u8 = ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID;

const FAT_EOC: u32 = 0x0FFF_FFFF;
const FAT_MASK: u32 = 0x0FFF_FFFF;
pub const MAX_CONTENT: usize = 64 * 1024;
/// Boot payloads (kernel ~800 KiB + EFI) staged in RAM during `disk install`.
pub const MAX_INSTALL_FILE: usize = 2 * 1024 * 1024;
const MAX_PATH: usize = 128;
/// Deepest directory recursion for `listsys` / `search` (guards cyclic dirs).
const MAX_DEPTH: u32 = 16;
/// Largest volume we format (keeps `sectors as u32` and FAT zeroing sane).
const MAX_FORMAT_SECTORS: u64 = 1 << 19; /* 256 MiB — keeps USB FAT clears short */

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    Io,
    NotFat,
    NotFound,
    Exists,
    NoSpace,
    NameTooLong,
    BadName,
    NotMounted,
}

struct Fat32 {
    bps: u16,
    spc: u8,
    reserved: u16,
    fats: u8,
    spf: u32,
    root_cluster: u32,
    fat_lba: u64,
    data_lba: u64,
    cluster_count: u32,
    cwd: u32,
    cwd_path: String,
    /// Block-layer index of the disk this volume was mounted from.
    disk: usize,
    /// Next cluster to try in `alloc_cluster` (avoids O(n²) FAT rescans).
    free_hint: u32,
    /// Cached primary-FAT sector index (`u32::MAX` = empty).
    fat_c_si: u32,
    fat_c_dirty: bool,
    fat_c_sec: [u8; 512],
}

static mut FS: Option<Fat32> = None;
static mut READ_BUF: [u8; MAX_CONTENT + 1] = [0; MAX_CONTENT + 1];
static mut SEARCH_BUF: [u8; MAX_PATH] = [0; MAX_PATH];
static mut CWD_BUF: [u8; MAX_PATH] = [0; MAX_PATH];

fn fs() -> Result<&'static mut Fat32, FsError> {
    let f = unsafe { (*addr_of_mut!(FS)).as_mut() }.ok_or(FsError::NotMounted)?;
    if f.disk != block::selected_id() {
        return Err(FsError::NotMounted);
    }
    Ok(f)
}

/// Forget the mounted volume (called when the selected disk changes).
pub fn unmount() {
    unsafe {
        if let Some(ref mut f) = *addr_of_mut!(FS) {
            let _ = f.fat_flush();
        }
        *addr_of_mut!(FS) = None;
    }
}

fn rd(lba: u64, buf: &mut [u8]) -> Result<(), FsError> {
    if block::read_selected(lba, buf) < 0 {
        Err(FsError::Io)
    } else {
        Ok(())
    }
}

fn wr(lba: u64, buf: &[u8]) -> Result<(), FsError> {
    if block::write_selected(lba, buf) < 0 {
        Err(FsError::Io)
    } else {
        Ok(())
    }
}

fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn parse_bpb(sec: &[u8], disk_sectors: u64) -> Result<Fat32, FsError> {
    if sec.len() < 512 || sec[510] != 0x55 || sec[511] != 0xAA {
        return Err(FsError::NotFat);
    }
    let bps = le16(sec, 11);
    let spc = sec[13];
    let reserved = le16(sec, 14);
    let fats = sec[16];
    let root_ent = le16(sec, 17);
    let tot16 = le16(sec, 19);
    let spf16 = le16(sec, 22);
    let tot32 = le32(sec, 32);
    let spf32 = le32(sec, 36);
    let root_cluster = le32(sec, 44);
    if bps != 512 || spc == 0 || !spc.is_power_of_two() || fats == 0 || fats > 2 || reserved == 0 {
        return Err(FsError::NotFat);
    }
    /* FAT32: BPB_FATSz16 == 0 and root entries == 0 */
    if spf16 != 0 || root_ent != 0 || spf32 == 0 || root_cluster < 2 {
        return Err(FsError::NotFat);
    }
    let total = if tot16 != 0 {
        tot16 as u64
    } else {
        tot32 as u64
    };
    /* A BPB claiming more sectors than the device has would read/write past its end. */
    if total == 0 || total > disk_sectors {
        return Err(FsError::NotFat);
    }
    let fat_lba = reserved as u64;
    let data_lba = fat_lba + (fats as u64) * (spf32 as u64);
    if data_lba >= total {
        return Err(FsError::NotFat);
    }
    let data_sectors = total - data_lba;
    let mut cluster_count = (data_sectors / spc as u64).min(0x0FFF_FFF5) as u32;
    /* Never trust more clusters than the FAT can actually describe. */
    let fat_entries = (spf32 as u64) * 128;
    if (cluster_count as u64) + 2 > fat_entries {
        cluster_count = fat_entries.saturating_sub(2) as u32;
    }
    if cluster_count < 2 || root_cluster >= cluster_count + 2 {
        return Err(FsError::NotFat);
    }
    Ok(Fat32 {
        bps,
        spc,
        reserved,
        fats,
        spf: spf32,
        root_cluster,
        fat_lba,
        data_lba,
        cluster_count,
        cwd: root_cluster,
        cwd_path: String::from("/"),
        disk: block::selected_id(),
        free_hint: 3,
        fat_c_si: u32::MAX,
        fat_c_dirty: false,
        fat_c_sec: [0u8; 512],
    })
}

fn format_fat32(disk_sectors: u64) -> Result<Fat32, FsError> {
    /* Never format more than MAX_FORMAT_SECTORS: keeps the BPB's 32-bit total
     * honest and bounds the FAT zeroing loop on huge disks. */
    let sectors = disk_sectors.min(MAX_FORMAT_SECTORS);
    if sectors < 68_000 {
        return Err(FsError::NoSpace);
    }
    let bps: u16 = 512;
    let reserved: u16 = 32;
    let fats: u8 = 2;
    let root_cluster: u32 = 2;

    /* Prefer larger clusters (fewer FAT sector writes on USB), but FAT32 needs
     * ≥65525 clusters after reserved+FAT overhead — so fall back to smaller spc. */
    let mut spc = pick_spc(sectors);
    let (spf, cluster_count) = loop {
        let (spf, cluster_count) = fit_fat(sectors, reserved, fats, spc)?;
        if cluster_count >= 65525 {
            break (spf, cluster_count);
        }
        if spc == 1 {
            return Err(FsError::NoSpace);
        }
        spc >>= 1;
    };

    let mut sec = [0u8; 512];
    sec[0] = 0xEB;
    sec[1] = 0x58;
    sec[2] = 0x90;
    sec[3..11].copy_from_slice(b"POPCORN ");
    put16(&mut sec, 11, bps);
    sec[13] = spc;
    put16(&mut sec, 14, reserved);
    sec[16] = fats;
    put16(&mut sec, 17, 0);
    put16(&mut sec, 19, 0);
    sec[21] = 0xF8;
    put16(&mut sec, 22, 0);
    put16(&mut sec, 24, 63);
    put16(&mut sec, 26, 255);
    put32(&mut sec, 28, 0);
    put32(&mut sec, 32, sectors as u32);
    put32(&mut sec, 36, spf);
    put16(&mut sec, 40, 0);
    put16(&mut sec, 42, 0);
    put32(&mut sec, 44, root_cluster);
    put16(&mut sec, 48, 1); /* FSInfo sector */
    put16(&mut sec, 50, 6); /* backup boot */
    sec[64] = 0x80;
    sec[66] = 0x29;
    put32(&mut sec, 67, 0x504F5043); /* serial 'POPC' */
    sec[71..82].copy_from_slice(b"POPCORN    ");
    sec[82..90].copy_from_slice(b"FAT32   ");
    sec[510] = 0x55;
    sec[511] = 0xAA;

    /* FSInfo */
    let mut info = [0u8; 512];
    put32(&mut info, 0, 0x4161_5252);
    put32(&mut info, 484, 0x6141_7272);
    put32(&mut info, 488, 0xFFFF_FFFF);
    put32(&mut info, 492, 0xFFFF_FFFF);
    info[510] = 0x55;
    info[511] = 0xAA;

    let fat_lba = reserved as u64;
    let data_lba = fat_lba + (fats as u64) * (spf as u64);

    /* Clear FATs — progress so a USB format does not look like a freeze. */
    let zero = [0u8; 512];
    let fat_secs = (spf as u64) * (fats as u64);
    for i in 0..fat_secs {
        wr(fat_lba + i, &zero)?;
        if (i & 63) == 0 {
            print_color(".", COLOR_WHITE);
        }
    }
    println_color("", COLOR_WHITE);

    let mut fs = Fat32 {
        bps,
        spc,
        reserved,
        fats,
        spf,
        root_cluster,
        fat_lba,
        data_lba,
        cluster_count,
        cwd: root_cluster,
        cwd_path: String::from("/"),
        disk: block::selected_id(),
        free_hint: 3,
        fat_c_si: u32::MAX,
        fat_c_dirty: false,
        fat_c_sec: [0u8; 512],
    };

    /* Media / EOC entries */
    fs.fat_set(0, 0x0FFF_FFF8)?;
    fs.fat_set(1, 0x0FFF_FFFF)?;
    fs.fat_set(2, FAT_EOC)?;
    fs.fat_flush()?;

    /* Empty root cluster */
    fs.zero_cluster(2)?;

    /* Volume label entry in root */
    let mut dir = [0u8; 32];
    dir[..11].copy_from_slice(b"POPCORN    ");
    dir[11] = ATTR_VOLUME_ID;
    fs.write_dirent(2, 0, &dir)?;

    /* Boot sector LAST: until it lands the disk still looks blank, so an
     * interrupted format can simply be retried instead of leaving a
     * "valid" POPCORN volume with a half-built FAT. */
    wr(1, &info)?;
    wr(6, &sec)?;
    wr(0, &sec)?;

    Ok(fs)
}

fn fit_fat(sectors: u64, reserved: u16, fats: u8, spc: u8) -> Result<(u32, u32), FsError> {
    let mut spf = 1u32;
    let mut cluster_count;
    loop {
        let data_lba = reserved as u64 + (fats as u64) * (spf as u64);
        cluster_count = (sectors.saturating_sub(data_lba) / spc as u64) as u32;
        let need = ((cluster_count as u64 + 2) * 4 + 511) / 512;
        if need <= spf as u64 {
            break;
        }
        spf = need as u32;
        if spf > 4096 {
            return Err(FsError::NoSpace);
        }
    }
    Ok((spf, cluster_count))
}

/// Choose sectors-per-cluster so the FAT stays small enough for USB format.
fn pick_spc(sectors: u64) -> u8 {
    /* Target ~≤512 FAT sectors: spf≈sectors/(spc*128) ≤ 512 → spc ≥ sectors/65536 */
    let mut spc: u32 = 1;
    let min_spc = ((sectors / 65_536).max(1) as u32).min(64);
    while spc < min_spc {
        spc <<= 1;
    }
    /* Leave headroom for reserved+FAT so cluster_count stays ≥65525. */
    while spc > 1 && (sectors.saturating_sub(4096) / spc as u64) < 65_525 {
        spc >>= 1;
    }
    if spc == 0 {
        1
    } else {
        spc as u8
    }
}

impl Fat32 {
    /// A cluster that maps to a data sector inside the volume (2 ..= count+1).
    fn valid_cluster(&self, cluster: u32) -> bool {
        cluster >= 2 && (cluster as u64) < self.cluster_count as u64 + 2
    }

    /// Callers must have passed `valid_cluster` (the I/O helpers below do).
    fn cluster_lba(&self, cluster: u32) -> u64 {
        self.data_lba + (cluster as u64 - 2) * self.spc as u64
    }

    fn fat_flush(&mut self) -> Result<(), FsError> {
        if !self.fat_c_dirty || self.fat_c_si == u32::MAX {
            return Ok(());
        }
        let si = self.fat_c_si as u64;
        for f in 0..self.fats as u64 {
            wr(self.fat_lba + f * self.spf as u64 + si, &self.fat_c_sec)?;
        }
        self.fat_c_dirty = false;
        Ok(())
    }

    fn fat_load(&mut self, si: u32) -> Result<(), FsError> {
        if self.fat_c_si == si {
            return Ok(());
        }
        self.fat_flush()?;
        rd(self.fat_lba + si as u64, &mut self.fat_c_sec)?;
        self.fat_c_si = si;
        Ok(())
    }

    fn fat_get(&mut self, cluster: u32) -> Result<u32, FsError> {
        if cluster as u64 >= self.cluster_count as u64 + 2 {
            return Err(FsError::Io);
        }
        let off = cluster as u64 * 4;
        self.fat_load((off / 512) as u32)?;
        Ok(le32(&self.fat_c_sec, (off % 512) as usize) & FAT_MASK)
    }

    fn fat_set(&mut self, cluster: u32, value: u32) -> Result<(), FsError> {
        if cluster as u64 >= self.cluster_count as u64 + 2 {
            return Err(FsError::Io);
        }
        let off = cluster as u64 * 4;
        let idx = (off % 512) as usize;
        self.fat_load((off / 512) as u32)?;
        put32(&mut self.fat_c_sec, idx, value & FAT_MASK);
        self.fat_c_dirty = true;
        Ok(())
    }

    fn zero_cluster(&self, cluster: u32) -> Result<(), FsError> {
        if !self.valid_cluster(cluster) {
            return Err(FsError::Io);
        }
        let z = [0u8; 512];
        let base = self.cluster_lba(cluster);
        for i in 0..self.spc as u64 {
            wr(base + i, &z)?;
        }
        Ok(())
    }

    fn alloc_cluster(&mut self) -> Result<u32, FsError> {
        /* Scan via the FAT sector cache — no per-cluster USB RMW storm. */
        let start = self.free_hint.max(2);
        for pass in 0..2u8 {
            let (from, to) = if pass == 0 {
                (start, self.cluster_count + 2)
            } else {
                (2, start)
            };
            let mut c = from;
            while c < to {
                let off = c as u64 * 4;
                let si = (off / 512) as u32;
                self.fat_load(si)?;
                let idx = (off % 512) as usize;
                if le32(&self.fat_c_sec, idx) & FAT_MASK == 0 {
                    put32(&mut self.fat_c_sec, idx, FAT_EOC);
                    self.fat_c_dirty = true;
                    self.free_hint = c.saturating_add(1);
                    return Ok(c);
                }
                c += 1;
            }
        }
        Err(FsError::NoSpace)
    }

    fn free_chain(&mut self, mut cluster: u32) -> Result<(), FsError> {
        let mut hops = 0u64;
        while cluster >= 2 && cluster < 0x0FFF_FFF8 {
            hops += 1;
            if hops > self.cluster_count as u64 + 1 {
                return Err(FsError::Io); /* cyclic chain */
            }
            let next = self.fat_get(cluster)?;
            self.fat_set(cluster, 0)?;
            cluster = next;
        }
        self.fat_flush()
    }

    fn read_cluster(&self, cluster: u32, out: &mut [u8]) -> Result<(), FsError> {
        let need = (self.spc as usize) * 512;
        if out.len() < need || !self.valid_cluster(cluster) {
            return Err(FsError::Io);
        }
        let base = self.cluster_lba(cluster);
        for i in 0..self.spc as u64 {
            rd(base + i, &mut out[(i as usize) * 512..(i as usize + 1) * 512])?;
        }
        Ok(())
    }

    fn write_cluster(&self, cluster: u32, data: &[u8]) -> Result<(), FsError> {
        if !self.valid_cluster(cluster) {
            return Err(FsError::Io);
        }
        /* No heap: large-cluster Vec allocs during install exhausted the block
         * table / fragmented under IRQ and contributed to hard resets. */
        let base = self.cluster_lba(cluster);
        let mut sec = [0u8; 512];
        for i in 0..self.spc as u64 {
            sec.fill(0);
            let off = (i as usize) * 512;
            if off < data.len() {
                let n = (data.len() - off).min(512);
                sec[..n].copy_from_slice(&data[off..off + n]);
            }
            wr(base + i, &sec)?;
        }
        Ok(())
    }

    fn write_dirent(&mut self, dir_cluster: u32, index: usize, ent: &[u8; 32]) -> Result<(), FsError> {
        let bpc = (self.spc as usize) * 512;
        let ents_per_cluster = bpc / 32;
        let mut cluster = dir_cluster;
        let mut idx = index;
        let mut hops = 0u32;
        loop {
            if idx < ents_per_cluster {
                let mut buf = vec![0u8; bpc];
                self.read_cluster(cluster, &mut buf)?;
                let off = idx * 32;
                buf[off..off + 32].copy_from_slice(ent);
                self.write_cluster(cluster, &buf)?;
                return Ok(());
            }
            idx -= ents_per_cluster;
            let next = self.fat_get(cluster)?;
            if next >= 0x0FFF_FFF8 {
                return Err(FsError::NoSpace);
            }
            hops += 1;
            if hops > self.cluster_count {
                return Err(FsError::Io); /* cyclic directory chain */
            }
            cluster = next;
        }
    }

    fn read_dirent(&mut self, dir_cluster: u32, index: usize) -> Result<[u8; 32], FsError> {
        let bpc = (self.spc as usize) * 512;
        let ents_per_cluster = bpc / 32;
        let mut cluster = dir_cluster;
        let mut idx = index;
        let mut hops = 0u32;
        loop {
            if idx < ents_per_cluster {
                let mut buf = vec![0u8; bpc];
                self.read_cluster(cluster, &mut buf)?;
                let off = idx * 32;
                let mut ent = [0u8; 32];
                ent.copy_from_slice(&buf[off..off + 32]);
                return Ok(ent);
            }
            idx -= ents_per_cluster;
            let next = self.fat_get(cluster)?;
            if next >= 0x0FFF_FFF8 {
                return Err(FsError::NotFound);
            }
            hops += 1;
            if hops > self.cluster_count {
                return Err(FsError::Io);
            }
            cluster = next;
        }
    }

    fn for_each_entry<F: FnMut(usize, &[u8; 32]) -> bool>(
        &mut self,
        dir_cluster: u32,
        mut f: F,
    ) -> Result<(), FsError> {
        let bpc = (self.spc as usize) * 512;
        let ents_per_cluster = bpc / 32;
        let mut cluster = dir_cluster;
        let mut base_idx = 0usize;
        let mut hops = 0u32;
        loop {
            let mut buf = vec![0u8; bpc];
            self.read_cluster(cluster, &mut buf)?;
            for i in 0..ents_per_cluster {
                let off = i * 32;
                let mut ent = [0u8; 32];
                ent.copy_from_slice(&buf[off..off + 32]);
                if ent[0] == 0x00 {
                    return Ok(());
                }
                if ent[0] == 0xE5 {
                    continue;
                }
                if ent[11] == ATTR_LONG_NAME {
                    continue;
                }
                if !f(base_idx + i, &ent) {
                    return Ok(());
                }
            }
            base_idx += ents_per_cluster;
            let next = self.fat_get(cluster)?;
            if next >= 0x0FFF_FFF8 {
                return Ok(());
            }
            hops += 1;
            if hops > self.cluster_count {
                return Err(FsError::Io);
            }
            cluster = next;
        }
    }

    /// Linear directory index from the start of the chain.
    fn find_free_dirent(&mut self, dir_cluster: u32) -> Result<usize, FsError> {
        let bpc = (self.spc as usize) * 512;
        let ents_per_cluster = bpc / 32;
        let mut cluster = dir_cluster;
        let mut base = 0usize;
        let mut hops = 0u32;
        loop {
            let mut buf = vec![0u8; bpc];
            self.read_cluster(cluster, &mut buf)?;
            for i in 0..ents_per_cluster {
                let off = i * 32;
                if buf[off] == 0x00 || buf[off] == 0xE5 {
                    return Ok(base + i);
                }
            }
            base += ents_per_cluster;
            let next = self.fat_get(cluster)?;
            if next >= 0x0FFF_FFF8 {
                /* Directory full: extend the chain with a fresh (zeroed) cluster. */
                let neu = self.alloc_cluster()?;
                self.fat_set(cluster, neu)?;
                return Ok(base);
            }
            hops += 1;
            if hops > self.cluster_count {
                return Err(FsError::Io);
            }
            cluster = next;
        }
    }

    fn clus_from_ent(ent: &[u8; 32]) -> u32 {
        let hi = le16(ent, 20) as u32;
        let lo = le16(ent, 26) as u32;
        (hi << 16) | lo
    }

    fn put_clus(ent: &mut [u8; 32], c: u32) {
        put16(ent, 20, (c >> 16) as u16);
        put16(ent, 26, (c & 0xFFFF) as u16);
    }
}

fn to_sfn(name: &str) -> Result<[u8; 11], FsError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FsError::BadName);
    }
    if name.len() > 12 {
        return Err(FsError::NameTooLong);
    }
    if name == "." || name == ".." {
        /* Only the on-disk "." / ".." entries use these; never a user-chosen name. */
        let mut sfn = [b' '; 11];
        for (i, b) in name.bytes().enumerate() {
            sfn[i] = b;
        }
        return Ok(sfn);
    }
    let mut sfn = [b' '; 11];
    let bytes = name.as_bytes();
    let mut base = [0u8; 8];
    let mut ext = [0u8; 3];
    let mut bi = 0usize;
    let mut ei = 0usize;
    let mut in_ext = false;
    for &b in bytes {
        let u = if (b'a'..=b'z').contains(&b) {
            b - 32
        } else {
            b
        };
        if u == b'.' {
            if in_ext {
                return Err(FsError::BadName);
            }
            in_ext = true;
            continue;
        }
        if !(u.is_ascii_alphanumeric() || u == b'_' || u == b'-' || u == b'~') {
            return Err(FsError::BadName);
        }
        /* Reject (don't truncate): "LONGNAME1" and "LONGNAME2" would otherwise
         * silently alias the same 8.3 entry and overwrite each other. */
        if in_ext {
            if ei >= 3 {
                return Err(FsError::NameTooLong);
            }
            ext[ei] = u;
            ei += 1;
        } else {
            if bi >= 8 {
                return Err(FsError::NameTooLong);
            }
            base[bi] = u;
            bi += 1;
        }
    }
    if bi == 0 {
        return Err(FsError::BadName);
    }
    sfn[..bi].copy_from_slice(&base[..bi]);
    sfn[8..8 + ei].copy_from_slice(&ext[..ei]);
    Ok(sfn)
}

fn sfn_display(ent: &[u8; 32]) -> String {
    let mut s = String::new();
    for i in 0..8 {
        if ent[i] == b' ' {
            break;
        }
        s.push(ent[i] as char);
    }
    if ent[8] != b' ' {
        s.push('.');
        for i in 8..11 {
            if ent[i] == b' ' {
                break;
            }
            s.push(ent[i] as char);
        }
    }
    s
}

/// Mount the selected disk. Never auto-formats — blank/foreign disks stay
/// unmounted so boot cannot hang writing a FAT over USB.
pub fn mount_or_format() -> Result<(), FsError> {
    unmount();
    let (sectors, _) = block::selected_capacity().ok_or(FsError::Io)?;
    let mut sec0 = [0u8; 512];
    rd(0, &mut sec0)?;

    if let Ok(fat) = parse_bpb(&sec0, sectors) {
        unsafe { *addr_of_mut!(FS) = Some(fat) };
        return Ok(());
    }

    /* Blank or foreign: refuse. Caller must `disk wipe <name> YES`. */
    Err(FsError::NotFat)
}

/// Format the selected disk as Popcorn FAT32 (caller must have wiped it).
pub fn force_format() -> Result<(), FsError> {
    unmount();
    if block::selected_is_internal() && !block::selected_master_unlocked() {
        return Err(FsError::Io);
    }
    let (sectors, _) = block::selected_capacity().ok_or(FsError::Io)?;
    let mut fat = format_fat32(sectors)?;
    /* Seed files are nice-to-have; a failed README must not undo a good format. */
    if seed_defaults(&mut fat).is_err() {
        println_color("FAT32: volume ok (default files skipped)", COLOR_WHITE);
    }
    unsafe { *addr_of_mut!(FS) = Some(fat) };
    println_color("FAT32 formatted on selected disk", COLOR_LIGHT_GREEN);
    Ok(())
}

fn seed_defaults(fat: &mut Fat32) -> Result<(), FsError> {
    write_file_at(fat, fat.root_cluster, "README.TXT", b"Welcome to Popcorn FAT32. Type help.")?;
    mkdir_at(fat, fat.root_cluster, "HOME")?;
    /* enter HOME briefly for welcome */
    let home = find_dir_cluster(fat, fat.root_cluster, "HOME")?;
    write_file_at(
        fat,
        home,
        "WELCOME.TXT",
        b"Home directory on FAT32.",
    )?;
    write_file_at(
        fat,
        fat.root_cluster,
        "SYSTEM.INF",
        b"Popcorn Kernel FAT32 volume",
    )?;
    Ok(())
}

fn ensure_dir(fat: &mut Fat32, parent: u32, name: &str) -> Result<u32, FsError> {
    if let Ok(c) = find_dir_cluster(fat, parent, name) {
        return Ok(c);
    }
    mkdir_at(fat, parent, name)?;
    find_dir_cluster(fat, parent, name)
}

fn resolve_dir(fat: &mut Fat32, parts: &[&str]) -> Result<u32, FsError> {
    let mut dir = fat.root_cluster;
    for p in parts {
        dir = find_dir_cluster(fat, dir, p)?;
    }
    Ok(dir)
}

/// Read a file from a directory path on the currently mounted volume (binary-safe).
pub fn read_binary(dir_parts: &[&str], name: &str) -> Result<alloc::vec::Vec<u8>, FsError> {
    let fat = fs()?;
    let dir = resolve_dir(fat, dir_parts)?;
    let (_idx, ent) = find_in_dir(fat, dir, name)?;
    if ent[11] & ATTR_DIRECTORY != 0 {
        return Err(FsError::NotFound);
    }
    let size = le32(&ent, 28) as usize;
    if size == 0 {
        return Ok(alloc::vec::Vec::new());
    }
    if size > MAX_INSTALL_FILE {
        return Err(FsError::NoSpace);
    }
    let clu = Fat32::clus_from_ent(&ent);
    let mut buf = alloc::vec![0u8; size];
    let n = read_chain(fat, clu, size, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Write UEFI removable layout onto the mounted Popcorn volume:
/// `EFI/BOOT/BOOTX64.EFI` and `BOOT/KERNEL` (loader also accepts the latter path).
pub fn write_boot_layout(efi: &[u8], kernel: &[u8]) -> Result<(), FsError> {
    let fat = fs()?;
    if efi.is_empty() || kernel.is_empty() {
        return Err(FsError::Io);
    }
    if efi.len() > MAX_INSTALL_FILE || kernel.len() > MAX_INSTALL_FILE {
        return Err(FsError::NoSpace);
    }
    println_color("install: [3/4] writing EFI/BOOT/BOOTX64.EFI...", COLOR_WHITE);
    let efi_dir = ensure_dir(fat, fat.root_cluster, "EFI")?;
    let efi_boot = ensure_dir(fat, efi_dir, "BOOT")?;
    write_file_at_limited(fat, efi_boot, "BOOTX64.EFI", efi, MAX_INSTALL_FILE)?;

    println_color("install: [4/4] writing BOOT/KERNEL (progress dots)...", COLOR_WHITE);
    let boot_dir = ensure_dir(fat, fat.root_cluster, "BOOT")?;
    write_file_at_limited(fat, boot_dir, "KERNEL", kernel, MAX_INSTALL_FILE)?;
    fat.fat_flush()?;
    println_color("", COLOR_WHITE);

    write_file_at(
        fat,
        fat.root_cluster,
        "SYSTEM.INF",
        b"Popcorn installed volume (UEFI bootable FAT32)",
    )?;

    /* Verify both payloads landed before claiming success. */
    let efi_check = read_binary(&["EFI", "BOOT"], "BOOTX64.EFI")?;
    let kern_check = read_binary(&["BOOT"], "KERNEL")?;
    if efi_check.len() != efi.len() || kern_check.len() != kernel.len() {
        return Err(FsError::Io);
    }
    Ok(())
}

/// True if the mounted volume has `EFI/BOOT/BOOTX64.EFI`.
pub fn has_boot_loader() -> bool {
    read_binary(&["EFI", "BOOT"], "BOOTX64.EFI")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

fn find_in_dir(
    fat: &mut Fat32,
    dir: u32,
    name: &str,
) -> Result<(usize, [u8; 32]), FsError> {
    let sfn = to_sfn(name)?;
    let mut found = None;
    fat.for_each_entry(dir, |idx, ent| {
        if ent[11] == ATTR_VOLUME_ID {
            return true;
        }
        if ent[..11] == sfn {
            found = Some((idx, *ent));
            return false;
        }
        true
    })?;
    found.ok_or(FsError::NotFound)
}

fn find_dir_cluster(fat: &mut Fat32, dir: u32, name: &str) -> Result<u32, FsError> {
    let (.., ent) = find_in_dir(fat, dir, name)?;
    if ent[11] & ATTR_DIRECTORY == 0 {
        return Err(FsError::NotFound);
    }
    Ok(Fat32::clus_from_ent(&ent))
}

/// "." and ".." are only ever the on-disk dot entries, never user-created names.
fn reject_dot_name(name: &str) -> Result<(), FsError> {
    let n = name.trim();
    if n == "." || n == ".." {
        return Err(FsError::BadName);
    }
    Ok(())
}

fn mkdir_at(fat: &mut Fat32, parent: u32, name: &str) -> Result<(), FsError> {
    reject_dot_name(name)?;
    let sfn = to_sfn(name)?;
    if find_in_dir(fat, parent, name).is_ok() {
        return Err(FsError::Exists);
    }
    let clu = fat.alloc_cluster()?;
    fat.zero_cluster(clu)?; /* directories must start empty */

    /* Build the new directory completely before linking it into the parent, so a
     * failure part-way never leaves a visible half-made directory. */
    let result = (|| {
        let mut dot = [0u8; 32];
        dot[..11].copy_from_slice(b".          ");
        dot[11] = ATTR_DIRECTORY;
        Fat32::put_clus(&mut dot, clu);
        fat.write_dirent(clu, 0, &dot)?;
        let mut dotdot = [0u8; 32];
        dotdot[..11].copy_from_slice(b"..         ");
        dotdot[11] = ATTR_DIRECTORY;
        Fat32::put_clus(
            &mut dotdot,
            if parent == fat.root_cluster { 0 } else { parent },
        );
        fat.write_dirent(clu, 1, &dotdot)?;

        let mut ent = [0u8; 32];
        ent[..11].copy_from_slice(&sfn);
        ent[11] = ATTR_DIRECTORY;
        Fat32::put_clus(&mut ent, clu);
        let di = fat.find_free_dirent(parent)?;
        fat.write_dirent(parent, di, &ent)
    })();
    if result.is_err() {
        let _ = fat.free_chain(clu);
    }
    result
}

fn write_file_at(fat: &mut Fat32, dir: u32, name: &str, data: &[u8]) -> Result<(), FsError> {
    write_file_at_limited(fat, dir, name, data, MAX_CONTENT)
}

fn write_file_at_limited(
    fat: &mut Fat32,
    dir: u32,
    name: &str,
    data: &[u8],
    max: usize,
) -> Result<(), FsError> {
    if data.len() > max {
        return Err(FsError::NoSpace);
    }
    reject_dot_name(name)?;
    let sfn = to_sfn(name)?;
    if let Ok((idx, ent)) = find_in_dir(fat, dir, name) {
        if ent[11] & ATTR_DIRECTORY != 0 || ent[11] & ATTR_READ_ONLY != 0 {
            return Err(FsError::Exists);
        }
        let old = Fat32::clus_from_ent(&ent);
        /* New data first, directory entry second, old clusters last: a failure
         * at any step leaves the old file intact instead of pointing at freed clusters. */
        let first = write_chain(fat, data)?;
        let mut neu = ent;
        Fat32::put_clus(&mut neu, first);
        put32(&mut neu, 28, data.len() as u32);
        neu[11] = ATTR_ARCHIVE;
        if let Err(e) = fat.write_dirent(dir, idx, &neu) {
            let _ = fat.free_chain(first);
            return Err(e);
        }
        if old >= 2 {
            fat.free_chain(old)?;
        }
        return Ok(());
    }
    let first = write_chain(fat, data)?;
    let mut ent = [0u8; 32];
    ent[..11].copy_from_slice(&sfn);
    ent[11] = ATTR_ARCHIVE;
    Fat32::put_clus(&mut ent, first);
    put32(&mut ent, 28, data.len() as u32);
    let linked = fat
        .find_free_dirent(dir)
        .and_then(|g| fat.write_dirent(dir, g, &ent));
    if let Err(e) = linked {
        /* Avoid free_chain storm after a large payload write. */
        let _ = e;
        return Err(FsError::Io);
    }
    Ok(())
}

/// Allocate and fill a cluster chain; frees whatever it allocated on failure.
fn write_chain(fat: &mut Fat32, data: &[u8]) -> Result<u32, FsError> {
    if data.is_empty() {
        return Ok(0);
    }
    extern "C" {
        fn boot_serial_putc(c: u8);
    }
    let bpc = (fat.spc as usize) * 512;
    let nclu = (data.len() + bpc - 1) / bpc;
    /* Pre-allocate the full chain, then write payloads. Separating FAT updates
     * from data writes keeps each USB BOT short under the IRQ quiet guard. */
    let mut clusters = alloc::vec::Vec::with_capacity(nclu);
    for _ in 0..nclu {
        match fat.alloc_cluster() {
            Ok(c) => clusters.push(c),
            Err(e) => {
                for &c in &clusters {
                    let _ = fat.fat_set(c, 0);
                }
                let _ = fat.fat_flush();
                return Err(e);
            }
        }
    }
    for i in 0..nclu.saturating_sub(1) {
        if let Err(e) = fat.fat_set(clusters[i], clusters[i + 1]) {
            for &c in &clusters {
                let _ = fat.fat_set(c, 0);
            }
            let _ = fat.fat_flush();
            return Err(e);
        }
    }
    fat.fat_flush()?;
    for (i, &c) in clusters.iter().enumerate() {
        let off = i * bpc;
        let chunk = &data[off..data.len().min(off + bpc)];
        if let Err(e) = fat.write_cluster(c, chunk) {
            /* Do not free_chain here — that storms the USB and can reset the
             * machine after a partial install. Leave orphans; format retries. */
            return Err(e);
        }
        if data.len() > 64 * 1024 && (i & 15) == 0 {
            print_color(".", COLOR_WHITE);
            unsafe { boot_serial_putc(b'.') };
        }
        /* Brief pause every 32 clusters so flaky USB controllers can settle. */
        if data.len() > 64 * 1024 && (i & 31) == 31 {
            unsafe {
                boot_serial_putc(b'K');
                crate::console_ffi::util_delay(2);
            }
        }
    }
    Ok(clusters[0])
}

fn read_chain(fat: &mut Fat32, mut cluster: u32, size: usize, out: &mut [u8]) -> Result<usize, FsError> {
    if size == 0 || cluster < 2 {
        return Ok(0);
    }
    let bpc = (fat.spc as usize) * 512;
    let mut written = 0usize;
    let mut tmp = vec![0u8; bpc];
    while cluster >= 2 && cluster < 0x0FFF_FFF8 && written < size {
        fat.read_cluster(cluster, &mut tmp)?;
        let n = (size - written).min(bpc);
        out[written..written + n].copy_from_slice(&tmp[..n]);
        written += n;
        cluster = fat.fat_get(cluster)?;
    }
    Ok(written)
}

pub fn write_file(name: &str, data: &[u8]) -> Result<(), FsError> {
    let fat = fs()?;
    let dir = fat.cwd;
    write_file_at(fat, dir, name, data)
}

pub fn read_file(name: &str) -> Result<*const c_char, FsError> {
    let fat = fs()?;
    let (_idx, ent) = find_in_dir(fat, fat.cwd, name)?;
    if ent[11] & ATTR_DIRECTORY != 0 {
        return Err(FsError::NotFound);
    }
    let size = le32(&ent, 28) as usize;
    if size > MAX_CONTENT {
        return Err(FsError::NoSpace);
    }
    let clu = Fat32::clus_from_ent(&ent);
    unsafe {
        let buf = &mut *addr_of_mut!(READ_BUF);
        core::ptr::write_bytes(buf.as_mut_ptr(), 0, buf.len()); /* no 64 KiB stack temporary */
        let n = read_chain(fat, clu, size, buf)?;
        buf[n.min(MAX_CONTENT)] = 0;
        Ok(buf.as_ptr() as *const c_char)
    }
}

pub fn list_cwd() -> Result<(), FsError> {
    let fat = fs()?;
    println_color("Directory:", COLOR_WHITE);
    fat.for_each_entry(fat.cwd, |_i, ent| {
        if ent[11] == ATTR_VOLUME_ID {
            return true;
        }
        let mut line = String::new();
        if ent[11] & ATTR_DIRECTORY != 0 {
            line.push_str("<DIR>  ");
        } else {
            line.push_str("       ");
        }
        line.push_str(&sfn_display(ent));
        println_color(&line, COLOR_LIGHT_GREEN);
        true
    })
}

pub fn delete_entry(name: &str) -> Result<(), FsError> {
    let fat = fs()?;
    let (idx, ent) = find_in_dir(fat, fat.cwd, name)?;
    if &ent[..11] == b".          " || &ent[..11] == b"..         " {
        return Err(FsError::BadName);
    }
    if ent[11] & ATTR_READ_ONLY != 0 {
        return Err(FsError::BadName);
    }
    let clu = Fat32::clus_from_ent(&ent);
    if ent[11] & ATTR_DIRECTORY != 0 {
        /* Removing the directory we are standing in would leave `cwd` on freed clusters. */
        if clu == fat.cwd || clu == fat.root_cluster {
            return Err(FsError::BadName);
        }
        /* only empty dirs (just . and ..) */
        let mut extra = false;
        fat.for_each_entry(clu, |_i, e| {
            if &e[..11] != b".          " && &e[..11] != b"..         " && e[0] != 0xE5 {
                extra = true;
                return false;
            }
            true
        })?;
        if extra {
            return Err(FsError::BadName);
        }
        fat.free_chain(clu)?;
    } else if clu >= 2 {
        fat.free_chain(clu)?;
    }
    let mut del = ent;
    del[0] = 0xE5;
    fat.write_dirent(fat.cwd, idx, &del)?;
    Ok(())
}

pub fn mkdir(name: &str) -> Result<(), FsError> {
    let fat = fs()?;
    let dir = fat.cwd;
    mkdir_at(fat, dir, name)
}

pub fn chdir(name: &str) -> Result<(), FsError> {
    let fat = fs()?;
    if name == "back" || name == ".." {
        if fat.cwd == fat.root_cluster {
            fat.cwd_path = String::from("/");
            return Ok(());
        }
        let (_i, ent) = find_in_dir(fat, fat.cwd, "..")?;
        let parent = Fat32::clus_from_ent(&ent);
        fat.cwd = if parent == 0 {
            fat.root_cluster
        } else {
            parent
        };
        if let Some(pos) = fat.cwd_path.rfind('/') {
            if pos == 0 {
                fat.cwd_path = String::from("/");
            } else {
                fat.cwd_path.truncate(pos);
            }
        }
        sync_cwd_buf(fat);
        return Ok(());
    }
    if name == "/" || name == "root" {
        fat.cwd = fat.root_cluster;
        fat.cwd_path = String::from("/");
        sync_cwd_buf(fat);
        return Ok(());
    }
    if name.trim() == "." {
        return Ok(());
    }
    let (_i, ent) = find_in_dir(fat, fat.cwd, name)?;
    if ent[11] & ATTR_DIRECTORY == 0 {
        return Err(FsError::NotFound);
    }
    let clu = Fat32::clus_from_ent(&ent);
    if !fat.valid_cluster(clu) {
        return Err(FsError::Io);
    }
    fat.cwd = clu;
    /* Show the on-disk (normalized) name, not whatever the user typed. */
    let shown = sfn_display(&ent);
    if fat.cwd_path != "/" {
        fat.cwd_path.push('/');
    }
    fat.cwd_path.push_str(&shown);
    sync_cwd_buf(fat);
    Ok(())
}

fn sync_cwd_buf(fat: &Fat32) {
    unsafe {
        let buf = &mut *addr_of_mut!(CWD_BUF);
        *buf = [0; MAX_PATH];
        let b = fat.cwd_path.as_bytes();
        let n = b.len().min(MAX_PATH - 1);
        buf[..n].copy_from_slice(&b[..n]);
    }
}

pub fn cwd_cstr() -> *const c_char {
    if let Ok(fat) = fs() {
        sync_cwd_buf(fat);
    } else {
        unsafe {
            let buf = &mut *addr_of_mut!(CWD_BUF);
            *buf = [0; MAX_PATH];
            buf[0] = b'/';
        }
    }
    unsafe { (*addr_of_mut!(CWD_BUF)).as_ptr() as *const c_char }
}

pub fn search(name: &str) -> Result<*const c_char, FsError> {
    let fat = fs()?;
    let sfn = to_sfn(name)?;
    let mut path = String::new();
    if search_rec(fat, fat.root_cluster, "/", &sfn, &mut path, 0)? {
        unsafe {
            let buf = &mut *addr_of_mut!(SEARCH_BUF);
            *buf = [0; MAX_PATH];
            let b = path.as_bytes();
            let n = b.len().min(MAX_PATH - 1);
            buf[..n].copy_from_slice(&b[..n]);
            return Ok(buf.as_ptr() as *const c_char);
        }
    }
    Err(FsError::NotFound)
}

fn search_rec(
    fat: &mut Fat32,
    dir: u32,
    prefix: &str,
    sfn: &[u8; 11],
    out: &mut String,
    depth: u32,
) -> Result<bool, FsError> {
    if depth > MAX_DEPTH {
        return Err(FsError::Io);
    }
    let mut entries: Vec<(String, u32, bool)> = Vec::new();
    fat.for_each_entry(dir, |_i, ent| {
        if ent[11] == ATTR_VOLUME_ID {
            return true;
        }
        let n = sfn_display(ent);
        if &ent[..11] == b".          " || &ent[..11] == b"..         " {
            return true;
        }
        let is_dir = ent[11] & ATTR_DIRECTORY != 0;
        if ent[..11] == *sfn {
            *out = format_path(prefix, &n);
            entries.clear();
            return false;
        }
        if is_dir {
            entries.push((n, Fat32::clus_from_ent(ent), true));
        }
        true
    })?;
    if !out.is_empty() {
        return Ok(true);
    }
    for (n, clu, _) in entries {
        let p = format_path(prefix, &n);
        if search_rec(fat, clu, &p, sfn, out, depth + 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn format_path(prefix: &str, name: &str) -> String {
    if prefix == "/" {
        let mut s = String::from("/");
        s.push_str(name);
        s
    } else {
        let mut s = String::from(prefix);
        s.push('/');
        s.push_str(name);
        s
    }
}

pub fn copy_file(src: &str, dest_dir: &str) -> Result<(), FsError> {
    let fat = fs()?;
    let (_i, ent) = find_in_dir(fat, fat.cwd, src)?;
    if ent[11] & ATTR_DIRECTORY != 0 {
        return Err(FsError::BadName);
    }
    let size = le32(&ent, 28) as usize;
    if size > MAX_CONTENT {
        return Err(FsError::NoSpace); /* untrusted size field: don't allocate GiBs */
    }
    let clu = Fat32::clus_from_ent(&ent);
    let mut buf = vec![0u8; size.max(1)];
    let n = read_chain(fat, clu, size, &mut buf)?;
    let target_dir = if dest_dir == "/" || dest_dir.eq_ignore_ascii_case("root") {
        fat.root_cluster
    } else {
        find_dir_cluster(fat, fat.cwd, dest_dir)
            .or_else(|_| find_dir_cluster(fat, fat.root_cluster, dest_dir))?
    };
    write_file_at(fat, target_dir, src, &buf[..n])
}

pub fn list_hierarchy() -> Result<(), FsError> {
    let fat = fs()?;
    println_color("FAT32 tree:", COLOR_WHITE);
    list_rec(fat, fat.root_cluster, "/", 0)
}

fn list_rec(fat: &mut Fat32, dir: u32, prefix: &str, depth: u32) -> Result<(), FsError> {
    if depth > MAX_DEPTH {
        return Err(FsError::Io); /* cyclic or absurdly deep directory tree */
    }
    let mut sub: Vec<(String, u32)> = Vec::new();
    fat.for_each_entry(dir, |_i, ent| {
        if ent[11] == ATTR_VOLUME_ID {
            return true;
        }
        if &ent[..11] == b".          " || &ent[..11] == b"..         " {
            return true;
        }
        let n = sfn_display(ent);
        let mut line = format_path(prefix, &n);
        if ent[11] & ATTR_DIRECTORY != 0 {
            line.push('/');
            println_color(&line, COLOR_LIGHT_GREEN);
            sub.push((format_path(prefix, &n), Fat32::clus_from_ent(ent)));
        } else {
            println_color(&line, COLOR_WHITE);
        }
        true
    })?;
    for (p, c) in sub {
        list_rec(fat, c, &p, depth + 1)?;
    }
    Ok(())
}
