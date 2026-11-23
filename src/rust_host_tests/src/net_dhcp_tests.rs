//! DHCP client against a fake server on the far side of the wire: DORA, NAK,
//! retransmit backoff, static fallback, renew at T1, rebind at T2, expiry.

use crate::net::iface::{DhcpState, Iface, IpConfig};
use crate::net_wire::*;

const LEASED: IpConfig = IpConfig { ip: IP_A, prefix: 24, gw: IP_GW, dns: IP_DNS };
const TAG: &str = "net: dhcp 10.0.2.15 gw 10.0.2.2 dns 10.0.2.3";

/// A client message as seen by the server.
#[derive(Clone, Debug)]
struct Seen {
    t: u64,
    udp: UdpView,
    msg: DhcpMsg,
}

struct FakeDhcp {
    lease: u32,
    /// Answer anything at all.
    answer: bool,
    /// OFFERs / ACKs to swallow before answering.
    drop_offers: usize,
    drop_acks: usize,
    /// NAK the next REQUEST.
    nak_next: bool,
    /// Send the ACK unicast to the offered address (not broadcast).
    ack_unicast: bool,
    seen: Vec<Seen>,
}

impl FakeDhcp {
    fn new(lease: u32) -> FakeDhcp {
        FakeDhcp { lease, answer: true, drop_offers: 0, drop_acks: 0, nak_next: false, ack_unicast: false, seen: vec![] }
    }

    fn serve(&mut self, dev: &mut Wire, t: u64) {
        for f in dev.take_sent() {
            if let Some(tpa) = arp_request_target(&f) {
                if tpa == IP_GW {
                    dev.inject(&arp_frame(MAC_A, 2, MAC_GW, IP_GW, MAC_A, IP_A));
                }
                continue;
            }
            let Some(udp) = parse_udp_frame(&f) else { continue };
            if udp.dport != 67 {
                continue;
            }
            assert!(udp.csum_ok, "client UDP checksum");
            assert_eq!(udp.sport, 68);
            let msg = parse_dhcp(&udp.payload).expect("well-formed DHCP");
            self.seen.push(Seen { t, udp: udp.clone(), msg: msg.clone() });
            if !self.answer {
                continue;
            }
            match msg.msg_type() {
                Some(1) => {
                    if self.drop_offers > 0 {
                        self.drop_offers -= 1;
                        continue;
                    }
                    let r = dhcp_reply(2, msg.xid, msg.chaddr, IP_A, &slirp_opts(self.lease));
                    dev.inject(&dhcp_server_frame(BCAST_MAC, [255; 4], &r));
                }
                Some(3) => {
                    if self.nak_next {
                        self.nak_next = false;
                        let r = dhcp_reply(6, msg.xid, msg.chaddr, [0; 4], &[(54, IP_GW.to_vec())]);
                        dev.inject(&dhcp_server_frame(BCAST_MAC, [255; 4], &r));
                        continue;
                    }
                    if self.drop_acks > 0 {
                        self.drop_acks -= 1;
                        continue;
                    }
                    let r = dhcp_reply(5, msg.xid, msg.chaddr, IP_A, &slirp_opts(self.lease));
                    if msg.ciaddr != [0; 4] || self.ack_unicast {
                        dev.inject(&dhcp_server_frame(MAC_A, IP_A, &r));
                    } else {
                        dev.inject(&dhcp_server_frame(BCAST_MAC, [255; 4], &r));
                    }
                }
                other => panic!("unexpected client message {other:?}"),
            }
        }
    }

    fn of_type(&self, ty: u8) -> Vec<&Seen> {
        self.seen.iter().filter(|s| s.msg.msg_type() == Some(ty)).collect()
    }
}

/// Poll + serve every `step` ms over (from, to].
fn run(a: &mut Iface, dev: &mut Wire, srv: &mut FakeDhcp, from: u64, to: u64, step: u64) {
    let mut t = from;
    while t < to {
        t += step;
        a.poll(dev, t);
        srv.serve(dev, t);
    }
}

fn invariant(a: &Iface) {
    let s = a.stats();
    assert_eq!(s.rx_frames, s.rx_delivered + s.rx_dropped(), "{s:#?}");
}

fn unconfigured() -> Iface {
    Iface::new(MAC_A, IpConfig::NONE, log)
}

#[test]
fn dhcp_full_dora_applies_config_and_logs() {
    let mut dev = Wire::single(MAC_A);
    let mut a = unconfigured();
    let mut srv = FakeDhcp::new(86400);
    srv.ack_unicast = true; // exercises unicast acceptance while unconfigured
    assert_eq!(a.dhcp_state(), DhcpState::Idle);
    a.dhcp_start(&mut dev, 0, 0xC0FFEE, Some(IpConfig::SLIRP));
    assert_eq!(a.dhcp_state(), DhcpState::Selecting);
    srv.serve(&mut dev, 0);
    a.poll(&mut dev, 10);
    assert_eq!(a.dhcp_state(), DhcpState::Requesting);
    srv.serve(&mut dev, 10);
    a.poll(&mut dev, 20);
    assert_eq!(a.dhcp_state(), DhcpState::Bound { lease_s: 86400 });
    assert_eq!(a.config(), LEASED);
    assert_eq!(count_log(TAG), 1);
    assert_eq!(a.dhcp_server(), IP_GW);

    // DISCOVER, byte level.
    let d = &srv.seen[0];
    assert_eq!(d.msg.msg_type(), Some(1));
    assert_eq!((d.udp.eth_dst, d.udp.eth_src), (BCAST_MAC, MAC_A));
    assert_eq!((d.udp.src, d.udp.dst), ([0; 4], [255; 4]));
    assert_eq!((d.msg.op, d.msg.flags, d.msg.secs), (1, 0x8000, 0));
    assert_eq!((d.msg.ciaddr, d.msg.yiaddr, d.msg.chaddr), ([0; 4], [0; 4], MAC_A));
    let prl = d.msg.opt(55).expect("parameter request list");
    for want in [1u8, 3, 6, 51] {
        assert!(prl.contains(&want), "option 55 asks for {want}");
    }
    assert!(d.msg.opt(50).is_none() && d.msg.opt(54).is_none());
    assert!(d.udp.payload.len() >= 300, "BOOTP minimum");
    // REQUEST.
    let r = &srv.seen[1];
    assert_eq!(r.msg.msg_type(), Some(3));
    assert_eq!(r.msg.xid, d.msg.xid, "same transaction");
    assert_eq!((r.udp.src, r.udp.dst, r.udp.eth_dst), ([0; 4], [255; 4], BCAST_MAC));
    assert_eq!((r.msg.flags, r.msg.ciaddr), (0x8000, [0; 4]));
    assert_eq!(r.msg.opt(50), Some(&IP_A[..]));
    assert_eq!(r.msg.opt(54), Some(&IP_GW[..]));
    // A different seed gives a different xid.
    let mut b = unconfigured();
    let mut dev_b = Wire::single(MAC_A);
    b.dhcp_start(&mut dev_b, 0, 0xC0FFEF, None);
    assert_ne!(b.dhcp_xid(), d.msg.xid);

    // Quiet afterwards: no fallback at 3 s, no traffic until T1.
    run(&mut a, &mut dev, &mut srv, 20, 10_000, 100);
    assert_eq!(srv.seen.len(), 2);
    assert_eq!(a.config(), LEASED);
    assert_eq!(count_log("net: dhcp timeout, static 10.0.2.15/24"), 0);
    let s = a.stats();
    assert_eq!((s.dhcp_offers, s.dhcp_acks, s.tx_dhcp, s.rx_dhcp, s.dhcp_retransmits), (1, 1, 2, 2, 0));
    invariant(&a);

    // Now configured, the stack answers ARP for the leased address.
    dev.inject(&arp_frame(BCAST_MAC, 1, MAC_GW, IP_GW, [0; 6], IP_A));
    a.poll(&mut dev, 10_001);
    assert_eq!(dev.take_sent().len(), 1);
}

#[test]
fn dhcp_nak_restarts_with_new_xid() {
    let mut dev = Wire::single(MAC_A);
    let mut a = unconfigured();
    let mut srv = FakeDhcp::new(3600);
    srv.nak_next = true;
    a.dhcp_start(&mut dev, 0, 7, Some(IpConfig::SLIRP));
    srv.serve(&mut dev, 0); // OFFER
    a.poll(&mut dev, 10); // REQUEST
    srv.serve(&mut dev, 10); // NAK
    a.poll(&mut dev, 20); // NAK -> SELECTING, DISCOVER right away
    assert_eq!(count_log("net: dhcp nak"), 1);
    assert_eq!(a.dhcp_state(), DhcpState::Selecting);
    let old_xid = srv.seen[0].msg.xid;
    // A late OFFER for the old transaction is ignored.
    let stale = dhcp_reply(2, old_xid, MAC_A, [10, 0, 2, 99], &slirp_opts(60));
    dev.inject(&dhcp_server_frame(BCAST_MAC, [255; 4], &stale));
    let before = a.stats().drop_dhcp;
    srv.serve(&mut dev, 20); // answers the new DISCOVER
    a.poll(&mut dev, 30);
    assert_eq!(a.stats().drop_dhcp, before + 1);
    srv.serve(&mut dev, 30);
    a.poll(&mut dev, 40);
    assert_eq!(a.dhcp_state(), DhcpState::Bound { lease_s: 3600 });
    assert_eq!(a.config(), LEASED);
    let discovers = srv.of_type(1);
    assert_eq!(discovers.len(), 2);
    assert_ne!(discovers[1].msg.xid, old_xid);
    assert_eq!(srv.of_type(3).last().unwrap().msg.xid, discovers[1].msg.xid);
    assert_eq!(a.stats().dhcp_naks, 1);
    assert_eq!(count_log(TAG), 1);
    invariant(&a);
}

#[test]
fn dhcp_lost_offers_and_acks_retransmit_with_backoff() {
    let mut dev = Wire::single(MAC_A);
    let mut a = unconfigured();
    let mut srv = FakeDhcp::new(3600);
    srv.drop_offers = 3;
    srv.drop_acks = 2;
    a.dhcp_start(&mut dev, 0, 99, None); // no fallback: keep trying
    srv.serve(&mut dev, 0);
    run(&mut a, &mut dev, &mut srv, 0, 20_000, 100);
    let dt: Vec<u64> = srv.of_type(1).iter().map(|s| s.t).collect();
    assert_eq!(dt, vec![0, 1000, 3000, 7000], "DISCOVER backoff 1 s, 2 s, 4 s");
    let secs: Vec<u16> = srv.of_type(1).iter().map(|s| s.msg.secs).collect();
    assert_eq!(secs, vec![0, 1, 3, 7]);
    // OFFER for the 7 s DISCOVER handled at 7.1 s; REQUESTs back off from there.
    let rt: Vec<u64> = srv.of_type(3).iter().map(|s| s.t).collect();
    assert_eq!(rt, vec![7100, 8100, 10_100]);
    assert!(srv.seen.iter().all(|s| s.msg.xid == srv.seen[0].msg.xid));
    assert_eq!(a.dhcp_state(), DhcpState::Bound { lease_s: 3600 });
    assert_eq!(a.stats().dhcp_retransmits, 3 + 2);
    assert_eq!(count_log(TAG), 1);
    invariant(&a);
}

#[test]
fn dhcp_no_server_falls_back_to_static_at_3s() {
    let mut dev = Wire::single(MAC_A);
    let mut a = unconfigured();
    a.dhcp_start(&mut dev, 0, 1, Some(IpConfig::SLIRP));
    let mut discovers = vec![0u64];
    dev.take_sent();
    let mut t = 0;
    while t < 2900 {
        t += 100;
        a.poll(&mut dev, t);
        if !dev.take_sent().is_empty() {
            discovers.push(t);
        }
    }
    assert_eq!(a.dhcp_state(), DhcpState::Selecting);
    assert_eq!(a.config(), IpConfig::NONE);
    a.poll(&mut dev, 3000);
    assert!(dev.take_sent().is_empty(), "no DISCOVER at the deadline");
    assert_eq!(a.dhcp_state(), DhcpState::Failed);
    assert_eq!(a.config(), IpConfig::SLIRP);
    assert_eq!(count_log("net: dhcp timeout, static 10.0.2.15/24"), 1);
    assert_eq!(discovers, vec![0, 1000]);
    for t in (3100..20_000).step_by(100) {
        a.poll(&mut dev, t);
    }
    assert!(dev.take_sent().is_empty(), "client stopped");
    assert_eq!(count_log("net: dhcp timeout, static 10.0.2.15/24"), 1);
    assert_eq!(a.stats().dhcp_fallbacks, 1);

    // Without a fallback the client gives up at 60 s and leaves the config alone.
    let mut dev = Wire::single(MAC_A);
    let mut b = unconfigured();
    b.dhcp_start(&mut dev, 0, 1, None);
    for t in (1000..=59_000).step_by(1000) {
        b.poll(&mut dev, t);
    }
    assert_eq!(b.dhcp_state(), DhcpState::Selecting);
    b.poll(&mut dev, 60_000);
    assert_eq!(b.dhcp_state(), DhcpState::Failed);
    assert_eq!(b.config(), IpConfig::NONE);
    assert_eq!(count_log("net: dhcp fail"), 1);
}

#[test]
fn dhcp_renews_at_t1_rebinds_at_t2_expires_at_lease_end() {
    let mut dev = Wire::single(MAC_A);
    let mut a = unconfigured();
    let mut srv = FakeDhcp::new(100); // T1 50 s, T2 87.5 s
    a.dhcp_start(&mut dev, 0, 5, Some(IpConfig::SLIRP));
    srv.serve(&mut dev, 0);
    run(&mut a, &mut dev, &mut srv, 0, 20, 10); // ACK handled at t = 20
    assert_eq!(a.dhcp_state(), DhcpState::Bound { lease_s: 100 });
    let dora_xid = srv.seen[0].msg.xid;

    run(&mut a, &mut dev, &mut srv, 20, 50_000, 500);
    assert_eq!(srv.seen.len(), 2, "nothing before T1");
    run(&mut a, &mut dev, &mut srv, 50_000, 52_000, 500);
    // T1 = 50.02 s: the 50.5 s poll starts renewing (ARP for the server first).
    let renew = &srv.seen[2];
    assert_eq!(renew.msg.msg_type(), Some(3));
    assert!(renew.t >= 50_500 && renew.t <= 51_500, "renew at {}", renew.t);
    assert_eq!((renew.udp.src, renew.udp.dst, renew.udp.eth_dst), (IP_A, IP_GW, MAC_GW), "unicast to the server");
    assert_eq!((renew.msg.ciaddr, renew.msg.flags), (IP_A, 0));
    assert!(renew.msg.opt(50).is_none() && renew.msg.opt(54).is_none());
    assert_ne!(renew.msg.xid, dora_xid);
    assert_eq!(a.dhcp_state(), DhcpState::Bound { lease_s: 100 });
    assert_eq!(count_log(TAG), 2);
    let renewed_at = renew.t + 500; // ACK processed on the next poll

    // Server silent: renew at T1 again, then rebind (broadcast) at T2.
    srv.answer = false;
    let t1 = renewed_at + 50_000;
    let t2 = renewed_at + 87_500;
    run(&mut a, &mut dev, &mut srv, 52_000, t2 + 1000, 500);
    let reqs: Vec<&Seen> = srv.of_type(3).into_iter().skip(2).collect();
    assert_eq!(reqs.len(), 2, "one renew (next retry clamps to T2) + one rebind");
    assert!(reqs[0].t >= t1 && reqs[0].t < t1 + 1000);
    assert_eq!(reqs[0].udp.dst, IP_GW);
    assert!(reqs[1].t >= t2 && reqs[1].t < t2 + 1000);
    assert_eq!((reqs[1].udp.src, reqs[1].udp.dst, reqs[1].udp.eth_dst), (IP_A, [255; 4], BCAST_MAC));
    assert_eq!(reqs[1].msg.ciaddr, IP_A);
    assert!(matches!(a.dhcp_state(), DhcpState::Rebinding { .. }));
    assert_eq!(a.config(), LEASED, "address kept while rebinding");

    // Lease runs out: config dropped, back to DISCOVER from 0.0.0.0.
    let expire = renewed_at + 100_000;
    run(&mut a, &mut dev, &mut srv, t2 + 1000, expire + 500, 500);
    assert_eq!(count_log("net: dhcp lease expired"), 1);
    assert_eq!(a.config(), IpConfig::NONE);
    assert_eq!(a.dhcp_state(), DhcpState::Selecting);
    let last = srv.seen.last().unwrap();
    assert_eq!(last.msg.msg_type(), Some(1));
    assert_eq!(last.udp.src, [0; 4]);

    // Server back: a fresh lease.
    srv.answer = true;
    run(&mut a, &mut dev, &mut srv, expire + 500, expire + 5000, 100);
    assert_eq!(a.dhcp_state(), DhcpState::Bound { lease_s: 100 });
    assert_eq!(a.config(), LEASED);
    assert_eq!(count_log(TAG), 3);
    invariant(&a);
}

#[test]
fn dhcp_rebind_ack_rebinds() {
    let mut dev = Wire::single(MAC_A);
    let mut a = unconfigured();
    let mut srv = FakeDhcp::new(100);
    a.dhcp_start(&mut dev, 0, 11, None);
    srv.serve(&mut dev, 0);
    run(&mut a, &mut dev, &mut srv, 0, 20, 10);
    srv.answer = false;
    run(&mut a, &mut dev, &mut srv, 20, 88_000, 500); // past T2 = 87.52 s
    assert!(matches!(a.dhcp_state(), DhcpState::Rebinding { .. }));
    srv.answer = true;
    // The next rebind REQUEST is at expire (clamped); answer it.
    run(&mut a, &mut dev, &mut srv, 88_000, 99_000, 500);
    assert!(matches!(a.dhcp_state(), DhcpState::Rebinding { .. }));
    // Inject an ACK for the current rebind transaction directly.
    let xid = a.dhcp_xid();
    let r = dhcp_reply(5, xid, MAC_A, IP_A, &slirp_opts(100));
    dev.inject(&dhcp_server_frame(MAC_A, IP_A, &r));
    a.poll(&mut dev, 99_100);
    assert_eq!(a.dhcp_state(), DhcpState::Bound { lease_s: 100 });
    assert_eq!(count_log(TAG), 2);
    invariant(&a);
}

#[test]
fn dhcp_bind_announces_with_one_gratuitous_arp() {
    let mut dev = Wire::single(MAC_A);
    let mut a = unconfigured();
    let mut srv = FakeDhcp::new(86400);
    a.dhcp_start(&mut dev, 0, 0x5EED, None);
    srv.serve(&mut dev, 0);
    a.poll(&mut dev, 10);
    srv.serve(&mut dev, 10);
    a.poll(&mut dev, 20);
    assert!(matches!(a.dhcp_state(), DhcpState::Bound { .. }));
    let sent = dev.take_sent();
    let garps = sent
        .iter()
        .filter(|f| arp_request_target(f) == Some(IP_A) && f.len() >= 42 && f[28..32] == IP_A)
        .count();
    assert_eq!(garps, 1, "one gratuitous ARP for the leased address");
    a.poll(&mut dev, 30);
    assert!(dev.take_sent().iter().all(|f| arp_request_target(f) != Some(IP_A)), "announced only once");
    assert_eq!(a.stats().tx_garp, 1);
}
