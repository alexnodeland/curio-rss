//! The SSRF address policy: which resolved IPs may be fetched.
//!
//! The deny-by-default rule of contract W1: after DNS resolution (and on
//! every redirect hop), an address must be *public* to be fetched unless
//! the per-feed `allow_private_network` flag exempts the feed. "Not
//! public" covers loopback, RFC 1918 private ranges, link-local,
//! carrier-grade NAT, ULA, multicast, documentation/benchmark ranges and
//! other never-routable space — for both IPv4 and IPv6, including every
//! IPv4-embedding IPv6 form (checked against the embedded IPv4 address,
//! so `::ffff:10.0.0.1` is as private as `10.0.0.1`): IPv4-mapped
//! (`::ffff:0:0/96`), IPv4-compatible (`::/96`), NAT64 (`64:ff9b::/96`,
//! RFC 6052 — on a NAT64 network `64:ff9b::192.168.0.1` reaches
//! `192.168.0.1`), 6to4 (`2002::/16`) and Teredo (`2001::/32`, both the
//! server and the obfuscated client address).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// `true` iff `addr` is publicly routable — the only kind of address the
/// policed client will fetch without the per-feed exemption.
#[must_use]
pub fn is_public_address(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => ipv4_is_public(v4),
        IpAddr::V6(v6) => ipv6_is_public(v6),
    }
}

fn ipv4_is_public(addr: Ipv4Addr) -> bool {
    let octets = addr.octets();
    let this_network = octets[0] == 0; // 0.0.0.0/8 (includes unspecified)
    let shared_cgnat = octets[0] == 100 && (octets[1] & 0b1100_0000) == 64; // 100.64.0.0/10
    let ietf_protocol = octets[0] == 192 && octets[1] == 0 && octets[2] == 0; // 192.0.0.0/24
    let benchmarking = octets[0] == 198 && (octets[1] & 0xfe) == 18; // 198.18.0.0/15
    let reserved = octets[0] >= 240; // 240.0.0.0/4 (includes broadcast)

    !(this_network
        || addr.is_loopback()
        || addr.is_private()
        || addr.is_link_local()
        || shared_cgnat
        || ietf_protocol
        || benchmarking
        || addr.is_documentation()
        || addr.is_multicast()
        || addr.is_broadcast()
        || reserved)
}

fn ipv6_is_public(addr: Ipv6Addr) -> bool {
    if addr.is_unspecified() || addr.is_loopback() {
        return false;
    }
    // IPv4-mapped (::ffff:a.b.c.d) and the deprecated IPv4-compatible
    // (::a.b.c.d) forms smuggle an IPv4 address — judge the embedded one.
    if let Some(v4) = addr.to_ipv4_mapped().or_else(|| addr.to_ipv4()) {
        return ipv4_is_public(v4);
    }
    let segments = addr.segments();
    // NAT64 (RFC 6052): on a NAT64-enabled network the kernel translates
    // 64:ff9b::<v4> straight to <v4>, so an AAAA record pointing at
    // 64:ff9b::192.168.0.1 reaches 192.168.0.1 — judge the embedded
    // address for the well-known prefix 64:ff9b::/96, and reject the
    // rest of 64:ff9b::/32 outright (64:ff9b:1::/48 is local-use NAT64
    // space, never publicly routable).
    if segments[0] == 0x0064 && segments[1] == 0xff9b {
        if segments[2..6] == [0, 0, 0, 0] {
            return ipv4_is_public(embedded_ipv4(segments[6], segments[7]));
        }
        return false;
    }
    // 6to4 (2002::/16): the tunnel endpoint is the embedded IPv4.
    if segments[0] == 0x2002 {
        return ipv4_is_public(embedded_ipv4(segments[1], segments[2]));
    }
    // Teredo (2001::/32): carries the server's IPv4 in the clear and the
    // client's IPv4 ones-complement-obfuscated — judge both.
    if segments[0] == 0x2001 && segments[1] == 0 {
        let server = embedded_ipv4(segments[2], segments[3]);
        let client = embedded_ipv4(!segments[6], !segments[7]);
        return ipv4_is_public(server) && ipv4_is_public(client);
    }
    let unique_local = (segments[0] & 0xfe00) == 0xfc00; // fc00::/7 (ULA)
    let link_local = (segments[0] & 0xffc0) == 0xfe80; // fe80::/10
    let multicast = (segments[0] & 0xff00) == 0xff00; // ff00::/8
    let documentation = segments[0] == 0x2001 && segments[1] == 0x0db8; // 2001:db8::/32
    let benchmarking = segments[0] == 0x2001 && segments[1] == 0x0002 && segments[2] == 0; // 2001:2::/48

    !(unique_local || link_local || multicast || documentation || benchmarking)
}

/// The IPv4 address packed into two IPv6 segments.
fn embedded_ipv4(hi: u16, lo: u16) -> Ipv4Addr {
    let [a, b] = hi.to_be_bytes();
    let [c, d] = lo.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn addr(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// The contract's table, exhaustively: every named private class is
    /// rejected, boundary neighbors and real public addresses pass.
    #[test]
    fn address_policy_table() {
        #[rustfmt::skip]
        let cases: &[(&str, bool)] = &[
            // IPv4 loopback 127.0.0.0/8
            ("127.0.0.1", false),
            ("127.255.255.255", false),
            // IPv4 private 10.0.0.0/8
            ("10.0.0.1", false),
            ("10.255.255.255", false),
            // IPv4 private 172.16.0.0/12 — with both boundaries
            ("172.16.0.1", false),
            ("172.31.255.255", false),
            ("172.15.255.255", true),
            ("172.32.0.1", true),
            // IPv4 private 192.168.0.0/16
            ("192.168.0.1", false),
            ("192.168.255.255", false),
            ("192.167.255.255", true),
            ("192.169.0.1", true),
            // IPv4 link-local 169.254.0.0/16 (cloud metadata lives here)
            ("169.254.169.254", false),
            ("169.254.0.1", false),
            ("169.253.255.255", true),
            // 0.0.0.0/8, CGNAT 100.64/10, 192.0.0.0/24, benchmarking,
            // documentation, multicast, broadcast, reserved
            ("0.0.0.0", false),
            ("0.1.2.3", false),
            ("100.64.0.1", false),
            ("100.127.255.255", false),
            ("100.63.255.255", true),
            ("100.128.0.1", true),
            ("192.0.0.1", false),
            ("192.0.2.1", false),      // TEST-NET-1
            ("198.51.100.7", false),   // TEST-NET-2
            ("203.0.113.9", false),    // TEST-NET-3
            ("198.18.0.1", false),
            ("198.19.255.255", false),
            ("224.0.0.1", false),
            ("255.255.255.255", false),
            ("240.0.0.1", false),
            // IPv4 public
            ("1.1.1.1", true),
            ("8.8.8.8", true),
            ("93.184.216.34", true),
            ("11.0.0.1", true),
            ("128.0.0.1", true),
            // IPv6 loopback + unspecified
            ("::1", false),
            ("::", false),
            // IPv6 ULA fc00::/7 — both halves
            ("fc00::1", false),
            ("fd12:3456:789a::1", false),
            ("fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", false),
            // IPv6 link-local fe80::/10
            ("fe80::1", false),
            ("febf:ffff::1", false),
            ("fec0::1", true), // deprecated site-local is outside fe80::/10
            // IPv6 multicast, documentation, benchmarking
            ("ff02::1", false),
            ("2001:db8::1", false),
            ("2001:2::1", false),
            // IPv4-mapped / IPv4-compatible smuggling
            ("::ffff:127.0.0.1", false),
            ("::ffff:10.0.0.1", false),
            ("::ffff:192.168.1.1", false),
            ("::ffff:169.254.169.254", false),
            ("::ffff:1.1.1.1", true),
            ("::10.0.0.1", false),
            // NAT64 well-known prefix (RFC 6052): the embedded IPv4 is
            // what the NAT64 gateway actually reaches.
            ("64:ff9b::192.168.0.1", false),
            ("64:ff9b::10.0.0.1", false),
            ("64:ff9b::169.254.169.254", false),
            ("64:ff9b::127.0.0.1", false),
            ("64:ff9b::8.8.8.8", true),
            // NAT64 local-use space 64:ff9b:1::/48 (and the rest of
            // 64:ff9b::/32) is never publicly routable.
            ("64:ff9b:1::192.168.0.1", false),
            ("64:ff9b:1::8.8.8.8", false),
            // 6to4: the tunnel endpoint is the embedded IPv4.
            ("2002:c0a8:1::", false),      // 192.168.0.1
            ("2002:a00:1::", false),       // 10.0.0.1
            ("2002:a9fe:a9fe::", false),   // 169.254.169.254
            ("2002:808:808::1", true),     // 8.8.8.8
            // Teredo: server IPv4 in the clear, client IPv4 inverted.
            ("2001:0:c0a8:1::f7f7:f7f7", false), // server 192.168.0.1
            ("2001:0:505:505::3f57:fffe", false), // client 192.168.0.1
            ("2001:0:505:505::f7f7:f7f7", true), // server+client 8.8.8.8/5.5.5.5
            // IPv6 public
            ("2606:4700:4700::1111", true),
            ("2001:4860:4860::8888", true),
            ("2a00:1450:4001:80b::200e", true),
        ];
        for &(input, expected) in cases {
            assert_eq!(
                is_public_address(addr(input)),
                expected,
                "policy disagreed on {input}"
            );
        }
    }

    #[test]
    fn every_172_16_slash_12_network_is_rejected() {
        for second in 16..=31u8 {
            assert!(
                !is_public_address(addr(&format!("172.{second}.0.1"))),
                "172.{second}.0.1 must be private"
            );
        }
    }
}
