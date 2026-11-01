//! DNS for the policed client: the [`DnsLookup`] seam every resolution goes
//! through, the timeout that bounds it, and the pinning resolver the pooled
//! reqwest clients connect through.
//!
//! The SSRF screen runs on the result of one bounded lookup per hop
//! (`Pools::screen` in `pool.rs`); the screened address set is then
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
use std::time::{Duration, Instant};

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

/// Below this many pins the resolver never sweeps: a sweep is a linear scan,
/// and a few hundred small entries are not worth one.
const SWEEP_FLOOR: usize = 256;

/// The resolver installed in every pooled reqwest client. Answers a pinned
/// name with its screened address set; anything else gets a bounded lookup.
///
/// Pins are per client, and clients are keyed by SSRF policy (see
/// `pool.rs`), so a deny-private client only ever holds addresses that
/// passed the deny-private screen — and never shares a pooled connection
/// with an `allow_private_network` request.
#[derive(Debug)]
pub(super) struct PinningResolver {
    pins: Mutex<Pins>,
    lookup: Arc<dyn DnsLookup>,
    timeout: Duration,
    /// How long a pin must survive (see `pool.rs`'s `pin_ttl`): older pins
    /// are swept once the map outgrows [`Pins::sweep_at`].
    ttl: Duration,
}

/// The pin map, bounded by sweeping pins older than the resolver's TTL.
#[derive(Debug)]
struct Pins {
    /// Domain → the address set the most recent pre-flight screened, and
    /// when it was pinned.
    by_domain: HashMap<String, (Arc<[IpAddr]>, Instant)>,
    /// Sweep when the map reaches this size; reset to twice the survivors
    /// after each sweep, so the scan stays amortised O(1) per pin.
    sweep_at: usize,
}

impl PinningResolver {
    pub(super) fn new(lookup: Arc<dyn DnsLookup>, timeout: Duration, ttl: Duration) -> Self {
        Self {
            pins: Mutex::new(Pins {
                by_domain: HashMap::new(),
                sweep_at: SWEEP_FLOOR,
            }),
            lookup,
            timeout,
            ttl,
        }
    }

    /// Pins `domain` to `addrs`, which the caller has already screened
    /// under this client's policy, replacing any earlier pin. Every request
    /// re-pins its host just before it is sent, but concurrent requests to
    /// one host on one pool race: the last pin wins, so a new connection
    /// may dial the set another in-flight request screened. That is
    /// harmless — both sets passed the same policy, because the pool is
    /// keyed by `allow_private` and the trusted addresses are config-wide.
    pub(super) fn pin(&self, domain: &str, addrs: Vec<IpAddr>) {
        let mut pins = self.pins.lock().unwrap_or_else(PoisonError::into_inner);
        pins.by_domain
            .insert(domain.to_owned(), (addrs.into(), Instant::now()));
        if pins.by_domain.len() >= pins.sweep_at {
            let ttl = self.ttl;
            pins.by_domain
                .retain(|_, (_, pinned_at)| pinned_at.elapsed() < ttl);
            pins.sweep_at = pins.by_domain.len().saturating_mul(2).max(SWEEP_FLOOR);
        }
    }

    fn pinned(&self, domain: &str) -> Option<Arc<[IpAddr]>> {
        self.pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .by_domain
            .get(domain)
            .map(|(addrs, _)| Arc::clone(addrs))
    }

    #[cfg(test)]
    fn pin_count(&self) -> usize {
        self.pins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .by_domain
            .len()
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
        // Unpinned: the only way to a pooled request is `Pools::request`,
        // which pins the screened host first (the `Screened` type enforces
        // it, and the fetch tests pin that hyper asks for the very name we
        // pinned), so this is only ever the HTTP(S)_PROXY host from the
        // user's own environment — user configuration, resolved (bounded)
        // like the system resolver did before pooling, not screened as feed
        // content: a proxy on a private address is a normal setup.
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
            Duration::from_secs(60),
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
            Duration::from_secs(60),
        );
        let addrs: Vec<SocketAddr> = resolver
            .resolve(Name::from_str("proxy.test").unwrap())
            .await
            .unwrap()
            .collect();
        assert_eq!(addrs, vec![SocketAddr::new(ip("192.0.2.7"), 0)]);

        let hung = PinningResolver::new(
            Arc::new(Hangs),
            Duration::from_millis(20),
            Duration::from_secs(60),
        );
        let err = hung
            .resolve(Name::from_str("proxy.test").unwrap())
            .await
            .err()
            .unwrap();
        assert!(err.to_string().contains("timed out"), "got {err}");
    }

    fn numbered(n: usize) -> String {
        format!("host-{n}.test")
    }

    #[test]
    fn live_pins_survive_a_sweep() {
        // A TTL no test outlives: nothing is old enough to sweep, so the
        // map holds every pin and the next sweep backs off to twice that.
        let resolver = PinningResolver::new(
            Arc::new(Table(Vec::new())),
            Duration::from_secs(1),
            Duration::from_secs(3600),
        );
        for n in 0..SWEEP_FLOOR + 10 {
            resolver.pin(&numbered(n), vec![ip("192.0.2.1")]);
        }
        assert_eq!(resolver.pin_count(), SWEEP_FLOOR + 10);
        assert_eq!(
            resolver.pins.lock().unwrap().sweep_at,
            SWEEP_FLOOR * 2,
            "the sweep at the floor kept all {SWEEP_FLOOR} pins and backed off"
        );
        assert!(resolver.pinned(&numbered(0)).is_some());
    }

    #[test]
    fn expired_pins_are_swept_once_the_map_outgrows_the_floor() {
        let resolver = PinningResolver::new(
            Arc::new(Table(Vec::new())),
            Duration::from_secs(1),
            Duration::ZERO,
        );
        for n in 0..SWEEP_FLOOR - 1 {
            resolver.pin(&numbered(n), vec![ip("192.0.2.1")]);
        }
        // Under the floor nothing is swept, however old.
        assert_eq!(resolver.pin_count(), SWEEP_FLOOR - 1);
        // Reaching it sweeps every expired pin — with a zero TTL, all of
        // them, the one just pinned included.
        resolver.pin("last.test", vec![ip("192.0.2.2")]);
        assert_eq!(resolver.pin_count(), 0);
        assert_eq!(resolver.pins.lock().unwrap().sweep_at, SWEEP_FLOOR);
    }
}
