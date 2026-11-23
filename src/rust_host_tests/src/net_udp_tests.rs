//! UDP: checksum against a byte-wise reference, socket send/receive between two
//! ifaces, echo service, unbound ports, ring limits, broadcast acceptance.

use crate::net::iface::{Iface, IpConfig, UDP_MAX_PAYLOAD, UDP_POOL, UDP_RING};
use crate::net::{udp, NetError};
use crate::net_wire::*;

const IP_B: [u8; 4] = [10, 0, 2, 16];

fn pair() -> (Wire, Wire, Iface, Iface) {
    let (wa, wb) = Wire::pair(MAC_A, MAC_B);
    let a = Iface::new(MAC_A, IpConfig::SLIRP, log);
    let b = Iface::new(MAC_B, IpConfig { ip: IP_B, ..IpConfig::SLIRP }, log);
    (wa, wb, a, b)
}

/// Poll both sides a few rounds (enough for ARP + data + reply).
fn pump(wa: &mut Wire, wb: &mut Wire, a: &mut Iface, b: &mut Iface, now: &mut u64) {
    for _ in 0..4 {
        *now += 1;
        b.poll(wb, *now);
        *now += 1;
        a.poll(wa, *now);
    }
}

fn invariant(i: &Iface) {
    let s = i.stats();
    assert_eq!(s.rx_frames, s.rx_delivered + s.rx_dropped(), "{s:#?}");
}

/// `Iface` lives in a static but is built on a 128 KiB kernel stack.
#[test]
fn iface_size_is_bounded() {
    let n = core::mem::size_of::<Iface>();
    println!("size_of::<Iface>() = {n}");
    assert!(n < 40 * 1024, "Iface is {n} bytes");
}

#[test]
fn udp_checksum_matches_reference() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    let mut buf = vec![0u8; 2000];
    for len in 0..=700usize {
        let payload: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let src = [rng.byte(), rng.byte(), rng.byte(), rng.byte()];
        let dst = [rng.byte(), rng.byte(), rng.byte(), rng.byte()];
        let (sp, dp) = (rng.next() as u16, rng.next() as u16);
        let n = udp::write(&mut buf, src, dst, sp, dp, &payload).unwrap();
        assert_eq!(n, 8 + len);
        let want = udp_seg(src, dst, sp, dp, &payload, true);
        assert_eq!(&buf[..n], &want[..], "len {len}");
        // Parse accepts it, returns the payload; a flipped bit is rejected.
        let (h, p) = udp::parse(src, dst, &buf[..n]).unwrap();
        assert_eq!((h.src_port, h.dst_port, h.len), (sp, dp, n));
        assert_eq!(p, &payload[..]);
        let i = rng.below(n);
        buf[i] ^= 1 << rng.below(8);
        let r = udp::parse(src, dst, &buf[..n]);
        // A flip in ports/payload/checksum must be caught (unless it zeroed the
        // checksum field, which means "none"); a flipped length byte is skipped
        // here (BadLen or a checksum over a different span).
        let csum_zeroed = buf[6] == 0 && buf[7] == 0;
        if i != 4 && i != 5 && !csum_zeroed {
            assert!(r.is_err(), "len {len} flip {i}");
        }
        // Wrong pseudo header (other destination) is rejected too.
        let n = udp::write(&mut buf, src, dst, sp, dp, &payload).unwrap();
        let other = [dst[0], dst[1], dst[2], dst[3].wrapping_add(1)];
        assert_eq!(udp::parse(src, other, &buf[..n]).err(), Some(udp::UdpError::BadCsum));
    }

    // Zero checksum = none: accepted unverified, even with a "wrong" payload.
    let seg = udp_seg(IP_GW, IP_A, 1, 2, b"odd", false);
    assert_eq!(&seg[6..8], &[0, 0]);
    let (_, p) = udp::parse(IP_GW, IP_A, &seg).unwrap();
    assert_eq!(p, b"odd");
    let (_, p) = udp::parse([1, 2, 3, 4], IP_A, &seg).unwrap();
    assert_eq!(p, b"odd");

    // A computed checksum of 0 is transmitted as 0xFFFF (and still verifies).
    let mut payload = vec![0x11, 0x22, 0x33, 0x44, 0, 0];
    let s = ref_csum(&[&udp_pseudo(IP_GW, IP_A, 14), &udp_seg(IP_GW, IP_A, 9, 9, &payload, false)]);
    // ref_csum is the inverted sum; set the last word to the folded sum's complement.
    let sum = !s;
    payload[4..6].copy_from_slice(&(!sum).to_be_bytes());
    let n = udp::write(&mut buf, IP_GW, IP_A, 9, 9, &payload).unwrap();
    assert_eq!(&buf[6..8], &[0xFF, 0xFF], "computed zero sent as 0xFFFF");
    assert!(udp::parse(IP_GW, IP_A, &buf[..n]).is_ok());

    // Length field checks.
    let mut seg = udp_seg(IP_GW, IP_A, 1, 2, b"abcd", true);
    seg[4..6].copy_from_slice(&7u16.to_be_bytes());
    assert_eq!(udp::parse(IP_GW, IP_A, &seg).err(), Some(udp::UdpError::BadLen));
    seg[4..6].copy_from_slice(&13u16.to_be_bytes());
    assert_eq!(udp::parse(IP_GW, IP_A, &seg).err(), Some(udp::UdpError::BadLen));
    assert_eq!(udp::parse(IP_GW, IP_A, &seg[..7]).err(), Some(udp::UdpError::Short));
    // Trailing bytes beyond the UDP length are ignored (checksum over len only).
    let mut seg = udp_seg(IP_GW, IP_A, 1, 2, b"abcd", true);
    seg.extend_from_slice(&[0xEE; 5]);
    assert_eq!(udp::parse(IP_GW, IP_A, &seg).unwrap().1, b"abcd");
}

#[test]
fn udp_send_to_bound_socket_between_ifaces() {
    let (mut wa, mut wb, mut a, mut b) = pair();
    let mut now = 100;
    let sb = b.udp_bind(5000).unwrap();
    assert_eq!(b.udp_port(sb), Some(5000));
    for (k, len) in [0usize, 1, 17, 512, UDP_MAX_PAYLOAD].into_iter().enumerate() {
        let payload: Vec<u8> = (0..len).map(|i| (i * 7 + k) as u8).collect();
        a.udp_send_to(&mut wa, 40000, IP_B, 5000, &payload, now).unwrap();
        pump(&mut wa, &mut wb, &mut a, &mut b, &mut now);
        let mut buf = vec![0u8; 2000];
        let (src, sport, n) = b.udp_recv(sb, &mut buf).expect("datagram");
        assert_eq!((src, sport, n), (IP_A, 40000, len));
        assert_eq!(&buf[..n], &payload[..]);
        assert!(b.udp_recv(sb, &mut buf).is_none());
    }
    // Too big for one packet.
    assert_eq!(a.udp_send_to(&mut wa, 1, IP_B, 5000, &vec![0; UDP_MAX_PAYLOAD + 1], now), Err(NetError::TooBig));
    // Truncating receive.
    a.udp_send_to(&mut wa, 40000, IP_B, 5000, b"0123456789", now).unwrap();
    pump(&mut wa, &mut wb, &mut a, &mut b, &mut now);
    let mut small = [0u8; 4];
    assert_eq!(b.udp_recv(sb, &mut small), Some((IP_A, 40000, 4)));
    assert_eq!(&small, b"0123");

    // Every datagram on the wire: valid checksum, TTL 64, ARP happened once.
    let mut seen = 0;
    for f in &wa.history {
        if let Some(u) = parse_udp_frame(f) {
            assert!(u.csum_ok && u.csum != 0, "tx checksum always generated");
            assert_eq!((u.src, u.dst, u.ttl, u.eth_dst), (IP_A, IP_B, 64, MAC_B));
            seen += 1;
        }
    }
    assert_eq!(seen, 6);
    let (sa, sb_) = (a.stats(), b.stats());
    assert_eq!(sa.tx_arp_requests, 1);
    assert_eq!(sa.tx_udp, 6);
    assert_eq!(sb_.rx_udp, 6);
    assert_eq!(sa.rx_dropped() + sb_.rx_dropped(), 0);
    invariant(&a);
    invariant(&b);
    assert_eq!(b.udp_pool_used(), 0);
    assert_eq!(wa.lent_count() + wb.lent_count(), 0);
}

#[test]
fn udp_echo_service_round_trip() {
    let (mut wa, mut wb, mut a, mut b) = pair();
    let mut now = 0;
    b.udp_echo(7).unwrap();
    let sa = a.udp_bind(0).unwrap();
    let port = a.udp_port(sa).unwrap();
    assert!(port >= 49152, "ephemeral {port}");
    for msg in [&b"hello"[..], &b""[..], &b"x"[..], &[0xA5; 1000][..]] {
        a.udp_send(&mut wa, sa, IP_B, 7, msg, now).unwrap();
        pump(&mut wa, &mut wb, &mut a, &mut b, &mut now);
        let mut buf = [0u8; 1500];
        let (src, sport, n) = a.udp_recv(sa, &mut buf).expect("echo reply");
        assert_eq!((src, sport), (IP_B, 7));
        assert_eq!(&buf[..n], msg);
    }
    let s = b.stats();
    assert_eq!((s.rx_udp, s.tx_udp_echo, s.tx_udp), (4, 4, 4));
    assert_eq!(b.udp_pool_used(), 0, "echo never queues");
    invariant(&a);
    invariant(&b);
}

#[test]
fn udp_unbound_port_and_bad_checksum_counted() {
    let mut dev = Wire::single(MAC_A);
    let mut a = Iface::new(MAC_A, IpConfig::SLIRP, log);
    dev.inject(&udp_frame(MAC_A, MAC_GW, IP_GW, IP_A, 1234, 9999, b"nobody home"));
    a.poll(&mut dev, 1);
    let s = a.stats();
    assert_eq!((s.rx_udp, s.drop_udp_no_socket, s.drop_unknown_proto), (1, 1, 0));
    assert!(dev.take_sent().is_empty(), "no ICMP port unreachable");

    let sock = a.udp_bind(9999).unwrap();
    // Corrupted payload with a non-zero checksum: dropped.
    let mut f = udp_frame(MAC_A, MAC_GW, IP_GW, IP_A, 1234, 9999, b"payload!");
    let n = f.len();
    f[n - 1] ^= 0x40;
    dev.inject(&f);
    // Same corruption but checksum 0 ("none"): delivered as is.
    let mut seg = udp_seg(IP_GW, IP_A, 1234, 9999, b"payload!", false);
    let n = seg.len();
    seg[n - 1] ^= 0x40;
    dev.inject(&udp_frame_raw(MAC_A, MAC_GW, IP_GW, IP_A, &seg));
    // Bad source address.
    dev.inject(&udp_frame(MAC_A, MAC_GW, [224, 0, 0, 1], IP_A, 1, 9999, b"m"));
    a.poll(&mut dev, 2);
    let s = a.stats();
    assert_eq!(s.drop_udp_csum, 1);
    assert_eq!(s.drop_bad_src, 1);
    let mut buf = [0u8; 64];
    let (_, _, n) = a.udp_recv(sock, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"payload\x61");
    invariant(&a);
}

#[test]
fn udp_ring_limits_unbind_and_stale_ids() {
    let mut dev = Wire::single(MAC_A);
    let mut a = Iface::new(MAC_A, IpConfig::SLIRP, log);
    let s1 = a.udp_bind(1000).unwrap();
    let s2 = a.udp_bind(1001).unwrap();
    assert_eq!(a.udp_bind(1000), Err(NetError::PortInUse));
    for i in 0..UDP_RING + 2 {
        dev.inject(&udp_frame(MAC_A, MAC_GW, IP_GW, IP_A, 7, 1000, &[i as u8; 3]));
    }
    a.poll(&mut dev, 1);
    assert_eq!(a.udp_queued(s1), UDP_RING);
    assert_eq!(a.stats().drop_udp_rx_full, 2);
    // The shared pool limits the second socket.
    for i in 0..UDP_RING {
        dev.inject(&udp_frame(MAC_A, MAC_GW, IP_GW, IP_A, 7, 1001, &[0x80 | i as u8]));
    }
    a.poll(&mut dev, 2);
    assert_eq!(a.udp_queued(s2), (UDP_POOL - UDP_RING).min(UDP_RING));
    assert_eq!(a.udp_pool_used(), UDP_POOL.min(2 * UDP_RING));
    // FIFO order.
    let mut buf = [0u8; 8];
    for i in 0..UDP_RING {
        let (_, _, n) = a.udp_recv(s1, &mut buf).unwrap();
        assert_eq!(&buf[..n], &[i as u8; 3]);
    }
    // Unbind frees the rest; the id goes stale.
    assert!(a.udp_unbind(s2));
    assert!(!a.udp_unbind(s2));
    assert_eq!(a.udp_pool_used(), 0);
    assert!(a.udp_recv(s2, &mut buf).is_none());
    assert_eq!(a.udp_send(&mut dev, s2, IP_GW, 1, b"x", 3), Err(NetError::BadSocket));
    let s3 = a.udp_bind(1001).unwrap();
    assert_ne!(s2, s3, "generation distinguishes reuse of a slot");
    // Table full.
    let mut ids = vec![];
    while let Ok(id) = a.udp_bind(0) {
        ids.push(id);
    }
    assert_eq!(a.udp_bind(2000), Err(NetError::NoSockets));
    assert_eq!(ids.len(), crate::net::iface::UDP_SOCKETS - 2);
    invariant(&a);
}

#[test]
fn udp_broadcast_accepted_and_sent_without_arp() {
    let (mut wa, mut wb, mut a, mut b) = pair();
    let mut now = 0;
    let sb = b.udp_bind(5000).unwrap();
    a.udp_send_to(&mut wa, 4000, [255; 4], 5000, b"all", now).unwrap();
    a.udp_send_to(&mut wa, 4000, [10, 0, 2, 255], 5000, b"subnet", now).unwrap();
    pump(&mut wa, &mut wb, &mut a, &mut b, &mut now);
    assert_eq!(a.stats().tx_arp_requests, 0, "broadcast needs no ARP");
    let first = parse_udp_frame(&wa.history[0]).unwrap();
    assert_eq!((first.eth_dst, first.dst), (BCAST_MAC, [255; 4]));
    let mut buf = [0u8; 16];
    let (_, _, n) = b.udp_recv(sb, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"all");
    let (_, _, n) = b.udp_recv(sb, &mut buf).unwrap();
    assert_eq!(&buf[..n], b"subnet");
    // Broadcast to another subnet and ICMP to a broadcast address are not ours.
    let mut dev = Wire::single(MAC_A);
    let mut c = Iface::new(MAC_A, IpConfig::SLIRP, log);
    dev.inject(&udp_frame(BCAST_MAC, MAC_GW, IP_GW, [10, 0, 3, 255], 1, 5000, b"x"));
    dev.inject(&echo_frame(BCAST_MAC, MAC_GW, IP_GW, [255; 4], 8, 1, 1, b"x"));
    c.poll(&mut dev, 1);
    assert_eq!(c.stats().drop_not_for_us, 2);
    assert!(dev.take_sent().is_empty());
    invariant(&b);
    invariant(&c);
}
