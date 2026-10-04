//! NIP-FI assertion validation at WebSocket upgrade.
//!
//! Assertion evaluation and the HTTP denial contract are shared with HTTP
//! ingress: both come from [`crate::nip_fi_core`], so the transports cannot
//! drift.
//!
//! Per [NIP-FI.md](../../../docs/nips/NIP-FI.md) §Client-attached transport:
//! - Exactly one `Nostr-Federated-Identity: Bearer <compact-JWS>` field.
//! - Missing, repeated, comma-combined, empty, non-Bearer, and mixed-profile
//!   fields all deny. [FI-TRACE-TRANSPORT-CLOSED]
//! - Per §Rejection table, pre-101 denials are HTTP responses; the exact wire
//!   contract is fixed (status, body, headers). [FI-TRACE-DENIAL-ORACLE]

use axum::body::Body;
use axum::http::{HeaderMap, Response};
use buzz_auth::{DenialClass, NipFiMode, VerifiedAssertion, VerifyAssertion};

use crate::nip_fi_config::NipFiCommunities;
use crate::nip_fi_core::{
    evaluate_attached_assertion, http_denial, resolve_community, AssertionRejection,
};

/// Outcome of NIP-FI assertion validation at upgrade time.
pub(crate) enum NipFiUpgradeOutcome {
    /// Assertion validated successfully. Carry the result into the connection.
    Admitted(VerifiedAssertion),
    /// Off, or Shadow without a passing assertion — none is required.
    NotRequired,
    /// Shadow: the assertion passed and is only observed from here on.
    Observed(VerifiedAssertion),
    /// Enforcement active but assertion absent/rejected — return the HTTP
    /// denial response.
    Denied(Response<Body>),
}

/// The enforced and the shadow-observed assertion slots of an admitted upgrade.
pub(crate) type UpgradeAssertions = (Option<VerifiedAssertion>, Option<VerifiedAssertion>);

impl NipFiUpgradeOutcome {
    /// The enforced and the shadow-observed assertion, or the denial response.
    pub(crate) fn into_assertions(self) -> Result<UpgradeAssertions, Box<Response<Body>>> {
        match self {
            Self::NotRequired => Ok((None, None)),
            Self::Admitted(assertion) => Ok((Some(assertion), None)),
            Self::Observed(assertion) => Ok((None, Some(assertion))),
            Self::Denied(response) => Err(Box::new(response)),
        }
    }
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
pub(crate) fn check_nip_fi_at_upgrade(
    route: crate::nip_fi_session::NipFiWsRoute,
    headers: &HeaderMap,
    communities: &NipFiCommunities,
    verifier: Option<&dyn VerifyAssertion>,
    mode: NipFiMode,
) -> NipFiUpgradeOutcome {
    if !mode.restricts() {
        // Shadow records an upgrade would-deny; a passing assertion is
        // observed until AUTH, where its admission verdict is recorded.
        if mode.observes_only() {
            let verdict = resolve_community(headers, communities)
                .map_err(|class| (crate::nip_fi_shadow::Stage::Community, class))
                .and_then(|community| {
                    evaluate_attached_assertion(headers, community, verifier).map_err(|rejection| {
                        (
                            crate::nip_fi_shadow::Stage::Assertion,
                            rejection.denial_class(),
                        )
                    })
                });
            match verdict {
                Ok(assertion) => return NipFiUpgradeOutcome::Observed(assertion),
                Err(deny) => {
                    let route = route.shadow_label();
                    crate::nip_fi_shadow::record(route, headers, communities, Err(deny));
                }
            }
        }
        return NipFiUpgradeOutcome::NotRequired;
    }

    if mode.denies_unconditionally() {
        return NipFiUpgradeOutcome::Denied(http_denial(DenialClass::AuthorizationUnavailable));
    }

    // Enforce mode: resolve the Host's community, then validate the assertion.
    let community = match resolve_community(headers, communities) {
        Ok(community) => community,
        Err(class) => return NipFiUpgradeOutcome::Denied(http_denial(class)),
    };
    match evaluate_attached_assertion(headers, community, verifier) {
        Ok(assertion) => NipFiUpgradeOutcome::Admitted(assertion),
        Err(rejection) => {
            if let AssertionRejection::Verifier(err) = rejection {
                tracing::debug!(code = err.code(), "nip-fi assertion denied at upgrade");
            }
            NipFiUpgradeOutcome::Denied(http_denial(rejection.denial_class()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: crate::nip_fi_session::NipFiWsRoute = crate::nip_fi_session::NipFiWsRoute::Root;
    use crate::nip_fi_core::extract_bearer_token;
    use crate::nip_fi_core::test_support::{communities, host_headers};
    use crate::nip_fi_core::tests::ScriptedVerifier;
    use axum::http::{HeaderValue, StatusCode};
    use buzz_auth::CLIENT_ATTACHED_HEADER;

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = host_headers();
        h.insert(
            CLIENT_ATTACHED_HEADER,
            HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    // ── transport parsing ─────────────────────────────────────────────────────

    #[test]
    fn absent_header_gives_missing_evidence() {
        let h = host_headers();
        assert!(
            matches!(extract_bearer_token(&h), Err(DenialClass::MissingEvidence)),
            "absent NIP-FI header must be MissingEvidence"
        );
    }

    #[test]
    fn repeated_header_gives_evidence_rejected() {
        let mut h = host_headers();
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
        let resp = http_denial(DenialClass::MissingEvidence);
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
        let resp = http_denial(DenialClass::EvidenceRejected);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert!(
            resp.headers().get("WWW-Authenticate").is_none(),
            "EvidenceRejected must not carry WWW-Authenticate"
        );
        assert_eq!(body_bytes(resp), b"evidence rejected\n");
    }

    #[test]
    fn authorization_denied_response_is_403_exact_body() {
        let resp = http_denial(DenialClass::AuthorizationDenied);
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(body_bytes(resp), b"authorization denied\n");
    }

    #[test]
    fn authorization_unavailable_response_is_503_exact_body() {
        let resp = http_denial(DenialClass::AuthorizationUnavailable);
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
        // conditions reach the exact same `denial_frame(route, AuthorizationDenied)`
        // call.  This test pins that call against the production frame builder
        // and asserts that:
        //   1. Root and audio denial frames carry the correct denial text.
        //   2. `AuthorizationDenied` HTTP response is 403 exact bytes.
        //   3. `EvidenceRejected` (public) is distinct from `AuthorizationDenied`
        //      (private-state) — the oracle property.
        //
        // Mutation evidence:
        //   A) Change `DenialClass::AuthorizationDenied` in `denial_frame`
        //      → `nostr_text()` differs → root/audio text assertions panic.
        //   B) Swap the root NOTICE with a raw string → JSON parse fails or
        //      content assertion panics.
        //   C) Map `EvidenceRejected` to the same body → distinctness assert panics.
        use crate::nip_fi_session::{denial_frame, NipFiWsRoute};
        use axum::extract::ws::Message as WsMessage;

        let expected_text = buzz_auth::DenialClass::AuthorizationDenied.nostr_text();

        // Root frame: NOTICE JSON, content == nostr_text().
        let root_frame = denial_frame(
            NipFiWsRoute::Root,
            buzz_auth::DenialClass::AuthorizationDenied,
        );
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
        let audio_frame = denial_frame(
            NipFiWsRoute::Audio,
            buzz_auth::DenialClass::AuthorizationDenied,
        );
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
        let resp_private = http_denial(DenialClass::AuthorizationDenied);
        assert_eq!(resp_private.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            body_bytes(resp_private),
            b"authorization denied\n",
            "private-state denial HTTP body must be 'authorization denied\\n' [FI-TRACE-DENIAL-ORACLE]"
        );

        // Distinctness: public-evidence denial (EvidenceRejected) produces
        // different bytes from private-state denial (AuthorizationDenied).
        let resp_evidence = http_denial(DenialClass::EvidenceRejected);
        let resp_private2 = http_denial(DenialClass::AuthorizationDenied);
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
        let headers = host_headers();
        // add a valid-looking header so we don't short-circuit on missing evidence
        let mut h = headers;
        h.insert(
            CLIENT_ATTACHED_HEADER,
            axum::http::HeaderValue::from_static("Bearer eyJhbGciOiJFUzI1NiJ9.e30.sig"),
        );
        let outcome = check_nip_fi_at_upgrade(
            ROOT,
            &h,
            &communities(),
            None,
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
        let headers = host_headers();
        let outcome = check_nip_fi_at_upgrade(
            ROOT,
            &headers,
            &communities(),
            None,
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

    // Off → NotRequired (no assertion needed — OSS default, no regression)
    #[test]
    fn off_mode_returns_not_required() {
        let headers = host_headers(); // no assertion header
        let outcome = check_nip_fi_at_upgrade(
            ROOT,
            &headers,
            &communities(),
            None,
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
        let headers = host_headers();
        let outcome = check_nip_fi_at_upgrade(
            ROOT,
            &headers,
            &communities(),
            None,
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

    // ── Characterization: upgrade evaluation contract ────────────────────────

    fn denied_parts(outcome: NipFiUpgradeOutcome) -> (StatusCode, Vec<u8>) {
        match outcome {
            NipFiUpgradeOutcome::Denied(resp) => (resp.status(), body_bytes(resp)),
            _ => panic!("expected Denied"),
        }
    }

    // Pins: upgrade maps verifier errors through `VerifierError::denial_class`
    // (503 for unavailable dependencies, 403 evidence rejected otherwise).
    // Mutation: mapping every verifier error to EvidenceRejected fails the
    // 503 row.
    #[test]
    fn characterize_upgrade_verifier_error_classes() {
        use buzz_auth::VerifierError;
        let rows = [
            (
                VerifierError::KeySourceUnavailable,
                StatusCode::SERVICE_UNAVAILABLE,
                &b"authorization unavailable\n"[..],
            ),
            (
                VerifierError::InvalidSignatureOrClaims,
                StatusCode::FORBIDDEN,
                &b"evidence rejected\n"[..],
            ),
        ];
        for (err, status, body) in rows {
            let verifier = ScriptedVerifier::new(Err(err));
            let (got_status, got_body) = denied_parts(check_nip_fi_at_upgrade(
                ROOT,
                &headers_with("Bearer a.b.c"),
                &communities(),
                Some(&verifier),
                NipFiMode::Enforce,
            ));
            assert_eq!(got_status, status, "{err:?}");
            assert_eq!(got_body, body, "{err:?}");
        }
    }

    // Pins: transport extraction precedes the verifier-presence check.
    // Mutation: checking the verifier first turns this 403 into 503.
    #[test]
    fn characterize_upgrade_transport_precedes_verifier_presence() {
        let (status, body) = denied_parts(check_nip_fi_at_upgrade(
            ROOT,
            &headers_with("junk"),
            &communities(),
            None,
            NipFiMode::Enforce,
        ));
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, b"evidence rejected\n");
    }

    // Pins: upgrade does NOT perform key pairing — a claimless assertion is
    // admitted here and carried into the session, where NIP-42 pairing denies
    // it later. Mutation: adding a pairing check at upgrade denies instead.
    #[test]
    fn characterize_upgrade_admits_claimless_assertion_for_later_pairing() {
        let verifier = ScriptedVerifier::new(Ok(None));
        match check_nip_fi_at_upgrade(
            ROOT,
            &headers_with("Bearer a.b.c"),
            &communities(),
            Some(&verifier),
            NipFiMode::Enforce,
        ) {
            NipFiUpgradeOutcome::Admitted(assertion) => {
                assert_eq!(assertion.asserted_key(), None);
            }
            _ => panic!("claimless assertion must be admitted at upgrade"),
        }
        assert_eq!(verifier.calls(), 1);
    }

    // Pins: Off and DenyProtected short-circuit before the verifier runs, even
    // with a valid-looking header. Mutation: evaluating before the mode checks
    // makes the call count non-zero.
    #[test]
    fn characterize_upgrade_mode_short_circuits_skip_verifier() {
        let verifier = ScriptedVerifier::new(Ok(Some(nostr::Keys::generate().public_key())));
        let headers = headers_with("Bearer a.b.c");
        assert!(matches!(
            check_nip_fi_at_upgrade(
                ROOT,
                &headers,
                &communities(),
                Some(&verifier),
                NipFiMode::Off
            ),
            NipFiUpgradeOutcome::NotRequired
        ));
        let (status, _) = denied_parts(check_nip_fi_at_upgrade(
            ROOT,
            &headers,
            &communities(),
            Some(&verifier),
            NipFiMode::DenyProtected,
        ));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(verifier.calls(), 0);
    }

    // Pins: shadow evaluates the assertion but only observes it: the enforced
    // slot stays empty, so no connection is handed a verified assertion, and a
    // rejected one admits as Off. Mutation: returning `Admitted` in shadow, or
    // `Observed` for a rejection, fails the split.
    #[test]
    fn shadow_upgrade_observes_but_never_admits_an_assertion() {
        let verifier = ScriptedVerifier::new(Ok(Some(nostr::Keys::generate().public_key())));
        let headers = headers_with("Bearer a.b.c");
        let check = || {
            check_nip_fi_at_upgrade(
                ROOT,
                &headers,
                &communities(),
                Some(&verifier),
                NipFiMode::Shadow,
            )
        };
        let Ok((None, Some(_))) = check().into_assertions() else {
            panic!("a passing shadow assertion is observed, never enforced");
        };
        assert_eq!(verifier.calls(), 1, "shadow must evaluate the assertion");
        let Ok((None, None)) = check_nip_fi_at_upgrade(
            ROOT,
            &host_headers(),
            &communities(),
            Some(&verifier),
            NipFiMode::Shadow,
        )
        .into_assertions() else {
            panic!("a missing shadow assertion admits as Off");
        };
    }

    // Pins: a shadow upgrade would-deny records under its own ingress, so
    // audio refusals never count toward root's rate. Mutation: hard-coding
    // the `ws` route moves the audio records.
    #[test]
    fn shadow_upgrade_would_deny_records_its_ingress_route() {
        use crate::nip_fi_session::NipFiWsRoute::Audio;
        use crate::nip_fi_shadow_session::tests::shadow_records_by_route;
        let rejecting =
            ScriptedVerifier::new(Err(buzz_auth::VerifierError::InvalidSignatureOrClaims));
        let records = shadow_records_by_route(async {
            for route in [ROOT, Audio] {
                for headers in [host_headers(), headers_with("Bearer a.b.c")] {
                    let verifier = Some(&rejecting as &dyn VerifyAssertion);
                    let outcome = check_nip_fi_at_upgrade(
                        route,
                        &headers,
                        &communities(),
                        verifier,
                        NipFiMode::Shadow,
                    );
                    assert!(matches!(outcome, NipFiUpgradeOutcome::NotRequired));
                }
            }
        });
        let expected = [
            "audio assertion/missing",
            "audio assertion/rejected",
            "ws assertion/missing",
            "ws assertion/rejected",
        ];
        assert_eq!(records, expected);
    }
}
