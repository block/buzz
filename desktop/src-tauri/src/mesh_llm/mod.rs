use std::collections::BTreeMap;

mod coordinator;
pub(crate) use coordinator::{publish_current_status_once, publish_stopped_status_once};
pub use coordinator::{start_coordinator, MeshCoordinator, KIND_BUZZ_MESH_MEMBER_STATUS};

mod discovery;
pub use discovery::{
    availability_from_events, mesh_status_filter, owner_ids_from_events, relay_membership_filter,
};
pub(crate) use discovery::{
    current_member_pubkeys, has_membership_snapshot, MESH_STATUS_PAGE_SIZE,
};
use discovery::{device_name_from_status, endpoint_id_from_status, enrich_status_payload_identity};

mod catalog;
pub use catalog::{model_catalog, MeshModelCatalog};

mod identity;
pub use identity::ensure_owner_identity;

mod progress;
pub use progress::install_progress_sink;

mod transport_policy;
#[cfg(test)]
use transport_policy::iroh_relay_mode_from;
use transport_policy::{iroh_relay_mode, validate_advertised_endpoint, IrohRelayMode};

use mesh_llm_sdk::{client, serve, EmbeddedNodeHandle, MeshDiscoveryMode, TrustPolicy};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_MESH_API_PORT: u16 = 9337;
const DEFAULT_MESH_CONSOLE_PORT: u16 = 3131;
const MESH_STATUS_KIND: u64 = KIND_BUZZ_MESH_MEMBER_STATUS as u64;
const MESH_API_PORT_ENV: &str = "BUZZ_MESH_API_PORT";
const MESH_CONSOLE_PORT_ENV: &str = "BUZZ_MESH_CONSOLE_PORT";
/// Iroh relay tunneling for symmetric-NAT peers. Unset/empty/"1"/"default" =
/// enabled with the SDK's default iroh relays (the default — members connect
/// regardless of NAT). "0" = disabled (direct QUIC only, for
/// metadata-conscious deployments). Any other value = comma-separated custom
/// iroh relay URLs. Relays forward end-to-end encrypted QUIC (ciphertext
/// only) and are transport-only; mesh presence is NEVER published to public
/// Nostr relays regardless of this setting (`publish` is hardcoded false and
/// the Nostr relay list stays empty).
const MESH_IROH_RELAYS_ENV: &str = "BUZZ_MESH_IROH_RELAYS";
/// First model load can include a multi-GB download plus Metal warmup; the
/// SDK default (30s) times out long before that. Matches mesh-console.
const MESH_STARTUP_TIMEOUT: Duration = Duration::from_secs(180);
/// Sentinel model id meaning "let the mesh router pick". mesh-llm's OpenAI
/// ingress auto-routes `"model": "auto"` to a context-compatible live target
/// (`resolve_auto_routed_model`), so agents don't have to name a model and
/// can't pick one that doesn't fit their prompt.
pub const AUTO_MODEL_ID: &str = "auto";
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
    pub reason: Option<String>,
    pub models: Vec<MeshModelOption>,
    pub serve_targets: Vec<MeshServeTarget>,
}

impl MeshAvailability {
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
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
    /// Mesh owner ids admitted to this node (the member roster from
    /// member-signed discovery notes). `None` = caller did not resolve a roster
    /// (tests, direct invocations): the node runs without allowlist
    /// enforcement, matching an open relay. `Some` = enforce
    /// `TrustPolicy::Allowlist` over exactly these owners (self is always
    /// included by the caller).
    #[serde(default)]
    pub trusted_owner_ids: Option<Vec<String>>,
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
    /// The request this node was started with. Kept so the coordinator can
    /// detect roster drift (membership changed → trusted owners changed) and
    /// restart the node with the fresh roster — the SDK's trust store is
    /// fixed at node start, so a restart is how roster changes take effect.
    start_request: StartMeshNodeRequest,
}

async fn initialize_mesh_native_runtime() -> anyhow::Result<()> {
    // The dynamic host runtime installs the recommended signed runtime on first
    // use when no compatible version is cached. Keep that SDK-owned path intact
    // so release builds work on clean machines without bundling llama.cpp or
    // requiring a separate `mesh-llm runtime install` command.
    mesh_llm_host_runtime::initialize_host_runtime()
        .await
        .map_err(|error| {
            anyhow::anyhow!("mesh native runtime failed to install or load: {error:#}")
        })
}

/// Tokio worker stack size for the runtime that polls mesh-llm futures.
///
/// mesh-llm's async call chains (model download, node start/join) are deep
/// enough to overflow tokio's default 2 MiB worker stacks — observed as a
/// stack-guard SIGABRT inside `download_model_ref_with_progress_details`
/// when polled on Tauri's stock runtime. Upstream runs its own binary on
/// 8 MiB worker stacks for exactly this reason (mesh-llm `main.rs`,
/// `DEFAULT_WORKER_STACK_SIZE`), as does mesh-console. `lib.rs` installs a
/// runtime with this stack size via `tauri::async_runtime::set` before the
/// app starts, so every command future gets the same headroom.
pub const MESH_WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

/// Pre-download the model (with byte progress through the output sink)
/// before the node starts. Without this the download happens *inside*
/// `serve::start()` where the UI can only show a frozen "starting…" state.
/// Already-installed models return immediately from the cache scan.
async fn ensure_model_downloaded(model: &str) -> anyhow::Result<()> {
    let model_owned = model.to_string();
    let installed = tokio::task::spawn_blocking(move || {
        let cache = mesh_llm_node::models::default_huggingface_cache_dir();
        mesh_llm_node::models::scan_installed_models(cache)
            .iter()
            .any(|m| m.model_ref.contains(&model_owned))
    })
    .await
    .unwrap_or(false);
    if installed {
        return Ok(());
    }
    mesh_llm_host_runtime::models::download_model_ref_with_progress_details(model, true)
        .await
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("downloading {model} failed: {error}"))
}

impl DesktopMeshRuntime {
    pub async fn start(mut request: StartMeshNodeRequest) -> anyhow::Result<Self> {
        sanitize_no_leak_request(&mut request)?;
        initialize_mesh_native_runtime().await?;
        let model_id = request
            .model_id
            .clone()
            .filter(|value| !value.trim().is_empty());
        let model_name = model_id.clone();
        // Serve mode downloads weights before the node starts so byte
        // progress reaches the UI through the output sink; inside
        // serve::start() the download is invisible.
        if request.mode == MeshNodeMode::Serve {
            if let Some(model) = model_id.as_deref() {
                ensure_model_downloaded(model).await?;
            }
        }
        let handle = match request.mode {
            MeshNodeMode::Serve => {
                let model = model_id
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("modelId is required for serve mode"))?;
                let mut builder = serve::EmbeddedServeConfig::builder()
                    .model(model)
                    .api_port(mesh_api_port()?)
                    .console_port(mesh_console_port()?)
                    // No-leak invariants: never publish mesh presence, never
                    // auto-discover other meshes, no public Nostr relays.
                    // Iroh relays are transport-only and enabled by default
                    // (see MESH_IROH_RELAYS_ENV); everything else stays closed.
                    .publish(false)
                    .auto_join(false)
                    .discovery_mode(MeshDiscoveryMode::Nostr)
                    .startup_timeout(MESH_STARTUP_TIMEOUT)
                    .console_ui(true);
                builder = match iroh_relay_mode()? {
                    IrohRelayMode::Disabled => builder.disable_iroh_relays(true),
                    IrohRelayMode::Default => builder.disable_iroh_relays(false),
                    IrohRelayMode::Custom(urls) => builder
                        .disable_iroh_relays(false)
                        .iroh_relays(urls.into_iter().map(|url| url.to_string())),
                };
                if let Some(max_vram_gb) = request.max_vram_gb {
                    builder = builder.max_vram_gb(max_vram_gb as f64);
                }
                if let Some(join_token) = request.join_token.as_deref() {
                    builder = builder.join_token(join_token);
                }
                // Admission: present our owner attestation, and when a member
                // roster was resolved, admit only those owners. Membership in
                // the Buzz relay is the source of the roster; possession of a
                // dial pointer or relay reachability admits nobody.
                let identity = ensure_owner_identity()?;
                builder = builder.owner_key(identity.keystore_path.clone());
                if let Some(owners) = normalized_roster(&request.trusted_owner_ids, &identity) {
                    builder = builder
                        .owner_required(true)
                        .trust_policy(TrustPolicy::Allowlist)
                        .trust_owners(owners);
                }
                serve::start(builder.build()).await?
            }
            MeshNodeMode::Client => {
                let mut builder = client::EmbeddedClientConfig::builder()
                    .api_port(mesh_api_port()?)
                    .console_port(mesh_console_port()?)
                    // Same no-leak invariants as serve mode above.
                    .publish(false)
                    .auto_join(false)
                    .discovery_mode(MeshDiscoveryMode::Nostr)
                    .startup_timeout(MESH_STARTUP_TIMEOUT)
                    .console_ui(true);
                builder = match iroh_relay_mode()? {
                    IrohRelayMode::Disabled => builder.disable_iroh_relays(true),
                    IrohRelayMode::Default => builder.disable_iroh_relays(false),
                    IrohRelayMode::Custom(urls) => builder
                        .disable_iroh_relays(false)
                        .iroh_relays(urls.into_iter().map(|url| url.to_string())),
                };
                if let Some(join_token) = request.join_token.as_deref() {
                    builder = builder.join_token(join_token);
                }
                // Clients always present their owner attestation so allowlist
                // enforcing serve nodes can verify and admit them.
                let identity = ensure_owner_identity()?;
                builder = builder.owner_key(identity.keystore_path.clone());
                if let Some(owners) = normalized_roster(&request.trusted_owner_ids, &identity) {
                    builder = builder
                        .owner_required(true)
                        .trust_policy(TrustPolicy::Allowlist)
                        .trust_owners(owners);
                }
                client::start(builder.build()).await?
            }
        };

        Ok(Self {
            handle,
            mode: request.mode,
            model_id,
            model_name,
            start_request: request,
        })
    }

    /// The request this node was started with (roster drift detection).
    pub fn start_request(&self) -> &StartMeshNodeRequest {
        &self.start_request
    }

    pub async fn status(&self) -> anyhow::Result<MeshNodeStatus> {
        let status = self.handle.status().await?;
        self.status_from_sdk(status)
    }

    pub async fn status_report_payload(&self) -> anyhow::Result<serde_json::Value> {
        let status = self.handle.status().await?;
        let mut payload = status.payload;
        enrich_status_payload_identity(&mut payload, status.invite_token.as_deref());
        if let Ok(identity) = ensure_owner_identity() {
            payload["ownerId"] = serde_json::Value::String(identity.owner_id);
        }
        let models = models_from_status_payload(Some(&payload));
        payload["models"] = serde_json::to_value(&models)?;
        let endpoint_addr = status.invite_token.unwrap_or_default();
        let serve_targets = if self.mode == MeshNodeMode::Serve && !endpoint_addr.is_empty() {
            models
                .into_iter()
                .map(|model| MeshServeTarget {
                    model_id: model.id,
                    model_name: model.name,
                    endpoint_addr: endpoint_addr.clone(),
                    node_name: payload
                        .get("node_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    capacity: None,
                    endpoint_id: endpoint_id_from_status(&payload, Some(&endpoint_addr)),
                    device_id: None,
                    device_name: None,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        payload["serveTargets"] = serde_json::to_value(serve_targets)?;
        Ok(payload)
    }

    pub async fn dial_endpoint_addr(&self, endpoint_addr: impl Into<String>) -> anyhow::Result<()> {
        let endpoint_addr = endpoint_addr.into();
        let validated = validate_advertised_endpoint(&endpoint_addr)?;
        self.handle.join_token(validated.join_token).await
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
        let state = node_state_from_payload(self.mode, &health, &status.payload);
        let endpoint_id = endpoint_id_from_status(&status.payload, status.invite_token.as_deref());
        let device_name = device_name_from_status(&status.payload, endpoint_id.as_deref());
        let device_id = endpoint_id.clone();
        Ok(MeshNodeStatus {
            state,
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

/// Normalize a resolved roster for allowlist enforcement: sorted, deduped,
/// and always containing our own owner id (so a solo sharer can dial their
/// own node and the first member of a fresh relay isn't locked out).
/// `None` in = `None` out (no roster resolved → no enforcement).
fn normalized_roster(
    trusted_owner_ids: &Option<Vec<String>>,
    identity: &identity::OwnerIdentity,
) -> Option<Vec<String>> {
    let ids = trusted_owner_ids.as_ref()?;
    let mut owners: Vec<String> = ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    owners.push(identity.owner_id.clone());
    owners.sort();
    owners.dedup();
    Some(owners)
}

fn sanitize_no_leak_request(request: &mut StartMeshNodeRequest) -> anyhow::Result<()> {
    if let Some(join_token) = request.join_token.as_mut() {
        *join_token = validate_advertised_endpoint(join_token)?.join_token;
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

fn node_state_from_payload(
    mode: MeshNodeMode,
    health: &MeshHealth,
    payload: &serde_json::Value,
) -> MeshNodeState {
    if matches!(health.status, MeshHealthStatus::Failed) {
        return MeshNodeState::Failed;
    }
    if mode == MeshNodeMode::Serve && models_from_status_payload(Some(payload)).is_empty() {
        return MeshNodeState::Starting;
    }
    MeshNodeState::Running
}

pub fn models_from_status_payload(payload: Option<&serde_json::Value>) -> Vec<MeshModelOption> {
    let mut out = Vec::new();
    if let Some(payload) = payload {
        // The SDK's raw status uses `hosted_models` plus ready entries under
        // `runtime.models`. Buzz-authored status reports use `models`. Do not
        // use `serving_models`: MeshLLM fills it with the requested model while
        // the runtime is still in standby, before inference is available.
        for key in ["models", "hosted_models"] {
            if let Some(value) = payload.get(key) {
                collect_model_options(value, &mut out);
            }
        }
        if let Some(runtime_models) = payload
            .get("runtime")
            .and_then(|runtime| runtime.get("models"))
            .and_then(serde_json::Value::as_array)
        {
            for model in runtime_models {
                if model.get("status").and_then(serde_json::Value::as_str) == Some("ready") {
                    collect_model_options(model, &mut out);
                }
            }
        }
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
                .or_else(|| map.get("name"))
                .and_then(serde_json::Value::as_str)
            {
                let name = map
                    .get("display_name")
                    .or_else(|| map.get("displayName"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string);
                push_model(out, id, name);
            } else {
                for child in map.values().filter(|child| {
                    matches!(
                        child,
                        serde_json::Value::Array(_) | serde_json::Value::Object(_)
                    )
                }) {
                    collect_model_options(child, out);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for child in values {
                collect_model_options(child, out);
            }
        }
        serde_json::Value::String(value) => {
            push_model(out, value, None);
        }
        _ => {}
    }
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

/// Canonical model id for equality/dedup. A serving node advertises the same
/// model under two strings — `org/model@main:Q4` (serveTargets[].modelId) and
/// `org/model:Q4` (available_models) — so keying on the raw string leaves BOTH
/// in the picker. Strip the `@main` ref-qualifier (matching the selection-time
/// rule in `pick_serve_target_for_model`) so the two collapse to one entry.
pub(super) fn canonical_model_id(value: &str) -> String {
    value.trim().replace("@main", "")
}

pub(super) fn dedupe_models(models: Vec<MeshModelOption>) -> Vec<MeshModelOption> {
    // Key by canonical id so `@main` / non-`@main` forms of the same model
    // dedup together. Display the canonical (stripped) id so the UI shows one
    // stable label; selection still matches because the picker canonicalizes
    // both sides too.
    let mut by_id = BTreeMap::<String, Option<String>>::new();
    for model in models {
        by_id
            .entry(canonical_model_id(&model.id))
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
