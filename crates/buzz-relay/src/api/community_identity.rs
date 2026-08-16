//! Host-scoped discovery of a channel's parent community identity.
//!
//! The endpoint is intentionally narrow: the request host binds the tenant
//! before the requested channel is checked, and the response contains only the
//! two UUIDs needed to pin an external launch contract. It exposes neither
//! channel metadata nor membership data.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::Serialize;
use uuid::Uuid;

use crate::state::AppState;

/// Response returned by [`channel_community`].
#[derive(Debug, Serialize, PartialEq, Eq)]
struct ChannelCommunityResponse {
    schema: &'static str,
    channel_id: Uuid,
    community_id: Uuid,
}

/// Return the parent community UUID for one active channel on this request host.
///
/// `GET /.well-known/buzz/channels/{channel_id}/community` is a deployment
/// bootstrap read. The request is bound to its host-derived tenant first, so a
/// channel UUID from another community cannot resolve here. Unknown hosts and
/// unknown or deleted channels both return the same generic 404 response.
pub async fn channel_community(
    State(state): State<Arc<AppState>>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = match crate::tenant::bind_community(&state.db, raw_host).await {
        Ok(tenant) => tenant,
        Err(_) => return not_found(),
    };

    if state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .is_err()
    {
        return not_found();
    }

    Json(ChannelCommunityResponse {
        schema: "buzz.channel-community/v1",
        channel_id,
        community_id: *tenant.community().as_uuid(),
    })
    .into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "relay: channel is not available on this host").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_contains_only_the_versioned_channel_community_binding() {
        let response = ChannelCommunityResponse {
            schema: "buzz.channel-community/v1",
            channel_id: Uuid::from_u128(1),
            community_id: Uuid::from_u128(2),
        };

        assert_eq!(
            serde_json::to_value(response).expect("response should serialize"),
            serde_json::json!({
                "schema": "buzz.channel-community/v1",
                "channel_id": "00000000-0000-0000-0000-000000000001",
                "community_id": "00000000-0000-0000-0000-000000000002"
            })
        );
    }
}
