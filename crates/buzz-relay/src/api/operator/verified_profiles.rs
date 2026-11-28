//! Operator delivery of issuer-authorized public names, without device impersonation.
//!
//! This output-only operation does not admit, enroll or grant membership to the
//! assertion's key. The operator authenticates delivery; the issuer authenticates
//! the target and names. Neither an operator-supplied name nor a kind-0 is accepted.

use super::*;
use buzz_db::verified_profiles::ProfileVerification;

pub(crate) const PATH: &str = "/operator/communities/verified-profile";
const MAX_ASSERTION_BYTES: usize = 16 * 1024;
type Error = (StatusCode, Json<Value>);

// Deliberately no Debug: the body contains a confidential bearer assertion.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    community_id: Uuid,
    expected_host: String,
    assertion: String,
}

fn refused(status: StatusCode, code: &'static str) -> Error {
    coded_api_error(status, code, code)
}

fn parse_request(body: &[u8]) -> Result<Request, Error> {
    let request: Request = serde_json::from_slice(body)
        .map_err(|_| refused(StatusCode::BAD_REQUEST, "invalid_request"))?;
    if request.community_id.is_nil()
        || normalize_candidate_host(&request.expected_host)
            .ok()
            .as_deref()
            != Some(request.expected_host.as_str())
        || request.assertion.is_empty()
        || request.assertion.len() > MAX_ASSERTION_BYTES
    {
        return Err(refused(StatusCode::BAD_REQUEST, "invalid_request"));
    }
    Ok(request)
}

/// Publish a relay-signed kind-30382 from a current human issuer assertion.
///
/// Requires deployment-operator NIP-98 authentication, a stable relay key,
/// enforced NIP-FI, a community-authorized opted-in issuer, and existing target
/// membership. Names and the target key come exclusively from the verified JWT.
/// Returns `published` with the signed event, or `not_written` for an unchanged,
/// superseded or no-longer-eligible assertion. A caller must not interpret
/// `not_written` as confirmation that its requested name is current.
pub async fn publish_verified_profile(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Error> {
    authorize_operator_request(&state, &slot, &headers, "POST", PATH, None, Some(&body)).await?;
    let request = parse_request(&body)?;
    if state.config.relay_private_key.is_none() || !state.config.nip_fi.mode.enforces() {
        return Err(refused(
            StatusCode::SERVICE_UNAVAILABLE,
            "verified_profiles_disabled",
        ));
    }
    let community = CommunityId::from_uuid(request.community_id);
    let host = state
        .db
        .lookup_community_host(community)
        .await
        .map_err(|_| refused(StatusCode::SERVICE_UNAVAILABLE, "authorization_unavailable"))?
        .filter(|host| host == &request.expected_host)
        .ok_or_else(|| refused(StatusCode::NOT_FOUND, "community_not_found"))?;
    if !crate::single_community::serves_host(&state.config, &host) {
        return Err(refused(StatusCode::FORBIDDEN, "community_not_served"));
    }
    slot.in_community(community);
    // The signed body selects the target; never resolve the inbound management
    // Host as a tenant or allow the caller to choose an assertion audience.
    let mut target_headers = HeaderMap::new();
    target_headers.insert(
        axum::http::header::HOST,
        host.parse()
            .map_err(|_| refused(StatusCode::BAD_REQUEST, "invalid_request"))?,
    );
    let binding = state
        .nip_fi_communities()
        .resolve(&target_headers)
        .await
        .map_err(|_| refused(StatusCode::SERVICE_UNAVAILABLE, "authorization_unavailable"))?;
    let verifier = state
        .nip_fi_verifier
        .as_deref()
        .ok_or_else(|| refused(StatusCode::SERVICE_UNAVAILABLE, "authorization_unavailable"))?;
    let assertion = verifier
        .verify_assertion(&request.assertion, &binding)
        .map_err(|error| {
            if crate::nip_fi_core::AssertionRejection::Verifier(error).denial_class()
                == buzz_auth::DenialClass::AuthorizationUnavailable
            {
                refused(StatusCode::SERVICE_UNAVAILABLE, "authorization_unavailable")
            } else {
                refused(StatusCode::FORBIDDEN, "assertion_rejected")
            }
        })?;
    let deny_map = state
        .nip_fi_deny_map
        .as_deref()
        .ok_or_else(|| refused(StatusCode::SERVICE_UNAVAILABLE, "authorization_unavailable"))?;
    if crate::nip_fi_core::assertion_denied(deny_map, &assertion, chrono::Utc::now()) {
        return Err(refused(StatusCode::FORBIDDEN, "assertion_rejected"));
    }
    let key = assertion
        .asserted_key()
        .ok_or_else(|| refused(StatusCode::FORBIDDEN, "public_profile_required"))?;
    let evidence = assertion
        .profile_assertion()
        .ok_or_else(|| refused(StatusCode::FORBIDDEN, "public_profile_required"))?;
    let profile = evidence
        .profile
        .as_ref()
        .filter(|_| assertion.owner().is_none())
        .ok_or_else(|| refused(StatusCode::FORBIDDEN, "public_profile_required"))?;
    let policy = crate::verified_profiles::enabled_policies(&state)
        .find(|p| p.issuer == assertion.identity().issuer())
        .ok_or_else(|| refused(StatusCode::FORBIDDEN, "issuer_publication_disabled"))?;
    let label = policy
        .label
        .as_deref()
        .ok_or_else(|| refused(StatusCode::FORBIDDEN, "issuer_publication_disabled"))?;
    binding
        .deadline()
        .check()
        .map_err(|_| refused(StatusCode::SERVICE_UNAVAILABLE, "authorization_unavailable"))?;
    let tenant = TenantContext::resolved(community, &host);
    let pubkey = key.to_hex();
    let event = state
        .db
        .publish_verified_profile(
            ProfileVerification {
                community,
                issuer: &policy.issuer,
                pubkey: &pubkey,
                issued_at: evidence.issued_at,
                verified_at: chrono::Utc::now(),
                display_name: &profile.display_name,
                username: &profile.username,
                full_name: profile.full_name.as_deref(),
                label,
                refresh_seconds: policy.refresh_seconds,
                // Unlike a WebSocket admission, delivery proves no device possession.
                // It may label only an already-existing member, even on an open relay.
                membership_required: true,
            },
            &state.relay_keypair,
        )
        .await
        .map_err(|_| refused(StatusCode::SERVICE_UNAVAILABLE, "publication_unavailable"))?;
    match event {
        Some(event) => {
            crate::verified_profiles::deliver(&state, &tenant, &event).await;
            Ok(Json(
                serde_json::json!({"status": "published", "event": event}),
            ))
        }
        None => Ok(Json(serde_json::json!({"status": "not_written"}))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_accepts_only_bounded_assertion_and_exact_target() {
        let valid = serde_json::json!({
            "community_id": Uuid::new_v4(), "expected_host": "relay.example", "assertion": "jwt"
        });
        assert!(parse_request(valid.to_string().as_bytes()).is_ok());
        for (field, value) in [
            ("display_name", serde_json::json!("Chosen by operator")),
            (
                "pubkey",
                serde_json::json!(nostr::Keys::generate().public_key().to_hex()),
            ),
            ("community_id", serde_json::json!(Uuid::nil())),
            ("expected_host", serde_json::json!("https://relay.example")),
            ("expected_host", serde_json::json!("RELAY.EXAMPLE")),
            ("assertion", serde_json::json!("")),
            (
                "assertion",
                serde_json::json!("a".repeat(MAX_ASSERTION_BYTES + 1)),
            ),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(
                parse_request(bad.to_string().as_bytes()).is_err(),
                "{field}"
            );
        }
    }
}
