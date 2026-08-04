//! Internal policy endpoint — pre-receive hook callback.
//!
//! The pre-receive hook POSTs here with HMAC-signed payload containing
//! the pusher's pubkey, repo ID, and ref updates. This endpoint:
//!
//! 1. Validates HMAC signature + 30s TTL (fail-closed)
//! 2. Resolves kind:30617 → protection rules
//! 3. Requires exactly one canonical `buzz-channel` binding
//! 4. Resolves the pusher's current active channel role
//! 5. Delegates repository-Owner Git authority to the active repository author;
//!    otherwise promotes Bot → Member
//! 6. Calls `buzz_core::git_perms::evaluate_push()`
//! 7. Returns 200 (allow) or 403 (deny with reasons)
//!
//! # Bot Role Model
//!
//! Bots are intentionally added to channels by members/admins. For git push,
//! they're promoted to Member — protection rules still apply. Bot is a
//! designation (what it is), not a permission tier (what it can do). The
//! promotion is scoped to this module; the core `MemberRole::Bot` hierarchy
//! is unchanged.
//!
//! # Repository Author Delegation
//!
//! The kind:30617 author is the repository authority and the only principal
//! accepted by the exact-main synchronization CLI. The author can already
//! republish kind:30617 and rewrite its role thresholds, so denying that same
//! principal a role threshold protects nothing; those thresholds constrain
//! other channel members. While the author is an active Member, Admin, Owner,
//! or Bot in the bound channel, Git policy therefore treats it as repository
//! Owner. This repository-scoped delegation does not change the channel role,
//! admit non-members or Guests, or bypass non-role protection rules such as
//! `no-force-push`.
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

use buzz_core::channel::MemberRole;
use buzz_core::git_perms::{
    evaluate_push, parse_protection_tags, Denial, RefUpdate, UpdateKind,
    GIT_NO_CHANNEL_BINDING_BODY,
};
use buzz_db::EventQuery;

use super::binding::{resolve_repo_binding, RepoBinding};
use crate::state::AppState;

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
/// len(repo_id):repo_id | repo_owner(64) | community_id(36) | pusher(64) | sorted_refs | timestamp
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

/// Map an active channel role to the role used only for repository Git policy.
///
/// This is called only after active membership has been proved. Guests remain
/// read-only. Every other actor keeps the existing role mapping, including
/// Bot → Member.
fn effective_git_role(active_role: MemberRole, is_repo_author: bool) -> MemberRole {
    match (active_role, is_repo_author) {
        (MemberRole::Guest, _) => MemberRole::Guest,
        (_, true) => MemberRole::Owner,
        (MemberRole::Bot, false) => MemberRole::Member,
        (role, false) => role,
    }
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
        pubkey: Some(owner_bytes),
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
        Ok(parsed) => {
            // Log unknown rules as warnings (helps catch typos).
            for unknown in &parsed.unknown_rules {
                warn!(repo = %req.repo_id, rule = %unknown, "unknown buzz-protect rule (skipped)");
            }
            parsed.rules
        }
        Err(e) => {
            warn!(repo = %req.repo_id, error = %e, "hook callback: malformed protection tags");
            // Fail-closed: malformed rules = deny.
            return (StatusCode::FORBIDDEN, "malformed protection rules").into_response();
        }
    };

    // 6. Resolve exactly one canonical channel binding and check archived
    // state. Missing, malformed, empty, or duplicate bindings fail closed for
    // every pusher. Legacy `h` / `project-channel` tags never participate.
    let channel_id = match resolve_repo_binding(&repo_event.event) {
        RepoBinding::Bound(id) => id,
        RepoBinding::NotBound => {
            warn!(repo = %req.repo_id, "hook callback: no buzz-channel binding");
            // Declared cross-component contract — see the const docs in
            // buzz-core::git_perms for who consumes the token and why
            // the body also repeats the legacy phrase.
            return (StatusCode::FORBIDDEN, GIT_NO_CHANNEL_BINDING_BODY).into_response();
        }
        RepoBinding::Broken => {
            warn!(repo = %req.repo_id, "hook callback: broken buzz-channel binding");
            return (StatusCode::FORBIDDEN, "invalid channel binding").into_response();
        }
    };

    match state.db.get_channel(community, channel_id).await {
        Ok(ch) if ch.archived_at.is_some() => {
            return (StatusCode::FORBIDDEN, "channel is archived (read-only)").into_response();
        }
        Err(e) => {
            error!(error = %e, "hook callback: channel lookup failed");
            return (StatusCode::FORBIDDEN, "internal error").into_response();
        }
        _ => {} // Channel exists and is not archived.
    }

    // 7. Resolve the pusher's current active channel role. Repository
    // authorship never admits a non-member, and managed-agent ownership never
    // confers Git authority.
    let pusher_bytes = match hex::decode(&req.pusher_pubkey) {
        Ok(bytes) if bytes.len() == 32 => bytes,
        _ => return (StatusCode::FORBIDDEN, "invalid pusher pubkey").into_response(),
    };
    let active_role = match state
        .db
        .get_member_role(community, channel_id, &pusher_bytes)
        .await
    {
        Ok(Some(role_str)) => match role_str.parse::<MemberRole>() {
            Ok(role) => role,
            Err(_) => {
                error!(role = %role_str, "hook callback: unknown role");
                return (StatusCode::FORBIDDEN, "internal error").into_response();
            }
        },
        Ok(None) => {
            return (StatusCode::FORBIDDEN, "not a channel member").into_response();
        }
        Err(e) => {
            error!(error = %e, "hook callback: role lookup failed");
            return (StatusCode::FORBIDDEN, "internal error").into_response();
        }
    };

    // 8. Effective Git role. The repository author gets repository-scoped
    // Owner authority only after active membership is proved. This closes the
    // exact-main import/mirror authorization graph without changing channel
    // authority or weakening protection rules for any other actor.
    let is_repo_author = req.pusher_pubkey == repo_event.event.pubkey.to_hex();
    let git_role = effective_git_role(active_role, is_repo_author);

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
            let response = HookCallbackResponse {
                allowed: false,
                denials: denials.into_iter().map(DenialResponse::from).collect(),
            };
            (StatusCode::FORBIDDEN, Json(response)).into_response()
        }
    }
}

/// Generate the HMAC signature for a hook callback payload.
///
/// Called by the relay when setting up the pre-receive hook environment.
pub fn generate_hook_hmac(
    secret: &[u8],
    repo_id: &str,
    repo_owner: &str,
    community_id: &str,
    pusher_pubkey: &str,
    ref_updates: &[HookRefUpdate],
    timestamp: u64,
) -> String {
    let req = HookCallbackRequest {
        repo_id: repo_id.to_string(),
        repo_owner: repo_owner.to_string(),
        community_id: community_id.to_string(),
        pusher_pubkey: pusher_pubkey.to_string(),
        ref_updates: ref_updates.to_vec(),
        timestamp,
        signature: String::new(), // Not used in computation.
    };
    let mac_bytes = compute_hmac(secret, &req);
    hex::encode(mac_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request() -> HookCallbackRequest {
        HookCallbackRequest {
            repo_id: "test-repo".to_string(),
            repo_owner: "a".repeat(64),
            community_id: uuid::Uuid::from_u128(1).to_string(),
            pusher_pubkey: "b".repeat(64),
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
    fn active_repo_author_can_create_admin_protected_main() {
        let role = effective_git_role(MemberRole::Member, true);
        assert_eq!(role, MemberRole::Owner);

        let rules = vec![buzz_core::git_perms::parse_protection_tag(&[
            "refs/heads/main",
            "push:admin",
            "no-force-push",
        ])
        .expect("valid protection")];
        let update = RefUpdate {
            ref_name: "refs/heads/main".to_string(),
            kind: UpdateKind::Create,
            old_oid: "0".repeat(40),
            new_oid: "1".repeat(40),
        };

        assert!(evaluate_push(std::slice::from_ref(&update), role, &rules).is_ok());
        assert!(
            evaluate_push(
                std::slice::from_ref(&update),
                effective_git_role(MemberRole::Member, false),
                &rules,
            )
            .is_err(),
            "non-author member remains denied by push:admin"
        );
        assert!(
            evaluate_push(
                &[update],
                effective_git_role(MemberRole::Admin, false),
                &rules,
            )
            .is_ok(),
            "non-author admin keeps the normal allowed path"
        );
    }

    #[test]
    fn repo_author_delegation_excludes_guest() {
        assert_eq!(
            effective_git_role(MemberRole::Guest, true),
            MemberRole::Guest
        );
        for active_role in [
            MemberRole::Member,
            MemberRole::Admin,
            MemberRole::Owner,
            MemberRole::Bot,
        ] {
            assert_eq!(
                effective_git_role(active_role, true),
                MemberRole::Owner,
                "active repository author with channel role {active_role}"
            );
        }
    }

    #[test]
    fn non_author_git_roles_keep_existing_mapping() {
        for (active_role, expected) in [
            (MemberRole::Owner, MemberRole::Owner),
            (MemberRole::Admin, MemberRole::Admin),
            (MemberRole::Member, MemberRole::Member),
            (MemberRole::Guest, MemberRole::Guest),
            (MemberRole::Bot, MemberRole::Member),
        ] {
            assert_eq!(
                effective_git_role(active_role, false),
                expected,
                "non-author with channel role {active_role}"
            );
        }
    }

    #[test]
    fn git_binding_requires_exactly_one_canonical_tag() {
        use nostr::{EventBuilder, Keys, Kind, Tag};

        let channel = Uuid::new_v4();
        let announcement = |tags: Vec<Tag>| {
            EventBuilder::new(Kind::Custom(30617), "")
                .tags(tags)
                .sign_with_keys(&Keys::generate())
                .expect("sign announcement")
        };

        let bound = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["buzz-channel", &channel.to_string()]).unwrap(),
        ]);
        assert_eq!(resolve_repo_binding(&bound), RepoBinding::Bound(channel));

        let duplicate = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["buzz-channel", &channel.to_string()]).unwrap(),
            Tag::parse(["buzz-channel", &channel.to_string()]).unwrap(),
        ]);
        assert_eq!(
            resolve_repo_binding(&duplicate),
            RepoBinding::Broken,
            "even identical duplicate bindings are ambiguous"
        );

        let legacy_only = announcement(vec![
            Tag::parse(["d", "repo"]).unwrap(),
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["project-channel", &channel.to_string()]).unwrap(),
        ]);
        assert_eq!(
            resolve_repo_binding(&legacy_only),
            RepoBinding::NotBound,
            "legacy rendering tags never authorize Git"
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
HMAC_INPUT="${{REPO_ID_LEN}}:${{BUZZ_REPO_ID}}|${{BUZZ_REPO_OWNER}}|${{BUZZ_COMMUNITY_ID}}|${{BUZZ_PUSHER_PUBKEY}}|"
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
HMAC_INPUT="${{REPO_ID_LEN}}:${{BUZZ_REPO_ID}}|{owner}|{community_id}|{pusher}|"
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
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_string());
        let pool = sqlx::PgPool::connect(&config.database_url)
            .await
            .expect("connect test DB");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
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
        use nostr::{EventBuilder, Kind, Tag};

        let mut tags = vec![Tag::parse(["d", repo_id]).unwrap()];
        tags.extend(binding_tags);
        let event = EventBuilder::new(Kind::Custom(30617), "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign 30617");
        state
            .db
            .insert_event(community, &event, None)
            .await
            .expect("insert 30617");

        let owner_hex = keys.public_key().to_hex();
        let mut req = HookCallbackRequest {
            repo_id: repo_id.to_string(),
            repo_owner: owner_hex.clone(),
            community_id: community.as_uuid().to_string(),
            pusher_pubkey: owner_hex,
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

    async fn create_bound_channel(
        state: &Arc<AppState>,
        community: buzz_core::CommunityId,
        author: &nostr::Keys,
        author_role: Option<MemberRole>,
    ) -> Uuid {
        let creator = nostr::Keys::generate();
        let creator_pk = creator.public_key().to_bytes().to_vec();
        let author_pk = author.public_key().to_bytes().to_vec();
        state
            .db
            .ensure_user(community, &creator_pk)
            .await
            .expect("creator user");
        state
            .db
            .ensure_user(community, &author_pk)
            .await
            .expect("author user");

        let channel_id = Uuid::new_v4();
        state
            .db
            .create_channel_with_id(
                community,
                channel_id,
                &format!("git-policy-{}", channel_id.simple()),
                buzz_db::channel::ChannelType::Stream,
                buzz_db::channel::ChannelVisibility::Open,
                None,
                &creator_pk,
                None,
            )
            .await
            .expect("channel");
        if let Some(role) = author_role {
            state
                .db
                .add_member(community, channel_id, &author_pk, role, Some(&creator_pk))
                .await
                .expect("author membership");
        }
        channel_id
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn hook_allows_active_member_author_on_admin_main() {
        use nostr::{Keys, Tag};

        let state = policy_test_state().await;
        let host = format!("policy-{}.example", Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("community")
            .id;
        let author = Keys::generate();
        let channel_id =
            create_bound_channel(&state, community, &author, Some(MemberRole::Member)).await;

        let response = owner_push_response(
            &state,
            community,
            &author,
            &format!("repo-{}", Uuid::new_v4().simple()),
            vec![
                Tag::parse(["buzz-channel", &channel_id.to_string()]).unwrap(),
                Tag::parse([
                    "buzz-protect",
                    "refs/heads/main",
                    "push:admin",
                    "no-force-push",
                ])
                .unwrap(),
            ],
        )
        .await;
        let (status, body) = body_string(response).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert!(body.contains("\"allowed\":true"), "body: {body}");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn hook_denies_nonmember_author_before_delegation() {
        use nostr::{Keys, Tag};

        let state = policy_test_state().await;
        let host = format!("policy-{}.example", Uuid::new_v4().simple());
        let community = state
            .db
            .ensure_configured_community(&host)
            .await
            .expect("community")
            .id;
        let author = Keys::generate();
        let channel_id = create_bound_channel(&state, community, &author, None).await;

        let response = owner_push_response(
            &state,
            community,
            &author,
            &format!("repo-{}", Uuid::new_v4().simple()),
            vec![
                Tag::parse(["buzz-channel", &channel_id.to_string()]).unwrap(),
                Tag::parse(["buzz-protect", "refs/heads/main", "push:admin"]).unwrap(),
            ],
        )
        .await;
        let (status, body) = body_string(response).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, "not a channel member");
    }

    /// Missing, malformed, or duplicate bindings fail closed for EVERYONE on
    /// push, including the announcement author. Repository authorship is not
    /// channel authority.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn push_gate_denies_owner_without_exact_live_binding() {
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

        // Missing binding also denies. The response keeps the remediation
        // token, but no owner bypass turns it into an authorization success.
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
}
