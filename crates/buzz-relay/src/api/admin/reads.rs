//! Staff-only, read-only community reads for the Admin Console: the community
//! directory, member search and lookup, and the delete preview.
//!
//! Unlike the legacy report reads, every route here requires a resolved
//! operator or moderator principal, so disabled-auth mode answers 403. Each
//! community-scoped route binds `communityHost` through the tenant binder and
//! reads nothing outside that community.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buzz_db::admin_moderation::{AdminCommunity, AdminEventPreview};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::auth::{authorize_read, lookup_admin_principal, AdminPrincipal};
use super::error::ApiError;
use super::{community_for_host, decode_hex_pubkey};
use crate::state::AppState;

type AppStateRef = State<Arc<AppState>>;

/// Authorize a read and require a staff principal. `authorize()` yields `None`
/// in disabled-auth mode; these routes refuse it instead of serving anonymously.
async fn require_staff(
    state: &AppState,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
) -> Result<AdminPrincipal, ApiError> {
    authorize_read(state, headers, method, uri)
        .await?
        .ok_or_else(ApiError::forbidden)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CommunitiesQuery {
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommunitiesPage {
    items: Vec<AdminCommunity>,
    next_cursor: Option<String>,
}

fn encode_community_cursor(community: &AdminCommunity) -> String {
    let payload = format!("{}_{}", community.id, community.host.to_ascii_lowercase());
    URL_SAFE_NO_PAD.encode(payload)
}

fn decode_community_cursor(token: &str) -> Result<(String, Uuid), ApiError> {
    let invalid = || ApiError::bad_request("invalid_cursor", "cursor is invalid");
    let bytes = URL_SAFE_NO_PAD.decode(token).map_err(|_| invalid())?;
    let payload = String::from_utf8(bytes).map_err(|_| invalid())?;
    let (id, host) = payload.split_once('_').ok_or_else(invalid)?;
    let id = Uuid::parse_str(id).map_err(|_| invalid())?;
    if host.is_empty() {
        return Err(invalid());
    }
    Ok((host.to_owned(), id))
}

/// `GET /communities?q=&cursor=&limit=` — active communities whose host starts
/// with `q`, paged by `(lower(host), id)`.
pub(super) async fn communities(
    State(state): AppStateRef,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<CommunitiesQuery>,
) -> Result<Json<CommunitiesPage>, ApiError> {
    require_staff(&state, &headers, &method, &uri).await?;
    let limit = match query.limit.unwrap_or(50) {
        limit @ 1..=100 => limit,
        _ => {
            return Err(ApiError::bad_request(
                "invalid_limit",
                "limit must be between 1 and 100",
            ))
        }
    };
    let after = query
        .cursor
        .as_deref()
        .map(decode_community_cursor)
        .transpose()?;
    let prefix = query.q.as_deref().unwrap_or("").trim();
    let items = state
        .db
        .admin_list_communities(
            prefix,
            after.as_ref().map(|(h, id)| (h.as_str(), *id)),
            limit,
        )
        .await?;
    let next_cursor = (items.len() as i64 == limit)
        .then(|| items.last().map(encode_community_cursor))
        .flatten();
    Ok(Json(CommunitiesPage { items, next_cursor }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MemberSearchQuery {
    community_host: String,
    q: String,
    limit: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberSearchItem {
    pubkey: String,
    display_name: Option<String>,
    nip05: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Serialize)]
pub(super) struct MemberSearchPage {
    items: Vec<MemberSearchItem>,
}

/// `GET /members/search?communityHost=&q=&limit=` — profile search inside one
/// community. Matches community profiles, so former members are included.
pub(super) async fn search_members(
    State(state): AppStateRef,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<MemberSearchQuery>,
) -> Result<Json<MemberSearchPage>, ApiError> {
    require_staff(&state, &headers, &method, &uri).await?;
    let q = query.q.trim();
    if q.is_empty() || q.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "invalid_query",
            "q must be 1 to 100 characters",
        ));
    }
    let limit = query.limit.unwrap_or(20).clamp(1, 50) as u32;
    let community = community_for_host(&state, &query.community_host).await?;
    let items = state
        .db
        .search_users(community, q, limit)
        .await?
        .into_iter()
        .map(|user| MemberSearchItem {
            pubkey: hex::encode(user.pubkey),
            display_name: user.display_name,
            nip05: user.nip05_handle,
            avatar_url: user.avatar_url,
        })
        .collect();
    Ok(Json(MemberSearchPage { items }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct HostQuery {
    community_host: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberProfile {
    display_name: Option<String>,
    nip05: Option<String>,
    avatar_url: Option<String>,
    about: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MemberLookup {
    pubkey: String,
    profile: Option<MemberProfile>,
    role: Option<String>,
    banned: bool,
    muted_until: Option<DateTime<Utc>>,
    is_staff: bool,
}

/// Whether `pubkey` is deployment staff, using the direct-action staff guard's
/// roster lookup. A lookup failure is an error (500), never "not staff".
async fn is_staff(state: &AppState, pubkey: &[u8]) -> Result<bool, ApiError> {
    let pubkey: [u8; 32] = pubkey.try_into().map_err(|_| ApiError::internal())?;
    Ok(lookup_admin_principal(state, pubkey).await?.is_some())
}

/// `GET /members/{pubkey}?communityHost=` — profile, community role, current
/// restriction and staff status for one pubkey in one community.
pub(super) async fn lookup_member(
    State(state): AppStateRef,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(pubkey_hex): Path<String>,
    Query(query): Query<HostQuery>,
) -> Result<Json<MemberLookup>, ApiError> {
    require_staff(&state, &headers, &method, &uri).await?;
    let pubkey = decode_hex_pubkey(&pubkey_hex)?;
    let pubkey_hex = hex::encode(&pubkey);
    let community = community_for_host(&state, &query.community_host).await?;
    let profile = state.db.get_user(community, &pubkey).await?;
    let role = state.db.get_relay_member(community, &pubkey_hex).await?;
    let restriction = state
        .db
        .moderation_restriction_state(community, &pubkey)
        .await?;
    Ok(Json(MemberLookup {
        profile: profile.map(|p| MemberProfile {
            display_name: p.display_name,
            nip05: p.nip05_handle,
            avatar_url: p.avatar_url,
            about: p.about,
        }),
        role: role.map(|member| member.role),
        banned: restriction.banned,
        muted_until: restriction.muted_until,
        is_staff: is_staff(&state, &pubkey).await?,
        pubkey: pubkey_hex,
    }))
}

/// `GET /events/{id}?communityHost=` — one event inside one community, for the
/// delete preview. An id stored only in another community is `event_not_found`.
pub(super) async fn event_preview(
    State(state): AppStateRef,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    Path(id_hex): Path<String>,
    Query(query): Query<HostQuery>,
) -> Result<Json<AdminEventPreview>, ApiError> {
    require_staff(&state, &headers, &method, &uri).await?;
    let id = decode_hex_pubkey(&id_hex)?;
    let community = community_for_host(&state, &query.community_host).await?;
    state
        .db
        .admin_get_event_preview(*community.as_uuid(), &id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError {
            status: StatusCode::NOT_FOUND,
            code: "event_not_found",
            message: "event was not found in this community".to_owned(),
        })
}

#[cfg(test)]
#[path = "reads_tests.rs"]
mod postgres_tests;
