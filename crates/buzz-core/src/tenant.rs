//! Tenant identity: the server-resolved community key carried on every scoped path.
//!
//! These types live in `buzz-core` (zero I/O deps) so the DB, auth, pub/sub,
//! search, audit, media, and relay-wiring layers all name a community the same
//! way without depending on each other.
//!
//! ## The fence
//!
//! The whole multi-tenant safety story rests on one invariant from the formal
//! model (conformance "row zero"): a request's community is *resolved from the
//! connection host by the server*, never supplied or influenced by the client.
//!
//! [`TenantContext`] expresses that invariant in the type system as far as the
//! type system can carry it: there is no `Default`, no `Deserialize`, and no
//! way to *parse* a community from client input. A `CommunityId` only ever
//! comes from host resolution or from a DB row the server already scoped.
//!
//! This is a **lint-and-review fence, not a compiler fence.**
//! [`TenantContext::resolved`] and [`CommunityId::from_uuid`] are public so the
//! host-resolution path (in another crate) can call them — which means a
//! determined caller elsewhere *could* call them too. The migration-lint
//! harness forbids constructing a `TenantContext` outside host resolution and
//! tests; the type only removes the *accidental* path (deserializing a
//! client-chosen community), and review/lint closes the deliberate one. We say
//! this plainly rather than overclaim a guarantee the `pub` API doesn't give.

use std::fmt;
use uuid::Uuid;

/// A community: the first-class tenant key on every scoped row.
///
/// Opaque UUID newtype. Equality and ordering are the underlying UUID's.
/// There is deliberately no `community_id` parsed from client input anywhere;
/// a `CommunityId` only ever originates from host resolution or from a DB row
/// the server already scoped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommunityId(Uuid);

impl CommunityId {
    /// Wrap a UUID that the server has already established as a community id
    /// (e.g. read back from the `communities` table during host resolution).
    ///
    /// This is intentionally not a parse-from-client entry point: callers must
    /// already hold a server-trusted UUID.
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// The underlying UUID, for DB binds and Redis key construction.
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Display for CommunityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// The resolved tenant of an in-flight request, bound once at connection /
/// request establishment before any handler observes tenant data.
///
/// Carried by reference (`&TenantContext`) through every scoped call. This is
/// the *only* way to name a community downstream, and it cannot be constructed
/// from client input — see the module-level "fence" note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantContext {
    community: CommunityId,
    host: String,
}

impl TenantContext {
    /// Construct a context from a completed host resolution.
    ///
    /// Call this *only* from the host-resolution path (the function that maps a
    /// connection's host to a `communities` row). Everywhere else takes
    /// `&TenantContext` and reads it; nothing else mints one.
    pub fn resolved(community: CommunityId, host: impl Into<String>) -> Self {
        Self {
            community,
            host: host.into(),
        }
    }

    /// The community every scoped operation under this request must use.
    pub const fn community(&self) -> CommunityId {
        self.community
    }

    /// The host that resolved to this community.
    ///
    /// Authoritative for the NIP-05 domain and audit labelling; never re-derive
    /// the community from it downstream — the community is already fixed.
    pub fn host(&self) -> &str {
        &self.host
    }
}

/// Normalize a connection `Host` into the canonical form used as the community
/// lookup key.
///
/// This is the *one* normalization rule shared by both sides of the fence:
/// the `communities.host` column is stored already-normalized, and host
/// resolution normalizes the incoming `Host` header with this same function
/// before looking it up. Because both sides agree by construction,
/// `Relay.Example`, `relay.example.`, and `relay.example:443` all resolve to
/// the one community — they can never split into distinct tenants.
///
/// Rules (host only — the caller has already split off any path/scheme):
/// - ASCII-lowercase (hosts are case-insensitive per RFC 3986);
/// - strip a single trailing dot (the FQDN root label);
/// - strip a default port suffix (`:80`, `:443`) — non-default ports are kept,
///   since a deployment may legitimately serve different communities on
///   different ports of the same name;
/// - fold loopback spellings (`localhost`, `127.0.0.0/8`, `::1`) to
///   `127.0.0.1`, keeping any non-default port. This matches the loopback
///   rewrite in [`crate::relay::normalize_relay_url`], so a host header and a
///   canonicalized relay URL for the same loopback deployment agree.
///
/// The input is trimmed of surrounding whitespace. An empty result (e.g. the
/// caller passed `""`) is returned as-is; resolution treats an empty or
/// unmapped host as a fail-closed rejection, never a default tenant.
#[must_use]
pub fn normalize_host(host: &str) -> String {
    let host = host.trim();
    let mut host = host.to_ascii_lowercase();
    // Strip default ports. We only touch a `:port` suffix that is exactly a
    // default port, so IPv6 literals like `[::1]` (which contain colons but no
    // trailing `:80`/`:443`) are left intact.
    if let Some(stripped) = host
        .strip_suffix(":443")
        .or_else(|| host.strip_suffix(":80"))
    {
        host = stripped.to_string();
    }
    // Strip a single trailing FQDN-root dot.
    if let Some(stripped) = host.strip_suffix('.') {
        host = stripped.to_string();
    }
    // Fold loopback spellings onto one key, mirroring the client-side
    // canonicalization in `crate::relay::normalize_relay_url`. All loopback
    // spellings address the same machine, so collapsing them cannot widen
    // access across a host boundary — but leaving them distinct splits one
    // loopback deployment into several unreachable tenants.
    if let Some((candidate, port)) = split_host_port(&host) {
        if is_loopback_host(candidate) {
            host = match port {
                Some(port) => format!("{LOOPBACK_HOST}:{port}"),
                None => LOOPBACK_HOST.to_string(),
            };
        }
    }
    host
}

/// Canonical spelling every loopback host folds to. Matches the host
/// `crate::relay::normalize_relay_url` rewrites loopback relay URLs to.
const LOOPBACK_HOST: &str = "127.0.0.1";

/// Split an authority into its host and optional port, keeping bracketed IPv6
/// literals intact (`[::1]:3000` becomes `("::1", Some("3000"))`).
///
/// Returns `None` when the input is not a well-formed authority. The caller
/// must then leave the value untouched: folding a malformed authority would
/// turn input that should fail closed as an unmapped host into a resolvable
/// community key. Rejected here: an unterminated bracket (`[::1`), trailing
/// junk after the bracket (`[::1]evil`), and a non-numeric or empty port
/// (`localhost:abc`).
fn split_host_port(authority: &str) -> Option<(&str, Option<&str>)> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        // Only `[addr]` and `[addr]:port` are well-formed bracketed forms.
        let (address, remainder) = rest.split_once(']')?;
        if remainder.is_empty() {
            (address, None)
        } else {
            (address, Some(remainder.strip_prefix(':')?))
        }
    } else {
        match authority.rsplit_once(':') {
            // A bare (unbracketed) IPv6 literal has several colons and no port.
            Some((head, port)) if !head.contains(':') => (head, Some(port)),
            _ => (authority, None),
        }
    };
    match port {
        Some(port) if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) => None,
        _ => Some((host, port)),
    }
}

/// Whether a host names the loopback interface, by any spelling.
///
/// Mirrors the loopback test in `crate::relay::normalize_relay_url`: the
/// `localhost` name, any address in `127.0.0.0/8`, and the IPv6 `::1`.
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(address) = host.parse::<std::net::Ipv4Addr>() {
        return address.is_loopback();
    }
    if let Ok(address) = host.parse::<std::net::Ipv6Addr>() {
        return address.is_loopback();
    }
    false
}

/// Extract the authority (host plus an explicit non-default port, if present)
/// from a relay URL in the same normalized shape as request `Host` headers and
/// `communities.host`.
///
/// Shared by the relay's host-resolution seam (startup community seeding and
/// the deployment-community bind), the relay's `bind_deployment_community`, and
/// the `buzz-admin` CLI's tenant resolution. All of these must derive the
/// *byte-identical* authority that live request resolution
/// ([`crate::tenant::normalize_host`]) produces from an inbound `Host`, or a
/// bootstrapped/looked-up community lands under a host no request resolves to.
///
/// In particular this preserves an explicit non-default port (`relay:8443` →
/// `relay:8443`) and IPv6 brackets (`[::1]:3000`) — both of which a naive
/// `Url::host_str()` drops. Returns the empty string when `relay_url` has no
/// parseable host (the caller fails closed on empty).
#[must_use]
pub fn relay_url_authority(relay_url: &str) -> String {
    let Ok(url) = url::Url::parse(relay_url) else {
        return String::new();
    };
    let Some(host) = url.host() else {
        return String::new();
    };
    let host = match host {
        url::Host::Domain(domain) => domain.to_string(),
        url::Host::Ipv4(addr) => addr.to_string(),
        url::Host::Ipv6(addr) => format!("[{addr}]"),
    };
    let authority = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    normalize_host(&authority)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_id_roundtrips_uuid() {
        let u = Uuid::from_u128(0x1234_5678_9abc_def0_1122_3344_5566_7788);
        let c = CommunityId::from_uuid(u);
        assert_eq!(c.as_uuid(), &u);
        assert_eq!(c.to_string(), u.to_string());
    }

    #[test]
    fn tenant_context_exposes_resolution_inputs() {
        let u = Uuid::from_u128(1);
        let ctx = TenantContext::resolved(CommunityId::from_uuid(u), "relay.example");
        assert_eq!(ctx.community().as_uuid(), &u);
        assert_eq!(ctx.host(), "relay.example");
    }

    #[test]
    fn normalize_host_collapses_tenant_split_variants() {
        // All of these are the SAME tenant and must normalize identically —
        // this is the property that stops accidental split-tenant.
        let canonical = "relay.example";
        for variant in [
            "relay.example",
            "Relay.Example",
            "RELAY.EXAMPLE",
            "relay.example.",    // trailing FQDN root dot
            "relay.example:443", // default https port
            "relay.example:80",  // default http port
            "Relay.Example.:443",
            "  relay.example  ", // surrounding whitespace
        ] {
            assert_eq!(normalize_host(variant), canonical, "variant {variant:?}");
        }
    }

    #[test]
    fn normalize_host_keeps_nondefault_port() {
        // A non-default port is a legitimate distinct selector — keep it.
        assert_eq!(normalize_host("relay.example:8443"), "relay.example:8443");
        assert_eq!(normalize_host("relay.example:3000"), "relay.example:3000");
    }

    #[test]
    fn normalize_host_leaves_ipv6_literal_intact() {
        // IPv6 literals contain colons but no trailing default-port suffix.
        // A non-loopback literal is used here because loopback spellings fold
        // to 127.0.0.1 — see `normalize_host_folds_loopback_spellings`.
        assert_eq!(normalize_host("[2001:db8::1]"), "[2001:db8::1]");
        assert_eq!(normalize_host("[2001:db8::1]:443"), "[2001:db8::1]");
        assert_eq!(normalize_host("[2001:db8::1]:8443"), "[2001:db8::1]:8443");
    }

    #[test]
    fn normalize_host_folds_loopback_spellings() {
        // Every spelling of the loopback interface is the SAME tenant.
        for variant in ["localhost", "LocalHost", "127.0.0.1", "127.1.2.3", "[::1]"] {
            assert_eq!(normalize_host(variant), "127.0.0.1", "variant: {variant}");
        }
        // A non-default port still selects a distinct community, so it is kept.
        for variant in ["localhost:3000", "127.0.0.1:3000", "[::1]:3000"] {
            assert_eq!(
                normalize_host(variant),
                "127.0.0.1:3000",
                "variant: {variant}"
            );
        }
        // Default ports are stripped before folding, so these collapse together.
        assert_eq!(normalize_host("localhost:80"), "127.0.0.1");
        assert_eq!(normalize_host("[::1]:443"), "127.0.0.1");
    }

    #[test]
    fn normalize_host_does_not_fold_malformed_authority() {
        // A malformed authority must never fold onto a resolvable key: it has
        // to stay unmatched so `bind_community` fails closed. Without this,
        // `Host: [::1]evil` would resolve to the loopback community.
        // Each of these passes through untouched, so it cannot match a stored
        // `communities.host` and resolution rejects it as an unmapped host.
        for malformed in [
            "[::1]evil",
            "[::1]xyz:3000",
            "[::1",
            "[::1]:",
            "[::1]:port",
            "localhost:abc",
            "127.0.0.1:",
        ] {
            assert_eq!(
                normalize_host(malformed),
                malformed,
                "malformed authority must not be rewritten: {malformed}"
            );
        }
    }

    #[test]
    fn normalize_host_does_not_fold_non_loopback() {
        // Only the loopback interface folds — nothing else may be collapsed
        // onto another tenant's key.
        assert_eq!(normalize_host("localhost.example"), "localhost.example");
        assert_eq!(normalize_host("notlocalhost"), "notlocalhost");
        assert_eq!(normalize_host("10.0.0.1"), "10.0.0.1");
        assert_eq!(normalize_host("128.0.0.1"), "128.0.0.1");
        assert_eq!(normalize_host("relay.example:3000"), "relay.example:3000");
    }

    #[test]
    fn normalize_host_agrees_with_relay_url_canonicalization() {
        // The invariant this whole fold exists to hold: clients canonicalize a
        // relay URL with `relay::normalize_relay_url` (which rewrites loopback
        // to 127.0.0.1), while tenant resolution keys on the request `Host`.
        // If the two disagree, one loopback deployment splits into a reachable
        // tenant and an unreachable one.
        for url in [
            "ws://localhost:3000",
            "ws://127.0.0.1:3000",
            "ws://[::1]:3000",
        ] {
            let canonical =
                crate::relay::normalize_relay_url(url).expect("loopback relay URL is valid");
            let authority = canonical
                .strip_prefix("ws://")
                .expect("normalized ws URL keeps its scheme");
            assert_eq!(
                normalize_host(authority),
                normalize_host("localhost:3000"),
                "url: {url}"
            );
        }
    }

    #[test]
    fn normalize_host_empty_stays_empty() {
        // Empty / whitespace-only resolves to empty; resolution fails closed.
        assert_eq!(normalize_host(""), "");
        assert_eq!(normalize_host("   "), "");
    }

    #[test]
    fn relay_url_authority_keeps_explicit_nondefault_port() {
        // The default dev seed: startup, bind_deployment_community, and
        // buzz-admin must all derive the port (NOT a bare host), or the admin
        // lookup misses the community startup seeded. The loopback host folds
        // to 127.0.0.1 (see `normalize_host`), but the port is still kept, and
        // every derivation path folds identically so they still agree.
        assert_eq!(relay_url_authority("ws://localhost:3000"), "127.0.0.1:3000");
        assert_eq!(
            relay_url_authority("wss://relay.example:8443"),
            "relay.example:8443"
        );
    }

    #[test]
    fn relay_url_authority_collapses_default_ports() {
        // Default ports collapse to the bare host, matching how an inbound
        // `Host` header for the same deployment normalizes.
        assert_eq!(
            relay_url_authority("wss://relay.example:443"),
            "relay.example"
        );
        assert_eq!(
            relay_url_authority("ws://relay.example:80"),
            "relay.example"
        );
        assert_eq!(relay_url_authority("wss://relay.example"), "relay.example");
    }

    #[test]
    fn relay_url_authority_preserves_ipv6_brackets() {
        // `host_str()` strips IPv6 brackets and the port; `relay_url_authority`
        // must keep both so the authority matches `communities.host`. Checked
        // with a non-loopback literal, since loopback folds to 127.0.0.1.
        assert_eq!(
            relay_url_authority("ws://[2001:db8::1]:3000"),
            "[2001:db8::1]:3000"
        );
        // The loopback literal folds — and lands on the same authority as the
        // other loopback spellings of the same deployment.
        assert_eq!(relay_url_authority("ws://[::1]:3000"), "127.0.0.1:3000");
        assert_eq!(
            relay_url_authority("ws://[::1]:3000"),
            relay_url_authority("ws://localhost:3000")
        );
    }

    #[test]
    fn relay_url_authority_unparseable_is_empty() {
        // No parseable host → empty authority; callers fail closed.
        assert_eq!(relay_url_authority("not a url"), "");
        assert_eq!(relay_url_authority(""), "");
    }
}
