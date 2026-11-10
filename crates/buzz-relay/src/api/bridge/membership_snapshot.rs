//! Opt-in NIP-CL compact discovery; ordinary arrays never assert completeness.

use axum::{http::StatusCode, Json};
use buzz_core::{
    kind::{KIND_MEMBERSHIP_PUBLIC, KIND_MEMBERSHIP_SNAPSHOT},
    TenantContext,
};
use buzz_db::membership_snapshot::{valid_public_types, valid_types, MAX_RESPONSE_BYTES};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{api::api_error, state::AppState};

pub(super) fn requested(filters: &[Value]) -> bool {
    filters
        .iter()
        .any(|f| f.get("membership_snapshot").is_some() || f.get("membership_public").is_some())
}

fn types(
    filters: &[Value],
    viewer: &nostr::PublicKey,
    public: bool,
) -> Result<Vec<String>, &'static str> {
    let types: Vec<String> = filters
        .first()
        .and_then(|f| f.get("#t"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .filter(|types: &Vec<String>| valid_types(types) && (!public || valid_public_types(types)))
        .ok_or("compact membership requires a sorted unique nonempty supported type set")?;
    let expected = json!({
        "kinds": [if public { KIND_MEMBERSHIP_PUBLIC } else { KIND_MEMBERSHIP_SNAPSHOT }],
        "#p": [viewer.to_hex()],
        "#t": types,
        (if public { "membership_public" } else { "membership_snapshot" }): 1,
    });
    if filters != [expected] {
        return Err(
            "compact membership requires one exact version-1 filter for its mode and subject",
        );
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
    let public = filters.iter().any(|f| f.get("membership_public").is_some());
    if public
        && filters
            .iter()
            .any(|f| f.get("membership_snapshot").is_some())
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "membership modes cannot be mixed",
        ));
    }
    for filter in filters {
        for (key, kind) in [
            ("membership_public", KIND_MEMBERSHIP_PUBLIC),
            ("membership_snapshot", KIND_MEMBERSHIP_SNAPSHOT),
        ] {
            if filter.get(key).is_some() && filter.get("kinds") != Some(&json!([kind])) {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    "membership mode does not match response kind",
                ));
            }
        }
    }
    let mode = if public {
        "membership_public"
    } else {
        "membership_snapshot"
    };
    if !(if public {
        state.config.membership_public_enabled
    } else {
        state.config.membership_snapshot_enabled
    }) {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            &format!("{mode} unsupported"),
        ));
    }
    // Even dev mode must provide real NIP-98 for this new authority surface.
    if request_id == [0; 32] {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            &format!("{mode} requires NIP-98"),
        ));
    }
    let subject = if public {
        let subject = filters
            .first()
            .and_then(|f| f.get("#p"))
            .and_then(Value::as_array)
            .filter(|p| p.len() == 1)
            .and_then(|p| p[0].as_str())
            .filter(|p| {
                p.len() == 64
                    && p.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .and_then(|p| nostr::PublicKey::from_hex(p).ok())
            .ok_or_else(|| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    "membership_public requires one lowercase subject key",
                )
            })?;
        subject
    } else {
        *viewer
    };
    let types = types(filters, &subject, public)
        .map_err(|reason| api_error(StatusCode::BAD_REQUEST, reason))?;
    let snapshot = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        if public {
            state
                .db
                .public_memberships(tenant.community(), &subject, &types)
                .await
        } else {
            state
                .db
                .membership_snapshot(tenant.community(), viewer, &types)
                .await
        }
    })
    .await
    .map_err(|_| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("{mode} timed out"),
        )
    })?
    .map_err(|error| match error {
        buzz_db::DbError::MembershipSnapshotTooLarge => api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("{mode} exceeds limit"),
        ),
        _ => {
            tracing::error!(mode, error_kind = %error.kind(), "membership projection failed");
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                &format!("{mode} unavailable"),
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
    let serialized = if public {
        serde_json::to_string(&json!({
            "version": 1, "scope": "public", "community_id": content.community_id,
            "host": content.host, "requester": viewer.to_hex(), "subject": subject.to_hex(),
            "request_id": content.request_id, "types": content.types, "complete": true,
            "snapshot_id": content.snapshot_id, "channels": content.channels,
        }))
    } else {
        serde_json::to_string(&content)
    }
    .map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("{mode} encoding failed"),
        )
    })?;
    let tags = [
        ["d".to_string(), content.request_id],
        ["p".to_string(), subject.to_hex()],
    ]
    .into_iter()
    .map(nostr::Tag::parse)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("{mode} tags failed"),
        )
    })?;
    let event = nostr::EventBuilder::new(
        nostr::Kind::Custom(if public {
            KIND_MEMBERSHIP_PUBLIC
        } else {
            KIND_MEMBERSHIP_SNAPSHOT
        } as u16),
        serialized,
    )
    .tags(tags)
    .sign_with_keys(&state.relay_keypair)
    .map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("{mode} signing failed"),
        )
    })?;
    let envelope = json!({(mode):1, "event":event});
    let bytes = serde_json::to_vec(&envelope)
        .map_err(|_| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("{mode} encoding failed"),
            )
        })?
        .len();
    if bytes > MAX_RESPONSE_BYTES {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!("{mode} exceeds limit"),
        ));
    }
    metrics::histogram!("buzz_membership_snapshot_encode_sign_seconds", "mode" => mode)
        .record(started.elapsed().as_secs_f64());
    metrics::histogram!("buzz_membership_snapshot_response_bytes", "mode" => mode)
        .record(bytes as f64);
    Ok(Json(envelope))
}

#[cfg(test)]
mod postgres_tests;
