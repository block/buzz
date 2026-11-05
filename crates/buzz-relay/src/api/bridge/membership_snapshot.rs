//! Opt-in NIP-CL compact discovery; ordinary arrays never assert completeness.

use axum::{http::StatusCode, Json};
use buzz_core::{kind::KIND_MEMBERSHIP_SNAPSHOT, TenantContext};
use buzz_db::membership_snapshot::{valid_types, MAX_RESPONSE_BYTES};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{api::api_error, state::AppState};

pub(super) fn requested(filters: &[Value]) -> bool {
    filters
        .iter()
        .any(|f| f.get("membership_snapshot").is_some())
}

fn types(filters: &[Value], viewer: &nostr::PublicKey) -> Result<Vec<String>, &'static str> {
    let types: Vec<String> = filters
        .first()
        .and_then(|f| f.get("#t"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .filter(|types: &Vec<String>| valid_types(types))
        .ok_or("membership_snapshot requires a sorted unique nonempty known type set")?;
    let expected = json!({
        "kinds": [KIND_MEMBERSHIP_SNAPSHOT],
        "#p": [viewer.to_hex()],
        "#t": types,
        "membership_snapshot": 1,
    });
    if filters != [expected] {
        return Err("membership_snapshot requires one exact version-1 own-viewer filter");
    }
    Ok(types)
}

#[derive(Serialize)]
struct Content<'a> {
    version: u8,
    community_id: uuid::Uuid,
    host: &'a str,
    viewer: String,
    request_id: String,
    types: &'a [String],
    complete: bool,
    snapshot_id: &'a str,
    channels: &'a [buzz_db::membership_snapshot::ViewerMembership],
}

pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    viewer: &nostr::PublicKey,
    request_id: [u8; 32],
    filters: &[Value],
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if !state.config.membership_snapshot_enabled {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "membership_snapshot unsupported",
        ));
    }
    // Even dev mode must provide real NIP-98 for this new authority surface.
    if request_id == [0; 32] {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "membership_snapshot requires NIP-98",
        ));
    }
    let types =
        types(filters, viewer).map_err(|reason| api_error(StatusCode::BAD_REQUEST, reason))?;
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        state
            .db
            .membership_snapshot(tenant.community(), viewer, &types),
    )
    .await
    .map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "membership_snapshot timed out",
        )
    })?
    .map_err(|error| match error {
        buzz_db::DbError::MembershipSnapshotTooLarge => api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "membership_snapshot exceeds limit",
        ),
        _ => {
            tracing::error!(error_kind = %error.kind(), "membership snapshot failed");
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "membership_snapshot unavailable",
            )
        }
    })?;
    let started = std::time::Instant::now();
    let content = Content {
        version: 1,
        community_id: *tenant.community().as_uuid(),
        host: tenant.host(),
        viewer: viewer.to_hex(),
        request_id: hex::encode(request_id),
        types: &types,
        complete: true,
        snapshot_id: &snapshot.snapshot_id,
        channels: &snapshot.channels,
    };
    let serialized = serde_json::to_string(&content).map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "membership_snapshot encoding failed",
        )
    })?;
    let tags = [
        ["d".to_string(), content.request_id],
        ["p".to_string(), content.viewer],
    ]
    .into_iter()
    .map(nostr::Tag::parse)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "membership_snapshot tags failed",
        )
    })?;
    let event = nostr::EventBuilder::new(
        nostr::Kind::Custom(KIND_MEMBERSHIP_SNAPSHOT as u16),
        serialized,
    )
    .tags(tags)
    .sign_with_keys(&state.relay_keypair)
    .map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "membership_snapshot signing failed",
        )
    })?;
    let envelope = json!({"membership_snapshot":1, "event":event});
    let bytes = serde_json::to_vec(&envelope)
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "membership_snapshot encoding failed",
            )
        })?
        .len();
    if bytes > MAX_RESPONSE_BYTES {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "membership_snapshot exceeds limit",
        ));
    }
    metrics::histogram!("buzz_membership_snapshot_encode_sign_seconds")
        .record(started.elapsed().as_secs_f64());
    metrics::histogram!("buzz_membership_snapshot_response_bytes").record(bytes as f64);
    Ok(Json(envelope))
}

#[cfg(test)]
mod postgres_tests;
