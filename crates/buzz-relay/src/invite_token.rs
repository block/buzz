//! Stateless relay invite tokens.
//!
//! An invite code is a compact, URL-safe, HMAC-signed blob minted by a relay
//! admin/owner and later presented by a joining user. The relay verifies the
//! signature and expiry, then inserts the presenter into `relay_members` —
//! no server-side invite storage is required.
//!
//! ## Format
//!
//! ```text
//! code = base64url(payload_json) + "." + base64url(hmac_sha256(key, payload_json))
//! ```
//!
//! `payload_json` is a canonical JSON object:
//!
//! ```json
//! {"c":"<community uuid>","r":"member","e":1767000000,"n":"<random nonce>"}
//! ```
//!
//! ## Key derivation
//!
//! The HMAC key is derived from the relay's signing secret key:
//! `key = sha256(relay_secret_key_bytes || "buzz-invite-v1")`. Rotating the
//! relay keypair therefore invalidates all outstanding invites, which is the
//! intended blast-radius control for a leaked link.
//!
//! ## Security properties (and non-properties)
//!
//! - Codes are **multi-use until expiry** — there is no server-side "used"
//!   bit. Default expiry is deliberately short ([`DEFAULT_INVITE_TTL_SECS`]).
//! - Codes are **community-scoped**: a code minted for community A fails
//!   verification when presented to community B, even on the same deployment.
//! - Codes are **role-capped at `member`** at mint time (enforced by the mint
//!   route, and re-checked here on verify so a hand-crafted payload with an
//!   elevated role is rejected even if it carries a valid MAC from a future
//!   buggy caller).
//! - Revocation is coarse: rotate the relay keypair, or remove the member
//!   after the fact. Per-code revocation requires the future `relay_invites`
//!   table increment.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use buzz_core::tenant::CommunityId;

type HmacSha256 = Hmac<Sha256>;

/// Default invite lifetime: 72 hours.
pub const DEFAULT_INVITE_TTL_SECS: u64 = 72 * 60 * 60;

/// Maximum invite lifetime a mint request may ask for: 30 days.
pub const MAX_INVITE_TTL_SECS: u64 = 30 * 24 * 60 * 60;

/// Maximum accepted code length (defense against absurd inputs before any
/// parsing work happens). A real code is ~200 bytes.
const MAX_CODE_LEN: usize = 1024;

/// Domain-separation label mixed into the HMAC key derivation.
const KEY_DERIVATION_LABEL: &[u8] = b"buzz-invite-v1";

/// The signed payload carried inside an invite code.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InvitePayload {
    /// Community the invite admits into (UUID string form).
    pub c: String,
    /// Role granted on claim. Only `"member"` is valid in v1.
    pub r: String,
    /// Expiry as unix seconds.
    pub e: u64,
    /// Random nonce so identically-parameterised invites differ.
    pub n: String,
}

/// Why a code failed verification. Variants are deliberately coarse — the
/// HTTP layer maps all of them to a generic rejection so the endpoint does
/// not become an oracle for forging codes.
#[derive(Debug, PartialEq, Eq)]
pub enum InviteError {
    /// Structurally invalid (bad base64, bad JSON, wrong shape, too long).
    Malformed,
    /// MAC did not verify.
    BadSignature,
    /// Signature fine, but the expiry has passed.
    Expired,
    /// Signature fine, but minted for a different community.
    WrongCommunity,
    /// Signature fine, but the role is not one this relay grants via invite.
    InvalidRole,
}

impl std::fmt::Display for InviteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            InviteError::Malformed => "malformed invite code",
            InviteError::BadSignature => "invalid invite signature",
            InviteError::Expired => "invite code expired",
            InviteError::WrongCommunity => "invite not valid for this relay",
            InviteError::InvalidRole => "invite grants an unsupported role",
        };
        f.write_str(msg)
    }
}

/// Derive the invite HMAC key from the relay's signing secret.
///
/// `sha256(secret_key_bytes || label)` — the label domain-separates this use
/// from any other HMAC built on the same keypair.
pub fn derive_invite_key(relay_keys: &nostr::Keys) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(relay_keys.secret_key().as_secret_bytes());
    hasher.update(KEY_DERIVATION_LABEL);
    hasher.finalize().into()
}

fn sign_payload(key: &[u8; 32], payload_bytes: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(payload_bytes);
    mac.finalize().into_bytes().to_vec()
}

/// Mint an invite code for `community`, expiring `ttl_secs` from now.
///
/// The role is fixed to `"member"` — elevated roles are granted post-join via
/// the existing kind:9032 change-role command, never via a bearer link.
pub fn mint_invite(key: &[u8; 32], community: CommunityId, ttl_secs: u64) -> (String, u64) {
    let ttl = ttl_secs.clamp(60, MAX_INVITE_TTL_SECS);
    let expires_at = now_unix() + ttl;

    let nonce: [u8; 16] = rand::random();
    let payload = InvitePayload {
        c: community.as_uuid().to_string(),
        r: "member".to_string(),
        e: expires_at,
        n: URL_SAFE_NO_PAD.encode(nonce),
    };
    let payload_bytes = serde_json::to_vec(&payload).expect("payload serializes");
    let mac = sign_payload(key, &payload_bytes);

    let code = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&payload_bytes),
        URL_SAFE_NO_PAD.encode(mac)
    );
    (code, expires_at)
}

/// Verify an invite code presented to `community`.
///
/// Order matters: signature is checked before any claims inside the payload
/// are trusted (expiry / community / role), and errors after the MAC check
/// still return coarse variants so the endpoint stays a poor oracle.
pub fn verify_invite(
    key: &[u8; 32],
    community: CommunityId,
    code: &str,
) -> Result<InvitePayload, InviteError> {
    if code.len() > MAX_CODE_LEN {
        return Err(InviteError::Malformed);
    }
    let (payload_b64, mac_b64) = code.split_once('.').ok_or(InviteError::Malformed)?;
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| InviteError::Malformed)?;
    let mac_bytes = URL_SAFE_NO_PAD
        .decode(mac_b64)
        .map_err(|_| InviteError::Malformed)?;

    // Constant-time MAC verification before trusting anything in the payload.
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(&payload_bytes);
    mac.verify_slice(&mac_bytes)
        .map_err(|_| InviteError::BadSignature)?;

    let payload: InvitePayload =
        serde_json::from_slice(&payload_bytes).map_err(|_| InviteError::Malformed)?;

    if payload.e < now_unix() {
        return Err(InviteError::Expired);
    }
    if payload.c != community.as_uuid().to_string() {
        return Err(InviteError::WrongCommunity);
    }
    if payload.r != "member" {
        return Err(InviteError::InvalidRole);
    }
    Ok(payload)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_key() -> [u8; 32] {
        derive_invite_key(&nostr::Keys::generate())
    }

    fn community() -> CommunityId {
        CommunityId::from_uuid(Uuid::new_v4())
    }

    #[test]
    fn mint_then_verify_roundtrip() {
        let key = test_key();
        let c = community();
        let (code, expires_at) = mint_invite(&key, c, 3600);
        let payload = verify_invite(&key, c, &code).expect("valid code verifies");
        assert_eq!(payload.c, c.as_uuid().to_string());
        assert_eq!(payload.r, "member");
        assert_eq!(payload.e, expires_at);
    }

    #[test]
    fn rejects_wrong_community() {
        let key = test_key();
        let (code, _) = mint_invite(&key, community(), 3600);
        assert_eq!(
            verify_invite(&key, community(), &code),
            Err(InviteError::WrongCommunity)
        );
    }

    #[test]
    fn rejects_tampered_payload() {
        let key = test_key();
        let c = community();
        let (code, _) = mint_invite(&key, c, 3600);
        let (payload_b64, mac_b64) = code.split_once('.').unwrap();

        // Re-encode a payload with an elevated role but keep the original MAC.
        let mut payload: InvitePayload =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload_b64).unwrap()).unwrap();
        payload.r = "owner".to_string();
        let forged = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap()),
            mac_b64
        );
        assert_eq!(
            verify_invite(&key, c, &forged),
            Err(InviteError::BadSignature)
        );
    }

    #[test]
    fn rejects_wrong_key() {
        let c = community();
        let (code, _) = mint_invite(&test_key(), c, 3600);
        assert_eq!(
            verify_invite(&test_key(), c, &code),
            Err(InviteError::BadSignature)
        );
    }

    #[test]
    fn rejects_expired() {
        let key = test_key();
        let c = community();
        // ttl clamps to 60s minimum, so hand-mint an already-expired payload.
        let payload = InvitePayload {
            c: c.as_uuid().to_string(),
            r: "member".to_string(),
            e: now_unix() - 10,
            n: "n".to_string(),
        };
        let bytes = serde_json::to_vec(&payload).unwrap();
        let mac = sign_payload(&key, &bytes);
        let code = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&bytes),
            URL_SAFE_NO_PAD.encode(mac)
        );
        assert_eq!(verify_invite(&key, c, &code), Err(InviteError::Expired));
    }

    #[test]
    fn rejects_garbage() {
        let key = test_key();
        let c = community();
        assert_eq!(verify_invite(&key, c, ""), Err(InviteError::Malformed));
        assert_eq!(
            verify_invite(&key, c, "not-a-code"),
            Err(InviteError::Malformed)
        );
        // "a" / "b" are not valid base64 payloads.
        assert_eq!(verify_invite(&key, c, "a.b"), Err(InviteError::Malformed));
        let huge = "x".repeat(MAX_CODE_LEN + 1);
        assert_eq!(verify_invite(&key, c, &huge), Err(InviteError::Malformed));
    }

    #[test]
    fn ttl_is_capped() {
        let key = test_key();
        let (_, expires_at) = mint_invite(&key, community(), u64::MAX);
        assert!(expires_at <= now_unix() + MAX_INVITE_TTL_SECS + 5);
    }

    #[test]
    fn signed_role_other_than_member_rejected() {
        // Even a *correctly signed* payload with an elevated role must fail
        // verification (defense against a future buggy mint caller).
        let key = test_key();
        let c = community();
        let payload = InvitePayload {
            c: c.as_uuid().to_string(),
            r: "admin".to_string(),
            e: now_unix() + 3600,
            n: "n".to_string(),
        };
        let bytes = serde_json::to_vec(&payload).unwrap();
        let mac = sign_payload(&key, &bytes);
        let code = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(&bytes),
            URL_SAFE_NO_PAD.encode(mac)
        );
        assert_eq!(verify_invite(&key, c, &code), Err(InviteError::InvalidRole));
    }
}
