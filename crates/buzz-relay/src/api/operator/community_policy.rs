//! General operator policy intents and truthful current/historical readback.

use super::*;
use buzz_db::community_auth::{PolicyOutcome, PolicyRecord, PolicyRequest, UpgradeRequest};

const PATH: &str = "/operator/communities/nip-fi-policy";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation_id: Uuid,
    community_id: Uuid,
    expected_host: String,
    actor_pubkey: String,
    expected_owner_pubkey: String,
    #[serde(default)]
    expected_issuer: Option<String>,
    requires_nip_fi: bool,
    expected_policy_generation: i64,
    expected_requires_nip_fi: bool,
}

/// Exactly one of an operation receipt or a community/host current-state read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyQuery {
    operation_id: Option<Uuid>,
    community_id: Option<Uuid>,
    expected_host: Option<String>,
}

type Error = (StatusCode, Json<Value>);

fn invalid() -> Error {
    coded_api_error(
        StatusCode::BAD_REQUEST,
        "invalid_request",
        "invalid policy request",
    )
}

fn unavailable(error: buzz_db::DbError) -> Error {
    tracing::warn!(%error, "community auth policy unavailable");
    coded_api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "authorization_unavailable",
        "authorization unavailable",
    )
}

fn mode_fields(state: &AppState, required: Option<bool>) -> Value {
    let mode = state.config.nip_fi.mode;
    let name = crate::nip_fi_config::mode_name(mode);
    serde_json::json!({
        "mode": name,
        "effective_required": required.map(|r| mode.with_requirement(r).restricts()),
    })
}

fn response(state: &AppState, mut record: PolicyRecord) -> Json<Value> {
    // Supersession is history, not a trust failure. Disable needs no issuer.
    if record.acceptance.state == "pending"
        && upgrade_configured_trust(
            state,
            &record.acceptance.expected_host,
            &record.acceptance.expected_issuer,
        )
        .is_err()
    {
        record.acceptance.state = "unavailable".into();
    }
    let fields = mode_fields(state, record.current_requires_nip_fi);
    Json(serde_json::json!({
        "receipt": record,
        "mode": fields["mode"],
        "effective_required": fields["effective_required"],
    }))
}

/// Atomically accept an owner-authorized stored requirement with CAS and replay.
pub async fn set_community_nip_fi_policy(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Error> {
    let operator =
        authorize_operator_request(&state, &slot, &headers, "POST", PATH, None, Some(&body))
            .await?;
    let request: Request = serde_json::from_slice(&body).map_err(|_| invalid())?;
    let host = normalize_candidate_host(&request.expected_host).map_err(|_| invalid())?;
    if request.operation_id.is_nil()
        || request.community_id.is_nil()
        || request.expected_policy_generation < 0
        || host != request.expected_host
        || (request.requires_nip_fi
            && request
                .expected_issuer
                .as_ref()
                .is_none_or(|s| s.is_empty()))
        || (!request.requires_nip_fi && request.expected_issuer.is_some())
    {
        return Err(invalid());
    }
    let tuple = UpgradeRequest {
        operation_id: request.operation_id,
        community_id: request.community_id,
        expected_host: host,
        actor_pubkey: validate_pubkey_hex(&request.actor_pubkey).ok_or_else(invalid)?,
        expected_owner_pubkey: validate_pubkey_hex(&request.expected_owner_pubkey)
            .ok_or_else(invalid)?,
        expected_issuer: request.expected_issuer.unwrap_or_default(),
        operator_pubkey: operator.to_hex(),
    };
    let preflight = if !state.config.nip_fi_changes {
        Err("upgrades_disabled")
    } else if request.requires_nip_fi {
        upgrade_configured_trust(&state, &tuple.expected_host, &tuple.expected_issuer)
    } else {
        Ok(())
    };
    let intent = PolicyRequest {
        tuple,
        requires_nip_fi: request.requires_nip_fi,
        expected_policy_generation: request.expected_policy_generation,
        expected_requires_nip_fi: request.expected_requires_nip_fi,
    };
    match state
        .db
        .set_community_nip_fi_policy(&intent, preflight)
        .await
        .map_err(unavailable)?
    {
        PolicyOutcome::Accepted(record) => {
            state.invalidate_community_host(&TenantContext::resolved(
                buzz_core::CommunityId::from_uuid(record.acceptance.community_id),
                &record.acceptance.expected_host,
            ));
            Ok(response(&state, *record))
        }
        PolicyOutcome::Refused(code) => Err(coded_api_error(
            match code {
                "community_not_found" => StatusCode::NOT_FOUND,
                "upgrades_disabled" | "authorization_unavailable" => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
                _ => StatusCode::CONFLICT,
            },
            code,
            code,
        )),
    }
}

/// Read exact current preconditions or the originating operator's operation history.
pub async fn community_nip_fi_policy_status(
    State(state): State<Arc<AppState>>,
    slot: AccessSlot,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    Query(query): Query<PolicyQuery>,
) -> Result<Json<Value>, Error> {
    let operator =
        authorize_operator_request(&state, &slot, &headers, "GET", PATH, raw.as_deref(), None)
            .await?;
    match (query.operation_id, query.community_id, query.expected_host) {
        (Some(id), None, None) if !id.is_nil() => {
            let record = state
                .db
                .community_auth_policy_status(id, &operator.to_hex())
                .await
                .map_err(unavailable)?
                .ok_or_else(|| {
                    coded_api_error(
                        StatusCode::NOT_FOUND,
                        "operation_not_found",
                        "operation not found",
                    )
                })?;
            Ok(response(&state, record))
        }
        (None, Some(id), Some(host)) if !id.is_nil() => {
            if normalize_candidate_host(&host).map_err(|_| invalid())? != host {
                return Err(invalid());
            }
            let policy = state
                .db
                .community_auth_policy_for_host(&host)
                .await
                .map_err(unavailable)?
                .filter(|p| p.community_id == id)
                .ok_or_else(|| {
                    coded_api_error(
                        StatusCode::NOT_FOUND,
                        "community_not_found",
                        "community not found",
                    )
                })?;
            let fields = mode_fields(&state, Some(policy.requires_nip_fi));
            Ok(Json(serde_json::json!({
                "community_id": id, "expected_host": host,
                "expected_policy_generation": policy.policy_generation,
                "expected_requires_nip_fi": policy.requires_nip_fi,
                "mode": fields["mode"], "effective_required": fields["effective_required"],
            })))
        }
        _ => Err(invalid()),
    }
}
