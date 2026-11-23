//! POPCORN.LOG on the boot stick (`klog::fatlog`): finding the file on real FAT32
//! images made by the same tools as the flash image (mkfs.vfat + mtools, and the
//! build's own `popcorn_log_file`), refusing anything that is not one contiguous
//! regular file, and the header format carried from boot to boot.
//!
//! Image tests need mkfs.vfat and mtools (the image build needs them too); without
//! them they print a note and pass.

use crate::klog::fatlog::{self, Header, SECTOR};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

fn have(tool: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {tool}")).output().map_or(false, |o| o.status.success())
}

fn tools() -> bool {
    let ok = have("mkfs.vfat") && have("mcopy") && have("mdel") && have("mmd") && have("bash");
    if !ok {
        eprintln!("klog image tests skipped: need mkfs.vfat, mtools and bash");
    }
    ok
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "popcorn-klog-{}-{}-{}",
        std::process::id(),
        tag,
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(cmd: &mut Command) {
    let o = cmd.output().expect("spawn");
    assert!(o.status.success(), "{:?} failed: {}", cmd, String::from_utf8_lossy(&o.stderr));
}

/// POPCORN.LOG exactly as the image build writes it (scripts/lib/common.sh).
fn build_log_file(dir: &Path) -> PathBuf {
    let out = dir.join("POPCORN.LOG");
    let root = repo();
    run(Command::new("bash")
        .arg("-c")
        .arg(". scripts/lib/common.sh && popcorn_log_file \"$1\"")
        .arg("_")
        .arg(&out)
        .current_dir(&root)
        .env("POPCORN_ROOT", &root)
        .env("POPCORN_SRC", root.join("src"))
        .env("POPCORN_TARGET", dir.join("target"))
        .env("RUNTIME_DIR", dir.join("rt"))
        .env_remove("POPCORN_BUILD_COMMON"));
    out
}

/// Blank 64 MiB FAT32 superfloppy, formatted like `populate_fat_image`.
fn blank_image(dir: &Path) -> PathBuf {
    let img = dir.join("stick.img");
    let f = std::fs::File::create(&img).unwrap();
    f.set_len(64 << 20).unwrap();
    drop(f);
    run(Command::new("mkfs.vfat").args(["-F", "32", "-n", "POPCORN"]).arg(&img));
    img
}

fn mcopy(img: &Path, src: &Path, dst: &str) {
    run(Command::new("mcopy").arg("-i").arg(img).arg(src).arg(format!("::{dst}")));
}

fn file_of(dir: &Path, name: &str, bytes: usize, seed: u8) -> PathBuf {
    let p = dir.join(name);
    let v: Vec<u8> = (0..bytes).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect();
    std::fs::write(&p, v).unwrap();
    p
}

/// The flash image's layout: loader, kernel, then the log file.
fn flash_like_image(dir: &Path) -> (PathBuf, Vec<u8>) {
    let img = blank_image(dir);
    run(Command::new("mmd").arg("-i").arg(&img).args(["::/EFI", "::/EFI/BOOT", "::/boot"]));
    mcopy(&img, &file_of(dir, "BOOTX64.EFI", 14848, 1), "/EFI/BOOT/BOOTX64.EFI");
    mcopy(&img, &file_of(dir, "kernel", 1_157_624, 2), "/boot/kernel");
    mcopy(&img, &file_of(dir, "kernel2", 1_157_624, 3), "/EFI/BOOT/kernel");
    let log = build_log_file(dir);
    mcopy(&img, &log, "/POPCORN.LOG");
    (img, std::fs::read(&log).unwrap())
}

fn locate_in(img: &[u8], disk_sectors: u64) -> Result<fatlog::Extent, &'static str> {
    let mut read = |lba: u64, s: &mut [u8; SECTOR]| {
        let o = lba as usize * SECTOR;
        if o + SECTOR > img.len() {
            return false;
        }
        s.copy_from_slice(&img[o..o + SECTOR]);
        true
    };
    fatlog::locate(&mut read, disk_sectors)
}

fn sectors(img: &[u8]) -> u64 {
    (img.len() / SECTOR) as u64
}

#[test]
fn build_writes_the_layout_the_kernel_expects() {
    if !tools() {
        return;
    }
    let dir = scratch("layout");
    let log = std::fs::read(build_log_file(&dir)).unwrap();
    assert_eq!(log.len(), fatlog::FILE_BYTES);
    let mut h = [0u8; SECTOR];
    h.copy_from_slice(&log[..SECTOR]);
    assert_eq!(h[SECTOR - 1], b'\n');
    assert_eq!(fatlog::parse_header(&h, fatlog::FILE_BYTES - SECTOR), Header { boot: 0, used: 0 });
    for (off, &b) in log.iter().enumerate().skip(SECTOR) {
        assert_eq!(b, fatlog::pad_byte(off), "padding differs at {off}");
    }
}

#[test]
fn finds_the_log_on_a_flash_like_image() {
    if !tools() {
        return;
    }
    let dir = scratch("flash");
    let (img, log) = flash_like_image(&dir);
    let bytes = std::fs::read(&img).unwrap();
    let ext = locate_in(&bytes, sectors(&bytes)).expect("locate");
    assert_eq!(ext.sectors as usize * SECTOR, fatlog::FILE_BYTES);
    assert!(ext.lba >= 34, "inside the partition-table zone: {}", ext.lba);
    assert_eq!(ext.text_capacity(), fatlog::FILE_BYTES - SECTOR);
    let at = ext.lba as usize * SECTOR;
    assert_eq!(&bytes[at..at + log.len()], &log[..], "extent is not the file's bytes");
    /* The same image on a bigger stick (Etcher onto 16 GB) still works. */
    assert_eq!(locate_in(&bytes, 30_000_000), Ok(ext));
}

#[test]
fn writes_inside_the_extent_leave_a_clean_volume() {
    if !tools() || !have("fsck.fat") {
        return;
    }
    let dir = scratch("fsck");
    let (img, _) = flash_like_image(&dir);
    let mut bytes = std::fs::read(&img).unwrap();
    let ext = locate_in(&bytes, sectors(&bytes)).unwrap();
    /* What the kernel does: header + text sectors, anywhere inside the run. */
    let hdr = fatlog::header(Header { boot: 4, used: 3000 }, ext.text_capacity(), 0, 1234);
    let at = ext.lba as usize * SECTOR;
    bytes[at..at + SECTOR].copy_from_slice(&hdr);
    let last = at + (ext.sectors as usize - 1) * SECTOR;
    bytes[at + SECTOR..at + SECTOR + 3000].fill(b'x');
    bytes[last..last + SECTOR].fill(b'y');
    std::fs::write(&img, &bytes).unwrap();
    let o = Command::new("fsck.fat").arg("-n").arg(&img).output().unwrap();
    assert!(o.status.success(), "fsck.fat: {}", String::from_utf8_lossy(&o.stdout));
    let mut h = [0u8; SECTOR];
    h.copy_from_slice(&bytes[at..at + SECTOR]);
    assert_eq!(fatlog::parse_header(&h, ext.text_capacity()), Header { boot: 4, used: 3000 });
}

#[test]
fn no_log_file_is_reported() {
    if !tools() {
        return;
    }
    let dir = scratch("missing");
    let img = blank_image(&dir);
    mcopy(&img, &file_of(&dir, "OTHER.TXT", 4096, 9), "/OTHER.TXT");
    let bytes = std::fs::read(&img).unwrap();
    assert_eq!(locate_in(&bytes, sectors(&bytes)), Err("no POPCORN.LOG on the boot stick"));
}

/// FAT geometry of a mkfs.vfat image: (first FAT byte offset, data LBA, sectors per cluster).
fn geometry(b: &[u8]) -> (usize, u64, u64) {
    let le16 = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u64;
    let le32 = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as u64;
    let reserved = le16(14);
    let data = reserved + b[16] as u64 * le32(36);
    (reserved as usize * SECTOR, data, b[13] as u64)
}

fn fat_get(b: &[u8], fat: usize, c: u32) -> u32 {
    let o = fat + c as usize * 4;
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) & 0x0FFF_FFFF
}

fn fat_set(b: &mut [u8], fat: usize, c: u32, v: u32) {
    let o = fat + c as usize * 4;
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// The log's first and last cluster, plus a free cluster after it.
fn log_clusters(b: &[u8]) -> (usize, u32, u32, u32) {
    let ext = locate_in(b, sectors(b)).unwrap();
    let (fat, data, spc) = geometry(b);
    let first = ((ext.lba - data) / spc) as u32 + 2;
    let last = first + (ext.sectors / spc) as u32 - 1;
    let free = (last + 1..).find(|&c| fat_get(b, fat, c) == 0).unwrap();
    (fat, first, last, free)
}

#[test]
fn a_fragmented_log_is_refused() {
    if !tools() {
        return;
    }
    let dir = scratch("frag");
    let (img, _) = flash_like_image(&dir);
    let mut bytes = std::fs::read(&img).unwrap();
    let (fat, first, _, free) = log_clusters(&bytes);
    /* first -> free -> first+2: a detour through a cluster outside the run. */
    fat_set(&mut bytes, fat, first, free);
    fat_set(&mut bytes, fat, free, first + 2);
    assert_eq!(locate_in(&bytes, sectors(&bytes)), Err("POPCORN.LOG is fragmented"));
}

#[test]
fn a_chain_longer_than_the_file_is_refused() {
    if !tools() {
        return;
    }
    let dir = scratch("long");
    let (img, _) = flash_like_image(&dir);
    let mut bytes = std::fs::read(&img).unwrap();
    let (fat, _, last, free) = log_clusters(&bytes);
    fat_set(&mut bytes, fat, last, free);
    fat_set(&mut bytes, fat, free, 0x0FFF_FFFF);
    assert_eq!(locate_in(&bytes, sectors(&bytes)), Err("POPCORN.LOG chain is longer than its size"));
}

#[test]
fn odd_sizes_and_directories_are_refused() {
    if !tools() {
        return;
    }
    let dir = scratch("odd");
    let img = blank_image(&dir);
    mcopy(&img, &file_of(&dir, "POPCORN.LOG", 100_000, 6), "/POPCORN.LOG");
    let bytes = std::fs::read(&img).unwrap();
    assert_eq!(locate_in(&bytes, sectors(&bytes)), Err("POPCORN.LOG has an unusable size"));

    let img = blank_image(&dir);
    mcopy(&img, &file_of(&dir, "POPCORN.LOG", 2048, 7), "/POPCORN.LOG");
    let bytes = std::fs::read(&img).unwrap();
    assert_eq!(locate_in(&bytes, sectors(&bytes)), Err("POPCORN.LOG has an unusable size"));

    let img = blank_image(&dir);
    run(Command::new("mmd").arg("-i").arg(&img).arg("::/POPCORN.LOG"));
    let bytes = std::fs::read(&img).unwrap();
    assert_eq!(locate_in(&bytes, sectors(&bytes)), Err("POPCORN.LOG is not a regular file"));
}

#[test]
fn a_volume_bigger_than_the_disk_is_refused() {
    if !tools() {
        return;
    }
    let dir = scratch("small");
    let (img, _) = flash_like_image(&dir);
    let bytes = std::fs::read(&img).unwrap();
    assert_eq!(locate_in(&bytes, sectors(&bytes) - 1), Err("FAT32 volume larger than the disk"));
}

#[test]
fn a_looping_root_directory_chain_terminates() {
    if !tools() {
        return;
    }
    let dir = scratch("loop");
    let img = blank_image(&dir);
    mcopy(&img, &file_of(&dir, "OTHER.TXT", 4096, 8), "/OTHER.TXT");
    let mut bytes = std::fs::read(&img).unwrap();
    /* Point the root cluster's FAT entry at itself (corrupt stick). */
    let reserved = u16::from_le_bytes([bytes[14], bytes[15]]) as usize;
    let root = u32::from_le_bytes([bytes[44], bytes[45], bytes[46], bytes[47]]);
    let at = reserved * SECTOR + root as usize * 4;
    bytes[at..at + 4].copy_from_slice(&root.to_le_bytes());
    assert_eq!(locate_in(&bytes, sectors(&bytes)), Err("no POPCORN.LOG on the boot stick"));
}

#[test]
fn not_fat32_is_refused() {
    let mut img = vec![0u8; 4 << 20];
    assert_eq!(locate_in(&img, sectors(&img)), Err("no boot signature at LBA 0"));
    img[510] = 0x55;
    img[511] = 0xAA;
    img[11] = 0;
    img[12] = 8; /* 2048-byte sectors */
    assert_eq!(locate_in(&img, sectors(&img)), Err("sector size is not 512"));
    img[11] = 0;
    img[12] = 2;
    img[13] = 3; /* not a power of two */
    assert_eq!(locate_in(&img, sectors(&img)), Err("bad sectors per cluster"));
    img[13] = 1;
    img[14] = 32;
    img[16] = 2;
    img[17] = 0;
    img[18] = 2; /* FAT12/16 root entries */
    assert_eq!(locate_in(&img, sectors(&img)), Err("not a FAT32 volume"));
    assert_eq!(locate_in(&[], 0), Err("cannot read LBA 0"));
}

#[test]
fn header_round_trips_and_garbage_reads_as_fresh() {
    let cap = fatlog::FILE_BYTES - SECTOR;
    let h = fatlog::header(Header { boot: 12, used: 98_765 }, cap, 42, 61_234);
    assert_eq!(h.len(), SECTOR);
    assert_eq!(h[SECTOR - 1], b'\n');
    let text = std::str::from_utf8(&h).unwrap();
    assert!(text.starts_with("POPCORN.LOG  boot 12  used 98765 of 1048064 bytes  dropped 42  uptime 61.234 s\n"), "{text}");
    assert_eq!(fatlog::parse_header(&h, cap), Header { boot: 12, used: 98_765 });

    /* Garbage / foreign content: a fresh file whose whole text region gets blanked. */
    assert_eq!(fatlog::parse_header(&[b'z'; SECTOR], cap), Header { boot: 0, used: cap });
    /* A `used` past the file is clamped; a missing one means "blank it all". */
    let big = fatlog::header(Header { boot: 1, used: 5_000_000 }, cap, 0, 0);
    assert_eq!(fatlog::parse_header(&big, cap), Header { boot: 1, used: cap });
    let mut no_used = [b' '; SECTOR];
    no_used[..20].copy_from_slice(b"POPCORN.LOG  boot 3\n");
    assert_eq!(fatlog::parse_header(&no_used, cap), Header { boot: 3, used: cap });
    /* Fields on later lines do not count. */
    let mut later = [b' '; SECTOR];
    later[..33].copy_from_slice(b"POPCORN.LOG  boot 2\nfoo used 99\n ");
    assert_eq!(fatlog::parse_header(&later, cap), Header { boot: 2, used: cap });
}
