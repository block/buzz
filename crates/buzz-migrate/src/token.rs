//! Email magic-link tokens — the email-channel proof primitive.
//!
//! A token attests one thing: *whoever received this token controls the inbox
//! it was mailed to.* Because the operator mails it to the address Slack has on
//! file for a given user, holding the token proves control of that Slack user's
//! email — the identity proof for the email claim channel.
//!
//! # What a token deliberately does NOT carry
//!
//! A token binds only `subject` (`slack:<team id>:<user id>`), an expiry, and a random
//! nonce. It does **not** carry a Buzz pubkey. The pubkey is supplied later, by
//! the app that opens the deep link — i.e. the recipient's own client, using
//! its own key. This closes a phishing-takeover: an attacker who calls
//! `/email/start` for `victim@corp` only causes an email to land in the
//! victim's inbox; if the victim clicks it, the binding is completed with the
//! *victim's* key (from the victim's app), never a key the attacker chose. The
//! token cannot smuggle an attacker pubkey because it holds no pubkey at all.
//!
//! # Residual risk (accepted, standard for magic links)
//!
//! A token is a bearer secret: anyone who reads it before it is used or expires
//! can complete the email proof. Mitigation is the same as every magic-link
//! login — short TTL, single use, and delivery only to the real inbox. Single
//! use is enforced by the caller (see [`ConsumedNonces`]); expiry and integrity
//! are enforced here.

use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

/// A minted, still-opaque token as it travels in a magic-link URL.
///
/// Wire form (all ASCII, URL-safe, `.`-delimited):
/// `v1.<subject hex>.<exp>.<nonce hex>.<mac hex>`
///
/// `subject` is hex-encoded so its bytes can never collide with the `.`
/// delimiter, whatever a foreign workspace allows in a user id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagicToken(String);

impl MagicToken {
    /// The token's wire string (put this in the magic-link `?token=` query).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse a wire string without verifying it. Verification happens in
    /// [`verify`]; this is only for transport.
    pub fn from_wire(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

/// The verified contents of a token: the foreign identity it proves control of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedToken {
    /// `<source>:<foreign id>`, e.g. `slack:T0266FRGM:U060976D0QN`.
    pub subject: String,
    /// Unix seconds after which the token is invalid.
    pub exp: u64,
    /// Per-token random nonce — the single-use key (see [`ConsumedNonces`]).
    pub nonce: String,
}

/// Why a token failed verification. All variants are safe to surface to the
/// clicker (they reveal nothing about the secret).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("malformed token")]
    Malformed,
    #[error("token signature is invalid")]
    BadSignature,
    #[error("token has expired")]
    Expired,
    #[error("token has already been used")]
    AlreadyUsed,
    #[error("token service is unavailable")]
    Unavailable,
}

/// Serializable payload; the exact bytes the MAC is computed over. Field order
/// and JSON shape are the signed message — do not reorder.
#[derive(Serialize, Deserialize)]
struct Payload<'a> {
    v: u8,
    subject: &'a str,
    exp: u64,
    nonce: &'a str,
}

fn mac_hex(secret: &[u8], subject: &str, exp: u64, nonce: &str) -> Result<String, TokenError> {
    // Canonical signed message. `serde_json` on a fixed-shape struct is
    // deterministic here (fixed field order, no maps), so the bytes are stable.
    let payload = Payload {
        v: 1,
        subject,
        exp,
        nonce,
    };
    let msg = serde_json::to_vec(&payload).map_err(|_| TokenError::Unavailable)?;
    let mut mac =
        <HmacSha256 as KeyInit>::new_from_slice(secret).map_err(|_| TokenError::Unavailable)?;
    mac.update(&msg);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

/// Mint a token for `subject`, valid for `ttl_secs` from `now`.
///
/// `secret` is the service's signing key (keep it out of any public event).
/// `nonce_bytes` are fresh random bytes — 16 is plenty; the caller supplies
/// them so this stays pure and testable.
pub fn mint(
    secret: &[u8],
    subject: &str,
    now: u64,
    ttl_secs: u64,
    nonce_bytes: &[u8],
) -> Result<MagicToken, TokenError> {
    let exp = now.saturating_add(ttl_secs);
    let nonce = hex::encode(nonce_bytes);
    let mac = mac_hex(secret, subject, exp, &nonce)?;
    Ok(MagicToken(format!(
        "v1.{}.{}.{}.{}",
        hex::encode(subject.as_bytes()),
        exp,
        nonce,
        mac
    )))
}

/// Verify a token's integrity and expiry (NOT single use — that is
/// [`ConsumedNonces::try_reserve`]). Returns the proven subject on success.
///
/// The MAC comparison is constant-time, so a forger cannot learn the correct
/// signature byte-by-byte from timing.
pub fn verify(secret: &[u8], token: &MagicToken, now: u64) -> Result<VerifiedToken, TokenError> {
    let mut parts = token.0.split('.');
    let (Some(v), Some(subject_hex), Some(exp_s), Some(nonce), Some(mac), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(TokenError::Malformed);
    };
    if v != "v1" {
        return Err(TokenError::Malformed);
    }
    let subject_bytes = hex::decode(subject_hex).map_err(|_| TokenError::Malformed)?;
    let subject = String::from_utf8(subject_bytes).map_err(|_| TokenError::Malformed)?;
    let exp: u64 = exp_s.parse().map_err(|_| TokenError::Malformed)?;
    // Nonce must be hex; reject anything else so the single-use key is clean.
    if nonce.is_empty() || hex::decode(nonce).is_err() {
        return Err(TokenError::Malformed);
    }

    let expected = mac_hex(secret, &subject, exp, nonce)?;
    // Constant-time compare over equal-length hex strings.
    let ok: bool = expected.as_bytes().ct_eq(mac.as_bytes()).into();
    if !ok {
        return Err(TokenError::BadSignature);
    }
    if now >= exp {
        return Err(TokenError::Expired);
    }
    Ok(VerifiedToken {
        subject,
        exp,
        nonce: nonce.to_string(),
    })
}

/// In-memory single-use ledger keyed by token nonce, with lazy expiry so it
/// cannot grow without bound.
///
/// This is process-local. A single claim-service instance is the intended
/// deployment; a multi-instance operator must back single use with shared
/// storage (documented in the service README) or the same token could be
/// redeemed once per instance.
#[derive(Debug, Default)]
pub struct ConsumedNonces {
    /// nonce -> reservation/consumption state and token expiry.
    used: HashMap<String, NonceState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NonceState {
    Reserved { exp: u64 },
    Consumed { exp: u64 },
}

impl NonceState {
    fn exp(self) -> u64 {
        match self {
            Self::Reserved { exp } | Self::Consumed { exp } => exp,
        }
    }
}

impl ConsumedNonces {
    pub fn new() -> Self {
        Self::default()
    }

    /// Atomically reserve a verified token while its relay write is in flight.
    ///
    /// A reservation blocks concurrent redemption, but can be released after a
    /// failed relay write so the legitimate claimant can retry.
    pub fn try_reserve(&mut self, token: &VerifiedToken, now: u64) -> Result<(), TokenError> {
        self.sweep(now);
        if self.used.contains_key(&token.nonce) {
            return Err(TokenError::AlreadyUsed);
        }
        self.used
            .insert(token.nonce.clone(), NonceState::Reserved { exp: token.exp });
        Ok(())
    }

    /// Commit a reservation after the attestation was accepted by the relay.
    pub fn commit(&mut self, token: &VerifiedToken) {
        self.used
            .insert(token.nonce.clone(), NonceState::Consumed { exp: token.exp });
    }

    /// Release an in-flight reservation after a failed relay write.
    pub fn release(&mut self, token: &VerifiedToken) {
        if matches!(
            self.used.get(&token.nonce),
            Some(NonceState::Reserved { .. })
        ) {
            self.used.remove(&token.nonce);
        }
    }

    /// Drop entries whose tokens have expired — a used token past its expiry
    /// can never be presented again validly, so its ledger entry is dead weight.
    fn sweep(&mut self, now: u64) {
        self.used.retain(|_, state| state.exp() > now);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.used.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"test-signing-secret-32-bytes-long!!";
    const NONCE: &[u8] = &[7u8; 16];
    const SUBJECT: &str = "slack:U060976D0QN";

    #[test]
    fn roundtrip_verifies_and_returns_subject() {
        let t = mint(SECRET, SUBJECT, 1_000, 3_600, NONCE).expect("mint");
        let v = verify(SECRET, &t, 1_500).expect("valid");
        assert_eq!(v.subject, SUBJECT);
        assert_eq!(v.exp, 4_600);
    }

    #[test]
    fn wire_form_survives_transport() {
        let t = mint(SECRET, SUBJECT, 1_000, 3_600, NONCE).expect("mint");
        let reparsed = MagicToken::from_wire(t.as_str().to_string());
        assert!(verify(SECRET, &reparsed, 1_500).is_ok());
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let t = mint(SECRET, SUBJECT, 1_000, 3_600, NONCE).expect("mint");
        assert_eq!(
            verify(b"a-different-secret", &t, 1_500),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn tampered_subject_is_rejected() {
        let t = mint(SECRET, SUBJECT, 1_000, 3_600, NONCE).expect("mint");
        // Re-encode a different subject with the original exp/nonce/mac.
        let mut parts: Vec<&str> = t.as_str().split('.').collect();
        let evil = hex::encode("slack:UATTACKER".as_bytes());
        parts[1] = &evil;
        let forged = MagicToken::from_wire(parts.join("."));
        assert_eq!(
            verify(SECRET, &forged, 1_500),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn tampered_expiry_is_rejected() {
        let t = mint(SECRET, SUBJECT, 1_000, 10, NONCE).expect("mint"); // exp = 1010
        let mut parts: Vec<&str> = t.as_str().split('.').collect();
        parts[2] = "9999999999"; // extend expiry
        let forged = MagicToken::from_wire(parts.join("."));
        assert_eq!(
            verify(SECRET, &forged, 1_500),
            Err(TokenError::BadSignature)
        );
    }

    #[test]
    fn expired_token_is_rejected() {
        let t = mint(SECRET, SUBJECT, 1_000, 10, NONCE).expect("mint"); // exp = 1010
        assert_eq!(verify(SECRET, &t, 1_011), Err(TokenError::Expired));
        assert_eq!(
            verify(SECRET, &t, 1_010),
            Err(TokenError::Expired),
            "expiry is an exclusive upper bound"
        );
    }

    #[test]
    fn malformed_tokens_are_rejected() {
        for bad in [
            "",
            "v1",
            "v1.aa.bb",
            "v2.aa.100.bb.cc",           // wrong version
            "v1.zz.100.bb.cc",           // subject not hex
            "v1.aa.notanum.bb.cc",       // exp not numeric
            "v1.616263.100..cc",         // empty nonce
            "v1.616263.100.nothex.cc",   // nonce not hex
            "v1.616263.100.bb.cc.extra", // trailing segment
        ] {
            let r = verify(SECRET, &MagicToken::from_wire(bad), 50);
            assert!(
                matches!(
                    r,
                    Err(TokenError::Malformed) | Err(TokenError::BadSignature)
                ),
                "expected reject for {bad:?}, got {r:?}"
            );
        }
    }

    #[test]
    fn single_use_is_enforced_once() {
        let t = mint(SECRET, SUBJECT, 1_000, 3_600, NONCE).expect("mint");
        let v = verify(SECRET, &t, 1_500).unwrap();
        let mut ledger = ConsumedNonces::new();
        assert_eq!(ledger.try_reserve(&v, 1_500), Ok(()));
        ledger.commit(&v);
        assert_eq!(
            ledger.try_reserve(&v, 1_500),
            Err(TokenError::AlreadyUsed),
            "a token must not redeem twice"
        );
    }

    #[test]
    fn failed_write_can_release_reservation_for_retry() {
        let t = mint(SECRET, SUBJECT, 1_000, 3_600, NONCE).expect("mint");
        let v = verify(SECRET, &t, 1_500).expect("valid");
        let mut ledger = ConsumedNonces::new();

        ledger.try_reserve(&v, 1_500).expect("first reserve");
        assert_eq!(
            ledger.try_reserve(&v, 1_500),
            Err(TokenError::AlreadyUsed),
            "concurrent redemption is blocked"
        );
        ledger.release(&v);
        assert_eq!(
            ledger.try_reserve(&v, 1_500),
            Ok(()),
            "a failed relay write can be retried"
        );
    }

    #[test]
    fn distinct_nonces_are_independent() {
        let a = verify(
            SECRET,
            &mint(SECRET, SUBJECT, 1_000, 3_600, &[1u8; 16]).expect("mint"),
            1_500,
        )
        .unwrap();
        let b = verify(
            SECRET,
            &mint(SECRET, SUBJECT, 1_000, 3_600, &[2u8; 16]).expect("mint"),
            1_500,
        )
        .unwrap();
        let mut ledger = ConsumedNonces::new();
        assert_eq!(ledger.try_reserve(&a, 1_500), Ok(()));
        ledger.commit(&a);
        assert_eq!(
            ledger.try_reserve(&b, 1_500),
            Ok(()),
            "a different token for the same subject is still usable"
        );
    }

    #[test]
    fn ledger_sweeps_expired_entries() {
        let v = verify(
            SECRET,
            &mint(SECRET, SUBJECT, 1_000, 10, NONCE).expect("mint"),
            1_005,
        )
        .unwrap(); // exp 1010
        let mut ledger = ConsumedNonces::new();
        ledger.try_reserve(&v, 1_005).unwrap();
        ledger.commit(&v);
        assert_eq!(ledger.len(), 1);
        // A later consume of some other token triggers a sweep that drops the
        // now-expired entry.
        let other = verify(
            SECRET,
            &mint(SECRET, "slack:U2", 2_000, 10, &[9u8; 16]).expect("mint"),
            2_001,
        )
        .unwrap();
        ledger.try_reserve(&other, 2_001).unwrap();
        assert_eq!(
            ledger.len(),
            1,
            "expired entry swept, only the fresh one left"
        );
    }
}
