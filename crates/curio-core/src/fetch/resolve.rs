//! DNS for the policed client: the [`DnsLookup`] seam every resolution goes
//! through, the timeout that bounds it, and the pinning resolver the pooled
//! reqwest clients connect through.
//!
//! The SSRF screen runs on the result of one bounded lookup per hop
//! ([`super::PolicedClient`]'s pre-flight); the screened address set is then
//! *pinned* into the [`PinningResolver`] of the client that sends the
//! request. When that client opens a new connection, reqwest asks the
//! resolver, which answers with exactly the pinned set — every address, in
//! resolver order — so hyper's connector can fall back from a dead address
//! to the next (Happy Eyeballs across families) without ever reaching an
//! address the policy did not see. There is no second lookup between the
//! check and the connect, so DNS rebinding cannot slip in between them.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use super::FetchError;

/// The future a [`DnsLookup`] returns: every address the host resolves to,
/// in the order the resolver ranked them.
pub type LookupFuture = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;

/// The DNS seam of the policed client. Production uses [`SystemLookup`]
/// (the platform resolver); tests inject a fixed table to exercise the SSRF
/// screen, address fallback and the lookup timeout hermetically.
///
/// Implementations return addresses only — the port always comes from the
/// URL being fetched.
pub trait DnsLookup: Send + Sync + fmt::Debug {
    /// Resolves `host` (a domain name, never an IP literal) to addresses.
    fn lookup(&self, host: &str) -> LookupFuture;
}

/// The platform resolver (`getaddrinfo` via tokio's blocking pool).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLookup;

impl DnsLookup for SystemLookup {
    fn lookup(&self, host: &str) -> LookupFuture {
        let host = host.to_owned();
        Box::pin(async move {
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            Ok(addrs.map(|addr| addr.ip()).collect())
        })
    }
}

/// One lookup through the seam, bounded by `timeout` (a hung resolver
/// otherwise sits outside every reqwest timeout), deduplicated in order.
///
/// # Errors
///
/// [`FetchError::Dns`] on resolver failure or timeout,
/// [`FetchError::DnsNoAddresses`] on an empty answer.
pub(super) async fn bounded_lookup(
    lookup: &dyn DnsLookup,
    host: &str,
    timeout: Duration,
) -> Result<Vec<IpAddr>, FetchError> {
    let answer = tokio::time::timeout(timeout, lookup.lookup(host))
        .await
        .map_err(|_| FetchError::Dns {
            host: host.to_owned(),
            message: format!("lookup timed out after {}ms", timeout.as_millis()),
        })?
        .map_err(|err| FetchError::Dns {
            host: host.to_owned(),
            message: err.to_string(),
        })?;
    let mut addrs: Vec<IpAddr> = Vec::with_capacity(answer.len());
    for addr in answer {
        if !addrs.contains(&addr) {
            addrs.push(addr);
        }
    }
    if addrs.is_empty() {
        return Err(FetchError::DnsNoAddresses {
            host: host.to_owned(),
        });
    }
    Ok(addrs)
}

/// The resolver installed in every pooled reqwest client. Answers a pinned
/// name with its screened address set; anything else gets a bounded lookup.
///
/// Pins are per client, and clients are keyed by SSRF policy (see
/// `PolicedClient`), so a deny-private client only ever holds addresses that
/// passed the deny-private screen — and never shares a pooled connection
/// with an `allow_private_network` request.
#[derive(Debug)]
pub(super) struct PinningResolver {
    /// Domain → the address set the most recent pre-flight screened.
    pins: Mutex<HashMap<String, Arc<[IpAddr]>>>,
    lookup: Arc<dyn DnsLookup>,
    timeout: Duration,
}

impl PinningResolver {
    pub(super) fn new(lookup: Arc<dyn DnsLookup>, timeout: Duration) -> Self {
        Self {
            pins: Mutex::new(HashMap::new()),
            lookup,
            timeout,
        }
    }

    /// Pins `domain` to `addrs`, which the caller has already screened
    /// under this client's policy. Replaces any earlier pin: every request
    /// pre-flights (and re-pins) its host before it is sent, so the pin a
    /// new connection reads is always the one its own request screened.
    pub(super) fn pin(&self, domain: &str, addrs: Vec<IpAddr>) {
        self.pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(domain.to_owned(), addrs.into());
    }

    fn pinned(&self, domain: &str) -> Option<Arc<[IpAddr]>> {
        self.pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(domain)
            .cloned()
    }
}

/// Port 0 tells hyper to substitute the URL's own port (explicit, or the
/// scheme default) — one pin serves every scheme/port on the host.
fn to_addrs(addrs: &[IpAddr]) -> Addrs {
    let socket_addrs: Vec<SocketAddr> =
        addrs.iter().map(|addr| SocketAddr::new(*addr, 0)).collect();
    Box::new(socket_addrs.into_iter())
}

impl Resolve for PinningResolver {
    fn resolve(&self, name: Name) -> Resolving {
        if let Some(addrs) = self.pinned(name.as_str()) {
            let resolved = to_addrs(&addrs);
            return Box::pin(async move { Ok(resolved) });
        }
        // Unpinned: every request target is pinned before it is sent, so
        // this is only ever the HTTP(S)_PROXY host from the user's own
        // environment — user configuration, resolved (bounded) like the
        // system resolver did before pooling, not screened as feed content.
        let lookup = Arc::clone(&self.lookup);
        let timeout = self.timeout;
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let addrs = bounded_lookup(lookup.as_ref(), &host, timeout).await?;
            Ok(to_addrs(&addrs))
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::str::FromStr;

    use super::*;

    #[derive(Debug)]
    struct Table(Vec<IpAddr>);

    impl DnsLookup for Table {
        fn lookup(&self, _host: &str) -> LookupFuture {
            let addrs = self.0.clone();
            Box::pin(async move { Ok(addrs) })
        }
    }

    #[derive(Debug)]
    struct Hangs;

    impl DnsLookup for Hangs {
        fn lookup(&self, _host: &str) -> LookupFuture {
            Box::pin(std::future::pending())
        }
    }

    #[derive(Debug)]
    struct Fails;

    impl DnsLookup for Fails {
        fn lookup(&self, _host: &str) -> LookupFuture {
            Box::pin(async { Err(io::Error::other("nxdomain")) })
        }
    }

    fn ip(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    #[tokio::test]
    async fn bounded_lookup_dedupes_in_resolver_order() {
        let table = Table(vec![ip("2001:db8::1"), ip("192.0.2.1"), ip("2001:db8::1")]);
        let addrs = bounded_lookup(&table, "example.test", Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(addrs, vec![ip("2001:db8::1"), ip("192.0.2.1")]);
    }

    #[tokio::test]
    async fn bounded_lookup_maps_empty_failure_and_timeout() {
        let empty = bounded_lookup(&Table(Vec::new()), "a.test", Duration::from_secs(1)).await;
        assert!(matches!(empty, Err(FetchError::DnsNoAddresses { host }) if host == "a.test"));

        let failed = bounded_lookup(&Fails, "b.test", Duration::from_secs(1)).await;
        assert!(
            matches!(failed, Err(FetchError::Dns { ref message, .. }) if message.contains("nxdomain")),
            "got {failed:?}"
        );

        let hung = bounded_lookup(&Hangs, "c.test", Duration::from_millis(20)).await;
        assert!(
            matches!(hung, Err(FetchError::Dns { ref message, .. }) if message.contains("timed out")),
            "got {hung:?}"
        );
    }

    #[tokio::test]
    async fn resolver_answers_pins_with_every_address_and_port_zero() {
        // The lookup would answer something else entirely: a pinned name
        // must never reach it (no second lookup, no rebinding window).
        let resolver = PinningResolver::new(
            Arc::new(Table(vec![ip("10.0.0.1")])),
            Duration::from_secs(1),
        );
        resolver.pin("feed.test", vec![ip("2001:db8::1"), ip("192.0.2.1")]);
        let addrs: Vec<SocketAddr> = resolver
            .resolve(Name::from_str("feed.test").unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(
            addrs,
            vec![
                SocketAddr::new(ip("2001:db8::1"), 0),
                SocketAddr::new(ip("192.0.2.1"), 0)
            ]
        );

        // A re-pin replaces the set.
        resolver.pin("feed.test", vec![ip("192.0.2.9")]);
        let addrs: Vec<SocketAddr> = resolver
            .resolve(Name::from_str("feed.test").unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(addrs, vec![SocketAddr::new(ip("192.0.2.9"), 0)]);
    }

    #[tokio::test]
    async fn resolver_bounds_unpinned_lookups() {
        let resolver = PinningResolver::new(
            Arc::new(Table(vec![ip("192.0.2.7")])),
            Duration::from_secs(1),
        );
        let addrs: Vec<SocketAddr> = resolver
            .resolve(Name::from_str("proxy.test").unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(addrs, vec![SocketAddr::new(ip("192.0.2.7"), 0)]);

        let hung = PinningResolver::new(Arc::new(Hangs), Duration::from_millis(20));
        let err = hung
            .resolve(Name::from_str("proxy.test").unwrap())
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("timed out"), "got {err}");
    }
}
