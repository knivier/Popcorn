//! No-panic sweep for the UDP layer, the DHCP client and the DNS resolver: random
//! and mutated (mostly checksum-repaired, so they reach the parsers) server replies
//! and UDP frames through `Iface::poll`, with DHCP and DNS transactions kept open
//! so replies can match. Checks the counter invariant, buffer accounting and that
//! everything transmitted is well-formed.

use crate::net::iface::{DhcpState, Iface, IpConfig, UDP_POOL};
use crate::net_wire::*;
use crate::util::csum;

const FRAMES: usize = 150_000;

const NAMES: [&str; 6] = ["example.com", "www.google.com", "a.b.c.d.e", "x", "fuzz-me.test", "cdn.example.net"];

fn mutate_bytes(rng: &mut Rng, m: &mut Vec<u8>) {
    for _ in 0..1 + rng.below(8) {
        if m.is_empty() {
            break;
        }
        let i = rng.below(m.len());
        match rng.below(5) {
            0 => m[i] = rng.byte(),
            1 => m[i] ^= 1 << rng.below(8),
            2 => m[i] = [0, 0xFF, 0xC0, 0xC0 | 0x0C, 0x3F, 0x40, 1, 5, 53, 255][rng.below(10)],
            3 => {
                // Plant a compression pointer somewhere.
                if i + 1 < m.len() {
                    m[i] = 0xC0 | rng.below(4) as u8;
                    m[i + 1] = rng.byte();
                }
            }
            _ => {}
        }
    }
    match rng.below(8) {
        0 => {
            let n = rng.below(m.len() + 1);
            m.truncate(n);
        }
        1 => {
            let extra = rng.below(40);
            for _ in 0..extra {
                m.push(rng.byte());
            }
        }
        _ => {}
    }
}

fn rand_ip(rng: &mut Rng) -> [u8; 4] {
    match rng.below(4) {
        0 => IP_A,
        1 => [10, 0, 2, rng.byte()],
        _ => [rng.byte(), rng.byte(), rng.byte(), rng.byte()],
    }
}

fn dhcp_seed(rng: &mut Rng, xid: u32) -> Vec<u8> {
    let ty = [2u8, 5, 6, 2, 5, 1, 3][rng.below(7)];
    let xid = if rng.below(8) == 0 { rng.next() as u32 } else { xid };
    let mut opts = slirp_opts(match rng.below(4) {
        0 => 0,
        1 => 10 + rng.below(100) as u32,
        2 => u32::MAX,
        _ => rng.next() as u32,
    });
    if rng.below(3) == 0 {
        opts.push((58, (rng.next() as u32).to_be_bytes().to_vec()));
        opts.push((59, (rng.next() as u32).to_be_bytes().to_vec()));
    }
    if rng.below(4) == 0 {
        opts[1] = (1, vec![rng.byte(), rng.byte(), rng.byte(), rng.byte()]);
    }
    if rng.below(4) == 0 {
        opts.retain(|_| rng.below(2) == 0);
    }
    let yi = if rng.below(2) == 0 { IP_A } else { rand_ip(rng) };
    dhcp_reply(ty, xid, MAC_A, yi, &opts)
}

fn dns_seed(rng: &mut Rng, id: u16, qname: &[u8]) -> Vec<u8> {
    let rcode = if rng.below(4) == 0 { rng.below(6) as u8 } else { 0 };
    let ip = [rng.byte(), rng.byte(), rng.byte(), rng.byte()];
    let ttl = rng.next() as u32;
    let answers = match rng.below(5) {
        0 => vec![],
        1 => vec![Rr::a(PTR_Q.to_vec(), ttl, ip)],
        2 => vec![
            Rr::cname(PTR_Q.to_vec(), ttl, dns_name("alias.example.net")),
            Rr::a(dns_name("alias.example.net"), 30, ip),
        ],
        3 => vec![
            Rr::a(dns_name("unrelated.org"), 30, ip),
            Rr::cname(PTR_Q.to_vec(), ttl, vec![1, b'z', 0xC0, 0x0C]),
            Rr::a(vec![1, b'z', 0xC0, 0x0C], 30, ip),
        ],
        _ => vec![Rr::cname(PTR_Q.to_vec(), ttl, PTR_Q.to_vec())],
    };
    let id = if rng.below(10) == 0 { rng.next() as u16 } else { id };
    dns_response(id, rcode, qname, &answers)
}

/// Wrap `payload` in a UDP frame: usually a correct checksum, sometimes none or
/// a wrong one; occasionally a corrupted UDP length.
#[allow(clippy::too_many_arguments)]
fn frame(rng: &mut Rng, eth_dst: [u8; 6], src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
    let mut seg = udp_seg(src, dst, sport, dport, payload, rng.below(8) != 0);
    match rng.below(16) {
        0 => seg[7] ^= 0x01, // bad checksum (unless it was zero)
        1 => {
            let l = rng.below(seg.len() + 16) as u16;
            seg[4..6].copy_from_slice(&l.to_be_bytes());
        }
        _ => {}
    }
    udp_frame_raw(eth_dst, MAC_GW, src, dst, &seg)
}

fn check_tx(f: &[u8], ips: &[[u8; 4]]) {
    assert!(f.len() >= 60 && f.len() <= 1514, "tx len {}", f.len());
    assert_eq!(&f[6..12], &MAC_A);
    match [f[12], f[13]] {
        [0x08, 0x06] => {}
        [0x08, 0x00] => {
            let ip = &f[14..];
            assert_eq!(csum::checksum(&ip[..20]), 0, "ip header checksum");
            let src = [ip[12], ip[13], ip[14], ip[15]];
            assert!(src == [0; 4] || ips.contains(&src), "tx src {src:?}");
            match ip[9] {
                1 => {
                    let total = u16::from_be_bytes([ip[2], ip[3]]) as usize;
                    assert_eq!(csum::checksum(&ip[20..total]), 0);
                }
                17 => {
                    let u = parse_udp_frame(f).expect("tx udp");
                    assert!(u.csum_ok, "tx udp checksum");
                    if u.dport == 67 {
                        let m = parse_dhcp(&u.payload).expect("tx dhcp");
                        assert_eq!((m.op, m.chaddr), (1, MAC_A));
                        assert!(matches!(m.msg_type(), Some(1) | Some(3)));
                    } else if u.dport == 53 {
                        let q = parse_dns_query(&u.payload).expect("tx dns");
                        assert_eq!((q.qdcount, q.qtype, q.qclass), (1, 1, 1));
                    }
                }
                p => panic!("tx proto {p}"),
            }
        }
        t => panic!("tx ethertype {t:?}"),
    }
}

#[test]
fn fuzz_udp_dhcp_dns_replies_never_panic() {
    let mut rng = Rng(0xD1CE_F00D_1234_5678);
    let mut dev = Wire::single(MAC_A);
    dev.record = false;
    let mut a = Iface::new(MAC_A, IpConfig::SLIRP, log);
    let echo = a.udp_echo(7).unwrap();
    let sock = a.udp_bind(7000).unwrap();
    let mut now = 1u64;
    a.dhcp_start(&mut dev, now, 1, None);
    // (source port, id, qname) of the latest DNS query seen on the wire.
    let mut query: Option<(u16, u16, Vec<u8>)> = None;
    let mut fed = 0usize;
    let mut tx_seen = 0usize;
    let mut buf = [0u8; 2048];
    let mut rounds = 0usize;
    while fed < FRAMES {
        rounds += 1;
        // Keep the resolver and gateway resolvable.
        if rounds % 50 == 1 {
            dev.inject(&arp_frame(MAC_A, 2, MAC_GW, IP_GW, MAC_A, IP_A));
            dev.inject(&arp_frame(MAC_A, 2, MAC_DNS, IP_DNS, MAC_A, IP_A));
            fed += 2;
        }
        // Keep transactions open so replies can match.
        if a.dns_in_flight() == 0 {
            // Mostly bypass the cache so queries reach the wire.
            if rng.below(8) != 0 {
                a.dns_cache_flush();
            }
            let name = NAMES[rng.below(NAMES.len())];
            let _ = a.dns_query(&mut dev, name, now, rng.next() as u32);
        }
        match a.dhcp_state() {
            DhcpState::Idle | DhcpState::Failed => a.dhcp_start(&mut dev, now, rng.next() as u32, None),
            DhcpState::Bound { .. } if rng.below(20) == 0 => a.dhcp_start(&mut dev, now, rng.next() as u32, None),
            _ => {}
        }
        if rng.below(300) == 0 {
            a.dns_cache_flush();
        }

        let n = (1 + rng.below(30)).min(FRAMES - fed);
        for _ in 0..n {
            let f = match rng.below(10) {
                0..=2 => {
                    let mut m = dhcp_seed(&mut rng, a.dhcp_xid());
                    if rng.below(3) != 0 {
                        mutate_bytes(&mut rng, &mut m);
                    }
                    let (ed, d) = if rng.below(2) == 0 { (BCAST_MAC, [255; 4]) } else { (MAC_A, rand_ip(&mut rng)) };
                    frame(&mut rng, ed, IP_GW, d, 67, 68, &m)
                }
                3..=5 => match &query {
                    Some((port, id, qname)) => {
                        let mut m = dns_seed(&mut rng, *id, qname);
                        if rng.below(3) != 0 {
                            mutate_bytes(&mut rng, &mut m);
                        }
                        let src = if rng.below(20) == 0 { IP_GW } else { IP_DNS };
                        let sp = if rng.below(20) == 0 { 54 } else { 53 };
                        frame(&mut rng, MAC_A, src, IP_A, sp, *port, &m)
                    }
                    None => frame(&mut rng, MAC_A, IP_DNS, IP_A, 53, 50000, &[0; 12]),
                },
                6 | 7 => {
                    let len = rng.below(1500);
                    let p: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
                    let port = [7u16, 7000, 68, 53, rng.next() as u16][rng.below(5)];
                    let dst = [IP_A, [255; 4], [10, 0, 2, 255], rand_ip(&mut rng)][rng.below(4)];
                    let src = if rng.below(10) == 0 { rand_ip(&mut rng) } else { IP_GW };
                    let sport = rng.next() as u16;
                    frame(&mut rng, MAC_A, src, dst, sport, port, &p)
                }
                _ => {
                    // Mutated whole frame, IPv4 header checksum repaired.
                    let p: Vec<u8> = (0..rng.below(64)).map(|_| rng.byte()).collect();
                    let mut f = udp_frame(MAC_A, MAC_GW, IP_GW, IP_A, 53, 7000, &p);
                    mutate_bytes(&mut rng, &mut f);
                    if f.len() >= 34 {
                        let ihl = (f[14] & 0x0F) as usize * 4;
                        if ihl >= 20 && 14 + ihl <= f.len() {
                            fix_ip_csum(&mut f[14..14 + ihl]);
                        }
                    }
                    f
                }
            };
            dev.inject(&f);
        }
        fed += n;
        now += rng.below(400) as u64;
        a.poll(&mut dev, now);
        let ips = [IP_A, a.config().ip];
        for f in dev.take_sent() {
            check_tx(&f, &ips);
            if let Some(u) = parse_udp_frame(&f) {
                if u.dport == 53 {
                    if let Some(q) = parse_dns_query(&u.payload) {
                        query = Some((u.sport, q.id, q.qname));
                    }
                }
            }
            tx_seen += 1;
        }
        // DHCP ACKs may install random configs; keep the address the replies use.
        if a.config() != IpConfig::SLIRP {
            a.set_config(IpConfig::SLIRP);
        }
        if rng.below(3) == 0 {
            while a.udp_recv(sock, &mut buf).is_some() {}
        }
        assert_eq!(dev.lent_count(), 0, "every RX buffer recycled");
        assert_eq!(dev.tx_busy_count(), 0, "every TX buffer submitted and flushed");
        assert!(a.udp_pool_used() <= UDP_POOL);
        assert!(a.pending_frames() <= 8);
    }
    while dev.rx_queued() > 0 {
        a.poll(&mut dev, now);
        dev.take_sent();
    }

    let s = a.stats();
    assert_eq!(s.rx_frames, fed as u64);
    assert_eq!(s.rx_frames, s.rx_delivered + s.rx_dropped(), "{s:#?}");
    assert!(s.rx_dhcp + s.rx_dns <= s.rx_udp);
    // No buffer leak: unbinding returns every datagram buffer.
    assert!(a.udp_unbind(sock));
    assert!(a.udp_unbind(echo));
    assert_eq!(a.udp_pool_used(), 0);
    // The sweep must reach the deep paths.
    assert!(s.rx_udp > 100_000, "{s:#?}");
    assert!(s.dhcp_offers > 100 && s.dhcp_acks > 10 && s.dhcp_naks > 10, "{s:#?}");
    assert!(s.dns_answers > 100 && s.dns_failures > 100, "{s:#?}");
    assert!(s.drop_dns_malformed > 100 && s.drop_dns_mismatch > 100 && s.drop_dhcp > 1000, "{s:#?}");
    assert!(s.drop_udp_csum > 1000 && s.drop_udp_no_socket > 1000 && s.tx_udp_echo > 1000, "{s:#?}");
    assert!(tx_seen > 1000);
    println!("proto fuzz: {fed} frames, {tx_seen} tx, stats {s:?}");
}

/// Pure-parser sweep: DNS response parsing and DHCP parsing on raw random and
/// mutated buffers (no stack around them), 200k each.
#[test]
fn fuzz_dns_and_dhcp_parsers_directly() {
    use crate::net::{dhcp, dns};
    let mut rng = Rng(0xABAD_1DEA_5EED_0001);
    let qname = dns::encode_name("www.example.com").unwrap();
    for _ in 0..200_000 {
        let mut m = if rng.below(4) == 0 {
            (0..rng.below(600)).map(|_| rng.byte()).collect()
        } else {
            dns_seed(&mut rng, 0x4242, qname.as_bytes())
        };
        mutate_bytes(&mut rng, &mut m);
        if let Ok(Ok((_, ttl))) = dns::parse_response(&m, 0x4242, &qname) {
            assert!(ttl <= 0x7FFF_FFFF);
        }
        let mut w = dns::WireName::EMPTY;
        let off = rng.below(m.len() + 2);
        if let Some(end) = dns::read_name(&m, off, &mut w) {
            assert!(end <= m.len());
            assert!(w.as_bytes().len() <= dns::MAX_WIRE_NAME);
        }
        let mut d = if rng.below(4) == 0 {
            (0..rng.below(700)).map(|_| rng.byte()).collect()
        } else {
            dhcp_seed(&mut rng, 7)
        };
        mutate_bytes(&mut rng, &mut d);
        let _ = dhcp::parse(&d);
    }
}
