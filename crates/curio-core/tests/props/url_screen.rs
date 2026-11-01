//! URL screening — `fetch::screen_url` (the shape gate every hop passes)
//! and `fetch::policy::is_public_address` (the SSRF address gate): only
//! http(s) URLs with a host and no userinfo get through, and an IPv4
//! address smuggled inside any IPv6 embedding form is judged exactly like
//! the IPv4 address itself.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use curio_core::fetch::policy::is_public_address;
use curio_core::fetch::screen_url;
use proptest::prelude::*;

const SCHEMES: &[&str] = &[
    "http",
    "https",
    "HTTP",
    "HtTpS",
    "ftp",
    "file",
    "javascript",
    "data",
    "gopher",
    "ws",
    "feed",
    "about",
];

fn embedded(v4: Ipv4Addr) -> [u16; 2] {
    let [a, b, c, d] = v4.octets();
    [u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d])]
}

proptest! {
    #[test]
    fn whatever_passes_the_screen_is_fetchable_in_shape(raw in "\\PC{0,200}") {
        if let Ok(url) = screen_url(&raw) {
            prop_assert!(matches!(url.scheme(), "http" | "https"), "{raw:?}");
            prop_assert!(url.username().is_empty() && url.password().is_none(), "{raw:?}");
            prop_assert!(url.host().is_some(), "{raw:?}");
        }
    }

    #[test]
    fn the_screen_judges_scheme_and_userinfo(
        scheme in prop::sample::select(SCHEMES),
        user in prop::option::of("[a-z]{1,8}(:[a-z0-9]{0,8})?"),
        host in "[a-z]{1,12}\\.(example|test)",
        path in "(/[a-z0-9._~-]{0,8}){0,4}",
    ) {
        let userinfo = user.as_ref().map(|user| format!("{user}@")).unwrap_or_default();
        let raw = format!("{scheme}://{userinfo}{host}{path}");
        let http = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
        prop_assert_eq!(screen_url(&raw).is_ok(), http && user.is_none(), "{}", raw);
    }

    #[test]
    fn ipv6_embeddings_are_judged_by_their_ipv4(octets in any::<[u8; 4]>()) {
        let v4 = Ipv4Addr::from(octets);
        let verdict = is_public_address(IpAddr::V4(v4));
        let [hi, lo] = embedded(v4);
        let forms = [
            ("mapped", v4.to_ipv6_mapped()),
            ("compatible", Ipv6Addr::new(0, 0, 0, 0, 0, 0, hi, lo)),
            ("nat64", Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, hi, lo)),
            ("6to4", Ipv6Addr::new(0x2002, hi, lo, 0, 0, 0, 0, 1)),
        ];
        for (form, v6) in forms {
            prop_assert_eq!(is_public_address(IpAddr::V6(v6)), verdict, "{} form {}", form, v6);
        }
    }

    #[test]
    fn teredo_is_public_only_when_server_and_client_are(
        server in any::<[u8; 4]>(),
        client in any::<[u8; 4]>(),
        flags in any::<u16>(),
        port in any::<u16>(),
    ) {
        let (server, client) = (Ipv4Addr::from(server), Ipv4Addr::from(client));
        let [s_hi, s_lo] = embedded(server);
        let [c_hi, c_lo] = embedded(client);
        // RFC 4380: the client address and port are stored ones-complemented.
        let teredo = Ipv6Addr::new(0x2001, 0, s_hi, s_lo, flags, !port, !c_hi, !c_lo);
        let expected = is_public_address(IpAddr::V4(server)) && is_public_address(IpAddr::V4(client));
        prop_assert_eq!(is_public_address(IpAddr::V6(teredo)), expected);
    }

    #[test]
    fn private_ranges_are_never_public(
        tail in any::<[u8; 3]>(),
        segments in any::<[u16; 7]>(),
    ) {
        let [b, c, d] = tail;
        for v4 in [
            Ipv4Addr::new(10, b, c, d),
            Ipv4Addr::new(127, b, c, d),
            Ipv4Addr::new(192, 168, c, d),
            Ipv4Addr::new(169, 254, c, d),
            Ipv4Addr::new(100, 64 | (b & 0x3f), c, d),
            Ipv4Addr::new(172, 16 | (b & 0x0f), c, d),
        ] {
            prop_assert!(!is_public_address(IpAddr::V4(v4)), "{v4} judged public");
        }
        let [s1, s2, s3, s4, s5, s6, s7] = segments;
        for first in [0xfc00 | (s1 & 0x01ff), 0xfe80 | (s1 & 0x003f), 0xff00 | (s1 & 0x00ff)] {
            let v6 = Ipv6Addr::new(first, s2, s3, s4, s5, s6, s7, s1);
            prop_assert!(!is_public_address(IpAddr::V6(v6)), "{v6} judged public");
        }
    }
}
