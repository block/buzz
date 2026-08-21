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
/// - loopback aliases collapse: `127.0.0.1` and `[::1]` (with or without a
///   port) canonicalize to `localhost` — they name the same machine, and a
///   deployment seeded under one spelling must also answer the others (the
///   classic local failure: community seeded as `localhost:3999`, browser
///   sends `Host: 127.0.0.1:3999`, resolution fails closed with a 404);
/// - strip a single trailing dot (the FQDN root label);
/// - strip a default port suffix (`:80`, `:443`) — non-default ports are kept,
///   since a deployment may legitimately serve different communities on
///   different ports of the same name.
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
    // Collapse loopback aliases. `127.0.0.1`, `[::1]`, and `localhost` name
    // the same machine, so they must resolve to the same tenant — a community
    // seeded under one spelling has to answer requests arriving with another.
    // Applied after default-port stripping so `[::1]:443` → `[::1]` →
    // `localhost` too; a non-default port (`127.0.0.1:3999`) is preserved.
    // The alias must be the whole host or end at a port boundary — a domain
    // like `127.0.0.1.example` is NOT loopback and stays untouched.
    let loopback_port = ["127.0.0.1", "[::1]"].iter().find_map(|alias| {
        host.strip_prefix(alias)
            .filter(|rest| rest.is_empty() || rest.starts_with(':'))
    });
    if let Some(port) = loopback_port {
        host = format!("localhost{port}");
    }
    host
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
    fn normalize_host_collapses_ipv6_loopback_literal() {
        // The IPv6 loopback literal is a loopback alias like 127.0.0.1 and
        // collapses to `localhost`; non-loopback IPv6 literals stay intact.
        assert_eq!(normalize_host("[::1]"), "localhost");
        assert_eq!(normalize_host("[::1]:3000"), "localhost:3000");
        assert_eq!(normalize_host("[2001:db8::1]"), "[2001:db8::1]");
        assert_eq!(normalize_host("[2001:db8::1]:443"), "[2001:db8::1]");
    }

    #[test]
    fn normalize_host_empty_stays_empty() {
        // Empty / whitespace-only resolves to empty; resolution fails closed.
        assert_eq!(normalize_host(""), "");
        assert_eq!(normalize_host("   "), "");
    }

    #[test]
    fn normalize_host_collapses_loopback_aliases() {
        // All loopback spellings name the same machine and must normalize to
        // the `localhost` form, so a community seeded as `localhost:3999`
        // also answers `Host: 127.0.0.1:3999` (and the IPv6 literal).
        assert_eq!(normalize_host("127.0.0.1:3999"), "localhost:3999");
        assert_eq!(normalize_host("127.0.0.1"), "localhost");
        assert_eq!(normalize_host("127.0.0.1:443"), "localhost");
        assert_eq!(normalize_host("[::1]:3999"), "localhost:3999");
        assert_eq!(normalize_host("[::1]"), "localhost");
        assert_eq!(normalize_host("[::1]:443"), "localhost");
        assert_eq!(normalize_host("LOCALHOST:3999"), "localhost:3999");
        // Non-loopback hosts are untouched.
        assert_eq!(normalize_host("127.0.0.1.example"), "127.0.0.1.example");
        assert_eq!(normalize_host("example.com"), "example.com");
    }

    #[test]
    fn relay_url_authority_keeps_explicit_nondefault_port() {
        // The default dev seed: startup, bind_deployment_community, and
        // buzz-admin must all derive `localhost:3000` (NOT bare `localhost`),
        // or the admin lookup misses the community startup seeded.
        assert_eq!(relay_url_authority("ws://localhost:3000"), "localhost:3000");
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
    fn relay_url_authority_collapses_loopback_aliases() {
        // A relay URL spelled with a loopback IP must derive the same
        // authority as the `localhost` spelling — otherwise startup seeds the
        // community under one host and requests arriving with the other 404.
        assert_eq!(relay_url_authority("ws://127.0.0.1:3999"), "localhost:3999");
        assert_eq!(relay_url_authority("ws://[::1]:3000"), "localhost:3000");
        assert_eq!(relay_url_authority("http://127.0.0.1"), "localhost");
    }

    #[test]
    fn relay_url_authority_preserves_ipv6_brackets() {
        // Non-loopback IPv6 literals keep brackets and port.
        assert_eq!(
            relay_url_authority("ws://[2001:db8::1]:3000"),
            "[2001:db8::1]:3000"
        );
    }

    #[test]
    fn relay_url_authority_unparseable_is_empty() {
        // No parseable host → empty authority; callers fail closed.
        assert_eq!(relay_url_authority("not a url"), "");
        assert_eq!(relay_url_authority(""), "");
    }
}
