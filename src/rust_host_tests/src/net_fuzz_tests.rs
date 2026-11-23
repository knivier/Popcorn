//! No-panic sweep: random and mutated frames through `Iface::poll`. Checks the
//! counter invariant, that the device gets every buffer back, and that everything
//! the stack transmits is well-formed.

use crate::net::iface::{Iface, IpConfig};
use crate::net::{arp, eth, icmp, ipv4};
use crate::net_wire::*;
use crate::util::csum;

const FRAMES: usize = 200_000;

fn seeds() -> Vec<Vec<u8>> {
    let bc = [0xFF; 6];
    vec![
        arp_frame(bc, 1, MAC_GW, IP_GW, [0; 6], IP_A),
        arp_frame(MAC_A, 2, MAC_GW, IP_GW, MAC_A, IP_A),
        arp_frame(bc, 1, [2, 0, 0, 0, 0, 9], [10, 0, 2, 9], [0; 6], IP_A),
        echo_frame(MAC_A, MAC_GW, IP_GW, IP_A, 8, 1, 1, &[0xAB; 56]),
        echo_frame(MAC_A, MAC_GW, [93, 184, 216, 34], IP_A, 8, 2, 2, &[1, 2, 3]),
        echo_frame(MAC_A, MAC_GW, IP_GW, IP_A, 0, 0x4000, 1, &[0; 16]),
        echo_frame(MAC_A, MAC_GW, IP_GW, IP_A, 8, 3, 3, &[0x77; 1472]),
    ]
}

/// Every transmitted frame must be a well-formed ARP or IPv4/ICMP frame from us.
fn check_tx(f: &[u8]) {
    assert!(f.len() >= 60 && f.len() <= 1514, "tx len {}", f.len());
    let (h, p) = eth::parse(f).expect("tx eth");
    assert_eq!(h.src, MAC_A);
    match h.ethertype {
        eth::ETHERTYPE_ARP => {
            let a = arp::parse(p).expect("tx arp");
            assert_eq!((a.sha, a.spa), (MAC_A, IP_A));
        }
        eth::ETHERTYPE_IPV4 => {
            let (ih, msg) = ipv4::parse(p).expect("tx ipv4");
            assert_eq!(ih.src, IP_A);
            assert_eq!(ih.proto, ipv4::PROTO_ICMP);
            assert_eq!(csum::checksum(msg), 0);
            let (ic, _) = icmp::parse(msg).expect("tx icmp");
            assert!(ic.ty == icmp::TYPE_ECHO_REPLY || ic.ty == icmp::TYPE_ECHO_REQUEST);
        }
        t => panic!("tx ethertype {t:#x}"),
    }
}

fn mutate(rng: &mut Rng, seeds: &[Vec<u8>]) -> Vec<u8> {
    match rng.below(10) {
        // Pure noise, random length 0..=1514.
        0 => (0..rng.below(1515)).map(|_| rng.byte()).collect(),
        // Noise behind a plausible Ethernet header.
        1 => {
            let ty = if rng.below(2) == 0 { 0x0800 } else { 0x0806 };
            let mut f = eth_hdr(MAC_A, MAC_GW, ty);
            let n = rng.below(1501);
            f.extend((0..n).map(|_| rng.byte()));
            f
        }
        _ => {
            let mut f = seeds[rng.below(seeds.len())].clone();
            for _ in 0..1 + rng.below(6) {
                if f.is_empty() {
                    break;
                }
                let i = rng.below(f.len());
                match rng.below(4) {
                    0 => f[i] = rng.byte(),
                    1 => f[i] ^= 1 << rng.below(8),
                    2 => f[i] = [0, 0xFF, 0x45, 0x40, 0x20, 4, 6][rng.below(7)],
                    _ => {}
                }
            }
            match rng.below(6) {
                0 => {
                    let n = rng.below(f.len() + 1);
                    f.truncate(n);
                }
                1 => {
                    let extra = rng.below(64);
                    let n = (f.len() + extra).min(1514);
                    f.resize(n, rng.byte());
                }
                _ => {}
            }
            // Often repair checksums so mutations reach the deeper layers.
            if f.len() >= 34 && f[12..14] == [0x08, 0x00] && rng.below(2) == 0 {
                let ihl = (f[14] & 0x0F) as usize * 4;
                if ihl >= 20 && 14 + ihl <= f.len() {
                    fix_ip_csum(&mut f[14..14 + ihl]);
                    let total = u16::from_be_bytes([f[16], f[17]]) as usize;
                    if rng.below(2) == 0 && total >= ihl + 4 && 14 + total <= f.len() {
                        let msg = &mut f[14 + ihl..14 + total];
                        msg[2] = 0;
                        msg[3] = 0;
                        let c = csum::checksum(msg);
                        msg[2..4].copy_from_slice(&c.to_be_bytes());
                    }
                }
            }
            f
        }
    }
}

#[test]
fn fuzz_random_and_mutated_frames_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let seeds = seeds();
    let mut dev = Wire::single(MAC_A);
    dev.record = false;
    let mut a = Iface::new(MAC_A, IpConfig::SLIRP, log);
    let mut now = 0u64;
    let mut fed = 0usize;
    let mut tx_seen = 0usize;
    while fed < FRAMES {
        let n = (1 + rng.below(40)).min(FRAMES - fed);
        for _ in 0..n {
            dev.inject(&mutate(&mut rng, &seeds));
        }
        fed += n;
        now += rng.below(400) as u64;
        // Occasionally originate traffic so the ARP/echo tables churn too.
        if rng.below(8) == 0 {
            let dst = match rng.below(3) {
                0 => IP_GW,
                1 => [10, 0, 2, rng.byte()],
                _ => [rng.byte(), rng.byte(), rng.byte(), rng.byte()],
            };
            let _ = a.ping_send(&mut dev, dst, rng.byte() as u16, rng.byte() as u16, rng.below(1600), now);
        }
        if rng.below(500) == 0 {
            a.arp_flush();
        }
        a.poll(&mut dev, now);
        for f in dev.take_sent() {
            check_tx(&f);
            tx_seen += 1;
        }
        assert_eq!(dev.lent_count(), 0, "every RX buffer recycled");
        assert_eq!(dev.tx_busy_count(), 0, "every TX buffer submitted and flushed");
    }
    // Drain anything left over by the per-poll cap.
    while dev.rx_queued() > 0 {
        a.poll(&mut dev, now);
        dev.take_sent();
    }

    let s = a.stats();
    assert_eq!(s.rx_frames, FRAMES as u64);
    assert_eq!(s.rx_frames, s.rx_delivered + s.rx_dropped(), "{s:#?}");
    assert!(s.rx_arp + s.rx_ipv4 <= s.rx_frames);
    assert!(s.rx_icmp <= s.rx_ipv4);
    assert!(s.rx_echo_requests + s.rx_echo_replies <= s.rx_icmp);
    assert!(s.tx_echo_replies <= s.rx_echo_requests);
    assert!(a.pending_frames() <= 8);
    // The sweep must actually reach every layer.
    assert!(s.rx_echo_requests > 1000, "{s:#?}");
    assert!(s.rx_arp_requests > 1000, "{s:#?}");
    assert!(s.drop_bad_csum > 0 && s.drop_fragment > 0 && s.drop_bad_len > 0 && s.drop_bad_header > 0);
    assert!(tx_seen > 1000);
    println!("fuzz: {FRAMES} frames, {tx_seen} tx, stats {s:?}");
}
