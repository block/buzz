//! Multi-tenant conformance harness.
//!
//! This file mirrors the obligation table in `docs/multi-tenant-conformance.md`
//! **one row per module**. It is the executable form of the conformance
//! contract: the current single-community relay is the wire-parity *oracle*, and
//! these tests prove two things the rewrite must never break:
//!
//!   1. **N=1 parity** — with one configured host → one default community, every
//!      existing client observes byte-identical behavior. This is asserted by the
//!      *existing* e2e suites (`e2e_relay`, `e2e_media`, `e2e_git`, …) run
//!      with `RELAY_URL` pointed at the new relay; no new test is needed here,
//!      only the documented obligation that those suites stay green unchanged.
//!
//!   2. **A/B isolation** — with two hosts → two communities on the *same* relay
//!      deployment, no tenant-observable state crosses the boundary. These are
//!      the new tests below. They require a running multi-tenant relay with two
//!      host mappings, so they are `#[ignore]` by default and selected with
//!      `--ignored`.
//!
//! # Running the A/B isolation suite
//!
//! ```text
//! RELAY_URL_A=http://a.localhost:3000 \
//! RELAY_URL_B=http://b.localhost:3000 \
//! cargo test -p buzz-test-client --test conformance_multitenant -- --ignored
//! ```
//!
//! Both URLs MUST address the same relay process (same pod, same Postgres, same
//! Redis); only the `Host` header differs. That is the whole point: one binary,
//! one DB, two communities, provably isolated by `community_id` derived from the
//! host — never from caller input.
//!
//! # Status of each row
//!
//! A row is `todo!()`-stubbed until the lane it depends on lands on the
//! integration branch. The stub is intentional and load-bearing: it names the
//! exact obligation so the lane owner fills in *their* row, and a green run can
//! never be faked by an empty body. Lane ownership is noted per module.

#![allow(clippy::todo, unused)]

use std::time::Duration;

/// Host A's base URL (community A). Defaults to a local two-host relay.
fn url_a() -> String {
    std::env::var("RELAY_URL_A").unwrap_or_else(|_| "http://a.localhost:3000".to_string())
}

/// Host B's base URL (community B), same relay process, different host.
fn url_b() -> String {
    std::env::var("RELAY_URL_B").unwrap_or_else(|_| "http://b.localhost:3000".to_string())
}

/// An unmapped/unknown host on the same relay process. No community row maps to
/// it, so the relay must fail closed (404) rather than fall through to a default
/// tenant. `*.localhost` resolves to 127.0.0.1, so this addresses the same relay
/// as `url_a`/`url_b` but presents a `Host` no community is bound to.
fn url_unknown() -> String {
    std::env::var("RELAY_URL_UNKNOWN")
        .unwrap_or_else(|_| "http://unknown.localhost:3000".to_string())
}

/// Marker for a conformance obligation whose lane has not yet landed on the
/// integration branch. Centralizes the "not yet wired" panic so the harvest of
/// remaining work is one grep: `rg pending_lane conformance_multitenant.rs`.
#[track_caller]
fn pending_lane(lane: &str, obligation: &str) -> ! {
    todo!("conformance pending [{lane}]: {obligation}");
}

// ---------------------------------------------------------------------------
// Row zero: request community binding (Eva — relay-wiring)
// ---------------------------------------------------------------------------
mod row_zero_host_binding {
    use super::*;

    /// Obligation: an unknown/unmapped host fails closed with a *generic*
    /// rejection and never falls through to a default tenant.
    ///
    /// This is the wire complement promised by [`super::nip11_relay_info`]:
    /// NIP-11 is deliberately host-agnostic and serves the *identical* static
    /// document to every host (mapped or not) so a 404-vs-200 status difference
    /// cannot become an enumeration oracle on the `nostr+json` path. The
    /// fail-closed binding instead lives on the **WebSocket-upgrade / non-
    /// `nostr+json`** door (`router.rs::nip11_or_ws_handler` →
    /// `tenant::bind_community`), and *that* is what this row asserts.
    ///
    /// Three properties, all wire-observable:
    ///   1. **Fails closed** — an unmapped host does not fall through to a
    ///      default tenant. A non-`nostr+json` request to the unknown host is
    ///      rejected `404`, where a mapped host is *not* 404 (it serves the SPA
    ///      / NIP-11 fallback, or upgrades to WS). The status *difference*
    ///      between mapped and unmapped on this door is the proof the unmapped
    ///      host got no tenant. NOTE: this mapped-200/unmapped-404 status
    ///      *difference* is an intentional, door-scoped distinguisher — it
    ///      exists on the non-`nostr+json` (SPA/WS) door only, where a 404 is
    ///      how an unbound host is signalled. The `nostr+json` door
    ///      deliberately does *not* expose it (see `nip11_relay_info`), which is
    ///      why an unauthenticated NIP-11 probe cannot enumerate communities. A
    ///      future reader must not "fix" this into a 404-everywhere: that would
    ///      break the SPA fallback that legitimately serves mapped hosts here.
    ///   2. **Generic** — the rejection body is the fixed string the relay uses
    ///      for *both* "unmapped" and "lookup error" (`router.rs:181`); it must
    ///      not echo the host or otherwise distinguish the failure mode, so an
    ///      unauthenticated caller cannot probe which communities exist.
    ///   3. **The WS door fails too** — a raw WebSocket handshake to the unknown
    ///      host is rejected at the upgrade (before any frame is read), not
    ///      accepted-then-bound-to-a-default.
    ///
    /// Mutate-bite (would-it-fail-without-the-fix): make `bind_community` fall
    /// through to a default tenant on the unmapped host (e.g. `Err(_) =>` returns
    /// a real `TenantContext` instead of the 404) → the unmapped host stops
    /// 404'ing and the status-difference / WS-rejected assertions go red.
    #[tokio::test]
    #[ignore]
    async fn unmapped_host_fails_closed_generically() {
        // (2) Generic body + (1) fails-closed status, both on the non-
        // `nostr+json` HTTP door where the body is fully observable.
        let client = reqwest::Client::builder()
            .build()
            .expect("build reqwest client");

        // Default Accept (NOT `application/nostr+json`): a mapped host serves
        // the SPA / NIP-11 fallback (non-404); an unmapped host fails closed.
        let unknown_resp = client
            .get(url_unknown())
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {} failed: {e}", url_unknown()));
        let unknown_status = unknown_resp.status();
        let unknown_body = unknown_resp.text().await.expect("read unmapped body");

        let mapped_resp = client
            .get(url_a())
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {} failed: {e}", url_a()));
        let mapped_status = mapped_resp.status();

        // (1) Fails closed: unmapped is 404, mapped is not. The *difference* is
        // the proof — the unmapped host bound to no tenant, while the mapped one
        // proceeded past the bind. If an unmapped host silently fell through to a
        // default tenant, it would return the same non-404 as the mapped host.
        assert_eq!(
            unknown_status,
            reqwest::StatusCode::NOT_FOUND,
            "unmapped host must fail closed with 404, not fall through to a \
             default tenant (got {unknown_status})"
        );
        assert_ne!(
            mapped_status,
            reqwest::StatusCode::NOT_FOUND,
            "a mapped host must NOT 404 on this door — otherwise the 404 above \
             is not evidence the unmapped host was singled out as unbound"
        );

        // (2) Generic: the body must not echo the host or any tenant-
        // distinguishing fragment. The host authority `unknown.localhost[:port]`
        // (and the bare label) must be absent, so the rejection cannot be used
        // to confirm a host the relay does not serve.
        let unknown_url = url_unknown();
        let unknown_authority = unknown_url
            .strip_prefix("http://")
            .or_else(|| unknown_url.strip_prefix("https://"))
            .unwrap_or(&unknown_url);
        assert!(
            !unknown_body.contains(unknown_authority),
            "unmapped-host rejection echoed the host authority \
             {unknown_authority:?} in its body: {unknown_body:?} — the \
             rejection must be generic and reveal nothing host-specific"
        );
        assert!(
            !unknown_body.contains("unknown.localhost"),
            "unmapped-host rejection echoed the host label in its body: \
             {unknown_body:?} — the rejection must be generic"
        );

        // (3) The WS-upgrade door fails closed too: a raw WebSocket handshake to
        // the unknown host is rejected AT the upgrade, never accepted and then
        // bound to a default tenant. `bind_community` runs before
        // `WebSocketUpgrade::from_request`, so the 404 is returned in place of
        // the `101 Switching Protocols` and the handshake errors out.
        let ws_url = url_unknown().replacen("http://", "ws://", 1);
        let ws_result = tokio_tungstenite::connect_async(&ws_url).await;
        assert!(
            ws_result.is_err(),
            "WebSocket upgrade to an unmapped host must be rejected at the \
             handshake (fail-closed before any frame), but it succeeded — the \
             connection bound to a tenant it should not have"
        );
    }

    /// Obligation: a client-supplied `h` tag / token community stamp can never
    /// override the host-derived community; a disagreeing stamp is rejected.
    ///
    /// # What this row asserts, and how it is *distinct* from its siblings
    ///
    /// Per `NOSTR.md`: "The Nostr wire format does not grow a tenant tag.
    /// Client-supplied `#h` tags still name channels/groups and are checked
    /// against the host-derived community." So the only client-supplied
    /// community-ish signal on the EVENT wire is the `#h` channel tag, and the
    /// row-zero contract is that it is resolved *within* the host-derived
    /// community (`tenant.community()`), never honored as a cross-community
    /// override.
    ///
    /// This is the **override-attempt** scenario, deliberately partitioned from
    /// two siblings that share the same scope branch but assert different
    /// properties of it (see channel: `buzz-relay-rewrite`, 2026-06-27):
    ///
    ///   * [`super::channels_membership::same_channel_uuid_in_two_communities_is_isolated`]
    ///     (Mari, `buzz-db`) asserts **coexistence**: a channel UUID that exists
    ///     in *both* A and B; a post in A's instance never touches B's. Two
    ///     legitimate channels, non-interference.
    ///   * [`super::api_tokens_nip98_replay`] / Sami's
    ///     `verify_nip42_rejects_event_signed_for_wrong_communitys_host`
    ///     (`nip42_host_binding_live.rs`) assert the **AUTH `relay` tag** and
    ///     **token / NIP-98 `u`-host** override signals on their own paths.
    ///
    /// row_zero (b) asserts the **`#h` override-attempt**: a channel that exists
    /// *only in B*; an A connection `#h`-tagging it is **rejected** — the host
    /// binding wins over the claim. Sibling-not-replacement: this shares the
    /// `ingest::check_channel_membership` → `is_member_cached(tenant.community(),
    /// ch_id)` scope branch with Mari's row, but bites the *override* property,
    /// not coexistence.
    ///
    /// # Why the channel is `visibility=open` (isolating override from membership)
    ///
    /// The B channel is created **open**, so in B itself a non-member can post to
    /// it. That is load-bearing: it means the A-connection post can fail for
    /// **exactly one** reason — the channel does not exist in A's community
    /// (`get_channel(A, b_ch_id)` is `None` → not open → not member). If the
    /// channel were restricted, the rejection would be the ordinary
    /// "not a member" gate and would *not* prove the override property. The
    /// positive control (the same post succeeds against B) confirms the channel
    /// is genuinely postable, so the A-side rejection is the override-rejection
    /// and nothing else — the red comes from the override assertion, not a setup
    /// or shared-membership failure.
    ///
    /// Mutate-bite (would-it-fail-without-the-fix): make
    /// `ingest::check_channel_membership` resolve the channel against the
    /// *claimed* `#h` community instead of `tenant.community()` (honor the
    /// override) → the A-connection post of B's open channel is accepted and
    /// this row's "A must reject" assertion goes red.
    ///
    /// # Bite-specificity: the rejection is pinned to the override branch
    ///
    /// "A rejected" alone is not enough — `bind_community` (404), bridge-auth
    /// (403), NIP-98 replay, relay-membership (403), and JSON parse (400) all
    /// run *before* the channel-scope branch, so any of them could red this row
    /// while the override path was never reached. To rule that out, the
    /// override assertion below also pins the reason string
    /// `"restricted: not a channel member"` — the exact
    /// `IngestError::Rejected` the override path emits. That makes the red mean
    /// "A rejected *because* the host-derived community refused the `#h` claim,"
    /// not merely "A rejected." (Dawn + Mari converged on this independently
    /// from the sanitization and channels-membership sides, 2026-06-27.)
    #[tokio::test]
    #[ignore]
    async fn client_supplied_community_cannot_override_host() {
        use nostr::Keys;

        let keys = Keys::generate();

        // Create an OPEN channel that lives ONLY in community B (host B).
        let channel = create_open_channel(&url_b(), &keys).await;

        // Positive control: the channel is genuinely postable in B — a kind:9
        // message to B's host succeeds. This proves the A-side rejection below
        // is the cross-community override-rejection, not a broken/unpostable
        // channel or a membership gate.
        let (status_b, body_b) =
            post_kind9(&url_b(), &keys, &channel, "row-zero-b: legit post in B").await;
        assert!(
            status_b.is_success() && accepted(&body_b),
            "control failed: kind:9 to B's own open channel must be accepted \
             (status {status_b}, body {body_b}) — without this the A-side \
             rejection does not isolate the override property"
        );

        // The override attempt: on an A connection (host A → community A), post a
        // kind:9 `#h`-tagging the channel UUID that exists only in B. The
        // client-supplied `#h` community signal disagrees with
        // `resolve_host(A)`; row zero requires the host to win, so A must reject.
        let (status_a, body_a) = post_kind9(
            &url_a(),
            &keys,
            &channel,
            "row-zero-b: override attempt from A",
        )
        .await;

        // The override assertion itself: A rejects. `get_channel(A, b_ch_id)`
        // finds nothing in A's community, so the open-channel bypass cannot
        // apply and the host-resolved community refuses the claim. (A 2xx +
        // accepted:true here would mean A honored the B `#h` claim — the exact
        // override this row forbids.)
        assert!(
            !status_a.is_success() || !accepted(&body_a),
            "row zero violated: an A connection posting to a channel that exists \
             only in community B was ACCEPTED (status {status_a}, body {body_a}) \
             — the client-supplied `#h` community overrode the host-derived \
             community"
        );

        // Bite-specificity: the rejection above must come from the
        // channel-scope/override branch, not an incidental earlier gate
        // (`bind_community` 404, bridge-auth 403, NIP-98 replay, relay
        // membership 403, JSON parse 400) that would red this row while the
        // override path was never reached. The override path emits exactly
        // `IngestError::Rejected("restricted: not a channel member")`:
        // `get_channel(A, b_ch_id)` returns None against A's community, the
        // open-channel bypass cannot apply, and the host-resolved community
        // refuses the claim. Pinning the reason string converts "A rejected
        // for *some* reason" into "A rejected *because the host-derived
        // community refused the `#h` claim*" — the property this row exists to
        // prove. (Two cold reviewers, Dawn + Mari, converged on this
        // independently from the sanitization and channels-membership sides.)
        assert!(
            body_a.contains("restricted: not a channel member"),
            "row zero violated: A rejected, but not via the channel-scope \
             override branch — body {body_a:?} does not carry the \
             \"restricted: not a channel member\" reason, so the red could be \
             an incidental earlier gate (auth/parse/relay-membership) rather \
             than the host-binding refusing the client-supplied `#h` claim"
        );

        // And the rejection must not leak B's existence: the generic
        // channel-scope rejection ("restricted: not a channel member") reveals
        // nothing about whether the channel exists elsewhere. The B channel UUID
        // appearing in A's rejection body would itself be a cross-community
        // existence oracle.
        assert!(
            !body_a.contains(&channel),
            "A's rejection echoed the B-only channel UUID {channel:?} in its \
             body: {body_a:?} — the rejection must not confirm cross-community \
             existence"
        );
    }
}

/// Create an `open`-visibility channel (kind:9007) in the community bound to
/// `base_url`'s host, via the NIP-98 HTTP bridge (`POST /events`). Returns the
/// channel UUID. The `Host` header is implied by `base_url`, so the relay
/// derives the community from the host — the channel lands in exactly that
/// community and no other.
async fn create_open_channel(base_url: &str, keys: &nostr::Keys) -> String {
    use nostr::{EventBuilder, Kind, Tag};

    let channel_uuid = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_uuid]).expect("h tag"),
            Tag::parse(["name", &format!("row-zero-{channel_uuid}")]).expect("name tag"),
            Tag::parse(["channel_type", "stream"]).expect("channel_type tag"),
            Tag::parse(["visibility", "open"]).expect("visibility tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign create-channel event");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/events"))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).expect("serialize event"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("create-channel POST to {base_url} failed: {e}"));
    let status = resp.status();
    let body = resp.text().await.expect("read create-channel body");
    assert!(
        status.is_success() && body.contains("\"accepted\":true"),
        "create-channel in {base_url} must succeed (status {status}, body {body})"
    );

    channel_uuid
}

/// Post a kind:9 group message `#h`-tagging `channel` to the community bound to
/// `base_url`'s host. Returns `(status, body)` so callers can assert on the
/// wire-observable accept/reject. The relay derives the community from the
/// `Host` (implied by `base_url`); `channel` is the client-supplied `#h` claim.
async fn post_kind9(
    base_url: &str,
    keys: &nostr::Keys,
    channel: &str,
    content: &str,
) -> (reqwest::StatusCode, String) {
    use nostr::{EventBuilder, Kind, Tag};

    let event = EventBuilder::new(Kind::Custom(9), content)
        .tags(vec![Tag::parse(["h", channel]).expect("h tag")])
        .sign_with_keys(keys)
        .expect("sign kind:9 event");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/events"))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).expect("serialize event"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("kind:9 POST to {base_url} failed: {e}"));
    let status = resp.status();
    let body = resp.text().await.expect("read kind:9 body");
    (status, body)
}

/// Whether a `POST /events` JSON body reports the event as accepted.
fn accepted(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("accepted").and_then(|a| a.as_bool()))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// NIP-11 relay info and relay `self` (Eva — relay-wiring)
// ---------------------------------------------------------------------------
mod nip11_relay_info {
    use super::*;

    /// Fetch the NIP-11 relay information document from `base_url`'s root with
    /// `Accept: application/nostr+json`. Returns `(status, body)`; `body` is the
    /// raw response text (parsed by callers as needed).
    ///
    /// The `Host` header is implied by `base_url` — `a.localhost`/`b.localhost`
    /// both resolve to 127.0.0.1, so reqwest addresses the same relay process
    /// and the relay derives the community from the host. That host-derivation
    /// is exactly what this row exercises; nothing here is caller-supplied.
    async fn fetch_nip11(base_url: &str) -> (reqwest::StatusCode, String) {
        let client = reqwest::Client::builder()
            .build()
            .expect("build reqwest client");
        let resp = client
            .get(base_url)
            .header(reqwest::header::ACCEPT, "application/nostr+json")
            .send()
            .await
            .unwrap_or_else(|e| panic!("NIP-11 GET {base_url} failed: {e}"));
        let status = resp.status();
        let body = resp.text().await.expect("read NIP-11 body");
        (status, body)
    }

    /// Obligation: unauthenticated NIP-11 reads must not become an enumeration
    /// oracle for other communities; `RelayInfo::build` takes only static +
    /// host-scoped inputs (the static-input lint backs this at compile/CI time).
    ///
    /// This is the *black-box* complement to that compile-time fence
    /// (`crates/buzz-relay/src/nip11.rs::_RELAY_INFO_BUILD_STATIC_INPUT_FENCE`):
    /// the fence proves `RelayInfo::build` *cannot* take an unscoped DB/search
    /// input; this test proves the *observable wire behavior* — that the served
    /// document carries nothing that distinguishes one community from another.
    ///
    /// One field is deliberately host-scoped: `icon` (the NIP-WP workspace
    /// icon, per-community state served in the standard NIP-11 field, fetched
    /// through `bind_community` so it can only ever be the requesting host's
    /// own community state — intentionally public presentation, exactly what
    /// upstream NIP-11 `icon` is). So the
    /// oracle proof is: the documents are identical *modulo each community's
    /// own `icon`*, and an unmapped host's document carries no `icon` at all.
    /// The moment any *other* per-community value leaks into the doc, the
    /// icon-stripped bodies diverge and this assertion fails — that is the
    /// mutate-bite this row guards (seed a community-distinguishing field into
    /// the served doc → the A≡B assertion goes red).
    #[tokio::test]
    #[ignore]
    async fn nip11_is_not_a_cross_community_enumeration_oracle() {
        let (status_a, body_a) = fetch_nip11(&url_a()).await;
        let (status_b, body_b) = fetch_nip11(&url_b()).await;

        assert_eq!(
            status_a,
            reqwest::StatusCode::OK,
            "host A must serve its NIP-11 document"
        );
        assert_eq!(
            status_b,
            reqwest::StatusCode::OK,
            "host B must serve its NIP-11 document"
        );

        // Both bodies must be valid NIP-11 JSON — a relay-info object, not an
        // error page or a host echo.
        let mut json_a: serde_json::Value =
            serde_json::from_str(&body_a).expect("host A NIP-11 is valid JSON");
        let mut json_b: serde_json::Value =
            serde_json::from_str(&body_b).expect("host B NIP-11 is valid JSON");
        assert!(
            json_a.get("supported_nips").is_some(),
            "host A NIP-11 must be a relay-info document (has supported_nips)"
        );
        assert!(
            json_b.get("supported_nips").is_some(),
            "host B NIP-11 must be a relay-info document (has supported_nips)"
        );

        // `icon` is the one deliberately host-scoped field (the NIP-WP
        // workspace icon; each host sees only its own community's icon).
        // Strip it before the equality proof — every OTHER field must still
        // be community-agnostic.
        json_a.as_object_mut().map(|o| o.remove("icon"));
        json_b.as_object_mut().map(|o| o.remove("icon"));

        // The enumeration-oracle obligation: no other field of the served
        // document varies by community. Identical icon-stripped bodies are the
        // proof that the doc cannot be used to distinguish or probe another
        // tenant.
        assert_eq!(
            json_a, json_b,
            "NIP-11 from host A and host B must be identical apart from each \
             community's own `icon`: any other community-distinguishing field \
             would make the unauthenticated relay-info document an enumeration \
             oracle for other tenants"
        );

        // An *unmapped* host must get the SAME document too — not a 404. NIP-11
        // is intentionally host-agnostic (served from static facts BEFORE host
        // binding; see `router.rs::nip11_or_ws_handler`). If an unknown host
        // 404'd here while a mapped host returned 200, that status difference
        // would itself be the enumeration oracle — a caller could probe which
        // hosts are configured by watching for 404-vs-200. Serving the identical
        // static doc to every host, mapped or not, is precisely what denies that
        // oracle. (Fail-closed host binding lives on the WS-upgrade / non-
        // `nostr+json` path and is asserted by `row_zero_host_binding`.)
        let (status_unknown, body_unknown) = fetch_nip11(&url_unknown()).await;
        assert_eq!(
            status_unknown,
            reqwest::StatusCode::OK,
            "an unmapped host must still receive the static NIP-11 document, not \
             a 404 — a status difference between mapped and unmapped hosts would \
             itself be a community-enumeration oracle"
        );
        let json_unknown: serde_json::Value =
            serde_json::from_str(&body_unknown).expect("unmapped-host NIP-11 is valid JSON");
        assert!(
            json_unknown.get("icon").is_none(),
            "an unmapped host binds to no community, so its NIP-11 document must \
             carry no `icon` — leaking any community's icon to an unmapped host \
             would cross the tenant boundary"
        );
        assert_eq!(
            json_a, json_unknown,
            "NIP-11 served to an unmapped host must match a mapped host's \
             icon-stripped document: apart from the host's own `icon`, the \
             relay-info doc carries no host-derived field, so it cannot reveal \
             whether a given host is configured"
        );
    }
}

// ---------------------------------------------------------------------------
// API tokens and NIP-98 replay (Sami — buzz-auth)
// ---------------------------------------------------------------------------
mod api_tokens_nip98_replay {
    use super::*;

    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sha2::{Digest, Sha256};

    /// Obligation: token hash uniqueness/lookup is `(community_id, token_hash)`;
    /// a token minted in A does not authorize the same hash in B.
    ///
    /// **This row is doc-only — and that is the honest answer.**
    ///
    /// Every other wire-driven row in this file proves a black-box property: a
    /// client addresses the relay over HTTP/WS and observes that the response
    /// denies a cross-community oracle. This row's obligation cannot be tested
    /// that way because **the api_token mint surface does not exist on the wire
    /// in `buzz-relay`** — there is no route by which a client can bind a token
    /// to a community over HTTP, so the "mint in A, present to B" precondition
    /// has no entry point.
    ///
    /// Verified on PR head:
    ///   * `crates/buzz-relay/src/router.rs:52-79` — full route list is `/`,
    ///     `/info`, `/.well-known/nostr.json`, `/health`, `/_liveness`,
    ///     `/_readiness`, `/events`, `/query`, `/count`, `/hooks/{id}`, plus
    ///     the media, git, and git-policy sub-routers. **No `/tokens` route.**
    ///   * `crates/buzz-relay/src/api/` directory is `{bridge, events, media,
    ///     nip05, git/, mod}` — no `tokens` module. The 792-line
    ///     `crates/sprout-relay/src/api/tokens.rs` self-service-minting
    ///     endpoint that existed pre-rewrite (PR #37, commit `f84da74d3`) was
    ///     deliberately not ported. Porting it to enable this row would be a
    ///     product + security-surface decision, not a test-enablement task.
    ///   * Api_tokens are consumed (not minted) by the Blossom upload path at
    ///     `crates/buzz-relay/src/api/media.rs:638`, which extracts the
    ///     `X-Auth-Token: buzz_*` header and looks up
    ///     `state.db.get_api_token_by_hash_including_revoked(tenant.community(),
    ///     &hash)`. The comment immediately above that call names the row-44
    ///     fence explicitly: *"A token minted in community A presented to a
    ///     host that resolves to community B must not authorize."*
    ///
    /// So where other rows prove *the oracle is denied*, api_tokens proves
    /// *the oracle's input surface does not exist* — a sibling of the
    /// `audit_log` row (whose proof is that the *output* surface does not
    /// exist). Both are strictly stronger than a wire-denied assertion, and
    /// the honest way to state it is to cite the facts, not to invent an
    /// out-of-band mint path that would break the conformance file's black-box
    /// contract (its deps are `buzz-ws-client`/`reqwest`/`tokio-tungstenite`/
    /// `s3` — no `sqlx`, no `buzz-db`).
    ///
    /// The `(community_id, token_hash)` fence itself is proven directly at the
    /// storage layer, where direct Postgres access is in-convention:
    ///
    ///   1. `crates/buzz-db/src/api_token.rs:425
    ///      lookup_by_hash_is_scoped_to_community` — inserts two rows with
    ///      **identical 32-byte hash** in communities A and B (legal under
    ///      `UNIQUE(community_id, token_hash)`), then asserts A-scoped lookup
    ///      returns A's row only, B-scoped lookup returns B's row only, and a
    ///      third (unrelated) community returns None. The mutate-bite handle
    ///      is named in the test's doc-comment: strip `AND community_id = $1`
    ///      from `get_api_token_by_hash_including_revoked` and the lookup
    ///      becomes hash-only, returning whichever row Postgres picks
    ///      first — the cross-tenancy assertion fails. Sharp row-44 shape.
    ///   2. `crates/buzz-db/src/api_token.rs:488
    ///      active_lookup_by_hash_is_scoped_to_community` — mirror for the
    ///      `revoked_at IS NULL` variant `Db::get_api_token_by_hash`. Same
    ///      shape: same hash, distinct communities, scoped lookup returns the
    ///      caller's row only.
    ///
    /// And the consumer fence — `media.rs:638` — calls the scoped DB lookup
    /// with `tenant.community()` derived from the request host *before* token
    /// resolution (`media.rs:97` comment: *"This MUST run before scope
    /// resolution so the API-token lookup is keyed on (community_id,
    /// token_hash). Resolving scopes without a tenant in hand would query
    /// api_tokens by hash alone, defeating the cross-community fence."*).
    ///
    /// Substrate on PR head: `api_tokens` table has UNIQUE index
    /// `(community_id, token_hash)`; `CommunityId` is a server-resolved type
    /// (never client input); `tenant.community()` is bound from the request
    /// host by `bind_community` before any tenant-scoped DB read or write.
    #[test]
    fn token_minted_in_a_does_not_authorize_in_b() {
        // Compile-time anchor: this row is doc-only by design. The proof lives
        // in the cited storage-layer unit tests; the wire surface for minting
        // does not exist (see module doc-comment for the verified route list).
        // If anyone adds a `/tokens` route to `buzz-relay`, this row's shape
        // should be revisited and a wire-driven body added.
    }

    /// Obligation: NIP-98 replay seen-set is shared (any-pod) AND community
    /// scoped: a nonce spent in A is still spendable in B, but a replay within
    /// A is rejected from any pod.
    ///
    /// # Wire-observable claim and what bites it
    ///
    /// The load-bearing wire-observable property is **within-community replay
    /// rejection**: a NIP-98 event posted twice to the same community must be
    /// rejected on the second attempt. This is the proof that the shared
    /// (cross-pod) seen-set is in the request path at all — without it, any
    /// pod would happily re-honor a spent NIP-98 event.
    ///
    /// Mutate-bite for this assertion: turn `check_nip98_replay` into a no-op
    /// in `crates/buzz-relay/src/api/bridge.rs:79` (return `Ok(())` before
    /// consulting the guard). Under the mutation, the second POST goes 200
    /// instead of 401 → the within-community replay assertion fails RED. The
    /// bite fires on the replay check itself, not on a sibling fence.
    ///
    /// # Why the cross-community independence arm is a tripwire, not a bite
    ///
    /// The obligation's cross-community arm — "a nonce spent in A is still
    /// spendable in B" — IS asserted by this test, but as a **positive
    /// control** rather than a mutate-bite. Reasoning:
    ///
    /// The replay key shape is `buzz:{community}:nip98:{event_id_hex}` (see
    /// `crates/buzz-auth/src/nip98_replay.rs:103 nip98_replay_key`). The
    /// community prefix is what makes the key per-community; the
    /// `event_id_hex` is what makes it per-event. **On natural wire traffic
    /// the event_id is already community-distinct**, because the NIP-98 `u`
    /// tag is part of the signed canonical bytes and the per-tenant host
    /// binding (see `verify_bridge_auth` + `nip98_expected_url` after the
    /// row-44 sibling fix `bf8a1a4fa`) forces the `u` to differ per community.
    /// So two events posted to A and B respectively are signed against
    /// different `u` values → they have different event_ids → their seen-set
    /// keys have different suffixes → they do not collide *regardless of
    /// whether the community prefix is present*.
    ///
    /// This means the community prefix is **structurally redundant for
    /// natural wire traffic**. It is load-bearing only against an artificial
    /// "same event_id surfaces in two communities" scenario, which content-
    /// addressing makes implausible. The substrate's own doc-comment for the
    /// unit test that proves the prefix isolates such artificial collisions
    /// names the property as exactly that: "**Belt-and-suspenders**: even if a
    /// same-id event surfaces in two communities (which content-addressing
    /// makes implausible), the seen-set MUST consult two distinct rows."
    /// (`crates/buzz-auth/src/nip98_replay.rs:163
    /// key_isolates_communities_for_same_event_id`.)
    ///
    /// Why u-host can't be bypassed to manufacture a wire collision: the
    /// bridge processes requests in this order (`bridge.rs:242-259`):
    ///   1. `bind_community` from request `Host` header (row-zero fence)
    ///   2. `nip98_expected_url(state.config.relay_url, &tenant, "/events")`
    ///      builds the per-tenant expected `u`
    ///   3. `verify_bridge_auth` rejects with 401 unless the signed event's
    ///      `u` tag matches the per-tenant expected URL
    ///   4. **Only then** is `check_nip98_replay` called
    ///
    /// A same-physical-event posted to both A and B would be rejected at step
    /// 3 (u-host mismatch) for one of the two hosts, so it never reaches the
    /// replay check from a wire test. The replay-prefix-drop mutation
    /// proposed in earlier design rounds turned out to be vacuous against
    /// natural wire traffic; a wire-driven bite on the prefix's load-
    /// bearingness against the artificial-collision case is not constructible
    /// from this file's black-box vantage point. The unit test cited above
    /// proves it at the layer where the artificial construction is possible.
    ///
    /// What the tripwire DOES catch: a future regression that globalizes the
    /// seen-set namespace by truncating or normalizing the key (e.g.,
    /// "simplifying" the key to just `buzz:nip98:{event_id}`, or
    /// canonicalizing `u` in a way that collapses cross-tenant `u` values
    /// into the same event_id) would break the "spend in A doesn't burn the
    /// slot in B" arm even though u-tags differ. The tripwire assertion gives
    /// such a regression somewhere to land at the wire layer, on top of the
    /// unit-layer same-id-collision proof.
    ///
    /// # Test layout
    ///
    /// Two distinct keypairs per community (Eva's setup-equivalence vacuity
    /// scar: distinct values make any leak surface as wrong-pubkey-spent /
    /// wrong-content-stored, not silent-absent). The wire surface is `POST
    /// /events` with `Authorization: Nostr <base64-NIP-98-event>`. Bodies are
    /// minimal valid kind:1 nostr events authored by the same NIP-98 signer
    /// (relay-membership is open under `BUZZ_REQUIRE_AUTH_TOKEN=false`).
    #[tokio::test]
    #[ignore]
    async fn nip98_replay_seenset_is_shared_and_community_scoped() {
        let http_a = to_http(&url_a());
        let http_b = to_http(&url_b());

        // Distinct keypairs per community — if the seen-set ever leaked into
        // a globalized namespace via a regression in u-canonicalization, the
        // tripwire assertion below catches it because B's post would 401 as
        // "already spent" using A's key's slot.
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        assert_ne!(
            keys_a.public_key().to_hex(),
            keys_b.public_key().to_hex(),
            "test design requires distinct keys per community"
        );

        // (1) Within-community replay rejection — the load-bearing wire bite.
        //
        // Sign a NIP-98 event E with u = A's /events URL, post it to A → must
        // 200. Post the exact same NIP-98 event again to A → must 401 with a
        // body that names replay detection. The mutate-bite is `check_nip98_
        // replay → noop` in `bridge.rs:79`; under that mutation the second
        // post goes 200 because the seen-set is not consulted, and this
        // assertion fails RED on the within-community arm.
        let events_url_a = format!("{http_a}/events");
        let body_a = build_kind1_event_json(&keys_a, "A within-community replay test");
        let auth_a_first = build_nip98_header(&keys_a, &events_url_a, "POST", body_a.as_bytes());

        let client = reqwest::Client::new();
        let first_a = client
            .post(&events_url_a)
            .header("Authorization", &auth_a_first)
            .header("Content-Type", "application/json")
            .body(body_a.clone())
            .send()
            .await
            .unwrap_or_else(|e| panic!("first POST to A failed: {e}"));
        assert!(
            first_a.status().is_success(),
            "first NIP-98 post to A must succeed, got {} (body: {})",
            first_a.status(),
            first_a.text().await.unwrap_or_default(),
        );

        // Repost the SAME NIP-98 event (byte-identical Authorization header,
        // byte-identical body — same canonical event id).
        let second_a = client
            .post(&events_url_a)
            .header("Authorization", &auth_a_first)
            .header("Content-Type", "application/json")
            .body(body_a)
            .send()
            .await
            .unwrap_or_else(|e| panic!("second POST to A failed: {e}"));
        // The assertion below pins the status code (401), not just rejection,
        // because the system has defense-in-depth across two layers with
        // distinct rejection signatures:
        //   * auth-layer replay check (`check_nip98_replay`) — rejects with
        //     401 + body "NIP-98: replay detected".
        //   * storage-layer dedup (`events` PK `ON CONFLICT DO NOTHING` in
        //     `ingest_event`) — accepts with 200 + body `accepted: false,
        //     message: "duplicate"`.
        // Both reject a duplicate, but only the 401 path proves the seen-set
        // is in the request path. A body-only check like `!accepted` would
        // pass under a noop'd `check_nip98_replay` because storage-dedup
        // still 200-accepted-false's the second post — the bite would go
        // vacuous against the layer the obligation actually names. Status-
        // code is the load-bearing-layer discriminator; do not weaken this
        // to `!accepted` for "simpler reading."
        assert_eq!(
            second_a.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "second POST to A with the same NIP-98 event MUST be rejected as \
             replay (got {}) — if this returns 200, the shared seen-set is \
             not in the request path (storage-dedup at `ingest_event` would \
             return 200-accepted-false on the same input; only the 401 from \
             `check_nip98_replay` proves the auth-layer replay fence). \
             Mutate-bite handle: `check_nip98_replay` in bridge.rs:79.",
            second_a.status(),
        );
        let second_a_body = second_a.text().await.unwrap_or_default();
        assert!(
            second_a_body.contains("replay"),
            "second POST to A's 401 body should name replay detection, got: \
             {second_a_body:?}",
        );

        // (2) Cross-community spend-spread — tripwire, no mutate-bite.
        //
        // Sign an independent NIP-98 event E' for B's /events URL (different
        // u → different event_id by signed-canonical divergence). Post E' to
        // B → must 200 even though E was already spent in A. This catches a
        // future namespace-globalization regression in the seen-set (e.g.,
        // key truncation, u-normalization collapse) by giving such a
        // regression somewhere to land at the wire layer. The cross-
        // community-prefix isolation property against an artificial same-id
        // collision is proven at the unit layer by
        // `nip98_replay.rs:163 key_isolates_communities_for_same_event_id`.
        let events_url_b = format!("{http_b}/events");
        let body_b = build_kind1_event_json(&keys_b, "B cross-community spread test");
        let auth_b = build_nip98_header(&keys_b, &events_url_b, "POST", body_b.as_bytes());

        let first_b = client
            .post(&events_url_b)
            .header("Authorization", &auth_b)
            .header("Content-Type", "application/json")
            .body(body_b)
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST to B failed: {e}"));
        assert!(
            first_b.status().is_success(),
            "POST of an independent NIP-98 event to B must succeed (200) — \
             A's spent nonce must not burn B's seen-set slot. Got {} (body: \
             {}). Tripwire: if this fails, the seen-set namespace has been \
             globalized and the per-community scope is broken. The artificial \
             same-event_id-different-community case is proven separately at \
             `crates/buzz-auth/src/nip98_replay.rs:163 \
             key_isolates_communities_for_same_event_id`.",
            first_b.status(),
            first_b.text().await.unwrap_or_default(),
        );
    }

    /// Build a `Authorization: Nostr <base64>` header value for NIP-98 HTTP
    /// auth (kind 27235 `HttpAuth` with `u`/`method`/`payload` tags). Mirrors
    /// the pattern in `crates/buzz-auth/src/nip98.rs`; kept local to this
    /// row so the conformance file's rows stay self-contained.
    fn build_nip98_header(keys: &Keys, url: &str, method: &str, body: &[u8]) -> String {
        let payload_hash = hex::encode(Sha256::digest(body));
        let tags = vec![
            Tag::parse(["u", url]).expect("u tag"),
            Tag::parse(["method", method]).expect("method tag"),
            Tag::parse(["payload", &payload_hash]).expect("payload tag"),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign NIP-98 event");
        let json = nostr::JsonUtil::as_json(&event);
        let encoded = BASE64.encode(json.as_bytes());
        format!("Nostr {encoded}")
    }

    /// Build a minimal kind:1 nostr event JSON for the bridge ingest path.
    /// The body is just a valid signed event — the test does not care about
    /// the content beyond it surviving relay-side parsing.
    fn build_kind1_event_json(keys: &Keys, content: &str) -> String {
        let event = EventBuilder::new(Kind::TextNote, content)
            .sign_with_keys(keys)
            .expect("sign kind:1");
        nostr::JsonUtil::as_json(&event)
    }

    /// Convert any base form to `http(s)://` for REST.
    fn to_http(base: &str) -> String {
        if base.starts_with("http://") || base.starts_with("https://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("wss://", "https://")
                .replace("ws://", "http://")
                .trim_end_matches('/')
                .to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Membership / allowlist / archived identities (Sami — buzz-auth)
// ---------------------------------------------------------------------------
mod membership_allowlist {
    use super::*;

    /// Obligation: membership/allowlist/archive keyed `(community_id, pubkey)`;
    /// archiving a key in A cannot hide/archive it in B; errors stay generic.
    #[tokio::test]
    #[ignore]
    async fn archive_in_a_does_not_affect_b() {
        pending_lane(
            "buzz-auth",
            "archived_identities (community_id, pubkey) — A's archive invisible to B",
        );
    }
}

// ---------------------------------------------------------------------------
// Users / profiles / NIP-05 / user search (Sami+Quinn — auth+search)
// ---------------------------------------------------------------------------
mod users_profiles_nip05 {
    use super::*;

    use buzz_test_client::BuzzTestClient;
    use nostr::{EventBuilder, Keys, Kind};

    /// Convert any base form to `ws(s)://` for WS connect.
    fn to_ws(base: &str) -> String {
        if base.starts_with("ws://") || base.starts_with("wss://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("https://", "wss://")
                .replace("http://", "ws://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// Convert any base form to `http(s)://` for REST.
    fn to_http(base: &str) -> String {
        if base.starts_with("http://") || base.starts_with("https://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("wss://", "https://")
                .replace("ws://", "http://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// Publish a kind:0 (Metadata) event via WS for `keys`, with the supplied
    /// JSON content. Returns the event id hex. Latest kind:0 per
    /// `(community_id, pubkey)` wins under NIP-01 replaceable semantics; the
    /// relay's ingest side-effect at `crates/buzz-relay/src/handlers/side_effects.rs::handle_kind0_profile`
    /// also syncs the parsed fields into `users` via
    /// `update_user_profile(tenant.community(), pubkey, ...)`.
    async fn publish_kind0(client: &mut BuzzTestClient, keys: &Keys, content_json: &str) -> String {
        let event = EventBuilder::new(Kind::Metadata, content_json)
            .sign_with_keys(keys)
            .unwrap();
        let id_hex = event.id.to_hex();
        let ok = client.send_event(event).await.expect("send kind:0");
        assert!(ok.accepted, "kind:0 not accepted: {}", ok.message);
        id_hex
    }

    /// Query latest kind:0 for `pubkey_hex` via REST `POST /query` (relay's
    /// bridge endpoint; with `BUZZ_REQUIRE_AUTH_TOKEN=false` the dev-mode
    /// `X-Pubkey` header is sufficient — no NIP-98 mint needed). Returns the
    /// list of event JSON values (typically 0 or 1 since kind:0 is
    /// NIP-01-replaceable).
    async fn query_kind0(http_base: &str, pubkey_hex: &str) -> Vec<serde_json::Value> {
        let client = reqwest::Client::new();
        let filters = serde_json::json!([{
            "kinds": [0],
            "authors": [pubkey_hex],
            "limit": 1,
        }]);
        let resp = client
            .post(format!("{http_base}/query"))
            .header("X-Pubkey", pubkey_hex)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&filters).unwrap())
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST /query against {http_base} failed: {e}"));
        assert!(
            resp.status().is_success(),
            "POST /query against {http_base} returned {}",
            resp.status()
        );
        resp.json().await.expect("parse /query JSON")
    }

    /// Extract the NIP-11 / extract_domain form of a host. Mirrors
    /// `crates/buzz-relay/src/api/nip05.rs::extract_domain` — strips the
    /// scheme prefix and any port, returning the bare hostname. The relay's
    /// `canonicalize_nip05` requires a kind:0 `nip05` value to end in
    /// `@<this domain>`. In multi-tenant mode the expected domain is the bound
    /// request host (`tenant.host()`), not the global `RELAY_URL` host.
    fn extract_relay_domain(relay_url_env: &str) -> String {
        // Match the relay's logic exactly: strip wss/ws, then take everything
        // before the first ':' or '/' (port and path).
        let s = relay_url_env
            .trim_start_matches("wss://")
            .trim_start_matches("ws://")
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        s.split([':', '/']).next().unwrap_or(s).to_string()
    }

    /// Obligation: same pubkey can hold a different profile per community; kind:0
    /// replacement is scoped by `(community_id, pubkey)`.
    ///
    /// Wire-observable shape: the same keypair publishes kind:0 with
    /// community-distinct content (`{"display_name":"A profile"}` vs
    /// `{"display_name":"B profile"}`) on each host's WS-AUTH'd connection.
    /// NIP-01 replaceable semantics mean the latest kind:0 per `(community_id,
    /// pubkey)` is what subsequent queries return — REST `POST /query` on each
    /// host returns the kind:0 *its* community has, never the other's.
    ///
    /// Sibling-not-replacement framing: this row asserts the *per-community*
    /// view of the profile (the obligation text). The underlying SQL fence at
    /// `crates/buzz-db/src/user.rs:140` (`UPDATE users SET ... WHERE
    /// community_id = $N AND pubkey = $M`) AND the event-storage fence at
    /// `crates/buzz-db/src/event.rs::get_events_by_ids` (or the equivalent
    /// REQ-path filter) both contribute — same defense-in-depth shape Quinn
    /// caught in `search_fts` (see that row's doc-comment). The mutate-bite
    /// here drops the read-side community fence on kind:0 retrieval (the
    /// query.rs / event.rs path that materialises `POST /query` results); a
    /// dropped-fence leak surfaces as the other community's content in the
    /// response. Distinct content per community is what makes the leak
    /// observable on the wire (different content → different Nostr event id;
    /// without distinct content, the symmetric setup-equivalence vacuity Dawn
    /// caught in `audit_log` would collapse the leak into a same-id
    /// indistinguishable wire return).
    ///
    /// Same pubkey on both connections is required by the obligation itself
    /// ("same pubkey ... different profile per community"); it is what proves
    /// the fence is `community_id`, not `pubkey`.
    #[tokio::test]
    #[ignore]
    async fn same_pubkey_distinct_profiles_in_two_communities() {
        let ws_a = to_ws(&url_a());
        let ws_b = to_ws(&url_b());
        let http_a = to_http(&url_a());
        let http_b = to_http(&url_b());

        // Same key on both connections — that's the property under test.
        let keys = Keys::generate();
        let pubkey_hex = keys.public_key().to_hex();

        // Community-distinct content. Same JSON shape, different display_name:
        // the difference is what makes a cross-community leak observable on
        // the wire (a leak that returns the *other* community's kind:0 shows
        // up as a different display_name string in the response).
        let content_a = serde_json::json!({"display_name": "A profile"}).to_string();
        let content_b = serde_json::json!({"display_name": "B profile"}).to_string();

        // Connect each side, publish each side's kind:0.
        let mut client_a = BuzzTestClient::connect(&ws_a, &keys)
            .await
            .expect("connect A");
        let _id_a = publish_kind0(&mut client_a, &keys, &content_a).await;

        let mut client_b = BuzzTestClient::connect(&ws_b, &keys)
            .await
            .expect("connect B");
        let _id_b = publish_kind0(&mut client_b, &keys, &content_b).await;

        // Let the ingest side-effect settle (handle_kind0_profile is async on
        // the DB write path).
        tokio::time::sleep(Duration::from_millis(500)).await;

        // (1) Query on A: must return exactly one kind:0, whose content is
        // A's. A leak that surfaces B's row would show up as content_b in this
        // response.
        let hits_a = query_kind0(&http_a, &pubkey_hex).await;
        assert_eq!(
            hits_a.len(),
            1,
            "A's /query for kind:0 by pubkey returned {} events; expected exactly 1. \
             contents: {:?}",
            hits_a.len(),
            hits_a
                .iter()
                .map(|e| e["content"].as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        );
        let a_content = hits_a[0]["content"].as_str().unwrap_or("");
        assert_eq!(
            a_content, content_a,
            "A's kind:0 content is not A's profile — B's profile leaked through. \
             got: {a_content:?}"
        );

        // (2) Mirror: query on B returns B's profile, not A's.
        let hits_b = query_kind0(&http_b, &pubkey_hex).await;
        assert_eq!(
            hits_b.len(),
            1,
            "B's /query for kind:0 by pubkey returned {} events; expected exactly 1. \
             contents: {:?}",
            hits_b.len(),
            hits_b
                .iter()
                .map(|e| e["content"].as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        );
        let b_content = hits_b[0]["content"].as_str().unwrap_or("");
        assert_eq!(
            b_content, content_b,
            "B's kind:0 content is not B's profile — A's profile leaked through. \
             got: {b_content:?}"
        );

        client_a.disconnect().await.expect("disconnect A");
        client_b.disconnect().await.expect("disconnect B");
    }

    /// Obligation: the same NIP-05 local part can exist on two hosts; lookup only
    /// resolves handles for the requested host/community.
    ///
    /// Wire-observable shape: same local-part `alice` registered in BOTH
    /// communities, but owned by **distinct** pubkeys (one per community).
    /// `GET /.well-known/nostr.json?name=alice` against host A returns A's
    /// pubkey; same query against host B returns B's pubkey. A cross-community
    /// leak surfaces on the wire as wrong-pubkey-returned (not just
    /// count-changes) — exactly the setup-equivalence vacuity defense Dawn
    /// established in `audit_log` (distinct keys per community = the wrong
    /// answer is observable in the response, not just absent from it).
    ///
    /// Registration mechanism: kind:0 (Metadata) publish over WS with
    /// `{"nip05":"alice@<domain>"}`. The relay's side-effect handler at
    /// `crates/buzz-relay/src/handlers/side_effects.rs::handle_kind0_profile`
    /// validates the handle via `crates/buzz-relay/src/api/nip05.rs::canonicalize_nip05`
    /// against `extract_domain(state.config.relay_url)` (the relay's configured
    /// host) and then calls `update_user_profile(tenant.community(), pubkey, ...,
    /// Some(handle))`. The domain piece is config-pinned (e.g. `"localhost"`
    /// for `RELAY_URL=ws://localhost:3100`); the **community** scope is what
    /// isolates, and that's what this row exercises.
    ///
    /// Mutate-bite (would-it-fail-without-the-fix): drop `community_id = $1`
    /// from `crates/buzz-db/src/user.rs::get_user_by_nip05` (line 185 form
    /// `WHERE community_id = $1 AND LOWER(nip05_handle) = LOWER($2)`). With
    /// the community fence dropped, `LOWER(handle) = LOWER('alice@localhost')`
    /// matches BOTH communities' rows; the LIMIT 1 picks one
    /// non-deterministically — either A's or B's pubkey. From either host the
    /// `GET /.well-known/nostr.json?name=alice` response will sometimes
    /// resolve to the *other* community's pubkey (≠ the registered local
    /// pubkey). The assertion `resolved_pubkey == own_community_pubkey` reds
    /// because the resolved hex no longer matches the host's own user.
    #[tokio::test]
    #[ignore]
    async fn same_nip05_local_part_on_two_hosts_is_independent() {
        let ws_a = to_ws(&url_a());
        let ws_b = to_ws(&url_b());
        let http_a = to_http(&url_a());
        let http_b = to_http(&url_b());

        // Distinct keys per community — the same local-part must resolve to
        // each community's *own* pubkey. With identical keys, a leak would
        // return the same pubkey from either host (indistinguishable on the
        // wire). Distinct keys make the leak observable as
        // wrong-pubkey-returned.
        let keys_a = Keys::generate();
        let keys_b = Keys::generate();
        let pk_a_hex = keys_a.public_key().to_hex();
        let pk_b_hex = keys_b.public_key().to_hex();
        assert_ne!(pk_a_hex, pk_b_hex, "test design requires distinct pubkeys");

        // Same local-part `alice` in both communities, but each registered
        // under its own host's domain. Post-fix, `canonicalize_nip05` runs
        // against `tenant.host()` (not `config.relay_url`), so `alice@
        // a.localhost` is canonical on host A and `alice@b.localhost` is
        // canonical on host B. Pre-fix, both hosts only accept the global
        // `config.relay_url` domain; this registration would have been
        // silently dropped and the well-known lookup would return empty.
        // Deriving each handle from its own host URL ties the test to the
        // canonicalize-against-`tenant.host()` fix at the registration step.
        let domain_a = extract_relay_domain(&url_a());
        let domain_b = extract_relay_domain(&url_b());
        let local = format!("alice_{}", uuid::Uuid::new_v4().simple());
        let handle_a = format!("{local}@{domain_a}");
        let handle_b = format!("{local}@{domain_b}");
        let content_a = serde_json::json!({"display_name": local, "nip05": handle_a}).to_string();
        let content_b = serde_json::json!({"display_name": local, "nip05": handle_b}).to_string();

        // Register each pubkey under the same local-part in its own community.
        let mut client_a = BuzzTestClient::connect(&ws_a, &keys_a)
            .await
            .expect("connect A");
        let _ = publish_kind0(&mut client_a, &keys_a, &content_a).await;

        let mut client_b = BuzzTestClient::connect(&ws_b, &keys_b)
            .await
            .expect("connect B");
        let _ = publish_kind0(&mut client_b, &keys_b, &content_b).await;

        // Let the side-effect (handle_kind0_profile → update_user_profile)
        // settle.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // (1) NIP-05 lookup on A: must resolve `alice` to A's pubkey, not B's.
        let nip05_url_a = format!("{http_a}/.well-known/nostr.json?name={local}");
        let resp_a = reqwest::get(&nip05_url_a)
            .await
            .unwrap_or_else(|e| panic!("GET {nip05_url_a} failed: {e}"));
        assert_eq!(
            resp_a.status(),
            reqwest::StatusCode::OK,
            "NIP-05 lookup on A must return 200"
        );
        let body_a: serde_json::Value = resp_a.json().await.expect("parse NIP-05 JSON from A");
        let resolved_a = body_a["names"][&local].as_str();
        assert_eq!(
            resolved_a,
            Some(pk_a_hex.as_str()),
            "NIP-05 lookup on A for local-part {local:?} must resolve to A's pubkey \
             ({pk_a_hex}); got {resolved_a:?}. If this is B's pubkey ({pk_b_hex}), \
             the community fence on `get_user_by_nip05` has been dropped and B's \
             user leaked through A's lookup."
        );
        // Host-local NIP-05 identity (Mari's product bar): the relays
        // map for A's user must advertise A's own host, never the global
        // config.relay_url. Pre-fix, line 52 of `api/nip05.rs` cloned
        // `state.config.relay_url` here regardless of bound tenant, so a
        // client on host A would have been pointed back at the global URL
        // (e.g. `ws://localhost:3100`) instead of `ws://a.localhost:3000`.
        let relays_a = body_a["relays"][&pk_a_hex]
            .as_array()
            .expect("NIP-05 response from A must include a relays map entry for A's pubkey");
        let relay_a_url = relays_a
            .first()
            .and_then(|v| v.as_str())
            .expect("relays[] must contain at least one URL string");
        assert!(
            relay_a_url.contains(&domain_a),
            "NIP-05 relays map on host A must advertise A's own host \
             (expected to contain {domain_a:?}); got {relay_a_url:?}. If \
             this advertises the global config.relay_url, the line-52 \
             tenant-host drift in api/nip05.rs has regressed."
        );

        // (2) Mirror: same local-part on B resolves to B's pubkey, not A's.
        let nip05_url_b = format!("{http_b}/.well-known/nostr.json?name={local}");
        let resp_b = reqwest::get(&nip05_url_b)
            .await
            .unwrap_or_else(|e| panic!("GET {nip05_url_b} failed: {e}"));
        assert_eq!(
            resp_b.status(),
            reqwest::StatusCode::OK,
            "NIP-05 lookup on B must return 200"
        );
        let body_b: serde_json::Value = resp_b.json().await.expect("parse NIP-05 JSON from B");
        let resolved_b = body_b["names"][&local].as_str();
        assert_eq!(
            resolved_b,
            Some(pk_b_hex.as_str()),
            "NIP-05 lookup on B for local-part {local:?} must resolve to B's pubkey \
             ({pk_b_hex}); got {resolved_b:?}. If this is A's pubkey ({pk_a_hex}), \
             the community fence on `get_user_by_nip05` has been dropped and A's \
             user leaked through B's lookup."
        );
        let relays_b = body_b["relays"][&pk_b_hex]
            .as_array()
            .expect("NIP-05 response from B must include a relays map entry for B's pubkey");
        let relay_b_url = relays_b
            .first()
            .and_then(|v| v.as_str())
            .expect("relays[] must contain at least one URL string");
        assert!(
            relay_b_url.contains(&domain_b),
            "NIP-05 relays map on host B must advertise B's own host \
             (expected to contain {domain_b:?}); got {relay_b_url:?}. \
             Mirror of A's host-local identity check."
        );

        client_a.disconnect().await.expect("disconnect A");
        client_b.disconnect().await.expect("disconnect B");
    }
}

// ---------------------------------------------------------------------------
// Channel-less global events and DMs (Mari+Max — db+pubsub)
// ---------------------------------------------------------------------------
mod channelless_global_events_dms {
    use super::*;

    /// Obligation: same event id / d-tag / pubkey can co-exist in two
    /// communities; direct `GET /api/events/{id}` and REQ filter by community
    /// first; NIP-33 uniqueness is `(community_id, kind, pubkey, d_tag)`.
    #[tokio::test]
    #[ignore]
    async fn same_event_id_and_dtag_coexist_across_communities() {
        pending_lane(
            "buzz-db",
            "same id/d-tag in A and B both retrievable, each scoped; no cross-fetch",
        );
    }

    /// Obligation: a DM `#p` in A does not cross-deliver to B.
    #[tokio::test]
    #[ignore]
    async fn dm_does_not_cross_deliver_between_communities() {
        pending_lane(
            "buzz-pubsub",
            "DM addressed in A never fans out to the same pubkey's B subscription",
        );
    }
}

// ---------------------------------------------------------------------------
// Feed read-side isolation (Perci — buzz-db/relay bridge)
// ---------------------------------------------------------------------------
mod feed_read_side_isolation {
    use super::*;

    /// Obligation: two-host feed reads are scoped by the request host's
    /// community. A mention/feed item for the same pubkey in A must never be
    /// returned by B's feed endpoint, including the empty-accessible-channel
    /// case where SQL must mean "community-global only" rather than "all
    /// channels".
    ///
    /// DB-level adversarial regressions for this exact leak shape landed in
    /// `crates/buzz-db/src/feed.rs`; this row is the wire-level conformance
    /// lane left pending until the two-host feed client harness is wired.
    #[tokio::test]
    #[ignore]
    async fn feed_mentions_do_not_cross_communities_over_the_wire() {
        pending_lane(
            "feed read-side conformance",
            concat!(
                "two-host feed mentions query for the same pubkey returns only the ",
                "host-derived community's events; empty accessible-channel lists ",
                "remain community-global only",
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// Channels and channel membership (Mari — buzz-db)
// ---------------------------------------------------------------------------
mod channels_membership {
    use super::*;

    use buzz_test_client::BuzzTestClient;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// Convert any base form to `ws(s)://` for WS connect.
    fn to_ws(base: &str) -> String {
        if base.starts_with("ws://") || base.starts_with("wss://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("https://", "wss://")
                .replace("http://", "ws://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// Convert any base form to `http(s)://` for REST.
    fn to_http(base: &str) -> String {
        if base.starts_with("http://") || base.starts_with("https://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("wss://", "https://")
                .replace("ws://", "http://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// Create a `visibility=open`, `channel_type=stream` channel with a
    /// caller-chosen UUID in the community resolved by `http_base` (the relay
    /// derives community from the request host; UUID is client-supplied per
    /// kind:9007 semantics, and the community is server-resolved). Returns
    /// the channel UUID hex (== `channel_uuid` input).
    ///
    /// Using a caller-supplied UUID is the load-bearing setup for this row:
    /// the obligation asserts the *same UUID* in two communities, which the
    /// channels PK `(community_id, id)` permits. The relay accepts the UUID
    /// because it is paired with the community-from-host, not with a
    /// client-supplied tenant — verify by reading the create-channel path in
    /// `crates/buzz-relay/src/handlers/side_effects.rs::handle_create_group`.
    async fn create_channel(http_base: &str, keys: &Keys, channel_uuid: uuid::Uuid) -> String {
        let client = reqwest::Client::new();
        let pubkey_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(9007), "")
            .tags(vec![
                Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
                Tag::parse(["name", &format!("conformance-channels-{channel_uuid}")]).unwrap(),
                Tag::parse(["channel_type", "stream"]).unwrap(),
                Tag::parse(["visibility", "open"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let resp = client
            .post(format!("{http_base}/events"))
            .header("X-Pubkey", &pubkey_hex)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&event).unwrap())
            .send()
            .await
            .expect("submit create-channel");
        assert!(
            resp.status().is_success(),
            "create-channel HTTP failed against {http_base}: {}",
            resp.status()
        );
        let body: serde_json::Value = resp.json().await.expect("parse create-channel response");
        assert!(
            body["accepted"].as_bool().unwrap_or(false),
            "create-channel not accepted against {http_base}: {body}"
        );
        channel_uuid.to_string()
    }

    /// Post a kind:9 message to `channel_id` over a WS-AUTH'd connection,
    /// returning the event id hex. The post succeeds because the channel is
    /// `visibility=open` in its host-derived community; the `#h: channel_id`
    /// tag is checked against `tenant.community()` (the host-derived
    /// community), so a post to A's channel via A's connection resolves to
    /// A's channel row, never B's.
    async fn post_kind9(
        client: &mut BuzzTestClient,
        keys: &Keys,
        channel_id: &str,
        content: &str,
    ) -> String {
        let h_tag = Tag::parse(["h", channel_id]).unwrap();
        let event = EventBuilder::new(Kind::Custom(9), content)
            .tags([h_tag])
            .sign_with_keys(keys)
            .unwrap();
        let id_hex = event.id.to_hex();
        let ok = client.send_event(event).await.expect("send kind:9");
        assert!(ok.accepted, "kind:9 not accepted: {}", ok.message);
        id_hex
    }

    /// Query kind:9 events on a given channel via REST `POST /query`
    /// (dev-mode `X-Pubkey` auth — no NIP-98 mint needed under the
    /// `BUZZ_REQUIRE_AUTH_TOKEN=false` recipe). Returns the events as their
    /// raw JSON values (typically 0 or more depending on what the
    /// host-derived community has stored against that channel id).
    async fn query_kind9_in_channel(
        http_base: &str,
        pubkey_hex: &str,
        channel_id: &str,
    ) -> Vec<serde_json::Value> {
        let client = reqwest::Client::new();
        let filters = serde_json::json!([{
            "kinds": [9],
            "#h": [channel_id],
            "limit": 100,
        }]);
        let resp = client
            .post(format!("{http_base}/query"))
            .header("X-Pubkey", pubkey_hex)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&filters).unwrap())
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST /query against {http_base} failed: {e}"));
        assert!(
            resp.status().is_success(),
            "POST /query against {http_base} returned {}",
            resp.status()
        );
        resp.json().await.expect("parse /query JSON")
    }

    /// Obligation: the same channel UUID legitimately co-exists in two
    /// communities (DB PK `(community_id, id)`); an `h` tag resolving to a
    /// channel in another community is rejected generically.
    ///
    /// # What this row asserts: the **coexistence** half of the obligation
    ///
    /// This row's scope is the **positive arm** of the same `is_member_cached`
    /// scope branch that [`super::row_zero_host_binding::client_supplied_community_cannot_override_host`]
    /// exercises as the **negative arm**. Sibling-not-replacement, per the
    /// frame Dawn established when reading row_zero (b):
    ///
    ///   * **row_zero (b) (override-attempt)**: channel U exists *only in B*;
    ///     an A connection `#h`-tagging it is rejected (the negative arm of
    ///     `is_member_cached(A, U)`, which returns `false` because
    ///     `get_channel(A, U)` is `None`). Proves: client-supplied claim
    ///     never overrides host-derived community.
    ///   * **this row (coexistence)**: channel U exists in *both* A and B
    ///     as distinct rows (legal under the `(community_id, id)` PK);
    ///     posts in A land only in A's instance; posts in B land only in
    ///     B's instance. The positive arm of `is_member_cached`: when U
    ///     exists in the tenant-derived community, the membership/permission
    ///     check finds the *right* row, not the other community's.
    ///
    /// A bug that resolves `get_channel`/`is_member_cached` against the
    /// claimed community instead of the host-derived one would pass row_zero
    /// (b)'s negative-arm test (both sides would still reject A's post
    /// against B-only's U for some reason) but would fail this row's
    /// positive-arm test (A's post might land in B's channel, or vice versa).
    /// So this row catches a class of bugs row_zero (b) structurally cannot,
    /// even though both share the `is_member_cached` scope branch.
    ///
    /// # Wire-observable shape
    ///
    /// One keypair shared across both communities — that's load-bearing for
    /// the test: it proves the fence is `community_id`, not `pubkey`. Same
    /// channel UUID `U` created in both A and B. Same pubkey posts kind:9 to
    /// U in A (`"A message"`) and to U in B (`"B message"`).
    ///
    /// REST `POST /query` with `{kinds:[9], #h:[U]}` against A's host
    /// returns *exactly* A's message; same query against B's host returns
    /// *exactly* B's message. Each side's response has count == 1 and
    /// content == its own community's. A cross-community leak surfaces as
    /// either count == 2 OR a hit whose content is the *other* community's.
    ///
    /// # Distinct content per community: the setup-equivalence-vacuity defense
    ///
    /// Per the named lesson in `landed-on-head-discipline` (Quinn's
    /// `search_fts` + Dawn's `audit_log`), identical event content collapses
    /// distinct rows into colliding Nostr event ids (id = hash(pubkey,
    /// created_at, kind, tags, content); community is server-side provenance,
    /// not in the id hash). Distinct content per community produces distinct
    /// event ids and makes the leak observable as
    /// wrong-content-on-the-wire, not just same-id-different-row (which
    /// looks identical to the honest path).
    ///
    /// # Mutate-bite (would-it-fail-without-the-fix)
    ///
    /// Drop `WHERE community_id = $1` from
    /// `crates/buzz-db/src/event.rs::query_events` (the non-p-tag branch,
    /// currently lines 266-270 form
    /// `FROM events WHERE community_id = $1` → `FROM events WHERE TRUE`).
    /// With distinct content per community + the shared `#h: U` filter, A's
    /// `/query` now returns BOTH events (different ids, both matching
    /// `channel_id = U` because U exists as both A's and B's channel rows
    /// under the same UUID). Either `hits_a.len() == 1` fails (count==2) or
    /// the content assertion fails (the leaked row substitutes for A's).
    /// The bite is observable on the wire either way. Restore → GREEN.
    ///
    /// # Single-fence-per-path topology
    ///
    /// Unlike `search_fts`, which had defense-in-depth (FTS predicate +
    /// `get_events_by_ids` re-filter), `query_events`'s `WHERE community_id`
    /// is the only community fence on the `POST /query` read path. Single-
    /// fence mutation will bite immediately. The `get_channel` fence at
    /// `channel.rs:275` is the *write/permission-check* fence (exercised by
    /// `is_member_cached` on the post path), not the read fence; together
    /// they're the two-layer scoping of the *whole channel system*, but for
    /// this row's wire-observable read property the load-bearing fence is
    /// `query_events`.
    #[tokio::test]
    #[ignore]
    async fn same_channel_uuid_in_two_communities_is_isolated() {
        let ws_a = to_ws(&url_a());
        let ws_b = to_ws(&url_b());
        let http_a = to_http(&url_a());
        let http_b = to_http(&url_b());

        // One keypair shared across both communities — proves the fence is
        // community, not pubkey.
        let keys = Keys::generate();
        let pubkey_hex = keys.public_key().to_hex();

        // Same channel UUID `U` in both communities. The `(community_id, id)`
        // PK permits this; this row's whole obligation rests on it.
        let shared_uuid = uuid::Uuid::new_v4();
        let chan_a = create_channel(&http_a, &keys, shared_uuid).await;
        let chan_b = create_channel(&http_b, &keys, shared_uuid).await;
        assert_eq!(
            chan_a, chan_b,
            "channels must share UUID — test design (the PK `(community_id, id)` makes this legal)"
        );

        // Community-distinct content. The leak between A's and B's instances
        // of the same UUID becomes wire-observable only when each side's
        // message has identifying content; identical content collapses both
        // events to the same Nostr event id and a leak becomes
        // indistinguishable from the honest path on the wire.
        let content_a = "A message in shared-UUID channel".to_string();
        let content_b = "B message in shared-UUID channel".to_string();

        // Connect each side, post each side's kind:9.
        let mut client_a = BuzzTestClient::connect(&ws_a, &keys)
            .await
            .expect("connect A");
        let _id_a = post_kind9(&mut client_a, &keys, &chan_a, &content_a).await;

        let mut client_b = BuzzTestClient::connect(&ws_b, &keys)
            .await
            .expect("connect B");
        let _id_b = post_kind9(&mut client_b, &keys, &chan_b, &content_b).await;

        // Let the relay's write-path settle. `events` is committed
        // synchronously with the OK response, but `received_at`/Redis fan-out
        // and the channel materialised-row updates may race a fast follow-up
        // read; the same 500ms wait used by every other row in this file.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // (1) A's query: shared UUID `U` resolves to A's channel row; the
        // events returned must be A's message, count == 1.
        let hits_a = query_kind9_in_channel(&http_a, &pubkey_hex, &chan_a).await;
        assert_eq!(
            hits_a.len(),
            1,
            "A's /query for kind:9 #h:{chan_a} returned {} events; expected exactly 1. \
             cross-community leak suspected. hit contents: {:?}",
            hits_a.len(),
            hits_a
                .iter()
                .map(|e| e["content"].as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        );
        let a_content = hits_a[0]["content"].as_str().unwrap_or("");
        assert_eq!(
            a_content, content_a,
            "A's kind:9 content is not A's message — B's message leaked through. \
             got: {a_content:?}"
        );

        // (2) Mirror: B's query returns B's message, count == 1.
        let hits_b = query_kind9_in_channel(&http_b, &pubkey_hex, &chan_b).await;
        assert_eq!(
            hits_b.len(),
            1,
            "B's /query for kind:9 #h:{chan_b} returned {} events; expected exactly 1. \
             cross-community leak suspected. hit contents: {:?}",
            hits_b.len(),
            hits_b
                .iter()
                .map(|e| e["content"].as_str().unwrap_or("").to_string())
                .collect::<Vec<_>>()
        );
        let b_content = hits_b[0]["content"].as_str().unwrap_or("");
        assert_eq!(
            b_content, content_b,
            "B's kind:9 content is not B's message — A's message leaked through. \
             got: {b_content:?}"
        );

        client_a.disconnect().await.expect("disconnect A");
        client_b.disconnect().await.expect("disconnect B");
    }
}

// ---------------------------------------------------------------------------
// Workflows / runs / approvals / webhooks / schedules (Mari+Max)
// ---------------------------------------------------------------------------
mod workflows {
    use super::*;

    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// Workflow definition command (NIP-custom kind 30620). `content` is the
    /// YAML body; `h` tags the channel. The server *generates* the workflow id
    /// and returns it in the OK message — it is **not** the `d` tag.
    const KIND_WORKFLOW_DEF: u16 = 30620;
    /// Workflow trigger command (kind 46020). `d` tag = the server-generated
    /// workflow id to fire.
    const KIND_WORKFLOW_TRIGGER: u16 = 46020;

    /// Convert any base form to `http(s)://` for the REST `POST /events` door.
    fn to_http(base: &str) -> String {
        if base.starts_with("http://") || base.starts_with("https://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("wss://", "https://")
                .replace("ws://", "http://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// A minimal webhook-triggered workflow YAML. `send_message` is the simplest
    /// valid action; the trigger type is irrelevant to *this* row (we fire via
    /// the kind:46020 command door, not the webhook door), but it must parse.
    fn workflow_yaml(name: &str) -> String {
        format!(
            "name: {name}\n\
             description: conformance trigger-isolation probe\n\
             trigger:\n\
             \x20 on: webhook\n\
             steps:\n\
             \x20 - id: step1\n\
             \x20   name: Notify\n\
             \x20   action: send_message\n\
             \x20   text: \"conformance\"\n"
        )
    }

    /// Submit a signed event to the community bound to `http_base`'s host via
    /// the REST bridge (`POST /events`). In dev mode (`BUZZ_REQUIRE_AUTH_TOKEN
    /// =false`) the `X-Pubkey` header authenticates. Returns the parsed JSON
    /// `{accepted, message, ...}` body. The community is derived from the host,
    /// never from anything in the event — that's row zero.
    async fn submit_event(http_base: &str, keys: &Keys, event: nostr::Event) -> serde_json::Value {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{http_base}/events"))
            .header("X-Pubkey", keys.public_key().to_hex())
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&event).expect("serialize event"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST /events to {http_base} failed: {e}"));
        let status = resp.status();
        let body = resp.text().await.expect("read /events body");
        assert!(
            status.is_success(),
            "POST /events to {http_base} returned HTTP {status}: {body}"
        );
        serde_json::from_str(&body)
            .unwrap_or_else(|e| panic!("parse /events JSON from {http_base}: {e} (body: {body})"))
    }

    /// Create an `open`-visibility channel with a caller-chosen UUID in the
    /// community bound to `http_base`'s host (kind:9007, h-tag UUID → the
    /// `create_channel_with_id` path, which bootstraps the creator as
    /// owner-member). Using a caller-supplied UUID lets the test reuse the
    /// *same* channel UUID across both communities: the PK is `(community_id,
    /// id)`, so the same UUID legitimately co-exists, and `created_by` is
    /// inserted into `channel_members` as `owner` on **each** side — so the one
    /// keypair is a member of `U` in A *and* in B. That shared membership is
    /// load-bearing: it removes "not a member" as an alternate explanation for
    /// B rejecting the cross-community trigger, leaving the community fence on
    /// `get_workflow` as the *only* thing that can produce the rejection.
    async fn create_open_channel(http_base: &str, keys: &Keys, channel_uuid: uuid::Uuid) -> String {
        let event = EventBuilder::new(Kind::Custom(9007), "")
            .tags(vec![
                Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
                Tag::parse(["name", &format!("conformance-wf-{channel_uuid}")]).unwrap(),
                Tag::parse(["channel_type", "stream"]).unwrap(),
                Tag::parse(["visibility", "open"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let body = submit_event(http_base, keys, event).await;
        assert!(
            body["accepted"].as_bool().unwrap_or(false),
            "create-channel not accepted against {http_base}: {body}"
        );
        channel_uuid.to_string()
    }

    /// Define a workflow in `channel_id` on `http_base`'s community (kind:30620,
    /// `h`=channel, content=YAML). Returns the **server-generated** workflow id,
    /// parsed out of the OK message (`response:{"workflow_id":"…"}`). This id is
    /// the tenant-scoped handle the trigger door confines: defined under A, it
    /// only resolves under A.
    async fn define_workflow(http_base: &str, keys: &Keys, channel_id: &str, name: &str) -> String {
        // `h` binds the channel; `name` is required by `handle_workflow_def`
        // (it rejects "missing workflow name" before parsing YAML). We use the
        // `name` tag, not `d`: the server *generates* the workflow id, and that
        // generated id — not any client-supplied `d` — is the handle this row
        // confines. A `d` tag here would falsely imply the trigger resolves by
        // client key.
        let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF), workflow_yaml(name))
            .tags(vec![
                Tag::parse(["h", channel_id]).unwrap(),
                Tag::parse(["name", name]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let body = submit_event(http_base, keys, event).await;
        assert!(
            body["accepted"].as_bool().unwrap_or(false),
            "workflow def not accepted against {http_base}: {body}"
        );
        // The command executor returns `message: "response:{json}"` where json
        // carries `workflow_id`. Extract it.
        let msg = body["message"].as_str().unwrap_or_default();
        let json_part = msg.strip_prefix("response:").unwrap_or_else(|| {
            panic!("workflow def OK message missing `response:` prefix: {msg:?}")
        });
        let resp: serde_json::Value = serde_json::from_str(json_part)
            .unwrap_or_else(|e| panic!("parse workflow def response json: {e} ({json_part:?})"));
        resp["workflow_id"]
            .as_str()
            .unwrap_or_else(|| panic!("workflow def response missing workflow_id: {resp}"))
            .to_string()
    }

    /// Fire a workflow by id on `http_base`'s community (kind:46020, `d`=id).
    /// Returns a normalized `{accepted, message}` body so the caller can assert
    /// on the *wire-observable* accept/reject and message. The HTTP bridge maps
    /// `IngestError::Rejected` to HTTP 400 + `{error}` while the WS door maps the
    /// same condition to `OK false`; for this conformance row either envelope is
    /// acceptable. The safety property is the scoped lookup and generic message.
    async fn trigger_workflow(
        http_base: &str,
        keys: &Keys,
        workflow_id: &str,
    ) -> serde_json::Value {
        let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_TRIGGER), "")
            .tags(vec![Tag::parse(["d", workflow_id]).unwrap()])
            .sign_with_keys(keys)
            .unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{http_base}/events"))
            .header("X-Pubkey", keys.public_key().to_hex())
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&event).expect("serialize event"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST workflow trigger to {http_base} failed: {e}"));
        let status = resp.status();
        let body = resp.text().await.expect("read workflow trigger body");
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|e| {
            panic!("parse workflow trigger JSON from {http_base}: {e} (body: {body})")
        });
        if status.is_success() {
            return parsed;
        }
        if status == reqwest::StatusCode::BAD_REQUEST {
            return serde_json::json!({
                "accepted": false,
                "message": parsed["error"].as_str().unwrap_or_default(),
            });
        }
        panic!("POST workflow trigger to {http_base} returned HTTP {status}: {body}");
    }

    /// Obligation (trigger-confinement half): a workflow id defined under
    /// community A is triggerable only under A. Firing A's id under host B —
    /// even by a caller who is a legitimate member of the *same channel UUID* in
    /// B — must fail closed, because trigger resolution is
    /// `get_workflow(host_community, id)` and the row lives in A's community
    /// only. The mirror positive (A fires its own id) must succeed, proving the
    /// rejection is the community fence and not a workflow that is simply
    /// untriggerable.
    ///
    /// Wire-observable shape (single keypair K; the fence under test must be
    /// `community_id`, never `pubkey` or channel membership):
    ///   1. Create the **same** channel UUID `U` in A and in B. PK is
    ///      `(community_id, id)`, so both inserts succeed and K is bootstrapped
    ///      as owner-member of `U` on *each* side (`create_channel_with_id`).
    ///      This deliberately removes "not a member of U in B" as an alternate
    ///      cause of the B rejection — K *is* a member of U in B.
    ///   2. Define a workflow in `U` under **A** (kind:30620). The server
    ///      generates `W` and returns it. `W` is an A-community row.
    ///   3. Fire `W` under host **B** (kind:46020, `d`=W) as K. Must be
    ///      rejected — `accepted == false` and the generic `workflow not found`
    ///      message — because `get_workflow(B_community, W)` finds nothing: `W`
    ///      exists only in A. K's membership of U-in-B is irrelevant; the
    ///      lookup never reaches the membership check.
    ///   4. Fire `W` under host **A** as K. Must be accepted — the positive
    ///      half proves the rejection in (3) is community confinement, not a
    ///      workflow that can never trigger. (This also exercises the
    ///      same-community happy path through the very fence we're testing.)
    ///
    /// Mutate-bite (would-it-fail-without-the-fix): drop the community fence on
    /// the trigger lookup at
    /// `crates/buzz-relay/src/handlers/command_executor.rs:703`
    /// (`get_workflow(community_id, workflow_id)` → bare-id lookup, e.g.
    /// `get_workflow_any(workflow_id)`). Then B's trigger in step 3 loads A's
    /// workflow row, passes the membership check against B's colliding channel
    /// `U` (K is a member there), and **accepts** — step 3's `accepted == false`
    /// assertion goes RED. Restore the `community_id` argument → GREEN. This is
    /// the exact invariant commit `c81b89355` documents at that call site.
    ///
    /// NOTE — approval-token isolation is a **separate, not-yet-wire-live**
    /// obligation, deliberately left as a `pending_lane` below. The grant
    /// handler (`get_approval_by_stored_hash(community, hash)`) is already
    /// community-scoped, but nothing *mints* a pending approval over the wire:
    /// the executor's approval gate is an explicit TODO
    /// (`crates/buzz-workflow/src/lib.rs` — "approval gates not yet implemented,
    /// see WF-08") and `create_approval` is only reached from unit tests. A
    /// green end-to-end approval-isolation test therefore cannot be exercised
    /// today; writing one would violate this file's contract (a green run can
    /// never be faked by an empty/DB-only body). It lands with WF-08.
    #[tokio::test]
    #[ignore]
    async fn workflow_trigger_is_community_confined() {
        let http_a = to_http(&url_a());
        let http_b = to_http(&url_b());

        // One keypair across both communities — the fence under test must be
        // community_id, never pubkey.
        let keys = Keys::generate();

        // (1) Same channel UUID in both communities. (community_id, id) PK
        // permits this; K becomes owner-member of U on *each* side, so K's
        // membership of U-in-B cannot explain the B rejection in step (3).
        let shared_uuid = uuid::Uuid::new_v4();
        let chan_a = create_open_channel(&http_a, &keys, shared_uuid).await;
        let chan_b = create_open_channel(&http_b, &keys, shared_uuid).await;
        assert_eq!(chan_a, chan_b, "channels must share UUID — test design");

        // (2) Define the workflow under A. Server generates W.
        let name = format!("wfconf_{}", uuid::Uuid::new_v4().simple());
        let workflow_id = define_workflow(&http_a, &keys, &chan_a, &name).await;
        assert!(
            uuid::Uuid::parse_str(&workflow_id).is_ok(),
            "server-generated workflow_id must be a UUID, got {workflow_id:?}"
        );

        // (3) Fire W under host B as K. Must fail closed: W is an A-community
        // row, and get_workflow(B_community, W) finds nothing. K is a member of
        // U in B, so a leak here is the community fence failing, not membership.
        let b_resp = trigger_workflow(&http_b, &keys, &workflow_id).await;
        assert_eq!(
            b_resp["accepted"].as_bool(),
            Some(false),
            "host B accepted a trigger for an A-community workflow id — cross-community \
             trigger leak. response: {b_resp}"
        );
        let b_msg = b_resp["message"].as_str().unwrap_or_default();
        assert!(
            b_msg.contains("workflow not found"),
            "host B rejection must be the generic `workflow not found` (no enumeration \
             oracle); got: {b_msg:?}"
        );

        // (4) Mirror positive: fire W under host A as K. Must be accepted —
        // proves the B rejection is community confinement, not an
        // untriggerable workflow, and exercises the same-community happy path
        // through the fence under test.
        let a_resp = trigger_workflow(&http_a, &keys, &workflow_id).await;
        assert_eq!(
            a_resp["accepted"].as_bool(),
            Some(true),
            "host A rejected a trigger for its own workflow id — positive control failed, \
             so the B rejection cannot be attributed to community confinement. response: \
             {a_resp}"
        );
    }

    /// Obligation (approval-token half): an approval token (its stored hash)
    /// minted under community A cannot be satisfied by a grant on host B, and
    /// vice versa. The grant resolution is already community-scoped
    /// (`get_approval_by_stored_hash(community, hash)`), but this half is
    /// **not wire-live**: nothing mints a pending approval over the wire yet —
    /// the executor approval gate is an explicit TODO (WF-08), and
    /// `create_approval` is reached only from unit tests. Left as a precise
    /// `pending_lane` so the WF-08 owner fills in *their* row; a green run can
    /// never be faked by a DB-only/empty body. Depends on WF-08 (approval
    /// minting), **not** buzz-db — the scoping fence it will exercise is
    /// already landed.
    #[tokio::test]
    #[ignore]
    async fn approval_token_is_community_confined() {
        pending_lane(
            "WF-08 (approval minting)",
            "an approval token minted under A cannot be satisfied by a grant on host B — \
             blocked until the executor approval gate (WF-08) mints pending approvals over \
             the wire; the get_approval_by_stored_hash(community, hash) fence is already landed",
        );
    }
}

// ---------------------------------------------------------------------------
// Search / FTS (Quinn — buzz-search)
// ---------------------------------------------------------------------------
mod search_fts {
    use super::*;

    use buzz_test_client::{BuzzTestClient, RelayMessage};
    use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};

    /// Convert an `http(s)://host[:port]` base into the `ws(s)://` form the
    /// websocket client needs. The conformance docstring documents URLs as
    /// `http://` for human/REST clarity; the WS upgrade still happens on the
    /// same host:port.
    fn to_ws(base: &str) -> String {
        if base.starts_with("ws://") || base.starts_with("wss://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("https://", "wss://")
                .replace("http://", "ws://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// Convert any base form to `http(s)://` for REST calls.
    fn to_http(base: &str) -> String {
        if base.starts_with("http://") || base.starts_with("https://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("wss://", "https://")
                .replace("ws://", "http://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// Create a visibility=open channel with a caller-chosen UUID in the
    /// community resolved by `http_base` (the relay derives community from the
    /// request host, never from caller input — that's row zero). Using a
    /// caller-supplied UUID lets the test reuse the *same* channel UUID across
    /// two communities, which is the load-bearing shape: PK is
    /// `(community_id, id)`, so the same UUID legitimately co-exists, and the
    /// only thing keeping A's events from surfacing under B's `#h:UUID` search
    /// is the FTS `community_id` predicate.
    async fn create_channel(http_base: &str, keys: &Keys, channel_uuid: uuid::Uuid) -> String {
        let client = reqwest::Client::new();
        let pubkey_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(9007), "")
            .tags(vec![
                Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
                Tag::parse(["name", &format!("conformance-fts-{channel_uuid}")]).unwrap(),
                Tag::parse(["channel_type", "stream"]).unwrap(),
                Tag::parse(["visibility", "open"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let resp = client
            .post(format!("{http_base}/events"))
            .header("X-Pubkey", &pubkey_hex)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&event).unwrap())
            .send()
            .await
            .expect("submit create-channel");
        assert!(
            resp.status().is_success(),
            "create-channel HTTP failed against {http_base}: {}",
            resp.status()
        );
        let body: serde_json::Value = resp.json().await.expect("parse create-channel response");
        assert!(
            body["accepted"].as_bool().unwrap_or(false),
            "create-channel not accepted against {http_base}: {body}"
        );
        channel_uuid.to_string()
    }

    /// Post a kind:9 with `content` to `channel_id` over the WS connection
    /// `client`. Returns the event id hex (so we can target it with NIP-09).
    async fn post_kind9(
        client: &mut BuzzTestClient,
        keys: &Keys,
        channel_id: &str,
        content: &str,
    ) -> String {
        let h_tag = Tag::parse(["h", channel_id]).unwrap();
        let event = EventBuilder::new(Kind::Custom(9), content)
            .tags([h_tag])
            .sign_with_keys(keys)
            .unwrap();
        let id_hex = event.id.to_hex();
        let ok = client.send_event(event).await.expect("send kind:9");
        assert!(ok.accepted, "kind:9 not accepted: {}", ok.message);
        id_hex
    }

    /// Run a one-shot NIP-50 search for `token` scoped to `channel_id` and
    /// return the events received before EOSE.
    async fn search_for(
        client: &mut BuzzTestClient,
        channel_id: &str,
        token: &str,
    ) -> Vec<nostr::Event> {
        let sub_id = format!("fts-{}", uuid::Uuid::new_v4());
        let filter = Filter::new()
            .kind(Kind::Custom(9))
            .search(token)
            .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel_id]);
        client
            .subscribe(&sub_id, vec![filter])
            .await
            .expect("subscribe");
        client
            .collect_until_eose(&sub_id, Duration::from_secs(10))
            .await
            .expect("collect until EOSE")
    }

    /// Obligation: every search `filter` includes `community_id`; same
    /// channel/token in A and B return only same-community hits; deleting in A
    /// does not delete the B document. Enforced in Postgres FTS
    /// (search_tsv/GIN), not a separate search engine.
    ///
    /// Shape (designed so the wire-observable property is a *single*
    /// per-community row whose content is the community's own):
    ///   1. Pick one keypair (same author across both communities — proves the
    ///      fence is `community_id`, not `pubkey`).
    ///   2. Create the *same* channel UUID `U` in A and in B (PK is
    ///      `(community_id, id)`, so this is legitimate). Each side's
    ///      `accessible_channels` therefore contains `U`, so a search REQ with
    ///      `#h: U` from either side passes the `accessible_channels`
    ///      intersect.
    ///   3. Post kind:9 with the shared FTS token but **community-distinct
    ///      content** (`"A community probe {token}"` vs `"B community probe
    ///      {token}"`) to `U` in A and to `U` in B. Distinct content → distinct
    ///      Nostr event ids (id is a hash of (pubkey, created_at, kind, tags,
    ///      content), so different content hashes to different ids); the
    ///      shared token still makes both rows match the FTS predicate. A
    ///      cross-community leak therefore shows up on the wire as either a
    ///      second hit (count == 2) OR a hit whose content is the *other*
    ///      community's — earlier iterations of this test used identical
    ///      content and discovered the hard way that identical content
    ///      collapses both leak modes into "indistinguishable on the wire"
    ///      (defense-in-depth at the FTS + `get_events_by_ids` layers ate the
    ///      mutation by returning the live row regardless of which community
    ///      claimed it).
    ///   4. NIP-50 search on A with `#h: U` for that token: returns exactly
    ///      one hit whose content is `content_a`.
    ///   5. Mirror: NIP-50 search on B with `#h: U` returns exactly one hit
    ///      whose content is `content_b`.
    ///   6. NIP-09 kind:5 deletion against A's event over the A connection
    ///      (`#h: U`, `#e: id_a`).
    ///   7. Re-search on A: zero hits (search excludes `deleted_at IS NOT
    ///      NULL`).
    ///   8. Re-search on B: B's hit unchanged (delete did not cross), content
    ///      still equals `content_b`.
    ///
    /// Mutate-bite to confirm load-bearing: drop the community fences at
    /// BOTH layers of the search read path simultaneously:
    ///   - `crates/buzz-search/src/query.rs::search` lines 160-161 (the FTS
    ///     `WHERE community_id = $ctx` predicate); and
    ///   - `crates/buzz-db/src/event.rs::get_events_by_ids` lines 870-872
    ///     (the read-side `WHERE community_id = $1` predicate).
    /// Replace each with `WHERE TRUE` to keep SQL syntax valid; the second
    /// fence is reached via `state.db.get_events_by_ids(tenant.community(),
    /// ...)` from `handlers/req.rs::handle_search_req` at line ~590. With
    /// distinct content per community (see step 3), A's search now returns
    /// *both* communities' rows (different ids, both matching `#h: U` via the
    /// shared channel UUID, both matching FTS via the shared token). Step 4's
    /// `hits_a.len() == 1` assertion goes RED with the failure message
    /// listing both contents — explicitly "B community probe …" surfacing
    /// inside A's wire response. Restore both fences → GREEN.
    ///
    /// Defense-in-depth: each layer alone catches the leak. Mutating only the
    /// FTS fence leaves the read fence to filter to A's community; mutating
    /// only the read fence leaves FTS to never emit B's id in the first
    /// place. Both must drop for the wire-observable property to fail. The
    /// two non-negotiable predicates the obligation text names live at these
    /// two layers; the test's red-with-both-down evidence is that the union
    /// of those fences is what makes the property wire-observable, and each
    /// fence individually is non-vacuous (single-fence mutation keeps the
    /// test green because the other defends). This is the correct,
    /// honest mutate-bite for an obligation defended by redundant layers.
    #[tokio::test]
    #[ignore]
    async fn search_results_and_deletes_are_community_scoped() {
        let ws_a = to_ws(&url_a());
        let ws_b = to_ws(&url_b());
        let http_a = to_http(&url_a());
        let http_b = to_http(&url_b());

        // One keypair shared across both communities: every cross-community
        // filter we apply must be community_id, never pubkey.
        let keys = Keys::generate();

        // Same channel UUID in both communities. The (community_id, id) PK
        // permits this and the test depends on it: see module docstring above.
        let shared_uuid = uuid::Uuid::new_v4();
        let chan_a = create_channel(&http_a, &keys, shared_uuid).await;
        let chan_b = create_channel(&http_b, &keys, shared_uuid).await;
        assert_eq!(chan_a, chan_b, "channels must share UUID — test design");

        // Unique token that cannot match anything else in the DB.
        let token = format!("ftsconf_{}", uuid::Uuid::new_v4().simple());
        // Distinct content per community. The shared token is what FTS
        // matches; the per-community label makes each row uniquely
        // identifiable on the wire AND produces distinct Nostr event ids
        // (the id hash includes content, so different content → different
        // id → no PK collision games). This is load-bearing for the
        // mutate-bite: with identical content the two communities' rows
        // would hash to the same Nostr id and a leak that returns "the
        // other community's row" would be indistinguishable on the wire
        // from "the right community's row." Distinct content makes the
        // leak observable.
        let content_a = format!("A community probe {token}");
        let content_b = format!("B community probe {token}");

        // Connect to A, post in A.
        let mut client_a = BuzzTestClient::connect(&ws_a, &keys)
            .await
            .expect("connect A");
        let id_a = post_kind9(&mut client_a, &keys, &chan_a, &content_a).await;

        // Connect to B, post in B (same key, same channel UUID, same token —
        // only the community label in the content + the community itself
        // differ).
        let mut client_b = BuzzTestClient::connect(&ws_b, &keys)
            .await
            .expect("connect B");
        let _id_b = post_kind9(&mut client_b, &keys, &chan_b, &content_b).await;

        // Let FTS write-path settle. `search_tsv` is a generated column, so it
        // commits with the row; this matches the existing e2e search test's
        // wait.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // (4) Search on A: must return exactly one hit, and that hit must be
        // the row written via the A connection (the relay returns the event
        // *body* with the canonical id, but the row provenance is community
        // A). Crucially: count==1 — a missing community fence would surface 2
        // (A's row + B's row through the shared #h:U filter).
        let hits_a = search_for(&mut client_a, &chan_a, &token).await;
        assert_eq!(
            hits_a.len(),
            1,
            "A's search returned {} hits; expected exactly 1. cross-community leak suspected. \
             hit contents: {:?}",
            hits_a.len(),
            hits_a.iter().map(|e| e.content.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            hits_a[0].content, content_a,
            "A's hit content is not A's row — B's content leaked through the wire \
             (id-equality is irrelevant; content distinguishes the communities). \
             got: {:?}",
            hits_a[0].content,
        );

        // (5) Mirror: search on B must also return exactly one hit, and that
        // hit must carry B's content (not A's).
        let hits_b = search_for(&mut client_b, &chan_b, &token).await;
        assert_eq!(
            hits_b.len(),
            1,
            "B's search returned {} hits; expected exactly 1. cross-community leak suspected. \
             hit contents: {:?}",
            hits_b.len(),
            hits_b.iter().map(|e| e.content.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            hits_b[0].content, content_b,
            "B's hit content is not B's row — A's content leaked through the wire. got: {:?}",
            hits_b[0].content,
        );

        // (6) NIP-09 delete in A targeting A's event.
        let delete_a = EventBuilder::new(Kind::Custom(5), "conformance delete")
            .tags([
                Tag::parse(["h", &chan_a]).unwrap(),
                Tag::parse(["e", &id_a]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        let ok_del = client_a.send_event(delete_a).await.expect("send delete");
        assert!(ok_del.accepted, "A delete rejected: {}", ok_del.message);

        // Soft-delete may flow through indexing async. Same wait as above.
        tokio::time::sleep(Duration::from_millis(500)).await;

        // (7) Re-search on A: A's hit gone. The search SQL has both
        // `community_id = $ctx` AND `deleted_at IS NULL`; the latter excludes
        // A's now-soft-deleted row. Count==0 in A's community.
        let hits_a_post = search_for(&mut client_a, &chan_a, &token).await;
        assert_eq!(
            hits_a_post.len(),
            0,
            "A's deleted event still returned by search. hit ids={:?}",
            hits_a_post
                .iter()
                .map(|e| e.id.to_hex())
                .collect::<Vec<_>>()
        );

        // (8) Re-search on B: B's hit unchanged — delete in A did not cross
        // into B (NIP-09 `soft_delete_event` is keyed on `(community_id,
        // event_id)`).
        let hits_b_post = search_for(&mut client_b, &chan_b, &token).await;
        assert_eq!(
            hits_b_post.len(),
            1,
            "B's hit count changed after A's delete; cross-community deletion suspected. \
             hit ids={:?}",
            hits_b_post
                .iter()
                .map(|e| e.id.to_hex())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            hits_b_post[0].content, content_b,
            "B's hit content drifted after A's delete"
        );

        // Drain any trailing live events so disconnect is clean. NIP-50 is
        // one-shot; we don't expect any.
        let _ = client_a.recv_event(Duration::from_millis(50)).await;
        let _ = client_b.recv_event(Duration::from_millis(50)).await;

        client_a.disconnect().await.expect("disconnect A");
        client_b.disconnect().await.expect("disconnect B");

        // `RelayMessage` is imported for documentation of `search_for`'s
        // return path; reference it so the unused-import lint doesn't fire.
        let _ = std::any::type_name::<RelayMessage>();
    }
}

// ---------------------------------------------------------------------------
// Redis pub/sub, presence, typing, cache invalidation (Max — buzz-pubsub)
// ---------------------------------------------------------------------------
mod pubsub_presence_typing {
    use super::*;

    use buzz_test_client::{BuzzTestClient, RelayMessage};
    use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};

    const KIND_PRESENCE_UPDATE: u16 = 20001;
    const KIND_TYPING_INDICATOR: u16 = 20002;

    /// Convert any base form to `ws(s)://` for WS connect.
    fn to_ws(base: &str) -> String {
        if base.starts_with("ws://") || base.starts_with("wss://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("https://", "wss://")
                .replace("http://", "ws://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// Convert any base form to `http(s)://` for REST.
    fn to_http(base: &str) -> String {
        if base.starts_with("http://") || base.starts_with("https://") {
            base.trim_end_matches('/').to_string()
        } else {
            base.replace("wss://", "https://")
                .replace("ws://", "http://")
                .trim_end_matches('/')
                .to_string()
        }
    }

    /// Create the same visibility=open channel UUID in the community resolved by
    /// `http_base`. Open visibility keeps the typing half focused on pub/sub
    /// fan-out scoping rather than membership setup: either host can post to its
    /// own tenant-local instance of the UUID, and the only live-delivery fence
    /// left is the server-resolved community in the subscription/fan-out index.
    async fn create_open_channel(http_base: &str, keys: &Keys, channel_uuid: uuid::Uuid) -> String {
        let client = reqwest::Client::new();
        let pubkey_hex = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(9007), "")
            .tags(vec![
                Tag::parse(["h", &channel_uuid.to_string()]).unwrap(),
                Tag::parse(["name", &format!("conformance-pubsub-{channel_uuid}")]).unwrap(),
                Tag::parse(["channel_type", "stream"]).unwrap(),
                Tag::parse(["visibility", "open"]).unwrap(),
            ])
            .sign_with_keys(keys)
            .unwrap();
        let resp = client
            .post(format!("{http_base}/events"))
            .header("X-Pubkey", &pubkey_hex)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&event).unwrap())
            .send()
            .await
            .expect("submit create-channel");
        assert!(
            resp.status().is_success(),
            "create-channel HTTP failed against {http_base}: {}",
            resp.status()
        );
        let body: serde_json::Value = resp.json().await.expect("parse create-channel response");
        assert!(
            body["accepted"].as_bool().unwrap_or(false),
            "create-channel not accepted against {http_base}: {body}"
        );
        channel_uuid.to_string()
    }

    async fn publish_presence(client: &mut BuzzTestClient, keys: &Keys, status: &str) {
        let event = EventBuilder::new(Kind::Custom(KIND_PRESENCE_UPDATE), status)
            .sign_with_keys(keys)
            .unwrap();
        let ok = client.send_event(event).await.expect("send presence");
        assert!(ok.accepted, "presence not accepted: {}", ok.message);
    }

    /// Query synthesized presence through REST `POST /query`. This exercises the
    /// bridge intercept at `api/bridge.rs::synthesize_presence`, which reads
    /// Redis via `get_presence_bulk(tenant, authors)`; there is no DB fallback
    /// for this ephemeral state.
    async fn query_presence(http_base: &str, pubkey_hex: &str) -> Vec<serde_json::Value> {
        let client = reqwest::Client::new();
        let filters = serde_json::json!([{
            "kinds": [KIND_PRESENCE_UPDATE],
            "authors": [pubkey_hex],
            "limit": 1,
        }]);
        let resp = client
            .post(format!("{http_base}/query"))
            .header("X-Pubkey", pubkey_hex)
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&filters).unwrap())
            .send()
            .await
            .unwrap_or_else(|e| panic!("POST /query against {http_base} failed: {e}"));
        assert!(
            resp.status().is_success(),
            "POST /query against {http_base} returned {}",
            resp.status()
        );
        resp.json().await.expect("parse /query JSON")
    }

    async fn subscribe_typing(client: &mut BuzzTestClient, sub_id: &str, channel_id: &str) {
        let filter = Filter::new()
            .kind(Kind::Custom(KIND_TYPING_INDICATOR))
            .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel_id]);
        client
            .subscribe(sub_id, vec![filter])
            .await
            .expect("subscribe to typing");
        let historical = client
            .collect_until_eose(sub_id, Duration::from_secs(10))
            .await
            .expect("collect typing EOSE");
        assert!(
            historical.is_empty(),
            "typing is ephemeral and should not return historical events; got {}",
            historical.len()
        );
    }

    async fn publish_typing(
        client: &mut BuzzTestClient,
        keys: &Keys,
        channel_id: &str,
        content: &str,
    ) -> String {
        let event = EventBuilder::new(Kind::Custom(KIND_TYPING_INDICATOR), content)
            .tags([Tag::parse(["h", channel_id]).unwrap()])
            .sign_with_keys(keys)
            .unwrap();
        let id_hex = event.id.to_hex();
        let ok = client.send_event(event).await.expect("send typing");
        assert!(ok.accepted, "typing not accepted: {}", ok.message);
        id_hex
    }

    /// Drain live events for `sub_id` until `quiet_for` elapses, returning only
    /// EVENT frames for that subscription. This deliberately keeps listening
    /// after the expected local event so a cross-community leak has a window to
    /// surface as a second/wrong-content live delivery.
    async fn drain_live_events(
        client: &mut BuzzTestClient,
        sub_id: &str,
        quiet_for: Duration,
    ) -> Vec<nostr::Event> {
        let mut events = Vec::new();
        loop {
            match client.recv_event(quiet_for).await {
                Ok(RelayMessage::Event {
                    subscription_id,
                    event,
                }) if subscription_id == sub_id => events.push(*event),
                Ok(_) => {}
                Err(_) => return events,
            }
        }
    }

    fn assert_one_content(events: &[nostr::Event], expected: &str, forbidden: &str, side: &str) {
        let contents = events.iter().map(|e| e.content.clone()).collect::<Vec<_>>();
        assert_eq!(
            events.len(),
            1,
            "{side}'s typing subscription received {} live events; expected exactly its own. \
             contents: {contents:?}",
            events.len()
        );
        assert_eq!(
            events[0].content, expected,
            "{side}'s typing event content is not its own — cross-community content leaked. \
             got: {:?}, expected: {:?}, forbidden other-community content: {:?}",
            events[0].content, expected, forbidden
        );
        assert!(
            !contents.iter().any(|content| content == forbidden),
            "{side}'s typing subscription saw other-community content {forbidden:?}; \
             subscription/fan-out community scoping leaked"
        );
    }

    /// Obligation: keys are `buzz:{community}:…`; cross-node fan-out never
    /// delivers an A event to a B subscription, even for the same channel UUID;
    /// the same pubkey can be online in A and away in B independently.
    ///
    /// Wire-observable shape, split across the two load-bearing pub/sub fences:
    ///
    /// 1. **Presence / Redis key fence.** The same keypair publishes kind:20001
    ///    presence with distinct statuses in A and B. Presence is ephemeral, so
    ///    `/query` is synthesized from Redis (`api/bridge.rs::synthesize_presence`
    ///    → `buzz_pubsub::get_presence_bulk`). A's query must return only A's
    ///    status and B's only B's. This bites the Redis key format in
    ///    `crates/buzz-pubsub/src/presence.rs::presence_key`, which must include
    ///    `ctx.community()` (`buzz:{community}:presence:{pubkey}`). Same pubkey
    ///    is required: it proves the isolation coordinate is community, not key.
    ///
    /// 2. **Typing / subscription fan-out fence.** The same channel UUID is
    ///    created in both communities, both sides subscribe to kind:20002 + `#h`
    ///    for that UUID, then each side publishes distinct typing content.
    ///    Because the channel UUID and kind are intentionally identical, the only
    ///    in-memory live-delivery discriminator is the server-resolved community
    ///    in `SubscriptionRegistry`'s `(CommunityId, channel/kind)` indexes and
    ///    `fan_out_scoped(community_id, event)`. Distinct content makes a leak a
    ///    wrong-answer-returned failure instead of setup-equivalence vacuity.
    ///
    /// Mutate-bites:
    ///   - Presence: drop/neutralize `ctx.community()` in
    ///     `buzz-pubsub/src/presence.rs::presence_key` (shared Redis key). The B
    ///     publish overwrites A's status for the same pubkey, so A's `/query`
    ///     returns `status_b` and the presence assertion reds.
    ///   - Typing: drop both live-delivery tenant fences at once: ignore
    ///     `community_id` in `SubscriptionRegistry::fan_out_scoped`'s
    ///     channel-kind lookup AND remove the receiver-side
    ///     `community_for_conn(conn_id) == Some(event_community)` check in
    ///     `handlers/event.rs::filter_fanout_by_access`. With the same channel
    ///     UUID + kind in both tenants, A's live typing event is delivered to B's
    ///     subscription (and/or vice versa); the exact-one-content assertions red
    ///     with the other community's content in the drain. Either fence alone is
    ///     defense-in-depth: the index prevents cross-tenant candidates, and the
    ///     send chokepoint drops any stale/injected cross-tenant candidate.
    #[tokio::test]
    #[ignore]
    async fn fanout_and_presence_do_not_cross_communities() {
        let ws_a = to_ws(&url_a());
        let ws_b = to_ws(&url_b());
        let http_a = to_http(&url_a());
        let http_b = to_http(&url_b());

        // Same key across both communities is load-bearing for presence: the
        // Redis key must be `(community, pubkey)`, not just `pubkey`.
        let keys = Keys::generate();
        let pubkey_hex = keys.public_key().to_hex();

        let mut client_a = BuzzTestClient::connect(&ws_a, &keys)
            .await
            .expect("connect A");
        let mut client_b = BuzzTestClient::connect(&ws_b, &keys)
            .await
            .expect("connect B");

        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let status_a = format!("online-a-{suffix}");
        let status_b = format!("away-b-{suffix}");
        publish_presence(&mut client_a, &keys, &status_a).await;
        publish_presence(&mut client_b, &keys, &status_b).await;

        // Let Redis writes and the bridge's subsequent read see a stable value.
        tokio::time::sleep(Duration::from_millis(250)).await;

        let presence_a = query_presence(&http_a, &pubkey_hex).await;
        assert_eq!(
            presence_a.len(),
            1,
            "A's synthesized presence query returned {} events; expected exactly 1. \
             contents: {:?}",
            presence_a.len(),
            presence_a
                .iter()
                .map(|e| e["content"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        );
        let got_status_a = presence_a[0]["content"].as_str().unwrap_or_default();
        assert_eq!(
            got_status_a, status_a,
            "A's synthesized presence is not A's status — B's status leaked/overwrote it. \
             got: {got_status_a:?}, expected: {status_a:?}, B status: {status_b:?}"
        );

        let presence_b = query_presence(&http_b, &pubkey_hex).await;
        assert_eq!(
            presence_b.len(),
            1,
            "B's synthesized presence query returned {} events; expected exactly 1. \
             contents: {:?}",
            presence_b.len(),
            presence_b
                .iter()
                .map(|e| e["content"].as_str().unwrap_or_default().to_string())
                .collect::<Vec<_>>()
        );
        let got_status_b = presence_b[0]["content"].as_str().unwrap_or_default();
        assert_eq!(
            got_status_b, status_b,
            "B's synthesized presence is not B's status — A's status leaked/overwrote it. \
             got: {got_status_b:?}, expected: {status_b:?}, A status: {status_a:?}"
        );

        // Same channel UUID in both communities. This makes `(channel_id, kind)`
        // identical on both sides; community_id is the only correct fan-out
        // partition key.
        let shared_uuid = uuid::Uuid::new_v4();
        let chan_a = create_open_channel(&http_a, &keys, shared_uuid).await;
        let chan_b = create_open_channel(&http_b, &keys, shared_uuid).await;
        assert_eq!(chan_a, chan_b, "channels must share UUID — test design");

        let sub_a = format!("typing-a-{suffix}");
        let sub_b = format!("typing-b-{suffix}");
        subscribe_typing(&mut client_a, &sub_a, &chan_a).await;
        subscribe_typing(&mut client_b, &sub_b, &chan_b).await;

        let content_a = format!("typing from A {suffix}");
        let content_b = format!("typing from B {suffix}");
        let _id_a = publish_typing(&mut client_a, &keys, &chan_a, &content_a).await;
        let _id_b = publish_typing(&mut client_b, &keys, &chan_b, &content_b).await;

        let live_a = drain_live_events(&mut client_a, &sub_a, Duration::from_millis(500)).await;
        let live_b = drain_live_events(&mut client_b, &sub_b, Duration::from_millis(500)).await;
        assert_one_content(&live_a, &content_a, &content_b, "A");
        assert_one_content(&live_b, &content_b, &content_a, "B");

        client_a.disconnect().await.expect("disconnect A");
        client_b.disconnect().await.expect("disconnect B");
    }
}

// ---------------------------------------------------------------------------
// Media / Blossom / S3 (Perci — buzz-media)
// ---------------------------------------------------------------------------
mod media_blossom {
    use super::*;

    /// Obligation: public blob `GET/HEAD /media/{sha256.ext}` stays
    /// unauthenticated (N=1 compat, shared CAS bytes). The community boundary is
    /// the metadata/descriptor/upload-auth/quota/audit layer: B's private upload
    /// metadata/errors must not be observable from A, even when the blob bytes
    /// are deduplicated and shared.
    #[tokio::test]
    #[ignore]
    async fn media_metadata_boundary_holds_while_blob_bytes_shared() {
        pending_lane(
            "buzz-media",
            "shared SHA bytes OK; A cannot read B's upload metadata/quota/audit; errors generic",
        );
    }
}

// ---------------------------------------------------------------------------
// Git hosting / NIP-34 / object storage (Perci — buzz-media/git)
// ---------------------------------------------------------------------------
mod git_hosting {
    use super::*;

    /// Obligation: pointer/name keys include community; same owner/repo in two
    /// communities are independent; a push in A does not advance B's pointer.
    #[tokio::test]
    #[ignore]
    async fn same_owner_repo_isolated_push_does_not_cross() {
        pending_lane(
            "buzz-media",
            "repos/{community}/{owner}/{repo}/pointer; push in A leaves B pointer unchanged",
        );
    }
}

// ---------------------------------------------------------------------------
// Mesh / agents / ACP / MCP / CLI (Eva — relay-wiring smoke)
// ---------------------------------------------------------------------------
mod mesh_agents_cli {
    use super::*;

    /// Obligation: one portable key may join multiple communities, but
    /// memberships, DMs, profiles, jobs, and presence do not bleed across them.
    #[tokio::test]
    #[ignore]
    async fn one_key_two_communities_no_bleed() {
        pending_lane(
            "relay-wiring",
            "same key joins A and B; CLI/ACP smoke shows distinct memberships/profiles",
        );
    }
}

// ---------------------------------------------------------------------------
// Audit log and observability (Dawn — buzz-audit)
// ---------------------------------------------------------------------------
mod audit_log {
    //! Obligation: audit reads verify exactly one community chain
    //! (`(community_id, seq)` / `(community_id, hash)`); error strings must not
    //! leak cross-community IDs, constraint names, or existence facts.
    //!
    //! **This row is doc-only — and that is the strongest statement in the file.**
    //!
    //! Every other row here proves a black-box property: the relay serves a wire
    //! response, and the test asserts that response denies a cross-community
    //! oracle. The audit log has no such response to assert against — it has **no
    //! client-reachable wire surface at all**. There is no `/audit` route in
    //! `crates/buzz-relay/src/router.rs` (the route list is `/`, `/info`,
    //! `/.well-known/nostr.json`, the health probes, `/events`, `/query`,
    //! `/count`, `/hooks`, the media and git sub-routers, and the audio WS — no
    //! audit endpoint). Audit is written as an ingest side-effect
    //! (`handlers/event.rs`, `dispatch_persistent_event`) and read only via
    //! `buzz_audit::AuditService::{verify_chain, get_entries}`, which are
    //! operator-internal (consumed by `buzz-admin`). `crates/buzz-audit/src/
    //! error.rs` states it directly: `AuditError` is "never relayed to a client
    //! on the wire," and "no variant embeds a `community_id`."
    //!
    //! So where other rows prove *the oracle is denied*, audit proves *the
    //! oracle's surface does not exist* — a strictly stronger isolation claim,
    //! and the honest way to state it is to cite the facts, not to invent a wire
    //! observation that the architecture does not offer. Reaching behind the
    //! wire into Postgres here would also break this file's black-box contract
    //! (its deps are `buzz-ws-client`/`reqwest`/`tokio-tungstenite`/`s3` — no
    //! `sqlx`, no `buzz-audit`), and a DB-direct read can never catch a
    //! wire-layer bug because it never traverses the wire read path.
    //!
    //! The two halves of the obligation are proven in their proper homes, where
    //! direct Postgres access is in-convention:
    //!
    //!   1. **One chain per community** —
    //!      `buzz_audit::service::tests::chains_are_independent_per_community`
    //!      (direct `AuditService::log`) proves interleaved A/B writes keep
    //!      independent `(community_id, seq)` chains, each starting at seq 1 with
    //!      its own `prev_hash`, and that `verify_chain`/`get_entries` scoped to
    //!      one community never traverse another. The *integrated* path — that a
    //!      community resolved from the request's `TenantContext` at relay ingest
    //!      lands in the correct chain and stays isolated — is proven by
    //!      `buzz_relay::handlers::event::tests::
    //!      audit_chain_is_isolated_per_tenant_through_relay_ingest`, driving
    //!      `dispatch_persistent_event` under two tenants against a shared
    //!      Postgres (no WS-AUTH dependency).
    //!   2. **Errors don't leak** —
    //!      `buzz_audit::error::tests::audit_error_text_carries_no_community_id_or_constraint`
    //!      asserts no `AuditError` variant's rendered text embeds a
    //!      `community_id`, constraint name, or cross-community object id.
    //!
    //! Substrate on PR head: `crates/buzz-audit/src/entry.rs` keys `AuditEntry`
    //! `(community_id, seq)` with per-community `prev_hash`; `NewAuditEntry.
    //! community_id` is typed `CommunityId` (server-resolved, never client
    //! input).
}

// ---------------------------------------------------------------------------
// Migration gate 5: N=1 conformance (Eva — orchestration)
// ---------------------------------------------------------------------------
mod n1_parity {
    //! N=1 parity is asserted by the *existing* e2e suites run against the new
    //! relay with one configured host → one default community. The obligation:
    //! no existing client needs new tags, paths, event fields, CLI flags, or
    //! protocol messages. This module documents the gate; the assertion lives in
    //! the unchanged `e2e_*` suites passing green under `RELAY_URL=<new relay>`.
    //!
    //! Parity runner (driven from the integration job, not a unit test):
    //!   1. Boot new relay, one host → default community, backfill existing data.
    //!   2. Run the full `e2e_*` ignored suite with `RELAY_URL` at the new relay.
    //!   3. Every suite green, unchanged === N=1 parity proven.
}
