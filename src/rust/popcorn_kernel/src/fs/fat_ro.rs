//! Read-only FAT32 file access for the boot stick (a whole-disk FAT32 volume,
//! BPB at LBA 0), used to load Wi-Fi firmware. Pure `core`: the kernel passes a
//! sector reader, the host tests pass real mkfs.vfat/mtools images.
//!
//! Strict on purpose, because what it reads ends up in a device: every cluster
//! must lie inside the volume and the disk, a file's chain must have exactly as
//! many clusters as its size needs and end in end-of-chain, directory walks and
//! path depth are bounded, and names are 8.3 only (the image build writes
//! upper-case 8.3 names, so long-name entries are skipped, never needed).
//! Nothing here ever writes.

use crate::klog::fatlog::{le16, le32, parse_bpb, Bpb, EOC_MIN, SECTOR};

/// Directory clusters walked at most per directory (a looping chain must end).
const MAX_DIR_CLUSTERS: u32 = 4096;
/// Path components at most.
const MAX_DEPTH: usize = 8;

const ATTR_VOLUME: u8 = 0x08;
const ATTR_DIR: u8 = 0x10;
const ATTR_LFN: u8 = 0x0F;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRef {
    pub first: u32,
    pub size: u32,
}

pub struct Volume {
    pub bpb: Bpb,
    disk_sectors: u64,
    /// One cached FAT sector: firmware reads walk the FAT sequentially.
    fat_lba: u64,
    fat: [u8; SECTOR],
}

/// "AMSS.BIN" -> b"AMSS    BIN". Upper-case 8.3 only; None for anything else.
pub fn name83(s: &str) -> Option<[u8; 11]> {
    let (base, ext) = match s.split_once('.') {
        Some((b, e)) => (b, e),
        None => (s, ""),
    };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || ext.contains('.') {
        return None;
    }
    let ok = |c: u8| c.is_ascii_uppercase() || c.is_ascii_digit() || b"!#$%&'()-@^_`{}~".contains(&c);
    let mut out = [b' '; 11];
    for (i, c) in base.bytes().enumerate() {
        if !ok(c) {
            return None;
        }
        out[i] = c;
    }
    for (i, c) in ext.bytes().enumerate() {
        if !ok(c) {
            return None;
        }
        out[8 + i] = c;
    }
    Some(out)
}

impl Volume {
    pub fn open(read: &mut dyn FnMut(u64, &mut [u8; SECTOR]) -> bool, disk_sectors: u64) -> Result<Volume, &'static str> {
        let mut s = [0u8; SECTOR];
        if !read(0, &mut s) {
            return Err("cannot read LBA 0");
        }
        let bpb = parse_bpb(&s)?;
        if bpb.total > disk_sectors {
            return Err("FAT32 volume larger than the disk");
        }
        Ok(Volume { bpb, disk_sectors, fat_lba: u64::MAX, fat: [0; SECTOR] })
    }

    fn clus_bytes(&self) -> usize {
        self.bpb.sec_per_clus as usize * SECTOR
    }

    /// Next cluster after `c` (first FAT copy), through the one-sector cache.
    fn next(&mut self, read: &mut dyn FnMut(u64, &mut [u8; SECTOR]) -> bool, c: u32) -> Result<u32, &'static str> {
        let off = c as u64 * 4;
        let lba = self.bpb.fat_lba + off / SECTOR as u64;
        if lba >= self.bpb.data_lba {
            return Err("FAT lookup outside the FAT");
        }
        if lba != self.fat_lba {
            if !read(lba, &mut self.fat) {
                self.fat_lba = u64::MAX;
                return Err("cannot read the FAT");
            }
            self.fat_lba = lba;
        }
        Ok(le32(&self.fat, (off % SECTOR as u64) as usize) & 0x0FFF_FFFF)
    }

    fn cluster_ok(&self, c: u32) -> bool {
        self.bpb.valid(c) && {
            let end = self.bpb.cluster_lba(c) + self.bpb.sec_per_clus;
            end <= self.bpb.total && end <= self.disk_sectors
        }
    }

    /// Entry `name` in the directory starting at cluster `dir`: (first cluster, size, attributes).
    fn find(
        &mut self,
        read: &mut dyn FnMut(u64, &mut [u8; SECTOR]) -> bool,
        dir: u32,
        name: &[u8; 11],
    ) -> Result<Option<(u32, u32, u8)>, &'static str> {
        let mut clus = dir;
        let mut s = [0u8; SECTOR];
        for _ in 0..MAX_DIR_CLUSTERS {
            if !self.cluster_ok(clus) {
                return Err("bad directory chain");
            }
            for k in 0..self.bpb.sec_per_clus {
                if !read(self.bpb.cluster_lba(clus) + k, &mut s) {
                    return Err("cannot read a directory");
                }
                for e in s.chunks_exact(32) {
                    if e[0] == 0x00 {
                        return Ok(None); // end of directory
                    }
                    if e[0] == 0xE5 || e[11] & 0x3F == ATTR_LFN || e[11] & ATTR_VOLUME != 0 {
                        continue;
                    }
                    if &e[0..11] == name {
                        let first = (le16(e, 20) << 16) | le16(e, 26);
                        return Ok(Some((first, le32(e, 28), e[11])));
                    }
                }
            }
            let n = self.next(read, clus)?;
            if n >= EOC_MIN {
                return Ok(None);
            }
            clus = n;
        }
        Err("directory chain too long (loop?)")
    }

    /// Find a regular file by absolute path ("/FW/WCN6855/AMSS.BIN").
    pub fn lookup(&mut self, read: &mut dyn FnMut(u64, &mut [u8; SECTOR]) -> bool, path: &str) -> Result<FileRef, &'static str> {
        let parts = path.split('/').filter(|p| !p.is_empty());
        let n = parts.clone().count();
        if n == 0 || n > MAX_DEPTH {
            return Err("bad path");
        }
        let mut dir = self.bpb.root_clus;
        for (i, p) in parts.enumerate() {
            let name = name83(p).ok_or("path is not upper-case 8.3")?;
            let (first, size, attr) = self.find(read, dir, &name)?.ok_or("file not found")?;
            let last = i + 1 == n;
            if !last {
                if attr & ATTR_DIR == 0 {
                    return Err("path component is not a directory");
                }
                if !self.bpb.valid(first) {
                    return Err("directory points outside the volume");
                }
                dir = first;
            } else {
                if attr & ATTR_DIR != 0 {
                    return Err("path is a directory");
                }
                if size != 0 && !self.bpb.valid(first) {
                    return Err("file points outside the volume");
                }
                return Ok(FileRef { first, size });
            }
        }
        Err("bad path")
    }

    /// Read the whole file into `out` (at least `f.size` bytes). `progress`
    /// gets the bytes read so far after every cluster. The chain must have
    /// exactly the clusters the size needs, then end-of-chain.
    pub fn read_file(
        &mut self,
        read: &mut dyn FnMut(u64, &mut [u8; SECTOR]) -> bool,
        f: FileRef,
        out: &mut [u8],
        progress: &mut dyn FnMut(usize),
    ) -> Result<usize, &'static str> {
        let size = f.size as usize;
        if out.len() < size {
            return Err("buffer smaller than the file");
        }
        if size == 0 {
            return Ok(0);
        }
        let cb = self.clus_bytes();
        let need = size.div_ceil(cb);
        let mut s = [0u8; SECTOR];
        let mut c = f.first;
        for i in 0..need {
            if !self.cluster_ok(c) {
                return Err("file cluster outside the volume");
            }
            let lba = self.bpb.cluster_lba(c);
            for k in 0..self.bpb.sec_per_clus as usize {
                let off = i * cb + k * SECTOR;
                if off >= size {
                    break;
                }
                if !read(lba + k as u64, &mut s) {
                    return Err("cannot read file data");
                }
                let n = (size - off).min(SECTOR);
                out[off..off + n].copy_from_slice(&s[..n]);
            }
            progress(((i + 1) * cb).min(size));
            let n = self.next(read, c)?;
            if i + 1 < need {
                if n >= EOC_MIN {
                    return Err("file chain shorter than its size");
                }
                c = n;
            } else if n < EOC_MIN {
                return Err("file chain longer than its size");
            }
        }
        Ok(size)
    }
}
