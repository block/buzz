//! Wire-side mapping for [`buzz_auth::AuthError`] → user-visible response.
//!
//! ## Why this exists
//!
//! `AuthError` is a rich operational enum — variants like `Internal(String)`
//! and `Nip98Invalid(String)` carry detail that is useful in server logs and
//! ruinous on the wire. Two specific risks the multi-tenant rewrite added:
//!
//! 1. **Key-prefix leak via `Internal(_)`.** The pubsub adapters wrap raw
//!    `redis::RedisError` values into `AuthError::Internal(format!("Redis ...: {e}"))`
//!    after touching community-prefixed keys (`buzz:{community}:nip98:{id}`,
//!    `buzz:{community}:ratelimit:{hex}:{kind}`). `RedisError::Display` is not
//!    guaranteed key-free across variants — `ResponseError`, `MOVED`/`ASK`,
//!    cluster redirects routinely include command/key context. Forwarding the
//!    chain to a client turns it into a cross-tenant existence oracle on Redis
//!    keys.
//! 2. **Variant-distinguishable replay reply.** Returning a distinct error for
//!    `Nip98Replay` tells the attacker the event id has been seen in *this*
//!    community (the seen-set is community-scoped per the S1 isolation fence).
//!    Even with zero key text leaked, a distinguishable reply is a presence
//!    oracle on community-scoped activity for any guessed/sniffed event id.
//!
//! See `RESEARCH/RELAY_REWRITE_AUTH_ERROR_ORACLE_AUDIT.md` for the full
//! payload-policy decisions (P1–P5).
//!
//! ## Contract
//!
//! All wire-side conversions of an `AuthError` go through
//! [`auth_error_wire`]. The returned [`AuthErrorWireCategory`] is the only
//! shape that may reach a client. The mapper deliberately collapses
//! `Nip98Invalid` and `Nip98Replay` to the same category (P2) and maps
//! `Internal(_)` to `InternalRedacted` (P1) — the original detail is logged
//! at the construction site, not re-emitted here.

use axum::{http::StatusCode, response::Json};
use buzz_auth::AuthError;
use serde_json::json;

/// User-visible category for an [`AuthError`] on the wire.
///
/// The discriminants are user-visible. Each maps to a fixed byte sequence on
/// each protocol surface (HTTP body, WS NOTICE). Adding a variant means
/// adding a new oracle-channel decision; do not add a variant without a
/// matching audit note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthErrorWireCategory {
    /// NIP-42 / NIP-98 verification failed for ANY reason (signature, replay,
    /// timestamp, pubkey, malformed event). Collapsed to one category to
    /// avoid a replay-vs-invalid oracle.
    AuthFailed,
    /// The authenticated identity lacks the required scope.
    InsufficientScope,
    /// The authenticated identity is not a member of the requested channel.
    ChannelAccessDenied,
    /// A server-side error occurred. No detail is exposed (P1).
    InternalRedacted,
}

impl AuthErrorWireCategory {
    /// Stable HTTP status code for this category.
    pub fn status_code(self) -> StatusCode {
        match self {
            Self::AuthFailed => StatusCode::UNAUTHORIZED,
            Self::InsufficientScope => StatusCode::FORBIDDEN,
            Self::ChannelAccessDenied => StatusCode::FORBIDDEN,
            Self::InternalRedacted => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable, byte-identical user-visible message for this category.
    ///
    /// Two `AuthError` values that map to the same category MUST produce the
    /// same bytes here — that's the property-test invariant proved in the
    /// in-module `#[cfg(test)] mod tests` below (see
    /// `verification_class_all_coalesce` and `internal_two_communities_byte_identical`).
    pub fn message(self) -> &'static str {
        match self {
            // Coalesces Nip98Invalid, Nip98Replay, InvalidSignature,
            // ChallengeMismatch, RelayUrlMismatch, EventExpired,
            // PubkeyMismatch — all "the auth artifact didn't verify."
            Self::AuthFailed => "authentication failed",
            Self::InsufficientScope => "insufficient scope",
            Self::ChannelAccessDenied => "channel access denied",
            Self::InternalRedacted => "internal error",
        }
    }

    /// Render this category as a standard JSON HTTP error response.
    pub fn http_response(self) -> (StatusCode, Json<serde_json::Value>) {
        (self.status_code(), Json(json!({ "error": self.message() })))
    }

    /// Render this category as a Nostr-protocol NOTICE message body.
    ///
    /// Returns the bare string (without the `["NOTICE", ...]` envelope) so
    /// the caller can decide whether to wrap with `RelayMessage::notice` or
    /// embed in an OK/CLOSED frame.
    pub fn notice_body(self) -> String {
        format!("error: {}", self.message())
    }
}

/// Map an [`AuthError`] to its wire category.
///
/// **Do not stringify `AuthError` for the wire by any other path.** This
/// function is the only sanctioned conversion. The load-bearing fence is
/// the wildcard-free match below: adding any new `AuthError` variant fails
/// to compile here until a wire-class decision is made — that catches the
/// cause (a new variant lacks a wire class), not just the symptom (raw
/// stringification). A `scripts/check-no-authror-leak.sh` grep-lint (P5 in
/// the audit note) is planned as a belt-and-suspenders follow-up but is
/// not yet wired into CI.
pub fn auth_error_wire(err: &AuthError) -> AuthErrorWireCategory {
    match err {
        // Verification class — coalesce.
        AuthError::InvalidSignature
        | AuthError::ChallengeMismatch
        | AuthError::RelayUrlMismatch
        | AuthError::EventExpired
        | AuthError::Nip98Invalid(_)
        | AuthError::Nip98Replay
        | AuthError::PubkeyMismatch => AuthErrorWireCategory::AuthFailed,

        AuthError::InsufficientScope { .. } => AuthErrorWireCategory::InsufficientScope,
        AuthError::ChannelAccessDenied => AuthErrorWireCategory::ChannelAccessDenied,

        // Internal class — never leak detail. The construction site logs the
        // detail; the wire only sees the category.
        AuthError::Internal(_) => AuthErrorWireCategory::InternalRedacted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::response::IntoResponse;
    use buzz_auth::AuthError;

    /// Render a wire category as the exact bytes a client would observe on
    /// the HTTP path. Used for byte-identity assertions across variants.
    async fn http_bytes(cat: AuthErrorWireCategory) -> (u16, Vec<u8>) {
        let (status, json) = cat.http_response();
        let resp = (status, json).into_response();
        let status = resp.status().as_u16();
        let body = to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("body")
            .to_vec();
        (status, body)
    }

    /// P2: Nip98Invalid and Nip98Replay collapse to one wire category.
    #[tokio::test]
    async fn replay_indistinguishable_from_invalid_on_wire() {
        let invalid = auth_error_wire(&AuthError::Nip98Invalid("garbage".into()));
        let replay = auth_error_wire(&AuthError::Nip98Replay);
        assert_eq!(invalid, replay);

        let (invalid_status, invalid_body) = http_bytes(invalid).await;
        let (replay_status, replay_body) = http_bytes(replay).await;
        assert_eq!(invalid_status, replay_status);
        assert_eq!(
            invalid_body, replay_body,
            "Nip98Invalid and Nip98Replay MUST produce byte-identical responses (P2)"
        );
    }

    /// P2 extended: all verification-class errors coalesce to one category.
    #[tokio::test]
    async fn verification_class_all_coalesce() {
        let errs = [
            AuthError::InvalidSignature,
            AuthError::ChallengeMismatch,
            AuthError::RelayUrlMismatch,
            AuthError::EventExpired,
            AuthError::Nip98Invalid("any detail".into()),
            AuthError::Nip98Replay,
            AuthError::PubkeyMismatch,
        ];
        let cats: Vec<_> = errs.iter().map(auth_error_wire).collect();
        for c in &cats {
            assert_eq!(*c, AuthErrorWireCategory::AuthFailed);
        }
        // Body+status all identical.
        let first = http_bytes(cats[0]).await;
        for c in cats {
            assert_eq!(http_bytes(c).await, first);
        }
    }

    /// P1: Internal(_) NEVER carries detail to the wire, regardless of the
    /// string. Synthesizes a worst-case payload that mimics a leaked Redis
    /// error containing a community-prefixed key, and asserts the wire body
    /// does not echo any byte of it.
    #[tokio::test]
    async fn internal_redacted_does_not_leak_inner_string() {
        let leaky =
            "Redis SET NX EX: ResponseError: WRONGTYPE on key buzz:00112233-4455-6677-8899-aabbccddeeff:nip98:deadbeef";
        let err = AuthError::Internal(leaky.into());
        let cat = auth_error_wire(&err);
        assert_eq!(cat, AuthErrorWireCategory::InternalRedacted);

        let (status, body) = http_bytes(cat).await;
        assert_eq!(status, 500);
        let body_str = std::str::from_utf8(&body).expect("body utf8");
        // The category-only response must NOT contain any byte of the inner
        // detail — not the key prefix, not the community UUID, not "Redis".
        assert!(
            !body_str.contains("buzz:"),
            "wire body leaks community key prefix: {body_str}"
        );
        assert!(
            !body_str.contains("nip98"),
            "wire body leaks Redis key shape: {body_str}"
        );
        assert!(
            !body_str.contains("Redis"),
            "wire body leaks Redis driver string: {body_str}"
        );
        assert!(
            !body_str.contains("00112233"),
            "wire body leaks community UUID: {body_str}"
        );
    }

    /// P1: two distinct Internal(_) values for two distinct communities
    /// produce byte-identical wire responses. Proves the wire shape is
    /// independent of the inner detail (closes the cross-tenant oracle).
    #[tokio::test]
    async fn internal_two_communities_byte_identical() {
        let community_a = "Redis pool: SET buzz:aaaaaaaa-1111-2222-3333-444444444444:nip98:cafe";
        let community_b = "Redis pool: SET buzz:bbbbbbbb-5555-6666-7777-888888888888:nip98:beef";
        let cat_a = auth_error_wire(&AuthError::Internal(community_a.into()));
        let cat_b = auth_error_wire(&AuthError::Internal(community_b.into()));
        assert_eq!(cat_a, cat_b);
        assert_eq!(http_bytes(cat_a).await, http_bytes(cat_b).await);
    }

    /// Scope and channel errors are distinct from auth-failed — they are
    /// authorization-class outcomes that the client must distinguish (different
    /// remediation: re-auth vs re-request access vs join channel). These do
    /// NOT carry tenant-scoped detail in the variant, so they're safe to
    /// distinguish. (If that ever changes, this test fails by design.)
    #[test]
    fn authorization_class_distinguishable() {
        assert_eq!(
            auth_error_wire(&AuthError::InsufficientScope {
                required: "write".into(),
                have: vec![]
            }),
            AuthErrorWireCategory::InsufficientScope
        );
        assert_eq!(
            auth_error_wire(&AuthError::ChannelAccessDenied),
            AuthErrorWireCategory::ChannelAccessDenied
        );
        assert_ne!(
            AuthErrorWireCategory::AuthFailed,
            AuthErrorWireCategory::InsufficientScope
        );
        assert_ne!(
            AuthErrorWireCategory::AuthFailed,
            AuthErrorWireCategory::ChannelAccessDenied
        );
    }

    /// NOTICE body parity: every category produces a NOTICE that does not
    /// leak inner detail, and verification-class NOTICEs are byte-identical.
    #[test]
    fn ws_notice_redacts_internal_and_coalesces_verification() {
        let leaky =
            AuthError::Internal("buzz:abcd:nip98:dead — should never appear on wire".into());
        let notice = auth_error_wire(&leaky).notice_body();
        assert!(
            !notice.contains("buzz:"),
            "NOTICE leaks key prefix: {notice}"
        );
        assert!(
            !notice.contains("nip98"),
            "NOTICE leaks Redis key: {notice}"
        );

        let invalid = auth_error_wire(&AuthError::Nip98Invalid("x".into())).notice_body();
        let replay = auth_error_wire(&AuthError::Nip98Replay).notice_body();
        assert_eq!(
            invalid, replay,
            "WS NOTICE for invalid and replay MUST be byte-identical"
        );
    }
}
