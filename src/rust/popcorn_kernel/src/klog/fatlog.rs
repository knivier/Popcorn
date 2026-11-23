//! Pure core of the USB-stick log: find `POPCORN.LOG` on a FAT32 superfloppy
//! (the flash image is a whole-disk FAT32 volume, BPB at LBA 0) and define the
//! file's on-disk layout. No I/O, no globals: the kernel passes a sector reader,
//! the host tests pass a FAT32 image.
//!
//! The file is created at full size by the image build (`uefi_stage_layout`), so
//! the kernel never allocates clusters or touches the FAT or a directory: it only
//! rewrites sectors inside the file's own, verified, contiguous cluster run.
//!
//! Layout (all plain text):
//!   sector 0          header: `POPCORN.LOG  boot <n>  used <bytes> of <cap> ...`
//!   sectors 1..       log text (`used` bytes), then padding
//!   padding           spaces, with '\n' at every file offset where offset % 64 == 63

pub const SECTOR: usize = 512;
/// File size the image build creates (must match scripts/lib/common.sh).
pub const FILE_BYTES: usize = 1 << 20;
/// Smallest file the kernel accepts (header + a little text).
pub const MIN_FILE_BYTES: usize = 8 * SECTOR;
/// 8.3 directory name of the log file.
pub const NAME_83: &[u8; 11] = b"POPCORN LOG";

/// Where the log lives on the disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// First sector of the file (its header).
    pub lba: u64,
    /// Sectors in the file (header included).
    pub sectors: u64,
}

impl Extent {
    /// Bytes of log text the file can hold (everything after the header).
    pub fn text_capacity(&self) -> usize {
        (self.sectors as usize - 1) * SECTOR
    }
}

/// Padding byte at `file_offset` (blank lines of 63 spaces).
pub fn pad_byte(file_offset: usize) -> u8 {
    if file_offset % 64 == 63 {
        b'\n'
    } else {
        b' '
    }
}

fn le16(b: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([b[o], b[o + 1]]) as u32
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// FAT32 geometry from a boot sector.
#[derive(Clone, Copy, Debug)]
struct Bpb {
    sec_per_clus: u64,
    fat_lba: u64,
    data_lba: u64,
    root_clus: u32,
    clusters: u32,
    total: u64,
}

fn parse_bpb(s: &[u8; SECTOR]) -> Result<Bpb, &'static str> {
    if s[510] != 0x55 || s[511] != 0xAA {
        return Err("no boot signature at LBA 0");
    }
    if le16(s, 11) as usize != SECTOR {
        return Err("sector size is not 512");
    }
    let spc = s[13] as u64;
    if spc == 0 || !spc.is_power_of_two() {
        return Err("bad sectors per cluster");
    }
    let reserved = le16(s, 14) as u64;
    let fats = s[16] as u64;
    let root_entries = le16(s, 17);
    let fat16_size = le16(s, 22);
    let fat_size = le32(s, 36) as u64;
    let tot16 = le16(s, 19) as u64;
    let total = if tot16 != 0 { tot16 } else { le32(s, 32) as u64 };
    if reserved == 0 || fats == 0 || root_entries != 0 || fat16_size != 0 || fat_size == 0 {
        return Err("not a FAT32 volume");
    }
    let data_lba = reserved + fats * fat_size;
    if total <= data_lba {
        return Err("bad FAT32 geometry");
    }
    let clusters = ((total - data_lba) / spc) as u32;
    let root_clus = le32(s, 44) & 0x0FFF_FFFF;
    if root_clus < 2 || root_clus >= clusters + 2 {
        return Err("bad root cluster");
    }
    Ok(Bpb { sec_per_clus: spc, fat_lba: reserved, data_lba, root_clus, clusters, total })
}

impl Bpb {
    fn cluster_lba(&self, c: u32) -> u64 {
        self.data_lba + (c as u64 - 2) * self.sec_per_clus
    }

    /// Next cluster in the chain (first FAT copy). `None` on a read error.
    fn next(&self, read: &mut dyn FnMut(u64, &mut [u8; SECTOR]) -> bool, c: u32) -> Option<u32> {
        let off = c as u64 * 4;
        let mut s = [0u8; SECTOR];
        if !read(self.fat_lba + off / SECTOR as u64, &mut s) {
            return None;
        }
        Some(le32(&s, (off % SECTOR as u64) as usize) & 0x0FFF_FFFF)
    }

    fn valid(&self, c: u32) -> bool {
        c >= 2 && c < self.clusters + 2
    }
}

const EOC_MIN: u32 = 0x0FFF_FFF8;
/// Root directory clusters walked at most (a loop in the chain must not hang boot).
const MAX_ROOT_CLUSTERS: u32 = 64;

/// Find `POPCORN.LOG` in the root directory of the FAT32 volume at LBA 0 and
/// return its sectors, after checking that it is a regular file of a usable size
/// whose cluster chain is one contiguous run ending in end-of-chain, inside the
/// volume and inside the disk (`disk_sectors`).
pub fn locate(read: &mut dyn FnMut(u64, &mut [u8; SECTOR]) -> bool, disk_sectors: u64) -> Result<Extent, &'static str> {
    let mut s = [0u8; SECTOR];
    if !read(0, &mut s) {
        return Err("cannot read LBA 0");
    }
    let bpb = parse_bpb(&s)?;
    if bpb.total > disk_sectors {
        return Err("FAT32 volume larger than the disk");
    }

    /* Root directory: walk its chain looking for the 8.3 entry. */
    let mut clus = bpb.root_clus;
    let mut found: Option<(u32, u32)> = None;
    'walk: for _ in 0..MAX_ROOT_CLUSTERS {
        if !bpb.valid(clus) {
            return Err("bad root directory chain");
        }
        for k in 0..bpb.sec_per_clus {
            if !read(bpb.cluster_lba(clus) + k, &mut s) {
                return Err("cannot read the root directory");
            }
            for e in s.chunks_exact(32) {
                if e[0] == 0x00 {
                    break 'walk; /* end of directory */
                }
                if e[0] == 0xE5 || e[11] == 0x0F {
                    continue; /* deleted / long-name entry */
                }
                if &e[0..11] == NAME_83 {
                    if e[11] & 0x18 != 0 {
                        return Err("POPCORN.LOG is not a regular file");
                    }
                    let first = (le16(e, 20) << 16) | le16(e, 26);
                    found = Some((first, le32(e, 28)));
                    break 'walk;
                }
            }
        }
        match bpb.next(read, clus) {
            Some(n) if n >= EOC_MIN => break,
            Some(n) => clus = n,
            None => return Err("cannot read the FAT"),
        }
    }
    let (first, size) = found.ok_or("no POPCORN.LOG on the boot stick")?;
    let size = size as usize;
    if size < MIN_FILE_BYTES || size % SECTOR != 0 {
        return Err("POPCORN.LOG has an unusable size");
    }
    let clus_bytes = bpb.sec_per_clus as usize * SECTOR;
    let want = size.div_ceil(clus_bytes) as u32;
    if !bpb.valid(first) || !bpb.valid(first + want - 1) {
        return Err("POPCORN.LOG points outside the volume");
    }

    /* Contiguous chain: first, first+1, ..., first+want-1, then end-of-chain. */
    let mut c = first;
    for i in 0..want {
        let n = bpb.next(read, c).ok_or("cannot read the FAT")?;
        let last = i + 1 == want;
        if last {
            if n < EOC_MIN {
                return Err("POPCORN.LOG chain is longer than its size");
            }
        } else if n != c + 1 {
            return Err("POPCORN.LOG is fragmented");
        }
        c += 1;
    }

    let ext = Extent { lba: bpb.cluster_lba(first), sectors: (size / SECTOR) as u64 };
    if ext.lba + ext.sectors > bpb.total {
        return Err("POPCORN.LOG runs past the volume");
    }
    Ok(ext)
}

/// Header fields carried from one boot to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub boot: u32,
    pub used: usize,
}

/// Read `boot` and `used` from a header sector. Anything unparsable reads as a
/// fresh file (boot 0, used = whole capacity so the next boot clears it all).
pub fn parse_header(s: &[u8; SECTOR], capacity: usize) -> Header {
    fn field(s: &[u8], key: &[u8]) -> Option<u64> {
        let line = &s[..s.iter().position(|&b| b == b'\n').unwrap_or(s.len())];
        let at = line.windows(key.len()).position(|w| w == key)? + key.len();
        let digits = line[at..].iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 || digits > 12 {
            return None;
        }
        let mut v = 0u64;
        for &b in &line[at..at + digits] {
            v = v * 10 + (b - b'0') as u64;
        }
        Some(v)
    }
    if !s.starts_with(b"POPCORN.LOG ") {
        return Header { boot: 0, used: capacity };
    }
    let boot = field(s, b" boot ").unwrap_or(0).min(u32::MAX as u64) as u32;
    let used = field(s, b" used ").map(|u| (u as usize).min(capacity)).unwrap_or(capacity);
    Header { boot, used }
}

/// Small fixed-buffer writer for the header (no heap).
struct Out<'a> {
    b: &'a mut [u8; SECTOR],
    n: usize,
}

impl core::fmt::Write for Out<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n >= SECTOR - 1 {
                return Err(core::fmt::Error);
            }
            self.b[self.n] = c;
            self.n += 1;
        }
        Ok(())
    }
}

/// Header sector text. `uptime_ms` and `dropped` are informational.
pub fn header(h: Header, capacity: usize, dropped: u64, uptime_ms: u64) -> [u8; SECTOR] {
    use core::fmt::Write;
    let mut b = [b' '; SECTOR];
    let mut o = Out { b: &mut b, n: 0 };
    let _ = write!(
        o,
        "POPCORN.LOG  boot {}  used {} of {} bytes  dropped {}  uptime {}.{:03} s\n\
         Kernel log of the last boot from this USB stick, rewritten while Popcorn runs.\n\
         The text starts on the next line and ends after `used` bytes; the rest is blank padding.\n",
        h.boot,
        h.used,
        capacity,
        dropped,
        uptime_ms / 1000,
        uptime_ms % 1000
    );
    /* Pad to the end of the sector so the text starts on a fresh line. */
    b[SECTOR - 1] = b'\n';
    b
}
