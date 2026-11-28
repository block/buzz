//! Internal policy endpoint — pre-receive hook callback.
//!
//! The pre-receive hook POSTs here with HMAC-signed payload containing
//! the pusher's pubkey, repo ID, and ref updates. This endpoint:
//!
//! 1. Validates HMAC signature + 30s TTL (fail-closed)
//! 2. Resolves kind:30617 → protection rules
//! 3. Denies any protection rule it does not understand
//! 4. Credits the pusher with its role in the bound channel, or for an
//!    owned agent the higher of that and its grant capped by its owner's
//!    role ([`super::authority::channel_role`]); nothing else grants
//!    authority. The land rechecks that grant and binds
//! 5. Calls `buzz_core::git_perms::evaluate_push()`
//! 6. Returns 200 (allow) or 403 (deny with reasons)
//!
//! # Bot Role Model
//!
//! Bots are intentionally added to channels by members/admins. For git they
//! are credited as members — protection rules still apply. Bot is a
//! designation (what it is), not a permission tier (what it can do). The
//! promotion lives in [`super::authority`]; the core `MemberRole::Bot`
//! hierarchy is unchanged.
//!
//! # Security invariants
//!
//! - Endpoint binds to 127.0.0.1 only (enforced at router level)
//! - HMAC binds callback to the specific push operation
//! - Fail-closed: any error → 403

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tracing::{error, warn};

use uuid::Uuid;

use buzz_core::git_perms::{
    evaluate_push, parse_protection_tags, Denial, RefUpdate, UpdateKind,
    GIT_NO_CHANNEL_BINDING_BODY,
};
use buzz_db::EventQuery;

use crate::state::AppState;

/// The hook wire format permits only the three delegation levels.
fn deserialize_grant_role<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<buzz_core::channel::MemberRole>, D::Error> {
    use buzz_core::channel::MemberRole;
    match Option::<String>::deserialize(deserializer)?.as_deref() {
        None => Ok(None),
        Some("guest") => Ok(Some(MemberRole::Guest)),
        Some("member") => Ok(Some(MemberRole::Member)),
        Some("admin") => Ok(Some(MemberRole::Admin)),
        _ => Err(serde::de::Error::custom("invalid grant role")),
    }
}

/// Maximum age of a hook callback (seconds). Push is synchronous so 30s is generous.
const MAX_CALLBACK_AGE_SECS: u64 = 30;

/// Request payload from the pre-receive hook.
#[derive(Debug, Clone, Deserialize)]
pub struct HookCallbackRequest {
    /// Repo identifier (d-tag from kind:30617).
    pub repo_id: String,
    /// Hex-encoded repo owner pubkey (from URL path, verified against kind:30617).
    pub repo_owner: String,
    /// Server-resolved community id from the git HTTP request that spawned the hook.
    /// Internal-only: set by relay env and HMAC-bound by the hook callback.
    pub community_id: String,
    /// Hex-encoded pusher pubkey.
    pub pusher_pubkey: String,
    /// Repo-scoped grant level, authenticated by the hook HMAC.
    /// The land revalidates liveness and the owner.
    #[serde(default, deserialize_with = "deserialize_grant_role")]
    pub grant: Option<buzz_core::channel::MemberRole>,
    /// Ref updates from git stdin (old_oid, new_oid, ref_name, is_ancestor).
    pub ref_updates: Vec<HookRefUpdate>,
    /// Unix timestamp when the hook was invoked.
    pub timestamp: u64,
    /// HMAC-SHA256 signature over the canonical payload.
    pub signature: String,
}

/// A single ref update as reported by the pre-receive hook.
#[derive(Debug, Clone, Deserialize)]
pub struct HookRefUpdate {
    /// Old object ID (40 hex chars, zero OID for creates).
    pub old_oid: String,
    /// New object ID (40 hex chars, zero OID for deletes).
    pub new_oid: String,
    /// Full ref name (e.g., "refs/heads/main").
    pub ref_name: String,
    /// Result of `git merge-base --is-ancestor old new`.
    /// For creates/deletes this is false (ignored by classifier).
    pub is_ancestor: bool,
}

/// Response to the hook — either allow or deny.
#[derive(Debug, Serialize)]
pub struct HookCallbackResponse {
    /// Whether the push is allowed.
    pub allowed: bool,
    /// Denial reasons (empty if allowed).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub denials: Vec<DenialResponse>,
}

/// A single denial reason in the hook response.
#[derive(Debug, Serialize)]
pub struct DenialResponse {
    /// The ref that was denied.
    pub ref_name: String,
    /// Human-readable reason for denial.
    pub reason: String,
}

impl From<Denial> for DenialResponse {
    fn from(d: Denial) -> Self {
        Self {
            ref_name: d.ref_name,
            reason: d.reason,
        }
    }
}

/// Compute the canonical HMAC payload.
///
/// Format (length-prefixed, `|`-separated, structurally unambiguous):
/// ```text
/// len(repo_id):repo_id | repo_owner(64) | community_id(36) | pusher(64) | grant role (or "none") | sorted_refs | timestamp
/// ```
/// where each ref is: `old_oid(40) + new_oid(40) + len(ref_name):ref_name + is_ancestor("1"/"0")`
///
/// Fixed-length fields (OIDs=40, pubkeys=64) need no length prefix.
/// Variable-length fields (repo_id, ref_name) are length-prefixed to prevent concatenation ambiguity.
fn compute_hmac(secret: &[u8], req: &HookCallbackRequest) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC can take key of any size");

    // Structurally unambiguous format: length-prefixed fields separated by |.
    // This prevents field confusion attacks (e.g., repo_id="a|b" being parsed differently).
    mac.update(req.repo_id.len().to_string().as_bytes());
    mac.update(b":");
    mac.update(req.repo_id.as_bytes());
    mac.update(b"|");
    mac.update(req.repo_owner.as_bytes()); // Fixed 64 chars, no ambiguity.
    mac.update(b"|");
    mac.update(req.community_id.as_bytes()); // Fixed UUID string from server-resolved tenant.
    mac.update(b"|");
    mac.update(req.pusher_pubkey.as_bytes()); // Fixed 64 chars, no ambiguity.
    mac.update(b"|");
    mac.update(req.grant.map_or("none", |r| r.as_str()).as_bytes());
    mac.update(b"|");
    // Deterministic ref update representation: sorted by ref_name.
    // Each ref is length-prefixed to prevent concatenation ambiguity.
    let mut refs_sorted: Vec<&HookRefUpdate> = req.ref_updates.iter().collect();
    refs_sorted.sort_by_key(|r| r.ref_name.clone());
    for r in &refs_sorted {
        mac.update(r.old_oid.as_bytes()); // Fixed 40 chars.
        mac.update(r.new_oid.as_bytes()); // Fixed 40 chars.
        mac.update(r.ref_name.len().to_string().as_bytes());
        mac.update(b":");
        mac.update(r.ref_name.as_bytes());
        mac.update(if r.is_ancestor { b"1" } else { b"0" });
    }
    mac.update(b"|");
    mac.update(req.timestamp.to_string().as_bytes());

    mac.finalize().into_bytes().to_vec()
}

/// Verify the HMAC signature on a hook callback.
fn verify_hmac(secret: &[u8], req: &HookCallbackRequest) -> bool {
    let expected = compute_hmac(secret, req);
    let provided = match hex::decode(&req.signature) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    // Constant-time comparison.
    use subtle::ConstantTimeEq;
    expected.ct_eq(&provided).into()
}

/// `POST /internal/git/policy` — pre-receive hook callback.
///
/// Fail-closed: ANY error returns 403. The hook script treats non-200 as deny.
pub async fn hook_policy_check(
    State(state): State<Arc<AppState>>,
    Json(req): Json<HookCallbackRequest>,
) -> Response {
    // 1. Validate input fields (cheap structural checks before expensive HMAC).
    // This prevents wasting CPU on malformed payloads.
    if req.repo_id.is_empty() || req.repo_id.len() > 64 {
        return (StatusCode::FORBIDDEN, "invalid repo_id").into_response();
    }
    if req.repo_owner.len() != 64
        || !req
            .repo_owner
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return (StatusCode::FORBIDDEN, "invalid repo_owner").into_response();
    }
    if req.pusher_pubkey.len() != 64
        || !req
            .pusher_pubkey
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return (StatusCode::FORBIDDEN, "invalid pusher_pubkey").into_response();
    }
    let community_uuid = match Uuid::parse_str(&req.community_id) {
        Ok(id) => id,
        Err(_) => return (StatusCode::FORBIDDEN, "invalid community_id").into_response(),
    };
    let community = buzz_core::CommunityId::from_uuid(community_uuid);
    if req.ref_updates.is_empty() || req.ref_updates.len() > 500 {
        return (StatusCode::FORBIDDEN, "invalid ref_updates count").into_response();
    }
    for r in &req.ref_updates {
        if r.old_oid.len() != 40 || !r.old_oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return (StatusCode::FORBIDDEN, "invalid old_oid").into_response();
        }
        if r.new_oid.len() != 40 || !r.new_oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return (StatusCode::FORBIDDEN, "invalid new_oid").into_response();
        }
        if r.ref_name.is_empty()
            || r.ref_name.len() > 256
            || !r.ref_name.starts_with("refs/")
            || r.ref_name.contains("..")
            || r.ref_name.bytes().any(|b| b <= 0x20 || b == 0x7f)
        {
            return (StatusCode::FORBIDDEN, "invalid ref_name").into_response();
        }
    }

    // 2. Verify HMAC signature (now that we know the payload is structurally valid).
    let secret = state.config.git_hook_hmac_secret.as_bytes();
    if !verify_hmac(secret, &req) {
        warn!(repo = %req.repo_id, "hook callback: HMAC verification failed");
        return (StatusCode::FORBIDDEN, "signature verification failed").into_response();
    }

    // 3. Validate timestamp (30s TTL, max 5s future tolerance).
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(req.timestamp) > MAX_CALLBACK_AGE_SECS {
        warn!(repo = %req.repo_id, age = now.saturating_sub(req.timestamp), "hook callback: expired");
        return (StatusCode::FORBIDDEN, "callback expired").into_response();
    }
    if req.timestamp.saturating_sub(now) > 5 {
        warn!(repo = %req.repo_id, "hook callback: timestamp too far in future");
        return (StatusCode::FORBIDDEN, "callback timestamp invalid").into_response();
    }

    // The localhost callback selects its tenant by an HMAC-bound UUID rather
    // than Host. Keep the control-plane transport usable for canonical pushes
    // without authorizing an extra community in single-community mode.
    if !state.config.multi_community_relay {
        match state.db.lookup_community_host(community).await {
            Ok(Some(host)) if crate::single_community::serves_host(&state.config, &host) => {}
            _ => return (StatusCode::FORBIDDEN, "community not served").into_response(),
        }
    }

    // 4. Validate and resolve kind:30617 for this repo.
    // Query by (community_id, kind=30617, pubkey=owner, d_tag=repo_id) to
    // prevent spoofing and keep the localhost hook callback on the same
    // server-resolved tenant as the git HTTP request that spawned it.
    let owner_bytes = match hex::decode(&req.repo_owner) {
        Ok(b) if b.len() == 32 => b,
        _ => {
            return (StatusCode::FORBIDDEN, "invalid repo owner").into_response();
        }
    };
    let query = EventQuery {
        kinds: Some(vec![30617]),
        pubkey: Some(owner_bytes.clone()),
        d_tag: Some(req.repo_id.clone()),
        global_only: true,
        limit: Some(1),
        ..EventQuery::for_community(community)
    };
    let repo_event = match state.db.query_events(&query).await {
        Ok(mut events) => {
            if let Some(event) = events.pop() {
                event
            } else {
                warn!(repo = %req.repo_id, "hook callback: kind:30617 not found");
                return (StatusCode::FORBIDDEN, "repository not found").into_response();
            }
        }
        Err(e) => {
            error!(repo = %req.repo_id, error = %e, "hook callback: DB error");
            return (StatusCode::FORBIDDEN, "internal error").into_response();
        }
    };

    // 5. Parse protection rules from kind:30617 tags.
    let tags: Vec<Vec<String>> = repo_event
        .event
        .tags
        .iter()
        .map(|t| t.as_slice().to_vec())
        .collect();

    let rules = match parse_protection_tags(&tags) {
        // A rule the relay does not understand means no: never skip it.
        Ok(parsed) if !parsed.unknown_rules.is_empty() => {
            warn!(repo = %req.repo_id, rules = ?parsed.unknown_rules, "hook callback: unknown buzz-protect rule (deny)");
            let reason = format!(
                "unknown protection rule {:?}: this relay does not enforce it, so it denies",
                parsed.unknown_rules[0]
            );
            let denials = req
                .ref_updates
                .iter()
                .map(|r| DenialResponse {
                    ref_name: r.ref_name.clone(),
                    reason: reason.clone(),
                })
                .collect();
            return refuse_refs(&state, community, &req, denials).await;
        }
        Ok(parsed) => parsed.rules,
        Err(e) => {
            warn!(repo = %req.repo_id, error = %e, "hook callback: malformed protection tags");
            // Fail-closed: malformed rules = deny.
            return refuse(&state, community, &req, "malformed protection rules").await;
        }
    };

    // 6. Resolve channel binding via the shared resolver (same first-tag,
    // fail-closed semantics as the read gate) and check archived state
    // (applies to ALL pushers including owner).
    //
    // `Broken` denies HERE, before owner resolution: a malformed or
    // ambiguous first binding fails closed for *everyone*, exactly like the
    // read gate. Letting it fall through as "unbound" would hand the owner
    // short-circuit below a push path through a binding the read gate
    // refuses to honor — the tri-state exists precisely so Broken and
    // NotBound cannot collapse. Only genuinely-NotBound repos proceed, and
    // only they may earn the remediation-token denial.
    let binding = crate::api::git::binding::resolve_repo_binding(&repo_event.event);
    let announced = match binding {
        crate::api::git::binding::RepoBinding::Bound(id) => Some(id),
        _ => None,
    };
    // The pinned channel is the binding (`git_land::bind_channel`); the tag
    // only seeds a pin the repository does not have yet.
    let repo = crate::api::git::land::coordinate(&req.repo_owner, &req.repo_id);
    let channel_id = match state.db.git_repo_channel(community, &repo, announced).await {
        Ok(Some(channel)) => Some(channel),
        Ok(None) if binding == crate::api::git::binding::RepoBinding::Broken => {
            warn!(repo = %req.repo_id, "hook callback: broken buzz-channel binding");
            // Deliberately NOT the no_channel_binding token body: the
            // remediation contract is NotBound-only. A broken binding is
            // ambiguity, and ambiguity gets a generic denial (matching the
            // read gate's posture for the same announcement).
            return refuse(&state, community, &req, "invalid channel binding").await;
        }
        Ok(None) => None,
        Err(e) => {
            error!(error = %e, "hook callback: binding lookup failed");
            return (StatusCode::FORBIDDEN, "internal error").into_response();
        }
    };

    if let Some(ch_id) = channel_id {
        match state.db.get_channel(community, ch_id).await {
            Ok(ch) if ch.archived_at.is_some() => {
                return refuse(&state, community, &req, "channel is archived (read-only)").await;
            }
            Err(e) => {
                error!(error = %e, "hook callback: channel lookup failed");
                return (StatusCode::FORBIDDEN, "internal error").into_response();
            }
            _ => {} // Channel exists and is not archived.
        }
    }

    // 7. Credit the pusher with its role in the bound channel, and nothing
    // else: announcing the repo or owning its announcer grants no authority.
    let Some(channel_id) = channel_id else {
        warn!(repo = %req.repo_id, "hook callback: no buzz-channel binding");
        // Declared cross-component contract — see the const docs in
        // buzz-core::git_perms for who consumes the token and why the body
        // also repeats the legacy phrase.
        return refuse(&state, community, &req, GIT_NO_CHANNEL_BINDING_BODY).await;
    };
    let pusher_bytes = match hex::decode(&req.pusher_pubkey) {
        Ok(bytes) if bytes.len() == 32 => bytes,
        _ => return (StatusCode::FORBIDDEN, "invalid pusher pubkey").into_response(),
    };
    // The hook authenticates the repo-scoped delegation level. The land
    // revalidates its liveness and owner before publishing anything.
    let git_role = match crate::api::git::authority::channel_role(
        &state.db,
        community,
        channel_id,
        &pusher_bytes,
        req.grant,
    )
    .await
    {
        Ok(Ok(role)) => role,
        Ok(Err(reason)) => return refuse(&state, community, &req, reason).await,
        Err(e) => {
            error!(error = %e, "hook callback: role lookup failed");
            return (StatusCode::FORBIDDEN, "internal error").into_response();
        }
    };

    // 9. Classify ref updates and evaluate policy.
    let updates: Vec<RefUpdate> = req
        .ref_updates
        .iter()
        .map(|r| RefUpdate {
            ref_name: r.ref_name.clone(),
            kind: UpdateKind::classify(&r.old_oid, &r.new_oid, r.is_ancestor),
            old_oid: r.old_oid.clone(),
            new_oid: r.new_oid.clone(),
        })
        .collect();

    match evaluate_push(&updates, git_role, &rules) {
        Ok(()) => Json(HookCallbackResponse {
            allowed: true,
            denials: vec![],
        })
        .into_response(),
        Err(denials) => {
            let denials = denials.into_iter().map(DenialResponse::from).collect();
            refuse_refs(&state, community, &req, denials).await
        }
    }
}

/// Refuse every ref in the push for `reason`, with a receipt.
async fn refuse(
    state: &AppState,
    community: buzz_core::CommunityId,
    req: &HookCallbackRequest,
    reason: &'static str,
) -> Response {
    let denials: Vec<_> = req
        .ref_updates
        .iter()
        .map(|r| DenialResponse {
            ref_name: r.ref_name.clone(),
            reason: reason.to_string(),
        })
        .collect();
    receipt(state, community, req, &denials).await;
    (StatusCode::FORBIDDEN, reason).into_response()
}

/// Refuse the push with per-ref `denials`, with a receipt.
async fn refuse_refs(
    state: &AppState,
    community: buzz_core::CommunityId,
    req: &HookCallbackRequest,
    denials: Vec<DenialResponse>,
) -> Response {
    receipt(state, community, req, &denials).await;
    let response = HookCallbackResponse {
        allowed: false,
        denials,
    };
    (StatusCode::FORBIDDEN, Json(response)).into_response()
}

/// Record a hook refusal as the land records its own: the hook refuses
/// before any land runs, so this is the refusal's only record. Only an
/// authenticated callback for a resolved repository gets here. The push is
/// refused whether or not the receipt commits.
async fn receipt(
    state: &AppState,
    community: buzz_core::CommunityId,
    req: &HookCallbackRequest,
    denials: &[DenialResponse],
) {
    let entry = buzz_audit::NewAuditEntry {
        community_id: community,
        action: buzz_audit::AuditAction::GitRefused,
        actor_pubkey: hex::decode(&req.pusher_pubkey).ok(),
        object_id: Some(
            crate::api::git::land::coordinate(&req.repo_owner, &req.repo_id).to_string(),
        ),
        detail: serde_json::json!({
            "kind": "push",
            "stage": "hook",
            "updates": req.ref_updates.iter().map(|u| serde_json::json!({
                "ref": u.ref_name, "old": u.old_oid, "new": u.new_oid,
            })).collect::<Vec<_>>(),
            "reasons": denials.iter().map(|d| serde_json::json!({
                "ref": d.ref_name, "reason": d.reason,
            })).collect::<Vec<_>>(),
        }),
    };
    let written = async {
        let mut tx = state.db.begin_event_write_transaction(community).await?;
        buzz_audit::AuditService::log_in_transaction(&mut tx, entry).await?;
        tx.commit().await?;
        Ok::<_, buzz_audit::AuditError>(())
    };
    if let Err(e) = written.await {
        error!(repo = %req.repo_id, error = %e, "hook callback: refusal receipt failed");
    }
}

/// Generate the HMAC signature for a hook callback payload.
///
/// Called by the relay when setting up the pre-receive hook environment.
#[allow(clippy::too_many_arguments)] // one per signed field
pub fn generate_hook_hmac(
    secret: &[u8],
    repo_id: &str,
    repo_owner: &str,
    community_id: &str,
    pusher_pubkey: &str,
    grant: Option<buzz_core::channel::MemberRole>,
    ref_updates: &[HookRefUpdate],
    timestamp: u64,
) -> String {
    let req = HookCallbackRequest {
        repo_id: repo_id.to_string(),
        repo_owner: repo_owner.to_string(),
        community_id: community_id.to_string(),
        pusher_pubkey: pusher_pubkey.to_string(),
        grant,
        ref_updates: ref_updates.to_vec(),
        timestamp,
        signature: String::new(), // Not used in computation.
    };
    let mac_bytes = compute_hmac(secret, &req);
    hex::encode(mac_bytes)
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    fn make_request() -> HookCallbackRequest {
        HookCallbackRequest {
            repo_id: "test-repo".to_string(),
            repo_owner: "a".repeat(64),
            community_id: uuid::Uuid::from_u128(1).to_string(),
            pusher_pubkey: "b".repeat(64),
            grant: None,
            ref_updates: vec![HookRefUpdate {
                old_oid: "1".repeat(40),
                new_oid: "2".repeat(40),
                ref_name: "refs/heads/main".to_string(),
                is_ancestor: true,
            }],
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            signature: String::new(),
        }
    }

    fn sign_request(req: &mut HookCallbackRequest, secret: &[u8]) {
        let mac = compute_hmac(secret, req);
        req.signature = hex::encode(mac);
    }

    #[test]
    fn hmac_valid_signature_accepted() {
        let secret = b"test-secret-key";
        let mut req = make_request();
        sign_request(&mut req, secret);
        assert!(verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_wrong_secret_rejected() {
        let mut req = make_request();
        sign_request(&mut req, b"correct-secret");
        assert!(!verify_hmac(b"wrong-secret", &req));
    }

    /// Deploy-skew guard for the unbound-repo deny body. The token
    /// (`no_channel_binding`, underscores) and the legacy phrase
    /// (`no channel binding`, spaces) do NOT contain each other, so the body
    /// must carry both: the token for structured consumers (Desktop's merge
    /// classifier and dialog matcher), the phrase for desktops already in
    /// the field that prose-match it. Relay ships continuously and Desktop
    /// on release cadence — dropping the phrase strands every old desktop
    /// on a new relay. Asserted against the shared consts, not re-typed
    /// literals, so the const and this test cannot drift apart separately.
    #[test]
    fn no_channel_binding_body_satisfies_old_and_new_matchers() {
        assert!(
            GIT_NO_CHANNEL_BINDING_BODY.starts_with(&format!(
                "{}: ",
                buzz_core::git_perms::GIT_NO_CHANNEL_BINDING_TOKEN
            )),
            "new structured consumers match the token prefix"
        );
        assert!(
            GIT_NO_CHANNEL_BINDING_BODY.contains("no channel binding"),
            "shipped desktops prose-match this exact phrase (spaces, not underscores)"
        );
    }

    #[test]
    fn hmac_tampered_repo_id_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        sign_request(&mut req, secret);
        req.repo_id = "evil-repo".to_string();
        assert!(!verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_tampered_pusher_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        sign_request(&mut req, secret);
        req.pusher_pubkey = "c".repeat(64);
        assert!(!verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_tampered_grant_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        sign_request(&mut req, secret);
        req.grant = Some(buzz_core::channel::MemberRole::Admin);
        assert!(!verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_tampered_ref_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        sign_request(&mut req, secret);
        req.ref_updates[0].ref_name = "refs/heads/evil".to_string();
        assert!(!verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_tampered_is_ancestor_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        sign_request(&mut req, secret);
        req.ref_updates[0].is_ancestor = false; // Flip FF → NFF
        assert!(!verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_tampered_owner_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        sign_request(&mut req, secret);
        req.repo_owner = "c".repeat(64);
        assert!(!verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_tampered_timestamp_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        sign_request(&mut req, secret);
        req.timestamp += 1;
        assert!(!verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_invalid_hex_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        req.signature = "not-valid-hex!!!".to_string();
        assert!(!verify_hmac(secret, &req));
    }

    /// Tampering the server-resolved community changes the HMAC input, so a
    /// hook callback cannot be replayed across communities even though the
    /// localhost policy endpoint itself has no inbound Host header.
    #[test]
    fn hmac_tampered_community_rejected() {
        let secret = b"test-secret";
        let mut req = make_request();
        sign_request(&mut req, secret);
        req.community_id = uuid::Uuid::from_u128(2).to_string();
        assert!(!verify_hmac(secret, &req));
    }

    #[test]
    fn hmac_deterministic_across_ref_order() {
        let secret = b"test-secret";
        let mut req1 = make_request();
        req1.ref_updates.push(HookRefUpdate {
            old_oid: "3".repeat(40),
            new_oid: "4".repeat(40),
            ref_name: "refs/heads/develop".to_string(),
            is_ancestor: false,
        });
        let mut req2 = req1.clone();
        // Reverse the ref order — HMAC should be the same (sorted internally).
        req2.ref_updates.reverse();
        let mac1 = compute_hmac(secret, &req1);
        let mac2 = compute_hmac(secret, &req2);
        assert_eq!(mac1, mac2);
    }

    #[test]
    fn generate_hook_hmac_matches_verify() {
        let secret = b"test-secret";
        let mut req = make_request();
        let sig = generate_hook_hmac(
            secret,
            &req.repo_id,
            &req.repo_owner,
            &req.community_id,
            &req.pusher_pubkey,
            req.grant,
            &req.ref_updates,
            req.timestamp,
        );
        req.signature = sig;
        assert!(verify_hmac(secret, &req));
    }

    /// Cross-boundary HMAC integration test.
    ///
    /// Runs the bash HMAC computation logic (extracted from the pre-receive hook)
    /// and compares its output against Rust's `generate_hook_hmac`. This is the
    /// most critical test — it verifies the bash/Rust format agreement that the
    /// entire security model depends on.
    #[test]
    fn bash_hmac_matches_rust_hmac() {
        let secret = "cross-boundary-test-secret-key-1234";
        let repo_id = "my-project";
        let repo_owner = "ab".repeat(32); // 64 hex chars
        let pusher = "cd".repeat(32); // 64 hex chars
        let community_id = uuid::Uuid::from_u128(1).to_string();
        let timestamp: u64 = 1700000000;

        // Two refs, intentionally out of sorted order to test sorting.
        let ref_updates = vec![
            HookRefUpdate {
                old_oid: "b".repeat(40),
                new_oid: "c".repeat(40),
                ref_name: "refs/heads/main".to_string(),
                is_ancestor: true,
            },
            HookRefUpdate {
                old_oid: "a".repeat(40),
                new_oid: "d".repeat(40),
                ref_name: "refs/heads/feature".to_string(),
                is_ancestor: false,
            },
        ];

        // Compute Rust-side HMAC.
        let rust_sig = generate_hook_hmac(
            secret.as_bytes(),
            repo_id,
            &repo_owner,
            &community_id,
            &pusher,
            Some(buzz_core::channel::MemberRole::Member),
            &ref_updates,
            timestamp,
        );

        // Bash script that replicates the hook's HMAC computation.
        // This is the exact logic from hook.rs PRE_RECEIVE_HOOK, extracted into
        // a standalone script with hardcoded values.
        let bash_script = format!(
            r#"
export LC_ALL=C
BUZZ_REPO_ID="{repo_id}"
BUZZ_REPO_OWNER="{repo_owner}"
BUZZ_COMMUNITY_ID="{community_id}"
BUZZ_PUSHER_PUBKEY="{pusher}"
BUZZ_PUSH_GRANT="member"
BUZZ_HOOK_SECRET="{secret}"
TIMESTAMP="{timestamp}"

# Simulate the HMAC_FILE with two refs (unsorted, like the hook writes them)
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT
HMAC_FILE="$WORK_DIR/hmac"

# Write refs in the order they'd arrive (main first, feature second)
echo "refs/heads/main {old1} {new1} 1" >> "$HMAC_FILE"
echo "refs/heads/feature {old2} {new2} 0" >> "$HMAC_FILE"

# Build HMAC input — exact logic from hook script
REPO_ID_LEN=${{#BUZZ_REPO_ID}}
HMAC_INPUT="${{REPO_ID_LEN}}:${{BUZZ_REPO_ID}}|${{BUZZ_REPO_OWNER}}|${{BUZZ_COMMUNITY_ID}}|${{BUZZ_PUSHER_PUBKEY}}|${{BUZZ_PUSH_GRANT}}|"
sort "$HMAC_FILE" | while IFS=' ' read -r ref_name old_oid new_oid is_anc; do
    REF_LEN=${{#ref_name}}
    printf '%s%s%s:%s%s' "$old_oid" "$new_oid" "$REF_LEN" "$ref_name" "$is_anc"
done > "$HMAC_FILE.concat"
HMAC_INPUT="${{HMAC_INPUT}}$(cat "$HMAC_FILE.concat")|${{TIMESTAMP}}"

# Compute HMAC-SHA256
printf '%s' "$HMAC_INPUT" | openssl dgst -sha256 -hmac "$BUZZ_HOOK_SECRET" -hex 2>/dev/null | sed 's/.*= //'
"#,
            repo_id = repo_id,
            repo_owner = repo_owner,
            community_id = community_id,
            pusher = pusher,
            secret = secret,
            timestamp = timestamp,
            old1 = "b".repeat(40),
            new1 = "c".repeat(40),
            old2 = "a".repeat(40),
            new2 = "d".repeat(40),
        );

        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(&bash_script)
            .output()
            .expect("failed to run bash");

        assert!(
            output.status.success(),
            "bash script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let bash_sig = String::from_utf8_lossy(&output.stdout).trim().to_string();

        assert_eq!(
            rust_sig, bash_sig,
            "HMAC mismatch!\n  Rust: {rust_sig}\n  Bash: {bash_sig}\n\
             The pre-receive hook and policy endpoint disagree on the canonical format."
        );
    }

    /// Cross-boundary test with a single ref (simpler case).
    #[test]
    fn bash_hmac_single_ref() {
        let secret = "single-ref-secret";
        let repo_id = "test-repo";
        let repo_owner = "a".repeat(64);
        let pusher = "b".repeat(64);
        let community_id = uuid::Uuid::from_u128(1).to_string();
        let timestamp: u64 = 1700000001;

        let ref_updates = vec![HookRefUpdate {
            old_oid: "1".repeat(40),
            new_oid: "2".repeat(40),
            ref_name: "refs/heads/main".to_string(),
            is_ancestor: true,
        }];

        let rust_sig = generate_hook_hmac(
            secret.as_bytes(),
            repo_id,
            &repo_owner,
            &community_id,
            &pusher,
            None,
            &ref_updates,
            timestamp,
        );

        let bash_script = format!(
            r#"
export LC_ALL=C
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT
HMAC_FILE="$WORK_DIR/hmac"
echo "refs/heads/main {old} {new} 1" >> "$HMAC_FILE"
BUZZ_REPO_ID="{repo_id}"
REPO_ID_LEN=${{#BUZZ_REPO_ID}}
HMAC_INPUT="${{REPO_ID_LEN}}:${{BUZZ_REPO_ID}}|{owner}|{community_id}|{pusher}|none|"
sort "$HMAC_FILE" | while IFS=' ' read -r ref_name old_oid new_oid is_anc; do
    REF_LEN=${{#ref_name}}
    printf '%s%s%s:%s%s' "$old_oid" "$new_oid" "$REF_LEN" "$ref_name" "$is_anc"
done > "$HMAC_FILE.concat"
HMAC_INPUT="${{HMAC_INPUT}}$(cat "$HMAC_FILE.concat")|{timestamp}"
printf '%s' "$HMAC_INPUT" | openssl dgst -sha256 -hmac "{secret}" -hex 2>/dev/null | sed 's/.*= //'
"#,
            old = "1".repeat(40),
            new = "2".repeat(40),
            repo_id = repo_id,
            owner = repo_owner,
            community_id = community_id,
            pusher = pusher,
            timestamp = timestamp,
            secret = secret,
        );

        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(&bash_script)
            .output()
            .expect("failed to run bash");

        assert!(
            output.status.success(),
            "bash script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let bash_sig = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(
            rust_sig, bash_sig,
            "Single-ref HMAC mismatch!\n  Rust: {rust_sig}\n  Bash: {bash_sig}"
        );
    }

    // ── hook_policy_check binding gate (requires Postgres) ──────────────

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn policy_test_state() -> Arc<AppState> {
        let mut config = crate::config::Config::for_test(); // [FI-TRACE-ENV-RACE]
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_string());
        let pool = sqlx::PgPool::connect(&config.database_url)
            .await
            .expect("connect test DB");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
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
        Arc::new(state)
    }

    /// Announce `repo_id` with the given tags, then push to it as its own
    /// announcement author and return the response.
    async fn owner_push_response(
        state: &Arc<AppState>,
        community: buzz_core::CommunityId,
        keys: &nostr::Keys,
        repo_id: &str,
        binding_tags: Vec<nostr::Tag>,
    ) -> axum::response::Response {
        announce(state, community, keys, repo_id, binding_tags).await;
        push_response(state, community, keys, repo_id, &keys.public_key(), None).await
    }

    async fn announce(
        state: &Arc<AppState>,
        community: buzz_core::CommunityId,
        keys: &nostr::Keys,
        repo_id: &str,
        extra_tags: Vec<nostr::Tag>,
    ) {
        use nostr::{EventBuilder, Kind, Tag};

        let mut tags = vec![Tag::parse(["d", repo_id]).unwrap()];
        tags.extend(extra_tags);
        let event = EventBuilder::new(Kind::Custom(30617), "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign 30617");
        state
            .db
            .insert_event(community, &event, None)
            .await
            .expect("insert 30617");
        // As the announcement handler does: the name row carries the pin.
        state
            .db
            .reserve_repo_name(community, repo_id, &keys.public_key().to_hex())
            .await
            .expect("reserve repo name");
    }

    /// Push a new `refs/heads/main` to `announcer`'s `repo_id` as `pusher`.
    async fn push_response(
        state: &Arc<AppState>,
        community: buzz_core::CommunityId,
        announcer: &nostr::Keys,
        repo_id: &str,
        pusher: &nostr::PublicKey,
        grant: Option<buzz_core::channel::MemberRole>,
    ) -> axum::response::Response {
        let mut req = HookCallbackRequest {
            repo_id: repo_id.to_string(),
            repo_owner: announcer.public_key().to_hex(),
            community_id: community.as_uuid().to_string(),
            pusher_pubkey: pusher.to_hex(),
            grant,
            ref_updates: vec![HookRefUpdate {
                old_oid: "0".repeat(40),
                new_oid: "2".repeat(40),
                ref_name: "refs/heads/main".to_string(),
                is_ancestor: false,
            }],
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            signature: String::new(),
        };
        let secret = state.config.git_hook_hmac_secret.clone();
        sign_request(&mut req, secret.as_bytes());
        hook_policy_check(State(Arc::clone(state)), Json(req)).await
    }

    async fn body_string(response: axum::response::Response) -> (StatusCode, String) {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        (status, String::from_utf8(bytes.to_vec()).expect("utf-8"))
    }

    /// The tri-state trap the resolver exists to prevent: a broken (malformed
    /// or ambiguous-first) binding must fail closed for EVERYONE on push —
    /// including the announcement author — *before* the owner short-circuit
    /// grants `MemberRole::Owner`. Collapsing `Broken` into "unbound" hands
    /// the owner a push path through a binding the read gate refuses to
    /// honor. The remediation token stays reserved for genuinely NotBound.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn push_gate_denies_owner_through_broken_binding() {
        use nostr::{Keys, Tag};

        let state = policy_test_state().await;
        let host = format!("policy-{}.example", uuid::Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("community")
            .id;
        let keys = Keys::generate();

        // Malformed first + valid-looking second: the ambiguity must deny,
        // and the parseable duplicate must not rescue the push.
        let response = owner_push_response(
            &state,
            community,
            &keys,
            &format!("repo-{}", uuid::Uuid::new_v4().simple()),
            vec![
                Tag::parse(["buzz-channel", "not-a-uuid"]).unwrap(),
                Tag::parse(["buzz-channel", &uuid::Uuid::new_v4().to_string()]).unwrap(),
            ],
        )
        .await;
        let (status, body) = body_string(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(
            body, "invalid channel binding",
            "owner pushing through a broken binding must be denied generically"
        );
        assert!(
            !body.contains(buzz_core::git_perms::GIT_NO_CHANNEL_BINDING_TOKEN),
            "remediation token is NotBound-only; Broken must never earn it"
        );

        // Control: the same owner pushing a genuinely NEVER-BOUND repo gets
        // the remediation body. Announcing a repo grants no authority, so
        // this also denies, but it names the fix; that pins the generic
        // denial above to Broken specifically.
        let response = owner_push_response(
            &state,
            community,
            &keys,
            &format!("repo-{}", uuid::Uuid::new_v4().simple()),
            vec![],
        )
        .await;
        let (status, body) = body_string(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, GIT_NO_CHANNEL_BINDING_BODY);
    }

    /// Who the push gate credits, in one channel-bound repo: only roles in
    /// the bound channel count, an agent never holds more than its recorded
    /// owner currently does, and a rule the relay does not know denies.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn push_gate_credits_only_channel_roles() {
        use buzz_core::channel::MemberRole;
        use nostr::{Keys, Tag};

        let state = policy_test_state().await;
        let host = format!("policy-{}.example", uuid::Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("community")
            .id;
        let admin = Keys::generate();
        let human = Keys::generate();
        let agent = Keys::generate();
        let announcer_agent = Keys::generate();
        let channel = uuid::Uuid::new_v4();
        for key in [&admin, &human, &agent, &announcer_agent] {
            state
                .db
                .ensure_user(community, key.public_key().as_bytes())
                .await
                .expect("user");
        }
        state
            .db
            .create_channel_with_id(
                community,
                channel,
                &format!("policy-{}", channel.simple()),
                buzz_db::channel::ChannelType::Stream,
                buzz_db::channel::ChannelVisibility::Open,
                None,
                admin.public_key().as_bytes(),
                None,
            )
            .await
            .expect("channel");
        let put = |key: &Keys, role| {
            let state = Arc::clone(&state);
            let (key, by) = (key.public_key(), admin.public_key());
            async move {
                state
                    .db
                    .add_member(
                        community,
                        channel,
                        key.as_bytes(),
                        role,
                        Some(by.as_bytes()),
                    )
                    .await
                    .expect("member");
            }
        };
        put(&agent, MemberRole::Bot).await;
        for owned in [&agent, &announcer_agent] {
            state
                .db
                .set_agent_owner(
                    community,
                    owned.public_key().as_bytes(),
                    human.public_key().as_bytes(),
                )
                .await
                .expect("owner");
        }
        let bound = || vec![Tag::parse(["buzz-channel", &channel.to_string()]).unwrap()];
        let push_granted = |announcer: &Keys, repo: &str, pusher: &Keys, granted| {
            let state = Arc::clone(&state);
            let (announcer, repo, pusher) =
                (announcer.clone(), repo.to_string(), pusher.public_key());
            async move {
                let response = push_response(
                    &state,
                    community,
                    &announcer,
                    &repo,
                    &pusher,
                    if granted {
                        Some(MemberRole::Member)
                    } else {
                        None
                    },
                )
                .await;
                body_string(response).await
            }
        };
        let push = |announcer: &Keys, repo: &str, pusher: &Keys| {
            push_granted(announcer, repo, pusher, true)
        };

        // Hook-stage refusal receipts for `repo`, newest first.
        let refused = |announcer: &Keys, repo: &str| {
            let state = Arc::clone(&state);
            let object = crate::api::git::land::coordinate(&announcer.public_key().to_hex(), repo)
                .to_string();
            async move {
                sqlx::query_scalar::<_, serde_json::Value>(
                    "SELECT detail FROM audit_log WHERE community_id = $1 \
                     AND action = 'git_refused' AND object_id = $2 ORDER BY seq DESC",
                )
                .bind(community.as_uuid())
                .bind(object)
                .fetch_all(state.db.pool())
                .await
                .unwrap()
            }
        };

        // An announcer outside the channel, and the human who owns the agent
        // that announced, hold nothing.
        let repo = format!("repo-{}", uuid::Uuid::new_v4().simple());
        announce(&state, community, &announcer_agent, &repo, bound()).await;
        for pusher in [&announcer_agent, &human] {
            let (code, body) = push(&announcer_agent, &repo, pusher).await;
            assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
            assert_eq!(body, "not a channel member");
        }

        // Direct membership stands without an owner channel role or a grant.
        let (code, body) = push_granted(&announcer_agent, &repo, &agent, false).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        let receipts = refused(&announcer_agent, &repo).await;
        assert_eq!(receipts.len(), 2, "{receipts:?}");
        assert_eq!(receipts[0]["stage"], "hook");

        // A weaker owner or grant cannot narrow the independent member role.
        put(&human, MemberRole::Guest).await;
        let (code, body) = push(&announcer_agent, &repo, &agent).await;
        assert_eq!(code, StatusCode::OK, "{body}");
        // Removing direct membership leaves only the owner's capped grant.
        state
            .db
            .remove_member(
                community,
                channel,
                agent.public_key().as_bytes(),
                admin.public_key().as_bytes(),
            )
            .await
            .unwrap();
        let (code, body) = push(&announcer_agent, &repo, &agent).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.contains("requires member role (you have guest)"),
            "{body}"
        );
        put(&human, MemberRole::Member).await;
        let (code, body) = push(&announcer_agent, &repo, &agent).await;
        assert_eq!(code, StatusCode::OK, "grant-only push: {body}");
        let (code, body) = push_granted(&announcer_agent, &repo, &agent, false).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "neither route: {body}");

        // A rule this relay does not know denies even the channel's owner.
        let strict = format!("repo-{}", uuid::Uuid::new_v4().simple());
        let mut tags = bound();
        tags.push(Tag::parse(["buzz-protect", "refs/heads/main", "require-two-reviews"]).unwrap());
        announce(&state, community, &admin, &strict, tags).await;
        let (code, body) = push(&admin, &strict, &admin).await;
        assert_eq!(code, StatusCode::FORBIDDEN, "{body}");
        assert!(body.contains("require-two-reviews"), "{body}");
        let receipts = refused(&admin, &strict).await;
        assert_eq!(receipts.len(), 1, "{receipts:?}");
        assert_eq!(receipts[0]["stage"], "hook");
        let known = format!("repo-{}", uuid::Uuid::new_v4().simple());
        let mut tags = bound();
        tags.push(Tag::parse(["buzz-protect", "refs/heads/main", "no-force-push"]).unwrap());
        announce(&state, community, &admin, &known, tags).await;
        let (code, body) = push(&admin, &known, &admin).await;
        assert_eq!(
            code,
            StatusCode::OK,
            "control: known rules still allow ({body})"
        );
        assert!(refused(&admin, &known).await.is_empty());
    }
}
