//! Typed error for admin mutation commands.

/// Error from an admin mutation command, carrying whether the relay
/// authoritatively answered so the UI can decide idempotency-retry policy
/// without string-matching the message.
///
/// `relayStatus` is `Some(code)` only when the relay returned an HTTP status —
/// the request reached the relay and it answered. It is `None` for a
/// pre-response transport failure (`send()` error, DNS/connect/timeout) or a
/// pre-send failure (auth build, body serialisation): the relay never
/// committed anything, so a retry must reuse the same idempotency key.
///
/// `bodyComplete` is `true` only when the relay's full response body was read —
/// an authoritative verdict. A non-409 4xx with `bodyComplete: true` is a
/// definitive pre-commit rejection, so the UI may mint a fresh idempotency key.
/// A status that arrives but whose body is lost mid-stream (or rejected over
/// the size cap) carries `bodyComplete: false`: the outcome is unknown, so the
/// caller preserves idempotency and lets the retry dedupe even on a 4xx.
///
/// `notSent` is `true` only when the desktop refused the request before sending
/// it (a malformed intent, or the relay or signer changed): nothing reached the
/// relay, so retrying the same intent can never succeed.
///
/// Serialises `rename_all = "camelCase"`; the JS bridge surfaces it as the
/// rejected `TauriInvokeError.payload`, from which the UI reads `relayStatus`
/// and `bodyComplete`.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminMutationError {
    /// Human-readable message — byte-identical to the string the command
    /// produced before typing, so existing message parsing is unaffected.
    pub message: String,
    /// The relay's HTTP status when a response was received; `None` for a
    /// transport/pre-send failure where no relay answer exists.
    pub relay_status: Option<u16>,
    /// Whether the relay's full response body was read. `true` only for an
    /// authoritative verdict; `false` when the body was lost or truncated.
    pub body_complete: bool,
    /// Refused before sending; see the type docs.
    pub not_sent: bool,
    /// The complete error body had zero bytes: a relay without the route
    /// answers this way. Derived from the received bytes, never the message.
    pub body_empty: bool,
}

impl AdminMutationError {
    /// Refused before any request was sent; retrying the same intent is futile.
    pub(super) fn not_sent(message: String) -> Self {
        Self {
            not_sent: true,
            ..Self::from(message)
        }
    }

    /// The relay answered with an HTTP status and its full body was read — an
    /// authoritative verdict.
    pub(super) fn authoritative(
        status: reqwest::StatusCode,
        message: String,
        body_empty: bool,
    ) -> Self {
        Self {
            message,
            relay_status: Some(status.as_u16()),
            body_complete: true,
            not_sent: false,
            body_empty,
        }
    }

    /// The relay answered with an HTTP status but the body was not fully read
    /// (redirect, over-cap, or a mid-stream read failure) — outcome unknown.
    pub(super) fn partial(status: reqwest::StatusCode, message: String) -> Self {
        Self {
            message,
            relay_status: Some(status.as_u16()),
            body_complete: false,
            not_sent: false,
            body_empty: false,
        }
    }
}

/// Pre-send and transport failures carry no relay status: the relay never saw
/// the request (or never answered), so the outcome is unambiguously "no commit".
impl From<String> for AdminMutationError {
    fn from(message: String) -> Self {
        Self {
            message,
            relay_status: None,
            body_complete: false,
            not_sent: false,
            body_empty: false,
        }
    }
}

/// Error from a staff read command (`/communities`, `/members/...`,
/// `/events/{id}`), keeping what the UI needs to tell "this relay has no such
/// route" (complete empty 404/405), "not in this community" (a coded 404) and
/// "unknown" (truncated body or no answer) apart without reading the message.
#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminReadError {
    pub message: String,
    /// The relay's HTTP status; `None` when no response arrived.
    pub relay_status: Option<u16>,
    /// Whether the whole error body was read.
    pub body_complete: bool,
    /// The complete body had zero bytes (false whenever it is incomplete).
    pub body_empty: bool,
    /// The relay envelope's `error.code`, when the complete body carried one.
    pub code: Option<String>,
}

impl AdminReadError {
    /// The relay answered but its body was not fully read.
    pub(super) fn partial(status: reqwest::StatusCode, message: String) -> Self {
        Self {
            relay_status: Some(status.as_u16()),
            ..Self::from(message)
        }
    }
}

/// Pre-send and transport failures: no relay answer.
impl From<String> for AdminReadError {
    fn from(message: String) -> Self {
        Self {
            message,
            relay_status: None,
            body_complete: false,
            body_empty: false,
            code: None,
        }
    }
}
