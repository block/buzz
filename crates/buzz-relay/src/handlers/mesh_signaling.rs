//! Mesh hole-punch signaling: the relay's only role in the v1 direct-iroh mesh.
//!
//! v1 mesh is "Sprout-coordinated direct iroh" — no server-side iroh relay/proxy.
//! The relay's entire job for connectivity is: when a member asks to dial a peer
//! it discovered via kind:30621, validate that BOTH ends are relay members, then
//! emit a *paired* live "call-me-now" so both desktops hole-punch at the same
//! moment. The relay never sees or stores the bulk iroh traffic — only this tiny
//! control-plane exchange.
//!
//! Flow:
//!   desktop A (member) reads 30621 → already holds B's `EndpointAddr` →
//!   publishes KIND_MESH_CONNECT_REQUEST (24621) `#p=B` with both endpoint addrs →
//!   relay validates B is a member → mints two relay-signed KIND_MESH_CALL_ME_NOW
//!   (24622): one `#p=A` carrying B's addr, one `#p=B` carrying A's addr →
//!   both fan out over the existing channel-less ephemeral path (local + Redis) →
//!   both desktops dial simultaneously → direct QUIC.
//!
//! The relay is ENDPOINT-STATELESS here: the requester supplies both dial hints
//! (it read them from the relay-signed 30621), so the relay only validates
//! membership and pairs. Endpoint addrs are dial hints, never auth — membership
//! is the gate.

use std::sync::Arc;

use nostr::{EventBuilder, Kind, Tag};
use sprout_core::event::StoredEvent;
use sprout_core::kind::{
    event_kind_u32, KIND_MESH_CALL_ME_NOW, KIND_MESH_CONNECT_REQUEST, KIND_MESH_STATUS_REPORT,
};

use crate::api::relay_members::{check_relay_membership, MembershipDecision};
use crate::state::AppState;

/// Check + bump the per-requester 24621 rate limit (20/sec window).
///
/// Each accepted connect request makes the relay sign + fan TWO 24622s, so we
/// bound the amplification. 20/sec is far above any real interactive use; a
/// buggy desktop loop can't storm the relay. Shared by the WS door
/// (`handlers::event`) and the HTTP door (`handle_mesh_event_http`) — one
/// limiter, two transports.
pub(crate) fn connect_request_rate_limited(state: &AppState, pubkey: &nostr::PublicKey) -> bool {
    let key: [u8; 32] = pubkey.to_bytes();
    let now = std::time::Instant::now();
    let mut entry = state
        .mesh_connect_rate_limiter
        .entry(key)
        .or_insert((0, now));
    let (count, window_start) = entry.value_mut();
    if now.duration_since(*window_start).as_secs() >= 1 {
        *count = 1;
        *window_start = now;
        false
    } else {
        *count += 1;
        *count > 20
    }
}

/// HTTP-door entry point for the two desktop-published mesh signaling kinds
/// (24620 status report, 24621 connect request).
///
/// These kinds are ephemeral, so `ingest_event`'s per-kind allowlist
/// (deliberately) rejects them — historically they arrived only over the WS
/// path in `handlers::event`. Since the desktop's Rust coordinator publishes
/// them via `POST /events` (NIP-98), the bridge routes them here instead.
/// Mirrors the WS door's checks in the same order: signature, pubkey-match
/// (strict — mesh kinds are never proxy-submittable), rate limit, then the
/// shared handlers. Membership is enforced both at the bridge and inside the
/// handlers (fail-closed `require_mesh_member`).
///
/// Returns the same message the WS door would put in its OK frame; the bridge
/// maps `Err` to HTTP 400.
pub async fn handle_mesh_event_http(
    state: &Arc<AppState>,
    auth_pubkey: &nostr::PublicKey,
    event: &nostr::Event,
) -> Result<(), String> {
    let event_clone = event.clone();
    tokio::task::spawn_blocking(move || sprout_core::verification::verify_event(&event_clone))
        .await
        .map_err(|_| "error: internal verification error".to_string())?
        .map_err(|e| format!("invalid: {e}"))?;

    if event.pubkey != *auth_pubkey {
        return Err("invalid: event pubkey does not match authenticated identity".to_string());
    }

    let pubkey_hex = auth_pubkey.to_hex();
    match event_kind_u32(event) {
        k if k == KIND_MESH_STATUS_REPORT => handle_status_report(state, &pubkey_hex, event).await,
        k if k == KIND_MESH_CONNECT_REQUEST => {
            if connect_request_rate_limited(state, auth_pubkey) {
                return Err("rate-limited: mesh connect request rate exceeded (20/sec)".to_string());
            }
            handle_connect_request(state, &pubkey_hex, event).await
        }
        k => Err(format!("invalid: kind {k} is not a mesh signaling kind")),
    }
}

/// Parsed `KIND_MESH_CONNECT_REQUEST` (24621) content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectRequest {
    /// Requester's own iroh EndpointAddr (base64 invite token) — sent to the peer.
    pub self_endpoint_addr: String,
    /// Peer's iroh EndpointAddr (base64 invite token), read by the requester from
    /// the peer's kind:30621 serve target — sent back to the requester.
    pub peer_endpoint_addr: String,
    /// Requester's own iroh endpoint id (optional) — correlation/instrumentation
    /// only, never trusted for auth. Copied into the peer's call-me-now.
    pub self_endpoint_id: Option<String>,
    /// Peer's iroh endpoint id (optional) — correlation/instrumentation only.
    /// Copied into the requester's call-me-now so the desktop can target the
    /// exact peer endpoint it picked from 30621 (multi-endpoint disambiguation).
    pub peer_endpoint_id: Option<String>,
    /// Correlates the two halves of one punch attempt.
    pub attempt_id: String,
}

/// Outcome of validating + parsing a connect request, before any relay state is touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// Content was not valid JSON in the expected shape.
    Malformed(String),
    /// No `#p` tag naming the target peer.
    MissingTarget,
}

/// Parse the JSON content of a 24621 connect request. Pure — no I/O.
pub fn parse_connect_request(content: &str) -> Result<ConnectRequest, RequestError> {
    let v: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| RequestError::Malformed(format!("not JSON: {e}")))?;
    let get = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let self_endpoint_addr = get("self_endpoint_addr")
        .ok_or_else(|| RequestError::Malformed("self_endpoint_addr".into()))?;
    let peer_endpoint_addr = get("peer_endpoint_addr")
        .ok_or_else(|| RequestError::Malformed("peer_endpoint_addr".into()))?;
    let attempt_id =
        get("attempt_id").ok_or_else(|| RequestError::Malformed("attempt_id".into()))?;
    Ok(ConnectRequest {
        self_endpoint_addr,
        peer_endpoint_addr,
        self_endpoint_id: get("self_endpoint_id"),
        peer_endpoint_id: get("peer_endpoint_id"),
        attempt_id,
    })
}

/// Extract the single `#p` target pubkey (hex) from the request event's tags.
/// If multiple `#p` tags are present, the first wins; multi-target mesh-connect
/// is not supported in v1.
pub fn extract_target_pubkey(event: &nostr::Event) -> Option<String> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.len() >= 2 && s[0] == "p" {
            Some(s[1].clone())
        } else {
            None
        }
    })
}

/// Build the JSON content for one call-me-now (24622) directed at `recipient`,
/// telling it to dial `peer_endpoint_addr` (optionally `peer_endpoint_id` for
/// multi-endpoint disambiguation). Pure — no I/O.
pub fn call_me_now_content(
    peer_endpoint_addr: &str,
    peer_endpoint_id: Option<&str>,
    attempt_id: &str,
    expires_at: u64,
) -> String {
    let mut obj = serde_json::json!({
        "v": 1,
        "type": "sprout-iroh-call-me-now",
        "peer_endpoint_addr": peer_endpoint_addr,
        "attempt_id": attempt_id,
        "expires_at": expires_at,
    });
    if let Some(eid) = peer_endpoint_id {
        obj["peer_endpoint_id"] = serde_json::Value::String(eid.to_string());
    }
    obj.to_string()
}

/// Pure: does a membership decision admit a peer into the v1 mesh? Direct relay
/// members (or open relays) only — `ViaOwner` (NIP-OA delegated) is intentionally
/// NOT admitted in v1, keeping the mesh trust boundary tighter and legible. This
/// is applied identically to BOTH the requester and the target, so the two ends
/// are symmetric. Isolated as a pure fn so the trust gate is unit-testable
/// without an `AppState`.
pub fn membership_admits_mesh(decision: &MembershipDecision) -> bool {
    matches!(
        decision,
        MembershipDecision::OpenRelay | MembershipDecision::Member
    )
}

/// Seconds a call-me-now is valid; iroh's punch loop runs ~60s, so this bounds
/// how stale a signal a desktop should act on.
pub const CALL_ME_NOW_TTL_SECS: u64 = 60;

/// Handle a verified KIND_MESH_CONNECT_REQUEST (24621) from an authenticated
/// relay member. Validates the target is also a member, then emits the paired
/// call-me-now to both ends. Returns Ok(()) on success or an Err(reason) string
/// suitable for an OK(false) reply (reason is for the requester, not secret).
pub async fn handle_connect_request(
    state: &Arc<AppState>,
    requester_pubkey_hex: &str,
    event: &nostr::Event,
) -> Result<(), String> {
    let req = parse_connect_request(&event.content)
        .map_err(|e| format!("invalid: malformed mesh connect request ({e:?})"))?;

    let target_hex = extract_target_pubkey(event)
        .ok_or_else(|| "invalid: mesh connect request missing #p target".to_string())?;

    if target_hex == requester_pubkey_hex {
        return Err("invalid: cannot mesh-connect to self".to_string());
    }

    // Membership gate, applied SYMMETRICALLY to both ends — direct relay members
    // only, gated purely by relay access. The requester reached this handler via
    // a NIP-42-authed WS, but that auth can be ViaOwner (NIP-OA delegated) when
    // SPROUT_ALLOW_NIP_OA_AUTH is on; v1 mesh excludes delegated identities, so we
    // re-check the requester here with no auth tag (which makes ViaOwner
    // unreachable — only Member/OpenRelay/Denied) to match the target check.
    require_mesh_member(state, requester_pubkey_hex)
        .await
        .map_err(|_| "restricted: delegated identities cannot initiate mesh in v1".to_string())?;

    require_mesh_member(state, &target_hex)
        .await
        .map_err(|_| "restricted: target is not a relay member".to_string())?;

    let expires_at = (chrono::Utc::now().timestamp().max(0) as u64) + CALL_ME_NOW_TTL_SECS;

    // Pair: tell the requester to dial the peer's addr, and the peer to dial the
    // requester's addr. Each is a relay-signed ephemeral #p-addressed event.
    // endpoint_id (if supplied) is copied through for desktop multi-endpoint
    // disambiguation — it is correlation metadata, never trusted for auth.
    let to_requester = build_call_me_now(
        state,
        requester_pubkey_hex,
        &req.peer_endpoint_addr,
        req.peer_endpoint_id.as_deref(),
        &req.attempt_id,
        expires_at,
    )?;
    let to_target = build_call_me_now(
        state,
        &target_hex,
        &req.self_endpoint_addr,
        req.self_endpoint_id.as_deref(),
        &req.attempt_id,
        expires_at,
    )?;

    publish_channelless_ephemeral(state, &to_requester).await;
    publish_channelless_ephemeral(state, &to_target).await;
    Ok(())
}

/// Async: confirm `pubkey_hex` is a direct relay member admissible to the mesh.
/// `None` auth_tag → ViaOwner is unreachable, so only Member/OpenRelay admit;
/// everything else (Denied, ViaOwner-if-it-somehow-appeared, or a check error)
/// FAILS CLOSED. Used symmetrically for requester and target.
async fn require_mesh_member(state: &Arc<AppState>, pubkey_hex: &str) -> Result<(), ()> {
    let bytes = hex::decode(pubkey_hex).map_err(|_| ())?;
    match check_relay_membership(state, &bytes, None).await {
        Ok(d) if membership_admits_mesh(&d) => Ok(()),
        Ok(_) => Err(()),
        Err(e) => {
            tracing::warn!("mesh connect: membership check failed (fail-closed): {e}");
            Err(())
        }
    }
}

/// Mint one relay-signed call-me-now (24622) addressed to `recipient_hex`.
fn build_call_me_now(
    state: &Arc<AppState>,
    recipient_hex: &str,
    peer_endpoint_addr: &str,
    peer_endpoint_id: Option<&str>,
    attempt_id: &str,
    expires_at: u64,
) -> Result<nostr::Event, String> {
    let content = call_me_now_content(peer_endpoint_addr, peer_endpoint_id, attempt_id, expires_at);
    let p_tag = Tag::parse(["p", recipient_hex])
        .map_err(|e| format!("error: failed to build p tag: {e}"))?;
    EventBuilder::new(Kind::Custom(KIND_MESH_CALL_ME_NOW as u16), content)
        .tags([p_tag])
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| format!("error: failed to sign call-me-now: {e}"))
}

/// Publish a channel-less ephemeral event over the same path NIP-AB pairing uses:
/// Redis fan-out (nil-UUID global routing key) for cross-pod, plus direct local
/// WS fan-out. The recipient's desktop receives it via a REQ on `#p=self kind:24622`.
///
/// Observability note: call-me-now is delivered via `#p` subscription matching,
/// so it is NOT private to the recipient — any member can REQ `kind:24622
/// #p=<other_member>` and observe who that member is mesh-connecting to (and the
/// endpoint addrs). This matches presence/typing being broadly observable and is
/// intentional for v1. A conn-scoped private delivery channel would be needed to
/// hide it.
async fn publish_channelless_ephemeral(state: &Arc<AppState>, event: &nostr::Event) {
    state.mark_local_event(&event.id);
    if let Err(e) = state.pubsub.publish_event(uuid::Uuid::nil(), event).await {
        state.local_event_ids.invalidate(&event.id.to_bytes());
        tracing::warn!(event_id = %event.id, "mesh call-me-now global publish failed: {e}");
    }
    let stored = StoredEvent::new(event.clone(), None);
    let matches = state.sub_registry.fan_out(&stored);
    metrics::histogram!("sprout_fanout_recipients").record(matches.len() as f64);
    if let Ok(event_json) = serde_json::to_string(event) {
        for (target_conn_id, sub_id) in &matches {
            let msg = format!(r#"["EVENT","{sub_id}",{event_json}]"#);
            let _ = state.conn_manager.send_to(*target_conn_id, msg);
        }
    }
}

/// Handle a verified KIND_MESH_STATUS_REPORT (24620) from an authenticated relay
/// member. The member reports its current mesh `/api/status` JSON; the relay
/// sanitizes it and republishes a relay-signed kind:30621 discovery note keyed
/// to the reporter (so members' notes never clobber each other). The report
/// itself is ephemeral — only the relay's projection is durable. Membership is
/// already enforced (the reporter is authenticated on a member-gated WS).
pub async fn handle_status_report(
    state: &Arc<AppState>,
    reporter_pubkey_hex: &str,
    event: &nostr::Event,
) -> Result<(), String> {
    // Same membership symmetry as handle_connect_request: a NIP-OA delegated
    // (ViaOwner) identity is authed on the WS but is NOT a v1 mesh participant.
    // If we let it report, the relay would advertise a serve_target under that
    // pubkey that the connect path (which denies ViaOwner) then refuses — broken
    // discovery. So gate the reporter the same way: direct members only, fail
    // closed. Keeps all three desktop-facing mesh kinds consistent on delegation.
    require_mesh_member(state, reporter_pubkey_hex)
        .await
        .map_err(|_| {
            "restricted: delegated identities cannot report mesh status in v1".to_string()
        })?;

    let payload: serde_json::Value = serde_json::from_str(&event.content)
        .map_err(|e| format!("invalid: mesh status report content is not JSON ({e})"))?;
    crate::mesh_status_publisher::publish_mesh_status_from_payload(
        state,
        reporter_pubkey_hex,
        &payload,
    )
    .await
    .map_err(|e| {
        tracing::warn!("mesh status report publish failed: {e}");
        "error: failed to publish mesh status".to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_request() {
        let c = r#"{"self_endpoint_addr":"AAA","peer_endpoint_addr":"BBB","attempt_id":"x1"}"#;
        let r = parse_connect_request(c).unwrap();
        assert_eq!(r.self_endpoint_addr, "AAA");
        assert_eq!(r.peer_endpoint_addr, "BBB");
        assert_eq!(r.attempt_id, "x1");
    }

    #[test]
    fn parse_rejects_missing_field() {
        let c = r#"{"self_endpoint_addr":"AAA","attempt_id":"x1"}"#;
        assert!(matches!(
            parse_connect_request(c),
            Err(RequestError::Malformed(_))
        ));
    }

    #[test]
    fn parse_rejects_empty_field() {
        let c = r#"{"self_endpoint_addr":"","peer_endpoint_addr":"BBB","attempt_id":"x1"}"#;
        assert!(matches!(
            parse_connect_request(c),
            Err(RequestError::Malformed(_))
        ));
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(matches!(
            parse_connect_request("not json"),
            Err(RequestError::Malformed(_))
        ));
    }

    #[test]
    fn call_me_now_content_shape() {
        let s = call_me_now_content("ENDPOINT", None, "att-1", 1234);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["type"], "sprout-iroh-call-me-now");
        assert_eq!(v["peer_endpoint_addr"], "ENDPOINT");
        assert_eq!(v["attempt_id"], "att-1");
        assert_eq!(v["expires_at"], 1234);
        assert_eq!(v["v"], 1);
        // No endpoint id supplied → field omitted entirely.
        assert!(v.get("peer_endpoint_id").is_none());
    }

    #[test]
    fn call_me_now_content_includes_endpoint_id_when_present() {
        let s = call_me_now_content("ENDPOINT", Some("EID-7"), "att-1", 1234);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["peer_endpoint_id"], "EID-7");
    }

    #[test]
    fn parse_round_trips_optional_endpoint_ids() {
        let c = r#"{"self_endpoint_addr":"A","peer_endpoint_addr":"B","self_endpoint_id":"SI","peer_endpoint_id":"PI","attempt_id":"x"}"#;
        let r = parse_connect_request(c).unwrap();
        assert_eq!(r.self_endpoint_id.as_deref(), Some("SI"));
        assert_eq!(r.peer_endpoint_id.as_deref(), Some("PI"));
        // endpoint ids are optional — absent is fine.
        let c2 = r#"{"self_endpoint_addr":"A","peer_endpoint_addr":"B","attempt_id":"x"}"#;
        let r2 = parse_connect_request(c2).unwrap();
        assert_eq!(r2.self_endpoint_id, None);
        assert_eq!(r2.peer_endpoint_id, None);
    }

    // ── Trust gate: membership_admits_mesh ──────────────────────────────────
    // This is the single pure predicate behind the requester, target, AND
    // reporter gates. v1 admits only direct relay members (or open relays);
    // NIP-OA-delegated (ViaOwner) and Denied are excluded, symmetrically.

    #[test]
    fn member_and_open_relay_are_admitted() {
        assert!(membership_admits_mesh(&MembershipDecision::Member));
        assert!(membership_admits_mesh(&MembershipDecision::OpenRelay));
    }

    #[test]
    fn denied_is_not_admitted() {
        assert!(!membership_admits_mesh(&MembershipDecision::Denied));
    }

    #[test]
    fn via_owner_is_not_admitted_in_v1() {
        // Delegated identities are excluded from v1 mesh on every desktop-facing
        // path (requester / target / reporter all run through this predicate).
        let owner = nostr::Keys::generate().public_key();
        assert!(!membership_admits_mesh(&MembershipDecision::ViaOwner(
            owner
        )));
    }

    #[test]
    fn extract_target_takes_the_p_tag() {
        let keys = nostr::Keys::generate();
        let target = nostr::Keys::generate().public_key().to_hex();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(24621), "{}")
            .tags([nostr::Tag::parse(["p", &target]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(
            extract_target_pubkey(&event).as_deref(),
            Some(target.as_str())
        );
    }

    #[test]
    fn extract_target_none_without_p_tag() {
        let keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(24621), "{}")
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(extract_target_pubkey(&event), None);
    }

    fn p_tag() -> nostr::SingleLetterTag {
        nostr::SingleLetterTag::lowercase(nostr::Alphabet::P)
    }

    fn test_config() -> crate::config::Config {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config
    }

    async fn test_state() -> std::sync::Arc<AppState> {
        let config = test_config();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = sprout_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = std::sync::Arc::new(
            sprout_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = sprout_audit::AuditService::new(pool);
        let auth = sprout_auth::AuthService::new(config.auth.clone());
        let search = sprout_search::SearchService::new(sprout_search::SearchConfig {
            url: config.typesense_url.clone(),
            api_key: config.typesense_key.clone(),
            collection: "events".to_string(),
        });
        let workflow_engine = std::sync::Arc::new(sprout_workflow::WorkflowEngine::new(
            db.clone(),
            sprout_workflow::WorkflowConfig::default(),
        ));
        let media_storage = sprout_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        std::sync::Arc::new(state)
    }

    fn register_call_me_now_sub(
        state: &AppState,
        recipient_hex: &str,
        sub_id: &str,
    ) -> (
        uuid::Uuid,
        tokio::sync::mpsc::Receiver<axum::extract::ws::Message>,
    ) {
        let conn_id = uuid::Uuid::new_v4();
        let (tx, rx) = tokio::sync::mpsc::channel(10);
        state.conn_manager.register(
            conn_id,
            tx,
            tokio_util::sync::CancellationToken::new(),
            std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0)),
            std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        );
        state.sub_registry.register(
            conn_id,
            sub_id.to_string(),
            vec![nostr::Filter::new()
                .kind(nostr::Kind::Custom(KIND_MESH_CALL_ME_NOW as u16))
                .custom_tags(p_tag(), [recipient_hex])],
            None,
        );
        (conn_id, rx)
    }

    fn connect_request_event(target_hex: &str) -> nostr::Event {
        let content = serde_json::json!({
            "self_endpoint_addr": "SELF_ADDR",
            "peer_endpoint_addr": "PEER_ADDR",
            "self_endpoint_id": "SELF_ID",
            "peer_endpoint_id": "PEER_ID",
            "attempt_id": "attempt-1"
        })
        .to_string();
        nostr::EventBuilder::new(nostr::Kind::Custom(24621), content)
            .tags([nostr::Tag::parse(["p", target_hex]).unwrap()])
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap()
    }

    fn event_from_ws_message(msg: axum::extract::ws::Message) -> nostr::Event {
        let axum::extract::ws::Message::Text(text) = msg else {
            panic!("expected text ws message");
        };
        let v: serde_json::Value = serde_json::from_str(&text).expect("EVENT frame JSON");
        assert_eq!(v[0], "EVENT");
        serde_json::from_value(v[2].clone()).expect("nostr event")
    }

    #[tokio::test]
    async fn accepted_connect_request_emits_two_relay_signed_call_me_now_events() {
        let state = test_state().await;
        let requester_hex = nostr::Keys::generate().public_key().to_hex();
        let target_hex = nostr::Keys::generate().public_key().to_hex();
        let (_requester_conn, mut requester_rx) =
            register_call_me_now_sub(&state, &requester_hex, "mesh_requester");
        let (_target_conn, mut target_rx) =
            register_call_me_now_sub(&state, &target_hex, "mesh_target");

        handle_connect_request(&state, &requester_hex, &connect_request_event(&target_hex))
            .await
            .expect("open relay admits both peers and emits pair");

        let requester_event = event_from_ws_message(
            requester_rx
                .try_recv()
                .expect("requester receives call-me-now"),
        );
        let target_event =
            event_from_ws_message(target_rx.try_recv().expect("target receives call-me-now"));
        assert!(
            requester_rx.try_recv().is_err(),
            "requester gets exactly one event"
        );
        assert!(
            target_rx.try_recv().is_err(),
            "target gets exactly one event"
        );

        for event in [&requester_event, &target_event] {
            assert_eq!(
                event.kind,
                nostr::Kind::Custom(KIND_MESH_CALL_ME_NOW as u16)
            );
            assert_eq!(event.pubkey, state.relay_keypair.public_key());
            event.verify().expect("relay-signed event verifies");
        }

        assert_eq!(
            extract_target_pubkey(&requester_event).as_deref(),
            Some(requester_hex.as_str())
        );
        assert_eq!(
            extract_target_pubkey(&target_event).as_deref(),
            Some(target_hex.as_str())
        );

        let requester_content: serde_json::Value =
            serde_json::from_str(&requester_event.content).expect("requester content JSON");
        assert_eq!(requester_content["type"], "sprout-iroh-call-me-now");
        assert_eq!(requester_content["peer_endpoint_addr"], "PEER_ADDR");
        assert_eq!(requester_content["peer_endpoint_id"], "PEER_ID");
        assert_eq!(requester_content["attempt_id"], "attempt-1");
        assert!(requester_content["expires_at"].as_u64().is_some());

        let target_content: serde_json::Value =
            serde_json::from_str(&target_event.content).expect("target content JSON");
        assert_eq!(target_content["type"], "sprout-iroh-call-me-now");
        assert_eq!(target_content["peer_endpoint_addr"], "SELF_ADDR");
        assert_eq!(target_content["peer_endpoint_id"], "SELF_ID");
        assert_eq!(target_content["attempt_id"], "attempt-1");
        assert_eq!(
            target_content["expires_at"],
            requester_content["expires_at"]
        );
    }

    #[tokio::test]
    async fn self_target_connect_request_emits_no_call_me_now_events() {
        let state = test_state().await;
        let requester_hex = nostr::Keys::generate().public_key().to_hex();
        let target_hex = nostr::Keys::generate().public_key().to_hex();
        let (_requester_conn, mut requester_rx) =
            register_call_me_now_sub(&state, &requester_hex, "mesh_requester");
        let (_target_conn, mut target_rx) =
            register_call_me_now_sub(&state, &target_hex, "mesh_target");

        let err = handle_connect_request(
            &state,
            &requester_hex,
            &connect_request_event(&requester_hex),
        )
        .await
        .expect_err("self-target is rejected before emitting");
        assert!(err.contains("self"), "unexpected error: {err}");
        assert!(
            requester_rx.try_recv().is_err(),
            "requester receives no event"
        );
        assert!(target_rx.try_recv().is_err(), "target receives no event");
    }

    // ── HTTP door (handle_mesh_event_http) ──────────────────────────────────
    // Regression coverage for the post-#879 transport: the desktop's Rust
    // coordinator publishes 24620/24621 via POST /events, which used to fall
    // into ingest_event's allowlist and 400 with "unknown event kind".

    fn signed_connect_request(keys: &nostr::Keys, target_hex: &str) -> nostr::Event {
        let content = serde_json::json!({
            "self_endpoint_addr": "SELF_ADDR",
            "peer_endpoint_addr": "PEER_ADDR",
            "attempt_id": "attempt-http-1"
        })
        .to_string();
        nostr::EventBuilder::new(nostr::Kind::Custom(24621), content)
            .tags([nostr::Tag::parse(["p", target_hex]).unwrap()])
            .sign_with_keys(keys)
            .unwrap()
    }

    #[tokio::test]
    async fn http_door_accepts_connect_request_and_emits_pair() {
        let state = test_state().await;
        let requester = nostr::Keys::generate();
        let requester_hex = requester.public_key().to_hex();
        let target_hex = nostr::Keys::generate().public_key().to_hex();
        let (_rc, mut requester_rx) =
            register_call_me_now_sub(&state, &requester_hex, "http_requester");
        let (_tc, mut target_rx) = register_call_me_now_sub(&state, &target_hex, "http_target");

        handle_mesh_event_http(
            &state,
            &requester.public_key(),
            &signed_connect_request(&requester, &target_hex),
        )
        .await
        .expect("HTTP door accepts a valid connect request");

        event_from_ws_message(requester_rx.try_recv().expect("requester call-me-now"));
        event_from_ws_message(target_rx.try_recv().expect("target call-me-now"));
    }

    #[tokio::test]
    async fn http_door_rejects_pubkey_mismatch() {
        let state = test_state().await;
        let signer = nostr::Keys::generate();
        let other = nostr::Keys::generate();
        let target_hex = nostr::Keys::generate().public_key().to_hex();

        let err = handle_mesh_event_http(
            &state,
            &other.public_key(),
            &signed_connect_request(&signer, &target_hex),
        )
        .await
        .expect_err("signer != authenticated identity must be rejected");
        assert!(err.contains("does not match"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn http_door_rejects_bad_signature() {
        let state = test_state().await;
        let keys = nostr::Keys::generate();
        let target_hex = nostr::Keys::generate().public_key().to_hex();
        let mut event = signed_connect_request(&keys, &target_hex);
        // Tamper after signing.
        event.content = "{}".to_string();

        let err = handle_mesh_event_http(&state, &keys.public_key(), &event)
            .await
            .expect_err("tampered event must be rejected");
        assert!(err.starts_with("invalid:"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn http_door_rejects_non_mesh_kind() {
        let state = test_state().await;
        let keys = nostr::Keys::generate();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(20001), "online")
            .sign_with_keys(&keys)
            .unwrap();

        let err = handle_mesh_event_http(&state, &keys.public_key(), &event)
            .await
            .expect_err("only 24620/24621 route through the mesh HTTP door");
        assert!(
            err.contains("not a mesh signaling kind"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn connect_request_rate_limiter_trips_on_21st_in_window() {
        let state = test_state().await;
        let pubkey = nostr::Keys::generate().public_key();
        for i in 0..20 {
            assert!(
                !connect_request_rate_limited(&state, &pubkey),
                "request {} should pass",
                i + 1
            );
        }
        assert!(
            connect_request_rate_limited(&state, &pubkey),
            "21st request in the same window must be limited"
        );
    }
}
