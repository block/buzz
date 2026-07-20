use std::collections::{BTreeMap, HashSet};

use nostr::Keys;
use serde::Deserialize;
use tauri::{AppHandle, State};

use super::agent_model_process::run_agent_models_command;

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, current_instance_id, discovery_env_with_baked_floor,
        find_managed_agent_mut, known_acp_runtime, load_managed_agents, load_personas,
        managed_agent_avatar_url, missing_command_message, normalize_agent_args, resolve_command,
        save_managed_agents, sync_managed_agent_processes, try_regenerate_nest, AgentModelInfo,
        AgentModelsResponse, UpdateManagedAgentRequest, UpdateManagedAgentResponse,
        DEFAULT_ACP_COMMAND,
    },
    relay::{relay_ws_url_with_override, sync_managed_agent_profile},
    util::now_iso,
};

/// Query available models from an agent via `buzz-acp models --json`.
///
/// Spawns a short-lived subprocess (no relay connection needed). The subprocess
/// starts the agent, queries its model catalog, and exits. ~2-5s total.
#[tauri::command]
pub async fn get_agent_models(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AgentModelsResponse, String> {
    let (resolved_acp, agent_command, agent_args, persisted_model, effective_provider, merged_env) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_session_cache(pubkey);
        }

        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;

        let resolved = resolve_command(&record.acp_command)
            .ok_or_else(|| missing_command_message(&record.acp_command, "ACP harness command"))?;

        // Resolve the effective harness from the linked persona (mirrors spawn),
        // so model discovery runs against the persona's current harness, not the
        // frozen record snapshot. An explicit per-agent override wins.
        let personas = load_personas(&app).unwrap_or_default();
        let effective_command = crate::managed_agents::record_agent_command(record, &personas);

        let args = normalize_agent_args(&effective_command, record.agent_args.clone());

        let resolved_agent = resolve_command(&effective_command)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| effective_command.clone());

        // ModelPicker can persist a selected model but not rewrite the saved
        // provider/env snapshot, and runtime spawn reads that same snapshot.
        // Discover models against the record snapshot so an out-of-date persona
        // cannot offer models for a provider this agent will not launch with.
        let discovery = saved_agent_model_discovery_config(record, &effective_command);

        (
            resolved,
            resolved_agent,
            args,
            discovery.model,
            discovery.provider,
            discovery.env,
        )
    }; // store lock released — subprocess runs without holding the lock

    let merged_env = discovery_env_with_baked_floor(merged_env);
    if let Some(models) = discover_openai_compatible_models(
        &state.http_client,
        effective_provider.as_deref(),
        &merged_env,
        persisted_model.clone(),
    )
    .await?
    {
        return Ok(models);
    }

    if let Some(models) = discover_anthropic_models(
        &state.http_client,
        effective_provider.as_deref(),
        &merged_env,
        persisted_model.clone(),
    )
    .await?
    {
        return Ok(models);
    }

    if let Some(models) = discover_databricks_models(
        &state.http_client,
        effective_provider.as_deref(),
        &merged_env,
        persisted_model.clone(),
    )
    .await?
    {
        return Ok(models);
    }

    run_agent_models_command(
        resolved_acp,
        agent_command,
        agent_args,
        persisted_model,
        merged_env,
    )
    .await
}

#[derive(Debug, PartialEq, Eq)]
struct SavedAgentModelDiscoveryConfig {
    model: Option<String>,
    provider: Option<String>,
    env: BTreeMap<String, String>,
}

fn saved_agent_model_discovery_config(
    record: &crate::managed_agents::ManagedAgentRecord,
    agent_command: &str,
) -> SavedAgentModelDiscoveryConfig {
    let mut derived_env = BTreeMap::new();
    if let Some(meta) = known_acp_runtime(agent_command) {
        for (key, value) in crate::managed_agents::runtime_metadata_env_vars(
            meta.model_env_var,
            meta.provider_env_var,
            meta.provider_locked,
            record.model.as_deref(),
            record.provider.as_deref(),
        ) {
            derived_env.insert(key.to_string(), value.to_string());
        }
    }

    SavedAgentModelDiscoveryConfig {
        model: record.model.clone(),
        provider: record.provider.clone(),
        env: crate::managed_agents::merged_user_env(&derived_env, &record.env_vars),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverAgentModelsInput {
    #[serde(default)]
    pub acp_command: Option<String>,
    pub agent_command: String,
    #[serde(default)]
    pub agent_args: Vec<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub env_vars: BTreeMap<String, String>,
}

/// Query available models from an unsaved agent configuration.
///
/// This powers the new-agent dialog before a persona/agent record exists. It
/// mirrors the saved-agent discovery command, but derives runtime/provider/env
/// from the current form state instead of loading a persisted record.
#[tauri::command]
pub async fn discover_agent_models(
    input: DiscoverAgentModelsInput,
    state: State<'_, AppState>,
) -> Result<AgentModelsResponse, String> {
    crate::managed_agents::validate_user_env_keys(&input.env_vars)?;

    let acp_command = input
        .acp_command
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_ACP_COMMAND);
    let resolved_acp = resolve_command(acp_command)
        .ok_or_else(|| missing_command_message(acp_command, "ACP harness command"))?;

    let agent_command = input.agent_command.trim();
    if agent_command.is_empty() {
        return Err("agent command is required for model discovery".to_string());
    }
    let agent_args = normalize_agent_args(agent_command, input.agent_args);
    let resolved_agent = resolve_command(agent_command)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| agent_command.to_string());

    let mut derived_env = BTreeMap::new();
    if let Some(meta) = known_acp_runtime(agent_command) {
        let provider = input
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if !meta.provider_locked {
            if let (Some(env_key), Some(provider)) = (meta.provider_env_var, provider) {
                derived_env.insert(env_key.to_string(), provider.to_string());
            }
        }
    }
    let merged_env = crate::managed_agents::merged_user_env(&derived_env, &input.env_vars);
    let merged_env = discovery_env_with_baked_floor(merged_env);

    // Buzz shared compute discovery must not depend on the local OpenAI ingress: that
    // client endpoint is started only after a live target is selected.
    #[cfg(feature = "mesh-llm")]
    if input.provider.as_deref().map(str::trim)
        == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID)
    {
        let events = crate::relay::query_relay(
            &state,
            &[
                crate::mesh_llm::mesh_status_filter(),
                crate::mesh_llm::relay_membership_filter(),
            ],
        )
        .await
        .map_err(|error| format!("Buzz shared compute model discovery failed: {error}"))?;
        let availability = crate::mesh_llm::availability_from_events(events);
        if availability.models.is_empty() {
            return Err(availability.reason.unwrap_or_else(|| {
                "No live Buzz shared compute models are available".to_string()
            }));
        }
        return Ok(AgentModelsResponse {
            agent_name: crate::managed_agents::RELAY_MESH_PROVIDER_ID.to_string(),
            agent_version: "relay-availability".to_string(),
            models: availability
                .models
                .into_iter()
                .map(|model| AgentModelInfo {
                    id: model.id,
                    name: model.name,
                    description: None,
                })
                .collect(),
            agent_default_model: None,
            selected_model: None,
            supports_switching: true,
        });
    }
    #[cfg(not(feature = "mesh-llm"))]
    if input.provider.as_deref().map(str::trim)
        == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID)
    {
        return Err("Buzz shared compute is not available in this build".to_string());
    }

    if let Some(models) = discover_openai_compatible_models(
        &state.http_client,
        input.provider.as_deref(),
        &merged_env,
        None,
    )
    .await?
    {
        return Ok(models);
    }

    if let Some(models) = discover_anthropic_models(
        &state.http_client,
        input.provider.as_deref(),
        &merged_env,
        None,
    )
    .await?
    {
        return Ok(models);
    }

    if let Some(models) = discover_databricks_models(
        &state.http_client,
        input.provider.as_deref(),
        &merged_env,
        None,
    )
    .await?
    {
        return Ok(models);
    }

    run_agent_models_command(resolved_acp, resolved_agent, agent_args, None, merged_env).await
}

#[derive(Debug, Deserialize)]
struct OpenAiModelListResponse {
    data: Vec<OpenAiModelListItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelListItem {
    id: String,
    #[serde(default)]
    created: Option<i64>,
}

fn is_openai_compatible_provider(provider: Option<&str>) -> bool {
    matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("openai" | "openai-compat" | "gemini")
    )
}

fn is_gemini_provider(provider: Option<&str>) -> bool {
    provider
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
        == Some("gemini")
}

/// Base-URL env var + default for an OpenAI-compatible provider. Gemini reuses
/// the discovery path but reads `GEMINI_BASE_URL`, defaulting to Google's
/// OpenAI-compatible surface.
fn openai_compatible_base_url_env(provider: Option<&str>) -> (&'static str, &'static str) {
    if is_gemini_provider(provider) {
        (
            "GEMINI_BASE_URL",
            "https://generativelanguage.googleapis.com/v1beta/openai",
        )
    } else {
        ("OPENAI_COMPAT_BASE_URL", "https://api.openai.com/v1")
    }
}

/// API-key env var for an OpenAI-compatible provider (Gemini uses `GEMINI_API_KEY`).
fn openai_compatible_api_key_env(provider: Option<&str>) -> &'static str {
    if is_gemini_provider(provider) {
        "GEMINI_API_KEY"
    } else {
        "OPENAI_COMPAT_API_KEY"
    }
}

#[cfg(test)]
fn openai_compatible_models_url(env: &BTreeMap<String, String>, provider: Option<&str>) -> String {
    let (key, default) = openai_compatible_base_url_env(provider);
    let base_url = env_value(env, key).unwrap_or_else(|| default.to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn openai_compatible_models_url_for_discovery(
    env: &BTreeMap<String, String>,
    provider: Option<&str>,
) -> String {
    let (key, default) = openai_compatible_base_url_env(provider);
    let base_url = env_or_process_value(env, key).unwrap_or_else(|| default.to_string());
    format!("{}/models", base_url.trim_end_matches('/'))
}

fn env_value(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn env_or_process_value(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env_value(env, key).or_else(|| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn redaction_env_with_value(
    env: &BTreeMap<String, String>,
    key: &str,
    value: &str,
) -> BTreeMap<String, String> {
    let mut redaction_env = env.clone();
    redaction_env.insert(key.to_string(), value.to_string());
    redaction_env
}

fn is_agent_text_model_id(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    if [
        "audio",
        "dall-e",
        "embedding",
        "image",
        "moderation",
        "realtime",
        "speech",
        "transcribe",
        "tts",
        "whisper",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return false;
    }

    lower.starts_with("gpt-") || lower.starts_with('o') || lower.starts_with("chatgpt-")
}

fn openai_dated_snapshot_alias(id: &str) -> Option<String> {
    let (base, date) = id.rsplit_once('-')?;
    if date.len() != 2 || !date.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let (base, month) = base.rsplit_once('-')?;
    if month.len() != 2 || !month.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    let (base, year) = base.rsplit_once('-')?;
    if year.len() != 4 || !year.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }

    Some(base.to_string())
}

fn openai_model_display_name(id: &str) -> String {
    let canonical = openai_dated_snapshot_alias(id).unwrap_or_else(|| id.to_string());
    if let Some(rest) = canonical.strip_prefix("chatgpt-") {
        return format!("ChatGPT {}", title_case_model_suffix(rest));
    }
    if let Some(rest) = canonical.strip_prefix("gpt-") {
        return format!("GPT-{}", title_case_model_suffix(rest));
    }

    canonical
}

fn title_case_model_suffix(value: &str) -> String {
    value
        .split('-')
        .enumerate()
        .map(|(index, part)| {
            let part = if part.eq_ignore_ascii_case("pro") {
                "Pro".to_string()
            } else if part.eq_ignore_ascii_case("mini") {
                "mini".to_string()
            } else if part.eq_ignore_ascii_case("nano") {
                "nano".to_string()
            } else {
                part.to_string()
            };

            if index == 0 {
                part
            } else {
                format!(" {part}")
            }
        })
        .collect::<String>()
}

fn normalize_openai_compatible_models(
    response: OpenAiModelListResponse,
    provider: Option<&str>,
) -> Vec<AgentModelInfo> {
    let mut seen = HashSet::new();
    let mut items = response.data;
    let filter_to_openai_text_models = matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("openai")
    );
    let all_ids = items
        .iter()
        .map(|item| item.id.clone())
        .collect::<HashSet<String>>();
    items.sort_by(|left, right| {
        right
            .created
            .cmp(&left.created)
            .then_with(|| left.id.cmp(&right.id))
    });

    items
        .into_iter()
        .filter(|item| !filter_to_openai_text_models || is_agent_text_model_id(&item.id))
        .filter(|item| match openai_dated_snapshot_alias(&item.id) {
            Some(alias) if filter_to_openai_text_models => !all_ids.contains(&alias),
            Some(_) | None => true,
        })
        .filter(|item| seen.insert(item.id.clone()))
        .map(|item| AgentModelInfo {
            name: Some(openai_model_display_name(&item.id)),
            id: item.id,
            description: None,
        })
        .collect()
}

async fn discover_openai_compatible_models(
    client: &reqwest::Client,
    provider: Option<&str>,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    let relay_mesh = provider.map(str::trim) == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID);
    if !relay_mesh && !is_openai_compatible_provider(provider) {
        return Ok(None);
    }

    let api_key_env = openai_compatible_api_key_env(provider);
    let api_key = if relay_mesh {
        crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER.to_string()
    } else {
        env_or_process_value(env, api_key_env)
            .ok_or_else(|| format!("config: {api_key_env} required"))?
    };
    let redaction_env = redaction_env_with_value(env, api_key_env, &api_key);
    let url = if relay_mesh {
        format!("{}/models", crate::managed_agents::RELAY_MESH_API_BASE_URL)
    } else {
        openai_compatible_models_url_for_discovery(env, provider)
    };
    let response = client
        .get(&url)
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|error| format!("OpenAI model discovery request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body = crate::managed_agents::redact_env_values_in(&body, &redaction_env);
        return Err(format!("OpenAI model discovery HTTP {status}: {body}"));
    }

    let response = response
        .json::<OpenAiModelListResponse>()
        .await
        .map_err(|error| format!("OpenAI model discovery response parse failed: {error}"))?;
    let models = normalize_openai_compatible_models(response, provider);
    if models.is_empty() {
        return Err("OpenAI model discovery returned no compatible text models".to_string());
    }

    Ok(Some(AgentModelsResponse {
        agent_name: provider.unwrap_or("openai").trim().to_string(),
        agent_version: "models-api".to_string(),
        models,
        agent_default_model: None,
        selected_model,
        supports_switching: true,
    }))
}

#[derive(Debug, Deserialize)]
struct AnthropicModelListResponse {
    data: Vec<AnthropicModelListItem>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    last_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModelListItem {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
}

fn is_anthropic_provider(provider: Option<&str>) -> bool {
    matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("anthropic")
    )
}

#[cfg(test)]
fn anthropic_models_url(env: &BTreeMap<String, String>) -> String {
    let base_url = env_value(env, "ANTHROPIC_BASE_URL")
        .unwrap_or_else(|| "https://api.anthropic.com".to_string());
    anthropic_models_url_from_base(&base_url)
}

fn anthropic_models_url_for_discovery(env: &BTreeMap<String, String>) -> String {
    let base_url = env_or_process_value(env, "ANTHROPIC_BASE_URL")
        .unwrap_or_else(|| "https://api.anthropic.com".to_string());
    anthropic_models_url_from_base(&base_url)
}

fn anthropic_models_url_from_base(base_url: &str) -> String {
    let base_url = base_url.trim_end_matches('/');
    if base_url.ends_with("/v1") {
        format!("{base_url}/models")
    } else {
        format!("{base_url}/v1/models")
    }
}

fn normalize_anthropic_models(response: AnthropicModelListResponse) -> Vec<AgentModelInfo> {
    let mut seen = HashSet::new();
    response
        .data
        .into_iter()
        .filter(|item| seen.insert(item.id.clone()))
        .map(|item| AgentModelInfo {
            id: item.id,
            name: item.display_name,
            description: None,
        })
        .collect()
}

async fn fetch_anthropic_model_page(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    after_id: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Result<AnthropicModelListResponse, String> {
    let mut request = client
        .get(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01");
    if let Some(after_id) = after_id {
        request = request.query(&[("after_id", after_id)]);
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("Anthropic model discovery request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let body = crate::managed_agents::redact_env_values_in(&body, env);
        return Err(format!("Anthropic model discovery HTTP {status}: {body}"));
    }

    response
        .json::<AnthropicModelListResponse>()
        .await
        .map_err(|error| format!("Anthropic model discovery response parse failed: {error}"))
}

async fn discover_anthropic_models(
    client: &reqwest::Client,
    provider: Option<&str>,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    if !is_anthropic_provider(provider) {
        return Ok(None);
    }

    let api_key = env_or_process_value(env, "ANTHROPIC_API_KEY")
        .ok_or_else(|| "config: ANTHROPIC_API_KEY required".to_string())?;
    let redaction_env = redaction_env_with_value(env, "ANTHROPIC_API_KEY", &api_key);
    let url = anthropic_models_url_for_discovery(env);
    let mut models = Vec::new();
    let mut after_id: Option<String> = None;
    for _ in 0..20 {
        let response =
            fetch_anthropic_model_page(client, &url, &api_key, after_id.as_deref(), &redaction_env)
                .await?;
        let has_more = response.has_more;
        after_id = response.last_id.clone();
        models.extend(normalize_anthropic_models(response));
        if !has_more {
            break;
        }
        if after_id.as_deref().unwrap_or_default().is_empty() {
            return Err("Anthropic model discovery pagination did not return last_id".to_string());
        }
    }
    let mut seen = HashSet::new();
    models.retain(|model| seen.insert(model.id.clone()));
    if models.is_empty() {
        return Err("Anthropic model discovery returned no models".to_string());
    }

    Ok(Some(AgentModelsResponse {
        agent_name: provider.unwrap_or("anthropic").trim().to_string(),
        agent_version: "models-api".to_string(),
        models,
        agent_default_model: None,
        selected_model,
        supports_switching: true,
    }))
}

// ---------------------------------------------------------------------------
// Databricks model discovery (v1 + v2)
// ---------------------------------------------------------------------------
//
// Delegates to buzz_agent_pkg::catalog::discover_databricks_models, which
// acquires auth in-process via build_token_source:
//   - Static bearer (DATABRICKS_TOKEN): returned immediately.
//   - PKCE cache hit: returned from disk without a browser flow.
//   - No token, no cache: returns Err(LlmAuth) → we return Ok(None) and fall
//     through to run_agent_models_command. Never hangs, never opens a browser.

fn is_databricks_provider(provider: Option<&str>) -> bool {
    matches!(
        provider
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("databricks" | "databricks_v2" | "databricks-v2")
    )
}

fn databricks_agent_provider(provider: &str) -> buzz_agent_pkg::config::Provider {
    if provider.trim().eq_ignore_ascii_case("databricks_v2")
        || provider.trim().eq_ignore_ascii_case("databricks-v2")
    {
        buzz_agent_pkg::config::Provider::DatabricksV2
    } else {
        buzz_agent_pkg::config::Provider::Databricks
    }
}

async fn discover_databricks_models(
    _client: &reqwest::Client,
    provider: Option<&str>,
    env: &BTreeMap<String, String>,
    selected_model: Option<String>,
) -> Result<Option<AgentModelsResponse>, String> {
    let provider_str = match provider {
        Some(p) if is_databricks_provider(Some(p)) => p,
        _ => return Ok(None),
    };

    let host = match env_or_process_value(env, "DATABRICKS_HOST") {
        Some(h) => h,
        None => return Ok(None), // no host → fall through to subprocess
    };

    // api_key = DATABRICKS_TOKEN (empty string = use PKCE cache).
    let api_key = env_or_process_value(env, "DATABRICKS_TOKEN").unwrap_or_default();

    let agent_provider = databricks_agent_provider(provider_str);
    let cfg = buzz_agent_pkg::config::Config::for_discovery(agent_provider, api_key, host);

    // Build a redaction env so the token never appears in surfaced errors.
    let token_for_redact = env_or_process_value(env, "DATABRICKS_TOKEN").unwrap_or_default();
    let redaction_env = redaction_env_with_value(env, "DATABRICKS_TOKEN", &token_for_redact);

    let entries = match buzz_agent_pkg::discover_databricks_models(&cfg).await {
        Ok(e) => e,
        Err(buzz_agent_pkg::AgentError::LlmAuth(_)) => {
            // No token + no PKCE cache → fall through to subprocess.
            return Ok(None);
        }
        Err(e) => {
            let msg = crate::managed_agents::redact_env_values_in(&e.to_string(), &redaction_env);
            return Err(format!("Databricks model discovery failed: {msg}"));
        }
    };

    if entries.is_empty() {
        return Err("Databricks model discovery returned no models".to_string());
    }

    let models = entries
        .into_iter()
        .map(|e| AgentModelInfo {
            id: e.id,
            name: Some(e.name),
            description: None,
        })
        .collect();

    Ok(Some(AgentModelsResponse {
        agent_name: provider_str.trim().to_string(),
        agent_version: "models-api".to_string(),
        models,
        agent_default_model: None,
        selected_model,
        supports_switching: true,
    }))
}

/// Update mutable fields on an existing managed agent record.
///
/// Does NOT auto-restart the agent. Runtime config changes (system prompt,
/// parallelism, commands, toolsets) take effect on the next agent spawn.
/// Name changes are synced to the relay immediately via a kind:0 re-publish.
#[tauri::command]
pub async fn update_managed_agent(
    input: UpdateManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateManagedAgentResponse, String> {
    // Phase 1: local save (synchronous, under lock)
    let (summary, sync_params) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let (_, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        for pubkey in &exited_pubkeys {
            state.clear_session_cache(pubkey);
        }

        let record = find_managed_agent_mut(&mut records, &input.pubkey)?;

        let mut name_changed = false;
        if let Some(name_update) = input.name {
            let trimmed = name_update.trim().to_string();
            if !trimmed.is_empty() && trimmed != record.name {
                record.name = trimmed;
                name_changed = true;
            }
        }
        if let Some(model_update) = input.model {
            record.model = model_update;
        }
        if let Some(provider_update) = input.provider {
            record.provider = provider_update;
        }
        if let Some(prompt_update) = input.system_prompt {
            record.system_prompt = prompt_update;
        }
        if let Some(parallelism) = input.parallelism {
            record.parallelism = parallelism;
        }
        // turn_timeout_seconds is intentionally not applied here —
        // BUZZ_ACP_TURN_TIMEOUT is deprecated and ignored by the harness.
        // Use idle_timeout_seconds or max_turn_duration_seconds instead.
        // Store the relay override exactly as supplied (trimmed). An explicit
        // value pins the agent; empty falls back to the workspace relay at
        // read-time. A name-only edit (relay_url == None) leaves the pin intact.
        if let Some(relay_url) = input.relay_url {
            record.relay_url = relay_url.trim().to_string();
        }
        if let Some(acp_command) = input.acp_command {
            record.acp_command = acp_command;
        }
        // Harness edit: the persona's runtime is authoritative, so an explicit
        // `agent_command_override` is persisted ONLY when the user picks a
        // command that diverges from the persona, and the empty/whitespace
        // "Inherit from persona" sentinel clears both the pin and the
        // materialized record runtime. A name-only edit
        // (`agent_command == None`) leaves the pin intact. `harness_override`
        // threads the user's explicit intent — see `apply_agent_command_update`
        // and `update_time_agent_command_override` for the full resolution
        // rules.
        if let Some(agent_command) = input.agent_command {
            let personas = load_personas(&app).unwrap_or_default();
            crate::managed_agents::apply_agent_command_update(
                record,
                &personas,
                &agent_command,
                input.harness_override,
            );
        }
        if let Some(agent_args) = input.agent_args {
            record.agent_args = agent_args;
        }
        // mcp_command is intentionally not applied here — the effective MCP
        // command is always catalog-derived (known_acp_runtime at spawn time)
        // and the per-record field is never read by the runtime.
        if let Some(env_vars) = input.env_vars {
            crate::managed_agents::validate_user_env_keys(&env_vars)?;
            record.env_vars = env_vars;
        }

        // Native provider/model fields are authoritative. Keep the typed marker
        // derived for new records while retaining legacy typed records for
        // non-native providers.
        if record.provider.as_deref() == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID) {
            let model_ref = record
                .model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(crate::managed_agents::RELAY_MESH_AUTO_MODEL_ID)
                .to_string();
            record.model = Some(model_ref.clone());
            record.relay_mesh = Some(crate::managed_agents::RelayMeshConfig { model_ref });
        }

        // Inbound author gate: merge patch onto current values, then validate
        // the merged state. This lets a single update switch to Allowlist AND
        // supply pubkeys atomically.
        let prospective_mode = input.respond_to.unwrap_or(record.respond_to);
        let prospective_allowlist = match input.respond_to_allowlist.as_ref() {
            Some(list) => crate::managed_agents::validate_respond_to_allowlist(list)?,
            None => record.respond_to_allowlist.clone(),
        };
        if prospective_mode == crate::managed_agents::RespondTo::Allowlist
            && prospective_allowlist.is_empty()
        {
            return Err(
                "respond-to mode 'allowlist' requires at least one pubkey in the allowlist"
                    .to_string(),
            );
        }
        record.respond_to = prospective_mode;
        // Preserve the persisted allowlist across mode toggles — only replace
        // when the caller explicitly supplied a new list.
        if input.respond_to_allowlist.is_some() {
            record.respond_to_allowlist = prospective_allowlist;
        }

        record.updated_at = now_iso();

        save_managed_agents(&app, &records)?;

        let record = records
            .iter()
            .find(|r| r.pubkey == input.pubkey)
            .ok_or_else(|| format!("agent {} not found", input.pubkey))?;

        // Publish the edit to the relay. After-save, inside the lock, before
        // any .await. The retention upsert hashes the opt-IN projection, so an
        // update that touched only runtime/local fields is a no-op publish.
        super::agents::retain_managed_agent_pending(&app, &state, record);

        let sync_params = if name_changed {
            let agent_keys = Keys::parse(&record.private_key_nsec)
                .map_err(|e| format!("failed to parse agent keys: {e}"))?;
            // Re-publish the renamed profile to the agent's effective relay:
            // an explicit per-agent relay wins; empty falls back to workspace.
            let relay_url = crate::relay::effective_agent_relay_url(
                &record.relay_url,
                &relay_ws_url_with_override(&state),
            );
            let display_name = record.name.clone();
            // Avatar fallback derives from the EFFECTIVE harness (persona-wins),
            // not the frozen snapshot, so an inherited harness picks the right
            // default avatar.
            let personas = load_personas(&app).unwrap_or_default();
            let effective_command = crate::managed_agents::record_agent_command(record, &personas);
            let avatar_url = record
                .avatar_url
                .clone()
                .or_else(|| managed_agent_avatar_url(&effective_command));
            let auth_tag = record.auth_tag.clone();
            Some((agent_keys, relay_url, display_name, avatar_url, auth_tag))
        } else {
            None
        };

        let summary = {
            let personas = load_personas(&app).unwrap_or_default();
            build_managed_agent_summary(&app, record, &runtimes, &personas)?
        };
        (summary, sync_params)
    }; // lock dropped here

    try_regenerate_nest(&app);

    // Phase 2: relay profile sync (async, best-effort, outside lock)
    let profile_sync_error =
        if let Some((agent_keys, relay_url, display_name, avatar_url, auth_tag)) = sync_params {
            match sync_managed_agent_profile(
                &state,
                &relay_url,
                &agent_keys,
                &display_name,
                avatar_url.as_deref(),
                auth_tag.as_deref(),
            )
            .await
            {
                Ok(()) => None,
                Err(e) => {
                    eprintln!("buzz-desktop: relay profile sync failed after rename: {e}");
                    Some(e)
                }
            }
        } else {
            None
        };

    Ok(UpdateManagedAgentResponse {
        agent: summary,
        profile_sync_error,
    })
}

// ── Model normalization ───────────────────────────────────────────────────────

/// Normalize raw `buzz-acp models --json` output into a typed DTO for the frontend.
///
/// Merges models from both ACP paths (stable configOptions + unstable SessionModelState),
/// deduplicates by ID (stable takes precedence), and returns a unified list.
pub(super) fn normalize_agent_models(
    raw: &serde_json::Value,
    persisted_model: Option<String>,
) -> AgentModelsResponse {
    let agent_name = raw["agent"]["name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let agent_version = raw["agent"]["version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let mut models: Vec<AgentModelInfo> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    // 1. Stable configOptions (preferred). Only entries with category "model"
    //    are model options — the CLI pre-filters, but we're defensive here.
    if let Some(config_options) = raw["stable"]["configOptions"].as_array() {
        for opt in config_options {
            if opt.get("category").and_then(|c| c.as_str()) != Some("model") {
                continue;
            }
            if let Some(options) = opt.get("options").and_then(|v| v.as_array()) {
                for o in options {
                    if let Some(value) = o.get("value").and_then(|v| v.as_str()) {
                        if seen_ids.insert(value.to_string()) {
                            models.push(AgentModelInfo {
                                id: value.to_string(),
                                name: o
                                    .get("displayName")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string),
                                description: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // 2. Unstable availableModels (fallback — skip duplicates from stable).
    let mut agent_default_model: Option<String> = None;
    if let Some(unstable) = raw.get("unstable") {
        agent_default_model = unstable["currentModelId"].as_str().map(str::to_string);
        if let Some(available) = unstable["availableModels"].as_array() {
            for m in available {
                if let Some(id) = m.get("modelId").and_then(|v| v.as_str()) {
                    if seen_ids.insert(id.to_string()) {
                        models.push(AgentModelInfo {
                            id: id.to_string(),
                            name: m.get("name").and_then(|v| v.as_str()).map(str::to_string),
                            description: m
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        });
                    }
                }
            }
        }
    }

    let supports_switching = !models.is_empty();

    AgentModelsResponse {
        agent_name,
        agent_version,
        models,
        agent_default_model,
        selected_model: persisted_model,
        supports_switching,
    }
}

#[cfg(test)]
#[path = "agent_models_tests.rs"]
mod tests;
