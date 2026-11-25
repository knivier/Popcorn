//! Wi-Fi stage W2: QRTR packets, QMI encoding and the WLFW messages, the
//! board-2.bin container (against the real file when it has been fetched),
//! and the MHI command / transfer elements.

use crate::drivers::bus::mhi;
use crate::drivers::bus::qrtr::{self, Hdr};
use crate::drivers::wifi::board2;
use crate::drivers::wifi::qmi;

// ---------------------------------------------------------------- QRTR

#[test]
fn qrtr_hello_is_the_linux_wire_form() {
    let h = Hdr {
        kind: qrtr::TYPE_HELLO,
        src_node: qrtr::HOST_NODE,
        src_port: qrtr::PORT_CTRL,
        confirm_rx: false,
        size: 0,
        dst_node: qrtr::NODE_BCAST,
        dst_port: qrtr::PORT_CTRL,
    };
    let p = qrtr::encode(&h, &qrtr::ctrl(qrtr::TYPE_HELLO, [0; 4]));
    assert_eq!(p.len(), 52);
    assert_eq!(&p[0..4], &1u32.to_le_bytes()); // version 1
    assert_eq!(&p[4..8], &2u32.to_le_bytes()); // HELLO
    assert_eq!(&p[8..12], &1u32.to_le_bytes()); // src node 1
    assert_eq!(&p[12..16], &0xFFFF_FFFEu32.to_le_bytes()); // ctrl port
    assert_eq!(&p[20..24], &20u32.to_le_bytes()); // size
    assert_eq!(&p[24..28], &0xFFFF_FFFFu32.to_le_bytes());
    assert_eq!(&p[32..36], &2u32.to_le_bytes()); // cmd HELLO
    let (d, payload) = qrtr::decode(&p).unwrap();
    assert_eq!(d.kind, qrtr::TYPE_HELLO);
    assert_eq!(d.size, 20);
    assert_eq!(payload.len(), 20);
}

#[test]
fn qrtr_data_is_padded_and_round_trips() {
    let h = Hdr { kind: qrtr::TYPE_DATA, src_node: 1, src_port: 0x4000, confirm_rx: true, size: 0, dst_node: 3, dst_port: 0x1234 };
    let p = qrtr::encode(&h, b"hello");
    assert_eq!(p.len(), 32 + 8); // 5 bytes padded to 8
    assert_eq!(&p[37..40], &[0, 0, 0]);
    let (d, payload) = qrtr::decode(&p).unwrap();
    assert_eq!(payload, b"hello");
    assert!(d.confirm_rx);
    assert_eq!((d.src_port, d.dst_node, d.dst_port), (0x4000, 3, 0x1234));
    // Declared size past the buffer, short header, unknown version.
    let mut bad = p.clone();
    bad[20] = 200;
    assert!(qrtr::decode(&bad).is_err());
    assert!(qrtr::decode(&p[..20]).is_err());
    let mut v9 = p.clone();
    v9[0] = 9;
    assert!(qrtr::decode(&v9).is_err());
    assert!(qrtr::decode(&[]).is_err());
}

#[test]
fn qrtr_v2_and_control_payloads() {
    // v2: ver 3, type NEW_SERVER, flags 0, optlen 0, size 20, src 3:0xfffe(ctrl), dst 1:0xfffe.
    let mut p = vec![3u8, 4, 0, 0];
    p.extend_from_slice(&20u32.to_le_bytes());
    for v in [3u16, 0xFFFE, 1, 0xFFFE] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    p.extend_from_slice(&qrtr::ctrl(qrtr::TYPE_NEW_SERVER, [0x45, 0x101, 3, 0x22]));
    let (h, payload) = qrtr::decode(&p).unwrap();
    assert_eq!((h.kind, h.src_node, h.src_port, h.dst_port), (4, 3, qrtr::PORT_CTRL, qrtr::PORT_CTRL));
    let (cmd, w) = qrtr::parse_ctrl(payload).unwrap();
    assert_eq!(cmd, 4);
    assert_eq!(w, [0x45, 0x101, 3, 0x22]);
    assert_eq!(qrtr::service_instance(1, 1), 0x101);
    assert_eq!(qrtr::parse_ctrl(&[1, 2]), None);
}

// ---------------------------------------------------------------- QMI

#[test]
fn qmi_header_and_tlvs() {
    let m = qmi::Writer::new(qmi::REQUEST, 1, qmi::HOST_CAP).u32(0x10, 1).u8(0x13, 0x1E).finish();
    // 00 | 01 00 | 34 00 | len | 10 04 00 01 00 00 00 | 13 01 00 1e
    assert_eq!(&m[..5], &[0x00, 0x01, 0x00, 0x34, 0x00]);
    assert_eq!(u16::from_le_bytes([m[5], m[6]]) as usize, m.len() - 7);
    assert_eq!(&m[7..14], &[0x10, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00]);
    assert_eq!(&m[14..], &[0x13, 0x01, 0x00, 0x1E]);
    let p = qmi::parse(&m).unwrap();
    assert_eq!((p.kind, p.txn, p.id), (qmi::REQUEST, 1, qmi::HOST_CAP));
    assert_eq!(qmi::tlv(p.body, 0x13), Some(&[0x1Eu8][..]));
    assert_eq!(qmi::tlv(p.body, 0x99), None);
    // Truncated TLV length stops the walk instead of over-reading.
    assert_eq!(qmi::tlv(&[0x10, 0xFF, 0x00, 0x01], 0x10), None);
    assert!(qmi::parse(&m[..6]).is_err());
}

#[test]
fn qmi_response_result() {
    let ok = qmi::Writer::new(qmi::RESPONSE, 2, qmi::CAP).tlv(0x02, &[0, 0, 0, 0]).finish();
    assert_eq!(qmi::result(qmi::parse(&ok).unwrap().body), Some((0, 0)));
    let fail = qmi::Writer::new(qmi::RESPONSE, 2, qmi::CAP).tlv(0x02, &[1, 0, 94, 0]).finish();
    assert_eq!(qmi::result(qmi::parse(&fail).unwrap().body), Some((1, 94)));
    assert_eq!(qmi::result(&[]), None);
}

#[test]
fn qmi_wlfw_requests_carry_athk_values() {
    let r = qmi::ind_register_req(7);
    let b = qmi::parse(&r).unwrap().body;
    assert_eq!(qmi::tlv(b, 0x15), Some(&0x4b4e454cu32.to_le_bytes()[..]));
    for t in [0x10u8, 0x16, 0x17, 0x18, 0x1B] {
        assert_eq!(qmi::tlv(b, t), Some(&[1u8][..]), "tlv {:#x}", t);
    }
    let h = qmi::host_cap_req(8, false);
    let b = qmi::parse(&h).unwrap().body;
    assert_eq!(qmi::tlv(b, 0x13), Some(&[0x1Eu8][..]));
    assert_eq!(qmi::tlv(b, 0x1C), Some(&[0u8][..]));
    let m3 = qmi::m3_info_req(9, 0x1234_5000, 266_684);
    let b = qmi::parse(&m3).unwrap().body;
    assert_eq!(qmi::tlv(b, 0x01), Some(&0x1234_5000u64.to_le_bytes()[..]));
    assert_eq!(qmi::tlv(b, 0x02), Some(&266_684u32.to_le_bytes()[..]));
    assert_eq!(qmi::parse(&qmi::cap_req(3)).unwrap().body.len(), 0);
}

#[test]
fn qmi_memory_request_and_response() {
    // Two segments: 0x40000 type 1 (no cfg), 0x80000 type 4 (one cfg entry).
    let mut v = vec![2u8];
    v.extend_from_slice(&0x40000u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.push(0);
    v.extend_from_slice(&0x80000u32.to_le_bytes());
    v.extend_from_slice(&4u32.to_le_bytes());
    v.push(1);
    v.extend_from_slice(&[0u8; 13]);
    let ind = qmi::Writer::new(qmi::INDICATION, 0, qmi::REQUEST_MEM_IND).tlv(0x01, &v).finish();
    let segs = qmi::request_mem_ind(qmi::parse(&ind).unwrap().body).unwrap();
    assert_eq!(segs, vec![qmi::MemSeg { size: 0x40000, kind: 1 }, qmi::MemSeg { size: 0x80000, kind: 4 }]);
    // Truncated and absurd inputs are refused.
    let short = qmi::Writer::new(qmi::INDICATION, 0, qmi::REQUEST_MEM_IND).tlv(0x01, &v[..10]).finish();
    assert!(qmi::request_mem_ind(qmi::parse(&short).unwrap().body).is_err());
    let many = qmi::Writer::new(qmi::INDICATION, 0, qmi::REQUEST_MEM_IND).tlv(0x01, &[200]).finish();
    assert!(qmi::request_mem_ind(qmi::parse(&many).unwrap().body).is_err());
    // Response: count, then 17-byte entries.
    let r = qmi::respond_mem_req(5, &[(0x1000_0000, 0x40000, 1), (0x2000_0000, 0x80000, 4)]);
    let t = qmi::tlv(qmi::parse(&r).unwrap().body, 0x01).unwrap();
    assert_eq!(t.len(), 1 + 2 * 17);
    assert_eq!(t[0], 2);
    assert_eq!(&t[1..9], &0x1000_0000u64.to_le_bytes());
    assert_eq!(&t[9..13], &0x40000u32.to_le_bytes());
    assert_eq!(&t[13..17], &1u32.to_le_bytes());
    assert_eq!(t[17], 0); // restore
}

#[test]
fn qmi_capability_response() {
    let mut fwv = 0x0123_4567u32.to_le_bytes().to_vec();
    let ts = b"2023-01-01 12:00";
    fwv.push(ts.len() as u8);
    fwv.extend_from_slice(ts);
    let resp = qmi::Writer::new(qmi::RESPONSE, 4, qmi::CAP)
        .tlv(0x02, &[0, 0, 0, 0])
        .tlv(0x10, &[2, 0, 0, 0, 0x81, 0x40, 0, 0])
        .u32(0x11, 0xFF)
        .u32(0x12, 0x400c_1211)
        .tlv(0x13, &fwv)
        .tlv(0x14, b"WLAN.HSP.1.1-03125-QCAHSPSWPL_V1_V2_SILICONZ_LITE-3.6510.41")
        .u8(0x15, 2)
        .u32(0x19, 2000)
        .finish();
    let c = qmi::cap_resp(qmi::parse(&resp).unwrap().body);
    assert_eq!(c.chip_id, Some(2));
    assert_eq!(c.chip_family, Some(0x4081));
    assert_eq!(c.board_id, Some(255));
    assert_eq!(c.soc_id, Some(0x400c_1211));
    assert_eq!(c.fw_version, Some(0x0123_4567));
    assert_eq!(c.fw_build_timestamp, "2023-01-01 12:00");
    assert!(c.fw_build_id.starts_with("WLAN.HSP.1.1-03125"));
    assert_eq!(c.num_macs, Some(2));
    assert_eq!(c.eeprom_read_timeout, Some(2000));
    // Empty response: nothing known, nothing panics.
    assert_eq!(qmi::cap_resp(&[]), qmi::Cap::default());
}

#[test]
fn qmi_bdf_chunks() {
    let data = vec![0xAB; 100];
    let r = qmi::bdf_req(6, 255, 100, 3, &data, true, qmi::BDF_TYPE_ELF);
    let b = qmi::parse(&r).unwrap().body;
    assert_eq!(qmi::tlv(b, 0x01), Some(&[1u8][..]));
    assert_eq!(qmi::tlv(b, 0x10), Some(&255u32.to_le_bytes()[..]));
    assert_eq!(qmi::tlv(b, 0x11), Some(&100u32.to_le_bytes()[..]));
    assert_eq!(qmi::tlv(b, 0x12), Some(&3u32.to_le_bytes()[..]));
    let d = qmi::tlv(b, 0x13).unwrap();
    assert_eq!(u16::from_le_bytes([d[0], d[1]]), 100);
    assert_eq!(&d[2..], &data[..]);
    assert_eq!(qmi::tlv(b, 0x14), Some(&[1u8][..]));
    assert_eq!(qmi::tlv(b, 0x15), Some(&[1u8][..]));
    // Largest chunk fits the 8 KiB QRTR buffer with headers.
    let big = qmi::bdf_req(1, 255, 6144, 0, &vec![0; qmi::BDF_CHUNK], false, 0);
    assert!(big.len() + qrtr::HDR_V1 + 3 <= 8192);
    assert_eq!(qmi::bdf_type_of(b"\x7fELF...."), qmi::BDF_TYPE_ELF);
    assert_eq!(qmi::bdf_type_of(b"LW\x04\x00"), qmi::BDF_TYPE_BIN);
}

// ---------------------------------------------------------------- board-2.bin

fn synthetic_board2() -> Vec<u8> {
    fn ie(id: u32, data: &[u8]) -> Vec<u8> {
        let mut v = id.to_le_bytes().to_vec();
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        while v.len() % 4 != 0 {
            v.push(0);
        }
        v
    }
    let mut f = b"QCA-ATH11K-BOARD\0\0\0\0".to_vec();
    let mut b1 = ie(0, b"bus=pci,qmi-chip-id=2");
    b1.extend(ie(1, b"BOARD-A"));
    f.extend(ie(0, &b1));
    let mut b2 = ie(0, b"bus=pci,vendor=17cb,device=1103,subsystem-vendor=17aa,subsystem-device=9309,qmi-chip-id=2,qmi-board-id=255");
    b2.extend(ie(0, b"alias-name"));
    b2.extend(ie(1, b"BOARD-B!"));
    f.extend(ie(0, &b2));
    let mut r = ie(0, b"bus=pci");
    r.extend(ie(1, b"REGDB"));
    f.extend(ie(1, &r));
    f
}

#[test]
fn board2_lookup_by_exact_name() {
    let f = synthetic_board2();
    let names = board2::board_names(0x17cb, 0x1103, 0x17aa, 0x9309, 2, 255);
    assert_eq!(board2::find(&f, board2::IE_BOARD, &names[0]), Ok(&b"BOARD-B!"[..]));
    assert_eq!(board2::find(&f, board2::IE_BOARD, "alias-name"), Ok(&b"BOARD-B!"[..]));
    assert_eq!(board2::find(&f, board2::IE_BOARD, &names[1]), Ok(&b"BOARD-A"[..]));
    // Prefixes and other kinds do not match.
    assert!(board2::find(&f, board2::IE_BOARD, "bus=pci,qmi-chip-id").is_err());
    assert!(board2::find(&f, board2::IE_BOARD, "bus=pci").is_err());
    assert_eq!(board2::find(&f, board2::IE_REGDB, "bus=pci"), Ok(&b"REGDB"[..]));
    assert!(board2::find(b"nope", board2::IE_BOARD, "x").is_err());
    // A truncated file errors instead of over-reading.
    assert!(board2::find(&f[..f.len() - 3], board2::IE_REGDB, "bus=pci").is_err());
    assert_eq!(names[1], "bus=pci,qmi-chip-id=2");
}

#[test]
fn board2_real_file_has_the_t14s_entries() {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/firmware/wcn6855/BOARD-2.BIN");
    let Ok(f) = std::fs::read(&p) else {
        eprintln!("skipped: {} not fetched (core.sh firmware)", p.display());
        return;
    };
    let all = board2::names(&f, board2::IE_BOARD);
    assert!(all.len() > 50);
    for (dev, chip) in [(0x9309u16, 2u32), (0x9309, 18), (0x9308, 2)] {
        let n = board2::board_names(0x17cb, 0x1103, 0x17aa, dev, chip, 255);
        let d = board2::find(&f, board2::IE_BOARD, &n[0]).unwrap();
        assert!(d.starts_with(b"\x7fELF"), "{}", n[0]);
        assert!(d.len() <= qmi::BOARD_MAX);
    }
    let r = board2::find(&f, board2::IE_REGDB, "bus=pci").unwrap();
    assert_eq!(r.len(), 24310);
}

// ---------------------------------------------------------------- MHI elements

#[test]
fn mhi_command_and_transfer_elements() {
    assert_eq!(mhi::cmd_el(mhi::PKT_START_CHAN, 20), [0, 0, 0, 0x1412_0000]);
    assert_eq!(mhi::cmd_el(mhi::PKT_START_CHAN, 21), [0, 0, 0, 0x1512_0000]);
    assert_eq!(mhi::cmd_el(mhi::PKT_RESET_CHAN, 20), [0, 0, 0, 0x1410_0000]);
    let t = mhi::transfer_el(0x1_2345_6000, 8192, true);
    assert_eq!(t, [0x2345_6000, 0x1, 8192, 0x0002_0200]);
    assert_eq!(mhi::transfer_el(0x1000, 0x1_0005, false)[2], 5); // length is 16 bits
}

// ---------------------------------------------------------------- firmware start (W3a)

#[test]
fn qmi_firmware_start_requests() {
    let ini = qmi::wlan_ini_req(1, true);
    let b = qmi::parse(&ini).unwrap();
    assert_eq!(b.id, qmi::WLAN_INI);
    assert_eq!(qmi::tlv(b.body, 0x10), Some(&[1u8][..]));

    let mode = qmi::wlan_mode_req(2, qmi::FIRMWARE_MODE_NORMAL);
    let b = qmi::parse(&mode).unwrap();
    assert_eq!(qmi::tlv(b.body, 0x01), Some(&0u32.to_le_bytes()[..]));
    assert_eq!(qmi::tlv(b.body, 0x10), Some(&[0u8][..]));
    assert!(mode.len() <= 7 + 11);

    let cfg = qmi::wlan_cfg_req(3);
    assert!(cfg.len() <= 7 + 803, "{} bytes", cfg.len());
    let b = qmi::parse(&cfg).unwrap();
    assert_eq!(b.id, qmi::WLAN_CFG);
    assert_eq!(qmi::tlv(b.body, 0x10), Some(&b"WIN"[..]));
    let ce = qmi::tlv(b.body, 0x11).unwrap();
    assert_eq!(ce.len(), 1 + 9 * 20);
    assert_eq!(ce[0], 9);
    // Row 4: pipe 4, out, 256 entries, 256 bytes, no interrupts.
    let row = |i: usize, k: usize| u32::from_le_bytes(ce[1 + i * 20 + k * 4..5 + i * 20 + k * 4].try_into().unwrap());
    assert_eq!([row(4, 0), row(4, 1), row(4, 2), row(4, 3), row(4, 4)], [4, 2, 256, 256, 8]);
    assert_eq!([row(6, 0), row(6, 1), row(6, 3)], [6, 3, 16384]);
    let svc = qmi::tlv(b.body, 0x12).unwrap();
    assert_eq!(svc.len(), 1 + 14 * 12);
    assert_eq!(svc[0], 14);
    // WMI control: host->target on pipe 3, target->host on pipe 2.
    let srow = |i: usize, k: usize| u32::from_le_bytes(svc[1 + i * 12 + k * 4..5 + i * 12 + k * 4].try_into().unwrap());
    assert_eq!([srow(8, 0), srow(8, 1), srow(8, 2)], [0x100, 2, 3]);
    assert_eq!([srow(9, 0), srow(9, 1), srow(9, 2)], [0x100, 1, 2]);
    let sh = qmi::tlv(b.body, 0x14).unwrap();
    assert_eq!(sh.len(), 1 + 30 * 4);
    assert_eq!(sh[0], 30);
    let first = u32::from_le_bytes(sh[1..5].try_into().unwrap());
    let ce0 = u32::from_le_bytes(sh[1 + 20 * 4..5 + 20 * 4].try_into().unwrap());
    assert_eq!(first, 0x00a3_b034);
    assert_eq!(ce0, 0x01b8_0400);
    assert_eq!(qmi::tlv(b.body, 0x13), None); // no v1 shadow list
    // Every copy-engine pipe the service map names exists in the CE table.
    for r in qmi::SVC_MAP {
        assert!(qmi::TARGET_CE.iter().any(|c| c[0] == r[2]));
    }
}

// ---------------------------------------------------------------- copy engines (W3b)

use crate::drivers::wifi::ath11k_hw as hw;

#[test]
fn ce_register_blocks_match_ath11k() {
    // CE0 source block, CE2 destination and status blocks, CE3 source block.
    assert_eq!(hw::ce_blocks(0), (0x01b8_0000, 0x01b8_1000, 0x01b8_1058));
    assert_eq!(hw::ce_blocks(2), (0x01b8_4000, 0x01b8_5000, 0x01b8_5058));
    assert_eq!(hw::ce_blocks(3).0, 0x01b8_6000);
    // R2 pointer registers.
    assert_eq!(hw::ce_blocks(3).0 + hw::CE_R2, 0x01b8_6400);
    assert_eq!(hw::CE_DST_BASE + 2 * hw::CE_STRIDE + hw::CE_STATUS_R2, 0x01b8_5408);
    // Ring ids.
    assert_eq!(hw::RING_ID_CE_SRC + 3, 35);
    assert_eq!(hw::RING_ID_CE_DST + 2, 58);
    assert_eq!(hw::RING_ID_CE_STATUS + 2, 82);
}

#[test]
fn ce_shadow_slots_follow_the_wlan_cfg_list() {
    use crate::drivers::wifi::qmi::SHADOW_V2;
    // Each ring's shadow target, as the host computes it, and the slot address.
    let cases = [
        (0x01b8_0400u32, 20usize, 0x94cu32), // CE0 SRC (HP)
        (0x01b8_3400, 21, 0x950),            // CE1 DST (HP: source-type ring)
        (0x01b8_340c, 22, 0x954),            // CE1 STATUS (TP)
        (0x01b8_5400, 23, 0x958),            // CE2 DST
        (0x01b8_540c, 24, 0x95c),            // CE2 STATUS
        (0x01b8_6400, 25, 0x960),            // CE3 SRC
    ];
    for (target, slot, bar) in cases {
        assert_eq!(hw::shadow_index(&SHADOW_V2, target), Some(slot), "{:#x}", target);
        assert_eq!(hw::shadow_slot(slot), bar);
    }
    assert_eq!(hw::shadow_index(&SHADOW_V2, 0x1234), None);
    // CE2 status target computed the way the CE layer does it.
    assert_eq!(hw::CE_DST_BASE + hw::CE_STATUS_R2 + 2 * hw::CE_STRIDE + 4, 0x01b8_540c);
}

#[test]
fn ce_descriptors_and_ring_fields() {
    assert_eq!(hw::ring_base_msb(0x1_2345_6000, 128), 0x01 | (128 << 8));
    assert_eq!(hw::ring_base_msb(0x2000, 0x10_0000) >> 8, 0); // size field is 20 bits
    assert_eq!(hw::ce_src_desc(0x1_0000_1000, 300, 1), [0x0000_1000, 0x01 | (300 << 16), 1, 0]);
    assert_eq!(hw::ce_dst_desc(0x3_0000_2000), [0x0000_2000, 0x03]);
    assert_eq!(hw::ce_status_len(0x0040_0000 | 0x123), 0x40);
}

#[test]
fn htc_header_and_ready() {
    // eid 0, flags 0, payload 8 | ctrl; READY: id 1, credits 2, size 2048, 9 endpoints.
    let mut m = Vec::new();
    m.extend_from_slice(&(0u32 | (8 << 16)).to_le_bytes());
    m.extend_from_slice(&0u32.to_le_bytes());
    m.extend_from_slice(&(1u32 | (2 << 16)).to_le_bytes());
    m.extend_from_slice(&(2048u32 | (9 << 16)).to_le_bytes());
    let h = hw::htc_hdr(&m).unwrap();
    assert_eq!((h.eid, h.flags, h.len), (0, 0, 8));
    assert_eq!(hw::htc_ready(&m[hw::HTC_HDR..]), Some((2, 2048, 9)));
    assert_eq!(hw::htc_ready(&m[hw::HTC_HDR..hw::HTC_HDR + 4]), None);
    let mut not_ready = m.clone();
    not_ready[8] = 3;
    assert_eq!(hw::htc_ready(&not_ready[hw::HTC_HDR..]), None);
    assert_eq!(hw::htc_hdr(&m[..7]), None);
}
