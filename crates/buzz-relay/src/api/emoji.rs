//! Reads of the community's custom emoji catalog (NIP-CE).
//!
//! A catalog can hold 100,000 emoji, and a client wants all of them on hand
//! to complete and resolve shortcodes without a request per keystroke. Nostr
//! filters can only page kind:30180 events by timestamp, which repeats or
//! skips rows written in the same second and returns a full signed event for
//! each emoji. This endpoint reads the catalog table instead: a client asks
//! for the rows written after the last change it holds and gets compact rows
//! in change order.

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use buzz_db::custom_emoji::{CustomEmojiRow, CUSTOM_EMOJI_LOOKUP_MAX, CUSTOM_EMOJI_PAGE_MAX};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::state::AppState;

use super::{api_error, bridge, db_read_error};

/// Route path for catalog reads.
pub(crate) const CATALOG_PATH: &str = "/emoji/catalog";

/// `POST /emoji/catalog` body: either `since` (with optional `limit`) to read
/// changes, or `shortcodes` to look names up.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogRequest {
    /// Read rows written after this change. 0 reads the whole catalog.
    since: Option<i64>,
    /// Rows per page, 1 through [`CUSTOM_EMOJI_PAGE_MAX`].
    limit: Option<u32>,
    /// Shortcodes to look up, at most [`CUSTOM_EMOJI_LOOKUP_MAX`].
    shortcodes: Option<Vec<String>>,
}

/// A validated catalog read.
#[derive(Debug, PartialEq)]
enum CatalogRead {
    Changes { since: i64, limit: u32 },
    Lookup(Vec<String>),
}

fn parse_request(body: &[u8]) -> Result<CatalogRead, (StatusCode, Json<Value>)> {
    let bad = |message: &str| api_error(StatusCode::BAD_REQUEST, message);
    let request: CatalogRequest =
        serde_json::from_slice(body).map_err(|_| bad("invalid emoji catalog JSON"))?;
    match (request.since, request.shortcodes) {
        (Some(since), None) => {
            if since < 0 {
                return Err(bad("since must not be negative"));
            }
            let limit = request.limit.unwrap_or(CUSTOM_EMOJI_PAGE_MAX);
            if !(1..=CUSTOM_EMOJI_PAGE_MAX).contains(&limit) {
                return Err(bad(&format!(
                    "limit must be 1 through {CUSTOM_EMOJI_PAGE_MAX}"
                )));
            }
            Ok(CatalogRead::Changes { since, limit })
        }
        (None, Some(shortcodes)) => {
            if request.limit.is_some() {
                return Err(bad("limit applies only with since"));
            }
            if shortcodes.is_empty() || shortcodes.len() > CUSTOM_EMOJI_LOOKUP_MAX {
                return Err(bad(&format!(
                    "shortcodes must hold 1 through {CUSTOM_EMOJI_LOOKUP_MAX} names"
                )));
            }
            // A name that cannot be a shortcode matches nothing; normalizing
            // lets `:Party_Parrot:` find `party_parrot`.
            Ok(CatalogRead::Lookup(
                shortcodes
                    .iter()
                    .filter_map(|name| buzz_sdk::normalize_custom_emoji_shortcode(name).ok())
                    .collect(),
            ))
        }
        _ => Err(bad("send exactly one of since or shortcodes")),
    }
}

/// Rows as `[shortcode, change, url, author]`, where `url` is null for a
/// removed shortcode and `author` indexes the returned `authors` list. Owners
/// repeat across many rows, so each pubkey is sent once.
fn rows_json(rows: &[CustomEmojiRow]) -> (Vec<Value>, Vec<String>) {
    let mut authors = Vec::new();
    let mut index: HashMap<[u8; 32], usize> = HashMap::new();
    let emoji = rows
        .iter()
        .map(|row| {
            let author = *index.entry(row.pubkey).or_insert_with(|| {
                authors.push(hex::encode(row.pubkey));
                authors.len() - 1
            });
            json!([row.shortcode, row.change, row.url, author])
        })
        .collect();
    (emoji, authors)
}

/// Read the community's custom emoji catalog for an authenticated member.
///
/// With `since`, responds with `{emoji, authors, more}`: the rows written
/// after that change, oldest first. The last row's change is the next
/// `since`; `more` says another page follows. With `shortcodes`, responds
/// with the live rows for those names in the same shape, `more` false.
pub async fn catalog(
    State(state): State<Arc<AppState>>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    catalog_inner(state, slot, headers, body)
        .await
        .into_response()
}

#[allow(clippy::result_large_err)] // Response is the natural error type for axum handlers
async fn catalog_inner(
    state: Arc<AppState>,
    slot: crate::access::AccessSlot,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Response> {
    // Admission first so the denial contract wins over request validation.
    let (tenant, _pubkey) =
        bridge::authenticate_member_post(&state, &slot, &headers, CATALOG_PATH, &body).await?;
    let read = parse_request(&body).map_err(IntoResponse::into_response)?;
    let (rows, more) = match read {
        CatalogRead::Changes { since, limit } => state
            .db
            .custom_emoji_changes(tenant.community(), since, limit)
            .await
            .map_err(|e| db_read_error("emoji catalog read error", &e).into_response())?,
        CatalogRead::Lookup(shortcodes) if shortcodes.is_empty() => (Vec::new(), false),
        CatalogRead::Lookup(shortcodes) => (
            state
                .db
                .custom_emoji_by_shortcode(tenant.community(), &shortcodes)
                .await
                .map_err(|e| db_read_error("emoji catalog lookup error", &e).into_response())?,
            false,
        ),
    };
    let (emoji, authors) = rows_json(&rows);
    Ok(Json(
        json!({ "emoji": emoji, "authors": authors, "more": more }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_takes_exactly_one_of_since_or_shortcodes() {
        assert_eq!(
            parse_request(br#"{"since":0}"#).unwrap(),
            CatalogRead::Changes {
                since: 0,
                limit: CUSTOM_EMOJI_PAGE_MAX
            }
        );
        assert_eq!(
            parse_request(br#"{"since":41,"limit":5}"#).unwrap(),
            CatalogRead::Changes {
                since: 41,
                limit: 5
            }
        );
        assert_eq!(
            parse_request(br#"{"shortcodes":[":Party_Parrot:","not a name","ok"]}"#).unwrap(),
            CatalogRead::Lookup(vec!["party_parrot".to_owned(), "ok".to_owned()])
        );
        let too_many = serde_json::to_vec(&json!({
            "shortcodes": vec!["a"; CUSTOM_EMOJI_LOOKUP_MAX + 1]
        }))
        .unwrap();
        for body in [
            br#"{}"#.to_vec(),
            br#"{"since":-1}"#.to_vec(),
            br#"{"since":0,"limit":0}"#.to_vec(),
            format!(r#"{{"since":0,"limit":{}}}"#, CUSTOM_EMOJI_PAGE_MAX + 1).into_bytes(),
            br#"{"since":0,"shortcodes":["a"]}"#.to_vec(),
            br#"{"shortcodes":[]}"#.to_vec(),
            br#"{"shortcodes":["a"],"limit":1}"#.to_vec(),
            too_many,
            br#"{"since":0,"extra":1}"#.to_vec(),
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

    #[test]
    fn rows_send_each_author_once() {
        let row = |shortcode: &str, change, url: Option<&str>, pubkey| CustomEmojiRow {
            shortcode: shortcode.to_owned(),
            change,
            url: url.map(str::to_owned),
            pubkey: [pubkey; 32],
        };
        let (emoji, authors) = rows_json(&[
            row("a", 1, Some("https://example.com/a.png"), 7),
            row("b", 2, None, 9),
            row("c", 3, Some("https://example.com/c.png"), 7),
        ]);
        assert_eq!(authors, vec!["07".repeat(32), "09".repeat(32)]);
        assert_eq!(
            emoji,
            vec![
                json!(["a", 1, "https://example.com/a.png", 0]),
                json!(["b", 2, null, 1]),
                json!(["c", 3, "https://example.com/c.png", 0]),
            ]
        );
    }
}
