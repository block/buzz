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

use std::{collections::HashSet, path::Path, sync::Arc, time::Duration};

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
const MAX_CATALOG_RESPONSE_BODY_BYTES: usize = 2 * 1024 * 1024;
const CATALOG_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CATALOG_MAX_RETRIES: usize = 3;
const CATALOG_RETRY_BACKOFF: Duration = Duration::from_millis(100);

#[derive(Clone, Copy)]
struct CatalogRequestPolicy {
    timeout: Duration,
    max_retries: usize,
    retry_backoff: Duration,
}

const DEFAULT_CATALOG_REQUEST_POLICY: CatalogRequestPolicy = CatalogRequestPolicy {
    timeout: CATALOG_REQUEST_TIMEOUT,
    max_retries: CATALOG_MAX_RETRIES,
    retry_backoff: CATALOG_RETRY_BACKOFF,
};
const WORKSPACE_CATALOG_QUERY: &str = "?page_size=100";
const UNITY_CATALOG_QUERY: &str = "?page_size=100&view=FULL";
type CatalogPage<T> = Result<(Vec<T>, Option<String>), AgentError>;

#[derive(Clone, Copy)]
struct CatalogDescriptor<T> {
    name: &'static str,
    path: &'static str,
    initial_query: &'static str,
    parse_page: fn(&Value) -> CatalogPage<T>,
}

const WORKSPACE_CATALOG_DESCRIPTOR: CatalogDescriptor<V2Endpoint> = CatalogDescriptor {
    name: "Databricks workspace endpoint catalog",
    path: "/api/ai-gateway/v2/endpoints",
    initial_query: WORKSPACE_CATALOG_QUERY,
    parse_page: parse_v2_endpoints_page,
};
const UNITY_CATALOG_DESCRIPTOR: CatalogDescriptor<ModelEntry> = CatalogDescriptor {
    name: "Databricks Unity Catalog model-service catalog",
    path: "/api/2.1/unity-catalog/model-services",
    initial_query: UNITY_CATALOG_QUERY,
    parse_page: parse_uc_model_services_page,
};

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

/// Heuristic chat-capability filter for v2 workspace endpoints.
///
/// The v2 catalog omits task metadata. Known embedding endpoint families cannot
/// answer chat-completions requests, so do not offer them as selectable models.
/// Unknown names remain visible; this filter is intentionally narrow.
pub(crate) fn is_chat_capable_endpoint(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("embedding") {
        return false;
    }
    !lower
        .split('-')
        .any(|segment| matches!(segment, "bge" | "gte"))
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
    discover_databricks_models_with_cache_dir(cfg, None).await
}

/// Discover Databricks models while storing PKCE credentials under an explicit
/// cache root. `None` preserves buzz-agent's production cache location.
pub async fn discover_databricks_models_with_cache_dir(
    cfg: &Config,
    cache_dir: Option<&Path>,
) -> Result<Vec<ModelEntry>, AgentError> {
    let token_source = if matches!(cfg.provider, Provider::Databricks | Provider::DatabricksV2)
        && cfg.api_key.is_empty()
    {
        crate::auth::PkceOAuthTokenSource::new(crate::llm::databricks_pkce_config(
            &cfg.base_url,
            cache_dir.map(Path::to_path_buf),
        ))?
    } else {
        build_token_source(cfg)?
    };
    discover_databricks_models_with_token_source(cfg, token_source).await
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
    let json = fetch_catalog_page(
        http,
        &url,
        "Databricks serving-endpoints catalog",
        bearer,
        DEFAULT_CATALOG_REQUEST_POLICY,
    )
    .await?;

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
    fetch_v2_models_with_policy(
        http,
        host,
        bearer,
        filter,
        allow_partial_auth_failure,
        DEFAULT_CATALOG_REQUEST_POLICY,
    )
    .await
}

async fn fetch_v2_models_with_policy(
    http: &Client,
    host: &str,
    bearer: &str,
    filter: Option<&DatabricksModelFilter>,
    allow_partial_auth_failure: bool,
    policy: CatalogRequestPolicy,
) -> Result<Vec<ModelEntry>, AgentError> {
    let workspace =
        fetch_catalog_pages_with_policy(http, host, bearer, WORKSPACE_CATALOG_DESCRIPTOR, policy);
    let unity_catalog =
        fetch_catalog_pages_with_policy(http, host, bearer, UNITY_CATALOG_DESCRIPTOR, policy);

    let (workspace, unity_catalog) = tokio::join!(workspace, unity_catalog);
    let (workspace, unity_catalog, both_succeeded) = match (workspace, unity_catalog) {
        (Ok(workspace), Ok(unity_catalog)) => (workspace, unity_catalog, true),
        (Ok(workspace), Err(error)) => {
            if matches!(&error, AgentError::LlmAuth(_)) && !allow_partial_auth_failure {
                return Err(error);
            }
            tracing::warn!(
                catalog = "unity-catalog model-services",
                error_kind = catalog_error_kind(&error),
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
                error_kind = catalog_error_kind(&error),
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

fn catalog_error_kind(error: &AgentError) -> &'static str {
    match error {
        AgentError::InvalidParams(_) => "invalid-params",
        AgentError::Llm(_) => "llm",
        AgentError::LlmAuth(_) => "auth",
        AgentError::LlmModelNotFound(_) => "model-not-found",
        AgentError::LlmContextExceeded(_) => "context-exceeded",
        AgentError::UnsupportedImageInput(_) => "unsupported-image",
        AgentError::Mcp(_) => "mcp",
        AgentError::Cancelled => "cancelled",
    }
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

async fn fetch_catalog_pages_with_policy<T>(
    http: &Client,
    host: &str,
    bearer: &str,
    descriptor: CatalogDescriptor<T>,
    policy: CatalogRequestPolicy,
) -> Result<Vec<T>, AgentError> {
    let CatalogDescriptor {
        name: catalog,
        path,
        initial_query,
        parse_page,
    } = descriptor;
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
        let json = fetch_catalog_page(http, &url, catalog, bearer, policy).await?;
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

struct ReadResponseBody {
    bytes: Vec<u8>,
    truncated: bool,
}

enum CatalogRequestError {
    Auth,
    Status {
        status: reqwest::StatusCode,
        body: String,
    },
    Transport(reqwest::Error),
    Body(reqwest::Error),
    InvalidJson(serde_json::Error),
    BodyTooLarge,
}

async fn fetch_catalog_page(
    http: &Client,
    url: &str,
    catalog: &str,
    bearer: &str,
    policy: CatalogRequestPolicy,
) -> Result<Value, AgentError> {
    let max_retries = policy.max_retries.max(1);
    let error_body_limit = if bearer.len() > MAX_CATALOG_ERROR_BODY_BYTES {
        0
    } else {
        MAX_CATALOG_ERROR_BODY_BYTES.saturating_add(bearer.len())
    };

    for attempt in 0..max_retries {
        let result = tokio::time::timeout(policy.timeout, async {
            let response = http
                .get(url)
                .bearer_auth(bearer)
                .send()
                .await
                .map_err(CatalogRequestError::Transport)?;
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                // Preserve the auth contract: do not consume an auth-failure
                // body because gateways may echo credential material. The
                // bounded attempt ends at headers for this intentionally
                // redacted branch; all other status/body paths below consume
                // their response body inside the same deadline.
                return Err(CatalogRequestError::Auth);
            }
            if !status.is_success() {
                let mut response = response;
                let body = read_catalog_error_body(&mut response, error_body_limit)
                    .await
                    .map_err(CatalogRequestError::Body)?;
                return Err(CatalogRequestError::Status { status, body });
            }

            let mut response = response;
            if response
                .content_length()
                .is_some_and(|length| length > MAX_CATALOG_RESPONSE_BODY_BYTES as u64)
            {
                return Err(CatalogRequestError::BodyTooLarge);
            }
            let body = read_response_body(&mut response, MAX_CATALOG_RESPONSE_BODY_BYTES)
                .await
                .map_err(CatalogRequestError::Body)?;
            if body.truncated {
                return Err(CatalogRequestError::BodyTooLarge);
            }
            serde_json::from_slice(&body.bytes).map_err(CatalogRequestError::InvalidJson)
        })
        .await;

        match result {
            Ok(Ok(json)) => return Ok(json),
            Ok(Err(CatalogRequestError::Auth)) => {
                return Err(AgentError::LlmAuth(format!("{catalog} HTTP 401")));
            }
            Ok(Err(CatalogRequestError::Status { status, body })) => {
                if (status.as_u16() == 499 || status.is_server_error())
                    && retry_catalog_attempt(
                        catalog,
                        attempt,
                        max_retries,
                        policy.retry_backoff,
                        Some(status.as_u16()),
                        "transient status",
                    )
                    .await
                {
                    continue;
                }
                return Err(catalog_http_error_body(catalog, status, &body, bearer));
            }
            Ok(Err(CatalogRequestError::Transport(error))) => {
                if (error.is_timeout() || error.is_connect() || error.is_request())
                    && retry_catalog_attempt(
                        catalog,
                        attempt,
                        max_retries,
                        policy.retry_backoff,
                        None,
                        "transport error",
                    )
                    .await
                {
                    continue;
                }
                return Err(AgentError::Llm(format!(
                    "{catalog} request failed: {error}"
                )));
            }
            Ok(Err(CatalogRequestError::Body(error))) => {
                if retry_catalog_attempt(
                    catalog,
                    attempt,
                    max_retries,
                    policy.retry_backoff,
                    None,
                    "response body error",
                )
                .await
                {
                    continue;
                }
                return Err(AgentError::Llm(format!(
                    "{catalog} response body read failed: {error}"
                )));
            }
            Ok(Err(CatalogRequestError::InvalidJson(error))) => {
                if retry_catalog_attempt(
                    catalog,
                    attempt,
                    max_retries,
                    policy.retry_backoff,
                    None,
                    "invalid JSON response",
                )
                .await
                {
                    continue;
                }
                return Err(AgentError::Llm(format!(
                    "{catalog} response parse failed: {error}"
                )));
            }
            Ok(Err(CatalogRequestError::BodyTooLarge)) => {
                return Err(AgentError::Llm(format!(
                    "{catalog} response exceeded {MAX_CATALOG_RESPONSE_BODY_BYTES} bytes"
                )));
            }
            Err(_) => {
                if retry_catalog_attempt(
                    catalog,
                    attempt,
                    max_retries,
                    policy.retry_backoff,
                    None,
                    "attempt timeout",
                )
                .await
                {
                    continue;
                }
                return Err(AgentError::Llm(format!(
                    "{catalog} request timed out after {:?}",
                    policy.timeout
                )));
            }
        }
    }

    Err(AgentError::Llm(format!(
        "{catalog} request failed after {max_retries} attempts"
    )))
}

async fn retry_catalog_attempt(
    catalog: &str,
    attempt: usize,
    max_attempts: usize,
    backoff: Duration,
    status: Option<u16>,
    reason: &'static str,
) -> bool {
    if attempt + 1 >= max_attempts {
        return false;
    }

    tracing::warn!(
        catalog,
        attempt = attempt + 1,
        max_attempts,
        status = ?status,
        reason,
        "Databricks model discovery catalog request retrying"
    );
    tokio::time::sleep(backoff).await;
    true
}

fn catalog_http_error_body(
    catalog: &str,
    status: reqwest::StatusCode,
    body: &str,
    bearer: &str,
) -> AgentError {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return AgentError::LlmAuth(format!("{catalog} HTTP {status}"));
    }

    let body = if bearer.len() > MAX_CATALOG_ERROR_BODY_BYTES {
        String::new()
    } else if bearer.is_empty() {
        body.to_string()
    } else {
        body.replace(bearer, "[redacted]")
    };
    let body = truncate_utf8_bytes(&body, MAX_CATALOG_ERROR_BODY_BYTES);
    let classification = if status.as_u16() == 499 || status.is_server_error() {
        "transient"
    } else {
        "failed"
    };
    AgentError::Llm(format!("{catalog} {classification} HTTP {status}: {body}"))
}

async fn read_response_body(
    response: &mut reqwest::Response,
    limit: usize,
) -> Result<ReadResponseBody, reqwest::Error> {
    let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
    if limit == 0 {
        return Ok(ReadResponseBody {
            bytes,
            truncated: true,
        });
    }

    loop {
        if bytes.len() == limit {
            // Probe one frame past the bound. Without this read, a chunked body
            // whose first chunk lands exactly on `limit` would be accepted
            // without noticing the next frame.
            let truncated = response.chunk().await?.is_some();
            return Ok(ReadResponseBody { bytes, truncated });
        }

        let Some(chunk) = response.chunk().await? else {
            return Ok(ReadResponseBody {
                bytes,
                truncated: false,
            });
        };
        let remaining = limit - bytes.len();
        if chunk.len() > remaining {
            bytes.extend_from_slice(&chunk[..remaining]);
            return Ok(ReadResponseBody {
                bytes,
                truncated: true,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
}

async fn read_catalog_error_body(
    response: &mut reqwest::Response,
    limit: usize,
) -> Result<String, reqwest::Error> {
    let body = read_response_body(response, limit).await?;
    Ok(String::from_utf8_lossy(&body.bytes).into_owned())
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
            if name.is_empty() || !is_chat_capable_endpoint(&name) {
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
/// Only the exact resource prefix, a structurally valid three-component FQN,
/// and chat-capable service metadata are selectable. Missing or empty capability
/// metadata is retained for compatibility with older Databricks workspaces; a
/// non-empty capability list must advertise the MLflow chat API used for model-
/// service inference. The positive visibility filter is applied later.
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
            if !crate::model_capabilities::is_databricks_model_service_fqn(fqn)
                || !uc_model_service_supports_chat(service)
            {
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

fn uc_model_service_supports_chat(service: &Value) -> bool {
    let Some(api_types) = service.get("supported_api_types").and_then(Value::as_array) else {
        return true;
    };

    api_types.is_empty()
        || api_types
            .iter()
            .any(|api_type| api_type.as_str() == Some("mlflow/v1/chat/completions"))
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

    const TEST_CATALOG_DESCRIPTOR: CatalogDescriptor<V2Endpoint> = CatalogDescriptor {
        name: "test catalog",
        path: "/catalog",
        initial_query: "?page_size=100",
        parse_page: parse_v2_endpoints_page,
    };

    fn test_policy(timeout: Duration, max_retries: usize) -> CatalogRequestPolicy {
        CatalogRequestPolicy {
            timeout,
            max_retries,
            retry_backoff: Duration::ZERO,
        }
    }

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

        let entries = fetch_catalog_pages_with_policy(
            &Client::new(),
            &host,
            "token",
            TEST_CATALOG_DESCRIPTOR,
            DEFAULT_CATALOG_REQUEST_POLICY,
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
        let error = fetch_catalog_pages_with_policy(
            &Client::new(),
            &host,
            "token",
            TEST_CATALOG_DESCRIPTOR,
            DEFAULT_CATALOG_REQUEST_POLICY,
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

        let error = fetch_catalog_pages_with_policy(
            &Client::new(),
            &host,
            "token",
            TEST_CATALOG_DESCRIPTOR,
            DEFAULT_CATALOG_REQUEST_POLICY,
        )
        .await
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("pagination exhausted after 20 pages"));
        assert_eq!(requests.load(Ordering::SeqCst), 20);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn v2_discovery_degrades_a_stalled_secondary_catalog() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/api/ai-gateway/v2/endpoints",
                get(|| async {
                    Json(serde_json::json!({
                        "endpoints": [{"name": "workspace-only"}],
                        "next_page_token": null,
                    }))
                }),
            )
            .route(
                "/api/2.1/unity-catalog/model-services",
                get(|| async {
                    // The handler never sends headers. The catalog attempt
                    // deadline must still let the workspace result win.
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Json(serde_json::json!({
                        "model_services": [],
                        "next_page_token": null,
                    }))
                }),
            );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let started = std::time::Instant::now();
        let models = fetch_v2_models_with_policy(
            &Client::new(),
            &host,
            "token",
            None,
            false,
            test_policy(Duration::from_millis(40), 1),
        )
        .await
        .unwrap();

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "stalled catalog exceeded its request deadline: {:?}",
            started.elapsed()
        );
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["workspace-only"]
        );
    }

    #[tokio::test]
    async fn catalog_retries_499_and_5xx_then_recovers() {
        for status in [
            StatusCode::from_u16(499).unwrap(),
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let host = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(AtomicUsize::new(0));
            let requests_for_route = requests.clone();
            let app = Router::new().route(
                "/catalog",
                get(move || {
                    let attempt = requests_for_route.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if attempt == 0 {
                            Err((status, "provider body secret-token"))
                        } else {
                            Ok(Json(serde_json::json!({
                                "endpoints": [{"name": "recovered"}],
                                "next_page_token": null,
                            })))
                        }
                    }
                }),
            );
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });

            let entries = fetch_catalog_pages_with_policy(
                &Client::new(),
                &host,
                "secret-token",
                TEST_CATALOG_DESCRIPTOR,
                test_policy(Duration::from_secs(1), 3),
            )
            .await
            .unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].entry.id, "recovered");
            assert_eq!(requests.load(Ordering::SeqCst), 2);
        }
    }

    #[tokio::test]
    async fn catalog_retries_malformed_json_then_recovers() {
        use axum::response::IntoResponse;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_route = requests.clone();
        let app = Router::new().route(
            "/catalog",
            get(move || {
                let attempt = requests_for_route.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt == 0 {
                        (StatusCode::OK, "not-json").into_response()
                    } else {
                        Json(serde_json::json!({
                            "endpoints": [{"name": "json-recovered"}],
                            "next_page_token": null,
                        }))
                        .into_response()
                    }
                }
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let entries = fetch_catalog_pages_with_policy(
            &Client::new(),
            &host,
            "token",
            TEST_CATALOG_DESCRIPTOR,
            test_policy(Duration::from_secs(1), 3),
        )
        .await
        .unwrap();

        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(entries[0].entry.id, "json-recovered");
    }

    #[tokio::test]
    async fn catalog_transient_failure_exhausts_exactly_three_attempts_without_bearer_leak() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let requests_for_route = requests.clone();
        let app = Router::new().route(
            "/catalog",
            get(move || {
                requests_for_route.fetch_add(1, Ordering::SeqCst);
                async {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        "provider body secret-token",
                    )
                }
            }),
        );
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let error = fetch_catalog_pages_with_policy(
            &Client::new(),
            &host,
            "secret-token",
            TEST_CATALOG_DESCRIPTOR,
            test_policy(Duration::from_secs(1), 3),
        )
        .await
        .unwrap_err();

        assert_eq!(requests.load(Ordering::SeqCst), 3);
        let message = error.to_string();
        assert!(
            message.contains("transient HTTP 503"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("provider body"),
            "body context was lost: {message}"
        );
        assert!(
            !message.contains("secret-token"),
            "bearer leaked through catalog error: {message}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn catalog_retries_when_headers_arrive_but_response_body_stalls() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let headers_sent = Arc::new(AtomicUsize::new(0));
        let requests_for_server = requests.clone();
        let headers_for_server = headers_sent.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let attempt = requests_for_server.fetch_add(1, Ordering::SeqCst);
                let headers_sent = headers_for_server.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        match socket.read(&mut chunk).await {
                            Ok(0) | Err(_) => return,
                            Ok(read) => request.extend_from_slice(&chunk[..read]),
                        }
                    }

                    if attempt == 0 {
                        socket
                            .write_all(
                                b"HTTP/1.1 200 OK\r\n\
                                  Content-Type: application/json\r\n\
                                  Content-Length: 64\r\n\
                                  Connection: close\r\n\r\n\
                                  {\"endpoints\": [",
                            )
                            .await
                            .ok();
                        headers_sent.store(1, Ordering::SeqCst);
                        // Keep the declared body incomplete. The outer attempt
                        // timeout, not reqwest::send(), must terminate this read.
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    } else {
                        let body =
                            r#"{"endpoints":[{"name":"body-recovered"}],"next_page_token":null}"#;
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        socket.write_all(response.as_bytes()).await.ok();
                    }
                });
            }
        });

        let entries = fetch_catalog_pages_with_policy(
            &Client::new(),
            &host,
            "token",
            TEST_CATALOG_DESCRIPTOR,
            test_policy(Duration::from_millis(40), 2),
        )
        .await
        .unwrap();

        assert_eq!(headers_sent.load(Ordering::SeqCst), 1);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(entries[0].entry.id, "body-recovered");
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
        let bearer = "secret-token";
        let provider_body = format!("prefix {bearer} {}", "x".repeat(8_192));
        let status = reqwest::StatusCode::SERVICE_UNAVAILABLE;
        let error = catalog_http_error_body("test catalog", status, &provider_body, bearer);
        let message = error.to_string();
        assert!(
            message.contains("transient HTTP 503"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("[redacted]"),
            "bearer was not redacted: {message}"
        );
        assert!(!message.contains(bearer), "bearer leaked: {message}");
        let prefix = format!("llm: test catalog transient HTTP {status}: ");
        assert!(
            message.starts_with(&prefix),
            "unexpected catalog error prefix: message={message:?}, prefix={prefix:?}"
        );
        let diagnostic = &message[prefix.len()..];
        assert!(
            diagnostic.len() <= MAX_CATALOG_ERROR_BODY_BYTES,
            "error body exceeded diagnostic bound: {}",
            diagnostic.len()
        );

        // Keep the UTF-8 boundary behavior explicit as well.
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
    fn v2_parse_drops_embedding_endpoints() {
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
            vec!["databricks-claude-opus-5", "databricks-gemini-3-pro-image",]
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
    fn uc_parse_filters_known_non_chat_services_and_preserves_unknown_capabilities() {
        let json = serde_json::json!({
            "model_services": [
                {
                    "name": "model-services/system.ai.chat-model",
                    "supported_api_types": [
                        "mlflow/v1/chat/completions",
                        "mlflow/v1/responses"
                    ]
                },
                {
                    "name": "model-services/system.ai.embedding-model",
                    "supported_api_types": ["mlflow/v1/embeddings"]
                },
                {
                    "name": "model-services/system.ai.responses-only-model",
                    "supported_api_types": ["mlflow/v1/responses"]
                },
                {
                    "name": "model-services/catalog.schema.empty-capabilities",
                    "supported_api_types": []
                },
                {"name": "model-services/catalog.schema.absent-capabilities"},
            ]
        });

        let (models, _) = parse_uc_model_services_page(&json).unwrap();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "system.ai.chat-model",
                "catalog.schema.empty-capabilities",
                "catalog.schema.absent-capabilities",
            ]
        );
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
