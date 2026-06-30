//! Databricks model catalog discovery.
//!
//! Exposes [`discover_databricks_models`] — an async helper that lists
//! available models for the `databricks` and `databricks_v2` providers
//! without triggering a browser OAuth flow. Auth is acquired in-process via
//! [`build_token_source`](crate::llm::build_token_source):
//!
//! - Static bearer (`DATABRICKS_TOKEN`): returned immediately.
//! - PKCE cache hit: returned from disk without a network round-trip.
//! - PKCE cache empty / no token: returns `Err(AgentError::LlmAuth)` — the
//!   caller degrades gracefully; no browser, no hang.

use reqwest::Client;

use crate::{
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

/// Known Databricks AI Gateway v2 models — used as a fallback when the
/// `api/ai-gateway/v2/endpoints` call returns an empty list.
/// Mirrors goose's `DATABRICKS_V2_KNOWN_MODELS`.
pub const DATABRICKS_V2_KNOWN_MODELS: &[&str] =
    &["databricks-gpt-5-5", "databricks-claude-opus-4-7"];

/// Discover available models for a Databricks provider.
///
/// Returns a non-empty `Vec<ModelEntry>` on success. Returns
/// `Err(AgentError::LlmAuth)` when no token is available (no static token,
/// no PKCE cache) — callers should degrade gracefully rather than hanging.
///
/// # Panics
/// Never panics.
pub async fn discover_databricks_models(cfg: &Config) -> Result<Vec<ModelEntry>, AgentError> {
    let token_source = build_token_source(cfg)?;
    let bearer = token_source.bearer_no_browser().await?;

    let http = Client::new();
    let host = cfg.base_url.trim_end_matches('/');

    match cfg.provider {
        Provider::Databricks => fetch_v1_models(&http, host, &bearer).await,
        Provider::DatabricksV2 => fetch_v2_models(&http, host, &bearer).await,
        _ => Err(AgentError::InvalidParams(
            "discover_databricks_models called for non-Databricks provider".into(),
        )),
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
    let mut all_models: Vec<ModelEntry> = Vec::new();
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
            return Err(AgentError::Llm(format!(
                "Databricks v2 model discovery HTTP {status}: {body}"
            )));
        }

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AgentError::Llm(format!(
                "Databricks v2 model discovery response parse failed: {e}"
            ))
        })?;

        let (page_models, next) = parse_v2_endpoints_page(&json)?;
        all_models.extend(page_models);

        match next {
            Some(tok) if Some(&tok) != page_token.as_ref() => page_token = Some(tok),
            _ => break,
        }
    }

    // Fall back to known-model list if the API returned nothing.
    if all_models.is_empty() {
        all_models = DATABRICKS_V2_KNOWN_MODELS
            .iter()
            .map(|id| ModelEntry {
                id: id.to_string(),
                name: id.to_string(),
            })
            .collect();
    }

    Ok(all_models)
}

/// Parse one page of a `GET api/ai-gateway/v2/endpoints` response.
///
/// Returns `(models, next_page_token)`. An empty or absent `next_page_token`
/// signals the last page.
pub(crate) fn parse_v2_endpoints_page(
    json: &serde_json::Value,
) -> Result<(Vec<ModelEntry>, Option<String>), AgentError> {
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
            Some(ModelEntry {
                id: name.clone(),
                name,
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
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
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
}
