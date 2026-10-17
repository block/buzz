//! Community reads: the directory, member search and lookup, and
//! the message preview before a delete.
//!
//! Each command returns a structured [`AdminReadError`] so the UI can tell a
//! relay without the route, a target absent from the community, and an
//! unknown outcome apart. Every community-scoped call validates its explicit
//! host natively; nothing is derived from the active relay.

use super::error::AdminReadError;
use super::{client, helpers, origin, routes, ERROR_BODY_CAP, SUCCESS_JSON_CAP};

/// GET `url` signed by `keys`, with one fresh-nonce retry on 401.
pub(super) async fn send_admin_read(
    keys: &nostr::Keys,
    url: &str,
    cap: u64,
) -> Result<Vec<u8>, AdminReadError> {
    let http_client = client::ADMIN_CLIENT
        .get()
        .ok_or_else(|| "admin client not initialised".to_string())?;
    let send = || async {
        helpers::build_admin_mutation_request(http_client, keys, &reqwest::Method::GET, url, None)?
            .send()
            .await
            .map_err(|e| crate::relay::classify_request_error(&e))
    };
    let mut resp = send().await?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        resp = send().await?;
    }
    read_admin_read_response(resp, cap, ERROR_BODY_CAP).await
}

/// Read a response under the success/error caps. Only a fully read non-2xx
/// body yields `bodyComplete`, and only then are `bodyEmpty` and `code` known.
pub(super) async fn read_admin_read_response(
    resp: reqwest::Response,
    success_cap: u64,
    error_cap: u64,
) -> Result<Vec<u8>, AdminReadError> {
    use futures_util::StreamExt;

    let status = resp.status();
    if status.is_redirection() {
        return Err(AdminReadError::partial(
            status,
            format!("admin API returned a {status} redirect (not followed)"),
        ));
    }
    let cap = if status.is_success() {
        success_cap
    } else {
        error_cap
    };
    if resp.content_length().is_some_and(|cl| cl > cap) {
        return Err(AdminReadError::partial(
            status,
            format!("admin response too large (cap {cap} bytes)"),
        ));
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            AdminReadError::partial(status, format!("admin response stream error: {e}"))
        })?;
        if bytes.len() as u64 + chunk.len() as u64 > cap {
            return Err(AdminReadError::partial(
                status,
                format!("admin response too large (cap {cap} bytes)"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    if status.is_success() {
        return Ok(bytes);
    }
    let code = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v["error"]["code"].as_str().map(str::to_string));
    Err(AdminReadError {
        message: format!("admin API error: {}", String::from_utf8_lossy(&bytes)),
        relay_status: Some(status.as_u16()),
        body_complete: true,
        body_empty: bytes.is_empty(),
        code,
    })
}

/// Build the URL for `route`, validating the explicit community host.
fn read_url(
    origin: &str,
    route: &routes::AdminRoute,
    mut q: routes::AdminQuery,
) -> Result<String, String> {
    let origin = origin::AdminOrigin::parse(origin)?;
    if let Some(host) = q.community_host.take() {
        q.community_host = Some(
            buzz_core_pkg::tenant::validate_community_host(host.trim())
                .map_err(|e| format!("invalid community host: {e}"))?,
        );
    }
    Ok(origin.route_url(route, &q))
}

async fn read_json(
    url: Result<String, String>,
    state: &crate::app_state::AppState,
) -> Result<serde_json::Value, AdminReadError> {
    let url = url?;
    let keys = state.signing_keys()?;
    let bytes = send_admin_read(&keys, &url, SUCCESS_JSON_CAP).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON from relay: {e}").into())
}

fn hex64(raw: &str, what: &str) -> Result<routes::Hex64, String> {
    routes::Hex64::parse(raw).map_err(|e| format!("invalid {what}: {e}"))
}

/// Community directory — GET /api/admin/v1/communities?q=&cursor=&limit=.
#[tauri::command]
pub async fn admin_list_communities(
    origin: String,
    q: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, AdminReadError> {
    let query = routes::AdminQuery {
        q,
        cursor,
        limit,
        ..Default::default()
    };
    read_json(
        read_url(&origin, &routes::AdminRoute::CommunitiesList, query),
        &state,
    )
    .await
}

/// Name search in one community — GET /members/search?communityHost=&q=&limit=.
#[tauri::command]
pub async fn admin_search_members(
    origin: String,
    community_host: String,
    q: String,
    limit: Option<i64>,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, AdminReadError> {
    let query = routes::AdminQuery {
        community_host: Some(community_host),
        q: Some(q),
        limit,
        ..Default::default()
    };
    read_json(
        read_url(&origin, &routes::AdminRoute::MembersSearch, query),
        &state,
    )
    .await
}

/// One member's state in one community — GET /members/{pubkey}?communityHost=.
#[tauri::command]
pub async fn admin_get_member(
    origin: String,
    community_host: String,
    pubkey: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, AdminReadError> {
    let url = hex64(&pubkey, "member pubkey").and_then(|pubkey| {
        read_url(
            &origin,
            &routes::AdminRoute::MemberDetail { pubkey },
            routes::AdminQuery {
                community_host: Some(community_host),
                ..Default::default()
            },
        )
    });
    read_json(url, &state).await
}

/// Message preview in one community — GET /events/{id}?communityHost=.
#[tauri::command]
pub async fn admin_get_event(
    origin: String,
    community_host: String,
    id: String,
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<serde_json::Value, AdminReadError> {
    let url = hex64(&id, "event id").and_then(|id| {
        read_url(
            &origin,
            &routes::AdminRoute::EventDetail { id },
            routes::AdminQuery {
                community_host: Some(community_host),
                ..Default::default()
            },
        )
    });
    read_json(url, &state).await
}

/// The community host the active relay serves, as the relay binds it.
#[tauri::command]
pub fn admin_connected_community_host(
    state: tauri::State<'_, crate::app_state::AppState>,
) -> Result<String, String> {
    let base = crate::relay::relay_api_base_url_with_override(&state);
    let host = buzz_core_pkg::tenant::relay_url_authority(&base);
    if host.is_empty() {
        return Err("admin_community_host_unresolved".to_string());
    }
    Ok(host)
}

#[cfg(test)]
#[path = "reads_tests.rs"]
mod tests;
