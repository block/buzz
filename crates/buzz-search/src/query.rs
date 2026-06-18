//! Search query building and result parsing.

use serde::Deserialize;
use tracing::debug;

use crate::error::SearchError;

/// Parameters for a Typesense search request.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// The search query string (`"*"` matches all documents).
    pub q: String,
    /// Optional Typesense filter expression (e.g. `"kind:=1"`).
    pub filter_by: Option<String>,
    /// Optional sort expression (e.g. `"created_at:desc"`).
    pub sort_by: Option<String>,
    /// Page number (1-indexed).
    pub page: u32,
    /// Number of results per page.
    pub per_page: u32,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            q: "*".into(),
            filter_by: None,
            sort_by: Some("created_at:desc".into()),
            page: 1,
            per_page: 20,
        }
    }
}

impl SearchQuery {
    /// Converts the query into Typesense HTTP query parameters.
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = vec![
            ("q".into(), self.q.clone()),
            ("query_by".into(), "content".into()),
            ("page".into(), self.page.to_string()),
            ("per_page".into(), self.per_page.to_string()),
        ];

        if let Some(ref filter) = self.filter_by {
            params.push(("filter_by".into(), filter.clone()));
        }

        if let Some(ref sort) = self.sort_by {
            params.push(("sort_by".into(), sort.clone()));
        }

        params
    }
}

/// A single search result hit.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// Hex event ID of the matching event.
    pub event_id: String,
    /// Event content text **as indexed in Typesense** — not necessarily the
    /// canonical event content.
    ///
    /// For kind:0 (user metadata) events, `flatten_kind0_for_indexing` in
    /// `index.rs` appends the parsed `display_name` / `name` / `nip05` values
    /// to the original JSON content (space-separated) so the default
    /// tokenizer can produce clean word tokens. That doctored string is what
    /// lands here.
    ///
    /// All production read paths (`bridge.rs::handle_bridge_search`,
    /// `handlers/req.rs` WS REQ) refetch the canonical `StoredEvent` from
    /// Postgres by `event_id` and ignore this field — which is why the
    /// append-to-content trick is safe. If you're adding a new feature that
    /// reads this field directly, do the same: fetch the canonical event by
    /// id rather than trusting `content` to round-trip.
    pub content: String,
    /// Nostr kind number.
    pub kind: u16,
    /// Hex public key of the event author.
    pub pubkey: String,
    /// Channel UUID string, if the event is scoped to a channel.
    pub channel_id: Option<String>,
    /// Unix timestamp of event creation.
    pub created_at: i64,
    /// Typesense relevance score.
    pub score: f64,
}

/// The result of a search query.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Matching hits for this page.
    pub hits: Vec<SearchHit>,
    /// Total number of matching documents across all pages.
    pub found: u64,
    /// Current page number.
    pub page: u32,
}

#[derive(Debug, Deserialize)]
struct TypesenseMultiSearchResponse {
    results: Vec<TypesenseSearchResponse>,
}

#[derive(Debug, Deserialize)]
struct TypesenseSearchResponse {
    found: u64,
    page: u32,
    hits: Vec<TypesenseHit>,
}

#[derive(Debug, Deserialize)]
struct TypesenseHit {
    document: TypesenseDocument,
    #[serde(rename = "text_match")]
    text_match: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TypesenseDocument {
    id: String,
    content: String,
    kind: i32,
    pubkey: String,
    channel_id: Option<String>,
    created_at: i64,
}

/// Executes a search query against Typesense and returns parsed results.
pub async fn search(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    collection_name: &str,
    query: &SearchQuery,
) -> Result<SearchResult, SearchError> {
    debug!(
        q = %query.q,
        page = query.page,
        per_page = query.per_page,
        collection = collection_name,
        "Executing search"
    );

    // Typesense GET search has a 4000-char query string limit. When filter_by
    // contains hundreds of channel UUIDs, the URL exceeds this. Use the
    // /multi_search POST endpoint which accepts the same params in a JSON body.
    let url = format!("{}/multi_search", base_url);
    let mut search_params = serde_json::json!({
        "collection": collection_name,
        "q": query.q,
        "query_by": "content",
        "page": query.page,
        "per_page": query.per_page,
    });
    if let Some(ref filter) = query.filter_by {
        search_params["filter_by"] = serde_json::Value::String(filter.clone());
    }
    if let Some(ref sort) = query.sort_by {
        search_params["sort_by"] = serde_json::Value::String(sort.clone());
    }
    let body = serde_json::json!({ "searches": [search_params] });

    let resp = client
        .post(&url)
        .header("X-TYPESENSE-API-KEY", api_key)
        .json(&body)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(SearchError::Api { status, body });
    }

    // multi_search wraps results: {"results": [<search_response>]}
    let wrapper: TypesenseMultiSearchResponse = resp.json().await?;
    let ts_resp = wrapper.results.into_iter().next().ok_or(SearchError::Api {
        status: 200,
        body: "empty multi_search results".into(),
    })?;
    parse_response(ts_resp)
}

fn parse_response(ts_resp: TypesenseSearchResponse) -> Result<SearchResult, SearchError> {
    let hits = ts_resp
        .hits
        .into_iter()
        .map(|hit| {
            // Raw Typesense text_match relevance score (not normalized).
            let score = hit.text_match.unwrap_or(0) as f64;
            SearchHit {
                event_id: hit.document.id,
                content: hit.document.content,
                kind: u16::try_from(hit.document.kind).unwrap_or(0),
                pubkey: hit.document.pubkey,
                channel_id: hit.document.channel_id.filter(|id| id != "__global__"),
                created_at: hit.document.created_at,
                score,
            }
        })
        .collect();

    Ok(SearchResult {
        hits,
        found: ts_resp.found,
        page: ts_resp.page,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_search_query_building() {
        let q = SearchQuery {
            q: "hello world".into(),
            filter_by: Some("kind:=1".into()),
            sort_by: Some("created_at:desc".into()),
            page: 2,
            per_page: 10,
        };

        let params = q.to_query_params();
        let get = |key: &str| -> Option<String> {
            params
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };

        assert_eq!(get("q").unwrap(), "hello world");
        assert_eq!(get("query_by").unwrap(), "content");
        assert_eq!(get("page").unwrap(), "2");
        assert_eq!(get("per_page").unwrap(), "10");
        assert_eq!(get("filter_by").unwrap(), "kind:=1");
        assert_eq!(get("sort_by").unwrap(), "created_at:desc");
    }

    #[test]
    fn test_search_query_no_optional_fields() {
        let q = SearchQuery {
            q: "*".into(),
            filter_by: None,
            sort_by: None,
            page: 1,
            per_page: 20,
        };

        let params = q.to_query_params();
        let has_key = |key: &str| params.iter().any(|(k, _)| k == key);

        assert!(has_key("q"));
        assert!(has_key("query_by"));
        assert!(has_key("page"));
        assert!(has_key("per_page"));
        assert!(!has_key("filter_by"));
        assert!(!has_key("sort_by"));
    }

    #[test]
    fn test_search_result_parsing() {
        let raw = json!({
            "found": 42,
            "page": 1,
            "hits": [
                {
                    "document": {
                        "id": "abc123",
                        "content": "hello buzz",
                        "kind": 1,
                        "pubkey": "deadbeef",
                        "channel_id": "chan-uuid",
                        "created_at": 1700000000i64,
                        "tags_flat": ["e:ref123"]
                    },
                    "text_match": 578730123i64
                },
                {
                    "document": {
                        "id": "def456",
                        "content": "another message",
                        "kind": 42,
                        "pubkey": "cafebabe",
                        "channel_id": null,
                        "created_at": 1700000100i64,
                        "tags_flat": []
                    },
                    "text_match": null
                }
            ]
        });

        let ts_resp: TypesenseSearchResponse = serde_json::from_value(raw).expect("should parse");
        let result = parse_response(ts_resp).expect("should succeed");

        assert_eq!(result.found, 42);
        assert_eq!(result.page, 1);
        assert_eq!(result.hits.len(), 2);

        let h0 = &result.hits[0];
        assert_eq!(h0.event_id, "abc123");
        assert_eq!(h0.content, "hello buzz");
        assert_eq!(h0.kind, 1);
        assert_eq!(h0.pubkey, "deadbeef");
        assert_eq!(h0.channel_id.as_deref(), Some("chan-uuid"));
        assert_eq!(h0.created_at, 1700000000);
        assert!(h0.score > 0.0);

        let h1 = &result.hits[1];
        assert_eq!(h1.event_id, "def456");
        assert_eq!(h1.kind, 42);
        assert!(h1.channel_id.is_none());
        assert_eq!(h1.score, 0.0); // null text_match → 0
    }

    #[test]
    fn test_search_result_empty() {
        let raw = json!({
            "found": 0,
            "page": 1,
            "hits": []
        });

        let ts_resp: TypesenseSearchResponse = serde_json::from_value(raw).expect("should parse");
        let result = parse_response(ts_resp).expect("should succeed");

        assert_eq!(result.found, 0);
        assert!(result.hits.is_empty());
    }
}
