use std::collections::BTreeMap;

mod coordinator;
pub(crate) use coordinator::{
    publish_current_status_once, publish_stopped_status_once, RelayMeshConnectRequest,
};
pub use coordinator::{spawn_listener, start_client, MeshCoordinator};

mod discovery;
pub use discovery::{availability_from_events, mesh_status_filter};
use discovery::{device_name_from_status, endpoint_id_from_status, enrich_status_payload_identity};

mod preset;
pub use preset::{agent_preset, MeshAgentPreset, MeshAgentPresetRequest};

use mesh_llm_sdk::{client, serve, EmbeddedNodeHandle, MeshDiscoveryMode};
use serde::{Deserialize, Serialize};

const DEFAULT_MESH_API_PORT: u16 = 9337;
const DEFAULT_MESH_CONSOLE_PORT: u16 = 3131;
const MESH_STATUS_KIND: u64 = 30_621;
const MESH_API_PORT_ENV: &str = "BUZZ_MESH_API_PORT";
const MESH_CONSOLE_PORT_ENV: &str = "BUZZ_MESH_CONSOLE_PORT";
const RELAY_MESH_API_KEY_PLACEHOLDER: &str = "buzz-mesh-local";
/// ACP provider relay-mesh agents run on. Sources of truth for its command +
/// MCP live in the runtime catalog (`known_acp_runtime_exact`); these are
/// only the fallbacks. `buzz-agent` reads the `BUZZ_AGENT_PROVIDER` /
/// `OPENAI_COMPAT_*` env vars below — goose (the global default) does not.
const MESH_AGENT_PROVIDER_ID: &str = "buzz-agent";
const MESH_AGENT_MCP_COMMAND: &str = "buzz-dev-mcp";

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
    #[serde(default)]
    pub reporter_pubkey: Option<String>,
    #[serde(default)]
    pub endpoint_id: Option<String>,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnsureMeshClientRequest {
    pub model_id: String,
    #[serde(default)]
    pub endpoint_addr: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
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
        endpoint_id: None,
        device_id: None,
        device_name: None,
    }
}

pub struct DesktopMeshRuntime {
    handle: EmbeddedNodeHandle,
    mode: MeshNodeMode,
    model_id: Option<String>,
    model_name: Option<String>,
}

fn initialize_mesh_native_runtime() -> anyhow::Result<()> {
    let cache = mesh_llm_sdk::native_runtime::native_runtime_cache(None)?;
    let installed = cache.installed()?;
    let current = mesh_llm_sdk::native_runtime::CURRENT_MESH_VERSION;
    if !installed
        .iter()
        .any(|runtime| runtime.mesh_version == current)
    {
        anyhow::bail!(
            "mesh native runtime for MeshLLM {current} is not installed; run `just mesh=1 staging` or `just mesh-e2e-hardware` to prepare it"
        );
    }
    mesh_llm_host_runtime::initialize_host_runtime().map_err(|error| {
        anyhow::anyhow!(
            "mesh native runtime failed to load; run `just mesh=1 staging` or `just mesh-e2e-hardware` to repair it: {error}"
        )
    })
}

impl DesktopMeshRuntime {
    pub async fn start(request: StartMeshNodeRequest) -> anyhow::Result<Self> {
        validate_no_leak_request(&request)?;
        initialize_mesh_native_runtime()?;
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
                    .api_port(mesh_api_port()?)
                    .console_port(mesh_console_port()?)
                    .publish(false)
                    .auto_join(false)
                    .disable_iroh_relays(true)
                    .discovery_mode(MeshDiscoveryMode::Nostr)
                    .console_ui(true);
                if let Some(max_vram_gb) = request.max_vram_gb {
                    builder = builder.max_vram_gb(max_vram_gb as f64);
                }
                if let Some(join_token) = request.join_token.as_deref() {
                    builder = builder.join_token(join_token);
                }
                serve::start(builder.build()).await?
            }
            MeshNodeMode::Client => {
                let mut builder = client::EmbeddedClientConfig::builder()
                    .api_port(mesh_api_port()?)
                    .console_port(mesh_console_port()?)
                    .publish(false)
                    .auto_join(false)
                    .disable_iroh_relays(true)
                    .discovery_mode(MeshDiscoveryMode::Nostr)
                    .console_ui(true);
                if let Some(join_token) = request.join_token.as_deref() {
                    builder = builder.join_token(join_token);
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

    pub async fn status_report_payload(&self) -> anyhow::Result<serde_json::Value> {
        let status = self.handle.status().await?;
        let mut payload = status.payload;
        enrich_status_payload_identity(&mut payload, status.invite_token.as_deref());
        Ok(payload)
    }

    pub async fn dial_endpoint_addr(&self, endpoint_addr: impl Into<String>) -> anyhow::Result<()> {
        self.handle.join_token(endpoint_addr).await
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
        let endpoint_id = endpoint_id_from_status(&status.payload, status.invite_token.as_deref());
        let device_name = device_name_from_status(&status.payload, endpoint_id.as_deref());
        let device_id = endpoint_id.clone();
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
            endpoint_id,
            device_id,
            device_name,
        })
    }

    pub async fn stop(self) -> anyhow::Result<()> {
        self.handle.stop().await
    }
}

fn mesh_api_port() -> anyhow::Result<u16> {
    mesh_port_from_env(MESH_API_PORT_ENV, DEFAULT_MESH_API_PORT)
}

fn mesh_console_port() -> anyhow::Result<u16> {
    mesh_port_from_env(MESH_CONSOLE_PORT_ENV, DEFAULT_MESH_CONSOLE_PORT)
}

fn mesh_port_from_env(name: &str, default: u16) -> anyhow::Result<u16> {
    let Some(raw) = std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(default);
    };
    let port = raw
        .parse::<u16>()
        .map_err(|error| anyhow::anyhow!("{name} must be a TCP port (got {raw:?}): {error}"))?;
    if port == 0 {
        anyhow::bail!("{name} must be a non-zero TCP port");
    }
    Ok(port)
}

fn relay_mesh_api_base_url() -> Result<String, String> {
    let port = mesh_api_port().map_err(|error| error.to_string())?;
    Ok(format!("http://127.0.0.1:{port}/v1"))
}

fn validate_no_leak_request(request: &StartMeshNodeRequest) -> anyhow::Result<()> {
    if request.join_token.as_deref().is_some_and(str::is_empty) {
        anyhow::bail!("joinToken cannot be empty when provided");
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
    // Match a typed phase field (not stringify-and-grep over the whole payload).
    let phase = ["phase", "status", "state", "stage"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(serde_json::Value::as_str))?
        .to_ascii_lowercase();
    for needle in ["download", "fetch", "resolv", "prepar"] {
        if phase.contains(needle) {
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
    // Family-agnostic: a bare string is a ref only via URI scheme or .gguf ext.
    let trimmed = value.trim();
    !trimmed.is_empty()
        && (trimmed.starts_with("hf://") || trimmed.to_ascii_lowercase().ends_with(".gguf"))
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

pub(super) fn dedupe_models(models: Vec<MeshModelOption>) -> Vec<MeshModelOption> {
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

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
