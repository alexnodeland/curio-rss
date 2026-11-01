//! The pooled reqwest clients behind [`super::PolicedClient`]: one per
//! (TLS backend, SSRF policy), built on first use and kept for the policed
//! client's lifetime, so keep-alive and TLS session reuse span feeds,
//! favicons and images on the same host. The User-Agent rides each request
//! rather than the client, so UA classes share a pool.
//!
//! The SSRF screen lives here too, because the two are one invariant: a
//! pooled connection may only ever dial an address the screen passed.
//! [`Pools::screen`] is the only way to a [`Screened`] hop, and
//! [`Pools::request`] — which takes that hop by value and pins its address
//! set before handing out a request for exactly the screened URL — is the
//! only way to a pooled request. A send path therefore cannot skip the
//! screen, nor pin one host and request another.

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use reqwest::Method;
use url::{Host, Url};

use super::client::map_reqwest;
use super::resolve::{self, DnsLookup, PinningResolver};
use super::{FetchError, policy};

/// What a pooled reqwest client is keyed by. The TLS backend is fixed at
/// build time; the SSRF policy is part of the key so a deny-private request
/// never reuses a connection an `allow_private_network` request opened (to
/// an address the deny-private screen would have refused).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PoolKey {
    native_tls: bool,
    allow_private: bool,
}

/// One pooled reqwest client and the resolver its connections dial through.
#[derive(Debug, Clone)]
struct Pool {
    http: reqwest::Client,
    resolver: Arc<PinningResolver>,
}

/// One hop's target after the SSRF screen. Only [`Pools::screen`] builds
/// one, and only [`Pools::request`] consumes it.
#[derive(Debug)]
pub(super) struct Screened {
    url: Url,
    allow_private: bool,
    /// Every address a domain-name host resolved to, in resolver order, all
    /// screened. `None` for an IP-literal host: checked in place, never
    /// resolved (hyper dials a literal without asking the resolver).
    addrs: Option<Vec<IpAddr>>,
}

/// The pool registry, plus the DNS seam and the address policy every hop is
/// screened against.
#[derive(Debug)]
pub(super) struct Pools {
    lookup: Arc<dyn DnsLookup>,
    /// Harness-only SSRF exemptions ([`super::FetchConfig::trusted_addrs`]).
    trusted_addrs: HashSet<IpAddr>,
    connect_timeout: Duration,
    request_timeout: Duration,
    built: Mutex<HashMap<PoolKey, Pool>>,
}

impl Pools {
    pub(super) fn new(
        lookup: Arc<dyn DnsLookup>,
        trusted_addrs: HashSet<IpAddr>,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Self {
        Self {
            lookup,
            trusted_addrs,
            connect_timeout,
            request_timeout,
            built: Mutex::new(HashMap::new()),
        }
    }

    /// Resolves the URL's host (one lookup, bounded by the connect
    /// timeout, for domain names) and applies the address policy to *every*
    /// resolved address — one private record poisons the lot, so
    /// split-horizon/rebinding games with mixed record sets cannot slip
    /// through.
    ///
    /// # Errors
    ///
    /// [`FetchError::NoHost`], the DNS errors of [`resolve::bounded_lookup`],
    /// or [`FetchError::PrivateAddress`].
    pub(super) async fn screen(
        &self,
        url: &Url,
        allow_private: bool,
    ) -> Result<Screened, FetchError> {
        let host = url.host().ok_or_else(|| FetchError::NoHost {
            url: url.to_string(),
        })?;
        let addrs = match host {
            Host::Ipv4(v4) => {
                self.check_addr(url, IpAddr::V4(v4), allow_private)?;
                None
            }
            Host::Ipv6(v6) => {
                self.check_addr(url, IpAddr::V6(v6), allow_private)?;
                None
            }
            Host::Domain(domain) => {
                let addrs =
                    resolve::bounded_lookup(self.lookup.as_ref(), domain, self.connect_timeout)
                        .await?;
                for addr in &addrs {
                    self.check_addr(url, *addr, allow_private)?;
                }
                Some(addrs)
            }
        };
        Ok(Screened {
            url: url.clone(),
            allow_private,
            addrs,
        })
    }

    fn check_addr(&self, url: &Url, addr: IpAddr, allow_private: bool) -> Result<(), FetchError> {
        if allow_private || self.trusted_addrs.contains(&addr) || policy::is_public_address(addr) {
            return Ok(());
        }
        Err(FetchError::PrivateAddress {
            host: url.host_str().unwrap_or_default().to_owned(),
            addr,
        })
    }

    /// A `method` request for the screened URL on the pooled client of the
    /// hop's policy, with the screened address set pinned into that
    /// client's resolver. Redirects are disabled (the policed client follows
    /// them itself). A new connection dials only the pinned addresses —
    /// every one, in resolver order, so a dead first address falls back to
    /// the next — closing the DNS-rebinding window between the check and
    /// the connect; a pooled connection is reused as is (its address passed
    /// the same screen when it was opened).
    ///
    /// # Errors
    ///
    /// A transport error if the pooled client cannot be built.
    pub(super) fn request(
        &self,
        screened: Screened,
        native_tls: bool,
        method: Method,
    ) -> Result<reqwest::RequestBuilder, FetchError> {
        let Screened {
            url,
            allow_private,
            addrs,
        } = screened;
        let pool = self.pool(
            &url,
            PoolKey {
                native_tls,
                allow_private,
            },
        )?;
        if let (Some(addrs), Some(domain)) = (addrs, url.domain()) {
            pool.resolver.pin(domain, addrs);
        }
        Ok(pool.http.request(method, url))
    }

    /// The pool for `key`, built on first use. The build (which loads TLS
    /// roots) runs outside the registry lock so concurrent first requests
    /// never park a runtime worker on it; if two race, the first insert
    /// wins and the other build is dropped.
    fn pool(&self, url: &Url, key: PoolKey) -> Result<Pool, FetchError> {
        if let Some(pool) = self.registry().get(&key) {
            return Ok(pool.clone());
        }
        let pool = self.build(url, key.native_tls)?;
        Ok(self.registry().entry(key).or_insert(pool).clone())
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, HashMap<PoolKey, Pool>> {
        self.built.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn build(&self, url: &Url, native_tls: bool) -> Result<Pool, FetchError> {
        // rustls is the deliberate default for every host; a per-host override
        // (reddit.com) opts into the platform-native stack to dodge a
        // ClientHello-fingerprint block. Both backends are compiled in.
        let tls = if native_tls {
            reqwest::Client::builder().use_native_tls()
        } else {
            reqwest::Client::builder().use_rustls_tls()
        };
        let resolver = Arc::new(PinningResolver::new(
            Arc::clone(&self.lookup),
            self.connect_timeout,
            pin_ttl(self.connect_timeout, self.request_timeout),
        ));
        let http = tls
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(self.connect_timeout)
            .timeout(self.request_timeout)
            .dns_resolver(Arc::clone(&resolver))
            .build()
            .map_err(|err| map_reqwest(url, &err))?;
        Ok(Pool { http, resolver })
    }
}

/// How long a pin must survive. A pin is only ever read when a connection
/// starts, and every connection starts inside the send of a request that
/// re-pinned its host immediately before sending — a send the client's
/// total timeout bounds (the connect timeout covers the dial itself). Twice
/// their sum is a generous margin; a pin older than that is dead weight the
/// resolver may sweep.
fn pin_ttl(connect_timeout: Duration, request_timeout: Duration) -> Duration {
    connect_timeout
        .saturating_add(request_timeout)
        .saturating_mul(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_ttl_outlives_every_send_it_serves() {
        let ttl = pin_ttl(Duration::from_secs(10), Duration::from_secs(30));
        assert_eq!(ttl, Duration::from_secs(80));
        // Saturates instead of panicking on absurd configuration.
        assert_eq!(pin_ttl(Duration::MAX, Duration::MAX), Duration::MAX);
    }
}
