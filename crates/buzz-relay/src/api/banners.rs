//! Client-facing relay banner API.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};
use buzz_core::TenantContext;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::{api_error, bridge, internal_error, relay_members};
use crate::state::AppState;

/// Client route for retrieving the active banner for the authenticated user.
pub(crate) const BANNER_ACTIVE_PATH: &str = "/api/banners/current";
/// Client route for acknowledging a successful banner render/view.
pub(crate) const BANNER_VIEW_ROUTE: &str = "/api/banners/{banner_id}/view";
/// Header carrying a client-stable UUID for idempotent view retries.
pub(crate) const BANNER_VIEW_ID_HEADER: &str = "x-buzz-banner-view-id";
/// Client route for dismissing a banner.
pub(crate) const BANNER_DISMISS_ROUTE: &str = "/api/banners/{banner_id}/dismiss";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BannerResponse {
    /// Public banner id.
    pub id: String,
    /// Banner event kind clients may also query over Nostr.
    pub kind: u32,
    /// Banner severity/type.
    pub severity: &'static str,
    /// Plain-text banner text.
    pub text: String,
    /// Number of successful views each user may receive before suppression.
    pub max_displays: i32,
    /// Every v1 severity is dismissible.
    pub dismissible: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BannerAckResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    changed: Option<bool>,
}

impl From<buzz_db::RelayBannerRecord> for BannerResponse {
    fn from(value: buzz_db::RelayBannerRecord) -> Self {
        Self {
            id: value.public_id.to_string(),
            kind: buzz_core::kind::KIND_RELAY_BANNER,
            severity: value.severity.as_str(),
            text: value.message,
            max_displays: value.max_displays,
            dismissible: true,
        }
    }
}

pub(crate) fn banner_event(
    relay_keypair: &nostr::Keys,
    banner: &buzz_db::RelayBannerRecord,
    created_at: Option<nostr::Timestamp>,
) -> Result<nostr::Event, String> {
    let id = banner.public_id.to_string();
    let scope = if banner.target_all_communities {
        "all"
    } else {
        "communities"
    };
    let content = serde_json::json!({
        "id": id,
        "severity": banner.severity.as_str(),
        "text": banner.message,
        "maxDisplays": banner.max_displays,
        "dismissible": true,
    })
    .to_string();
    let tags = vec![
        nostr::Tag::parse(["d", id.as_str()]).map_err(|e| e.to_string())?,
        nostr::Tag::parse(["scope", scope]).map_err(|e| e.to_string())?,
    ];
    let builder = nostr::EventBuilder::new(
        nostr::Kind::Custom(buzz_core::kind::KIND_RELAY_BANNER as u16),
        content,
    )
    .tags(tags);
    let builder = if let Some(created_at) = created_at {
        builder.custom_created_at(created_at)
    } else {
        builder
    };
    builder
        .sign_with_keys(relay_keypair)
        .map_err(|e| e.to_string())
}

pub(crate) fn banner_disabled_event(
    relay_keypair: &nostr::Keys,
    banner: &buzz_db::RelayBannerRecord,
    created_at: Option<nostr::Timestamp>,
) -> Result<nostr::Event, String> {
    let id = banner.public_id.to_string();
    let scope = if banner.target_all_communities {
        "all"
    } else {
        "communities"
    };
    let tags = vec![
        nostr::Tag::parse(["d", id.as_str()]).map_err(|e| e.to_string())?,
        nostr::Tag::parse(["scope", scope]).map_err(|e| e.to_string())?,
        nostr::Tag::parse(["status", "disabled"]).map_err(|e| e.to_string())?,
    ];
    let builder = nostr::EventBuilder::new(
        nostr::Kind::Custom(buzz_core::kind::KIND_RELAY_BANNER as u16),
        "",
    )
    .tags(tags);
    let builder = if let Some(created_at) = created_at {
        builder.custom_created_at(created_at)
    } else {
        builder
    };
    builder
        .sign_with_keys(relay_keypair)
        .map_err(|e| e.to_string())
}

pub(crate) async fn active_banner_event_for_user(
    state: &AppState,
    tenant: &TenantContext,
    pubkey_bytes: &[u8],
) -> Result<Option<nostr::Event>, String> {
    let Some(banner) = state
        .db
        .active_relay_banner_for_user(tenant.community(), pubkey_bytes)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    banner_event(&state.relay_keypair, &banner, None).map(Some)
}

pub(crate) async fn get_active_banner(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Option<BannerResponse>>, (axum::http::StatusCode, Json<Value>)> {
    let (tenant, pubkey, event_id, signed_created_at) =
        authenticate_client_request(&state, &headers, "GET", BANNER_ACTIVE_PATH, None, false)
            .await?;
    bridge::check_nip98_replay(&state, &tenant, event_id).await?;
    enforce_member(&state, &tenant, &headers, &pubkey, signed_created_at).await?;

    let banner = state
        .db
        .active_relay_banner_for_user(tenant.community(), pubkey.as_bytes())
        .await
        .map_err(|e| internal_error(&format!("banner lookup: {e}")))?
        .map(BannerResponse::from);
    Ok(Json(banner))
}

pub(crate) async fn ack_banner_view(
    State(state): State<Arc<AppState>>,
    Path(banner_id): Path<uuid::Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<BannerAckResponse>, (axum::http::StatusCode, Json<Value>)> {
    let (tenant, pubkey, event_id, signed_created_at) = authenticate_client_request(
        &state,
        &headers,
        "POST",
        &format!("/api/banners/{banner_id}/view"),
        Some(&body),
        true,
    )
    .await?;
    bridge::check_nip98_replay(&state, &tenant, event_id).await?;
    enforce_member(&state, &tenant, &headers, &pubkey, signed_created_at).await?;
    if !body.is_empty() {
        return Err(non_empty_ack_body_error());
    }
    let view_id = parse_view_id(&headers)?;
    match state
        .db
        .ack_relay_banner_view(tenant.community(), banner_id, pubkey.as_bytes(), view_id)
        .await
        .map_err(|e| internal_error(&format!("banner view ack: {e}")))?
    {
        buzz_db::RelayBannerViewOutcome::Accepted { changed, .. } => {
            if changed {
                metrics::counter!(
                    "buzz_relay_banner_views_total",
                    "community" => tenant.host().to_owned()
                )
                .increment(1);
            }
            Ok(Json(BannerAckResponse {
                status: "accepted",
                changed: Some(changed),
            }))
        }
        buzz_db::RelayBannerViewOutcome::Dismissed => Ok(Json(BannerAckResponse {
            status: "dismissed",
            changed: None,
        })),
        buzz_db::RelayBannerViewOutcome::Exhausted { .. } => Ok(Json(BannerAckResponse {
            status: "exhausted",
            changed: None,
        })),
        buzz_db::RelayBannerViewOutcome::NotEligible => Err(api_error(
            axum::http::StatusCode::NOT_FOUND,
            "banner not eligible",
        )),
    }
}

pub(crate) async fn ack_banner_dismiss(
    State(state): State<Arc<AppState>>,
    Path(banner_id): Path<uuid::Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<BannerAckResponse>, (axum::http::StatusCode, Json<Value>)> {
    let (tenant, pubkey, event_id, signed_created_at) = authenticate_client_request(
        &state,
        &headers,
        "POST",
        &format!("/api/banners/{banner_id}/dismiss"),
        Some(&body),
        true,
    )
    .await?;
    bridge::check_nip98_replay(&state, &tenant, event_id).await?;
    enforce_member(&state, &tenant, &headers, &pubkey, signed_created_at).await?;
    if !body.is_empty() {
        return Err(non_empty_ack_body_error());
    }
    match state
        .db
        .ack_relay_banner_dismiss(tenant.community(), banner_id, pubkey.as_bytes())
        .await
        .map_err(|e| internal_error(&format!("banner dismiss ack: {e}")))?
    {
        buzz_db::RelayBannerDismissOutcome::Dismissed { changed } => {
            if changed {
                metrics::counter!(
                    "buzz_relay_banner_dismissals_total",
                    "community" => tenant.host().to_owned()
                )
                .increment(1);
            }
            Ok(Json(BannerAckResponse {
                status: "dismissed",
                changed: Some(changed),
            }))
        }
        buzz_db::RelayBannerDismissOutcome::NotEligible => Err(api_error(
            axum::http::StatusCode::NOT_FOUND,
            "banner not eligible",
        )),
    }
}

async fn authenticate_client_request(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    require_payload: bool,
) -> Result<
    (TenantContext, nostr::PublicKey, [u8; 32], Option<u64>),
    (axum::http::StatusCode, Json<Value>),
> {
    let raw_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                axum::http::StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    let verified = bridge::verify_bridge_auth_with_options(
        headers,
        method,
        &url,
        body,
        state.config.require_auth_token,
        require_payload,
    )?;
    bridge::enforce_http_admission(state, &tenant, &verified.pubkey).await?;
    Ok((
        tenant,
        verified.pubkey,
        verified.event_id_bytes,
        verified.signed_created_at,
    ))
}

fn parse_view_id(headers: &HeaderMap) -> Result<Uuid, (axum::http::StatusCode, Json<Value>)> {
    let Some(raw) = headers
        .get(BANNER_VIEW_ID_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return Err(invalid_view_id_error());
    };
    let Ok(view_id) = Uuid::parse_str(raw) else {
        return Err(invalid_view_id_error());
    };
    if raw != view_id.hyphenated().to_string() || view_id.is_nil() {
        return Err(invalid_view_id_error());
    }
    Ok(view_id)
}

fn invalid_view_id_error() -> (axum::http::StatusCode, Json<Value>) {
    api_error(
        axum::http::StatusCode::BAD_REQUEST,
        "missing or invalid banner view id",
    )
}

fn non_empty_ack_body_error() -> (axum::http::StatusCode, Json<Value>) {
    api_error(
        axum::http::StatusCode::BAD_REQUEST,
        "banner acknowledgement body must be empty",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner_record(target_all_communities: bool) -> buzz_db::RelayBannerRecord {
        buzz_db::RelayBannerRecord {
            id: 42,
            public_id: uuid::Uuid::from_u128(0x12345678123456781234567812345678),
            severity: buzz_db::RelayBannerSeverity::Warning,
            message: "scheduled maintenance".to_owned(),
            max_displays: 3,
            target_all_communities,
            community_ids: Vec::new(),
            created_at: chrono::DateTime::UNIX_EPOCH,
            updated_at: chrono::DateTime::UNIX_EPOCH,
            created_by: vec![7; 32],
            disabled_at: None,
        }
    }

    #[test]
    fn banner_event_uses_published_contract() {
        let keys = nostr::Keys::generate();
        let banner = banner_record(false);
        let event = banner_event(&keys, &banner, None).expect("banner event");

        assert_eq!(
            event.kind.as_u16() as u32,
            buzz_core::kind::KIND_RELAY_BANNER
        );
        assert_eq!(buzz_core::kind::KIND_RELAY_BANNER, 13536);
        assert_eq!(
            event
                .tags
                .iter()
                .find(|tag| tag.kind().to_string() == "d")
                .and_then(|tag| tag.content()),
            Some(banner.public_id.to_string().as_str())
        );
        assert_eq!(
            event
                .tags
                .iter()
                .find(|tag| tag.kind().to_string() == "scope")
                .and_then(|tag| tag.content()),
            Some("communities")
        );

        let content: serde_json::Value =
            serde_json::from_str(&event.content).expect("banner JSON content");
        assert_eq!(content["id"], banner.public_id.to_string());
        assert_eq!(content["severity"], "warning");
        assert_eq!(content["text"], "scheduled maintenance");
        assert_eq!(content["maxDisplays"], 3);
        assert_eq!(content["dismissible"], true);
        assert!(content.get("message").is_none());
    }

    #[test]
    fn disabled_banner_event_uses_clear_contract() {
        let keys = nostr::Keys::generate();
        let banner = banner_record(false);
        let event = banner_disabled_event(&keys, &banner, None).expect("disabled event");

        assert_eq!(
            event.kind.as_u16() as u32,
            buzz_core::kind::KIND_RELAY_BANNER
        );
        assert!(event.content.is_empty());
        assert_eq!(
            event
                .tags
                .iter()
                .find(|tag| tag.kind().to_string() == "scope")
                .and_then(|tag| tag.content()),
            Some("communities")
        );
        assert_eq!(
            event
                .tags
                .iter()
                .find(|tag| tag.kind().to_string() == "status")
                .and_then(|tag| tag.content()),
            Some("disabled")
        );
        assert!(event
            .tags
            .iter()
            .all(|tag| tag.kind().to_string() != "text"));
    }

    #[test]
    fn all_community_banner_event_has_all_scope() {
        let keys = nostr::Keys::generate();
        let banner = banner_record(true);
        let event = banner_event(&keys, &banner, None).expect("banner event");

        assert_eq!(
            event
                .tags
                .iter()
                .find(|tag| tag.kind().to_string() == "scope")
                .and_then(|tag| tag.content()),
            Some("all")
        );
    }

    #[test]
    fn banner_ack_routes_are_uuid_path_routes() {
        assert_eq!(BANNER_VIEW_ROUTE, "/api/banners/{banner_id}/view");
        assert_eq!(BANNER_DISMISS_ROUTE, "/api/banners/{banner_id}/dismiss");
    }

    #[test]
    fn parse_view_id_requires_canonical_non_nil_uuid() {
        let mut headers = HeaderMap::new();
        assert!(parse_view_id(&headers).is_err());

        headers.insert(BANNER_VIEW_ID_HEADER, "not-a-uuid".parse().expect("header"));
        assert!(parse_view_id(&headers).is_err());

        headers.insert(
            BANNER_VIEW_ID_HEADER,
            "00000000-0000-0000-0000-000000000000"
                .parse()
                .expect("header"),
        );
        assert!(parse_view_id(&headers).is_err());

        headers.insert(
            BANNER_VIEW_ID_HEADER,
            "12345678123456781234567812345678".parse().expect("header"),
        );
        assert!(parse_view_id(&headers).is_err());

        headers.insert(
            BANNER_VIEW_ID_HEADER,
            "12345678-1234-5678-1234-567812345678"
                .parse()
                .expect("header"),
        );
        assert_eq!(
            parse_view_id(&headers).expect("valid canonical uuid"),
            Uuid::parse_str("12345678-1234-5678-1234-567812345678").expect("uuid")
        );
    }

    #[test]
    fn non_empty_ack_body_is_rejected() {
        let (status, body) = non_empty_ack_body_error();

        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "banner acknowledgement body must be empty");
    }
}

async fn enforce_member(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: &nostr::PublicKey,
    signed_created_at: Option<u64>,
) -> Result<(), (axum::http::StatusCode, Json<Value>)> {
    let auth_tag = relay_members::extract_auth_tag_header(headers);
    relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        pubkey.as_bytes(),
        auth_tag,
        signed_created_at,
    )
    .await
    .map(|_| ())
}
