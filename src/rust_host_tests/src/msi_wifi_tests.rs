//! Plain MSI arithmetic, MHI state decoding and ath11k (WCN6855) register facts.

use crate::drivers::bus::mhi;
use crate::drivers::bus::msi_math::{self, Layout};
use crate::drivers::wifi::ath11k_hw::{self as hw, Access, Chip};

#[test]
fn msi_capable_and_enable_encodings() {
    // MMC field (bits 3:1): 2^n messages.
    assert_eq!(msi_math::capable_messages(0x0000), 1);
    assert_eq!(msi_math::capable_messages(0b101 << 1), 32);
    assert_eq!(msi_math::capable_messages(0b011 << 1), 8);
    // Reserved encodings 6/7 clamp to 32.
    assert_eq!(msi_math::capable_messages(0b111 << 1), 32);
    // MME field (bits 6:4).
    assert_eq!(msi_math::mme_bits(1), Some(0));
    assert_eq!(msi_math::mme_bits(2), Some(1 << 4));
    assert_eq!(msi_math::mme_bits(32), Some(5 << 4));
    for bad in [0u8, 3, 6, 12, 33, 64, 255] {
        assert_eq!(msi_math::mme_bits(bad), None, "{}", bad);
    }
}

#[test]
fn msi_register_layout_follows_64bit_and_masking_bits() {
    assert_eq!(
        msi_math::layout(0),
        Layout { addr_lo: 4, addr_hi: None, data: 8, mask: None, pending: None }
    );
    assert_eq!(
        msi_math::layout(msi_math::MC_64BIT),
        Layout { addr_lo: 4, addr_hi: Some(8), data: 12, mask: None, pending: None }
    );
    assert_eq!(
        msi_math::layout(msi_math::MC_PVM),
        Layout { addr_lo: 4, addr_hi: None, data: 8, mask: Some(12), pending: Some(16) }
    );
    assert_eq!(
        msi_math::layout(msi_math::MC_64BIT | msi_math::MC_PVM),
        Layout { addr_lo: 4, addr_hi: Some(8), data: 12, mask: Some(16), pending: Some(20) }
    );
}

#[test]
fn msi_x86_message() {
    assert_eq!(msi_math::x86_address(0), 0xFEE0_0000);
    assert_eq!(msi_math::x86_address(3), 0xFEE0_3000);
    assert_eq!(msi_math::x86_address(0x1FF), 0xFEEF_F000); // xAPIC: 8-bit ID only
    assert_eq!(msi_math::x86_data(0x41), 0x41);
}

#[test]
fn msi_vector_blocks_are_aligned_and_skip_used_vectors() {
    let free_all = |_v: u8| true;
    // Pool 0x40..=0xEF as in the kernel.
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 1, free_all), Some(0x40));
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 32, free_all), Some(0x40));
    assert_eq!(msi_math::find_aligned_block(0x41, 0xEF, 32, free_all), Some(0x60));
    assert_eq!(msi_math::find_aligned_block(0x43, 0xEF, 4, free_all), Some(0x44));
    // The kernel's syscall vector 0x80 sits inside 0x80..=0x9F, so a 32-block
    // that would cover it must be skipped.
    let not_80 = |v: u8| v != 0x80 && v != 0x81;
    let used_low = |v: u8| v >= 0x60 && not_80(v);
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 32, used_low), Some(0x60));
    let used_to_7f = |v: u8| v >= 0x80 && not_80(v);
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 32, used_to_7f), Some(0xA0));
    // Block must end inside the pool: 0xE0..=0xFF passes 0xEF.
    let only_high = |v: u8| v >= 0xE0;
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 32, only_high), None);
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 16, only_high), Some(0xE0));
    // One used vector in every 32-block: no 32-block, but 16 still fits.
    let holes = |v: u8| v % 32 != 5;
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 32, holes), None);
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 16, holes), Some(0x50));
    // Bad sizes.
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 3, free_all), None);
    assert_eq!(msi_math::find_aligned_block(0x40, 0xEF, 0, free_all), None);
    // Edge of u8: never wraps.
    assert_eq!(msi_math::find_aligned_block(0xF0, 0xFF, 32, free_all), None);
    assert_eq!(msi_math::find_aligned_block(0xF0, 0xFF, 16, free_all), Some(0xF0));
}

#[test]
fn ath11k_register_window() {
    assert_eq!(hw::access(0x224), Access::Direct(0x224));
    assert_eq!(hw::access(0x7FFFC), Access::Direct(0x7FFFC));
    // TCSR_SOC_HW_SUB_VER: window 0x32, BAR0 + 0x90010.
    assert_eq!(hw::access(hw::TCSR_SOC_HW_SUB_VER), Access::Windowed { window: 0x32, bar_off: 0x90010 });
    // LTSSM 0x1E081B0: window 0x3C, offset 0x81B0 inside it.
    assert_eq!(hw::access(hw::PCIE_PARF_LTSSM), Access::Windowed { window: 0x3C, bar_off: 0x80000 + 0x81B0 });
    // 0x80000 itself is window 1, offset 0.
    assert_eq!(hw::access(0x80000), Access::Windowed { window: 1, bar_off: 0x80000 });
    // Every windowed access stays inside BAR_NEEDED.
    for off in [0x80000u32, 0xFFFFC, 0x1F8_060C, 0x1E4_02BC, 0x1FF_FFFC] {
        if let Access::Windowed { window, bar_off } = hw::access(off) {
            assert!(bar_off + 4 <= hw::BAR_NEEDED);
            assert!(window < 64);
            assert_eq!(((window as usize) << 19) | (bar_off - hw::WINDOW_START), off as usize);
        } else {
            panic!("{:#x} should be windowed", off);
        }
    }
}

#[test]
fn ath11k_chip_revisions() {
    // hw2.0: major 2, minor 0 or 1.
    assert_eq!(hw::decode_version(0x0000_0200), (2, 0));
    assert_eq!(hw::chip(2, 0x00, None), Chip::Wcn6855Hw20);
    assert_eq!(hw::chip(2, 0x01, None), Chip::Wcn6855Hw20);
    assert!(!hw::needs_sub_version(2, 0x01));
    // hw2.1: minor 0x10/0x11, sub version picks the sibling chips.
    assert_eq!(hw::decode_version(0x0000_0211), (2, 0x11));
    assert!(hw::needs_sub_version(2, 0x10));
    assert_eq!(hw::chip(2, 0x10, Some(0x1234_5678)), Chip::Wcn6855Hw21);
    assert_eq!(hw::chip(2, 0x11, None), Chip::Wcn6855Hw21);
    assert_eq!(hw::chip(2, 0x10, Some(0x1019_B0E1)), Chip::Qca2066Hw21);
    assert_eq!(hw::chip(2, 0x11, Some(0x001E_60E1)), Chip::Qca6698aqHw21);
    // Anything else is refused, never guessed.
    assert_eq!(hw::chip(1, 0x00, None), Chip::Unsupported { major: 1, minor: 0 });
    assert_eq!(hw::chip(2, 0x20, None), Chip::Unsupported { major: 2, minor: 0x20 });
    assert!(!hw::chip(3, 0, None).supported());
    assert_eq!(hw::chip(2, 0x10, None).fw_dir(), Some("ath11k/WCN6855/hw2.1"));
    assert_eq!(hw::chip(2, 0x00, None).fw_dir(), Some("ath11k/WCN6855/hw2.0"));
    // All-ones (card not answering) decodes to an unsupported revision.
    let (ma, mi) = hw::decode_version(0xFFFF_FFFF);
    assert!(!hw::chip(ma, mi, None).supported());
}

#[test]
fn ath11k_msi_layouts_cover_every_message_once() {
    let mut seen = [0u8; 32];
    for u in hw::MSI_32 {
        for i in u.base..u.base + u.count {
            seen[i as usize] += 1;
        }
    }
    assert!(seen.iter().all(|&n| n == 1), "{:?}", seen);
    // One-message layout: everything except MHI's three event lines sits on 0.
    assert!(hw::MSI_ONE.iter().all(|u| u.base == 0));
}

#[test]
fn mhi_state_and_snapshot_decoding() {
    assert_eq!(mhi::state_of(0x0000_0200), 2);
    assert_eq!(mhi::state_name(0), "RESET");
    assert_eq!(mhi::state_name(0xFF), "SYS_ERR");
    assert!(mhi::ee_name(0).starts_with("PBL"));
    assert!(mhi::ee_name(2).starts_with("AMSS"));
    assert_eq!(mhi::bhi_status_name(0x8000_0000), "success");
    assert_eq!(mhi::bhi_status_name(0xC000_0000), "error");

    // A fake BAR: MHI at 0, BHI at 0x200, boot ROM waiting (EE 0), state READY.
    let bar_len = 0x1000usize;
    let mut regs = vec![0u32; bar_len / 4];
    regs[mhi::MHIVER / 4] = 0x0100_0000;
    regs[mhi::MHISTATUS / 4] = (0x1 << 8) | mhi::MHISTATUS_READY;
    regs[mhi::BHIOFF / 4] = 0x200;
    regs[(0x200 + mhi::BHI_EXECENV) / 4] = 0;
    regs[(0x200 + mhi::BHI_STATUS) / 4] = 0;
    let s = mhi::snapshot(bar_len, |o| regs[o / 4]);
    assert_eq!(s.state(), 1);
    assert!(s.ready() && !s.syserr() && !s.dead());
    assert_eq!(s.ee, Some(0));
    assert_eq!(s.bhi_status, Some(0));

    // BHIOFF pointing outside the BAR: no out-of-range read, EE unknown.
    regs[mhi::BHIOFF / 4] = 0x0FF0;
    let s = mhi::snapshot(bar_len, |o| {
        assert!(o + 4 <= bar_len, "read past the BAR at {:#x}", o);
        regs[o / 4]
    });
    assert_eq!(s.ee, None);
    // Unaligned BHIOFF is rejected too.
    regs[mhi::BHIOFF / 4] = 0x202;
    assert_eq!(mhi::snapshot(bar_len, |o| regs[o / 4]).ee, None);

    // A card that is not answering reads all-ones everywhere.
    let s = mhi::snapshot(bar_len, |_| 0xFFFF_FFFF);
    assert!(s.dead());
    assert_eq!(s.ee, None);
}
