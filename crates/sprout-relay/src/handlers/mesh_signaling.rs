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
use sprout_core::kind::KIND_MESH_CALL_ME_NOW;

use crate::api::relay_members::{check_relay_membership, MembershipDecision};
use crate::state::AppState;

/// Parsed `KIND_MESH_CONNECT_REQUEST` (24621) content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectRequest {
    /// Requester's own iroh EndpointAddr (base64 invite token) — sent to the peer.
    pub self_endpoint_addr: String,
    /// Peer's iroh EndpointAddr (base64 invite token), read by the requester from
    /// the peer's kind:30621 serve target — sent back to the requester.
    pub peer_endpoint_addr: String,
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
        attempt_id,
    })
}

/// Extract the single `#p` target pubkey (hex) from the request event's tags.
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
/// telling it to dial `peer_endpoint_addr`. Pure — no I/O.
pub fn call_me_now_content(peer_endpoint_addr: &str, attempt_id: &str, expires_at: u64) -> String {
    serde_json::json!({
        "v": 1,
        "type": "sprout-iroh-call-me-now",
        "peer_endpoint_addr": peer_endpoint_addr,
        "attempt_id": attempt_id,
        "expires_at": expires_at,
    })
    .to_string()
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

    // Membership gate: the requester is already a member (authenticated on the
    // WS under SPROUT_REQUIRE_RELAY_MEMBERSHIP). We must independently confirm
    // the TARGET is a member too — both ends gated by relay access, nothing else.
    let target_bytes =
        hex::decode(&target_hex).map_err(|_| "invalid: malformed #p target pubkey".to_string())?;
    match check_relay_membership(state, &target_bytes, None).await {
        Ok(MembershipDecision::OpenRelay) | Ok(MembershipDecision::Member) => {}
        // ViaOwner (NIP-OA delegated) is intentionally NOT admitted to mesh in v1,
        // matching the tighter mesh trust boundary.
        Ok(MembershipDecision::ViaOwner(_)) | Ok(MembershipDecision::Denied) => {
            return Err("restricted: target is not a relay member".to_string());
        }
        Err(e) => {
            // Fail closed: an internal membership-check blip must not admit.
            tracing::warn!("mesh connect: target membership check failed: {e}");
            return Err("error: membership check unavailable".to_string());
        }
    }

    let expires_at = (chrono::Utc::now().timestamp().max(0) as u64) + CALL_ME_NOW_TTL_SECS;

    // Pair: tell the requester to dial the peer's addr, and the peer to dial the
    // requester's addr. Each is a relay-signed ephemeral #p-addressed event.
    let to_requester = build_call_me_now(
        state,
        requester_pubkey_hex,
        &req.peer_endpoint_addr,
        &req.attempt_id,
        expires_at,
    )?;
    let to_target = build_call_me_now(
        state,
        &target_hex,
        &req.self_endpoint_addr,
        &req.attempt_id,
        expires_at,
    )?;

    publish_channelless_ephemeral(state, &to_requester).await;
    publish_channelless_ephemeral(state, &to_target).await;
    Ok(())
}

/// Mint one relay-signed call-me-now (24622) addressed to `recipient_hex`.
fn build_call_me_now(
    state: &Arc<AppState>,
    recipient_hex: &str,
    peer_endpoint_addr: &str,
    attempt_id: &str,
    expires_at: u64,
) -> Result<nostr::Event, String> {
    let content = call_me_now_content(peer_endpoint_addr, attempt_id, expires_at);
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
        let s = call_me_now_content("ENDPOINT", "att-1", 1234);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["type"], "sprout-iroh-call-me-now");
        assert_eq!(v["peer_endpoint_addr"], "ENDPOINT");
        assert_eq!(v["attempt_id"], "att-1");
        assert_eq!(v["expires_at"], 1234);
        assert_eq!(v["v"], 1);
    }
}
