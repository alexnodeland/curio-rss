//! URL screening: arbitrary text → `fetch::screen_url` (whatever passes is
//! http(s) with a host and no userinfo), and arbitrary addresses →
//! `policy::is_public_address`, which must judge every IPv4-embedding IPv6
//! form (mapped, NAT64, 6to4) exactly like the IPv4 address it carries.

#![no_main]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use curio_core::fetch::policy::is_public_address;
use curio_core::fetch::screen_url;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(url) = screen_url(text)
    {
        assert!(matches!(url.scheme(), "http" | "https"), "{text:?}");
        assert!(
            url.username().is_empty() && url.password().is_none(),
            "{text:?}"
        );
        assert!(url.host().is_some(), "{text:?}");
    }

    if let Some(octets) = data.first_chunk::<16>() {
        let _ = is_public_address(IpAddr::V6(Ipv6Addr::from(*octets)));
    }
    if let Some(octets) = data.first_chunk::<4>() {
        let v4 = Ipv4Addr::from(*octets);
        let verdict = is_public_address(IpAddr::V4(v4));
        let [a, b, c, d] = v4.octets();
        let (hi, lo) = (u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d]));
        for v6 in [
            v4.to_ipv6_mapped(),
            Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, hi, lo),
            Ipv6Addr::new(0x2002, hi, lo, 0, 0, 0, 0, 1),
        ] {
            assert_eq!(is_public_address(IpAddr::V6(v6)), verdict, "{v4} as {v6}");
        }
    }
});
