//! Wi-Fi stage W1: SHA-256, the firmware manifest (and the build script that
//! writes it), the read-only FAT32 reader on real mkfs.vfat/mtools images, and
//! the MHI boot wire formats.

use crate::drivers::bus::mhi::{self, Event};
use crate::drivers::wifi::fw_manifest;
use crate::fs::fat_ro::{self, Volume};
use crate::klog::fatlog::SECTOR;
use crate::klog_tests::{blank_image, fat_get, fat_set, geometry, have, mcopy, repo, run, scratch, tools};
use crate::util::sha256;
use std::path::Path;
use std::process::Command;

fn hex(d: &[u8; 32]) -> String {
    String::from_utf8(sha256::to_hex(d).to_vec()).unwrap()
}

// ---------------------------------------------------------------- SHA-256

#[test]
fn sha256_nist_vectors() {
    assert_eq!(hex(&sha256::digest(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hex(&sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(
        hex(&sha256::digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    let million = vec![b'a'; 1_000_000];
    assert_eq!(hex(&sha256::digest(&million)), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
}

#[test]
fn sha256_streaming_matches_one_shot_at_every_boundary() {
    let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 + 3) as u8).collect();
    for len in [0usize, 1, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 129, 1000] {
        let want = sha256::digest(&data[..len]);
        for chunk in [1usize, 3, 63, 64, 65, 200] {
            let mut s = sha256::Sha256::new();
            for c in data[..len].chunks(chunk) {
                s.update(c);
            }
            assert_eq!(s.finish(), want, "len {} chunk {}", len, chunk);
        }
    }
}

#[test]
fn sha256_matches_the_sha256sum_tool() {
    if !have("sha256sum") {
        eprintln!("skipped: no sha256sum");
        return;
    }
    let dir = scratch("sha");
    let p = dir.join("blob");
    let data: Vec<u8> = (0..300_001u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    std::fs::write(&p, &data).unwrap();
    let o = Command::new("sha256sum").arg(&p).output().unwrap();
    let tool = String::from_utf8_lossy(&o.stdout).split_whitespace().next().unwrap().to_string();
    assert_eq!(hex(&sha256::digest(&data)), tool);
}

#[test]
fn sha256_hex_round_trip() {
    let d = sha256::digest(b"popcorn");
    let h = sha256::to_hex(&d);
    assert_eq!(sha256::from_hex(&h), Some(d));
    let upper: Vec<u8> = h.iter().map(|c| c.to_ascii_uppercase()).collect();
    assert_eq!(sha256::from_hex(&upper), Some(d));
    assert_eq!(sha256::from_hex(&h[..63]), None);
    let mut bad = h;
    bad[10] = b'g';
    assert_eq!(sha256::from_hex(&bad), None);
}

// ---------------------------------------------------------------- manifest

#[test]
fn manifest_entries_and_verification() {
    let a = b"firmware A";
    let b = vec![7u8; 1234];
    let man = format!(
        "# Popcorn Wi-Fi firmware manifest: sha256 bytes name\n{} {} AMSS.BIN\n\n{} {} M3.BIN  # trailing comment\n",
        hex(&sha256::digest(a)),
        a.len(),
        hex(&sha256::digest(&b)),
        b.len()
    );
    let e = fw_manifest::entry(man.as_bytes(), "AMSS.BIN").unwrap();
    assert_eq!(e.size, a.len());
    assert!(fw_manifest::verify(a, &e).is_ok());
    assert_eq!(fw_manifest::verify(b"firmware B", &e), Err("SHA-256 differs from the manifest"));
    assert_eq!(fw_manifest::verify(b"short", &e), Err("size differs from the manifest"));
    let m3 = fw_manifest::entry(man.as_bytes(), "M3.BIN").unwrap();
    assert!(fw_manifest::verify(&b, &m3).is_ok());
    assert_eq!(fw_manifest::entry(man.as_bytes(), "amss.bin"), Err("file not listed in the manifest"));
    let mut names = Vec::new();
    assert_eq!(fw_manifest::for_each(man.as_bytes(), |n, _| names.push(n.to_string())), Ok(2));
    assert_eq!(names, ["AMSS.BIN", "M3.BIN"]);
    assert_eq!(fw_manifest::entry(b"abc 12 X\n", "X"), Err("bad SHA-256 in manifest"));
    assert_eq!(fw_manifest::entry(b"only two\n", "X"), Err("malformed manifest line"));
    assert_eq!(fw_manifest::for_each(&[0xFF, 0xFE], |_, _| {}), Err("manifest is not text"));
}

/// The image build's firmware staging (scripts/lib/firmware.sh): verified files
/// go to FW/WCN6855 with a MANIFEST.TXT the kernel parser accepts; a file that
/// does not match the pinned SHA-256 is never staged.
#[test]
fn build_stages_only_verified_firmware() {
    if !have("bash") || !have("sha256sum") {
        eprintln!("skipped: need bash and sha256sum");
        return;
    }
    let dir = scratch("fwstage");
    let fw = dir.join("fw");
    std::fs::create_dir_all(&fw).unwrap();
    let files: [(&str, Vec<u8>); 2] = [("AMSS.BIN", vec![0xA5; 70_000]), ("LICENSE.TXT", b"licence text\n".to_vec())];
    let mut pinned = String::from("# test manifest\n");
    for (name, data) in &files {
        std::fs::write(fw.join(name), data).unwrap();
        pinned += &format!("{} {} some/path/{} {}\n", hex(&sha256::digest(data)), data.len(), name, name);
    }
    let manifest = dir.join("test.manifest");
    std::fs::write(&manifest, &pinned).unwrap();
    let stage = |tag: &str| -> std::path::PathBuf {
        let st = dir.join(tag);
        std::fs::create_dir_all(&st).unwrap();
        let root = repo();
        run(Command::new("bash")
            .arg("-c")
            .arg(". scripts/lib/common.sh && . scripts/lib/firmware.sh && firmware_stage \"$1\"")
            .arg("_")
            .arg(&st)
            .current_dir(&root)
            .env("POPCORN_ROOT", &root)
            .env("POPCORN_SRC", root.join("src"))
            .env("POPCORN_TARGET", dir.join("target"))
            .env("RUNTIME_DIR", dir.join("rt"))
            .env("POPCORN_FW_MANIFEST", &manifest)
            .env("POPCORN_FW_DIR", &fw)
            .env("POPCORN_FW_URL", "file:///nonexistent-popcorn-firmware")
            .env_remove("POPCORN_FIRMWARE"));
        st
    };
    let st = stage("good");
    let man = std::fs::read(st.join("FW/WCN6855/MANIFEST.TXT")).unwrap();
    let mut seen = 0;
    fw_manifest::for_each(&man, |name, e| {
        let data = std::fs::read(st.join("FW/WCN6855").join(name)).unwrap();
        assert!(fw_manifest::verify(&data, &e).is_ok(), "{}", name);
        seen += 1;
    })
    .unwrap();
    assert_eq!(seen, 2);

    // Tamper with a cached file: it must not reach the stick (and the fake URL
    // makes the re-download fail).
    std::fs::write(fw.join("AMSS.BIN"), vec![0x5A; 70_000]).unwrap();
    let st = stage("tampered");
    assert!(!st.join("FW").exists(), "unverified firmware was staged");
}

// ---------------------------------------------------------------- FAT32 reader

fn reader(img: &[u8]) -> impl FnMut(u64, &mut [u8; SECTOR]) -> bool + '_ {
    move |lba, buf| {
        let o = lba as usize * SECTOR;
        if o + SECTOR > img.len() {
            return false;
        }
        buf.copy_from_slice(&img[o..o + SECTOR]);
        true
    }
}

fn pattern(n: usize, seed: u32) -> Vec<u8> {
    (0..n as u32).map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 11) as u8).collect()
}

fn read_back(img: &[u8], path: &str) -> Result<Vec<u8>, &'static str> {
    let mut rd = reader(img);
    let mut vol = Volume::open(&mut rd, (img.len() / SECTOR) as u64)?;
    let f = vol.lookup(&mut rd, path)?;
    let mut out = vec![0u8; f.size as usize];
    let mut last = 0usize;
    vol.read_file(&mut rd, f, &mut out, &mut |done| {
        assert!(done >= last && done <= f.size as usize);
        last = done;
    })?;
    assert!(f.size == 0 || last == f.size as usize);
    Ok(out)
}

fn firmware_like_image(dir: &Path) -> (std::path::PathBuf, Vec<(String, Vec<u8>)>) {
    let img = blank_image(dir);
    run(Command::new("mmd").arg("-i").arg(&img).args(["::/FW", "::/FW/WCN6855"]));
    let mut files = Vec::new();
    for (i, n) in [0usize, 1, 511, 512, 513, 4096, 70_001, 300_000].iter().enumerate() {
        let name = format!("F{}.BIN", i);
        let data = pattern(*n, i as u32);
        let p = dir.join(&name);
        std::fs::write(&p, &data).unwrap();
        mcopy(&img, &p, &format!("/FW/WCN6855/{}", name));
        files.push((name, data));
    }
    (img, files)
}

#[test]
fn fat_ro_reads_nested_files_of_every_size() {
    if !tools() {
        return;
    }
    let dir = scratch("fatro");
    let (img, files) = firmware_like_image(&dir);
    let b = std::fs::read(&img).unwrap();
    for (name, data) in &files {
        let got = read_back(&b, &format!("/FW/WCN6855/{}", name)).unwrap();
        assert_eq!(&got, data, "{}", name);
    }
    assert_eq!(read_back(&b, "/FW/WCN6855/NOPE.BIN"), Err("file not found"));
    assert_eq!(read_back(&b, "/FW/WCN6855"), Err("path is a directory"));
    assert_eq!(read_back(&b, "/FW/WCN6855/F1.BIN/X"), Err("path component is not a directory"));
    assert_eq!(read_back(&b, "/fw/wcn6855/f1.bin"), Err("path is not upper-case 8.3"));
    assert_eq!(read_back(&b, "/"), Err("bad path"));
    assert_eq!(read_back(&b, "/A/B/C/D/E/F/G/H/I"), Err("bad path"));
}

#[test]
fn fat_ro_name83() {
    assert_eq!(&fat_ro::name83("AMSS.BIN").unwrap(), b"AMSS    BIN");
    assert_eq!(&fat_ro::name83("BOARD-2.BIN").unwrap(), b"BOARD-2 BIN");
    assert_eq!(&fat_ro::name83("WCN6855").unwrap(), b"WCN6855    ");
    assert_eq!(&fat_ro::name83("FW").unwrap(), b"FW         ");
    for bad in ["amss.bin", "TOOLONGNAME.BIN", "A.BINX", "A.B.C", "", ".BIN", "A B.BIN", "A*.BIN"] {
        assert_eq!(fat_ro::name83(bad), None, "{}", bad);
    }
}

#[test]
fn fat_ro_reads_a_fragmented_file() {
    if !tools() {
        return;
    }
    let dir = scratch("fatfrag");
    let img = blank_image(&dir);
    let big = pattern(200_000, 99);
    let p = dir.join("BIG.BIN");
    std::fs::write(&p, &big).unwrap();
    mcopy(&img, &p, "/BIG.BIN");
    let mut b = std::fs::read(&img).unwrap();
    let (fat, data_lba, spc) = geometry(&b);
    let first = {
        let mut rd = reader(&b);
        let mut v = Volume::open(&mut rd, (b.len() / SECTOR) as u64).unwrap();
        v.lookup(&mut rd, "/BIG.BIN").unwrap().first
    };
    let mut chain = vec![first];
    while fat_get(&b, fat, *chain.last().unwrap()) < 0x0FFF_FFF8 {
        let n = fat_get(&b, fat, *chain.last().unwrap());
        chain.push(n);
    }
    // mtools writes files contiguously, so fragment by hand: move every other
    // middle cluster's data to free clusters far away and relink the chain.
    let clus_bytes = spc as usize * SECTOR;
    let at = |c: u32| (data_lba as usize + (c as usize - 2) * spc as usize) * SECTOR;
    let mut spare = chain.last().unwrap() + 1000;
    for k in (2..chain.len() - 2).step_by(2) {
        let (prev, old, next) = (chain[k - 1], chain[k], chain[k + 1]);
        let (src, dst) = (at(old), at(spare));
        let data = b[src..src + clus_bytes].to_vec();
        b[dst..dst + clus_bytes].copy_from_slice(&data);
        b[src..src + clus_bytes].fill(0xEE); // stale copy must never be read
        fat_set(&mut b, fat, prev, spare);
        fat_set(&mut b, fat, spare, next);
        fat_set(&mut b, fat, old, 0);
        chain[k] = spare;
        spare += 3;
    }
    assert!(chain.windows(2).any(|w| w[1] != w[0] + 1));
    assert_eq!(read_back(&b, "/BIG.BIN").unwrap(), big);
}

#[test]
fn fat_ro_refuses_chains_that_do_not_match_the_size() {
    if !tools() {
        return;
    }
    let dir = scratch("fatchain");
    let img = blank_image(&dir);
    let p = dir.join("A.BIN");
    std::fs::write(&p, pattern(20_000, 5)).unwrap();
    mcopy(&img, &p, "/A.BIN");
    let orig = std::fs::read(&img).unwrap();
    let (fat, _, _) = geometry(&orig);
    let first = {
        let mut rd = reader(&orig);
        let mut v = Volume::open(&mut rd, (orig.len() / SECTOR) as u64).unwrap();
        v.lookup(&mut rd, "/A.BIN").unwrap().first
    };
    // Walk to the last cluster.
    let mut chain = vec![first];
    while fat_get(&orig, fat, *chain.last().unwrap()) < 0x0FFF_FFF8 {
        let n = fat_get(&orig, fat, *chain.last().unwrap());
        chain.push(n);
    }
    assert!(chain.len() >= 3);
    // Cut the chain short.
    let mut b = orig.clone();
    fat_set(&mut b, fat, chain[1], 0x0FFF_FFFF);
    assert_eq!(read_back(&b, "/A.BIN"), Err("file chain shorter than its size"));
    // Extend past the size into a free cluster.
    let mut b = orig.clone();
    let spare = chain.last().unwrap() + 50;
    fat_set(&mut b, fat, *chain.last().unwrap(), spare);
    fat_set(&mut b, fat, spare, 0x0FFF_FFFF);
    assert_eq!(read_back(&b, "/A.BIN"), Err("file chain longer than its size"));
    // Point into the reserved clusters.
    let mut b = orig.clone();
    fat_set(&mut b, fat, chain[0], 1);
    assert_eq!(read_back(&b, "/A.BIN"), Err("file cluster outside the volume"));
    // A disk smaller than the volume is refused up front.
    let mut rd = reader(&orig);
    assert!(Volume::open(&mut rd, 100).is_err());
}

#[test]
fn fat_ro_looping_directory_terminates() {
    if !tools() {
        return;
    }
    let dir = scratch("fatloop");
    let img = blank_image(&dir);
    let mut b = std::fs::read(&img).unwrap();
    let (fat, data_lba, spc) = geometry(&b);
    // Fill the root directory with junk entries (no 0x00 terminator) and make
    // its cluster chain point at itself.
    let rc = {
        let mut rd = reader(&b);
        Volume::open(&mut rd, (b.len() / SECTOR) as u64).unwrap().bpb.root_clus
    };
    let lba = (data_lba + (rc as u64 - 2) * spc) as usize * SECTOR;
    for e in b[lba..lba + spc as usize * SECTOR].chunks_exact_mut(32) {
        if e[0] == 0 {
            e[..11].copy_from_slice(b"JUNK    BIN");
            e[11] = 0x20;
        }
    }
    fat_set(&mut b, fat, rc, rc);
    assert_eq!(read_back(&b, "/NOPE.BIN"), Err("directory chain too long (loop?)"));
}

// ---------------------------------------------------------------- MHI formats

#[test]
fn mhi_contexts_are_the_44_byte_layouts() {
    let e = mhi::event_ctx(1, 2, 0x1234_5000, 4096);
    assert_eq!(e[0], 1 << 16); // INTMODT
    assert_eq!(e[1], mhi::EV_TYPE_VALID);
    assert_eq!(e[2], 2); // msivec
    assert_eq!((e[3], e[4]), (0x1234_5000, 0)); // rbase @12
    assert_eq!((e[5], e[6]), (4096, 0)); // rlen @20
    assert_eq!((e[mhi::CTX_RP], e[mhi::CTX_RP + 1]), (0x1234_5000, 0)); // rp @28
    assert_eq!((e[mhi::CTX_WP], e[mhi::CTX_WP + 1]), (0x1234_5000, 0)); // wp @36
    let c = mhi::chan_ctx(mhi::CH_TYPE_INBOUND, 1, mhi::DB_BRST_DISABLE);
    assert_eq!(c[0], 2 << 8); // disabled, burst mode in bits 9:8
    assert_eq!((c[1], c[2]), (2, 1));
    assert!(c[3..].iter().all(|&w| w == 0));
    let k = mhi::cmd_ctx(0x2_0000_0800, 2048);
    assert_eq!(&k[..3], &[0, 0, 0]);
    assert_eq!((k[3], k[4]), (0x800, 2));
    assert_eq!(mhi::CTX_BYTES, 44);
}

#[test]
fn mhi_event_decoding() {
    // STATE_CHANGE to M0: type 0x20 in dword1 bits 23:16, state in dword0 31:24.
    assert_eq!(mhi::decode_event([0, 0, 0x0200_0000, 0x0020_0000]), Event::StateChange(mhi::STATE_M0));
    assert_eq!(mhi::decode_event([0, 0, 0x0100_0000, 0x0040_0000]), Event::ExecEnv(mhi::EE_SBL));
    assert_eq!(mhi::decode_event([0, 0, 0x0200_0000, 0x0040_0000]), Event::ExecEnv(mhi::EE_AMSS));
    assert_eq!(
        mhi::decode_event([0x1000, 0x1, 0x0100_0000, 0x0021_0000]),
        Event::CmdCompletion { code: 1, ptr: 0x1_0000_1000 }
    );
    assert_eq!(
        mhi::decode_event([0x40, 0, 0x0200_00FF, 0x1522_0000]),
        Event::Tx { chan: 0x15, code: 2, len: 0xFF, ptr: 0x40 }
    );
    assert!(matches!(mhi::decode_event([0, 0, 0, 0x0099_0000]), Event::Other { kind: 0x99, .. }));
}

#[test]
fn mhi_ring_pointers_wrap_and_reject_strays() {
    let base = 0x10_0000u64;
    assert_eq!(mhi::ring_next(base, 32, base), Some(base + 16));
    assert_eq!(mhi::ring_next(base, 32, base + 31 * 16), Some(base)); // wraps
    assert_eq!(mhi::ring_next(base, 32, base + 32 * 16), None); // one past
    assert_eq!(mhi::ring_next(base, 32, base + 8), None); // unaligned
    assert_eq!(mhi::ring_next(base, 32, base - 16), None);
    assert_eq!(mhi::ring_index(base, 32, base + 5 * 16), Some(5));
    assert_eq!(mhi::ring_index(base, 32, 0), None);
    assert_eq!(mhi::ring_index(base, u64::MAX, base), None); // overflow guarded
}

#[test]
fn mhi_bhie_table_for_the_real_amss_size() {
    // AMSS.BIN for WCN6855 is 4988928 bytes: 9 full 512 KiB segments + 270336.
    assert_eq!(mhi::bhie_segments(4_988_928, 512 * 1024), (10, 270_336));
    assert_eq!(mhi::bhie_segments(512 * 1024, 512 * 1024), (1, 512 * 1024));
    assert_eq!(mhi::bhie_segments(1, 512 * 1024), (1, 1));
    assert_eq!(mhi::bhie_segments(0, 512 * 1024), (0, 0));
    let e = mhi::vec_entry(0x1_2345_6000, 0x8_0000);
    assert_eq!(&e[..8], &0x1_2345_6000u64.to_le_bytes());
    assert_eq!(&e[8..], &0x8_0000u64.to_le_bytes());
}

#[test]
fn mhi_register_field_helpers() {
    for seed in [0u64, 1, 0x3FFF_FFFF, 0xFFFF_FFFF_FFFF_FFFF, 0x1234_5678_9ABC_DEF0] {
        let s = mhi::seq_number(seed);
        assert!(s != 0 && s <= 0x3FFF_FFFF, "{:#x}", s);
    }
    assert_eq!(mhi::txvecdb_with_seq(0xC000_0000, 0x1234), 0xC000_1234);
    assert_eq!(mhi::txvecdb_with_seq(0, 0xFFFF_FFFF), 0x3FFF_FFFF);
    assert_eq!(mhi::mhicfg_with_rings(0xAABB_1234, 2, 0), 0x0002_1234);
    assert_eq!(mhi::mhictrl_with_state(0x0000_0002, mhi::STATE_M0), 0x0000_0202);
    assert_eq!(mhi::mhictrl_with_state(0x0000_FF00, mhi::STATE_M0), 0x0000_0200);
    assert_eq!(mhi::xfer_status(0x8000_0000), mhi::XFER_DONE);
    assert_eq!(mhi::xfer_status(0xC000_0001), mhi::XFER_ERROR);
}

// ---------------------------------------------------------------- AMD IOMMU decoding

use crate::drivers::bus::iommu_regs::{self as ir, DmaPolicy};

#[test]
fn iommu_register_decoding() {
    assert_eq!(ir::mmio_base(0xFD20_0001, 0), Some(0xFD20_0000));
    assert_eq!(ir::mmio_base(0xFD20_0000, 0), None); // not enabled
    assert_eq!(ir::mmio_base(0x0000_4001, 0x1), Some(0x1_0000_4000));
    assert_eq!(ir::dev_table(0x0000_0001_2345_6000 | 0x1FF), (0x1_2345_6000, 512 * 4096));
    let (b, n) = ir::event_log((8u64 << 56) | 0xABCD_E000);
    assert_eq!((b, n), (0xABCD_E000, 256));
    assert_eq!(ir::device_id(1, 0, 0), 0x0100);
    assert_eq!(ir::device_id(0xC3, 0, 4), 0xC304);
}

#[test]
fn iommu_device_table_policies() {
    assert_eq!(ir::dte_policy(0), DmaPolicy::NotValid);
    assert_eq!(ir::dte_policy(1), DmaPolicy::NoTranslationInfo);
    assert_eq!(ir::dte_policy(3 | (1 << 61) | (1 << 62)), DmaPolicy::Passthrough { read: true, write: true });
    assert_eq!(ir::dte_policy(3), DmaPolicy::Passthrough { read: false, write: false });
    assert_eq!(
        ir::dte_policy(3 | (4 << 9) | 0x1234_5000 | (1 << 61)),
        DmaPolicy::Translated { mode: 4, root: 0x1234_5000, read: true, write: false }
    );
    assert_eq!(ir::dte_policy(3 | (7 << 9)), DmaPolicy::Reserved);
}

#[test]
fn iommu_event_decoding() {
    // IO_PAGE_FAULT (code 2) from 01:00.0 at the SBL buffer address.
    let e = ir::decode_event([0x0000_0100, 0x2000_0000 | (0x10 << 16), 0x01EE_3000, 0]);
    assert_eq!(e.code, 2);
    assert_eq!(e.device, 0x0100);
    assert_eq!(e.flags, 0x10);
    assert_eq!(e.address, 0x01EE_3000);
    assert_eq!(ir::event_name(2), "IO_PAGE_FAULT");
    assert_eq!(ir::event_name(0xF), "?");
}
