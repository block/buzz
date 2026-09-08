//! Privacy-preserving denial contract for NIP-FI (`FI-INV-13`, `FI-TRACE-DENIAL-ORACLE`).
//!
//! Public rejection is many-to-one: a fixed set of four classes, each with
//! byte-exact wire text on every surface where its condition can be decided.
//! Responses reveal no identity, key, claim, binding, tombstone, enrollment
//! mode, or private policy fact. The exact bytes are fixed by
//! [NIP-FI.md](../../../../docs/nips/NIP-FI.md) — the rejection table.
//!
//! This module owns only the closed contract. Each deciding layer maps its
//! private condition onto a [`DenialClass`] and emits these exact bytes:
//! assertion validation ([`super::verifier`]) maps every token rejection to
//! [`DenialClass::EvidenceRejected`]; the client-attached transport maps a
//! missing field to [`DenialClass::MissingEvidence`]; preparation and final
//! admission map private-state denials to [`DenialClass::AuthorizationDenied`];
//! an unreadable authoritative dependency maps to
//! [`DenialClass::AuthorizationUnavailable`].

/// A public NIP-FI denial class. Many private conditions collapse to one class
/// so that a response reveals nothing about the private cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenialClass {
    /// No assertion or proof was supplied. HTTP `401` with a `Nostr` challenge.
    MissingEvidence,
    /// Supplied evidence was malformed, invalid, or expired. HTTP `403`.
    EvidenceRejected,
    /// A private-state denial: replayed evidence, key mismatch, attestation
    /// required, binding conflict, retired pair, revoked key, lifecycle gate,
    /// binding required/expired, or local policy denial. HTTP `403`.
    ///
    /// Every condition in this class produces byte-identical responses so that
    /// resubmitting captured evidence reveals nothing about committed state.
    AuthorizationDenied,
    /// A required current authoritative dependency was unreadable. HTTP `503`.
    /// The sole class that may depend on server state rather than supplied
    /// evidence, and it reveals only unreadability, never a per-principal fact.
    AuthorizationUnavailable,
}

impl DenialClass {
    /// The exact UTF-8 Nostr text carried after an applicable NIP-42/NIP-01
    /// prefix, sent when the denial is decided after a connection exists.
    pub const fn nostr_text(self) -> &'static str {
        match self {
            Self::MissingEvidence => "auth-required: authentication required",
            Self::EvidenceRejected => "restricted: evidence rejected",
            Self::AuthorizationDenied => "restricted: authorization denied",
            Self::AuthorizationUnavailable => "restricted: authorization unavailable",
        }
    }

    /// The HTTP status code sent when the denial is decided on an HTTP request
    /// or a WebSocket upgrade, in place of `101`.
    pub const fn http_status(self) -> u16 {
        match self {
            Self::MissingEvidence => 401,
            Self::EvidenceRejected | Self::AuthorizationDenied => 403,
            Self::AuthorizationUnavailable => 503,
        }
    }

    /// The exact HTTP response body: the shown UTF-8 bytes with one trailing
    /// `LF` and no other bytes.
    pub const fn http_body(self) -> &'static str {
        match self {
            Self::MissingEvidence => "authentication required\n",
            Self::EvidenceRejected => "evidence rejected\n",
            Self::AuthorizationDenied => "authorization denied\n",
            Self::AuthorizationUnavailable => "authorization unavailable\n",
        }
    }

    /// The `WWW-Authenticate` challenge value, present only for
    /// [`Self::MissingEvidence`]. The `Nostr` challenge satisfies RFC 9110
    /// Section 15.5.2.
    pub const fn www_authenticate(self) -> Option<&'static str> {
        match self {
            Self::MissingEvidence => Some("Nostr"),
            _ => None,
        }
    }

    /// The `Content-Type` header value, identical across all classes.
    pub const fn content_type(self) -> &'static str {
        "text/plain; charset=utf-8"
    }
}
