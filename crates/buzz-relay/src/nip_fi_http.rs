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
//! one is via [`admit_nip_fi_http`].  Handler code that requires a
//! `NipFiAdmission` to obtain `proven_pubkey` cannot be reached without
//! executing the full admission sequence.
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
//! The deny map is S4 (Duncan).  Until S4 lands this module stubs it as a
//! fail-open no-op: [`HttpDenyMap::is_denied`] always returns false.  When S4
//! adds the real implementation, replace the stub in `admit_nip_fi_http_on_state`
//! with a reference to the real map.  The integration is a one-liner.
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

// ── Deny-map seam (S4 stub) ───────────────────────────────────────────────────

/// Narrow interface consumed by HTTP enforcement.  S4 (Duncan) will provide
/// the real implementation; until then, `AlwaysAdmitStubDenyMap` stubs it
/// fail-open (admits unconditionally).
///
/// Signature mirrors `NipFiDenyMap::is_denied` from S4 so integration is a
/// one-liner: replace `AlwaysAdmitStubDenyMap` with the shared map.
///
/// `(issuer, pubkey, now)` are required because the deny set is issuer-
/// scoped per `NIP-FI.md:624-627`.  Passing only pubkey would collide
/// across issuers — a deny for `(iss-A, k)` must not block `(iss-B, k)`.
///
/// Sealed: only implementations in this crate are accepted.
pub(crate) trait HttpDenyMap: sealed::Sealed {
    /// Returns `true` when `(issuer, pubkey)` has an active deny entry at
    /// `now` (`now < until`).  A poisoned or unavailable backing store MUST
    /// return `false` (admits) only when an explicit availability guarantee is
    /// established; the S4 real map currently admits on poisoned lock.  The
    /// S4 integration commit is expected to resolve the fail-closed story
    /// before S5 merges; the interface contract here is the agreed shape.
    fn is_denied(&self, issuer: &str, pubkey: &PublicKey, now: DateTime<Utc>) -> bool;
}

pub(crate) mod sealed {
    pub(crate) trait Sealed {}
}

/// Stub deny map that always admits.  Used until S4 provides the real map.
///
/// Name is explicit: this is **fail-open**, not fail-closed.  The stub phase
/// is intentional — deny-map enforcement defers to S4 landing.  The name
/// `AlwaysAdmitStubDenyMap` prevents a future integrator from assuming this
/// stub is safe for production use.
pub(crate) struct AlwaysAdmitStubDenyMap;
impl sealed::Sealed for AlwaysAdmitStubDenyMap {}
impl HttpDenyMap for AlwaysAdmitStubDenyMap {
    /// Always admits: the deny map is not yet wired (S4 pending).
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

/// Proof that the full NIP-FI admission sequence completed for one HTTP request.
///
/// Construction is private to [`admit_nip_fi_http`].  **No other code path
/// produces this type.**  A handler signature that requires `NipFiAdmission`
/// as input can therefore not be reached without executing the full sequence:
///
///   NIP-98 extraction → assertion extraction → verify → pair → deny-map → admit
///
/// `X` is caller-supplied side-data returned by the NIP-98 extraction closure
/// (e.g. replay-detection fields).  Use `()` when no side-data is needed.
///
/// [FI-TRACE-AUTHORITY-UNIFORM] Every protected HTTP surface produces this
/// type via `admit_nip_fi_http`; there is no other source.
#[must_use]
pub(crate) struct NipFiAdmission<X = ()> {
    /// The pubkey proven by NIP-98 and confirmed by assertion pairing.
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
    /// The pubkey proven by both NIP-98 and assertion pairing.
    ///
    /// This is the only way to obtain an authoritative pubkey for downstream
    /// authorization checks.  It is equal to the NIP-98 `pubkey` (what the
    /// request proved) and to the assertion's `nostr_pubkey` (what the
    /// federation identity bound).
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
/// 1. Run `extract_nip98` — the caller's NIP-98 extraction closure.  Returns
///    `(proven_pubkey, X)` on success, or a `Response` to emit on failure.
///    Running NIP-98 first allows the closure to short-circuit (e.g. missing
///    `Authorization` header) before the more expensive assertion verification.
/// 2. Off mode: skip assertion steps; return `Ok(NipFiAdmission { proven_pubkey,
///    assertion: None, extra: X })`.  Off-mode behavior is identical to
///    pre-NIP-FI (no assertion requirement).  [FI-INV-15]
/// 3. DenyProtected mode: unconditional 503 regardless of assertion presence.
/// 4. Enforce mode: extract `Nostr-Federated-Identity: Bearer <JWS>`.
/// 5. Verify assertion (signature, issuer, expiry, claims).
/// 6. Assert `assertion.asserted_key == proven_pubkey`.  [FI-INV-05]
/// 7. Check deny map for `(iss, proven_pubkey)`.  [FI-INV-14]
/// 8. Return `Ok(NipFiAdmission { proven_pubkey, assertion: Some(...), extra: X })`.
///
/// ## Bypass impossibility
///
/// [`NipFiAdmission`] has a private constructor.  The only source of a
/// `NipFiAdmission` value is this function.  A handler that skips this call
/// has no `NipFiAdmission` and cannot obtain `proven_pubkey` through the
/// NIP-FI admission channel.
///
/// ## Off-mode semantics
///
/// The NIP-98 closure is always called (steps 1–2).  In Off mode the closure
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
    // Step 1: run NIP-98 extraction.  Always runs regardless of mode.
    let Nip98Proof {
        pubkey: proven_pubkey,
        extra,
    } = extract_nip98()?;

    // Step 2 — Off mode: NIP-FI not required.  Return admission immediately.
    // The NIP-98 closure already enforced whatever auth the surface required.
    // [FI-INV-15 exemption]
    if matches!(mode, NipFiMode::Off) {
        return Ok(NipFiAdmission {
            proven_pubkey,
            assertion: None,
            extra,
        });
    }

    // Step 3 — DenyProtected mode: unconditional 503.
    if matches!(mode, NipFiMode::DenyProtected) {
        return Err(http_denial(DenialClass::AuthorizationUnavailable));
    }

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

/// Convenience wrapper: pull mode + verifier from `AppState` and call
/// [`admit_nip_fi_http`].
///
/// `extract_nip98` is a closure that performs NIP-98 authentication and
/// returns `(proven_pubkey, X)`.  This wrapper supplies `deny_map =
/// &AlwaysAdmitStubDenyMap`; S4 can replace the stub without touching call
/// sites by changing this wrapper.
///
/// This is the single entry-point every NIP-FI-protected surface calls.
/// There is no other way to produce a [`NipFiAdmission`].
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
    admit_nip_fi_http(
        headers,
        extract_nip98,
        verifier,
        mode,
        &AlwaysAdmitStubDenyMap,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Response<Body> is 128 bytes by axum's design; the large Err is intentional
    // throughout this module — it IS the HTTP response returned from tests.
    #![allow(clippy::result_large_err)]
    use super::*;
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

    // Private-state conditions (AuthorizationDenied) are byte-identical.
    // Key mismatch and denied pubkey both map to authorization_denied.
    // [FI-TRACE-DENIAL-ORACLE]
    //
    // Mutation evidence: if key_mismatch path emitted a different class, the
    // assert_eq on body would diverge.
    #[test]
    fn authorization_denied_rows_are_byte_identical() {
        let a = body_bytes(http_denial(DenialClass::AuthorizationDenied));
        // A second call produces the same bytes.
        let b = body_bytes(http_denial(DenialClass::AuthorizationDenied));
        assert_eq!(
            a, b,
            "all AuthorizationDenied responses must be byte-identical"
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

    // ── admit_nip_fi_http — deny_protected ───────────────────────────────────

    // DenyProtected → Err(503 authorization_unavailable).
    //
    // Mutation evidence: returning Ok from deny_protected mode makes
    // `unwrap_err()` panic.
    #[test]
    fn deny_protected_returns_503() {
        let headers = HeaderMap::new();
        let pubkey = any_pubkey();
        let outcome = admit_nip_fi_http(
            &headers,
            || Ok(Nip98Proof::new(pubkey, ())),
            None::<&dyn VerifyAssertion>,
            NipFiMode::DenyProtected,
            &AlwaysAdmitStubDenyMap,
        );
        match outcome {
            Err(resp) => {
                assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(body_bytes(resp), b"authorization unavailable\n");
            }
            _ => panic!("DenyProtected must deny with 503"),
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

    // ── admit_nip_fi_http — deny map stub admits ─────────────────────────────

    // The stub deny map always admits (never denies).
    //
    // Mutation evidence: if `is_denied` returned true, the deny path would
    // fire and the test would receive a Denied outcome instead of reaching
    // the verifier check (which would deny for a different reason — invalid
    // token).  The distinction is observable: 401 vs 403.
    #[test]
    fn stub_deny_map_never_denies() {
        let pubkey = any_pubkey();
        assert!(
            !AlwaysAdmitStubDenyMap.is_denied("https://idp.example.com", &pubkey, Utc::now()),
            "stub deny map MUST admit unconditionally until S4 provides the real map"
        );
    }
}
