use std::collections::BTreeMap;

use mesh_llm_sdk::{client, serve, EmbeddedNodeHandle, MeshDiscoveryMode};
use serde::{Deserialize, Serialize};

const DEFAULT_MESH_API_PORT: u16 = 9337;
const DEFAULT_MESH_CONSOLE_PORT: u16 = 3131;
const MESH_STATUS_KIND: u64 = 30_621;
const RELAY_MESH_API_BASE_URL: &str = "http://127.0.0.1:9337/v1";
const RELAY_MESH_API_KEY_PLACEHOLDER: &str = "sprout-mesh-local";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshModelOption {
    pub id: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeshServeTarget {
    pub model_id: String,
    pub model_name: Option<String>,
    pub endpoint_addr: String,
    pub node_name: Option<String>,
    pub capacity: Option<MeshTargetCapacity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeshTargetCapacity {
    pub vram_gb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeshHealthStatus {
    Ok,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshHealth {
    pub status: MeshHealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl MeshHealth {
    fn ok() -> Self {
        Self {
            status: MeshHealthStatus::Ok,
            reason: None,
        }
    }

    fn degraded(reason: impl Into<String>) -> Self {
        Self {
            status: MeshHealthStatus::Degraded,
            reason: Some(reason.into()),
        }
    }

    fn failed(reason: impl Into<String>) -> Self {
        Self {
            status: MeshHealthStatus::Failed,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeshAvailability {
    pub capable: bool,
    pub admitted: bool,
    pub available: bool,
    pub reason: Option<String>,
    pub models: Vec<MeshModelOption>,
    pub serve_targets: Vec<MeshServeTarget>,
}

impl MeshAvailability {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            capable: false,
            admitted: false,
            available: false,
            reason: Some(reason.into()),
            models: Vec::new(),
            serve_targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeshNodeMode {
    Serve,
    Client,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeshNodeState {
    Off,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartMeshNodeRequest {
    pub mode: MeshNodeMode,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub max_vram_gb: Option<u64>,
    #[serde(default)]
    pub join_token: Option<String>,
    #[serde(default)]
    pub iroh_relay_url: Option<String>,
    #[serde(default)]
    pub iroh_relay_auth: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeshNodeStatus {
    pub state: MeshNodeState,
    pub mode: Option<MeshNodeMode>,
    pub health: MeshHealth,
    pub api_base_url: Option<String>,
    pub console_url: Option<String>,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_token: Option<String>,
}

pub fn stopped_status() -> MeshNodeStatus {
    MeshNodeStatus {
        state: MeshNodeState::Off,
        mode: None,
        health: MeshHealth::ok(),
        api_base_url: None,
        console_url: None,
        model_id: None,
        model_name: None,
        invite_token: None,
    }
}

pub struct DesktopMeshRuntime {
    handle: EmbeddedNodeHandle,
    mode: MeshNodeMode,
    model_id: Option<String>,
    model_name: Option<String>,
}

impl DesktopMeshRuntime {
    pub async fn start(request: StartMeshNodeRequest) -> anyhow::Result<Self> {
        validate_no_leak_request(&request)?;
        let model_id = request
            .model_id
            .clone()
            .filter(|value| !value.trim().is_empty());
        let model_name = model_id.clone();
        let handle = match request.mode {
            MeshNodeMode::Serve => {
                let model = model_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("modelId is required for serve mode"))?;
                let mut builder = serve::EmbeddedServeConfig::builder()
                    .model(model)
                    .api_port(DEFAULT_MESH_API_PORT)
                    .console_port(DEFAULT_MESH_CONSOLE_PORT)
                    .publish(false)
                    .auto_join(false)
                    .discovery_mode(MeshDiscoveryMode::Nostr)
                    .console_ui(true);
                if let Some(max_vram_gb) = request.max_vram_gb {
                    builder = builder.max_vram_gb(max_vram_gb as f64);
                }
                if let Some(join_token) = request.join_token.as_deref() {
                    builder = builder.join_token(join_token);
                }
                if let Some(relay) = request.iroh_relay_url.as_deref() {
                    builder = builder.iroh_relay(relay);
                    if let Some(auth) = request.iroh_relay_auth.as_deref() {
                        builder = builder.iroh_relay_auth(relay, auth);
                    }
                }
                serve::start(builder.build()).await?
            }
            MeshNodeMode::Client => {
                let mut builder = client::EmbeddedClientConfig::builder()
                    .api_port(DEFAULT_MESH_API_PORT)
                    .console_port(DEFAULT_MESH_CONSOLE_PORT)
                    .publish(false)
                    .auto_join(false)
                    .discovery_mode(MeshDiscoveryMode::Nostr)
                    .console_ui(true);
                if let Some(join_token) = request.join_token.as_deref() {
                    builder = builder.join_token(join_token);
                }
                if let Some(relay) = request.iroh_relay_url.as_deref() {
                    builder = builder.iroh_relay(relay);
                    if let Some(auth) = request.iroh_relay_auth.as_deref() {
                        builder = builder.iroh_relay_auth(relay, auth);
                    }
                }
                client::start(builder.build()).await?
            }
        };

        Ok(Self {
            handle,
            mode: request.mode,
            model_id,
            model_name,
        })
    }

    pub async fn status(&self) -> anyhow::Result<MeshNodeStatus> {
        let status = self.handle.status().await?;
        self.status_from_sdk(status)
    }

    pub async fn installed_models(&self) -> anyhow::Result<Vec<MeshModelOption>> {
        let status = self.handle.status().await?;
        Ok(models_from_status_payload(Some(&status.payload)))
    }

    fn status_from_sdk(
        &self,
        status: mesh_llm_sdk::EmbeddedNodeStatus,
    ) -> anyhow::Result<MeshNodeStatus> {
        let health = health_from_payload(&status.payload);
        Ok(MeshNodeStatus {
            state: if matches!(health.status, MeshHealthStatus::Failed) {
                MeshNodeState::Failed
            } else {
                MeshNodeState::Running
            },
            mode: Some(self.mode),
            health,
            api_base_url: Some(status.api_base_url),
            console_url: Some(status.console_url),
            model_id: self.model_id.clone(),
            model_name: self.model_name.clone(),
            invite_token: status.invite_token,
        })
    }

    pub async fn stop(self) -> anyhow::Result<()> {
        self.handle.stop().await
    }
}

fn validate_no_leak_request(request: &StartMeshNodeRequest) -> anyhow::Result<()> {
    if request.join_token.as_deref().is_some_and(str::is_empty) {
        anyhow::bail!("joinToken cannot be empty when provided");
    }
    if request.iroh_relay_url.as_deref().is_none_or(str::is_empty) {
        anyhow::bail!("relay NIP-11 must advertise iroh_relay_url before starting mesh");
    }
    if request.iroh_relay_auth.as_deref().is_none_or(str::is_empty) {
        anyhow::bail!("iroh relay admission bearer is required before starting mesh");
    }
    Ok(())
}

fn health_from_payload(payload: &serde_json::Value) -> MeshHealth {
    if let Some(reason) = find_progressish_reason(payload) {
        return MeshHealth::degraded(reason);
    }
    if let Some(status) = payload.get("status").and_then(serde_json::Value::as_str) {
        if matches!(status, "failed" | "error") {
            return MeshHealth::failed(status);
        }
    }
    MeshHealth::ok()
}

fn find_progressish_reason(value: &serde_json::Value) -> Option<String> {
    let text = value.to_string().to_ascii_lowercase();
    for needle in ["download", "fetch", "resolv", "prepar"] {
        if text.contains(needle) {
            return Some(match needle {
                "download" => "downloading model".to_string(),
                "fetch" => "fetching model".to_string(),
                "resolv" => "resolving model".to_string(),
                _ => "preparing model".to_string(),
            });
        }
    }
    None
}

pub fn models_from_status_payload(payload: Option<&serde_json::Value>) -> Vec<MeshModelOption> {
    let mut out = Vec::new();
    if let Some(payload) = payload {
        collect_model_options(payload, &mut out);
    }
    dedupe_models(out)
}

fn collect_model_options(value: &serde_json::Value, out: &mut Vec<MeshModelOption>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(id) = map
                .get("model_id")
                .or_else(|| map.get("modelId"))
                .or_else(|| map.get("model_ref"))
                .or_else(|| map.get("modelRef"))
                .or_else(|| map.get("id"))
                .and_then(serde_json::Value::as_str)
            {
                let name = map
                    .get("name")
                    .or_else(|| map.get("display_name"))
                    .or_else(|| map.get("displayName"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string);
                push_model(out, id, name);
            }
            for child in map.values() {
                collect_model_options(child, out);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_model_options(child, out);
            }
        }
        serde_json::Value::String(value) if looks_like_model_ref(value) => {
            push_model(out, value, None);
        }
        _ => {}
    }
}

fn looks_like_model_ref(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && (trimmed.starts_with("hf://")
            || trimmed.ends_with(".gguf")
            || trimmed.contains("Qwen")
            || trimmed.contains("Llama")
            || trimmed.contains("GGUF"))
}

fn push_model(out: &mut Vec<MeshModelOption>, id: &str, name: Option<String>) {
    let id = id.trim();
    if id.is_empty() || id.starts_with("http://") || id.starts_with("https://") {
        return;
    }
    out.push(MeshModelOption {
        id: id.to_string(),
        name,
    });
}

fn dedupe_models(models: Vec<MeshModelOption>) -> Vec<MeshModelOption> {
    let mut by_id = BTreeMap::<String, Option<String>>::new();
    for model in models {
        by_id
            .entry(model.id)
            .and_modify(|name| {
                if name.is_none() {
                    *name = model.name.clone();
                }
            })
            .or_insert(model.name);
    }
    by_id
        .into_iter()
        .map(|(id, name)| MeshModelOption { id, name })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshAgentPresetRequest {
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshAgentPreset {
    pub provider_id: String,
    pub label: String,
    pub acp_command: String,
    pub agent_command: String,
    pub agent_args: Vec<String>,
    pub mcp_command: String,
    pub model: String,
    pub env_vars: BTreeMap<String, String>,
}

pub fn agent_preset(request: MeshAgentPresetRequest) -> Result<MeshAgentPreset, String> {
    let model = request.model_id.trim();
    if model.is_empty() {
        return Err("modelId is required".to_string());
    }
    Ok(MeshAgentPreset {
        provider_id: "relay-mesh".to_string(),
        label: "Relay mesh".to_string(),
        acp_command: crate::managed_agents::DEFAULT_ACP_COMMAND.to_string(),
        agent_command: crate::managed_agents::DEFAULT_AGENT_COMMAND.to_string(),
        agent_args: Vec::new(),
        mcp_command: crate::managed_agents::DEFAULT_MCP_COMMAND.to_string(),
        model: model.to_string(),
        env_vars: BTreeMap::from([
            ("SPROUT_AGENT_PROVIDER".to_string(), "openai".to_string()),
            (
                "OPENAI_COMPAT_BASE_URL".to_string(),
                RELAY_MESH_API_BASE_URL.to_string(),
            ),
            ("OPENAI_COMPAT_MODEL".to_string(), model.to_string()),
            (
                "OPENAI_COMPAT_API_KEY".to_string(),
                RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
            ),
            ("OPENAI_COMPAT_API".to_string(), "chat".to_string()),
        ]),
    })
}

pub fn availability_from_events(events: Vec<nostr::Event>) -> MeshAvailability {
    let Some(event) = events.into_iter().max_by_key(|event| event.created_at) else {
        return MeshAvailability::unavailable("relay mesh status is not published yet");
    };

    let Ok(content) = serde_json::from_str::<serde_json::Value>(&event.content) else {
        return MeshAvailability::unavailable("relay mesh status is malformed");
    };

    let serve_targets = content
        .get("serveTargets")
        .or_else(|| content.get("serve_targets"))
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<MeshServeTarget>>(value).ok())
        .unwrap_or_default();
    let models = content
        .get("models")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<MeshModelOption>>(value).ok())
        .unwrap_or_else(|| {
            dedupe_models(
                serve_targets
                    .iter()
                    .map(|target| MeshModelOption {
                        id: target.model_id.clone(),
                        name: target.model_name.clone(),
                    })
                    .collect(),
            )
        });

    let available = !serve_targets.is_empty();
    MeshAvailability {
        capable: true,
        admitted: true,
        available,
        reason: if available {
            None
        } else {
            Some("no relay mesh serve targets are available".to_string())
        },
        models,
        serve_targets,
    }
}

pub fn mesh_status_filter() -> serde_json::Value {
    serde_json::json!({
        "kinds": [MESH_STATUS_KIND],
        "#d": ["sprout-relay-mesh"],
        "limit": 1
    })
}
