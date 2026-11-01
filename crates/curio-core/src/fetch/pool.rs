//! The pooled reqwest clients behind [`super::PolicedClient`]: one per
//! (TLS backend, SSRF policy), built on first use and kept for the policed
//! client's lifetime, so keep-alive and TLS session reuse span feeds,
//! favicons and images on the same host. The User-Agent rides each request
//! rather than the client, so UA classes share a pool.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use url::Url;

use super::FetchError;
use super::client::map_reqwest;
use super::resolve::{DnsLookup, PinningResolver};

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

/// The pool registry, plus the DNS seam every lookup goes through.
#[derive(Debug)]
pub(super) struct Pools {
    lookup: Arc<dyn DnsLookup>,
    connect_timeout: Duration,
    request_timeout: Duration,
    built: Mutex<HashMap<PoolKey, Pool>>,
}

impl Pools {
    pub(super) fn new(
        lookup: Arc<dyn DnsLookup>,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Self {
        Self {
            lookup,
            connect_timeout,
            request_timeout,
            built: Mutex::new(HashMap::new()),
        }
    }

    /// The DNS seam (shared with every pool's resolver).
    pub(super) fn lookup(&self) -> &dyn DnsLookup {
        self.lookup.as_ref()
    }

    /// The pooled reqwest client for this hop, with the hop's screened
    /// address set pinned into its resolver. Redirects are disabled (the
    /// policed client follows them itself). A new connection dials only the
    /// pinned addresses — every one, in resolver order, so a dead first
    /// address falls back to the next — closing the DNS-rebinding window
    /// between the check and the connect; a pooled connection is reused as
    /// is (its address passed the same screen when it was opened).
    pub(super) fn client(
        &self,
        url: &Url,
        native_tls: bool,
        allow_private: bool,
        screened: Option<Vec<IpAddr>>,
    ) -> Result<reqwest::Client, FetchError> {
        let key = PoolKey {
            native_tls,
            allow_private,
        };
        let pool = {
            let mut built = self.built.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(pool) = built.get(&key) {
                pool.clone()
            } else {
                let pool = self.build(url, native_tls)?;
                built.insert(key, pool.clone());
                pool
            }
        };
        if let (Some(addrs), Some(domain)) = (screened, url.domain()) {
            pool.resolver.pin(domain, addrs);
        }
        Ok(pool.http)
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
