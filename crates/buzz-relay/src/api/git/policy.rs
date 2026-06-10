//! Internal policy endpoint — pre-receive hook callback.
//!
//! The pre-receive hook POSTs here with HMAC-signed payload containing
//! the pusher's pubkey, repo ID, and ref updates. This endpoint:
//!
//! 1. Validates HMAC signature + 30s TTL (fail-closed)
//! 2. Resolves kind:30617 → protection rules
//! 3. Resolves pusher's channel role via sprout-channel binding
//! 4. Promotes Bot → Member (bots in a channel push as members)
//! 5. Calls `sprout_core::git_perms::evaluate_push()`
//! 6. Returns 200 (allow) or 403 (deny with reasons)
//!
//! # Bot Role Model
//!
//! Bots are intentionally added to channels by members/admins. For git push,
//! they're promoted to Member — protection rules still apply. Bot is a
//! designation (what it is), not a permission tier (what it can do). The
//! promotion is scoped to this module; the core `MemberRole::Bot` hierarchy
//! is unchanged.
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

use sprout_core::channel::MemberRole;
use sprout_core::git_perms::{evaluate_push, parse_protection_tags, Denial, RefUpdate, UpdateKind};
use sprout_db::EventQuery;

use crate::state::AppState;

// ── Types ────────────────────────────────────────────────────────────────────

/// Maximum age of a hook callback (seconds). Push is synchronous so 30s is generous.
const MAX_CALLBACK_AGE_SECS: u64 = 30;

/// Request payload from the pre-receive hook.
#[derive(Debug, Clone, Deserialize)]
pub struct HookCallbackRequest {
    /// Repo identifier (d-tag from kind:30617).
    pub repo_id: String,
    /// Hex-encoded repo owner pubkey (from URL path, verified against kind:30617).
    pub repo_owner: String,
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

// ── HMAC Verification ────────────────────────────────────────────────────────

/// Compute the canonical HMAC payload.
///
/// Format (length-prefixed, `|`-separated, structurally unambiguous):
/// ```text
/// len(repo_id):repo_id | repo_owner(64) | pusher(64) | sorted_refs | timestamp
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

// ── Handler ──────────────────────────────────────────────────────────────────

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
    // Query by (kind=30617, pubkey=owner, d_tag=repo_id) to prevent spoofing.
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
        ..Default::default()
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
                warn!(repo = %req.repo_id, rule = %unknown, "unknown sprout-protect rule (skipped)");
            }
            parsed.rules
        }
        Err(e) => {
            warn!(repo = %req.repo_id, error = %e, "hook callback: malformed protection tags");
            // Fail-closed: malformed rules = deny.
            return (StatusCode::FORBIDDEN, "malformed protection rules").into_response();
        }
    };

    // 6. Resolve channel and check archived state (applies to ALL pushers including owner).
    let channel_id = tags
        .iter()
        .find(|t| t.first().map(|s| s.as_str()) == Some("sprout-channel"))
        .and_then(|t| t.get(1))
        .and_then(|id| Uuid::parse_str(id).ok());

    if let Some(ch_id) = channel_id {
        match state.db.get_channel(ch_id).await {
            Ok(ch) if ch.archived_at.is_some() => {
                return (StatusCode::FORBIDDEN, "channel is archived (read-only)").into_response();
            }
            Err(e) => {
                error!(error = %e, "hook callback: channel lookup failed");
                return (StatusCode::FORBIDDEN, "internal error").into_response();
            }
            _ => {} // Channel exists and is not archived.
        }
    }

    // 7. Resolve pusher's role.
    let repo_owner_hex = hex::encode(repo_event.event.pubkey.to_bytes());
    let role = if req.pusher_pubkey == repo_owner_hex {
        MemberRole::Owner
    } else {
        match channel_id {
            None => {
                warn!(repo = %req.repo_id, "hook callback: no sprout-channel binding");
                return (StatusCode::FORBIDDEN, "no channel binding").into_response();
            }
            Some(ch_id) => {
                let pusher_bytes = match hex::decode(&req.pusher_pubkey) {
                    Ok(b) if b.len() == 32 => b,
                    _ => {
                        return (StatusCode::FORBIDDEN, "invalid pusher pubkey").into_response();
                    }
                };
                match state.db.get_member_role(ch_id, &pusher_bytes).await {
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
                }
            }
        }
    };

    // 8. Effective git role: bots intentionally added to a channel push as members.
    // Protection rules (push:admin, no-force-push, require-patch, etc.) still apply.
    // Bot is a designation (what it is), not a permission tier (what it can do).
    let git_role = match role {
        MemberRole::Bot => MemberRole::Member,
        other => other,
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
            let response = HookCallbackResponse {
                allowed: false,
                denials: denials.into_iter().map(DenialResponse::from).collect(),
            };
            (StatusCode::FORBIDDEN, Json(response)).into_response()
        }
    }
}

// ── HMAC Generation (for the relay to pass to the hook) ──────────────────────

/// Generate the HMAC signature for a hook callback payload.
///
/// Called by the relay when setting up the pre-receive hook environment.
pub fn generate_hook_hmac(
    secret: &[u8],
    repo_id: &str,
    repo_owner: &str,
    pusher_pubkey: &str,
    ref_updates: &[HookRefUpdate],
    timestamp: u64,
) -> String {
    let req = HookCallbackRequest {
        repo_id: repo_id.to_string(),
        repo_owner: repo_owner.to_string(),
        pusher_pubkey: pusher_pubkey.to_string(),
        ref_updates: ref_updates.to_vec(),
        timestamp,
        signature: String::new(), // Not used in computation.
    };
    let mac_bytes = compute_hmac(secret, &req);
    hex::encode(mac_bytes)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_request() -> HookCallbackRequest {
        HookCallbackRequest {
            repo_id: "test-repo".to_string(),
            repo_owner: "a".repeat(64),
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
SPROUT_REPO_ID="{repo_id}"
SPROUT_REPO_OWNER="{repo_owner}"
SPROUT_PUSHER_PUBKEY="{pusher}"
SPROUT_HOOK_SECRET="{secret}"
TIMESTAMP="{timestamp}"

# Simulate the HMAC_FILE with two refs (unsorted, like the hook writes them)
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT
HMAC_FILE="$WORK_DIR/hmac"

# Write refs in the order they'd arrive (main first, feature second)
echo "refs/heads/main {old1} {new1} 1" >> "$HMAC_FILE"
echo "refs/heads/feature {old2} {new2} 0" >> "$HMAC_FILE"

# Build HMAC input — exact logic from hook script
REPO_ID_LEN=${{#SPROUT_REPO_ID}}
HMAC_INPUT="${{REPO_ID_LEN}}:${{SPROUT_REPO_ID}}|${{SPROUT_REPO_OWNER}}|${{SPROUT_PUSHER_PUBKEY}}|"
sort "$HMAC_FILE" | while IFS=' ' read -r ref_name old_oid new_oid is_anc; do
    REF_LEN=${{#ref_name}}
    printf '%s%s%s:%s%s' "$old_oid" "$new_oid" "$REF_LEN" "$ref_name" "$is_anc"
done > "$HMAC_FILE.concat"
HMAC_INPUT="${{HMAC_INPUT}}$(cat "$HMAC_FILE.concat")|${{TIMESTAMP}}"

# Compute HMAC-SHA256
printf '%s' "$HMAC_INPUT" | openssl dgst -sha256 -hmac "$SPROUT_HOOK_SECRET" -hex 2>/dev/null | sed 's/.*= //'
"#,
            repo_id = repo_id,
            repo_owner = repo_owner,
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
SPROUT_REPO_ID="{repo_id}"
REPO_ID_LEN=${{#SPROUT_REPO_ID}}
HMAC_INPUT="${{REPO_ID_LEN}}:${{SPROUT_REPO_ID}}|{owner}|{pusher}|"
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
}
