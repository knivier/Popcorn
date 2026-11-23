//! Net stack behaviour: ARP, ping between two ifaces, routing, ARP timeout, IPv4
//! validation, loopback device.

use crate::drivers::net::loopback::Loopback;
use crate::drivers::net::{NetDevice, RxBatch};
use crate::net::iface::{Iface, IpConfig, NetError, ARP_MAX_REQUESTS};
use crate::net::{arp, eth, icmp, ipv4};
use crate::net_wire::*;
use crate::util::csum;

const BCAST: [u8; 6] = [0xFF; 6];

fn iface_a() -> Iface {
    Iface::new(MAC_A, IpConfig::SLIRP, log)
}

/// Iface A with the gateway already in its ARP cache.
fn iface_a_with_gw(dev: &mut Wire) -> Iface {
    let mut a = iface_a();
    dev.inject(&arp_frame(MAC_A, 2, MAC_GW, IP_GW, MAC_A, IP_A));
    a.poll(dev, 0);
    assert_eq!(a.arp_lookup(IP_GW), Some(MAC_GW));
    dev.take_sent();
    a
}

fn is_arp_request_for(f: &[u8], tpa: [u8; 4]) -> bool {
    let Some((h, p)) = eth::parse(f) else { return false };
    if h.ethertype != eth::ETHERTYPE_ARP || h.dst != BCAST {
        return false;
    }
    matches!(arp::parse(p), Ok(a) if a.op == arp::OP_REQUEST && a.tpa == tpa && a.sha == MAC_A && a.spa == IP_A)
}

// ---- 1. ARP ------------------------------------------------------------------------

#[test]
fn arp_request_for_us_gets_reply() {
    let mut dev = Wire::single(MAC_A);
    let mut a = iface_a();
    dev.inject(&arp_frame(BCAST, 1, MAC_GW, IP_GW, [0; 6], IP_A));
    a.poll(&mut dev, 5);
    let sent = dev.take_sent();
    assert_eq!(sent.len(), 1);
    let f = &sent[0];
    assert_eq!(f.len(), 60, "padded by the device");
    // Ethernet
    assert_eq!(&f[0..6], &MAC_GW);
    assert_eq!(&f[6..12], &MAC_A);
    assert_eq!(&f[12..14], &[0x08, 0x06]);
    // ARP reply, byte for byte
    let mut want = vec![0, 1, 8, 0, 6, 4, 0, 2];
    want.extend_from_slice(&MAC_A);
    want.extend_from_slice(&IP_A);
    want.extend_from_slice(&MAC_GW);
    want.extend_from_slice(&IP_GW);
    assert_eq!(&f[14..42], &want[..]);
    assert!(f[42..].iter().all(|b| *b == 0));
    let s = a.stats();
    assert_eq!((s.rx_arp_requests, s.tx_arp_replies, s.rx_delivered), (1, 1, 1));
    // The requester was learned (no log: only replies log).
    assert_eq!(a.arp_lookup(IP_GW), Some(MAC_GW));
    assert_eq!(count_log("net: arp 10.0.2.2 52550a000202"), 0);
}

#[test]
fn arp_request_for_other_ip_ignored() {
    let mut dev = Wire::single(MAC_A);
    let mut a = iface_a();
    dev.inject(&arp_frame(BCAST, 1, MAC_GW, IP_GW, [0; 6], [10, 0, 2, 99]));
    a.poll(&mut dev, 0);
    assert!(dev.take_sent().is_empty());
    assert_eq!(a.stats().drop_not_for_us, 1);
    assert_eq!(a.arp_lookup(IP_GW), None, "not learned from a request for someone else");

    // Unsupported hardware/protocol sizes.
    let mut bad = arp_frame(BCAST, 1, MAC_GW, IP_GW, [0; 6], IP_A);
    bad[14 + 4] = 8; // hlen
    dev.inject(&bad);
    let mut bad2 = arp_frame(BCAST, 1, MAC_GW, IP_GW, [0; 6], IP_A);
    bad2[14 + 3] = 0xDD; // ptype
    dev.inject(&bad2);
    let mut bad3 = arp_frame(BCAST, 7, MAC_GW, IP_GW, [0; 6], IP_A); // opcode
    dev.inject(&bad3[..]);
    bad3.truncate(14 + 20);
    dev.inject(&bad3);
    a.poll(&mut dev, 0);
    assert!(dev.take_sent().is_empty());
    let s = a.stats();
    assert_eq!(s.drop_bad_header, 3);
    assert_eq!(s.drop_short, 1);
    // Frames for another MAC are not even looked at.
    dev.inject(&arp_frame([2, 0, 0, 0, 0, 1], 1, MAC_GW, IP_GW, [0; 6], IP_A));
    a.poll(&mut dev, 0);
    assert!(dev.take_sent().is_empty());
    assert_eq!(a.stats().drop_not_for_us, 2);
}

#[test]
fn arp_reply_fills_cache_and_logs() {
    let mut dev = Wire::single(MAC_A);
    let mut a = iface_a();
    assert_eq!(a.arp_lookup(IP_GW), None);
    dev.inject(&arp_frame(MAC_A, 2, MAC_GW, IP_GW, MAC_A, IP_A));
    a.poll(&mut dev, 10);
    assert_eq!(a.arp_lookup(IP_GW), Some(MAC_GW));
    assert_eq!(count_log("net: arp 10.0.2.2 52550a000202"), 1);
    assert!(dev.take_sent().is_empty(), "replies are not answered");
    // Same reply again: refreshed silently.
    dev.inject(&arp_frame(MAC_A, 2, MAC_GW, IP_GW, MAC_A, IP_A));
    a.poll(&mut dev, 20);
    assert_eq!(count_log("net: arp 10.0.2.2 52550a000202"), 1);
    assert_eq!(a.stats().rx_arp_replies, 2);
    // Entry expires after the TTL.
    a.poll(&mut dev, 20 + arp::CACHE_TTL_MS);
    assert_eq!(a.arp_lookup(IP_GW), None);
}

// ---- 2. two ifaces over a wire --------------------------------------------------

#[test]
fn two_ifaces_ping_twenty_times() {
    let ip_b = IP_GW;
    let (mut wa, mut wb) = Wire::pair(MAC_A, MAC_GW);
    let mut a = iface_a();
    let mut b = Iface::new(MAC_GW, IpConfig { ip: ip_b, prefix: 24, gw: [0; 4], dns: [0; 4] }, log);
    let id = 0xBEEF;
    let mut now = 1000u64;
    for seq in 0..20u16 {
        a.ping_send(&mut wa, ip_b, id, seq, 56, now).unwrap();
        // ARP request/reply takes an extra round trip on the first ping.
        for _ in 0..3 {
            now += 1;
            b.poll(&mut wb, now);
            now += 1;
            a.poll(&mut wa, now);
        }
        let rtt = a.ping_reply(id, seq).expect("reply arrived");
        assert_eq!(rtt, if seq == 0 { 4 } else { 2 }, "seq {seq}");
        now += 1000;
    }
    assert_eq!(a.ping_reply(id, 99), None);

    let sa = a.stats();
    let sb = b.stats();
    assert_eq!(sa.tx_arp_requests, 1, "ARP resolution happens once");
    assert_eq!(sa.rx_arp_replies, 1);
    assert_eq!(sb.tx_arp_requests, 0, "B learned A from the request");
    assert_eq!(sb.rx_arp_requests, 1);
    assert_eq!(sb.tx_arp_replies, 1);
    assert_eq!(sb.rx_echo_requests, 20);
    assert_eq!(sb.tx_echo_replies, 20);
    assert_eq!(sa.tx_echo_requests, 20);
    assert_eq!(sa.rx_echo_replies, 20);
    assert_eq!(sa.rx_dropped() + sb.rx_dropped(), 0);
    assert_eq!(count_log("net: arp 10.0.2.2 52550a000202"), 1);
    assert_eq!(a.pending_frames(), 0);

    // Every echo request on the wire: TTL 64, DF, valid header and ICMP checksums,
    // pattern payload.
    let mut seen = 0;
    for f in &wa.history {
        let (h, p) = eth::parse(f).unwrap();
        if h.ethertype != eth::ETHERTYPE_IPV4 {
            continue;
        }
        assert_eq!(h.dst, MAC_GW);
        assert_eq!(p[0], 0x45);
        assert_eq!(&p[6..8], &[0x40, 0x00], "DF, no fragment");
        assert_eq!(p[8], 64);
        assert_eq!(csum::checksum(&p[..20]), 0);
        let (ih, msg) = ipv4::parse(p).unwrap();
        assert_eq!((ih.src, ih.dst, ih.proto), (IP_A, ip_b, 1));
        assert_eq!(msg.len(), 8 + 56);
        let (ic, payload) = icmp::parse(msg).unwrap();
        assert_eq!((ic.ty, ic.id), (icmp::TYPE_ECHO_REQUEST, id));
        assert!(payload.iter().enumerate().all(|(i, b)| *b == icmp::pattern_byte(i)));
        seen += 1;
    }
    assert_eq!(seen, 20);
    assert_eq!(wa.lent_count() + wb.lent_count(), 0);
    assert_eq!(wa.tx_busy_count() + wb.tx_busy_count(), 0);
}

// ---- 3. off-link routing ----------------------------------------------------------

#[test]
fn off_link_goes_to_gateway() {
    let mut dev = Wire::single(MAC_A);
    let mut a = iface_a_with_gw(&mut dev);
    a.ping_send(&mut dev, [8, 8, 8, 8], 1, 1, 32, 100).unwrap();
    let sent = dev.take_sent();
    assert_eq!(sent.len(), 1);
    let (h, p) = eth::parse(&sent[0]).unwrap();
    assert_eq!(h.dst, MAC_GW);
    let (ih, _) = ipv4::parse(p).unwrap();
    assert_eq!(ih.dst, [8, 8, 8, 8]);

    // Unresolved gateway: we ARP for the gateway, never for 8.8.8.8.
    let mut dev2 = Wire::single(MAC_A);
    let mut a2 = iface_a();
    a2.ping_send(&mut dev2, [8, 8, 8, 8], 1, 1, 32, 100).unwrap();
    let sent = dev2.take_sent();
    assert_eq!(sent.len(), 1);
    assert!(is_arp_request_for(&sent[0], IP_GW));
    assert_eq!(a2.pending_frames(), 1);
    // Gateway answers: the queued echo goes out to its MAC.
    dev2.inject(&arp_frame(MAC_A, 2, MAC_GW, IP_GW, MAC_A, IP_A));
    a2.poll(&mut dev2, 150);
    let sent = dev2.take_sent();
    assert_eq!(sent.len(), 1);
    let (h, p) = eth::parse(&sent[0]).unwrap();
    assert_eq!(h.dst, MAC_GW);
    assert_eq!(ipv4::parse(p).unwrap().0.dst, [8, 8, 8, 8]);
    assert_eq!(a2.pending_frames(), 0);

    // No gateway configured: off-link is unroutable.
    let mut a3 = Iface::new(MAC_A, IpConfig { gw: [0; 4], ..IpConfig::SLIRP }, log);
    assert_eq!(a3.ping_send(&mut dev2, [8, 8, 8, 8], 1, 1, 8, 0), Err(NetError::NoRoute));
    // Unconfigured / oversized.
    let mut a4 = Iface::new(MAC_A, IpConfig::NONE, log);
    assert_eq!(a4.ping_send(&mut dev2, IP_GW, 1, 1, 8, 0), Err(NetError::NotConfigured));
    assert_eq!(a.ping_send(&mut dev, IP_GW, 1, 1, 1473, 0), Err(NetError::TooBig));
    a.ping_send(&mut dev, IP_GW, 1, 2, 1472, 0).unwrap();
    assert_eq!(dev.take_sent()[0].len(), 1514);
}

// ---- 4. ARP timeout -----------------------------------------------------------------

#[test]
fn unresolvable_next_hop_times_out() {
    let mut dev = Wire::single(MAC_A);
    let mut a = iface_a();
    let dst = [10, 0, 2, 77];
    let t0 = 5000;
    a.ping_send(&mut dev, dst, 7, 1, 16, t0).unwrap();
    let mut requests = Vec::new();
    let count = |dev: &mut Wire, t: u64, requests: &mut Vec<u64>| {
        for f in dev.take_sent() {
            assert!(is_arp_request_for(&f, dst), "only ARP requests may leave");
            requests.push(t);
        }
    };
    count(&mut dev, t0, &mut requests);
    for t in (t0 + 100..=t0 + 2999).step_by(100) {
        a.poll(&mut dev, t);
        count(&mut dev, t, &mut requests);
        assert_eq!(a.pending_frames(), 1, "t={t}");
    }
    assert_eq!(requests, vec![t0, t0 + 1000, t0 + 2000]);
    assert_eq!(requests.len(), ARP_MAX_REQUESTS as usize);
    assert_eq!(count_log("net: arp timeout 10.0.2.77"), 0);

    a.poll(&mut dev, t0 + 3000);
    count(&mut dev, t0 + 3000, &mut requests);
    assert_eq!(requests.len(), 3, "no 4th request");
    assert_eq!(a.pending_frames(), 0);
    let s = a.stats();
    assert_eq!(s.drop_arp_timeout, 1);
    assert_eq!(s.arp_timeouts, 1);
    assert_eq!(s.arp_retries, 2);
    assert_eq!(count_log("net: arp timeout 10.0.2.77"), 1);
    assert_eq!(a.ping_reply(7, 1), None);

    for t in [t0 + 4000, t0 + 10_000] {
        a.poll(&mut dev, t);
        assert!(dev.take_sent().is_empty());
    }
    assert_eq!(count_log("net: arp timeout 10.0.2.77"), 1);

    // The pending queue holds 8 frames, then refuses.
    for seq in 0..8 {
        a.ping_send(&mut dev, dst, 7, seq, 16, 20_000).unwrap();
    }
    assert_eq!(a.ping_send(&mut dev, dst, 7, 8, 16, 20_000), Err(NetError::PendingFull));
    assert_eq!(dev.take_sent().len(), 1, "one ARP request for all frames to the same hop");
    for t in [21_000, 22_000] {
        a.poll(&mut dev, t);
        assert_eq!(dev.take_sent().len(), 1, "retry at {t}");
        assert_eq!(a.pending_frames(), 8);
    }
    a.poll(&mut dev, 23_000);
    assert!(dev.take_sent().is_empty());
    assert_eq!(a.pending_frames(), 0);
    assert_eq!(a.stats().drop_arp_timeout, 9);
    assert_eq!(a.stats().drop_pending_full, 1);
}

// ---- 5. IPv4 validation ------------------------------------------------------------

fn valid_echo_request() -> Vec<u8> {
    echo_frame(MAC_A, MAC_GW, IP_GW, IP_A, 8, 0x42, 3, b"abcdefgh01234567")
}

#[test]
fn ipv4_malformed_dropped_and_counted() {
    let mut dev = Wire::single(MAC_A);
    let mut a = iface_a_with_gw(&mut dev);
    let base = a.stats();

    // Sanity: the valid frame is answered with a matching reply.
    dev.inject(&valid_echo_request());
    a.poll(&mut dev, 1);
    let sent = dev.take_sent();
    assert_eq!(sent.len(), 1);
    let (h, p) = eth::parse(&sent[0]).unwrap();
    assert_eq!(h.dst, MAC_GW);
    let (ih, msg) = ipv4::parse(p).unwrap();
    assert_eq!((ih.src, ih.dst, ih.ttl), (IP_A, IP_GW, 64));
    let (ic, payload) = icmp::parse(msg).unwrap();
    assert_eq!((ic.ty, ic.code, ic.id, ic.seq), (0, 0, 0x42, 3));
    assert_eq!(payload, b"abcdefgh01234567");

    let ip = 14; // IPv4 header offset in the frame
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut f = valid_echo_request();
    f[ip + 10] ^= 0x01;
    cases.push(("bad_csum", f));
    let mut f = valid_echo_request();
    f[ip + 6] |= 0x20; // MF
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("fragment", f));
    let mut f = valid_echo_request();
    f[ip + 7] = 0x08; // offset 8
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("fragment", f));
    let mut f = valid_echo_request();
    let tl = (f.len() - ip + 1) as u16; // one byte longer than the frame
    f[ip + 2..ip + 4].copy_from_slice(&tl.to_be_bytes());
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("bad_len", f));
    let mut f = valid_echo_request();
    f[ip + 2..ip + 4].copy_from_slice(&19u16.to_be_bytes());
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("bad_len", f));
    let mut f = valid_echo_request();
    f[ip] = 0x44; // IHL 4
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("bad_header", f));
    let mut f = valid_echo_request();
    f[ip] = 0x65; // version 6
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("bad_header", f));
    let mut f = valid_echo_request();
    f.truncate(ip + 19);
    cases.push(("short", f));
    let mut f = valid_echo_request();
    let n = f.len();
    f[n - 1] ^= 0xFF; // ICMP payload corrupted
    cases.push(("bad_csum", f));
    let mut f = valid_echo_request();
    f[ip + 16..ip + 20].copy_from_slice(&[10, 0, 2, 16]);
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("not_for_us", f));
    let mut f = valid_echo_request();
    f[ip + 9] = 6; // TCP
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("unknown_proto", f));
    let mut f = valid_echo_request();
    f[ip + 12..ip + 16].copy_from_slice(&[255, 255, 255, 255]); // broadcast source
    fix_ip_csum(&mut f[ip..ip + 20]);
    cases.push(("bad_src", f));
    let mut f = valid_echo_request();
    f[12..14].copy_from_slice(&[0x86, 0xDD]);
    cases.push(("unknown_ethertype", f));
    cases.push(("short", vec![0u8; 10]));

    for (why, frame) in &cases {
        let before = a.stats();
        dev.inject(frame);
        a.poll(&mut dev, 2);
        assert!(dev.take_sent().is_empty(), "{why}: must not be answered");
        let s = a.stats();
        let d = |x: u64, y: u64| x - y;
        let got = match *why {
            "bad_csum" => d(s.drop_bad_csum, before.drop_bad_csum),
            "fragment" => d(s.drop_fragment, before.drop_fragment),
            "bad_len" => d(s.drop_bad_len, before.drop_bad_len),
            "bad_header" => d(s.drop_bad_header, before.drop_bad_header),
            "short" => d(s.drop_short, before.drop_short),
            "not_for_us" => d(s.drop_not_for_us, before.drop_not_for_us),
            "unknown_proto" => d(s.drop_unknown_proto, before.drop_unknown_proto),
            "bad_src" => d(s.drop_bad_src, before.drop_bad_src),
            "unknown_ethertype" => d(s.drop_unknown_ethertype, before.drop_unknown_ethertype),
            _ => unreachable!(),
        };
        assert_eq!(got, 1, "{why}");
        assert_eq!(s.rx_dropped(), before.rx_dropped() + 1, "{why}: exactly one drop reason");
        assert_eq!(s.rx_echo_requests, before.rx_echo_requests);
    }
    let s = a.stats();
    assert_eq!(s.rx_echo_requests - base.rx_echo_requests, 1);
    assert_eq!(s.tx_echo_replies - base.tx_echo_replies, 1);
    assert_eq!(s.rx_frames, s.rx_delivered + s.rx_dropped());

    // Ethernet padding after a valid IPv4 packet is ignored (payload trimmed).
    let mut f = valid_echo_request();
    f.extend_from_slice(&[0xEE; 20]);
    dev.inject(&f);
    a.poll(&mut dev, 3);
    let sent = dev.take_sent();
    assert_eq!(sent.len(), 1);
    let (_, msg) = ipv4::parse(&sent[0][14..]).unwrap();
    assert_eq!(&msg[8..], b"abcdefgh01234567");
}

#[test]
fn ipv4_options_are_skipped() {
    let mut dev = Wire::single(MAC_A);
    let mut a = iface_a_with_gw(&mut dev);
    let msg = icmp_echo(8, 9, 9, b"xyz");
    let mut h = ip_hdr(IP_GW, IP_A, 1, msg.len() + 4);
    h[0] = 0x46; // IHL 6: one option word
    h.extend_from_slice(&[1, 1, 1, 0]); // NOP NOP NOP EOL
    fix_ip_csum(&mut h);
    let mut f = eth_hdr(MAC_A, MAC_GW, 0x0800);
    f.extend_from_slice(&h);
    f.extend_from_slice(&msg);
    dev.inject(&f);
    a.poll(&mut dev, 1);
    let sent = dev.take_sent();
    assert_eq!(sent.len(), 1);
    let (_, m) = ipv4::parse(&sent[0][14..]).unwrap();
    assert_eq!(&m[8..], b"xyz");
}

#[test]
fn unmatched_echo_reply_counted() {
    let mut dev = Wire::single(MAC_A);
    let mut a = iface_a_with_gw(&mut dev);
    a.ping_send(&mut dev, IP_GW, 5, 1, 8, 10).unwrap();
    dev.take_sent();
    // Wrong seq, then wrong source, then the right one (twice: second is a dup).
    let pl = [0u8, 1, 2, 3, 4, 5, 6, 7];
    dev.inject(&echo_frame(MAC_A, MAC_GW, IP_GW, IP_A, 0, 5, 2, &pl));
    dev.inject(&echo_frame(MAC_A, MAC_GW, [10, 0, 2, 9], IP_A, 0, 5, 1, &pl));
    dev.inject(&echo_frame(MAC_A, MAC_GW, IP_GW, IP_A, 0, 5, 1, &pl));
    dev.inject(&echo_frame(MAC_A, MAC_GW, IP_GW, IP_A, 0, 5, 1, &pl));
    a.poll(&mut dev, 25);
    assert_eq!(a.ping_reply(5, 1), Some(15));
    let s = a.stats();
    assert_eq!(s.rx_echo_replies, 1);
    assert_eq!(s.drop_unmatched_reply, 3);
}

// ---- 7. loopback device -------------------------------------------------------------

#[test]
fn loopback_round_trips_a_frame() {
    let mut lo = Loopback::new(MAC_A);
    let frame = echo_frame(MAC_A, MAC_A, IP_A, IP_A, 8, 1, 1, &[0x5A; 100]);
    let mut buf = lo.tx_alloc().unwrap();
    buf.frame_mut()[..frame.len()].copy_from_slice(&frame);
    lo.tx_submit(buf, frame.len()).unwrap();
    lo.tx_flush();
    let mut batch = RxBatch::new();
    assert_eq!(lo.rx_batch(32, &mut batch), 1);
    assert_eq!(batch.frame(0), &frame[..]);
    lo.rx_recycle(&mut batch);
    assert_eq!(lo.rx_batch(32, &mut batch), 0);
    // Short frames come back padded to 60.
    let mut buf = lo.tx_alloc().unwrap();
    buf.frame_mut()[..20].fill(7);
    lo.tx_submit(buf, 20).unwrap();
    lo.tx_flush();
    assert_eq!(lo.rx_batch(32, &mut batch), 1);
    assert_eq!(batch.frame(0).len(), 60);
    assert_eq!(&batch.frame(0)[..20], &[7u8; 20]);
    lo.rx_recycle(&mut batch);
    assert_eq!(lo.stats().tx_free, 16);
}

#[test]
fn iface_pings_itself_over_loopback() {
    let mut lo = Loopback::new(MAC_A);
    let mut a = iface_a();
    a.ping_send(&mut lo, IP_A, 3, 1, 56, 0).unwrap();
    for t in 1..6 {
        a.poll(&mut lo, t);
    }
    assert!(a.ping_reply(3, 1).is_some());
    let s = a.stats();
    assert_eq!((s.rx_echo_requests, s.tx_echo_replies, s.rx_echo_replies), (1, 1, 1));
    assert_eq!(s.rx_frames, s.rx_delivered + s.rx_dropped());
    assert_eq!(a.arp_lookup(IP_A), Some(MAC_A));
}
