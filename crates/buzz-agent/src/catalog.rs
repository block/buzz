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

/// Returns the discovery-failure fallback catalog for a Databricks provider.
///
/// This is the list of models advertised by `session/new` when
/// `discover_databricks_models` returns an error (e.g., no token available).
///
/// - `DatabricksV2` falls back to the configured model plus
///   [`DATABRICKS_V2_KNOWN_MODELS`] so the model-picker is always populated for
///   AI Gateway v2 users. The configured model leads: without it a fallback
///   catalog can omit the very model the agent is running, leaving the picker
///   unable to represent the current selection.
/// - Legacy `Databricks` falls back to only the configured model — the
///   `DATABRICKS_V2_KNOWN_MODELS` IDs are AI Gateway v2 endpoints that the
///   `/serving-endpoints/{model}/invocations` API may not serve.
///
/// Extracting this as a pure function makes the split testable without
/// spawning an async runtime or making network calls.
pub fn discovery_failure_fallback(provider: Provider, configured_model: &str) -> Vec<ModelEntry> {
    let configured = ModelEntry {
        id: configured_model.to_string(),
        name: configured_model.to_string(),
    };
    match provider {
        Provider::DatabricksV2 => {
            let mut entries = Vec::with_capacity(DATABRICKS_V2_KNOWN_MODELS.len() + 1);
            if !configured_model.trim().is_empty() {
                entries.push(configured);
            }
            entries.extend(
                DATABRICKS_V2_KNOWN_MODELS
                    .iter()
                    .filter(|id| **id != configured_model)
                    .map(|id| ModelEntry {
                        id: id.to_string(),
                        name: id.to_string(),
                    }),
            );
            entries
        }
        Provider::Databricks => vec![configured],
        _ => vec![configured],
    }
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
/// signals the last page. Endpoints that cannot serve chat traffic are dropped
/// (see [`is_chat_capable_endpoint`]) so the model picker only offers models the
/// agent can actually run.
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
            if !is_chat_capable_endpoint(&name) {
                return None;
            }
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
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        // Image endpoints DO answer chat requests, so they are retained.
        assert_eq!(
            ids,
            vec!["databricks-claude-opus-5", "databricks-gemini-3-pro-image"]
        );
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

    #[test]
    fn v2_discovery_failure_fallback_leads_with_configured_model() {
        let result = discovery_failure_fallback(Provider::DatabricksV2, "databricks-claude-opus-5");
        let ids: Vec<&str> = result.iter().map(|m| m.id.as_str()).collect();

        // The running model must be representable in the picker even when
        // discovery failed, so it leads the fallback catalog.
        assert_eq!(ids.first(), Some(&"databricks-claude-opus-5"));
        for known in DATABRICKS_V2_KNOWN_MODELS {
            assert!(ids.contains(known), "fallback must retain '{known}'");
        }
    }

    #[test]
    fn v2_discovery_failure_fallback_does_not_duplicate_configured_model() {
        let configured = DATABRICKS_V2_KNOWN_MODELS[0];
        let result = discovery_failure_fallback(Provider::DatabricksV2, configured);
        let occurrences = result.iter().filter(|m| m.id == configured).count();
        assert_eq!(occurrences, 1, "got: {result:?}");
        assert_eq!(result.len(), DATABRICKS_V2_KNOWN_MODELS.len());
    }

    #[test]
    fn v2_discovery_failure_fallback_tolerates_blank_configured_model() {
        let result = discovery_failure_fallback(Provider::DatabricksV2, "");
        let ids: Vec<&str> = result.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, DATABRICKS_V2_KNOWN_MODELS.to_vec());
    }
}
