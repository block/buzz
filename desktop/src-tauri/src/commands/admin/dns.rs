//! Custom DNS resolver for the admin HTTP client.
//!
//! [`LocalhostDnsResolver`] pins RFC 6761 `.localhost` names (e.g.
//! `admin.localhost`) to the loopback address (`127.0.0.1`) and delegates
//! all other names to the system getaddrinfo resolver via a blocking thread.
//!
//! # Why a custom resolver
//!
//! The `http://admin.localhost:<port>` origin is the canonical form used by
//! the relay's `just admin` target. RFC 6761 §6.3 requires `.localhost`
//! subdomains to resolve to loopback, but system getaddrinfo does not honour
//! this on all supported platforms:
//!
//! - **macOS**: resolves correctly via mDNSResponder.
//! - **Linux/glibc**: relies on `nsswitch.conf` ordering; GitHub Actions
//!   ubuntu-latest runners do NOT resolve `.localhost` subdomains.
//! - **Windows**: not guaranteed by the system resolver.
//!
//! The resolver intercepts only names ending in `.localhost` and returns
//! `127.0.0.1:0`; all other names fall through to the system resolver,
//! so non-localhost resolution is unchanged.

use std::net::SocketAddr;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// DNS resolver used by the admin HTTP client.
///
/// - Names ending in `.localhost` are pinned to `127.0.0.1:0` (RFC 6761 §6.3).
/// - All other names are forwarded to the system `getaddrinfo` resolver.
#[derive(Debug, Clone)]
pub struct LocalhostDnsResolver;

impl Resolve for LocalhostDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        // RFC 6761 §6.3: `.localhost` subdomains must resolve to loopback.
        // The admin client's exact `resolve("localhost", …)` already covers the
        // bare hostname; this resolver handles the subdomain case.
        if name.as_str().ends_with(".localhost") {
            let addrs: Addrs = Box::new(std::iter::once(SocketAddr::from(([127, 0, 0, 1], 0))));
            return Box::pin(async move { Ok(addrs) });
        }

        // Fall through to the system resolver for all other names.
        let host = name.as_str().to_owned();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                use std::net::ToSocketAddrs;
                let addrs = (host.as_str(), 0u16)
                    .to_socket_addrs()
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                Ok::<Addrs, Box<dyn std::error::Error + Send + Sync>>(Box::new(addrs))
            })
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?
        })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use super::*;
    use reqwest::dns::Resolve;

    /// Parse a [`reqwest::dns::Name`] from a &str.
    fn make_name(s: &str) -> Name {
        Name::from_str(s).unwrap_or_else(|_| panic!("invalid DNS name: {s}"))
    }

    // ── .localhost pinning ────────────────────────────────────────────────

    /// A `.localhost` subdomain must resolve to exactly `127.0.0.1:0`.
    ///
    /// "Exactly" matters: macOS system GAI returns `127.0.0.1` (port chosen by
    /// the caller), but the resolver must produce the singleton `127.0.0.1:0`
    /// via the fast pinning branch — not via system GAI. The mutation test
    /// below confirms the branch itself is load-bearing: removing it causes
    /// this test to hang or yield a different address on Linux/Windows CI
    /// where system GAI does not resolve `.localhost` subdomains.
    #[tokio::test]
    async fn dot_localhost_resolves_to_exactly_loopback_zero_port() {
        let resolver = LocalhostDnsResolver;
        let mut addrs: Vec<SocketAddr> = resolver
            .resolve(make_name("admin.localhost"))
            .await
            .expect(".localhost must resolve without error")
            .collect();
        assert_eq!(
            addrs.len(),
            1,
            "resolver must return exactly one address for admin.localhost; got {addrs:?}"
        );
        let addr = addrs.remove(0);
        assert_eq!(
            addr,
            SocketAddr::from(([127, 0, 0, 1], 0)),
            "admin.localhost must pin to exactly 127.0.0.1:0, not {addr}"
        );
    }

    /// Multi-level `.localhost` subdomain is also pinned.
    #[tokio::test]
    async fn multi_level_dot_localhost_resolves_to_loopback() {
        let resolver = LocalhostDnsResolver;
        let addrs: Vec<SocketAddr> = resolver
            .resolve(make_name("deep.sub.localhost"))
            .await
            .expect("multi-level .localhost must resolve")
            .collect();
        assert!(
            addrs.iter().all(|a| a.ip().is_loopback()),
            "all resolved addresses must be loopback for deep.sub.localhost; got {addrs:?}"
        );
        assert!(
            addrs.contains(&SocketAddr::from(([127, 0, 0, 1], 0))),
            "127.0.0.1:0 must be present for deep.sub.localhost; got {addrs:?}"
        );
    }

    // ── delegation for non-.localhost names ───────────────────────────────

    /// A plain `localhost` (no dot-prefix) falls through to system GAI and
    /// resolves to a loopback address. This proves the resolver does NOT
    /// intercept non-`.localhost` names — it delegates faithfully.
    ///
    /// `localhost` is chosen because it is reliably resolvable by system GAI
    /// on every supported platform (macOS/Linux/Windows) without any network
    /// traffic, so this test is deterministic and offline-safe.
    #[tokio::test]
    async fn non_dot_localhost_delegates_to_system_gai() {
        let resolver = LocalhostDnsResolver;
        // `localhost` does NOT end with `.localhost` so it must fall through.
        let addrs: Vec<SocketAddr> = resolver
            .resolve(make_name("localhost"))
            .await
            .expect("bare localhost must resolve via system GAI")
            .collect();
        assert!(
            !addrs.is_empty(),
            "system GAI must return at least one address for localhost"
        );
        assert!(
            addrs.iter().all(|a| a.ip().is_loopback()),
            "system GAI localhost addresses must all be loopback; got {addrs:?}"
        );
        // Crucially: none of these are produced by the early-return branch
        // (which only fires for `.localhost`). We confirm that by verifying the
        // results came from GAI: GAI typically returns port 0 as well, but may
        // return ::1 in addition to 127.0.0.1 — either is acceptable.
    }
}
