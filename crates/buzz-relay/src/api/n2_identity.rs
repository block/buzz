//! Secret-authenticated provisioning for N2-managed staff identities.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use hmac::{Hmac, KeyInit, Mac};
use nostr::{EventBuilder, Keys, Kind, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use uuid::Uuid;

use crate::{handlers::event::dispatch_persistent_event, state::AppState};

use super::{api_error, internal_error};

const SECRET_HEADER: &str = "x-n2-sync-secret";
const MAX_PROFILE_FIELD_LENGTH: usize = 255;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// N2-owned profile fields projected into Buzz.
pub struct N2UserSyncRequest {
    /// Stable N2 login name.
    pub username: String,
    /// Human-readable staff name.
    pub display_name: String,
}

#[derive(Debug, Serialize)]
/// Stable Buzz identity returned to the N2 bridge.
pub struct N2UserSyncResponse {
    /// Immutable N2 user identifier supplied in the route.
    pub n2_user_id: Uuid,
    /// Community-scoped Nostr public key for genuine Buzz mentions.
    pub pubkey: String,
    /// Normalized username persisted in the signed profile.
    pub username: String,
    /// Normalized display name persisted in the signed profile.
    pub display_name: String,
}

/// Upsert an N2 staff profile as a server-managed Buzz identity.
pub async fn sync_user(
    State(state): State<Arc<AppState>>,
    Path(n2_user_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<N2UserSyncRequest>,
) -> Result<Json<N2UserSyncResponse>, (StatusCode, Json<serde_json::Value>)> {
    let config = state
        .config
        .n2_identity_sync
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "not found"))?;
    let supplied_secret = headers
        .get(SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !crate::webhook_secret::verify_secret(supplied_secret, &config.sync_secret) {
        return Err(api_error(StatusCode::UNAUTHORIZED, "authentication failed"));
    }

    let username = validate_profile_field("username", &request.username)?;
    let display_name = validate_profile_field("display_name", &request.display_name)?;

    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| api_error(StatusCode::NOT_FOUND, "not found"))?;

    let keys = derive_n2_identity(
        config.derivation_key.as_bytes(),
        tenant.community().as_uuid(),
        n2_user_id,
    )?;
    let pubkey = keys.public_key();
    let pubkey_hex = pubkey.to_hex();
    let pubkey_bytes = pubkey.to_bytes().to_vec();
    let metadata = serde_json::json!({
        "name": username,
        "display_name": display_name,
        "about": "N2 staff identity managed by the N2 directory bridge.",
        "n2_user_id": n2_user_id,
    });
    let event = EventBuilder::new(Kind::Metadata, metadata.to_string())
        .sign_with_keys(&keys)
        .map_err(|error| internal_error(&format!("failed to sign N2 identity: {error}")))?;

    state
        .db
        .ensure_user(tenant.community(), &pubkey_bytes)
        .await
        .map_err(|error| internal_error(&format!("failed to create N2 identity: {error}")))?;
    state
        .db
        .update_user_profile(
            tenant.community(),
            &pubkey_bytes,
            Some(display_name),
            None,
            Some("N2 staff identity managed by the N2 directory bridge."),
            None,
        )
        .await
        .map_err(|error| internal_error(&format!("failed to update N2 identity: {error}")))?;

    let (stored, inserted) = state
        .db
        .replace_addressable_event(tenant.community(), &event, None)
        .await
        .map_err(|error| internal_error(&format!("failed to store N2 identity: {error}")))?;
    if inserted {
        dispatch_persistent_event(&tenant, &state, &stored, 0, &pubkey_hex, None).await;
    }

    Ok(Json(N2UserSyncResponse {
        n2_user_id,
        pubkey: pubkey_hex,
        username: username.to_owned(),
        display_name: display_name.to_owned(),
    }))
}

fn validate_profile_field<'a>(
    field: &str,
    value: &'a str,
) -> Result<&'a str, (StatusCode, Json<serde_json::Value>)> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_PROFILE_FIELD_LENGTH {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            &format!("{field} must contain 1..={MAX_PROFILE_FIELD_LENGTH} bytes"),
        ));
    }
    Ok(value)
}

fn derive_n2_identity(
    derivation_key: &[u8],
    community_id: &Uuid,
    n2_user_id: Uuid,
) -> Result<Keys, (StatusCode, Json<serde_json::Value>)> {
    for counter in 0u32..=u32::MAX {
        let mut hmac = Hmac::<Sha256>::new_from_slice(derivation_key)
            .map_err(|_| internal_error("invalid N2 identity derivation key"))?;
        hmac.update(b"buzz:n2-identity:v1");
        hmac.update(community_id.as_bytes());
        hmac.update(n2_user_id.as_bytes());
        hmac.update(&counter.to_be_bytes());
        let candidate = hmac.finalize().into_bytes();
        if let Ok(secret_key) = SecretKey::from_slice(&candidate) {
            return Ok(Keys::new(secret_key));
        }
    }

    Err(internal_error("failed to derive N2 identity"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_a_stable_community_scoped_identity() {
        let key = b"0123456789abcdef0123456789abcdef";
        let community_a = Uuid::from_u128(1);
        let community_b = Uuid::from_u128(2);
        let user_id = Uuid::from_u128(3);

        let first = derive_n2_identity(key, &community_a, user_id)
            .expect("first identity")
            .public_key();
        let repeated = derive_n2_identity(key, &community_a, user_id)
            .expect("repeated identity")
            .public_key();
        let other_community = derive_n2_identity(key, &community_b, user_id)
            .expect("other community identity")
            .public_key();

        assert_eq!(first, repeated);
        assert_ne!(first, other_community);
    }

    #[test]
    fn rejects_empty_and_oversized_profile_fields() {
        assert!(validate_profile_field("username", " ").is_err());
        assert!(
            validate_profile_field("display_name", &"x".repeat(MAX_PROFILE_FIELD_LENGTH + 1))
                .is_err()
        );
    }
}
