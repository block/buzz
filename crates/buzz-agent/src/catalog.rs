//! Databricks model catalog discovery.
//!
//! Exposes [`discover_databricks_models`] — an async helper that lists
//! available models for the `databricks` and `databricks_v2` providers
//! without triggering a browser OAuth flow. Auth is acquired in-process via
//! [`build_token_source`](crate::llm::build_token_source):
//!
//! - Static bearer (`DATABRICKS_TOKEN`): returned immediately.
//! - PKCE cache hit: returned from disk without a network round-trip.
//! - PKCE cache empty / no token: returns `Err(AgentError::LlmAuth)`.
//!
//! This helper never opens a browser. Callers choose whether to reject, degrade,
//! or start a separate interactive authentication flow.

use std::sync::Arc;

use reqwest::Client;

use crate::{
    auth::TokenSource,
    config::{Config, Provider},
    llm::build_token_source,
    types::AgentError,
};

/// A discovered model entry: `id` is the picker value, `name` is the display
/// label (same as `id` for Databricks — the API has no separate display name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
}

/// Known Databricks AI Gateway v2 models — used only when an authenticated
/// `api/ai-gateway/v2/endpoints` call succeeds with an empty list.
/// Mirrors goose's `DATABRICKS_V2_KNOWN_MODELS`.
pub const DATABRICKS_V2_KNOWN_MODELS: &[&str] =
    &["databricks-gpt-5-5", "databricks-claude-opus-4-7"];

const AUTHENTICATED_EMPTY_CATALOG_SUFFIX: &str = " (default catalog)";

fn authenticated_empty_v2_catalog() -> Vec<ModelEntry> {
    DATABRICKS_V2_KNOWN_MODELS
        .iter()
        .map(|id| ModelEntry {
            id: id.to_string(),
            name: format!("{id}{AUTHENTICATED_EMPTY_CATALOG_SUFFIX}"),
        })
        .collect()
}

/// Heuristic: `true` when a v2 AI Gateway endpoint name looks like it serves
/// chat/completions traffic.
///
/// The v1 `serving-endpoints` payload carries `task`, so [`parse_v1_endpoints`]
/// can filter on it directly. The v2 `ai-gateway/v2/endpoints` payload carries
/// no task or readiness field at all, so the only signal available here is the
/// endpoint name. Embedding endpoints are the one family that reliably cannot
/// serve a chat request — they reject it with
/// `API type 'mlflow/v1/chat/completions' is not supported by '<name>'` — so
/// they are dropped rather than offered as selectable models.
///
/// Deliberately narrow: image-capable endpoints (e.g.
/// `databricks-gemini-3-pro-image`) do answer chat requests, so they stay. Any
/// name this heuristic does not recognise is kept — preferring to include over
/// silently dropping, matching [`parse_v1_endpoints`].
pub(crate) fn is_chat_capable_endpoint(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("embedding") {
        return false;
    }
    // Segment match so `bge`/`gte` cannot fire on a substring of a longer word.
    !lower
        .split('-')
        .any(|segment| matches!(segment, "bge" | "gte"))
}

/// Discover available models for a Databricks provider.
///
/// Returns a non-empty `Vec<ModelEntry>` on success. Returns
/// `Err(AgentError::LlmAuth)` when no token is available (no static token,
/// no PKCE cache). The helper itself never starts interactive authentication.
///
/// # Panics
/// Never panics.
pub async fn discover_databricks_models(cfg: &Config) -> Result<Vec<ModelEntry>, AgentError> {
    discover_databricks_models_with_token_source(cfg, build_token_source(cfg)?).await
}

async fn discover_databricks_models_with_token_source(
    cfg: &Config,
    token_source: Arc<dyn TokenSource>,
) -> Result<Vec<ModelEntry>, AgentError> {
    let mut bearer = token_source.bearer_no_browser().await?;
    let http = Client::new();
    let host = cfg.base_url.trim_end_matches('/');
    let mut refreshed = false;

    loop {
        let result = match cfg.provider {
            Provider::Databricks => fetch_v1_models(&http, host, &bearer).await,
            Provider::DatabricksV2 => fetch_v2_models(&http, host, &bearer).await,
            _ => {
                return Err(AgentError::InvalidParams(
                    "discover_databricks_models called for non-Databricks provider".into(),
                ));
            }
        };

        match result {
            Err(AgentError::LlmAuth(_)) if !refreshed => {
                refreshed = true;
                let fresh = token_source.refresh_now(&bearer).await?;
                if fresh == bearer {
                    return Err(AgentError::LlmAuth(
                        "Databricks rejected the configured credential".into(),
                    ));
                }
                bearer = fresh;
            }
            result => return result,
        }
    }
}

// ---------------------------------------------------------------------------
// v1 — api/2.0/serving-endpoints
// ---------------------------------------------------------------------------

async fn fetch_v1_models(
    http: &Client,
    host: &str,
    bearer: &str,
) -> Result<Vec<ModelEntry>, AgentError> {
    let url = format!("{host}/api/2.0/serving-endpoints");
    let response = http
        .get(&url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| AgentError::Llm(format!("Databricks model discovery request failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        if status.as_u16() == 401 {
            return Err(AgentError::LlmAuth(format!(
                "Databricks model discovery HTTP {status}"
            )));
        }
        return Err(AgentError::Llm(format!(
            "Databricks model discovery HTTP {status}: {body}"
        )));
    }

    let json: serde_json::Value = response.json().await.map_err(|e| {
        AgentError::Llm(format!(
            "Databricks model discovery response parse failed: {e}"
        ))
    })?;

    parse_v1_endpoints(&json)
}

/// Parse a `GET api/2.0/serving-endpoints` response.
///
/// Filters to endpoints that are READY and serve an LLM chat/completions task.
/// When `state.ready` or `task` is absent the endpoint is included — prefer
/// including over silently dropping, per spec.
pub(crate) fn parse_v1_endpoints(json: &serde_json::Value) -> Result<Vec<ModelEntry>, AgentError> {
    let endpoints = json
        .get("endpoints")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            AgentError::Llm(
                "Databricks model discovery: unexpected response (missing 'endpoints' array)"
                    .into(),
            )
        })?;

    let models = endpoints
        .iter()
        .filter_map(|endpoint| {
            let name = endpoint.get("name")?.as_str()?.to_string();

            // Require READY state when present; include when absent.
            let state_ready = endpoint
                .get("state")
                .and_then(|s| s.get("ready"))
                .and_then(|r| r.as_str())
                .map(|r| r == "READY")
                .unwrap_or(true);
            if !state_ready {
                return None;
            }

            // Require LLM chat or completions task when present.
            let task_ok = endpoint
                .get("task")
                .and_then(|t| t.as_str())
                .map(|t| t == "llm/v1/chat" || t == "llm/v1/completions")
                .unwrap_or(true);
            if !task_ok {
                return None;
            }

            Some(ModelEntry {
                id: name.clone(),
                name,
            })
        })
        .collect();

    Ok(models)
}

// ---------------------------------------------------------------------------
// v2 — api/ai-gateway/v2/endpoints (paginated)
// ---------------------------------------------------------------------------

/// Percent-encode a string for use as a URL query parameter value.
/// Only encodes characters that are not unreserved (RFC 3986).
fn percent_encode(s: &str) -> String {
    s.bytes()
        .flat_map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![b as char]
            }
            _ => format!("%{b:02X}").chars().collect(),
        })
        .collect()
}

async fn fetch_v2_models(
    http: &Client,
    host: &str,
    bearer: &str,
) -> Result<Vec<ModelEntry>, AgentError> {
    let mut all_endpoints: Vec<V2Endpoint> = Vec::new();
    let mut page_token: Option<String> = None;
    let base_url = format!("{host}/api/ai-gateway/v2/endpoints");

    // Cap at 20 pages (2 000 endpoints) to bound execution time.
    for _ in 0..20 {
        // Build URL with query params manually — avoids requiring the `query`
        // reqwest feature in buzz-agent's Cargo.toml.
        let url = match &page_token {
            Some(tok) => format!(
                "{base_url}?page_size=100&page_token={}",
                percent_encode(tok)
            ),
            None => format!("{base_url}?page_size=100"),
        };
        let response = http
            .get(&url)
            .bearer_auth(bearer)
            .send()
            .await
            .map_err(|e| {
                AgentError::Llm(format!("Databricks v2 model discovery request failed: {e}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            if status.as_u16() == 401 {
                return Err(AgentError::LlmAuth(format!(
                    "Databricks v2 model discovery HTTP {status}"
                )));
            }
            return Err(AgentError::Llm(format!(
                "Databricks v2 model discovery HTTP {status}: {body}"
            )));
        }

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AgentError::Llm(format!(
                "Databricks v2 model discovery response parse failed: {e}"
            ))
        })?;

        let (page_endpoints, next) = parse_v2_endpoints_page(&json)?;
        all_endpoints.extend(page_endpoints);

        match next {
            Some(tok) if Some(&tok) != page_token.as_ref() => page_token = Some(tok),
            _ => break,
        }
    }

    // Fall back to known-model list if the API returned nothing.
    if all_endpoints.is_empty() {
        return Ok(authenticated_empty_v2_catalog());
    }

    sort_v2_endpoints_newest_first(&mut all_endpoints);

    Ok(all_endpoints
        .into_iter()
        .map(|endpoint| endpoint.entry)
        .collect())
}

/// A v2 gateway endpoint plus the key discovery orders the catalog by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct V2Endpoint {
    pub(crate) entry: ModelEntry,
    /// `created_timestamp` as epoch milliseconds. `None` when the field is
    /// absent or unparseable — those sort last rather than jumping the queue.
    pub(crate) created_ms: Option<i64>,
}

/// Read `created_timestamp` from one endpoint object.
///
/// The gateway sends epoch milliseconds as a JSON *string*
/// (`"created_timestamp": "1699610000000"`); accept a bare number too, so a
/// wire-shape change doesn't silently drop every endpoint to the bottom.
fn endpoint_created_ms(endpoint: &serde_json::Value) -> Option<i64> {
    let value = endpoint.get("created_timestamp")?;
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

/// Order the catalog newest-first, breaking ties by name.
///
/// The gateway returns endpoints in two phases — Databricks-managed first, then
/// workspace-created — each alphabetical by name, which buries a brand-new
/// frontier model deep in the list. Newest-first puts the models people are
/// reaching for at the top of the picker.
///
/// Endpoints with no usable timestamp sort last, and the name tiebreak keeps the
/// result stable: several managed endpoints share one placeholder timestamp, so
/// without it their relative order would be arbitrary.
pub(crate) fn sort_v2_endpoints_newest_first(endpoints: &mut [V2Endpoint]) {
    endpoints.sort_by(|a, b| {
        // `None` < `Some(_)`, so reversing puts timestamped endpoints first.
        b.created_ms
            .cmp(&a.created_ms)
            .then_with(|| a.entry.name.cmp(&b.entry.name))
    });
}

/// Parse one page of a `GET api/ai-gateway/v2/endpoints` response.
///
/// Returns `(endpoints, next_page_token)`. An empty or absent `next_page_token`
/// signals the last page. Endpoints that cannot serve chat traffic are dropped
/// (see [`is_chat_capable_endpoint`]) so the model picker only offers models the
/// agent can actually run. Page order is preserved here; the caller sorts once
/// every page is in (see [`sort_v2_endpoints_newest_first`]).
pub(crate) fn parse_v2_endpoints_page(
    json: &serde_json::Value,
) -> Result<(Vec<V2Endpoint>, Option<String>), AgentError> {
    let endpoints = json
        .get("endpoints")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            AgentError::Llm(
                "Databricks v2 model discovery: unexpected response (missing 'endpoints' array)"
                    .into(),
            )
        })?;

    let models = endpoints
        .iter()
        .filter_map(|endpoint| {
            let name = endpoint.get("name")?.as_str()?.to_string();
            if !is_chat_capable_endpoint(&name) {
                return None;
            }
            Some(V2Endpoint {
                entry: ModelEntry {
                    id: name.clone(),
                    name,
                },
                created_ms: endpoint_created_ms(endpoint),
            })
        })
        .collect();

    let next_page_token = json
        .get("next_page_token")
        .and_then(|v| v.as_str())
        .filter(|token| !token.is_empty())
        .map(str::to_string);

    Ok((models, next_page_token))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RefreshingTestTokenSource {
        refreshes: AtomicUsize,
    }

    #[async_trait]
    impl TokenSource for RefreshingTestTokenSource {
        async fn bearer(&self) -> Result<String, AgentError> {
            Ok("rejected".into())
        }

        async fn refresh_now(&self, rejected: &str) -> Result<String, AgentError> {
            assert_eq!(rejected, "rejected");
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            Ok("fresh".into())
        }
    }

    #[tokio::test]
    async fn discovery_refreshes_rejected_bearer_once_then_retries_successfully() {
        use axum::{
            extract::Query,
            http::{HeaderMap, StatusCode},
            routing::get,
            Json, Router,
        };
        use std::collections::HashMap;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_route = requests.clone();
        let app = Router::new().route(
            "/api/ai-gateway/v2/endpoints",
            get(
                move |headers: HeaderMap, Query(_query): Query<HashMap<String, String>>| {
                    let requests = requests_for_route.clone();
                    async move {
                        requests.fetch_add(1, Ordering::SeqCst);
                        match headers
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                        {
                            Some("Bearer fresh") => Ok(Json(serde_json::json!({
                                "endpoints": [{"name": "discovered-model"}],
                                "next_page_token": null,
                            }))),
                            _ => Err((StatusCode::UNAUTHORIZED, "rejected")),
                        }
                    }
                },
            ),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let source = Arc::new(RefreshingTestTokenSource {
            refreshes: AtomicUsize::new(0),
        });
        let cfg = Config::for_discovery(Provider::DatabricksV2, String::new(), host);
        let models = discover_databricks_models_with_token_source(&cfg, source.clone())
            .await
            .unwrap();

        assert_eq!(models[0].id, "discovered-model");
        assert_eq!(source.refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn v1_parse_filters_ready_chat_endpoints() {
        let json = serde_json::json!({
            "endpoints": [
                // included: READY + llm/v1/chat
                {"name": "my-llm", "state": {"ready": "READY"}, "task": "llm/v1/chat"},
                // included: READY + llm/v1/completions
                {"name": "my-completions", "state": {"ready": "READY"}, "task": "llm/v1/completions"},
                // excluded: NOT_READY
                {"name": "dead-endpoint", "state": {"ready": "NOT_READY"}, "task": "llm/v1/chat"},
                // excluded: wrong task
                {"name": "embedding-ep", "state": {"ready": "READY"}, "task": "llm/v1/embedding"},
                // included: no state field → include by default
                {"name": "no-state", "task": "llm/v1/chat"},
                // included: no task field → include by default
                {"name": "no-task", "state": {"ready": "READY"}},
            ]
        });

        let models = parse_v1_endpoints(&json).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["my-llm", "my-completions", "no-state", "no-task"]);
    }

    #[test]
    fn v1_parse_errors_on_missing_endpoints_array() {
        let json = serde_json::json!({"data": []});
        let err = parse_v1_endpoints(&json).unwrap_err();
        assert!(
            err.to_string().contains("missing 'endpoints' array"),
            "got: {err}"
        );
    }

    #[test]
    fn v1_parse_empty_endpoints_returns_empty_vec() {
        let json = serde_json::json!({"endpoints": []});
        let models = parse_v1_endpoints(&json).unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn v2_parse_extracts_names_and_page_token() {
        let json = serde_json::json!({
            "endpoints": [
                {"name": "databricks-claude-opus-4-7"},
                {"name": "databricks-gpt-5-5"},
                {"name": "custom-model"}
            ],
            "next_page_token": "tok123"
        });

        let (models, next) = parse_v2_endpoints_page(&json).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.entry.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "databricks-claude-opus-4-7",
                "databricks-gpt-5-5",
                "custom-model"
            ]
        );
        assert_eq!(next.as_deref(), Some("tok123"));
    }

    #[test]
    fn v2_parse_empty_token_signals_last_page() {
        let json = serde_json::json!({
            "endpoints": [{"name": "only-model"}],
            "next_page_token": ""
        });

        let (models, next) = parse_v2_endpoints_page(&json).unwrap();
        assert_eq!(models.len(), 1);
        assert!(
            next.is_none(),
            "empty token should be treated as no more pages"
        );
    }

    #[test]
    fn v2_parse_absent_token_signals_last_page() {
        let json = serde_json::json!({"endpoints": [{"name": "only-model"}]});
        let (_, next) = parse_v2_endpoints_page(&json).unwrap();
        assert!(next.is_none());
    }

    #[test]
    fn v2_parse_errors_on_missing_endpoints_array() {
        let json = serde_json::json!({"data": []});
        let err = parse_v2_endpoints_page(&json).unwrap_err();
        assert!(
            err.to_string().contains("missing 'endpoints' array"),
            "got: {err}"
        );
    }

    #[test]
    fn v2_parse_drops_embedding_endpoints() {
        // The v2 payload carries no `task`, so embedding endpoints are only
        // recognisable by name. They reject chat requests, so offering them in
        // the picker can only produce a 400 at send time.
        let json = serde_json::json!({
            "endpoints": [
                {"name": "databricks-bge-large-en"},
                {"name": "databricks-gte-large-en"},
                {"name": "databricks-qwen3-embedding-0-6b"},
                {"name": "databricks-claude-opus-5"},
                {"name": "databricks-gemini-3-pro-image"},
            ]
        });

        let (models, _) = parse_v2_endpoints_page(&json).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.entry.id.as_str()).collect();
        // Image endpoints DO answer chat requests, so they are retained.
        assert_eq!(
            ids,
            vec!["databricks-claude-opus-5", "databricks-gemini-3-pro-image"]
        );
    }

    #[test]
    fn v2_parse_reads_created_timestamp_in_either_wire_shape() {
        // The gateway sends epoch ms as a string; a bare number must work too.
        let json = serde_json::json!({
            "endpoints": [
                {"name": "string-ts", "created_timestamp": "1784932442251"},
                {"name": "number-ts", "created_timestamp": 1784932442251i64},
                {"name": "junk-ts", "created_timestamp": "not-a-number"},
                {"name": "no-ts"},
            ]
        });

        let (models, _) = parse_v2_endpoints_page(&json).unwrap();
        let stamps: Vec<Option<i64>> = models.iter().map(|m| m.created_ms).collect();
        assert_eq!(
            stamps,
            vec![Some(1784932442251), Some(1784932442251), None, None,]
        );
    }

    #[test]
    fn v2_endpoints_sort_newest_first_then_by_name() {
        // Mirrors the real catalog: the gateway pages Databricks-managed
        // endpoints first, then workspace-created ones, each alphabetical — so
        // the newest model is buried mid-list until this sort runs.
        let json = serde_json::json!({
            "endpoints": [
                {"name": "databricks-claude-opus-5", "created_timestamp": "1784851200000"},
                {"name": "databricks-gpt-5-6-sol", "created_timestamp": "1784073600000"},
                {"name": "databricks-gpt-5-6-luna", "created_timestamp": "1784073600000"},
                {"name": "databricks-llama-4-maverick", "created_timestamp": "1699610000000"},
                {"name": "goose-claude-opus-5", "created_timestamp": "1784932442251"},
                {"name": "endpoint-without-timestamp"},
            ]
        });

        let (mut models, _) = parse_v2_endpoints_page(&json).unwrap();
        sort_v2_endpoints_newest_first(&mut models);

        let ids: Vec<&str> = models.iter().map(|m| m.entry.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                // Newest first, across both pagination phases.
                "goose-claude-opus-5",
                "databricks-claude-opus-5",
                // Same timestamp — the name tiebreak keeps this deterministic.
                "databricks-gpt-5-6-luna",
                "databricks-gpt-5-6-sol",
                "databricks-llama-4-maverick",
                // No usable timestamp sorts last, never first.
                "endpoint-without-timestamp",
            ]
        );
    }

    #[test]
    fn authenticated_empty_v2_catalog_marks_fallback_provenance() {
        let models = authenticated_empty_v2_catalog();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(ids, DATABRICKS_V2_KNOWN_MODELS);
        assert!(models.iter().all(|model| {
            model.name == format!("{}{AUTHENTICATED_EMPTY_CATALOG_SUFFIX}", model.id)
        }));
    }

    #[test]
    fn is_chat_capable_endpoint_keeps_unrecognised_names() {
        // Prefer including over silently dropping — an unknown family is kept.
        assert!(is_chat_capable_endpoint("databricks-glm-5-2"));
        assert!(is_chat_capable_endpoint("some-teams-custom-endpoint"));
        // `bge`/`gte` match as whole segments only, never as substrings.
        assert!(is_chat_capable_endpoint("databricks-budget-gtex-model"));
        assert!(!is_chat_capable_endpoint("databricks-bge-large-en"));
        assert!(!is_chat_capable_endpoint("databricks-gte-large-en"));
        assert!(!is_chat_capable_endpoint("databricks-qwen3-embedding-0-6b"));
    }
}
