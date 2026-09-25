//! NIP-FI HTTP ingress enforcement.
//!
//! Every protected HTTP surface in enforce mode MUST call
//! [`admit_nip_fi_http`] (or its state-convenience wrapper
//! [`admit_nip_fi_http_on_state`]) which is the single authority for the
//! complete NIP-FI admission decision for one HTTP request:
//!
//! 1. Run the caller's NIP-98 extraction closure → `proven_pubkey`.
//! 2. Extract the `Nostr-Federated-Identity: Bearer <JWS>` assertion.
//! 3. Verify it offline against the configured issuer JWKS.
//! 4. Confirm the assertion's `nostr_pubkey` equals `proven_pubkey`. [FI-INV-05]
//! 5. Check the deny map for the proven pubkey. [FI-INV-14]
//!
//! HTTP is sessionless: every request re-verifies.  There is no lifetime-
//! partition concept — the session-bounds section of NIP-FI.md is WS-only.
//!
//! ## Structural authority
//!
//! [`NipFiAdmission`] has a private constructor.  The only way to produce
//! one is via [`admit_nip_fi_http`].  This does not force a handler to call
//! it.  In Enforce, a handler that skips the call and does its own NIP-98 is
//! still subject to the router's assertion guard, but a request with a valid
//! assertion passes without key pairing or a deny-map check.  (Off skips the
//! guard entirely; DenyProtected denies without verifying.)
//!
//! ## Carrier / precedence
//!
//! Per NIP-FI.md §Client-attached transport:
//! - Assertion: `Nostr-Federated-Identity: Bearer <compact-JWS>` (this
//!   module's responsibility).
//! - Nostr proof: `Authorization: Nostr <base64-event>` (NIP-98, owned by
//!   the NIP-98 closure passed to `admit_nip_fi_http`).
//! - `Authorization` is RESERVED for NIP-98; the assertion MUST NOT appear
//!   there.  Mixing the two fields is an `EvidenceRejected` (403) denial.
//!
//! ## Deny map
//!
//! HTTP admission reads the relay's single shared [`buzz_auth::NipFiDenyMap`]
//! (`AppState::nip_fi_deny_map`) — the same `Arc` the admin disconnect endpoint
//! writes and WebSocket admission reads — so one deny is enforced on every
//! ingress.
//!
//! ## Off-mode regression
//!
//! When `NipFiMode::Off`, `admit_nip_fi_http` still calls the NIP-98 closure
//! (preserving whatever auth the surface required before NIP-FI), then returns
//! `Ok(NipFiAdmission { assertion: None, ... })` immediately without the
//! assertion/pairing/deny steps.  Pre-NIP-FI behavior is fully preserved for
//! OSS deployments.
//!
//! [FI-TRACE-DENIAL-ORACLE]: exact HTTP response bytes are fixed in NIP-FI.md.
//! [FI-TRACE-TRANSPORT-CLOSED]: assertion transport is exactly one header.
//! [FI-TRACE-AUTHORITY-UNIFORM]: all protected surfaces call this function.

use axum::{
    body::Body,
    http::{HeaderMap, Response, StatusCode},
};
use buzz_auth::{
    DenialClass, NipFiMode, VerifiedAssertion, VerifyAssertion, CLIENT_ATTACHED_HEADER,
};
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use std::fmt;

// ── Deny-map seam ─────────────────────────────────────────────────────────────

/// Narrow interface consumed by HTTP enforcement.  Production implements it
/// for the shared [`buzz_auth::NipFiDenyMap`]; tests may substitute fixtures.
///
/// `(issuer, pubkey, now)` are required because the deny set is issuer-
/// scoped per `NIP-FI.md:624-627`.  Passing only pubkey would collide
/// across issuers — a deny for `(iss-A, k)` must not block `(iss-B, k)`.
///
/// Sealed: only implementations in this crate are accepted.
pub(crate) trait HttpDenyMap: sealed::Sealed {
    /// Returns `true` when `(issuer, pubkey)` has an active deny entry at
    /// `now` (`now < until`).  Implementations fail closed: a poisoned or
    /// unavailable backing store returns `true` (deny).
    fn is_denied(&self, issuer: &str, pubkey: &PublicKey, now: DateTime<Utc>) -> bool;
}

pub(crate) mod sealed {
    pub(crate) trait Sealed {}
}

impl sealed::Sealed for buzz_auth::NipFiDenyMap {}
impl HttpDenyMap for buzz_auth::NipFiDenyMap {
    /// Delegates to [`buzz_auth::NipFiDenyMap::is_denied`], which is issuer-
    /// scoped and returns `true` on a poisoned shard.
    fn is_denied(&self, issuer: &str, pubkey: &PublicKey, now: DateTime<Utc>) -> bool {
        buzz_auth::NipFiDenyMap::is_denied(self, issuer, pubkey, now)
    }
}

/// The deny map for a relay that has no `nip_fi_deny_map`.
///
/// The map is absent only when no command-capable issuer is configured (mode
/// `Off`, or before startup installs the command components).  Deny entries
/// are written solely into that map — by the admin disconnect endpoint and the
/// cross-pod consumer, both of which are disabled without it — so no entry can
/// exist and admitting is exact, not fail-open.
struct NoDenyMapConfigured;
impl sealed::Sealed for NoDenyMapConfigured {}
impl HttpDenyMap for NoDenyMapConfigured {
    fn is_denied(&self, _issuer: &str, _pubkey: &PublicKey, _now: DateTime<Utc>) -> bool {
        false
    }
}

// ── Admission type ────────────────────────────────────────────────────────────

/// Opaque NIP-98 proof produced by a NIP-98 extraction closure.
///
/// The `pubkey` field is private to this module.  Code that calls
/// `bridge::make_nip98_closure_for_admission`—or any other closure that yields
/// this type—cannot read the proven key directly; it must pass the closure to
/// [`admit_nip_fi_http`], which opens the proof internally and returns the key
/// only through the private-constructor `NipFiAdmission`.
///
/// ## Falsifier
///
/// Invoking the closure directly (`make_nip98_closure_for_admission(...)()`)
/// returns `Ok(Nip98Proof { .. })`.  Without the private `pubkey` accessor,
/// the call site cannot project the key — any attempt to destructure or call
/// `.pubkey` fails to compile.
///
/// `X` is caller-supplied side-data (e.g. replay-detection fields).
pub(crate) struct Nip98Proof<X = ()> {
    /// Private: only `admit_nip_fi_http` may read this field.
    pubkey: PublicKey,
    /// Side-data threaded through from the extraction closure.
    pub(crate) extra: X,
}

impl<X> Nip98Proof<X> {
    /// Construct a proof.  `pub(crate)` so that both bridge-internal closures
    /// and the media/git surfaces (which already hold a proven pubkey from a
    /// prior extractor) can build the token without leaking the key.
    pub(crate) fn new(pubkey: PublicKey, extra: X) -> Self {
        Self { pubkey, extra }
    }
}

/// Proof that the mode-appropriate NIP-FI admission path completed for one
/// HTTP request.
///
/// Construction is private to [`admit_nip_fi_http`].  **No other code path
/// produces this type.**  A value means the path for the configured mode ran:
///
///   Off:     NIP-98 extraction → admit (`assertion: None`, no pairing)
///   Enforce: NIP-98 extraction → assertion extraction → verify → pair →
///            deny-map → admit
///
/// DenyProtected never produces one.  Key pairing is guaranteed only in
/// Enforce.
///
/// `X` is caller-supplied side-data returned by the NIP-98 extraction closure
/// (e.g. replay-detection fields).  Use `()` when no side-data is needed.
///
/// [FI-TRACE-AUTHORITY-UNIFORM] Every protected HTTP surface produces this
/// type via `admit_nip_fi_http`; there is no other source.
#[must_use]
pub(crate) struct NipFiAdmission<X = ()> {
    /// The pubkey proven by NIP-98 (and, in Enforce, confirmed by assertion
    /// pairing).
    ///
    /// Private: obtain via [`NipFiAdmission::proven_pubkey`].
    /// Only set from within [`admit_nip_fi_http`].
    proven_pubkey: PublicKey,
    /// The verified federation assertion (Some in Enforce mode, None in Off).
    assertion: Option<VerifiedAssertion>,
    /// Caller-supplied side-data from the NIP-98 extraction closure.
    extra: X,
}

impl<X> fmt::Debug for NipFiAdmission<X> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NipFiAdmission")
            .field("proven_pubkey", &self.proven_pubkey)
            .field("assertion", &self.assertion)
            .finish_non_exhaustive()
    }
}

impl<X> NipFiAdmission<X> {
    /// The pubkey proven by NIP-98 (and, in Enforce, by assertion pairing).
    ///
    /// This is the only way to obtain an authoritative pubkey for downstream
    /// authorization checks.  It is equal to the NIP-98 `pubkey` (what the
    /// request proved); in Enforce it also equals the assertion's
    /// `nostr_pubkey` (what the federation identity bound).
    pub(crate) fn proven_pubkey(&self) -> &PublicKey {
        &self.proven_pubkey
    }

    /// The verified federation assertion, if NIP-FI was in Enforce mode.
    ///
    /// `None` in Off mode — the assertion was not required.
    #[allow(dead_code)]
    pub(crate) fn assertion(&self) -> Option<&VerifiedAssertion> {
        self.assertion.as_ref()
    }

    /// Caller-supplied side-data from the NIP-98 extraction closure.
    #[allow(dead_code)]
    pub(crate) fn extra(&self) -> &X {
        &self.extra
    }

    /// Consume the admission, returning ownership of the side-data.
    pub(crate) fn into_extra(self) -> X {
        self.extra
    }
}

// ── Main admission function ───────────────────────────────────────────────────

/// Run the full NIP-FI admission sequence for one HTTP request.
///
/// ## Sequence (per NIP-FI.md §Admission procedure)
///
/// 1. DenyProtected mode: unconditional 503, before the `Authorization`
///    cardinality check, the NIP-98 closure, or the verifier run.
/// 2. Enforce mode: reject more than one `Authorization` field (403).
/// 3. Run `extract_nip98` — the caller's NIP-98 extraction closure.  Returns
///    `(proven_pubkey, X)` on success, or a `Response` to emit on failure.
///    Off mode returns `Ok(NipFiAdmission { proven_pubkey, assertion: None,
///    extra: X })` here; Off-mode behavior is identical to pre-NIP-FI (no
///    assertion requirement).  [FI-INV-15]
/// 4. Enforce mode: extract `Nostr-Federated-Identity: Bearer <JWS>`.
/// 5. Verify assertion (signature, issuer, expiry, claims).
/// 6. Assert `assertion.asserted_key == proven_pubkey`.  [FI-INV-05]
/// 7. Check deny map for `(iss, proven_pubkey)`.  [FI-INV-14]
/// 8. Return `Ok(NipFiAdmission { proven_pubkey, assertion: Some(...), extra: X })`.
///
/// ## NIP-98 failure remapping in Enforce mode
///
/// When the NIP-98 closure fails in Enforce mode, the closure typically returns
/// a legacy JSON 401/403 (`api_error`).  NIP-FI.md §Admission procedure step 3
/// requires NIP-FI DenialClass responses instead:
/// - Absent `Authorization` header → `MissingEvidence` (401 "authentication required")
/// - Present but malformed/invalid `Authorization` → `EvidenceRejected` (403)
///
/// In Off mode the legacy response is returned unchanged ([FI-INV-15]).
/// [FI-TRACE-DENIAL-ORACLE]
///
/// ## What the private constructor guarantees
///
/// [`NipFiAdmission`] has a private constructor, so the only source of a
/// `NipFiAdmission` value is this function.  It does not force a handler to
/// call this function.  In Enforce, a handler that skips it and runs its own
/// NIP-98 is still subject to the router's assertion guard, but a request with
/// a valid assertion passes without key pairing or a deny-map check.  Off skips
/// the guard entirely; DenyProtected denies without verifying.
///
/// ## Off-mode semantics
///
/// In Off mode the NIP-98 closure is always called (step 3).  In Off mode the closure
/// result still gates entry — if NIP-98 auth is required for non-NIP-FI
/// reasons (e.g. `require_auth_token`), the closure encodes that.  NIP-FI
/// layers (assertion/pairing/deny) are skipped entirely.
///
/// [FI-TRACE-AUTHORITY-UNIFORM]
// Response<Body> is intentionally large (axum's design); boxing it here
// would add allocation without architectural benefit. The large Err variant
// is load-bearing: it IS the HTTP response, returned directly by handlers.
#[allow(clippy::result_large_err)]
pub(crate) fn admit_nip_fi_http<D, X, F>(
    headers: &HeaderMap,
    extract_nip98: F,
    verifier: Option<&dyn VerifyAssertion>,
    mode: NipFiMode,
    deny_map: &D,
) -> Result<NipFiAdmission<X>, Response<Body>>
where
    D: HttpDenyMap,
    F: FnOnce() -> Result<Nip98Proof<X>, Response<Body>>,
{
    // Step 1 — DenyProtected mode: unconditional 503.  Checked first so no
    // request shape (duplicate, missing, or invalid `Authorization`) can turn
    // it into a 401/403, and so neither NIP-98 nor the verifier runs.
    if matches!(mode, NipFiMode::DenyProtected) {
        return Err(http_denial(DenialClass::AuthorizationUnavailable));
    }

    // Step 2 — cardinality gate: Enforce mode requires exactly one Authorization
    // field per NIP-FI.md:695-700.  Off mode preserves legacy first-value behavior
    // (`.get()` silently takes the first) so no regression for Off deployments.
    //
    // Axum / hyper de-duplicates most header fields during HTTP/1.1 parsing, but
    // RFC 7230 permits comma-separated combining or multiple header lines;
    // `HeaderMap::get` silently takes only the FIRST value.  Rejecting duplicates
    // closes the attack where a relay-aware adversary slips a second credential
    // past the NIP-98 verifier.  [FI-INV-15]
    if !matches!(mode, NipFiMode::Off) {
        let auth_count = headers.get_all("authorization").iter().count();
        if auth_count > 1 {
            return Err(http_denial(DenialClass::EvidenceRejected));
        }
    }

    // Step 3: run NIP-98 extraction (Off and Enforce).
    let nip98_result = extract_nip98();

    // Off mode: NIP-FI not required.  Return admission immediately.
    // The NIP-98 closure already enforced whatever auth the surface required.
    // [FI-INV-15 exemption]
    if matches!(mode, NipFiMode::Off) {
        // Off mode: propagate the closure result unchanged (legacy behavior).
        let Nip98Proof {
            pubkey: proven_pubkey,
            extra,
        } = nip98_result?;
        return Ok(NipFiAdmission {
            proven_pubkey,
            assertion: None,
            extra,
        });
    }

    // Enforce mode: NIP-98 closure failure MUST
    // produce a NIP-FI DenialClass response, not a legacy JSON error.
    // [NIP-FI.md §Admission procedure step 3; FI-TRACE-DENIAL-ORACLE]
    let Nip98Proof {
        pubkey: proven_pubkey,
        extra,
    } = nip98_result.map_err(|_legacy| {
        // Determine the appropriate denial class from Authorization header presence.
        // Absent header → MissingEvidence (401); present-but-invalid → EvidenceRejected (403).
        // [FI-TRACE-DENIAL-ORACLE]
        let class = if headers.contains_key("authorization") {
            DenialClass::EvidenceRejected
        } else {
            DenialClass::MissingEvidence
        };
        http_denial(class)
    })?;

    // Steps 4–8 — Enforce mode.

    // Step 4: extract the assertion token.
    let token = extract_bearer_token(headers).map_err(http_denial)?;

    // Step 5: cryptographic verification (signature, issuer, expiry, claims).
    let verifier = verifier.ok_or_else(|| {
        // Verifier not yet constructed (startup race); fail closed.
        http_denial(DenialClass::AuthorizationUnavailable)
    })?;
    let assertion = verifier.verify_assertion(token).map_err(|e| {
        tracing::debug!(code = e.code(), "nip-fi assertion denied at http ingress");
        http_denial(e.denial_class())
    })?;

    // Step 6: key pairing — assertion.asserted_key MUST equal proven NIP-98 key.
    // A claimless assertion (no nostr_pubkey) is also a denial.  [FI-INV-05]
    match assertion.asserted_key() {
        Some(k) if k == proven_pubkey => {}
        _ => {
            metrics::counter!(
                "buzz_auth_failures_total",
                "reason" => "nip_fi_http_key_mismatch"
            )
            .increment(1);
            tracing::debug!(
                proven = %proven_pubkey.to_hex(),
                "NIP-FI HTTP key pairing mismatch"
            );
            // Key mismatch is a private-state denial: authorization_denied (403).
            // [FI-TRACE-DENIAL-ORACLE]
            return Err(http_denial(DenialClass::AuthorizationDenied));
        }
    }

    // Step 7: deny-map check — (iss, pubkey) must not be in an active deny window.
    // [FI-INV-14] [NIP-FI.md:624-627]
    let issuer = assertion.identity().issuer();
    if deny_map.is_denied(issuer, &proven_pubkey, Utc::now()) {
        metrics::counter!(
            "buzz_auth_failures_total",
            "reason" => "nip_fi_http_denied_pubkey"
        )
        .increment(1);
        // Denied-pubkey is a private-state denial.  [FI-TRACE-DENIAL-ORACLE]
        return Err(http_denial(DenialClass::AuthorizationDenied));
    }

    // Step 8: admit.
    Ok(NipFiAdmission {
        proven_pubkey,
        assertion: Some(assertion),
        extra,
    })
}

// ── Transport extraction ──────────────────────────────────────────────────────

/// Extract the single `Bearer <token>` from the `Nostr-Federated-Identity`
/// header.
///
/// Rejects all forms the spec prohibits:
/// - Absent → `MissingEvidence`
/// - Repeated (multiple header values) → `EvidenceRejected`
/// - Comma-combined (`,` in a single value) → `EvidenceRejected`
/// - Empty after `Bearer ` stripping → `EvidenceRejected`
/// - Non-`Bearer ` prefix → `EvidenceRejected`
/// - Whitespace in the token (after scheme) → `EvidenceRejected`
///
/// [FI-TRACE-TRANSPORT-CLOSED]
pub(crate) fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, DenialClass> {
    let mut values = headers.get_all(CLIENT_ATTACHED_HEADER).iter();
    let first = match values.next() {
        Some(v) => v,
        None => return Err(DenialClass::MissingEvidence),
    };
    // Repeated header fields deny. [FI-TRACE-TRANSPORT-CLOSED]
    if values.next().is_some() {
        return Err(DenialClass::EvidenceRejected);
    }
    let raw = first.to_str().map_err(|_| DenialClass::EvidenceRejected)?;
    // Comma-combined values deny.
    if raw.contains(',') {
        return Err(DenialClass::EvidenceRejected);
    }
    let token = raw
        .strip_prefix("Bearer ")
        .ok_or(DenialClass::EvidenceRejected)?;
    // Empty or whitespace-containing token denies.
    if token.is_empty() || token.contains(ascii_whitespace) {
        return Err(DenialClass::EvidenceRejected);
    }
    Ok(token)
}

fn ascii_whitespace(c: char) -> bool {
    c.is_ascii_whitespace()
}

// ── HTTP denial response ──────────────────────────────────────────────────────

/// Build the exact HTTP denial response for the given class.
///
/// The response contract is fixed by NIP-FI.md rejection table:
/// - Status, Content-Type, WWW-Authenticate (for 401), and body bytes are the
///   closed contract.  No other fields are added that depend on the private
///   condition. [FI-TRACE-DENIAL-ORACLE]
pub(crate) fn http_denial(class: DenialClass) -> Response<Body> {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(class.http_status()).expect("valid status"))
        .header("Content-Type", class.content_type());
    if let Some(challenge) = class.www_authenticate() {
        builder = builder.header("WWW-Authenticate", challenge);
    }
    builder
        .body(Body::from(class.http_body()))
        .expect("valid denial response")
}

// ── State-convenience wrapper ─────────────────────────────────────────────────

/// Convenience wrapper: pull mode, verifier, and the shared deny map from
/// `AppState` and call [`admit_nip_fi_http`].
///
/// `extract_nip98` is a closure that performs NIP-98 authentication and
/// returns `(proven_pubkey, X)`.  The deny map is `state.nip_fi_deny_map` —
/// the same `Arc` the disconnect endpoint writes and WS admission reads.
///
/// This is the single entry-point every NIP-FI-protected surface calls.  It
/// delegates to [`admit_nip_fi_http`], which alone constructs a
/// [`NipFiAdmission`].
///
/// [FI-TRACE-AUTHORITY-UNIFORM]
// Response<Body> is intentionally large (axum's design); see admit_nip_fi_http.
#[allow(clippy::result_large_err)]
pub(crate) fn admit_nip_fi_http_on_state<X, F>(
    state: &crate::state::AppState,
    headers: &HeaderMap,
    extract_nip98: F,
) -> Result<NipFiAdmission<X>, Response<Body>>
where
    F: FnOnce() -> Result<Nip98Proof<X>, Response<Body>>,
{
    let mode = state.config.nip_fi.mode;
    let verifier = state.nip_fi_verifier.as_deref();
    match state.nip_fi_deny_map.as_deref() {
        Some(deny_map) => admit_nip_fi_http(headers, extract_nip98, verifier, mode, deny_map),
        None => admit_nip_fi_http(headers, extract_nip98, verifier, mode, &NoDenyMapConfigured),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Response<Body> is 128 bytes by axum's design; the large Err is intentional
    // throughout this module — it IS the HTTP response returned from tests.
    #![allow(clippy::result_large_err)]
    use super::*;

    /// Test fixture: a deny map with no entries.
    struct AlwaysAdmitStubDenyMap;
    impl sealed::Sealed for AlwaysAdmitStubDenyMap {}
    impl HttpDenyMap for AlwaysAdmitStubDenyMap {
        fn is_denied(&self, _issuer: &str, _pubkey: &PublicKey, _now: DateTime<Utc>) -> bool {
            false
        }
    }
    use axum::http::HeaderValue;
    use buzz_auth::{NipFiMode, VerifyAssertion};
    use chrono::Utc;

    // Helper: read the body bytes synchronously (tests only).
    fn body_bytes(resp: Response<Body>) -> Vec<u8> {
        use http_body_util::BodyExt as _;
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                resp.into_body()
                    .collect()
                    .await
                    .expect("body")
                    .to_bytes()
                    .to_vec()
            })
    }

    fn any_pubkey() -> PublicKey {
        nostr::Keys::generate().public_key()
    }

    // ── extract_bearer_token ─────────────────────────────────────────────────

    // Absent header → MissingEvidence (401).
    //
    // Mutation evidence: returning EvidenceRejected instead makes the
    // `assert_eq!(class, DenialClass::MissingEvidence)` assertion panic.
    #[test]
    fn missing_header_is_missing_evidence() {
        let headers = HeaderMap::new();
        let class = extract_bearer_token(&headers).unwrap_err();
        assert_eq!(class, DenialClass::MissingEvidence);
    }

    // Repeated header → EvidenceRejected (403).
    //
    // Mutation evidence: keeping the first value instead of rejecting makes
    // `unwrap_err()` panic.
    #[test]
    fn repeated_header_is_evidence_rejected() {
        let mut headers = HeaderMap::new();
        headers.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer token1"),
        );
        headers.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer token2"),
        );
        let class = extract_bearer_token(&headers).unwrap_err();
        assert_eq!(class, DenialClass::EvidenceRejected);
    }

    // Comma-combined → EvidenceRejected.
    #[test]
    fn comma_combined_is_evidence_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer a, Bearer b"),
        );
        let class = extract_bearer_token(&headers).unwrap_err();
        assert_eq!(class, DenialClass::EvidenceRejected);
    }

    // Empty token after Bearer prefix → EvidenceRejected.
    #[test]
    fn empty_token_is_evidence_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(CLIENT_ATTACHED_HEADER, HeaderValue::from_static("Bearer "));
        let class = extract_bearer_token(&headers).unwrap_err();
        assert_eq!(class, DenialClass::EvidenceRejected);
    }

    // Wrong prefix (non-Bearer) → EvidenceRejected.
    #[test]
    fn wrong_prefix_is_evidence_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Token xyz"),
        );
        let class = extract_bearer_token(&headers).unwrap_err();
        assert_eq!(class, DenialClass::EvidenceRejected);
    }

    // Whitespace in token → EvidenceRejected.
    #[test]
    fn whitespace_in_token_is_evidence_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer foo bar"),
        );
        let class = extract_bearer_token(&headers).unwrap_err();
        assert_eq!(class, DenialClass::EvidenceRejected);
    }

    // Valid Bearer token → extracted.
    #[test]
    fn valid_bearer_token_extracted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer a.b.c"),
        );
        let token = extract_bearer_token(&headers).unwrap();
        assert_eq!(token, "a.b.c");
    }

    // ── http_denial ──────────────────────────────────────────────────────────

    // MissingEvidence → 401, exact body, WWW-Authenticate: Nostr.
    //
    // Mutation evidence: changing status to 403 makes the status assert panic.
    #[test]
    fn missing_evidence_denial_is_401_with_nostr_challenge() {
        let resp = http_denial(DenialClass::MissingEvidence);
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get("WWW-Authenticate")
                .and_then(|v| v.to_str().ok()),
            Some("Nostr"),
            "MissingEvidence MUST carry WWW-Authenticate: Nostr"
        );
        assert_eq!(body_bytes(resp), b"authentication required\n");
    }

    // EvidenceRejected → 403, exact body, no WWW-Authenticate.
    //
    // Mutation evidence: changing status to 401 or body to "denied" makes
    // corresponding assertions panic.
    #[test]
    fn evidence_rejected_denial_is_403_exact_bytes() {
        let resp = http_denial(DenialClass::EvidenceRejected);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(
            resp.headers().get("WWW-Authenticate").is_none(),
            "EvidenceRejected must not carry a WWW-Authenticate header"
        );
        assert_eq!(body_bytes(resp), b"evidence rejected\n");
    }

    // AuthorizationDenied → 403, exact body.
    //
    // Mutation evidence: body check.
    #[test]
    fn authorization_denied_is_403_exact_bytes() {
        let resp = http_denial(DenialClass::AuthorizationDenied);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(body_bytes(resp), b"authorization denied\n");
    }

    // AuthorizationUnavailable → 503, exact body.
    //
    // Mutation evidence: status and body checks.
    #[test]
    fn authorization_unavailable_is_503_exact_bytes() {
        let resp = http_denial(DenialClass::AuthorizationUnavailable);
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_bytes(resp), b"authorization unavailable\n");
    }

    // Private-state conditions are byte-identical at the admission boundary:
    // a key-pairing mismatch and a deny-map hit must produce the same status,
    // headers and body, so a client cannot tell which private state denied it.
    // A matching-key request with a non-denying map is admitted, proving the
    // deny-map fixture is what produced the second denial.
    // [FI-TRACE-DENIAL-ORACLE]
    //
    // Mutation evidence: changing the deny-map branch to any other denial
    // class makes the status/body equality assertion fail.
    #[test]
    fn authorization_denied_rows_are_byte_identical() {
        use buzz_auth::VerifiedAssertion;

        struct FixedKeyVerifier(PublicKey);
        impl VerifyAssertion for FixedKeyVerifier {
            fn verify_assertion(
                &self,
                _token: &str,
            ) -> Result<VerifiedAssertion, buzz_auth::VerifierError> {
                Ok(VerifiedAssertion::new_for_test(self.0))
            }
        }
        struct FixedDenyMap(bool);
        impl sealed::Sealed for FixedDenyMap {}
        impl HttpDenyMap for FixedDenyMap {
            fn is_denied(&self, _: &str, _: &PublicKey, _: DateTime<Utc>) -> bool {
                self.0
            }
        }

        let pubkey_a = any_pubkey();
        let pubkey_b = any_pubkey();
        let verifier = FixedKeyVerifier(pubkey_a);
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer any.valid.looking.token"),
        );
        let admit = |proven: PublicKey, deny_map: &FixedDenyMap| {
            admit_nip_fi_http(
                &headers,
                || Ok(Nip98Proof::new(proven, ())),
                Some(&verifier as &dyn VerifyAssertion),
                NipFiMode::Enforce,
                deny_map,
            )
        };
        let snapshot = |resp: Response<Body>| {
            let status = resp.status();
            let headers = resp.headers().clone();
            (status, headers, body_bytes(resp))
        };

        let Err(mismatch) = admit(pubkey_b, &FixedDenyMap(false)) else {
            panic!("key mismatch must be denied");
        };
        let Err(denied) = admit(pubkey_a, &FixedDenyMap(true)) else {
            panic!("deny-map hit must be denied");
        };
        let mismatch = snapshot(mismatch);
        assert_eq!(mismatch.0, StatusCode::FORBIDDEN);
        assert_eq!(mismatch.2, b"authorization denied\n");
        assert_eq!(
            mismatch,
            snapshot(denied),
            "key-mismatch and deny-map denials must be byte-identical (status, headers, body)"
        );

        assert!(
            admit(pubkey_a, &FixedDenyMap(false)).is_ok(),
            "matching keys with a non-denying map must be admitted"
        );
    }

    // ── admit_nip_fi_http — off mode ─────────────────────────────────────────

    // Off mode → Ok(NipFiAdmission) with assertion=None regardless of headers.
    // The NIP-98 closure is still called; its pubkey is forwarded.
    //
    // Mutation evidence: returning Err from off mode makes `unwrap()` panic.
    #[test]
    fn off_mode_admits_unconditionally() {
        let headers = HeaderMap::new(); // no assertion
        let expected_pubkey = any_pubkey();
        let ep = expected_pubkey;
        let outcome = admit_nip_fi_http(
            &headers,
            || Ok(Nip98Proof::new(ep, ())),
            None::<&dyn VerifyAssertion>,
            NipFiMode::Off,
            &AlwaysAdmitStubDenyMap,
        );
        let admission =
            outcome.expect("Off mode MUST not require NIP-FI assertion — OSS default regression");
        assert_eq!(*admission.proven_pubkey(), expected_pubkey);
        assert!(admission.assertion().is_none());
    }

    // Off mode: NIP-98 closure failure propagates even in off mode.
    //
    // Mutation evidence: if off-mode short-circuits before the closure, the
    // returned Err is swallowed → `unwrap_err()` panics.
    #[test]
    fn off_mode_propagates_nip98_closure_failure() {
        let headers = HeaderMap::new();
        let deny_resp = http_denial(DenialClass::MissingEvidence);
        let deny_status = deny_resp.status();
        let outcome = admit_nip_fi_http::<_, (), _>(
            &headers,
            || Err(deny_resp),
            None::<&dyn VerifyAssertion>,
            NipFiMode::Off,
            &AlwaysAdmitStubDenyMap,
        );
        let resp = outcome.unwrap_err();
        assert_eq!(resp.status(), deny_status);
    }

    // ── F3: NIP-98 failure remapping in Enforce mode ─────────────────────────
    //
    // In Enforce mode, NIP-98 closure failure MUST produce NIP-FI DenialClass
    // responses (not legacy JSON).  The class depends on whether the
    // Authorization header was present:
    //   - Absent header → MissingEvidence (401)
    //   - Present-but-invalid → EvidenceRejected (403)
    //
    // In Off mode the legacy response is propagated unchanged.
    //
    // Mutation evidence (absent-header path): replacing MissingEvidence with
    // EvidenceRejected makes the `assert_eq!(status, 401)` assertion panic.
    // Mutation evidence (present-header path): replacing EvidenceRejected with
    // MissingEvidence makes the `assert_eq!(status, 403)` assertion panic.

    #[test]
    fn enforce_nip98_failure_absent_auth_yields_missing_evidence() {
        // Authorization header absent → NIP-98 closure fails → MissingEvidence (401).
        let headers = HeaderMap::new(); // no Authorization header
        let legacy_resp = http_denial(DenialClass::EvidenceRejected); // would be 403 if propagated
        let outcome = admit_nip_fi_http::<_, (), _>(
            &headers,
            || Err(legacy_resp),
            None::<&dyn VerifyAssertion>,
            NipFiMode::Enforce,
            &AlwaysAdmitStubDenyMap,
        );
        let resp = outcome.unwrap_err();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "absent-header NIP-98 failure MUST yield 401 MissingEvidence in Enforce mode"
        );
        assert_eq!(body_bytes(resp), b"authentication required\n");
    }

    #[test]
    fn enforce_nip98_failure_present_auth_yields_evidence_rejected() {
        // Authorization header present (but NIP-98 fails) → EvidenceRejected (403).
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Nostr invalid_base64!!!"),
        );
        let legacy_resp = http_denial(DenialClass::MissingEvidence); // would be 401 if propagated
        let outcome = admit_nip_fi_http::<_, (), _>(
            &headers,
            || Err(legacy_resp),
            None::<&dyn VerifyAssertion>,
            NipFiMode::Enforce,
            &AlwaysAdmitStubDenyMap,
        );
        let resp = outcome.unwrap_err();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "present-but-invalid Authorization MUST yield 403 EvidenceRejected in Enforce mode"
        );
        assert_eq!(body_bytes(resp), b"evidence rejected\n");
    }

    #[test]
    fn off_mode_nip98_failure_propagates_legacy_response() {
        // Off mode: legacy response is returned unchanged ([FI-INV-15]).
        // If this test breaks, Off mode is remapping errors it should leave alone.
        let headers = HeaderMap::new(); // no Authorization header
        let legacy_status = StatusCode::UNAUTHORIZED;
        let legacy_resp = http_denial(DenialClass::MissingEvidence);
        let outcome = admit_nip_fi_http::<_, (), _>(
            &headers,
            || Err(legacy_resp),
            None::<&dyn VerifyAssertion>,
            NipFiMode::Off,
            &AlwaysAdmitStubDenyMap,
        );
        let resp = outcome.unwrap_err();
        assert_eq!(
            resp.status(),
            legacy_status,
            "Off mode MUST propagate legacy NIP-98 failure response unchanged"
        );
    }

    // ── R6(a) regression: Off-mode preserves exact legacy JSON bytes / content-type ──
    //
    // Thufir R6 / Carl F3: the Off test in the existing suite checks only status,
    // so it cannot establish that Off mode preserves the JSON body bytes and
    // content-type header that the pre-NIP-FI paths produce.  This test uses a
    // synthetic "legacy JSON 401" response (matching what `api_error` in bridge.rs
    // produces) and verifies the exact body bytes and content-type survive Off mode.
    //
    // Mutation evidence: if Off mode remapped the error to `http_denial()` format
    // (`text/plain; charset=utf-8`), the content-type assertion fires.  If it
    // remapped the body to NIP-FI denial bytes, the body assertion fires.
    #[test]
    fn off_mode_preserves_exact_legacy_json_body_and_content_type() {
        use axum::http::header::CONTENT_TYPE;
        // Build a synthetic legacy JSON error response, as `api_error` does.
        let legacy_body = b"{\"error\":\"NIP-98: missing Authorization\"}";
        let legacy_resp = axum::http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header(CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(legacy_body.as_ref()))
            .unwrap();
        let headers = HeaderMap::new();
        let outcome = admit_nip_fi_http::<_, (), _>(
            &headers,
            || Err(legacy_resp),
            None::<&dyn VerifyAssertion>,
            NipFiMode::Off,
            &AlwaysAdmitStubDenyMap,
        );
        let resp = outcome.unwrap_err();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/json"),
            "Off mode MUST preserve the legacy application/json content-type"
        );
        assert_eq!(
            body_bytes(resp),
            legacy_body,
            "Off mode MUST preserve exact legacy JSON error body bytes"
        );
    }

    // ── admit_nip_fi_http — deny_protected ───────────────────────────────────

    // DenyProtected → Err(503 authorization unavailable) for every request
    // shape, without running the NIP-98 closure or the verifier.  Every case
    // carries a syntactically valid assertion bearer so the verifier would be
    // reachable if the mode check moved below token extraction.
    //
    // Mutation evidence: moving the DenyProtected check below the cardinality
    // gate makes the duplicate case return 403 (status assertion fails).
    // Moving it below the closure makes the closure counter non-zero; moving
    // it below the closure *and* the error remap also turns the failed-closure
    // cases into 401/403.  Moving it below verification makes the verifier
    // counter non-zero on the successful-closure case.
    #[test]
    fn deny_protected_returns_503_before_nip98_or_verifier() {
        use std::cell::Cell;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingVerifier(AtomicUsize);
        impl VerifyAssertion for CountingVerifier {
            fn verify_assertion(
                &self,
                _token: &str,
            ) -> Result<VerifiedAssertion, buzz_auth::VerifierError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(VerifiedAssertion::new_for_test(any_pubkey()))
            }
        }

        let auth = |headers: &mut HeaderMap, value: &'static str| {
            headers.append("authorization", HeaderValue::from_static(value));
        };
        let with_bearer = || {
            let mut headers = HeaderMap::new();
            headers.insert(
                CLIENT_ATTACHED_HEADER,
                HeaderValue::from_static("Bearer header.payload.signature"),
            );
            headers
        };
        let mut duplicate = with_bearer();
        auth(&mut duplicate, "Nostr first");
        auth(&mut duplicate, "Nostr second");
        let missing = with_bearer();
        let mut present = with_bearer();
        auth(&mut present, "Nostr invalid");

        // (case, headers, closure succeeds)
        let cases: [(&str, &HeaderMap, bool); 4] = [
            ("duplicate Authorization", &duplicate, true),
            ("missing Authorization", &missing, false),
            ("failed NIP-98 closure", &present, false),
            ("successful NIP-98 closure", &present, true),
        ];
        for (case, headers, closure_ok) in cases {
            let closure_calls = Cell::new(0u32);
            let verifier = CountingVerifier(AtomicUsize::new(0));
            let outcome = admit_nip_fi_http::<_, (), _>(
                headers,
                || {
                    closure_calls.set(closure_calls.get() + 1);
                    if closure_ok {
                        Ok(Nip98Proof::new(any_pubkey(), ()))
                    } else {
                        Err(Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .body(Body::from("legacy"))
                            .unwrap())
                    }
                },
                Some(&verifier as &dyn VerifyAssertion),
                NipFiMode::DenyProtected,
                &AlwaysAdmitStubDenyMap,
            );
            let Err(resp) = outcome else {
                panic!("{case}: DenyProtected must deny with 503");
            };
            assert_eq!(
                resp.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{case}: DenyProtected status"
            );
            assert_eq!(
                body_bytes(resp),
                b"authorization unavailable\n",
                "{case}: DenyProtected body"
            );
            assert_eq!(
                closure_calls.get(),
                0,
                "{case}: NIP-98 closure must not run"
            );
            assert_eq!(
                verifier.0.load(Ordering::SeqCst),
                0,
                "{case}: verifier must not run"
            );
        }
    }

    // ── admit_nip_fi_http — enforce, missing assertion ───────────────────────

    // Enforce + missing assertion header → Err(401).
    //
    // Mutation evidence: the status assertion on the response panics if the
    // missing-header path returns 403 instead of 401.
    #[test]
    fn enforce_missing_assertion_is_401() {
        let headers = HeaderMap::new();
        let pubkey = any_pubkey();
        let outcome = admit_nip_fi_http(
            &headers,
            || Ok(Nip98Proof::new(pubkey, ())),
            None::<&dyn VerifyAssertion>,
            NipFiMode::Enforce,
            &AlwaysAdmitStubDenyMap,
        );
        // Missing header → MissingEvidence before verifier check.
        match outcome {
            Err(resp) => {
                assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
                assert_eq!(body_bytes(resp), b"authentication required\n");
            }
            _ => panic!("Missing assertion must deny with 401"),
        }
    }

    // ── admit_nip_fi_http — enforce, no verifier (startup race) ─────────────

    // Enforce + valid-looking header but no verifier (startup race) → Err(503).
    //
    // Mutation evidence: returning 403 from the None-verifier path makes the
    // status assertion panic.
    #[test]
    fn enforce_no_verifier_returns_503() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer eyJhbGciOiJFUzI1NiJ9.e30.sig"),
        );
        let pubkey = any_pubkey();
        let outcome = admit_nip_fi_http(
            &headers,
            || Ok(Nip98Proof::new(pubkey, ())),
            None::<&dyn VerifyAssertion>,
            NipFiMode::Enforce,
            &AlwaysAdmitStubDenyMap,
        );
        match outcome {
            Err(resp) => {
                assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(body_bytes(resp), b"authorization unavailable\n");
            }
            _ => panic!("Missing verifier must deny with 503"),
        }
    }

    // ── admit_nip_fi_http — key pairing falsifier ────────────────────────────

    // Enforce mode: valid assertion for key-A + NIP-98 proving key-B → Err(403
    // authorization_denied).
    //
    // This is the **pairing-wiring falsifier** Thufir required (Round 4).
    // The test uses a mock verifier that returns a VerifiedAssertion whose
    // asserted_key is key-A, while the NIP-98 closure returns key-B.
    //
    // Mutation evidence (pairing branch):
    //   Remove the `Some(k) if k == proven_pubkey` branch (replace with
    //   `Some(_)`) → function admits instead of denying → `unwrap_err()` panics.
    //
    // [FI-INV-05] [FI-TRACE-ASSERTION-KEY-MISMATCH]
    #[test]
    fn enforce_key_mismatch_is_denied() {
        use buzz_auth::{VerifiedAssertion, VerifyAssertion};

        let key_a = nostr::Keys::generate();
        let key_b = nostr::Keys::generate();
        let pubkey_a = key_a.public_key();
        let pubkey_b = key_b.public_key();

        // Mock verifier: always succeeds, always claims pubkey_a as asserted_key.
        struct PairingMockVerifier(nostr::PublicKey);
        impl VerifyAssertion for PairingMockVerifier {
            fn verify_assertion(
                &self,
                _token: &str,
            ) -> Result<VerifiedAssertion, buzz_auth::VerifierError> {
                Ok(VerifiedAssertion::new_for_test(self.0))
            }
        }
        let verifier = PairingMockVerifier(pubkey_a);

        // NIP-98 closure returns key-B; assertion claims key-A → mismatch.
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer any.valid.looking.token"),
        );

        let outcome = admit_nip_fi_http(
            &headers,
            || Ok(Nip98Proof::new(pubkey_b, ())),
            Some(&verifier as &dyn VerifyAssertion),
            NipFiMode::Enforce,
            &AlwaysAdmitStubDenyMap,
        );
        match outcome {
            Err(resp) => {
                assert_eq!(
                    resp.status(),
                    StatusCode::FORBIDDEN,
                    "key mismatch MUST deny with 403 authorization_denied"
                );
                assert_eq!(body_bytes(resp), b"authorization denied\n");
            }
            Ok(_) => panic!(
                "assertion-for-A + NIP-98-for-B MUST be denied; \
                 pairing branch removal would cause this panic"
            ),
        }
    }

    // ── R3 regression: Authorization cardinality ─────────────────────────────
    //
    // Thufir R3 / Carl F2: duplicate Authorization headers must be rejected in
    // Enforce mode, and must be ACCEPTED in Off mode (FI-INV-15:
    // Off behavior must match pre-NIP-FI base, which used `.get()` first-value).
    //
    // The cardinality gate is now in `admit_nip_fi_http`, not in
    // `verify_bridge_auth_with_options`, ensuring Off-mode callers are never
    // affected regardless of their `require_auth_token` flag.
    //
    // Mutation evidence (enforce branch): removing the cardinality gate makes
    // a duplicate-header request proceed to NIP-98 extraction, which either
    // succeeds (if both tokens are valid — impossible in these tests with a
    // None verifier) or fails with a different status code.  The test would
    // still 403 in Enforce (extraction failure) but for the wrong reason; in
    // DenyProtected it would 503; in Off it would either 401 (missing NIP-98)
    // or pass.  The combination uniquely identifies the gate.
    //
    // Mutation evidence (Off branch): if Off-mode also checked cardinality, the
    // Off duplicate test would receive 403 instead of the legacy NIP-98 closure
    // result (401 from the always-failing closure below).  The assert fires.

    #[test]
    fn enforce_duplicate_authorization_header_denied_403() {
        // Enforce mode + two Authorization fields → 403 EvidenceRejected before
        // NIP-98 extraction runs.
        //
        // Mutation: removing the `auth_count > 1` gate means the closure runs,
        // extraction fails (invalid token), and admission maps the failure to
        // 403 EvidenceRejected (header is present).  Status is the same (403)
        // but the body is different — the gate produces the standard
        // `evidence rejected\n` bytes; NIP-98 failure in Enforce mode also
        // produces `evidence rejected\n`.  To distinguish, we verify the body
        // comes from cardinality (gate fires before closure) rather than from
        // the NIP-98 path: the closure must NEVER be called.
        use std::sync::atomic::{AtomicBool, Ordering};
        let closure_ran = std::sync::Arc::new(AtomicBool::new(false));
        let closure_ran_clone = closure_ran.clone();
        let pubkey = any_pubkey();
        let mut headers = HeaderMap::new();
        headers.append(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Nostr first.token"),
        );
        headers.append(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Nostr second.token"),
        );

        let outcome = admit_nip_fi_http::<_, (), _>(
            &headers,
            || {
                closure_ran_clone.store(true, Ordering::SeqCst);
                Ok(Nip98Proof::new(pubkey, ()))
            },
            None::<&dyn VerifyAssertion>,
            NipFiMode::Enforce,
            &AlwaysAdmitStubDenyMap,
        );
        let resp = outcome.unwrap_err();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "duplicate Authorization in Enforce MUST yield 403 EvidenceRejected"
        );
        assert_eq!(body_bytes(resp), b"evidence rejected\n");
        assert!(
            !closure_ran.load(Ordering::SeqCst),
            "NIP-98 closure must NOT run when cardinality gate fires"
        );
    }

    #[test]
    fn off_mode_duplicate_authorization_header_passes_to_closure() {
        // Off mode + two Authorization fields → closure runs, legacy behavior.
        //
        // FI-INV-15: Off mode must preserve pre-NIP-FI base behavior exactly.
        // The base parser used `.get()` which silently accepted the first
        // value from a multi-value header map.  Off mode must NOT reject on
        // cardinality — that would be a behavioral regression.
        //
        // Mutation evidence: adding a cardinality check in Off mode makes the
        // closure never run and returns 403.  The `closure_ran` assert fires.
        use std::sync::atomic::{AtomicBool, Ordering};
        let closure_ran = std::sync::Arc::new(AtomicBool::new(false));
        let closure_ran_clone = closure_ran.clone();
        let pubkey = any_pubkey();
        let mut headers = HeaderMap::new();
        headers.append(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Nostr first.token"),
        );
        headers.append(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Nostr second.token"),
        );

        // Closure succeeds → Off mode should admit.
        let outcome = admit_nip_fi_http::<_, (), _>(
            &headers,
            || {
                closure_ran_clone.store(true, Ordering::SeqCst);
                Ok(Nip98Proof::new(pubkey, ()))
            },
            None::<&dyn VerifyAssertion>,
            NipFiMode::Off,
            &AlwaysAdmitStubDenyMap,
        );
        let admission = match outcome {
            Ok(a) => a,
            Err(resp) => panic!(
                "Off mode MUST admit when closure succeeds, even with duplicate auth header; \
                 got {} response",
                resp.status()
            ),
        };
        assert!(
            closure_ran.load(Ordering::SeqCst),
            "NIP-98 closure MUST run in Off mode; cardinality gate must not fire"
        );
        assert_eq!(
            *admission.proven_pubkey(),
            pubkey,
            "proven_pubkey must be the one returned by the closure"
        );
    }
}
