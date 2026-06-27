//! Multi-tenant conformance harness.
//!
//! This file mirrors the obligation table in `docs/multi-tenant-conformance.md`
//! **one row per module**. It is the executable form of the conformance
//! contract: the current single-community relay is the wire-parity *oracle*, and
//! these tests prove two things the rewrite must never break:
//!
//!   1. **N=1 parity** — with one configured host → one default community, every
//!      existing client observes byte-identical behavior. This is asserted by the
//!      *existing* e2e suites (`e2e_relay`, `e2e_rest_api`, `e2e_media`, …) run
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
    /// Because `RelayInfo::build` is genuinely static-input today, host A's and
    /// host B's NIP-11 bodies are byte-identical, and that identity *is* the
    /// proof: no field varies by community, so an unauthenticated reader cannot
    /// use the document to probe whether (or how) community B is configured.
    /// The moment a per-community value leaks into the doc, the two bodies
    /// diverge and this assertion fails — that is the mutate-bite this row
    /// guards (seed a community-distinguishing field into the served doc → the
    /// A≡B assertion goes red).
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
        let json_a: serde_json::Value =
            serde_json::from_str(&body_a).expect("host A NIP-11 is valid JSON");
        let json_b: serde_json::Value =
            serde_json::from_str(&body_b).expect("host B NIP-11 is valid JSON");
        assert!(
            json_a.get("supported_nips").is_some(),
            "host A NIP-11 must be a relay-info document (has supported_nips)"
        );
        assert!(
            json_b.get("supported_nips").is_some(),
            "host B NIP-11 must be a relay-info document (has supported_nips)"
        );

        // The enumeration-oracle obligation: no field of the served document
        // varies by community. Identical bodies are the proof that the doc
        // cannot be used to distinguish or probe another tenant.
        assert_eq!(
            json_a, json_b,
            "NIP-11 from host A and host B must be identical: any community-\
             distinguishing field would make the unauthenticated relay-info \
             document an enumeration oracle for other tenants"
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
        assert_eq!(
            json_a, json_unknown,
            "NIP-11 served to an unmapped host must be byte-identical to a mapped \
             host's document: the relay-info doc carries no host-derived field, \
             so it cannot reveal whether a given host is configured"
        );
    }
}

// ---------------------------------------------------------------------------
// API tokens and NIP-98 replay (Sami — buzz-auth)
// ---------------------------------------------------------------------------
mod api_tokens_nip98_replay {
    use super::*;

    /// Obligation: token hash uniqueness/lookup is `(community_id, token_hash)`;
    /// a token minted in A does not authorize the same hash in B.
    #[tokio::test]
    #[ignore]
    async fn token_minted_in_a_does_not_authorize_in_b() {
        pending_lane(
            "buzz-auth",
            "identical token_hash in A and B → A's token rejected against B",
        );
    }

    /// Obligation: NIP-98 replay seen-set is shared (any-pod) AND community
    /// scoped: a nonce spent in A is still spendable in B, but a replay within A
    /// is rejected from any pod.
    #[tokio::test]
    #[ignore]
    async fn nip98_replay_seenset_is_shared_and_community_scoped() {
        pending_lane(
            "buzz-auth",
            "replay key (community_id, event_id) in shared store; u-host must match req.community",
        );
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

    /// Obligation: same pubkey can hold a different profile per community; kind:0
    /// replacement is scoped by `(community_id, pubkey)`.
    #[tokio::test]
    #[ignore]
    async fn same_pubkey_distinct_profiles_in_two_communities() {
        pending_lane(
            "buzz-auth",
            "kind:0 replace scoped by (community_id, pubkey); no cross-community inheritance",
        );
    }

    /// Obligation: the same NIP-05 local part can exist on two hosts; lookup only
    /// resolves handles for the requested host/community.
    #[tokio::test]
    #[ignore]
    async fn same_nip05_local_part_on_two_hosts_is_independent() {
        pending_lane(
            "buzz-auth",
            "NIP-05 unique (community_id, lower(handle)); host A lookup never resolves B's",
        );
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
// Channels and channel membership (Mari — buzz-db)
// ---------------------------------------------------------------------------
mod channels_membership {
    use super::*;

    /// Obligation: the same channel UUID legitimately co-exists in two
    /// communities (DB PK `(community_id, id)`); an `h` tag resolving to a
    /// channel in another community is rejected generically.
    #[tokio::test]
    #[ignore]
    async fn same_channel_uuid_in_two_communities_is_isolated() {
        pending_lane(
            "buzz-db",
            "channel UUID U exists in A and B; member/post in A never touches B's U",
        );
    }
}

// ---------------------------------------------------------------------------
// Workflows / runs / approvals / webhooks / schedules (Mari+Max)
// ---------------------------------------------------------------------------
mod workflows {
    use super::*;

    /// Obligation: identical workflow UUID / approval-token hash in two
    /// communities are independent; trigger evaluation only sees same-community
    /// events; schedule execution is isolated (at-most-once per community).
    #[tokio::test]
    #[ignore]
    async fn identical_workflow_and_approval_token_are_isolated() {
        pending_lane(
            "buzz-db",
            "same workflow UUID + approval hash in A and B act only within their community",
        );
    }
}

// ---------------------------------------------------------------------------
// Search / FTS (Quinn — buzz-search)
// ---------------------------------------------------------------------------
mod search_fts {
    use super::*;

    /// Obligation: every search `filter` includes `community_id`; same
    /// id/content in A and B return only same-community hits; deleting in A does
    /// not delete the B document. Postgres FTS (search_tsv/GIN), not Typesense.
    #[tokio::test]
    #[ignore]
    async fn search_results_and_deletes_are_community_scoped() {
        pending_lane(
            "buzz-search",
            "FTS query scoped by community_id; delete in A leaves B hit intact",
        );
    }
}

// ---------------------------------------------------------------------------
// Redis pub/sub, presence, typing, cache invalidation (Max — buzz-pubsub)
// ---------------------------------------------------------------------------
mod pubsub_presence_typing {
    use super::*;

    /// Obligation: keys are `buzz:{community}:…`; cross-node fan-out never
    /// delivers an A event to a B subscription, even for the same channel UUID;
    /// the same pubkey can be online in A and away in B independently.
    #[tokio::test]
    #[ignore]
    async fn fanout_and_presence_do_not_cross_communities() {
        pending_lane(
            "buzz-pubsub",
            "event on A's channel UUID never reaches B's subscription on the same UUID",
        );
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
