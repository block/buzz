//! NIP-FI assertion validation at WebSocket upgrade.
//!
//! This module owns the exact NIP-FI HTTP denial contract for upgrade denials
//! and the header-parsing that feeds assertion validation.
//!
//! Per [NIP-FI.md](../../../docs/nips/NIP-FI.md) §Client-attached transport:
//! - Exactly one `Nostr-Federated-Identity: Bearer <compact-JWS>` field.
//! - Missing, repeated, comma-combined, empty, non-Bearer, and mixed-profile
//!   fields all deny. [FI-TRACE-TRANSPORT-CLOSED]
//! - Per §Rejection table, pre-101 denials are HTTP responses; the exact wire
//!   contract is fixed (status, body, headers). [FI-TRACE-DENIAL-ORACLE]

use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode};
use buzz_auth::{
    DenialClass, FederatedAssertionVerifier, IssuerKeySource, NipFiMode, VerifiedAssertion,
    CLIENT_ATTACHED_HEADER,
};

/// Outcome of NIP-FI assertion validation at upgrade time.
pub(crate) enum NipFiUpgradeOutcome {
    /// Assertion validated successfully. Carry the result into the connection.
    Admitted(VerifiedAssertion),
    /// Enforcement is off — no assertion required.
    NotRequired,
    /// Enforcement active but assertion absent/rejected — return the HTTP
    /// denial response.
    Denied(Response<Body>),
}

/// Validate the NIP-FI assertion on a WebSocket upgrade request.
///
/// Returns:
/// - `NotRequired` when the relay is in `Off` mode.
/// - `Admitted(assertion)` when the token is present, valid, and passes.
/// - `Denied(response)` with the exact NIP-FI HTTP denial contract otherwise.
///
/// The `DenyProtected` mode always returns `Denied(authorization_unavailable)`
/// (503), not `Denied(authorization_denied)` (403). This is intentional:
/// `DenyProtected` is operator-declared repair mode — the client's evidence may
/// be valid but authorization is temporarily unavailable — so "authorization
/// denied" would be false. "authorization unavailable, retry after repair" is
/// the accurate and correct signal. [FI-TRACE-DENIAL-ORACLE]
pub(crate) fn check_nip_fi_at_upgrade<S: IssuerKeySource>(
    headers: &HeaderMap,
    verifier: Option<&FederatedAssertionVerifier<S>>,
    mode: NipFiMode,
) -> NipFiUpgradeOutcome {
    if matches!(mode, NipFiMode::Off) {
        return NipFiUpgradeOutcome::NotRequired;
    }

    if matches!(mode, NipFiMode::DenyProtected) {
        return NipFiUpgradeOutcome::Denied(denial_response(DenialClass::AuthorizationUnavailable));
    }

    // Enforce mode: validate the assertion.
    let token = match extract_bearer_token(headers) {
        Ok(t) => t,
        Err(class) => return NipFiUpgradeOutcome::Denied(denial_response(class)),
    };

    let verifier = match verifier {
        Some(v) => v,
        None => {
            // Verifier not yet constructed (startup race); fail closed.
            return NipFiUpgradeOutcome::Denied(denial_response(
                DenialClass::AuthorizationUnavailable,
            ));
        }
    };

    match verifier.verify(token) {
        Ok(assertion) => NipFiUpgradeOutcome::Admitted(assertion),
        Err(err) => {
            tracing::debug!(code = err.code(), "nip-fi assertion denied at upgrade");
            NipFiUpgradeOutcome::Denied(denial_response(err.denial_class()))
        }
    }
}

/// Extract the single `Bearer <token>` value from the NIP-FI header.
///
/// Rejects all forms the spec prohibits:
/// - absent → `MissingEvidence`
/// - repeated (multiple header values) → `EvidenceRejected`
/// - comma-combined (`,` in a single value) → `EvidenceRejected`
/// - empty after `Bearer ` stripping → `EvidenceRejected`
/// - non-`Bearer ` prefix → `EvidenceRejected`
/// - value containing whitespace after the scheme → `EvidenceRejected`
///
/// [FI-TRACE-TRANSPORT-CLOSED]
fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, DenialClass> {
    let mut values = headers.get_all(CLIENT_ATTACHED_HEADER).iter();
    let first = match values.next() {
        Some(v) => v,
        None => return Err(DenialClass::MissingEvidence),
    };
    // Repeated header fields deny.
    if values.next().is_some() {
        return Err(DenialClass::EvidenceRejected);
    }
    let raw = first.to_str().map_err(|_| DenialClass::EvidenceRejected)?;
    // Comma-combined values deny.
    if raw.contains(',') {
        return Err(DenialClass::EvidenceRejected);
    }
    // Must be `Bearer <token>` — exactly that prefix.
    let token = raw
        .strip_prefix("Bearer ")
        .ok_or(DenialClass::EvidenceRejected)?;
    // Empty value after stripping denies.
    if token.is_empty() {
        return Err(DenialClass::EvidenceRejected);
    }
    // Whitespace within the token denies (mixed-profile detection).
    if token.contains(char::is_whitespace) {
        return Err(DenialClass::EvidenceRejected);
    }
    Ok(token)
}

/// Build the exact NIP-FI HTTP denial response for a WebSocket upgrade request.
///
/// Per the NIP-FI rejection table: status + exact body + `Content-Type`.
/// `MissingEvidence` additionally carries `WWW-Authenticate: Nostr`.
/// No free text, request ID, or per-principal information. [FI-TRACE-DENIAL-ORACLE]
pub(crate) fn denial_response(class: DenialClass) -> Response<Body> {
    let status =
        StatusCode::from_u16(class.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut builder = Response::builder()
        .status(status)
        .header("Content-Type", class.content_type());

    if let Some(www_auth) = class.www_authenticate() {
        builder = builder.header("WWW-Authenticate", www_auth);
    }

    builder
        .body(Body::from(class.http_body()))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    // ── transport parsing ─────────────────────────────────────────────────────

    #[test]
    fn absent_header_gives_missing_evidence() {
        let h = HeaderMap::new();
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::MissingEvidence)),
            "absent NIP-FI header must be MissingEvidence"
        );
    }

    #[test]
    fn repeated_header_gives_evidence_rejected() {
        let mut h = HeaderMap::new();
        h.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer aaa.bbb.ccc"),
        );
        h.append(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_static("Bearer ddd.eee.fff"),
        );
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "repeated NIP-FI header must be EvidenceRejected"
        );
    }

    #[test]
    fn comma_combined_gives_evidence_rejected() {
        let h = headers_with("Bearer aaa.bbb.ccc, Bearer ddd.eee.fff");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "comma-combined NIP-FI header must be EvidenceRejected"
        );
    }

    #[test]
    fn empty_value_gives_evidence_rejected() {
        let h = headers_with("");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "empty NIP-FI header must be EvidenceRejected"
        );
    }

    #[test]
    fn non_bearer_prefix_gives_evidence_rejected() {
        let h = headers_with("Token aaa.bbb.ccc");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "non-Bearer scheme must be EvidenceRejected"
        );
    }

    #[test]
    fn bearer_with_empty_token_gives_evidence_rejected() {
        let h = headers_with("Bearer ");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "empty token after Bearer must be EvidenceRejected"
        );
    }

    #[test]
    fn whitespace_in_token_gives_evidence_rejected() {
        let h = headers_with("Bearer aa bb.ccc.ddd");
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::EvidenceRejected)),
            "whitespace in token must be EvidenceRejected (mixed-profile)"
        );
    }

    #[test]
    fn valid_bearer_token_is_extracted() {
        let h = headers_with("Bearer eyJhbGciOiJFUzI1NiJ9.e30.sig");
        let token = extract_bearer_token(&h).expect("valid Bearer header must succeed");
        assert_eq!(token, "eyJhbGciOiJFUzI1NiJ9.e30.sig");
    }

    // ── denial response contract ──────────────────────────────────────────────
    //
    // NIP-FI requires the EXACT bytes; tests assert on exact body + headers.
    // [FI-TRACE-DENIAL-ORACLE]

    fn body_bytes(resp: Response<Body>) -> Vec<u8> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                axum::body::to_bytes(resp.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .to_vec()
            })
    }

    #[test]
    fn missing_evidence_response_is_401_with_www_authenticate() {
        let resp = denial_response(DenialClass::MissingEvidence);
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.headers()
                .get("WWW-Authenticate")
                .and_then(|v| v.to_str().ok()),
            Some("Nostr"),
            "MissingEvidence must carry WWW-Authenticate: Nostr"
        );
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(body_bytes(resp), b"authentication required\n");
    }

    #[test]
    fn evidence_rejected_response_is_403_exact_body() {
        let resp = denial_response(DenialClass::EvidenceRejected);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(
            resp.headers().get("WWW-Authenticate").is_none(),
            "EvidenceRejected must not carry WWW-Authenticate"
        );
        assert_eq!(body_bytes(resp), b"evidence rejected\n");
    }

    #[test]
    fn authorization_denied_response_is_403_exact_body() {
        let resp = denial_response(DenialClass::AuthorizationDenied);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(body_bytes(resp), b"authorization denied\n");
    }

    #[test]
    fn authorization_unavailable_response_is_503_exact_body() {
        let resp = denial_response(DenialClass::AuthorizationUnavailable);
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_bytes(resp), b"authorization unavailable\n");
    }

    #[test]
    fn private_state_denials_are_byte_identical() {
        // The spec's FI-TRACE-DENIAL-ORACLE: all private-state denial causes
        // (key mismatch, claimless assertion, expired lease) MUST map to the
        // same denial class (`AuthorizationDenied`) and produce byte-identical
        // wire frames on both ingresses.
        //
        // With `enforce_nip_fi_key_pairing` owning the full denial path, both
        // conditions reach the exact same `authorization_denied_frame(route)`
        // call.  This test pins that call against the production frame builder
        // and asserts that:
        //   1. Root and audio denial frames carry the correct denial text.
        //   2. `AuthorizationDenied` HTTP response is 403 exact bytes.
        //   3. `EvidenceRejected` (public) is distinct from `AuthorizationDenied`
        //      (private-state) — the oracle property.
        //
        // Mutation evidence:
        //   A) Change `DenialClass::AuthorizationDenied` in `authorization_denied_frame`
        //      → `nostr_text()` differs → root/audio text assertions panic.
        //   B) Swap the root NOTICE with a raw string → JSON parse fails or
        //      content assertion panics.
        //   C) Map `EvidenceRejected` to the same body → distinctness assert panics.
        use crate::nip_fi_session::{authorization_denied_frame, NipFiWsRoute};
        use axum::extract::ws::Message as WsMessage;

        let expected_text = buzz_auth::DenialClass::AuthorizationDenied.nostr_text();

        // Root frame: NOTICE JSON, content == nostr_text().
        let root_frame = authorization_denied_frame(NipFiWsRoute::Root);
        match root_frame {
            WsMessage::Text(t) => {
                let v: serde_json::Value =
                    serde_json::from_str(&t).expect("root denial frame is valid JSON");
                let content = v.get(1).and_then(|c| c.as_str()).unwrap_or("");
                assert_eq!(
                    content, expected_text,
                    "root denial frame content must equal AuthorizationDenied.nostr_text()"
                );
            }
            other => panic!("root denial frame must be WsMessage::Text; got {other:?}"),
        }

        // Audio frame: JSON object with type/message fields.
        let audio_frame = authorization_denied_frame(NipFiWsRoute::Audio);
        match audio_frame {
            WsMessage::Text(t) => {
                let v: serde_json::Value =
                    serde_json::from_str(&t).expect("audio denial frame is valid JSON");
                assert_eq!(
                    v.get("type").and_then(|x| x.as_str()),
                    Some("restricted"),
                    "audio denial frame type must be 'restricted'"
                );
                assert_eq!(
                    v.get("message").and_then(|x| x.as_str()),
                    Some(expected_text),
                    "audio denial frame message must equal AuthorizationDenied.nostr_text()"
                );
            }
            other => panic!("audio denial frame must be WsMessage::Text; got {other:?}"),
        }

        // HTTP-level oracle: AuthorizationDenied → 403 exact bytes.
        let resp_private = denial_response(DenialClass::AuthorizationDenied);
        assert_eq!(resp_private.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            body_bytes(resp_private),
            b"authorization denied\n",
            "private-state denial HTTP body must be 'authorization denied\\n' [FI-TRACE-DENIAL-ORACLE]"
        );

        // Distinctness: public-evidence denial (EvidenceRejected) produces
        // different bytes from private-state denial (AuthorizationDenied).
        let resp_evidence = denial_response(DenialClass::EvidenceRejected);
        let resp_private2 = denial_response(DenialClass::AuthorizationDenied);
        assert_ne!(
            body_bytes(resp_evidence),
            body_bytes(resp_private2),
            "public-evidence denial must be distinct from private-state denial"
        );
    }

    // ── Router-level gate: enforce mode, both WS ingresses ────────────────────
    //
    // `check_nip_fi_at_upgrade` is the single pre-101 gate called by BOTH the
    // root relay handler and the huddle audio handler (C1). Tests here drive it
    // with the exact request shapes that must deny and admit, establishing the
    // per-function mutation boundary.
    //
    // Note: these unit tests call `check_nip_fi_at_upgrade` directly and do NOT
    // falsify that the gate is wired into the router. The built-router integration
    // tests in `router.rs` (`nip_fi_enforce_*`) exercise the full WS upgrade
    // path through the real router for both `/` and `/huddle/{id}/audio` —
    // deleting either production gate call turns those tests red.
    //
    // Enforce + no verifier → 503 (dependency fail-closed; startup race)
    #[test]
    fn enforce_no_verifier_returns_503_exact_bytes() {
        // A None verifier in enforce mode means startup race — must deny 503.
        let headers = HeaderMap::new();
        // add a valid-looking header so we don't short-circuit on missing evidence
        let mut h = headers;
        h.insert(
            CLIENT_ATTACHED_HEADER,
            axum::http::HeaderValue::from_static("Bearer eyJhbGciOiJFUzI1NiJ9.e30.sig"),
        );
        let outcome = check_nip_fi_at_upgrade(
            &h,
            None::<&buzz_auth::FederatedAssertionVerifier<buzz_auth::ProductionJwksSource>>,
            buzz_auth::NipFiMode::Enforce,
        );
        match outcome {
            NipFiUpgradeOutcome::Denied(resp) => {
                assert_eq!(resp.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(body_bytes(resp), b"authorization unavailable\n");
            }
            _other => panic!("expected Denied(503), got non-denied outcome"),
        }
    }

    // Enforce + missing header → 401 exact bytes
    #[test]
    fn enforce_missing_header_returns_401_exact_bytes() {
        let headers = HeaderMap::new();
        let outcome = check_nip_fi_at_upgrade(
            &headers,
            None::<&buzz_auth::FederatedAssertionVerifier<buzz_auth::ProductionJwksSource>>,
            buzz_auth::NipFiMode::Enforce,
        );
        // Missing header → MissingEvidence; but None verifier fires first.
        // Correct behavior: extract_bearer_token is called before verifier check,
        // so missing header → 401 (MissingEvidence) before reaching the None verifier path.
        match outcome {
            NipFiUpgradeOutcome::Denied(resp) => {
                // Could be 401 (missing evidence extracted before verifier check)
                // or 503 (verifier check happens first). Either is a valid deny.
                // The exact ordering is:
                //   1. Off check → not off
                //   2. DenyProtected check → not deny_protected
                //   3. extract_bearer_token → Err(MissingEvidence) → return 401
                // So: 401 is the correct answer for missing header in enforce mode.
                assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
                assert_eq!(body_bytes(resp), b"authentication required\n");
            }
            _other => panic!("expected Denied, got non-denied outcome"),
        }
    }

    // Off mode → NotRequired (no assertion needed — OSS default, no regression)
    #[test]
    fn off_mode_returns_not_required() {
        let headers = HeaderMap::new(); // no assertion header
        let outcome = check_nip_fi_at_upgrade(
            &headers,
            None::<&buzz_auth::FederatedAssertionVerifier<buzz_auth::ProductionJwksSource>>,
            buzz_auth::NipFiMode::Off,
        );
        assert!(
            matches!(outcome, NipFiUpgradeOutcome::NotRequired),
            "Off mode must not require assertion — OSS default must not regress"
        );
    }

    // DenyProtected → 503 authorization_unavailable.
    //
    // DenyProtected is operator-declared repair mode. The relay denies all
    // upgrade attempts with `authorization_unavailable` (503), not
    // `authorization_denied` (403), because the client's evidence may be valid
    // but the authorization service is temporarily offline. A client retrying
    // after repair should succeed; "denied" is false and would suppress retries.
    //
    // Mutation evidence:
    //   A) Change `DenyProtected` handler to use `AuthorizationDenied` →
    //      status assertion panics (expected 503, got 403).
    //   B) Body assertion: change the body text → panics.
    #[test]
    fn deny_protected_returns_503_authorization_unavailable() {
        let headers = HeaderMap::new();
        let outcome = check_nip_fi_at_upgrade(
            &headers,
            None::<&buzz_auth::FederatedAssertionVerifier<buzz_auth::ProductionJwksSource>>,
            buzz_auth::NipFiMode::DenyProtected,
        );
        match outcome {
            NipFiUpgradeOutcome::Denied(resp) => {
                assert_eq!(
                    resp.status(),
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "DenyProtected must deny with 503 (authorization_unavailable), not 403"
                );
                assert_eq!(
                    body_bytes(resp),
                    b"authorization unavailable\n",
                    "DenyProtected body must be 'authorization unavailable\\n' [FI-TRACE-DENIAL-ORACLE]"
                );
            }
            _ => panic!("DenyProtected must return Denied(503), not NotRequired or Admitted"),
        }
    }
}
