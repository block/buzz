//! Row-zero host binding: resolve the request's community from the connection
//! host *before* any handler observes tenant data.
//!
//! Conformance "row zero": `req.community = resolve_host(connection.host)`,
//! bound at connection establishment. The host is the authoritative selector;
//! an unknown or unmapped host fails closed with a generic rejection and never
//! falls through to a default tenant. A client-supplied community (e.g. a token
//! stamp or an `h` tag) may narrow or authenticate authority but can never
//! override the host-derived community.
//!
//! This module owns the *seam* (the [`HostResolver`] trait and the fail-closed
//! [`bind_community`] helper) and the relay-side call site. The DB-backed
//! implementation that queries the `communities` table lives in `buzz-db`
//! (`Db::resolve_host`); the relay depends on the trait, not the query, so the
//! binding is testable without a database.

use buzz_core::tenant::{normalize_host, CommunityId, TenantContext};

/// Resolves a normalized connection host to its community, or `None` when the
/// host maps to no community on this deployment.
///
/// Implementors MUST treat the input as already normalized by
/// [`buzz_core::tenant::normalize_host`] — [`bind_community`] guarantees that,
/// so the stored `communities.host` key and the lookup key agree by
/// construction (the column is `UNIQUE(lower(host))`, frozen in migration
/// `0001`).
///
/// Uses a native `async fn` in trait (no `async-trait` dependency). The relay
/// holds a concrete resolver (`Db`), so callers are generic over `R:
/// HostResolver` and never need `dyn` dispatch.
pub trait HostResolver: Send + Sync {
    /// The error type surfaced when the lookup itself fails (e.g. the database
    /// is unreachable). This is distinct from "host not mapped", which is a
    /// successful lookup returning `None`.
    type Error;

    /// Look up the community for an already-normalized host.
    ///
    /// `Ok(Some(_))` — host maps to a community.
    /// `Ok(None)` — host is valid input but maps to nothing (fail closed).
    /// `Err(_)` — the lookup could not be performed.
    fn resolve_host(
        &self,
        normalized_host: &str,
    ) -> impl std::future::Future<Output = Result<Option<CommunityId>, Self::Error>> + Send;
}

/// The outcome of attempting to bind a request to a community.
#[derive(Debug)]
pub enum BindError<E> {
    /// The host did not map to any community on this deployment. Callers MUST
    /// reject the request with a *generic* error — never echo the host back or
    /// distinguish "unmapped" from other failures, so an unauthenticated
    /// caller cannot probe which hosts exist.
    UnmappedHost,
    /// The resolution lookup itself failed (e.g. database error). Treated as
    /// fail-closed: the request is rejected, never admitted to a default tenant.
    Lookup(E),
}

/// Bind a raw connection host to a [`TenantContext`], failing closed.
///
/// This is the single row-zero entry point. It normalizes the host with the
/// one shared rule, resolves it, and on any non-success (unmapped *or* lookup
/// error) returns a [`BindError`] the caller turns into a generic rejection.
/// There is deliberately no path that yields a default or fallback community.
///
/// The returned [`TenantContext`] carries the *normalized* host, so downstream
/// NIP-05 / audit labelling and the NIP-98 `u`-host check all see the same
/// canonical form the community was resolved from.
pub async fn bind_community<R: HostResolver>(
    resolver: &R,
    raw_host: &str,
) -> Result<TenantContext, BindError<R::Error>> {
    let host = normalize_host(raw_host);
    match resolver.resolve_host(&host).await {
        Ok(Some(community)) => Ok(TenantContext::resolved(community, host)),
        Ok(None) => Err(BindError::UnmappedHost),
        Err(e) => Err(BindError::Lookup(e)),
    }
}

/// Resolve the deployment's own community from the configured relay URL host.
///
/// For server-internal paths that have no inbound request `Host` header — the
/// git Smart-HTTP transport, the localhost pre-receive hook callback, the
/// workflow execution sink, and startup tasks — the tenant cannot come from a
/// connection. A relay deployment serves a single canonical host (its
/// `relay_url`), so we resolve that host through the same fail-closed
/// [`bind_community`] path. This is deliberately NOT a default/fallback
/// community: an unmapped `relay_url` host returns the same [`BindError`] as
/// any other unmapped host.
pub async fn bind_deployment_community<R: HostResolver>(
    resolver: &R,
    relay_url: &str,
) -> Result<TenantContext, BindError<R::Error>> {
    bind_community(resolver, &relay_url_authority(relay_url)).await
}

/// Extract the relay URL authority in the same normalized shape as request
/// `Host` headers and `communities.host`: host plus an explicit non-default
/// port, if present.
fn relay_url_authority(relay_url: &str) -> String {
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

/// Production [`HostResolver`]: the relay resolves hosts against the durable
/// `communities` host map in Postgres.
///
/// This is the *only* place the relay couples the row-zero seam to buzz-db. The
/// trait keeps `bind_community` and every call site database-free and testable;
/// this impl is the thin adapter from buzz-db's `lookup_community_by_host`
/// (which returns a `CommunityRecord`) to the seam's `CommunityId`. A lookup
/// that succeeds but finds no row is `Ok(None)` — fail-closed, never a default
/// tenant; a lookup that *fails* (DB unreachable) is `Err`, also fail-closed.
impl HostResolver for buzz_db::Db {
    type Error = buzz_db::DbError;

    async fn resolve_host(
        &self,
        normalized_host: &str,
    ) -> Result<Option<CommunityId>, Self::Error> {
        Ok(self
            .lookup_community_by_host(normalized_host)
            .await?
            .map(|record| record.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    /// In-memory resolver over a fixed host→community map, so the binding seam
    /// is testable without a database.
    struct MapResolver {
        map: HashMap<String, CommunityId>,
        fail: bool,
    }

    impl HostResolver for MapResolver {
        type Error = &'static str;
        async fn resolve_host(
            &self,
            normalized_host: &str,
        ) -> Result<Option<CommunityId>, Self::Error> {
            if self.fail {
                return Err("db down");
            }
            Ok(self.map.get(normalized_host).copied())
        }
    }

    fn resolver_with(host: &str, id: u128) -> MapResolver {
        let mut map = HashMap::new();
        map.insert(
            host.to_string(),
            CommunityId::from_uuid(Uuid::from_u128(id)),
        );
        MapResolver { map, fail: false }
    }

    #[tokio::test]
    async fn maps_known_host_to_its_community() {
        let r = resolver_with("relay.example", 1);
        let ctx = bind_community(&r, "relay.example").await.expect("bound");
        assert_eq!(ctx.community().as_uuid(), &Uuid::from_u128(1));
        assert_eq!(ctx.host(), "relay.example");
    }

    #[tokio::test]
    async fn normalizes_before_lookup_so_variants_resolve_to_one_tenant() {
        // The map holds the canonical form; case/dot/default-port variants must
        // all bind to the same community (they cannot split a tenant).
        let r = resolver_with("relay.example", 7);
        for variant in ["RELAY.EXAMPLE", "relay.example.", "relay.example:443"] {
            let ctx = bind_community(&r, variant)
                .await
                .unwrap_or_else(|_| panic!("variant {variant:?} should bind"));
            assert_eq!(
                ctx.community().as_uuid(),
                &Uuid::from_u128(7),
                "variant {variant:?}"
            );
            assert_eq!(ctx.host(), "relay.example", "variant {variant:?}");
        }
    }

    #[tokio::test]
    async fn deployment_url_keeps_nondefault_port_for_lookup() {
        let r = resolver_with("localhost:3000", 42);
        let ctx = bind_deployment_community(&r, "ws://localhost:3000")
            .await
            .expect("deployment host should bind with non-default port");
        assert_eq!(ctx.community().as_uuid(), &Uuid::from_u128(42));
        assert_eq!(ctx.host(), "localhost:3000");

        let wrong = resolver_with("localhost", 42);
        let err = bind_deployment_community(&wrong, "ws://localhost:3000")
            .await
            .unwrap_err();
        assert!(matches!(err, BindError::UnmappedHost));
    }

    #[tokio::test]
    async fn deployment_url_normalizes_default_ports() {
        let r = resolver_with("relay.example", 9);
        for url in ["ws://relay.example:80", "wss://relay.example:443"] {
            let ctx = bind_deployment_community(&r, url)
                .await
                .unwrap_or_else(|_| panic!("url {url:?} should bind"));
            assert_eq!(ctx.community().as_uuid(), &Uuid::from_u128(9));
            assert_eq!(ctx.host(), "relay.example", "url {url:?}");
        }
    }

    #[test]
    fn relay_url_authority_preserves_ipv6_brackets() {
        assert_eq!(relay_url_authority("ws://[::1]:3000"), "[::1]:3000");
        assert_eq!(relay_url_authority("wss://[::1]:443"), "[::1]");
    }

    #[tokio::test]
    async fn unmapped_host_fails_closed() {
        let r = resolver_with("relay.example", 1);
        let err = bind_community(&r, "evil.example").await.unwrap_err();
        assert!(matches!(err, BindError::UnmappedHost));
    }

    #[tokio::test]
    async fn lookup_error_fails_closed_not_default_tenant() {
        let r = MapResolver {
            map: HashMap::new(),
            fail: true,
        };
        let err = bind_community(&r, "relay.example").await.unwrap_err();
        assert!(matches!(err, BindError::Lookup("db down")));
    }

    // ── Red-team Attack 2: host spoof / empty-host defense-in-depth ─────────
    //
    // Spec property pinned: row-zero invariant
    //   req.community = resolve_host(connection.host)
    // (see `tenant.rs` module docs and `docs/spec/MultiTenantRelay.tla`'s
    // host-binding seam — every observation in community C must be reachable
    // only from a request whose `connection.host` resolved to C).
    //
    // The fence relies on a structural fact about the host map: no row in
    // `communities` has an empty `host`. The schema (`migrations/0001_initial_schema.sql`
    // line 53-59) declares `host VARCHAR(255) NOT NULL` but does NOT constrain
    // against the empty string, and the unique index is `lower(host)` — `''`
    // is a legal value.
    //
    // Meanwhile every door reads `axum::http::header::HOST` with
    // `.unwrap_or("")` (see `router.rs:169-172`, `audio/handler.rs:74`,
    // `api/{nip05.rs:39, bridge.rs:{174,263,552}, media.rs:112}`), and the
    // bridge derives `raw_host` from a URL whose `host_str()` may be empty for
    // pathological inputs. So if an operator (or a buggy migration) ever
    // inserts a row with `host=''`, a request with no/invalid Host header
    // silently binds to that community — without ever supplying the host.
    //
    // The fence has to defend itself: an empty raw_host MUST fail closed,
    // regardless of what the host map contains. Today it does not.
    mod redteam_attack2 {
        use super::*;

        /// RED gate. Configures a resolver with an `""→CommunityId` mapping
        /// (the schema permits it; no CHECK against empty host exists), then
        /// asks `bind_community` to bind an empty raw_host as a request with
        /// a missing/invalid Host header would. Today this returns
        /// `Ok(TenantContext{community=X})` — the fence collapses to the
        /// misconfigured row. The fix: short-circuit in `bind_community` so
        /// that `normalize_host(raw_host).is_empty()` returns
        /// `Err(BindError::UnmappedHost)` before any resolver lookup.
        ///
        /// Generic-rejection note: we reuse `UnmappedHost` (not a new
        /// `EmptyHost` variant) so the door's response is byte-identical to
        /// any other unmapped host — an unauthenticated caller cannot probe
        /// whether the deployment has an empty-host row.
        ///
        /// Delete this `#[ignore]` when the fix lands; verified RED with
        /// `cargo test -p buzz-relay --include-ignored
        ///   tenant::tests::redteam_attack2::empty_raw_host_fails_closed_even_if_db_has_empty_host_row`
        #[ignore = "RED gate: deletes the empty-host fence collapse — flip to active when fix lands"]
        #[tokio::test]
        async fn empty_raw_host_fails_closed_even_if_db_has_empty_host_row() {
            // Simulate operator misconfig / buggy migration: an empty-host row
            // exists in `communities`. The schema does not forbid this.
            let r = resolver_with("", 0xdeadbeef);

            // A request with a missing or unreadable Host header reaches
            // `bind_community` with raw_host = "" (router.rs:169-172). The
            // fence must reject — the request never supplied a host.
            let err = bind_community(&r, "").await.expect_err(
                "Inv_RowZero: an empty raw_host carries no community evidence; \
                 bind_community must fail closed regardless of the host map",
            );
            assert!(
                matches!(err, BindError::UnmappedHost),
                "fence must produce a generic UnmappedHost (no info leak about \
                 whether an empty-host row exists); got {err:?}",
            );
        }

        /// RED gate. Same property, whitespace-only host: `normalize_host`
        /// trims to empty (`buzz-core::tenant::normalize_host_empty_stays_empty`),
        /// so this is the same fence collapse via a different raw input.
        ///
        /// Delete `#[ignore]` when the fix lands.
        #[ignore = "RED gate: whitespace-only host normalizes to empty; same fence collapse"]
        #[tokio::test]
        async fn whitespace_only_raw_host_fails_closed_even_if_db_has_empty_host_row() {
            let r = resolver_with("", 0xdeadbeef);

            let err = bind_community(&r, "   ").await.expect_err(
                "Inv_RowZero: whitespace-only raw_host normalizes to empty \
                 (see buzz-core::tenant::normalize_host) and carries no \
                 community evidence",
            );
            assert!(
                matches!(err, BindError::UnmappedHost),
                "fence must produce a generic UnmappedHost; got {err:?}",
            );
        }

        /// Negative control: a *non-empty* unmapped host must still fail
        /// closed (this already passes — included so the redteam_attack2
        /// module documents both shapes of the fence's intended behavior and
        /// catches a fix that accidentally over-narrows to only-empty).
        #[tokio::test]
        async fn non_empty_unmapped_host_still_fails_closed_after_fix() {
            let r = resolver_with("", 0xdeadbeef);
            let err = bind_community(&r, "evil.example").await.unwrap_err();
            assert!(matches!(err, BindError::UnmappedHost));
        }
    }
}
