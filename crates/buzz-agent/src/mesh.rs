//! Relay-mesh `auto` policy: adaptive Mixture-of-Agents routing.
//!
//! # What this restores
//!
//! Buzz owns the meaning of relay-mesh's `auto` model. The desktop sets
//! `BUZZ_AGENT_PREFER_MESH_FOR_AUTO=1` on every relay-mesh agent
//! (`desktop/src-tauri/src/managed_agents/relay_mesh.rs:41-44`), and the
//! pre-goose agent loop honoured it per request (old `llm.rs:406-470`):
//!
//! * Poll the router's `/models` catalog, cached for 5s.
//! * If the virtual `mesh` model is advertised **and** ≥2 distinct physical
//!   models are present, send `mesh` (Mixture-of-Agents) instead of `auto`.
//! * If the mesh contracts mid-request, cool down for 30s and retry once with
//!   plain `auto`.
//!
//! The point is that a long-running agent adopts or leaves MoA as lab nodes
//! come and go, without a restart. Goose resolves a model once at session
//! start, so without this the agent would send `auto` forever and MoA would
//! never engage at all.
//!
//! # How
//!
//! [`MeshAutoProvider`] wraps goose's real provider and rewrites
//! `ModelConfig.model_name` per call. `Provider` only requires `get_name` and
//! `stream`, so wrapping is cheap — and this is the kind of thing that is only
//! possible because we drive goose as a library: an out-of-process ACP agent
//! has no seam to intercept model resolution.
//!
//! Hysteresis is deliberately identical to the old implementation: two
//! consecutive positive observations to enable, immediate disable on a negative
//! one, and an unknown (unreachable/malformed) catalog preserves the last
//! confirmed state rather than treating a failed probe as evidence the mesh
//! vanished.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use goose::providers::base::Provider;
use goose_provider_types::base::{MessageStream, ModelInfo};
use goose_provider_types::conversation::message::Message;
use goose_provider_types::conversation::token_usage::ProviderUsage;
use goose_provider_types::errors::ProviderError;
use goose_provider_types::model::ModelConfig;
use goose_provider_types::retry::RetryConfig;
use rmcp::model::Tool;
use serde_json::Value;
use tokio::sync::Mutex;

/// mesh-llm's virtual Mixture-of-Agents model.
const VIRTUAL_MODEL: &str = "mesh";
/// The router's ordinary single-model routing.
const AUTO_MODEL: &str = "auto";
/// How long one catalog observation is trusted.
const CATALOG_TTL: Duration = Duration::from_secs(5);
/// Budget for the catalog probe itself — it sits in the inference path.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(2);
/// Backoff after MoA is withdrawn or fails, so a flapping mesh cannot thrash.
const COOLDOWN: Duration = Duration::from_secs(30);
/// Consecutive positive observations required before enabling MoA.
const ENABLE_OBSERVATIONS: u8 = 2;

/// The 503 body mesh-llm returns when MoA is configured but under-provisioned
/// (`mesh-llm-host-runtime/src/network/openai/moa_gateway/mod.rs:56`).
const MOA_UNAVAILABLE_MESSAGE: &str = "MoA requires ≥2 models available in the mesh";

/// Messages from mesh-llm's *other* failure path: `error_response`
/// (`mesh-mixture-of-agents/src/lib.rs:1168`) returns a 502 whose body carries
/// both `error.message` and `error.type = "moa_failure"`.
///
/// Goose keeps only the message (`http_status.rs:186-197`), so the type never
/// reaches us and we have to match the messages themselves. These are every
/// call site of `error_response` in mesh-llm.
const MOA_FAILURE_MESSAGES: &[&str] = &[
    "All MoA workers failed",
    "All MoA reducers failed",
    "MoA could not produce a usable answer",
    "MoA reducer returned no usable answer",
    "Reducer failed",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Observation {
    Available,
    Unavailable,
    /// Probe failed or was unparseable. Explicitly *not* evidence of absence.
    Unknown,
}

#[derive(Debug, Default)]
struct State {
    last_checked: Option<Instant>,
    consecutive_available: u8,
    collective_enabled: bool,
    cooldown_until: Option<Instant>,
}

/// Does this catalog justify Mixture-of-Agents?
///
/// Requires the virtual `mesh` model to be advertised *and* at least two
/// distinct physical models behind it. `@main` suffixes are stripped so the
/// same model pinned to a branch does not count twice — MoA over one model is
/// pointless.
///
/// `None` means the response had no `data` array, i.e. not a catalog we
/// understand; the caller maps that to [`Observation::Unknown`].
fn catalog_supports_collective(catalog: &Value) -> Option<bool> {
    let models = catalog.get("data").and_then(Value::as_array)?;
    let mut has_virtual_mesh = false;
    let mut physical = BTreeSet::new();
    for id in models
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        if id == VIRTUAL_MODEL {
            has_virtual_mesh = true;
        } else if id != AUTO_MODEL {
            physical.insert(id.replace("@main", ""));
        }
    }
    Some(has_virtual_mesh && physical.len() >= 2)
}

/// Did this error mean "the mesh shrank", as opposed to a generic failure?
///
/// mesh-llm has two failure paths and both must be caught:
///
/// * **503, gateway level** — under-provisioned mesh. Plain message, no JSON
///   (`moa_gateway/mod.rs:56`).
/// * **502, MoA level** — workers or reducers failed. Body carries
///   `error.message` *and* `error.type = "moa_failure"`
///   (`mesh-mixture-of-agents/src/lib.rs:1168`).
///
/// Goose reduces a payload to `error.message` when that field exists
/// (`goose-providers/src/http_status.rs:186-197`), so `moa_failure` never
/// survives — we match [`MOA_FAILURE_MESSAGES`] instead. That second path is
/// the *common* one at runtime (a worker dying mid-turn); the 503 only fires
/// when the mesh is too small to start with.
///
/// Anything else is a real error and must surface — retrying every 5xx as
/// `auto` would mask outages.
fn is_mesh_contraction(err: &ProviderError) -> bool {
    let text = err.to_string();
    if text.contains(MOA_UNAVAILABLE_MESSAGE)
        || MOA_FAILURE_MESSAGES.iter().any(|m| text.contains(m))
    {
        return true;
    }
    // Fallback for the shape goose passes through whole: `moa_failure` with no
    // `error.message` field at all.
    let Some(start) = text.find('{') else {
        return false;
    };
    serde_json::from_str::<Value>(&text[start..])
        .ok()
        .is_some_and(|v| v.pointer("/error/type").and_then(Value::as_str) == Some("moa_failure"))
}

/// Wraps a provider and applies Buzz's relay-mesh `auto` policy.
pub struct MeshAutoProvider {
    inner: Arc<dyn Provider>,
    /// Router base URL, for the `/models` probe.
    base_url: String,
    api_key: String,
    http: reqwest::Client,
    state: Mutex<State>,
}

impl std::fmt::Debug for MeshAutoProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeshAutoProvider")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl MeshAutoProvider {
    /// Wrap `inner`, probing `base_url/models` to decide when to use MoA.
    pub fn new(inner: Arc<dyn Provider>, base_url: String, api_key: String) -> Self {
        Self {
            inner,
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            http: reqwest::Client::new(),
            state: Mutex::new(State::default()),
        }
    }

    /// Should this call be routed to MoA?
    ///
    /// Only rewrites the configured `auto`; an explicitly chosen model is never
    /// second-guessed.
    async fn resolve_model(&self, configured: &str) -> String {
        if configured != AUTO_MODEL {
            return configured.to_string();
        }

        let mut state = self.state.lock().await;
        let now = Instant::now();

        if let Some(until) = state.cooldown_until {
            if now < until {
                return AUTO_MODEL.to_string();
            }
            state.cooldown_until = None;
        }

        if state
            .last_checked
            .is_some_and(|at| at.elapsed() < CATALOG_TTL)
        {
            return self.model_for(state.collective_enabled);
        }

        let observation = self.observe().await;
        let checked_at = Instant::now();
        state.last_checked = Some(checked_at);
        match observation {
            Observation::Available => {
                state.consecutive_available = state.consecutive_available.saturating_add(1);
                if state.consecutive_available >= ENABLE_OBSERVATIONS {
                    state.collective_enabled = true;
                }
            }
            Observation::Unavailable => {
                if state.collective_enabled {
                    state.cooldown_until = Some(checked_at + COOLDOWN);
                }
                state.consecutive_available = 0;
                state.collective_enabled = false;
            }
            // A failed probe is not evidence a stable mesh disappeared.
            // Inference stays authoritative via the contraction fallback.
            Observation::Unknown => {}
        }

        let model = self.model_for(state.collective_enabled);
        tracing::debug!(
            configured_model = configured,
            request_model = %model,
            "relay-mesh auto: resolved request model from live catalog"
        );
        model
    }

    fn model_for(&self, collective: bool) -> String {
        if collective {
            VIRTUAL_MODEL
        } else {
            AUTO_MODEL
        }
        .to_string()
    }

    /// Note that inference just told us the mesh is too small, and back off.
    async fn cool_down(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().await;
        state.last_checked = Some(now);
        state.consecutive_available = 0;
        state.collective_enabled = false;
        state.cooldown_until = Some(now + COOLDOWN);
    }

    async fn observe(&self) -> Observation {
        let url = format!("{}/models", self.base_url);
        let response = match self
            .http
            .get(&url)
            .bearer_auth(&self.api_key)
            .timeout(CATALOG_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "relay-mesh auto: catalog probe failed; preserving last confirmed route"
                );
                return Observation::Unknown;
            }
        };
        if !response.status().is_success() {
            tracing::debug!(
                status = %response.status(),
                "relay-mesh auto: catalog probe unsuccessful; preserving last confirmed route"
            );
            return Observation::Unknown;
        }
        match response.json::<Value>().await {
            Ok(catalog) => match catalog_supports_collective(&catalog) {
                Some(true) => Observation::Available,
                Some(false) => Observation::Unavailable,
                None => {
                    tracing::debug!(
                        "relay-mesh auto: catalog has no data array; preserving last confirmed route"
                    );
                    Observation::Unknown
                }
            },
            Err(error) => {
                tracing::debug!(
                    %error,
                    "relay-mesh auto: invalid catalog response; preserving last confirmed route"
                );
                Observation::Unknown
            }
        }
    }

    fn config_for(model_config: &ModelConfig, model: &str) -> ModelConfig {
        let mut resolved = model_config.clone();
        resolved.model_name = model.to_string();
        resolved
    }
}

#[async_trait]
impl Provider for MeshAutoProvider {
    fn get_name(&self) -> &str {
        self.inner.get_name()
    }

    async fn stream(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let model = self.resolve_model(&model_config.model_name).await;
        let attempting_moa = model == VIRTUAL_MODEL;

        let resolved = Self::config_for(model_config, &model);
        match self.inner.stream(&resolved, system, messages, tools).await {
            Err(error) if attempting_moa && is_mesh_contraction(&error) => {
                // The mesh shrank between discovery and inference. Back off and
                // retry once with ordinary routing so the turn still completes.
                self.cool_down().await;
                tracing::warn!(
                    attempted_model = VIRTUAL_MODEL,
                    fallback_model = AUTO_MODEL,
                    provider_message = %error,
                    "relay-mesh auto: collective request failed; retrying once with auto"
                );
                let fallback = Self::config_for(model_config, AUTO_MODEL);
                self.inner.stream(&fallback, system, messages, tools).await
            }
            other => other,
        }
    }

    async fn complete(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let model = self.resolve_model(&model_config.model_name).await;
        let attempting_moa = model == VIRTUAL_MODEL;

        let resolved = Self::config_for(model_config, &model);
        match self
            .inner
            .complete(&resolved, system, messages, tools)
            .await
        {
            Err(error) if attempting_moa && is_mesh_contraction(&error) => {
                self.cool_down().await;
                tracing::warn!(
                    attempted_model = VIRTUAL_MODEL,
                    fallback_model = AUTO_MODEL,
                    provider_message = %error,
                    "relay-mesh auto: collective request failed; retrying once with auto"
                );
                let fallback = Self::config_for(model_config, AUTO_MODEL);
                self.inner
                    .complete(&fallback, system, messages, tools)
                    .await
            }
            other => other,
        }
    }

    // Everything below must delegate: the wrapper exists only to pick a model.

    async fn get_context_limit(&self, model_config: &ModelConfig) -> Result<usize, ProviderError> {
        self.inner.get_context_limit(model_config).await
    }

    fn retry_config(&self) -> RetryConfig {
        self.inner.retry_config()
    }

    async fn fetch_supported_models(&self) -> Result<Vec<String>, ProviderError> {
        self.inner.fetch_supported_models().await
    }

    async fn fetch_supported_model_info(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.inner.fetch_supported_model_info().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog(ids: &[&str]) -> Value {
        json!({ "data": ids.iter().map(|id| json!({ "id": id })).collect::<Vec<_>>() })
    }

    #[test]
    fn collective_needs_virtual_mesh_and_two_physical_models() {
        assert_eq!(
            catalog_supports_collective(&catalog(&["mesh", "qwen3", "llama3"])),
            Some(true)
        );
    }

    #[test]
    fn one_physical_model_is_not_a_collective() {
        // MoA over a single model is pointless, and mesh-llm 503s on it.
        assert_eq!(
            catalog_supports_collective(&catalog(&["mesh", "qwen3"])),
            Some(false)
        );
    }

    #[test]
    fn virtual_mesh_must_be_advertised() {
        assert_eq!(
            catalog_supports_collective(&catalog(&["qwen3", "llama3"])),
            Some(false)
        );
    }

    #[test]
    fn branch_pins_do_not_count_as_distinct_models() {
        // `qwen3` and `qwen3@main` are the same model.
        assert_eq!(
            catalog_supports_collective(&catalog(&["mesh", "qwen3", "qwen3@main"])),
            Some(false)
        );
    }

    #[test]
    fn auto_is_not_counted_as_a_physical_model() {
        assert_eq!(
            catalog_supports_collective(&catalog(&["mesh", "auto", "qwen3"])),
            Some(false)
        );
    }

    #[test]
    fn missing_data_array_is_unknown_not_false() {
        // Must be distinguishable: unknown preserves the last confirmed route,
        // false actively disables MoA.
        assert_eq!(catalog_supports_collective(&json!({})), None);
        assert_eq!(
            catalog_supports_collective(&json!({"data": []})),
            Some(false)
        );
    }

    #[test]
    fn blank_model_ids_are_ignored() {
        assert_eq!(
            catalog_supports_collective(&catalog(&["mesh", "qwen3", "  ", ""])),
            Some(false)
        );
    }

    #[test]
    fn recognises_the_moa_unavailable_503() {
        let err = ProviderError::ServerError(format!(
            "503: {{\"error\":{{\"message\":\"{MOA_UNAVAILABLE_MESSAGE}\"}}}}"
        ));
        assert!(is_mesh_contraction(&err));
    }

    #[test]
    fn recognises_moa_failure_type_when_no_message_field() {
        // Reachable only because goose falls back to the whole payload when
        // `error.message` is absent (http_status.rs:196).
        let err =
            ProviderError::ServerError("500: {\"error\":{\"type\":\"moa_failure\"}}".to_string());
        assert!(is_mesh_contraction(&err));
    }

    #[test]
    fn recognises_moa_worker_and_reducer_failures() {
        // mesh-llm's 502 path always sends message AND type; goose strips the
        // type, so these must be caught by message. This is the common runtime
        // failure — a worker dying mid-turn.
        for message in [
            "All MoA workers failed",
            "MoA could not produce a usable answer",
            "MoA reducer returned no usable answer",
            "Reducer failed (tried 3): timeout",
        ] {
            let err =
                ProviderError::ServerError(format!("Server error (502) at http://x: {message}"));
            assert!(is_mesh_contraction(&err), "missed: {message}");
        }
    }

    #[test]
    fn ordinary_server_errors_are_not_contractions() {
        // Critical: treating every 5xx as a contraction would silently mask
        // real outages behind an `auto` retry.
        let err =
            ProviderError::ServerError("500: {\"error\":{\"message\":\"internal\"}}".to_string());
        assert!(!is_mesh_contraction(&err));
        assert!(!is_mesh_contraction(&ProviderError::RequestFailed(
            "timeout".to_string()
        )));
    }
}
