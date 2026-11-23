//! DNS stub resolver against a fake resolver at 10.0.2.3: A, CNAME chains,
//! compression (and pointer loops), RCODEs, mismatches, timeout + retry, cache,
//! dotted quads, name validation, and a google.com response laid out like a real one.

use crate::net::dns::{self, DnsError};
use crate::net::iface::{Iface, IpConfig, QueryId, DNS_QUERIES};
use crate::net_wire::*;

fn setup() -> (Wire, Iface) {
    (Wire::single(MAC_A), Iface::new(MAC_A, IpConfig::SLIRP, log))
}

/// Answer ARP for the resolver/gateway; return the DNS queries seen.
fn collect(dev: &mut Wire) -> Vec<(UdpView, DnsQueryView)> {
    let mut out = vec![];
    for f in dev.take_sent() {
        if let Some(tpa) = arp_request_target(&f) {
            let mac = if tpa == IP_DNS { MAC_DNS } else { MAC_GW };
            dev.inject(&arp_frame(MAC_A, 2, mac, tpa, MAC_A, IP_A));
            continue;
        }
        if let Some(u) = parse_udp_frame(&f) {
            if u.dport == 53 {
                let q = parse_dns_query(&u.payload).expect("well-formed query");
                out.push((u, q));
            }
        }
    }
    out
}

/// Start a query and poll until it is on the wire (ARP first).
fn start(dev: &mut Wire, a: &mut Iface, name: &str, seed: u32, t: &mut u64) -> (QueryId, UdpView, DnsQueryView) {
    let id = a.dns_query(dev, name, *t, seed).expect("query started");
    for _ in 0..5 {
        if let Some((u, q)) = collect(dev).pop() {
            return (id, u, q);
        }
        *t += 1;
        a.poll(dev, *t);
    }
    panic!("query for {name} never sent");
}

fn reply(dev: &mut Wire, a: &mut Iface, u: &UdpView, msg: &[u8], t: u64) {
    dev.inject(&dns_frame(u.sport, msg));
    a.poll(dev, t);
}

fn invariant(a: &Iface) {
    let s = a.stats();
    assert_eq!(s.rx_frames, s.rx_delivered + s.rx_dropped(), "{s:#?}");
}

#[test]
fn dns_plain_a_record_and_cache() {
    let (mut dev, mut a) = setup();
    let mut t = 0;
    let (qid, u, q) = start(&mut dev, &mut a, "example.com", 1, &mut t);
    // The query on the wire.
    assert_eq!((u.dst, u.dport, u.src), (IP_DNS, 53, IP_A));
    assert!(u.sport >= 49152, "ephemeral source port {}", u.sport);
    assert!(u.csum_ok);
    assert_eq!((q.flags, q.qdcount, q.qtype, q.qclass), (0x0100, 1, 1, 1));
    assert_eq!(q.qname, dns_name("example.com"));
    assert_eq!(a.dns_result(qid), None, "in flight");
    assert_eq!(a.dns_in_flight(), 1);

    let msg = dns_response(q.id, 0, &q.qname, &[Rr::a(PTR_Q.to_vec(), 300, [93, 184, 216, 34])]);
    reply(&mut dev, &mut a, &u, &msg, t + 5);
    assert_eq!(a.dns_result(qid), Some(Ok([93, 184, 216, 34])));
    assert_eq!(count_log("net: dns example.com 93.184.216.34"), 1);
    let s = a.stats();
    assert_eq!((s.tx_dns, s.rx_dns, s.dns_answers, s.dns_retries), (1, 1, 1, 0));
    assert_eq!(a.dns_in_flight(), 0);

    // Cache hit: different case and a trailing dot, no query on the wire.
    let q2 = a.dns_query(&mut dev, "EXAMPLE.com.", 1000, 2).unwrap();
    assert_eq!(a.dns_result(q2), Some(Ok([93, 184, 216, 34])));
    a.poll(&mut dev, 1001);
    assert!(collect(&mut dev).is_empty(), "served from cache");
    assert_eq!(a.stats().dns_cache_hits, 1);
    assert_eq!(a.stats().tx_dns, 1);
    // The earlier result stays readable.
    assert_eq!(a.dns_result(qid), Some(Ok([93, 184, 216, 34])));

    // TTL honoured: expired after 300 s, so a new query goes out.
    let mut t = 301_000;
    a.poll(&mut dev, t);
    assert_eq!(a.dns_cached("example.com"), None);
    let (_, _, q3) = start(&mut dev, &mut a, "example.com", 3, &mut t);
    assert_eq!(q3.qname, dns_name("example.com"));
    assert_ne!(q3.id, q.id, "id derived from the new seed");
    invariant(&a);
}

#[test]
fn dns_ttl_is_capped() {
    let (mut dev, mut a) = setup();
    let mut t = 0;
    let (qid, u, q) = start(&mut dev, &mut a, "long.ttl", 1, &mut t);
    let msg = dns_response(q.id, 0, &q.qname, &[Rr::a(PTR_Q.to_vec(), 0x7FFF_FFFF, [1, 1, 1, 1])]);
    reply(&mut dev, &mut a, &u, &msg, 10);
    assert_eq!(a.dns_result(qid), Some(Ok([1, 1, 1, 1])));
    a.poll(&mut dev, 3_599_000);
    assert_eq!(a.dns_cached("long.ttl"), Some([1, 1, 1, 1]));
    a.poll(&mut dev, 3_601_000);
    assert_eq!(a.dns_cached("long.ttl"), None, "capped at one hour");

    // TTL 0 is not cached.
    let mut t = 3_700_000;
    let (_, u, q) = start(&mut dev, &mut a, "zero.ttl", 2, &mut t);
    let msg = dns_response(q.id, 0, &q.qname, &[Rr::a(PTR_Q.to_vec(), 0, [2, 2, 2, 2])]);
    reply(&mut dev, &mut a, &u, &msg, t + 1);
    assert_eq!(a.dns_cached("zero.ttl"), None);
}

#[test]
fn dns_cname_chain_with_compression() {
    let (mut dev, mut a) = setup();
    let mut t = 0;
    let (qid, u, q) = start(&mut dev, &mut a, "www.example.com", 9, &mut t);
    // Records out of order: unrelated A, the final A, then the CNAMEs.
    let mut msg = dns_response(
        q.id,
        0,
        &q.qname,
        &[
            Rr::a(dns_name("other.example.org"), 999, [9, 9, 9, 9]),
            Rr::a(dns_name("b.example.net"), 60, [1, 2, 3, 4]),
            Rr::cname(PTR_Q.to_vec(), 300, dns_name("a.example.net")),
        ],
    );
    // "example.net" inside the CNAME rdata just written: rdata = 1 'a' 7 example 3 net 0.
    let rdata_len = dns_name("a.example.net").len();
    let example_net = (msg.len() - rdata_len + 2) as u16;
    let a_name = (msg.len() - rdata_len) as u16;
    // a.example.net (as a pointer) CNAME "b" + pointer to example.net.
    msg.extend_from_slice(&(0xC000 | a_name).to_be_bytes());
    msg.extend_from_slice(&[0, 5, 0, 1, 0, 0, 0, 120, 0, 4, 1, b'b']);
    msg.extend_from_slice(&(0xC000 | example_net).to_be_bytes());
    msg[7] = 4; // ANCOUNT
    reply(&mut dev, &mut a, &u, &msg, 10);
    assert_eq!(a.dns_result(qid), Some(Ok([1, 2, 3, 4])));
    assert_eq!(count_log("net: dns www.example.com 1.2.3.4"), 1);
    // Cached for the chain's minimum TTL (60 s).
    a.poll(&mut dev, 59_000);
    assert_eq!(a.dns_cached("www.example.com"), Some([1, 2, 3, 4]));
    a.poll(&mut dev, 61_000);
    assert_eq!(a.dns_cached("www.example.com"), None);
    invariant(&a);
}

#[test]
fn dns_compression_loops_and_bad_pointers_are_ignored() {
    let (mut dev, mut a) = setup();
    let mut t = 0;
    let (qid, u, q) = start(&mut dev, &mut a, "loop.test", 4, &mut t);
    let base = dns_response(q.id, 0, &q.qname, &[]);
    let ans_off = base.len() as u16;
    let mut bad = vec![];
    // Owner name pointing at itself.
    let mut m = base.clone();
    m.extend_from_slice(&(0xC000 | ans_off).to_be_bytes());
    m.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 2, 3, 4]);
    m[7] = 1;
    bad.push(m);
    // Pointer forward (past itself).
    let mut m = base.clone();
    m.extend_from_slice(&(0xC000 | (ans_off + 2)).to_be_bytes());
    m.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 2, 3, 4]);
    m[7] = 1;
    bad.push(m);
    // Label running past the end.
    let mut m = base.clone();
    m.extend_from_slice(&[40, b'x', b'y']);
    m[7] = 1;
    bad.push(m);
    // RDLENGTH past the end.
    let mut m = dns_response(q.id, 0, &q.qname, &[Rr::a(PTR_Q.to_vec(), 60, [1, 2, 3, 4])]);
    let n = m.len();
    m[n - 5] = 0x40;
    bad.push(m);
    // ANCOUNT larger than the records present.
    let mut m = dns_response(q.id, 0, &q.qname, &[Rr::a(PTR_Q.to_vec(), 60, [1, 2, 3, 4])]);
    m[7] = 2;
    bad.push(m);
    // Reserved label type 0x40.
    let mut m = base.clone();
    m.extend_from_slice(&[0x41, b'x', 0, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 2, 3, 4]);
    m[7] = 1;
    bad.push(m);
    // A CNAME loop (a -> b -> a) without any A: NoAnswer, but only after the
    // malformed ones are rejected. Tested last below.
    for m in &bad {
        reply(&mut dev, &mut a, &u, m, 10);
        assert_eq!(a.dns_result(qid), None, "malformed response must be ignored");
    }
    assert_eq!(a.stats().drop_dns_malformed, bad.len() as u64);

    // Name compressed via a chain of pointers (pointer -> pointer -> labels) works.
    let mut m = base.clone();
    m.extend_from_slice(&PTR_Q); // answer 1 owner: -> question
    m.extend_from_slice(&[0, 5, 0, 1, 0, 0, 0, 60, 0, 2]);
    let ptr_to_ptr = ans_off; // the owner pointer of answer 1
    m.extend_from_slice(&PTR_Q); // CNAME target: itself (loop)
    m[7] = 1;
    // Second answer: owner = pointer to answer 1's owner pointer (-> question).
    m.extend_from_slice(&(0xC000 | ptr_to_ptr).to_be_bytes());
    m.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 30, 0, 4, 5, 6, 7, 8]);
    m[7] = 2;
    reply(&mut dev, &mut a, &u, &m, 11);
    assert_eq!(a.dns_result(qid), Some(Ok([5, 6, 7, 8])));
    invariant(&a);
}

#[test]
fn dns_cname_loop_without_a_is_no_answer() {
    let (mut dev, mut a) = setup();
    let mut t = 0;
    let (qid, u, q) = start(&mut dev, &mut a, "a.loop", 5, &mut t);
    let msg = dns_response(
        q.id,
        0,
        &q.qname,
        &[
            Rr::cname(PTR_Q.to_vec(), 60, dns_name("b.loop")),
            Rr::cname(dns_name("b.loop"), 60, dns_name("a.loop")),
        ],
    );
    reply(&mut dev, &mut a, &u, &msg, 10);
    assert_eq!(a.dns_result(qid), Some(Err(DnsError::NoAnswer)));
    assert_eq!(count_log("net: dns a.loop fail noanswer"), 1);
}

#[test]
fn dns_nxdomain_servfail_noanswer() {
    let (mut dev, mut a) = setup();
    let mut t = 0;
    for (name, rcode, want, tag) in [
        ("nope.example", 3u8, DnsError::NxDomain, "net: dns nope.example fail nxdomain"),
        ("broken.example", 2, DnsError::ServFail, "net: dns broken.example fail servfail"),
        ("refused.example", 5, DnsError::Refused, "net: dns refused.example fail refused"),
        ("empty.example", 0, DnsError::NoAnswer, "net: dns empty.example fail noanswer"),
    ] {
        let (qid, u, q) = start(&mut dev, &mut a, name, rcode as u32 + 100, &mut t);
        let msg = dns_response(q.id, rcode, &q.qname, &[]);
        t += 1;
        reply(&mut dev, &mut a, &u, &msg, t);
        assert_eq!(a.dns_result(qid), Some(Err(want)), "{name}");
        assert_eq!(count_log(tag), 1);
        assert_eq!(a.dns_cached(name), None);
    }
    assert_eq!(a.stats().dns_failures, 4);
    invariant(&a);
}

#[test]
fn dns_mismatched_responses_are_ignored() {
    let (mut dev, mut a) = setup();
    let mut t = 0;
    let (qid, u, q) = start(&mut dev, &mut a, "match.example", 6, &mut t);
    let good = dns_response(q.id, 0, &q.qname, &[Rr::a(PTR_Q.to_vec(), 60, [7, 7, 7, 7])]);
    // Wrong id.
    let wrong_id = dns_response(q.id.wrapping_add(1), 0, &q.qname, &[Rr::a(PTR_Q.to_vec(), 60, [6, 6, 6, 6])]);
    reply(&mut dev, &mut a, &u, &wrong_id, 10);
    // Wrong question.
    let other = dns_name("other.example");
    let wrong_q = dns_response(q.id, 0, &other, &[Rr::a(PTR_Q.to_vec(), 60, [6, 6, 6, 6])]);
    reply(&mut dev, &mut a, &u, &wrong_q, 11);
    // Not a response (QR clear).
    let mut not_resp = good.clone();
    not_resp[2] &= 0x7F;
    reply(&mut dev, &mut a, &u, &not_resp, 12);
    // Right bytes from the wrong server.
    dev.inject(&udp_frame(MAC_A, MAC_GW, [10, 0, 2, 4], IP_A, 53, u.sport, &good));
    a.poll(&mut dev, 13);
    assert_eq!(a.dns_result(qid), None);
    assert_eq!(a.stats().drop_dns_mismatch, 4);
    // To a port with no query: an ordinary unbound UDP port.
    dev.inject(&dns_frame(u.sport.wrapping_add(1), &good));
    a.poll(&mut dev, 14);
    assert_eq!(a.stats().drop_udp_no_socket, 1);
    // The real answer still completes the query.
    reply(&mut dev, &mut a, &u, &good, 15);
    assert_eq!(a.dns_result(qid), Some(Ok([7, 7, 7, 7])));
    // A duplicate afterwards has no query to match.
    reply(&mut dev, &mut a, &u, &good, 16);
    assert_eq!(a.stats().drop_udp_no_socket, 2);
    invariant(&a);
}

#[test]
fn dns_timeout_retries_once_then_fails() {
    let (mut dev, mut a) = setup();
    let qid = a.dns_query(&mut dev, "slow.example", 0, 77).unwrap();
    let mut seen: Vec<(u64, UdpView, DnsQueryView)> = vec![];
    for (u, q) in collect(&mut dev) {
        seen.push((0, u, q));
    }
    let mut t = 0;
    while t < 3900 {
        t += 100;
        a.poll(&mut dev, t);
        for (u, q) in collect(&mut dev) {
            seen.push((t, u, q));
        }
    }
    assert_eq!(a.dns_result(qid), None);
    let times: Vec<u64> = seen.iter().map(|s| s.0).collect();
    // First attempt waits for ARP (resolved on the first poll), the retry is at 2 s.
    assert_eq!(times, vec![100, 2000]);
    assert_eq!(seen[0].2.id, seen[1].2.id, "same id on retry");
    assert_eq!(seen[0].1.sport, seen[1].1.sport, "same port on retry");
    a.poll(&mut dev, 3999);
    assert_eq!(a.dns_result(qid), None);
    a.poll(&mut dev, 4000);
    assert_eq!(a.dns_result(qid), Some(Err(DnsError::Timeout)));
    assert_eq!(count_log("net: dns slow.example fail timeout"), 1);
    let s = a.stats();
    assert_eq!((s.tx_dns, s.dns_retries, s.dns_timeouts, s.dns_failures), (2, 1, 1, 1));
    // A late answer is not matched any more.
    let late = dns_response(seen[0].2.id, 0, &seen[0].2.qname, &[Rr::a(PTR_Q.to_vec(), 60, [1, 1, 1, 1])]);
    reply(&mut dev, &mut a, &seen[0].1, &late, 4100);
    assert_eq!(a.dns_result(qid), Some(Err(DnsError::Timeout)));
    invariant(&a);
}

#[test]
fn dns_dotted_quad_names_and_errors() {
    let (mut dev, mut a) = setup();
    let q = a.dns_query(&mut dev, "8.8.8.8", 0, 1).unwrap();
    assert_eq!(a.dns_result(q), Some(Ok([8, 8, 8, 8])));
    a.poll(&mut dev, 1);
    assert!(collect(&mut dev).is_empty(), "no query for a literal address");
    assert_eq!(a.stats().tx_dns, 0);

    let long_label = "a".repeat(64);
    let name_254 = format!("{}.{}.{}.{}", "a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(62));
    assert_eq!(name_254.len(), 254);
    for bad in ["", ".", "a..b", ".a", "exa mple.com", "bad/char", "ümlaut.de", long_label.as_str(), name_254.as_str()] {
        assert_eq!(a.dns_query(&mut dev, bad, 2, 1), Err(DnsError::BadName), "{bad:?}");
    }
    assert_eq!(count_log("net: dns a..b fail badname"), 1);
    // 253 characters and 63-byte labels are fine.
    let name_253 = &name_254[..253];
    let mut t = 10;
    let (_, _, q) = start(&mut dev, &mut a, name_253, 5, &mut t);
    assert_eq!(q.qname.len(), 255);
    assert_eq!(q.qname, dns_name(name_253));
    // The wire-format helpers agree.
    assert_eq!(dns::encode_name("A-b_1.Example.").unwrap().as_bytes(), &dns_name("a-b_1.example")[..]);
    assert_eq!(dns::parse_dotted_quad("1.2.3"), None);
    assert_eq!(dns::parse_dotted_quad("1.2.3.4.5"), None);
    assert_eq!(dns::parse_dotted_quad("1.2.3.256"), None);
    assert_eq!(dns::parse_dotted_quad("01.2.3.4"), Some([1, 2, 3, 4]));

    // No resolver configured.
    let mut b = Iface::new(MAC_A, IpConfig { dns: [0; 4], ..IpConfig::SLIRP }, log);
    assert_eq!(b.dns_query(&mut dev, "example.com", 0, 1), Err(DnsError::NoServer));
    assert_eq!(count_log("net: dns example.com fail noserver"), 1);
    // Unconfigured interface: the send fails at once.
    let mut c = Iface::new(MAC_A, IpConfig { ip: [0; 4], ..IpConfig::SLIRP }, log);
    assert!(matches!(c.dns_query(&mut dev, "example.com", 0, 1), Err(DnsError::Net(_))));
    assert_eq!(c.dns_in_flight(), 0);
}

#[test]
fn dns_query_table_busy_and_slot_reuse() {
    let (mut dev, mut a) = setup();
    let mut ids = vec![];
    for i in 0..DNS_QUERIES {
        ids.push(a.dns_query(&mut dev, &format!("h{i}.example"), 0, i as u32).unwrap());
    }
    assert_eq!(a.dns_query(&mut dev, "more.example", 0, 99), Err(DnsError::Busy));
    // Distinct ports for concurrent queries.
    a.poll(&mut dev, 1);
    let mut qs = collect(&mut dev);
    a.poll(&mut dev, 2);
    qs.extend(collect(&mut dev));
    let mut ports: Vec<u16> = qs.iter().map(|(u, _)| u.sport).collect();
    ports.sort();
    ports.dedup();
    assert_eq!(ports.len(), DNS_QUERIES);
    // Let them all time out; finished slots are then reused oldest first.
    for t in (100..=4100).step_by(100) {
        a.poll(&mut dev, t);
    }
    assert!(ids.iter().all(|id| a.dns_result(*id) == Some(Err(DnsError::Timeout))));
    let n = a.dns_query(&mut dev, "next.example", 5000, 7).unwrap();
    assert_eq!(a.dns_result(n), None);
    assert_eq!(ids.iter().filter(|id| a.dns_result(**id).is_none()).count(), 1, "one old slot recycled");
}

/// A google.com A response laid out like one captured from a public resolver:
/// QR RD RA, one answer with a pointer to the question, plus an EDNS OPT record.
#[test]
fn dns_google_com_captured_layout() {
    let (mut dev, mut a) = setup();
    let mut t = 0;
    let (qid, u, q) = start(&mut dev, &mut a, "google.com", 0x600d, &mut t);
    let id = q.id.to_be_bytes();
    #[rustfmt::skip]
    let msg: Vec<u8> = vec![
        id[0], id[1], 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
        // question: google.com A IN
        0x06, b'g', b'o', b'o', b'g', b'l', b'e', 0x03, b'c', b'o', b'm', 0x00, 0x00, 0x01, 0x00, 0x01,
        // answer: ptr 0x0c, A, IN, TTL 300, 142.250.80.46
        0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x01, 0x2c, 0x00, 0x04, 0x8e, 0xfa, 0x50, 0x2e,
        // additional: OPT (root, type 41, udp 512, no options)
        0x00, 0x00, 0x29, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    reply(&mut dev, &mut a, &u, &msg, 50);
    assert_eq!(a.dns_result(qid), Some(Ok([142, 250, 80, 46])));
    assert_eq!(count_log("net: dns google.com 142.250.80.46"), 1);
    a.poll(&mut dev, 299_000);
    assert_eq!(a.dns_cached("google.com"), Some([142, 250, 80, 46]));
    invariant(&a);
}
