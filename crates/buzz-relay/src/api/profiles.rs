//! Relay-owned profile name search.
//!
//! NIP-50 search over kind:0 matches the full profile JSON, so text inside
//! avatar data URLs, `about` and `nip05` competes with real names. This
//! endpoint keeps NIP-50 prefix matching and paging but applies them only to
//! the `name` and `display_name` fields of each current profile, and returns
//! the signed kind:0 events.

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use buzz_search::PROFILE_NAMES_PER_PAGE_MAX;
use serde::Deserialize;
use serde_json::Value;

use crate::state::AppState;

use super::{api_error, bridge, db_read_error, internal_error};

/// Route path for profile name search.
pub(crate) const SEARCH_PATH: &str = "/profiles/search";
/// Longest accepted search text, in characters.
const QUERY_MAX_CHARS: usize = 100;
/// Page size when the request omits `limit`.
const DEFAULT_LIMIT: u32 = 20;
/// Highest accepted page, matching the NIP-50 search page ceiling.
const PAGE_MAX: u32 = 1000;

/// `POST /profiles/search` body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchRequest {
    /// Text to find in `name` or `display_name`.
    query: String,
    /// Results per page, 1 through [`PROFILE_NAMES_PER_PAGE_MAX`].
    limit: Option<u32>,
    /// 1-based page of `limit` results, 1 through [`PAGE_MAX`].
    page: Option<u32>,
}

/// A validated search: text, page size and page.
type Search = (String, u32, u32);

fn parse_request(body: &[u8]) -> Result<Search, (StatusCode, Json<Value>)> {
    let request: SearchRequest = serde_json::from_slice(body)
        .map_err(|_| api_error(StatusCode::BAD_REQUEST, "invalid profile search JSON"))?;
    let query = request.query.trim();
    if query.is_empty() || query.chars().count() > QUERY_MAX_CHARS {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            &format!("query must be 1 through {QUERY_MAX_CHARS} characters"),
        ));
    }
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=PROFILE_NAMES_PER_PAGE_MAX).contains(&limit) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            &format!("limit must be 1 through {PROFILE_NAMES_PER_PAGE_MAX}"),
        ));
    }
    let page = request.page.unwrap_or(1);
    if !(1..=PAGE_MAX).contains(&page) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            &format!("page must be 1 through {PAGE_MAX}"),
        ));
    }
    Ok((query.to_owned(), limit, page))
}

/// Search current profiles by `name` and `display_name` for an authenticated
/// member. Responds with one page of signed kind:0 events, best match first.
pub async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    search_inner(state, headers, body).await.into_response()
}

#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn search_inner(
    state: Arc<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Response> {
    // Admission first so the denial contract wins over request validation.
    // [FI-TRACE-HTTP-INGRESS]
    let (tenant, pubkey) =
        bridge::authenticate_member_post(&state, &headers, SEARCH_PATH, &body).await?;
    let (query, limit, page) = parse_request(&body).map_err(IntoResponse::into_response)?;

    let hits = state
        .search
        .search_profile_names(tenant.community(), &query, page, limit)
        .await
        .map_err(|e| internal_error(&format!("profile search error: {e}")).into_response())?
        .hits;
    if hits.is_empty() {
        return Ok(Json(Value::Array(Vec::new())));
    }
    let ids: Vec<&[u8]> = hits.iter().map(|hit| hit.event_id.as_slice()).collect();
    let stored = state
        .db
        .get_events_by_ids_routed("profile_search_hydrate", tenant.community(), &ids)
        .await
        .map_err(|e| db_read_error("profile search fetch error", &e).into_response())?;
    let mut by_id: HashMap<[u8; 32], &buzz_core::StoredEvent> = stored
        .iter()
        .map(|stored| (stored.event.id.to_bytes(), stored))
        .collect();

    // Keep search order. Search never widens visibility: apply the shared
    // per-event read gate.
    let reader = pubkey.to_bytes();
    let events = hits
        .iter()
        .filter_map(|hit| by_id.remove(&hit.event_id))
        .filter(|stored| crate::handlers::req::event_visible_to_reader(&stored.event, &reader))
        .filter_map(|stored| serde_json::to_value(&stored.event).ok())
        .collect();
    Ok(Json(Value::Array(events)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_validation_bounds_query_and_limit() {
        assert_eq!(
            parse_request(br#"{"query":"  Wes  "}"#).unwrap(),
            ("Wes".to_owned(), DEFAULT_LIMIT, 1)
        );
        assert_eq!(
            parse_request(br#"{"query":"w","limit":100,"page":1000}"#).unwrap(),
            ("w".to_owned(), 100, 1000)
        );
        let long = "x".repeat(QUERY_MAX_CHARS + 1);
        for body in [
            br#"{"query":"   "}"#.to_vec(),
            format!(r#"{{"query":"{long}"}}"#).into_bytes(),
            br#"{"query":"w","limit":0}"#.to_vec(),
            br#"{"query":"w","limit":101}"#.to_vec(),
            br#"{"query":"w","page":0}"#.to_vec(),
            br#"{"query":"w","page":1001}"#.to_vec(),
            br#"{"query":"w","search_mode":"prefix"}"#.to_vec(),
            br#"{"q":"w"}"#.to_vec(),
            b"not json".to_vec(),
        ] {
            let (status, _) = parse_request(&body).unwrap_err();
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{}",
                String::from_utf8_lossy(&body)
            );
        }
    }
}
