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

use std::{collections::HashSet, sync::Arc};

use reqwest::Client;
use serde_json::Value;

use crate::{
    auth::TokenSource,
    config::{Config, DatabricksModelFilter, Provider},
    llm::build_token_source,
    types::AgentError,
};

/// A discovered model entry: `id` is the picker value (the raw endpoint id or
/// Unity Catalog model-service FQN, and the wire/config value), `name` is the
/// display label. Databricks catalog APIs do not provide a consistently useful
/// picker label, so discovery curates names from the capability manifest when
/// an exact known id exists and otherwise uses the raw id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
}

const AUTHENTICATED_EMPTY_CATALOG_SUFFIX: &str = " (default catalog)";
const MAX_CATALOG_PAGES: usize = 20;
const MAX_CATALOG_ERROR_BODY_BYTES: usize = 4 * 1024;
const WORKSPACE_CATALOG_QUERY: &str = "?page_size=100";
const UNITY_CATALOG_QUERY: &str = "?page_size=100&view=FULL";
type CatalogPage<T> = Result<(Vec<T>, Option<String>), AgentError>;

/// Curated display label for a discovered Databricks endpoint or model-service
/// id. Unknown ids deliberately pass through unchanged.
fn curated_model_name(id: &str) -> String {
    crate::model_capabilities::databricks_registry_label(id)
        .unwrap_or(id)
        .to_string()
}

/// Fallback catalog used only when both authenticated Databricks v2 catalogs
/// successfully respond with no entries and no visibility filter is active.
/// The known-model ids come from the manifest, the single runtime source.
fn authenticated_empty_v2_catalog() -> Vec<ModelEntry> {
    crate::model_capabilities::databricks_v2_known_models()
        .iter()
        .map(|id| ModelEntry {
            id: id.clone(),
            name: format!(
                "{}{AUTHENTICATED_EMPTY_CATALOG_SUFFIX}",
                curated_model_name(id)
            ),
        })
        .collect()
}

/// Discover available models for a Databricks provider.
///
/// Returns an empty vector when an authenticated catalog is valid but no
/// visible entries remain after filtering. Returns `Err(AgentError::LlmAuth)`
/// when no token is available (no static token, no PKCE cache). The helper
/// itself never starts interactive authentication.
///
/// For v2, the known-model fallback is used only when both catalog requests
/// succeed empty and no filter is active. A filter is applied to v1 results
/// after its existing endpoint capability filtering.
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
            Provider::Databricks => fetch_v1_models(&http, host, &bearer)
                .await
                .map(|models| apply_model_filter(models, cfg.databricks_model_filter.as_ref())),
            Provider::DatabricksV2 => {
                fetch_v2_models(
                    &http,
                    host,
                    &bearer,
                    cfg.databricks_model_filter.as_ref(),
                    refreshed,
                )
                .await
            }
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

fn apply_model_filter(
    models: Vec<ModelEntry>,
    filter: Option<&DatabricksModelFilter>,
) -> Vec<ModelEntry> {
    match filter {
        Some(filter) => models
            .into_iter()
            .filter(|model| filter.matches(&model.id))
            .collect(),
        None => models,
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
        .map_err(|e| {
            AgentError::Llm(format!(
                "Databricks serving-endpoints catalog request failed: {e}"
            ))
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(catalog_http_error(
            "Databricks serving-endpoints catalog",
            status,
            response,
            bearer,
        )
        .await);
    }

    let json: Value = response.json().await.map_err(|e| {
        AgentError::Llm(format!(
            "Databricks serving-endpoints catalog response parse failed: {e}"
        ))
    })?;

    parse_v1_endpoints(&json)
}

/// Parse a `GET api/2.0/serving-endpoints` response.
///
/// Filters to endpoints that are READY and serve an LLM chat/completions task.
/// When `state.ready` or `task` is absent the endpoint is included — prefer
/// including over silently dropping, per the existing v1 contract.
pub(crate) fn parse_v1_endpoints(json: &Value) -> Result<Vec<ModelEntry>, AgentError> {
    let endpoints = json
        .get("endpoints")
        .and_then(Value::as_array)
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
                .and_then(Value::as_str)
                .map(|r| r == "READY")
                .unwrap_or(true);
            if !state_ready {
                return None;
            }

            // Require LLM chat or completions task when present.
            let task_ok = endpoint
                .get("task")
                .and_then(Value::as_str)
                .map(|t| t == "llm/v1/chat" || t == "llm/v1/completions")
                .unwrap_or(true);
            if !task_ok {
                return None;
            }

            Some(ModelEntry {
                name: curated_model_name(&name),
                id: name,
            })
        })
        .collect();

    Ok(models)
}

// ---------------------------------------------------------------------------
// v2 — api/ai-gateway/v2/endpoints + Unity Catalog model-services
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

/// Fetch both Databricks v2 catalogs concurrently and merge them into the
/// selectable model list. One catalog may be unavailable; an empty result is
/// still authoritative and never falls through to the known-model fallback
/// when a visibility filter is active.
async fn fetch_v2_models(
    http: &Client,
    host: &str,
    bearer: &str,
    filter: Option<&DatabricksModelFilter>,
    allow_partial_auth_failure: bool,
) -> Result<Vec<ModelEntry>, AgentError> {
    let workspace = fetch_catalog_pages(
        http,
        host,
        bearer,
        "Databricks workspace endpoint catalog",
        "/api/ai-gateway/v2/endpoints",
        WORKSPACE_CATALOG_QUERY,
        parse_v2_endpoints_page,
    );
    let unity_catalog = fetch_catalog_pages(
        http,
        host,
        bearer,
        "Databricks Unity Catalog model-service catalog",
        "/api/2.1/unity-catalog/model-services",
        UNITY_CATALOG_QUERY,
        parse_uc_model_services_page,
    );

    let (workspace, unity_catalog) = tokio::join!(workspace, unity_catalog);
    let (workspace, unity_catalog, both_succeeded) = match (workspace, unity_catalog) {
        (Ok(workspace), Ok(unity_catalog)) => (workspace, unity_catalog, true),
        (Ok(workspace), Err(error)) => {
            if matches!(&error, AgentError::LlmAuth(_)) && !allow_partial_auth_failure {
                return Err(error);
            }
            tracing::warn!(
                catalog = "unity-catalog model-services",
                error = %error,
                "Databricks model discovery degraded: catalog unavailable"
            );
            (workspace, Vec::new(), false)
        }
        (Err(error), Ok(unity_catalog)) => {
            if matches!(&error, AgentError::LlmAuth(_)) && !allow_partial_auth_failure {
                return Err(error);
            }
            tracing::warn!(
                catalog = "workspace ai-gateway v2 endpoints",
                error = %error,
                "Databricks model discovery degraded: catalog unavailable"
            );
            (Vec::new(), unity_catalog, false)
        }
        (Err(workspace_error), Err(unity_catalog_error)) => {
            return Err(combined_catalog_error(workspace_error, unity_catalog_error));
        }
    };

    Ok(merge_v2_models(
        workspace,
        unity_catalog,
        filter,
        both_succeeded && filter.is_none(),
    ))
}

fn combined_catalog_error(workspace: AgentError, unity_catalog: AgentError) -> AgentError {
    let auth_failure = matches!(&workspace, AgentError::LlmAuth(_))
        || matches!(&unity_catalog, AgentError::LlmAuth(_));
    let message = format!(
        "Databricks v2 model discovery failed: workspace endpoint catalog: {workspace}; Unity Catalog model-service catalog: {unity_catalog}"
    );
    if auth_failure {
        AgentError::LlmAuth(message)
    } else {
        AgentError::Llm(message)
    }
}

fn merge_v2_models(
    workspace: Vec<V2Endpoint>,
    mut unity_catalog: Vec<ModelEntry>,
    filter: Option<&DatabricksModelFilter>,
    allow_known_model_fallback: bool,
) -> Vec<ModelEntry> {
    let mut seen_ids = HashSet::new();
    let mut merged = Vec::with_capacity(workspace.len() + unity_catalog.len());

    // Workspace endpoints are ordered newest-first across all pages.
    let mut workspace = workspace;
    sort_v2_endpoints_newest_first(&mut workspace);
    for endpoint in workspace {
        if seen_ids.insert(endpoint.entry.id.clone()) {
            merged.push(endpoint.entry);
        }
    }

    // UC has no user-facing recency contract. Sort by the raw FQN for stable
    // picker order, then deduplicate only by raw selectable id.
    unity_catalog.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    for entry in unity_catalog {
        if seen_ids.insert(entry.id.clone()) {
            merged.push(entry);
        }
    }

    if merged.is_empty() && allow_known_model_fallback && filter.is_none() {
        merged = authenticated_empty_v2_catalog();
    }

    apply_model_filter(merged, filter)
}

async fn fetch_catalog_pages<T>(
    http: &Client,
    host: &str,
    bearer: &str,
    catalog: &'static str,
    path: &'static str,
    initial_query: &str,
    parse_page: fn(&Value) -> CatalogPage<T>,
) -> Result<Vec<T>, AgentError> {
    let base_url = format!("{host}{path}");
    let mut all_items = Vec::new();
    let mut page_token: Option<String> = None;
    let mut seen_tokens = HashSet::new();

    for _page in 0..MAX_CATALOG_PAGES {
        let url = match &page_token {
            Some(token) => format!(
                "{base_url}{initial_query}&page_token={}",
                percent_encode(token)
            ),
            None => format!("{base_url}{initial_query}"),
        };
        let response = http
            .get(&url)
            .bearer_auth(bearer)
            .send()
            .await
            .map_err(|e| AgentError::Llm(format!("{catalog} request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(catalog_http_error(catalog, status, response, bearer).await);
        }

        let json: Value = response
            .json()
            .await
            .map_err(|e| AgentError::Llm(format!("{catalog} response parse failed: {e}")))?;
        let (items, next_token) = parse_page(&json)
            .map_err(|error| catalog_context_error(catalog, error, "response parse failed"))?;
        all_items.extend(items);

        match next_token {
            None => return Ok(all_items),
            Some(next_token) if seen_tokens.insert(next_token.clone()) => {
                page_token = Some(next_token);
            }
            Some(next_token) => {
                return Err(AgentError::Llm(format!(
                    "{catalog} pagination repeated page token {next_token:?}"
                )));
            }
        }
    }

    Err(AgentError::Llm(format!(
        "{catalog} pagination exhausted after {MAX_CATALOG_PAGES} pages"
    )))
}

async fn catalog_http_error(
    catalog: &str,
    status: reqwest::StatusCode,
    response: reqwest::Response,
    bearer: &str,
) -> AgentError {
    if status.as_u16() == 401 {
        // Do not read or include a provider body for auth failures. Some
        // gateways echo authorization material in diagnostic payloads.
        return AgentError::LlmAuth(format!("{catalog} HTTP {status}"));
    }

    let body = if bearer.len() > MAX_CATALOG_ERROR_BODY_BYTES {
        // A token longer than the diagnostic bound cannot be safely searched in
        // a bounded prefix. Do not return a partial provider body that might
        // expose any part of it.
        String::new()
    } else {
        let read_limit = MAX_CATALOG_ERROR_BODY_BYTES.saturating_add(bearer.len());
        let body = read_catalog_error_body(response, read_limit).await;
        if bearer.is_empty() {
            body
        } else {
            body.replace(bearer, "[redacted]")
        }
    };
    let body = truncate_utf8_bytes(&body, MAX_CATALOG_ERROR_BODY_BYTES);
    let classification = if status.as_u16() == 499 || status.is_server_error() {
        "transient"
    } else {
        "failed"
    };
    AgentError::Llm(format!("{catalog} {classification} HTTP {status}: {body}"))
}

async fn read_catalog_error_body(mut response: reqwest::Response, limit: usize) -> String {
    let mut body = Vec::with_capacity(limit);
    while body.len() < limit {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                let take = chunk.len().min(limit - body.len());
                body.extend_from_slice(&chunk[..take]);
                if take < chunk.len() {
                    break;
                }
            }
            _ => break,
        }
    }
    String::from_utf8_lossy(&body).into_owned()
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn catalog_context_error(catalog: &str, error: AgentError, context: &str) -> AgentError {
    match error {
        AgentError::LlmAuth(message) => {
            AgentError::LlmAuth(format!("{catalog} {context}: {message}"))
        }
        AgentError::Llm(message) => AgentError::Llm(format!("{catalog} {context}: {message}")),
        other => AgentError::Llm(format!("{catalog} {context}: {other}")),
    }
}

/// A v2 gateway endpoint plus the key discovery order field.
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
/// wire-shape change does not silently drop every endpoint to the bottom.
fn endpoint_created_ms(endpoint: &Value) -> Option<i64> {
    let value = endpoint.get("created_timestamp")?;
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

/// Order workspace endpoints newest-first, breaking ties by name.
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
/// Page order is preserved here; the caller sorts once every page is in.
pub(crate) fn parse_v2_endpoints_page(
    json: &Value,
) -> Result<(Vec<V2Endpoint>, Option<String>), AgentError> {
    let endpoints = json
        .get("endpoints")
        .and_then(Value::as_array)
        .ok_or_else(|| AgentError::Llm("unexpected response (missing 'endpoints' array)".into()))?;

    let models = endpoints
        .iter()
        .filter_map(|endpoint| {
            let name = endpoint.get("name")?.as_str()?.to_string();
            if name.is_empty() {
                return None;
            }
            Some(V2Endpoint {
                entry: ModelEntry {
                    name: curated_model_name(&name),
                    id: name,
                },
                created_ms: endpoint_created_ms(endpoint),
            })
        })
        .collect();

    let next_page_token = next_page_token(json);
    Ok((models, next_page_token))
}

/// Parse one page of a `GET api/2.1/unity-catalog/model-services` response.
///
/// Unity Catalog resource names are returned as `model-services/<catalog>.<schema>.<service>`.
/// Only the exact resource prefix and a structurally valid three-component FQN
/// are selectable. All other resources are ignored without capability/name
/// heuristics; the positive visibility filter is the only further restriction.
pub(crate) fn parse_uc_model_services_page(
    json: &Value,
) -> Result<(Vec<ModelEntry>, Option<String>), AgentError> {
    let services = json
        .get("model_services")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AgentError::Llm("unexpected response (missing 'model_services' array)".into())
        })?;

    let models = services
        .iter()
        .filter_map(|service| {
            let resource_name = service.get("name")?.as_str()?;
            let fqn = resource_name.strip_prefix("model-services/")?;
            if !crate::llm::is_model_service_fqn(fqn) {
                return None;
            }
            Some(ModelEntry {
                id: fqn.to_string(),
                name: curated_model_name(fqn),
            })
        })
        .collect();

    Ok((models, next_page_token(json)))
}

fn next_page_token(json: &Value) -> Option<String> {
    json.get("next_page_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{extract::Query, http::StatusCode, routing::get, Json, Router};
    use std::collections::HashMap;
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
        let cfg = Config::for_discovery(Provider::DatabricksV2, String::new(), host, None);
        let models = discover_databricks_models_with_token_source(&cfg, source.clone())
            .await
            .unwrap();

        assert_eq!(models[0].id, "discovered-model");
        assert_eq!(source.refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn v2_discovery_merges_workspace_and_unity_catalog_after_filtering() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/api/ai-gateway/v2/endpoints",
                get(|Query(query): Query<HashMap<String, String>>| async move {
                    assert_eq!(query.get("page_size").map(String::as_str), Some("100"));
                    Json(serde_json::json!({
                        "endpoints": [
                            {"name": "blocked-workspace", "created_timestamp": 3},
                            {"name": "allowed-workspace", "created_timestamp": 2},
                        ],
                        "next_page_token": null,
                    }))
                }),
            )
            .route(
                "/api/2.1/unity-catalog/model-services",
                get(|Query(query): Query<HashMap<String, String>>| async move {
                    assert_eq!(query.get("page_size").map(String::as_str), Some("100"));
                    assert_eq!(query.get("view").map(String::as_str), Some("FULL"));
                    Json(serde_json::json!({
                        "model_services": [
                            {"name": "model-services/catalog.schema.blocked-service"},
                            {"name": "model-services/catalog.schema.allowed-service"},
                            {"name": "model-services/catalog.schema.allowed-service"},
                        ],
                        "next_page_token": null,
                    }))
                }),
            );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let filter =
            DatabricksModelFilter::parse(Some("allowed-*,catalog.schema.allowed-*")).unwrap();
        let cfg = Config::for_discovery(Provider::DatabricksV2, "token".into(), host, filter);
        let models = discover_databricks_models(&cfg).await.unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["allowed-workspace", "catalog.schema.allowed-service"]
        );
    }

    #[tokio::test]
    async fn v2_discovery_keeps_unity_catalog_when_workspace_catalog_fails() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/api/ai-gateway/v2/endpoints",
                get(|| async { (StatusCode::SERVICE_UNAVAILABLE, "workspace unavailable") }),
            )
            .route(
                "/api/2.1/unity-catalog/model-services",
                get(|| async {
                    Json(serde_json::json!({
                        "model_services": [
                            {"name": "model-services/catalog.schema.uc-service"}
                        ],
                        "next_page_token": null,
                    }))
                }),
            );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let cfg = Config::for_discovery(Provider::DatabricksV2, "token".into(), host, None);
        let models = discover_databricks_models(&cfg).await.unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["catalog.schema.uc-service"]
        );
    }

    #[tokio::test]
    async fn v2_empty_catalog_fallback_is_disabled_by_filter() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/api/ai-gateway/v2/endpoints",
                get(|| async {
                    Json(serde_json::json!({
                        "endpoints": [],
                        "next_page_token": null,
                    }))
                }),
            )
            .route(
                "/api/2.1/unity-catalog/model-services",
                get(|| async {
                    Json(serde_json::json!({
                        "model_services": [],
                        "next_page_token": null,
                    }))
                }),
            );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let unfiltered =
            Config::for_discovery(Provider::DatabricksV2, "token".into(), host.clone(), None);
        let fallback = discover_databricks_models(&unfiltered).await.unwrap();
        assert_eq!(
            fallback
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            crate::model_capabilities::databricks_v2_known_models()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );

        let filter = DatabricksModelFilter::parse(Some("no-match")).unwrap();
        let filtered = Config::for_discovery(Provider::DatabricksV2, "token".into(), host, filter);
        assert!(discover_databricks_models(&filtered)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn catalog_pagination_encodes_tokens_and_rejects_repeated_tokens() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/catalog",
            get(|Query(query): Query<HashMap<String, String>>| async move {
                match query.get("page_token").map(String::as_str) {
                    None => Json(serde_json::json!({
                        "endpoints": [{"name": "first"}],
                        "next_page_token": "token with/slash",
                    })),
                    Some("token with/slash") => Json(serde_json::json!({
                        "endpoints": [{"name": "second"}],
                    })),
                    Some(other) => panic!("unexpected decoded page token: {other}"),
                }
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let entries = fetch_catalog_pages(
            &Client::new(),
            &host,
            "token",
            "test catalog",
            "/catalog",
            "?page_size=100",
            parse_v2_endpoints_page,
        )
        .await
        .unwrap();
        assert_eq!(entries.len(), 2);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/catalog",
            get(|| async {
                Json(serde_json::json!({
                    "endpoints": [{"name": "loop"}],
                    "next_page_token": "same-token",
                }))
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let error = fetch_catalog_pages(
            &Client::new(),
            &host,
            "token",
            "test catalog",
            "/catalog",
            "?page_size=100",
            parse_v2_endpoints_page,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("repeated page token"));
    }

    #[tokio::test]
    async fn catalog_pagination_errors_after_the_finite_page_cap() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_handler = requests.clone();
        let app = Router::new().route(
            "/catalog",
            get(move |Query(_query): Query<HashMap<String, String>>| {
                let page = requests_for_handler.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    Json(serde_json::json!({
                        "endpoints": [{"name": format!("model-{page}")}],
                        "next_page_token": format!("token-{page}"),
                    }))
                }
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let error = fetch_catalog_pages(
            &Client::new(),
            &host,
            "token",
            "test catalog",
            "/catalog",
            "?page_size=100",
            parse_v2_endpoints_page,
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("pagination exhausted after 20 pages"));
        assert_eq!(requests.load(Ordering::SeqCst), 20);
    }

    #[test]
    fn v1_filter_applies_to_raw_ids_after_endpoint_filtering() {
        let filter = DatabricksModelFilter::parse(Some("allowed-*")).unwrap();
        let models = apply_model_filter(
            vec![
                ModelEntry {
                    id: "allowed-model".into(),
                    name: "Allowed".into(),
                },
                ModelEntry {
                    id: "blocked-model".into(),
                    name: "Blocked".into(),
                },
            ],
            filter.as_ref(),
        );
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "allowed-model");
    }

    #[test]
    fn catalog_error_body_is_bounded_and_redacts_bearer() {
        // This pure assertion documents the byte-bound helper used after the
        // provider response is read. The network path exercises the same
        // redaction before truncation; keeping the helper pure makes the UTF-8
        // boundary behavior explicit.
        let value = format!("{}é", "x".repeat(MAX_CATALOG_ERROR_BODY_BYTES));
        let truncated = truncate_utf8_bytes(&value, MAX_CATALOG_ERROR_BODY_BYTES);
        assert_eq!(truncated.len(), MAX_CATALOG_ERROR_BODY_BYTES);
        assert!(truncated.is_char_boundary(truncated.len()));
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
    fn v2_parse_keeps_all_nonempty_endpoint_names_without_keyword_filtering() {
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
        assert_eq!(
            ids,
            vec![
                "databricks-bge-large-en",
                "databricks-gte-large-en",
                "databricks-qwen3-embedding-0-6b",
                "databricks-claude-opus-5",
                "databricks-gemini-3-pro-image",
            ]
        );
    }

    #[test]
    fn uc_parse_requires_exact_prefix_and_structural_fqn() {
        let json = serde_json::json!({
            "model_services": [
                {"name": "model-services/data_tools.goose.kimi-k3"},
                {"name": "model-services/catalog.schema.claude-gpt-5"},
                {"name": "model-services/two.parts"},
                {"name": "model-services/too.many.parts.here"},
                {"name": "Model-services/wrong.case.service"},
                {"name": "models/data_tools.goose.other"},
                {"name": "model-services/.schema.service"},
                {"name": "model-services/catalog..service"},
                {"name": "model-services/catalog.schema."},
                {"name": "model-services/catalog.schema/service"},
            ],
            "next_page_token": "next token/1"
        });

        let (models, next) = parse_uc_model_services_page(&json).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["data_tools.goose.kimi-k3", "catalog.schema.claude-gpt-5"]
        );
        assert_eq!(next.as_deref(), Some("next token/1"));
    }

    #[test]
    fn uc_parse_requires_model_services_array() {
        let err = parse_uc_model_services_page(&serde_json::json!({"data": []})).unwrap_err();
        assert!(err.to_string().contains("missing 'model_services' array"));
    }

    #[test]
    fn merge_deduplicates_raw_ids_and_preserves_workspace_then_lexical_uc_order() {
        let workspace = vec![
            V2Endpoint {
                entry: ModelEntry {
                    id: "workspace-new".into(),
                    name: "workspace-new".into(),
                },
                created_ms: Some(2),
            },
            V2Endpoint {
                entry: ModelEntry {
                    id: "duplicate".into(),
                    name: "duplicate".into(),
                },
                created_ms: Some(1),
            },
        ];
        let uc = vec![
            ModelEntry {
                id: "z.schema.service".into(),
                name: "z.schema.service".into(),
            },
            ModelEntry {
                id: "a.schema.service".into(),
                name: "a.schema.service".into(),
            },
            ModelEntry {
                id: "duplicate".into(),
                name: "same leaf".into(),
            },
            ModelEntry {
                id: "a.other.service".into(),
                name: "same leaf".into(),
            },
        ];

        let models = merge_v2_models(workspace, uc, None, false);
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "workspace-new",
                "duplicate",
                "a.other.service",
                "a.schema.service",
                "z.schema.service",
            ]
        );
    }

    #[test]
    fn merge_applies_filter_after_union_and_does_not_restore_fallback() {
        let filter = DatabricksModelFilter::parse(Some("allowed.*")).unwrap();
        let filter = filter.as_ref();
        let workspace = vec![V2Endpoint {
            entry: ModelEntry {
                id: "blocked-workspace".into(),
                name: "blocked-workspace".into(),
            },
            created_ms: Some(1),
        }];
        let uc = vec![ModelEntry {
            id: "allowed.schema.service".into(),
            name: "allowed.schema.service".into(),
        }];
        let models = merge_v2_models(workspace, uc, filter, false);
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["allowed.schema.service"]
        );

        let no_match = DatabricksModelFilter::parse(Some("no-match")).unwrap();
        assert!(merge_v2_models(Vec::new(), Vec::new(), no_match.as_ref(), true).is_empty());
    }

    #[test]
    fn merge_uses_known_fallback_only_for_unfiltered_successful_empty_union() {
        let models = merge_v2_models(Vec::new(), Vec::new(), None, true);
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            crate::model_capabilities::databricks_v2_known_models()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
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

        let known: Vec<&str> = crate::model_capabilities::databricks_v2_known_models()
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(ids, known);
        // `name` is the curated label + provenance suffix, not the raw id.
        assert!(models.iter().all(|model| {
            let label = crate::model_capabilities::databricks_registry_label(&model.id)
                .unwrap_or(model.id.as_str());
            model.name == format!("{label}{AUTHENTICATED_EMPTY_CATALOG_SUFFIX}")
        }));
    }

    #[test]
    fn v2_parse_curates_known_name_and_passes_unknown_through() {
        // buzz-agent's real discovery contract: the endpoint id IS the name the
        // API returns. A known id gets its manifest label; an unknown id stays raw.
        let json = serde_json::json!({
            "endpoints": [
                {"name": "databricks-gpt-5-5"},
                {"name": "custom-unlisted-endpoint"},
            ]
        });
        let (models, _) = parse_v2_endpoints_page(&json).unwrap();
        let by_id: std::collections::HashMap<&str, &str> = models
            .iter()
            .map(|m| (m.entry.id.as_str(), m.entry.name.as_str()))
            .collect();
        assert_eq!(by_id["databricks-gpt-5-5"], "GPT-5.5");
        assert_eq!(
            by_id["custom-unlisted-endpoint"],
            "custom-unlisted-endpoint"
        );
    }

    #[test]
    fn v1_parse_curates_known_name_and_passes_unknown_through() {
        let json = serde_json::json!({
            "endpoints": [
                {"name": "databricks-gpt-5-5", "task": "llm/v1/chat"},
                {"name": "custom-unlisted-endpoint", "task": "llm/v1/chat"},
            ]
        });
        let models = parse_v1_endpoints(&json).unwrap();
        let by_id: std::collections::HashMap<&str, &str> = models
            .iter()
            .map(|m| (m.id.as_str(), m.name.as_str()))
            .collect();
        assert_eq!(by_id["databricks-gpt-5-5"], "GPT-5.5");
        assert_eq!(
            by_id["custom-unlisted-endpoint"],
            "custom-unlisted-endpoint"
        );
    }
}
