use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};

use super::mesh_readiness::wait_for_mesh_inference;
use crate::{app_state::AppState, mesh_llm, relay};

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MeshSharingConfig {
    enabled: bool,
    /// A fresh Share Compute request that must cross a process boundary before
    /// it can start. Consumed before startup so an interrupted download is not
    /// resumed on a later launch.
    #[serde(default)]
    start_on_next_launch: bool,
    model_id: String,
    max_vram_gb: Option<u64>,
    /// Community relay where Share Compute was explicitly enabled. Older
    /// configs predate community binding and restore against the active relay.
    #[serde(default)]
    relay_url: Option<String>,
}

fn pending_new_start_checkpoint(config: &MeshSharingConfig) -> MeshSharingConfig {
    let mut checkpoint = config.clone();
    checkpoint.enabled = false;
    checkpoint.start_on_next_launch = false;
    checkpoint
}

fn one_shot_restart_checkpoint(config: &MeshSharingConfig) -> MeshSharingConfig {
    let mut checkpoint = config.clone();
    checkpoint.enabled = false;
    checkpoint.start_on_next_launch = true;
    checkpoint
}

fn mesh_sharing_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?
        .join("mesh-sharing.json"))
}

fn save_mesh_sharing_config(app: &AppHandle, config: &MeshSharingConfig) -> Result<(), String> {
    let path = mesh_sharing_config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create mesh config directory: {error}"))?;
    }
    let payload = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("failed to encode mesh sharing config: {error}"))?;
    crate::managed_agents::atomic_write_json(&path, &payload)
}

fn load_mesh_sharing_config(app: &AppHandle) -> Result<Option<MeshSharingConfig>, String> {
    let path = mesh_sharing_config_path(app)?;
    match std::fs::read(&path) {
        Ok(payload) => serde_json::from_slice(&payload)
            .map(Some)
            .map_err(|error| format!("failed to parse {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("failed to read {}: {error}", path.display())),
    }
}

const RELAY_MESH_RUNTIME_NO_TARGET: &str =
    "Buzz shared compute requires a live serving member; start serving the selected model on a member, then try again";

/// Whether the Share-compute "stop sharing" path (`mesh_stop_node`) should tear
/// down the runtime currently occupying the single slot.
///
/// Serve nodes (this machine SHARING compute) are torn down. Client nodes (this
/// machine CONSUMING a peer's compute) share the same slot and MUST be left
/// running — stopping "Share compute" must never kill a consume session the
/// user didn't start from this switch.
#[cfg(feature = "mesh-llm")]
fn share_stop_should_teardown(mode: mesh_llm::MeshNodeMode) -> bool {
    matches!(mode, mesh_llm::MeshNodeMode::Serve)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshStartPlan {
    Start,
    RestartToReplaceClient,
    RejectOccupied,
}

fn mesh_start_plan(
    requested_mode: mesh_llm::MeshNodeMode,
    existing_mode: Option<mesh_llm::MeshNodeMode>,
) -> MeshStartPlan {
    match (requested_mode, existing_mode) {
        (_, None) => MeshStartPlan::Start,
        (mesh_llm::MeshNodeMode::Serve, Some(mesh_llm::MeshNodeMode::Client)) => {
            MeshStartPlan::RestartToReplaceClient
        }
        _ => MeshStartPlan::RejectOccupied,
    }
}

fn sharing_config_from_request(
    request: &mesh_llm::StartMeshNodeRequest,
) -> CmdResult<MeshSharingConfig> {
    let model_id = request
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model_id| !model_id.is_empty())
        .ok_or_else(|| "modelId is required for serve mode".to_string())?;
    Ok(MeshSharingConfig {
        enabled: true,
        start_on_next_launch: false,
        model_id: model_id.to_string(),
        max_vram_gb: request.max_vram_gb,
        relay_url: request.relay_url.clone(),
    })
}

fn restarting_share_status(config: &MeshSharingConfig) -> mesh_llm::MeshNodeStatus {
    mesh_llm::MeshNodeStatus {
        state: mesh_llm::MeshNodeState::Starting,
        mode: Some(mesh_llm::MeshNodeMode::Serve),
        health: mesh_llm::MeshHealth {
            status: mesh_llm::MeshHealthStatus::Degraded,
            reason: Some("Buzz is restarting to switch this machine to sharing".to_string()),
        },
        api_base_url: None,
        console_url: None,
        model_id: Some(config.model_id.clone()),
        model_name: Some(config.model_id.clone()),
        invite_token: None,
        endpoint_id: None,
        device_id: None,
        device_name: None,
    }
}

fn restart_to_share(
    app: &AppHandle,
    config: &MeshSharingConfig,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    save_mesh_sharing_config(app, &one_shot_restart_checkpoint(config))?;
    let status = restarting_share_status(config);
    app.request_restart();
    Ok(status)
}

pub type CmdResult<T> = Result<T, String>;

fn buzz_mesh_name_for_relay(relay_url: &str) -> String {
    let normalized = url::Url::parse(relay_url.trim())
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| relay_url.trim().trim_end_matches('/').to_ascii_lowercase());
    let digest = hex::encode(Sha256::digest(normalized.as_bytes()));
    format!("buzz-community-{}", &digest[..32])
}

pub(super) fn buzz_mesh_name(state: &AppState) -> String {
    buzz_mesh_name_for_relay(&relay::relay_ws_url_with_override(state))
}

fn advance_mesh_status_cursor(
    filter: &mut serde_json::Value,
    page: &[nostr::Event],
) -> Result<(u64, String), String> {
    let last = page
        .last()
        .ok_or_else(|| "cannot advance an empty mesh status page".to_string())?;
    let cursor = (last.created_at.as_secs(), last.id.to_hex());
    filter["until"] = serde_json::json!(cursor.0);
    filter["before_id"] = serde_json::json!(cursor.1);
    Ok(cursor)
}

async fn query_mesh_discovery_events_at(
    state: &AppState,
    relay_url: &str,
) -> Result<Vec<nostr::Event>, String> {
    let api_base_url = relay::relay_http_base_url(relay_url);
    let mut events =
        relay::query_relay_at(state, &api_base_url, &[mesh_llm::relay_membership_filter()]).await?;
    let member_pubkeys = mesh_llm::current_member_pubkeys(&events);
    if member_pubkeys.is_empty() {
        // Distinguish "relay returned a membership snapshot listing zero
        // members" (authoritative empty — allowed to shrink the roster to
        // self-only) from "no membership snapshot came back at all" (a
        // transient gap / replication lag). The relay publishes an explicit
        // kind:13534 event even for a zero-member community, so its absence
        // means the query is incomplete: surface it as an error so the
        // reconcile loop keeps the current allowlist instead of flapping the
        // node down to self-only on a successful-but-empty response.
        if !mesh_llm::has_membership_snapshot(&events) {
            return Err("relay returned no membership snapshot".to_string());
        }
        return Ok(events);
    }
    let mut status_filter = mesh_llm::mesh_status_filter();
    status_filter["authors"] = serde_json::json!(member_pubkeys);
    let mut previous_cursor: Option<(u64, String)> = None;

    loop {
        let page = relay::query_relay_at(state, &api_base_url, &[status_filter.clone()]).await?;
        let done = page.len() < mesh_llm::MESH_STATUS_PAGE_SIZE;
        if !done {
            let cursor = advance_mesh_status_cursor(&mut status_filter, &page)?;
            if previous_cursor.as_ref() == Some(&cursor) {
                return Err("mesh status pagination did not advance".to_string());
            }
            previous_cursor = Some(cursor);
        }
        events.extend(page);
        if done {
            return Ok(events);
        }
    }
}

async fn query_mesh_discovery_events(state: &AppState) -> Result<Vec<nostr::Event>, String> {
    query_mesh_discovery_events_at(state, &relay::relay_ws_url_with_override(state)).await
}

/// Resolve the admission roster by intersecting member-signed mesh status
/// reporters with the current NIP-43 direct-member list.
///
/// Returns `Err` when the relay query fails. Callers MUST distinguish this from
/// an `Ok(empty)` roster (a genuinely empty community): a failed query must
/// never be collapsed into "self-only", or a transient relay blip de-admits
/// every other member. `reconcile_roster` relies on this to keep the current
/// allowlist on error instead of restarting the node down to self-only.
pub(crate) async fn resolve_trusted_owner_ids(state: &AppState) -> Result<Vec<String>, String> {
    let events = query_mesh_discovery_events(state).await?;
    Ok(mesh_llm::owner_ids_from_events(&events))
}

pub(crate) async fn resolve_trusted_owner_ids_at(
    state: &AppState,
    relay_url: &str,
) -> Result<Vec<String>, String> {
    let events = query_mesh_discovery_events_at(state, relay_url).await?;
    Ok(mesh_llm::owner_ids_from_events(&events))
}

/// Resolve the roster for an initial node *start*, failing closed to self-only
/// (an empty roster) when the relay query fails. This is safe only at start:
/// there is no established allowlist to preserve yet. The periodic
/// `reconcile_roster` path must NOT use this — it has a live roster to keep.
pub(crate) async fn resolve_trusted_owner_ids_or_self_only(state: &AppState) -> Vec<String> {
    match resolve_trusted_owner_ids(state).await {
        Ok(owners) => owners,
        Err(error) => {
            eprintln!("buzz-mesh: roster query failed; allowing only this node: {error}");
            Vec::new()
        }
    }
}

/// Choose validated live endpoints from other runtimes in this Buzz community.
/// The stable relay-derived mesh name gives every runtime the same MeshLLM mesh
/// identity; these endpoints supply transport bootstrap only.
fn buzz_mesh_join_targets(
    mut targets: Vec<mesh_llm::MeshServeTarget>,
    self_owner_id: &str,
) -> Vec<mesh_llm::MeshServeTarget> {
    targets.retain(|target| {
        target.reporter_pubkey.is_some()
            && target
                .owner_id
                .as_deref()
                .is_some_and(|owner| !owner.eq_ignore_ascii_case(self_owner_id.trim()))
    });
    targets.sort_by(|left, right| {
        left.reporter_pubkey
            .cmp(&right.reporter_pubkey)
            .then_with(|| left.owner_id.cmp(&right.owner_id))
            .then_with(|| left.endpoint_id.cmp(&right.endpoint_id))
            .then_with(|| left.endpoint_addr.cmp(&right.endpoint_addr))
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
    targets.dedup_by(|left, right| left.endpoint_addr == right.endpoint_addr);
    targets
}

/// Resolve the validated member endpoint this runtime should join to enter the
/// existing Buzz community mesh. `Ok(None)` means this machine is the first
/// live serving member (or is itself the shared bootstrap contact).
pub(crate) async fn resolve_buzz_mesh_join_targets_at(
    state: &AppState,
    relay_url: &str,
) -> Result<Vec<mesh_llm::MeshServeTarget>, String> {
    let events = query_mesh_discovery_events_at(state, relay_url).await?;
    let self_owner_id = mesh_llm::ensure_owner_identity()
        .map_err(|error| format!("failed to load mesh owner identity: {error}"))?
        .owner_id;
    Ok(buzz_mesh_join_targets(
        mesh_llm::availability_from_events(events).serve_targets,
        &self_owner_id,
    ))
}

/// Resolve the initial admission roster and bootstrap endpoint from one relay
/// snapshot. A node start used to repeat the full membership + status query
/// for each value, making Share Compute startup both slower and more exposed
/// to inconsistent snapshots.
async fn resolve_buzz_mesh_startup_at(
    state: &AppState,
    relay_url: &str,
) -> (Vec<String>, Option<String>) {
    match query_mesh_discovery_events_at(state, relay_url).await {
        Ok(events) => {
            let trusted_owner_ids = mesh_llm::owner_ids_from_events(&events);
            let join_token = mesh_llm::ensure_owner_identity()
                .ok()
                .and_then(|identity| {
                    buzz_mesh_join_targets(
                        mesh_llm::availability_from_events(events).serve_targets,
                        &identity.owner_id,
                    )
                    .into_iter()
                    .next()
                })
                .map(|target| target.endpoint_addr);
            (trusted_owner_ids, join_token)
        }
        Err(error) => {
            // Initial startup fails closed to this runtime's own owner. Share
            // Compute must still start for the first member and through a
            // transient relay outage; the coordinator retries convergence.
            eprintln!(
                "buzz-mesh: startup discovery failed; allowing only this node and starting isolated for now: {error}"
            );
            (Vec::new(), None)
        }
    }
}

pub(crate) async fn restore_mesh_sharing(app: &AppHandle, state: &AppState) -> CmdResult<()> {
    let Some(mut config) = load_mesh_sharing_config(app)? else {
        return Ok(());
    };
    if (!config.enabled && !config.start_on_next_launch) || config.model_id.trim().is_empty() {
        return Ok(());
    }
    config.model_id = mesh_llm::canonical_curated_model_id(&config.model_id).to_string();
    if state.mesh_llm_runtime.lock().await.is_some() {
        return Ok(());
    }
    let relay_url = config
        .relay_url
        .clone()
        .unwrap_or_else(|| relay::relay_ws_url_with_override(state));
    let (trusted_owner_ids, join_token) = resolve_buzz_mesh_startup_at(state, &relay_url).await;
    let mut runtime = state.mesh_llm_runtime.lock().await;
    if runtime.is_some() {
        return Ok(());
    }
    if config.start_on_next_launch {
        // Consume a role-switch request before doing any potentially long model
        // work. If Buzz exits during that work, the next launch stays stopped.
        config = pending_new_start_checkpoint(&config);
        save_mesh_sharing_config(app, &config)?;
    }
    // This is restoration of a previously inference-ready serving node. Keep
    // the enabled checkpoint armed while restoring so a transient startup
    // failure does not silently turn Share Compute off.
    let request = mesh_llm::StartMeshNodeRequest {
        mode: mesh_llm::MeshNodeMode::Serve,
        model_id: Some(config.model_id.clone()),
        max_vram_gb: config.max_vram_gb,
        join_token,
        mesh_name: Some(buzz_mesh_name_for_relay(&relay_url)),
        relay_url: Some(relay_url),
        trusted_owner_ids: Some(trusted_owner_ids),
    };
    let started = mesh_llm::DesktopMeshRuntime::start(request)
        .await
        .map_err(|error| format!("failed to restore Share Compute: {error:#}"))?;
    // Install the restored runtime immediately: it is tracked by AppState from
    // here on, so it can never be orphaned. Restoring a previously
    // inference-ready node still has to load ~tens of GB of weights and may
    // download package layers after the ports bind, and the readiness probe
    // itself serializes behind any first inference. None of that is a failed
    // restore — stopping the node and reporting failure (the old behaviour)
    // tore down a node that was simply still warming up. The checkpoint stays
    // armed (`enabled`), so a genuinely broken restore is retried next launch
    // rather than silently turning Share Compute off.
    *runtime = Some(started);
    config.enabled = true;
    config.start_on_next_launch = false;
    save_mesh_sharing_config(app, &config)?;
    drop(runtime);
    if let Err(error) = wait_for_mesh_inference(&config.model_id).await {
        eprintln!(
            "buzz-mesh: restored node is not inference-ready yet ({error}); \
             leaving it to warm up without tearing it down"
        );
    }
    mesh_llm::publish_current_status_once(app, "restore").await;
    Ok(())
}

#[tauri::command]
pub async fn mesh_start_node(
    app: AppHandle,
    state: State<'_, AppState>,
    mut request: mesh_llm::StartMeshNodeRequest,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    let relay_url = relay::relay_ws_url_with_override(&state);
    request.relay_url = Some(relay_url.clone());
    if let Some(model_id) = request.model_id.as_mut() {
        *model_id = mesh_llm::canonical_curated_model_id(model_id).to_string();
    }
    let sharing_config = if request.mode == mesh_llm::MeshNodeMode::Serve {
        Some(sharing_config_from_request(&request)?)
    } else {
        None
    };

    // Never replace a client runtime in-process. Even a Ready SDK handle can
    // finish `stop()` while its native listeners are still releasing :9337
    // and :3131; a pending client has no shutdown handle at all. Persist the
    // requested serving configuration and switch roles across a controlled
    // process restart, the only boundary that proves both ports are clean.
    {
        let runtime = state.mesh_llm_runtime.lock().await;
        if let Some(existing) = runtime.as_ref() {
            let plan = mesh_start_plan(request.mode, Some(existing.mode()));
            match plan {
                MeshStartPlan::RestartToReplaceClient => {
                    let config = sharing_config
                        .as_ref()
                        .ok_or_else(|| "serving configuration is unavailable".to_string())?;
                    drop(runtime);
                    return restart_to_share(&app, config);
                }
                MeshStartPlan::RejectOccupied => {
                    return Err("mesh node is already running".to_string());
                }
                MeshStartPlan::Start => {}
            }
        }
    }

    // Frontend requests never carry a roster. Resolve it and the bootstrap
    // endpoint from one snapshot so UI startup does not repeat relay probes.
    if request.trusted_owner_ids.is_none() || request.join_token.is_none() {
        let (trusted_owner_ids, join_token) =
            resolve_buzz_mesh_startup_at(&state, &relay_url).await;
        request.trusted_owner_ids.get_or_insert(trusted_owner_ids);
        if request.join_token.is_none() {
            request.join_token = join_token;
        }
    }
    request.mesh_name = Some(buzz_mesh_name_for_relay(&relay_url));
    let mut runtime = state.mesh_llm_runtime.lock().await;

    let plan = match runtime.as_ref() {
        Some(existing) => mesh_start_plan(request.mode, Some(existing.mode())),
        None => mesh_start_plan(request.mode, None),
    };
    if plan == MeshStartPlan::RestartToReplaceClient {
        let config = sharing_config
            .as_ref()
            .ok_or_else(|| "serving configuration is unavailable".to_string())?;
        drop(runtime);
        return restart_to_share(&app, config);
    }
    if plan == MeshStartPlan::RejectOccupied {
        return Err("mesh node is already running".to_string());
    }

    if let Some(config) = sharing_config.as_ref() {
        // Persist a DISARMED checkpoint to cover the window of the potentially
        // long `start()` below: if Buzz exits before the runtime is installed
        // and tracked, the next launch stays stopped rather than trying to
        // restore a node that never came up. The enabled config is armed right
        // after install succeeds.
        save_mesh_sharing_config(&app, &pending_new_start_checkpoint(config))?;
    }

    let started = mesh_llm::DesktopMeshRuntime::start(request)
        .await
        .map_err(|error| format!("{error:#}"))?;
    let status = match started.status().await {
        Ok(status) => status,
        Err(error) => {
            let cleanup = started.stop().await;
            if let Err(cleanup_error) = &cleanup {
                eprintln!(
                    "buzz-mesh: started node status failed and cleanup was incomplete: {cleanup_error:#}"
                );
            }
            // The handle was never installed into AppState, so shutdown cannot
            // see it again. Restart even when stop reported success: the
            // process boundary guarantees native :9337/:3131 listeners cannot
            // linger behind an untracked runtime.
            drop(runtime);
            app.request_restart();
            return Err(format!(
                "mesh node started but status probe failed: {error:#}; Buzz is restarting to guarantee cleanup"
            ));
        }
    };
    // Install (track) the runtime BEFORE probing readiness so it can never be
    // orphaned. A readiness timeout is not death: mesh binds its ports before
    // weights finish loading / layers finish downloading, and serializes all
    // ingress HTTP (this probe included) behind any in-flight turn — a cold
    // start can take minutes. The old code stopped the node and restarted the
    // app on that timeout, turning startup latency into a restart loop.
    *runtime = Some(started);
    drop(runtime);
    if let Some(config) = sharing_config.as_ref() {
        // Installed + tracked == Share Compute is on, so persist the enabled
        // config now (mirroring restore), not gated on the probe. Gating it
        // meant a slow first start served fine but came back OFF next launch.
        // Safe: neither the watchdog (evicts only a closed port) nor restore
        // (leaves a warming node alone) can loop a slow-but-alive node, and an
        // unstartable config fails earlier in `start()`. Probe is informational.
        save_mesh_sharing_config(&app, config)?;
        if let Err(error) = wait_for_mesh_inference(&config.model_id).await {
            eprintln!(
                "buzz-mesh: node started but inference is not ready yet ({error}); \
                 leaving it to warm up (Share Compute stays armed for next launch)"
            );
        }
    }
    mesh_llm::publish_current_status_once(&app, "start").await;
    Ok(status)
}

pub(crate) async fn ensure_client_node_for_model(
    state: &AppState,
    model_id: impl AsRef<str>,
    endpoint_addr: Option<String>,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    let requested_model = model_id.as_ref().trim();
    if requested_model.is_empty() {
        return Err("modelId is required".to_string());
    }

    {
        let runtime = state.mesh_llm_runtime.lock().await;
        if let Some(runtime) = runtime.as_ref() {
            // A running runtime — in any mode — is the mesh's local OpenAI
            // ingress on `9337`. mesh-llm's router already resolves the
            // requested model to a local, remote, or split target at request
            // time (see `route_missing_local_model` -> `hosts_for_model`), so
            // "serving" and "using the mesh as a client" are not mutually
            // exclusive: a serve node can host model A and route model B to a
            // peer through the same ingress. Hand the agent the existing
            // runtime; the router decides routability per request rather than
            // this preflight second-guessing it (a `/v1/models` check here
            // would race model gossip and wrongly reject freshly-discovered
            // remote/split models).
            //
            // If the caller selected a specific target, still dial it: that is
            // how the runtime joins the chosen peer's mesh. Skipping it would
            // let a serve runtime not yet connected to that target fail its
            // first inference while the frontend has already signalled the
            // peer to expect us.
            if let Some(endpoint_addr) = endpoint_addr
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                runtime
                    .dial_endpoint_addr(endpoint_addr)
                    .await
                    .map_err(|error| format!("mesh dial failed: {error}"))?;
            }
            return runtime.status().await.map_err(|error| error.to_string());
        }
    }

    let join_token = match endpoint_addr
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => value,
        None => return Err(RELAY_MESH_RUNTIME_NO_TARGET.to_string()),
    };

    let start = mesh_llm::StartMeshNodeRequest {
        mode: mesh_llm::MeshNodeMode::Client,
        model_id: None,
        max_vram_gb: None,
        join_token: Some(join_token.clone()),
        mesh_name: Some(buzz_mesh_name(state)),
        relay_url: Some(relay::relay_ws_url_with_override(state)),
        trusted_owner_ids: Some(resolve_trusted_owner_ids_or_self_only(state).await),
    };
    let mut runtime = state.mesh_llm_runtime.lock().await;
    if let Some(existing) = runtime.as_ref() {
        // Another GUI agent may have won the startup race while this caller
        // was resolving membership. The runtime is machine-scoped, not
        // agent-scoped: join its selected endpoint into the existing node and
        // let every caller reuse the same local ingress.
        existing
            .dial_endpoint_addr(join_token)
            .await
            .map_err(|error| format!("mesh dial failed: {error}"))?;
        return existing.status().await.map_err(|error| error.to_string());
    }
    let started = mesh_llm::DesktopMeshRuntime::start(start)
        .await
        .map_err(|error| format!("mesh client failed to start: {error:#}"))?;
    let status = started
        .status()
        .await
        .map_err(|error| format!("mesh client started but status probe failed: {error:#}"))?;
    *runtime = Some(started);
    Ok(status)
}

/// Re-resolve a live serve target's dial pointer for a saved relay-mesh agent.
///
/// The serve target's `endpoint_addr` is live discovery state — it comes from
/// the peer's client-signed mesh status event and rotates when the peer's
/// iroh endpoint changes — so it is never persisted onto the agent record.
/// Instead, a saved agent re-resolves a current bootstrap target at start time
/// by matching its configured model against the targets the relay is gossiping
/// right now. We only need *any* live target for the model to bootstrap the
/// client node; mesh-llm's router picks the per-request host afterwards.
///
/// `Err` means the relay query itself failed (relay down, auth, network) — we
/// could not refresh targets at all and must not pretend the peer is offline.
/// `Ok(None)` means the relay answered but no live target currently serves this
/// model (genuine peer-offline). `Ok(Some(addr))` is a dialable bootstrap
/// target.
pub(crate) async fn resolve_mesh_bootstrap_target(
    state: &AppState,
    model_id: &str,
) -> Result<Option<mesh_llm::MeshServeTarget>, String> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Ok(None);
    }
    let events = query_mesh_discovery_events(state).await?;
    Ok(pick_serve_target_for_model(
        mesh_llm::availability_from_events(events).serve_targets,
        model_id,
    ))
}

/// Pure target-selection used by `resolve_mesh_bootstrap_target`: the first
/// gossiped serve target that hosts `model_id`. Split out so the matching rule
/// is unit-testable without a relay round-trip.
fn pick_serve_target_for_model(
    targets: Vec<mesh_llm::MeshServeTarget>,
    model_id: &str,
) -> Option<mesh_llm::MeshServeTarget> {
    // "auto" delegates model choice to the mesh router (mesh-llm's
    // auto-route path): any live serve target is a valid bootstrap peer.
    if model_id == mesh_llm::AUTO_MODEL_ID {
        return targets.into_iter().next();
    }
    fn canonical_model_id(value: &str) -> String {
        value.trim().replace("@main", "")
    }
    let requested = canonical_model_id(model_id);
    targets
        .into_iter()
        .find(|target| canonical_model_id(&target.model_id) == requested)
}

/// Decide whether a relay-mesh agent may start, and bring up its local mesh
/// client when needed.
///
/// Every start follows the same backend-owned path. If a local runtime exists,
/// wait until its inference router is actually ready. Otherwise re-resolve a
/// current bootstrap target from the members' client-signed discovery notes,
/// then bring up the local MeshLLM client. The endpoint contains MeshLLM's
/// encrypted iroh relay addresses, so no Buzz relay connection coordination is
/// required. The two failure modes get distinct, actionable copy:
/// a relay query failure ("could not refresh targets") is not the same as a
/// relay that answered with no live target for this model ("peer offline").
/// Non relay-mesh records are a no-op.
pub(crate) async fn ensure_relay_mesh_for_record(
    app: &AppHandle,
    model_id: Option<&str>,
    _allow_fresh_create_start: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(model_id) = model_id else {
        return Ok(());
    };
    // A local serve/client runtime already owns the OpenAI ingress and its
    // router can resolve both `auto` and explicit remote models. Do not require
    // a separate relay-advertised target in that case — BUT only trust it when
    // the ingress is actually alive. A runtime that exited/wedged after launch
    // leaves `mesh_llm_runtime = Some` pointing at a dead `:9337` ingress, so a
    // blind `wait_for_mesh_inference` would just time out and the agent would
    // stay silent (#2062). Probe first; if the ingress is dead, drop the stale
    // runtime and fall through to re-arm it. The mesh coordinator watchdog also
    // calls this path after eviction so recovery is not start-only (Brad #2304).
    if state.mesh_llm_runtime.lock().await.is_some() {
        match mesh_llm::recover_stale_mesh_runtime(
            &state,
            mesh_llm::MeshRecoveryUrgency::Foreground,
        )
        .await
        {
            mesh_llm::MeshRuntimeRecovery::Live => {
                return wait_for_mesh_inference(model_id).await;
            }
            mesh_llm::MeshRuntimeRecovery::Evicted | mesh_llm::MeshRuntimeRecovery::Absent => {}
            mesh_llm::MeshRuntimeRecovery::Debouncing => {
                return Err(
                    "Buzz shared compute ingress is temporarily unresponsive; recovery is already scheduled. Try again shortly."
                        .to_string(),
                );
            }
            mesh_llm::MeshRuntimeRecovery::ReleasePending => {
                return Err(
                    "Buzz shared compute is still shutting down its previous local ingress. Try again shortly."
                        .to_string(),
                );
            }
            mesh_llm::MeshRuntimeRecovery::Replaced => {
                return wait_for_mesh_inference(model_id).await;
            }
            mesh_llm::MeshRuntimeRecovery::RestartRequired => {
                app.request_restart();
                return Err(
                    "Buzz shared compute startup lost its local ingress before shutdown control became available. Buzz is restarting to recover it."
                        .to_string(),
                );
            }
        }
    }

    // A persisted Share Compute configuration is authoritative about this
    // machine's role. If no runtime is currently tracked (for example after a
    // clean process restart), restore the serving node instead of treating an
    // agent request as permission to replace it with a client node.
    if load_mesh_sharing_config(app)?
        .is_some_and(|config| config.enabled && !config.model_id.trim().is_empty())
    {
        restore_mesh_sharing(app, &state).await?;
        return wait_for_mesh_inference(model_id).await;
    }

    let target = match resolve_mesh_bootstrap_target(&state, model_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return Err(
                "Buzz shared compute cannot start because no live member is serving this model. Start serving it on a member, then try again."
                    .to_string(),
            );
        }
        Err(error) => {
            return Err(format!(
                "could not refresh Buzz shared compute serving members: {error}"
            ));
        }
    };

    // No serving configuration exists, so this is a genuine consumer-only
    // start. A configured serving machine is restored above and never reaches
    // this client fallback.
    ensure_client_node_for_model(&state, model_id, Some(target.endpoint_addr)).await?;
    wait_for_mesh_inference(model_id).await
}

#[tauri::command]
pub async fn mesh_stop_node(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    // The single runtime slot is shared by serve (this machine SHARING
    // compute) and client (this machine CONSUMING a peer's compute) roles.
    // Stopping "Share compute" must NEVER tear down a client node: inspect the
    // role under the lock and, when it's a consume session, leave it running
    // and return its live status unchanged. The frontend also guards this, but
    // status can be stale between polls, so the backend is authoritative.
    let (taken, bound_relay_url) = {
        let mut guard = state.mesh_llm_runtime.lock().await;
        if let Some(runtime) = guard.as_ref() {
            if !share_stop_should_teardown(runtime.mode()) {
                return runtime.status().await.map_err(|error| error.to_string());
            }
        }
        let bound_relay_url = guard
            .as_ref()
            .and_then(|runtime| runtime.start_request().relay_url.clone());
        (guard.take(), bound_relay_url)
    };
    if let Some(runtime) = taken {
        runtime.stop().await.map_err(|error| error.to_string())?;
    }
    save_mesh_sharing_config(
        &app,
        &MeshSharingConfig {
            enabled: false,
            start_on_next_launch: false,
            model_id: String::new(),
            max_vram_gb: None,
            relay_url: None,
        },
    )?;
    mesh_llm::publish_stopped_status_once_at(&app, bound_relay_url.as_deref(), "stop").await;
    Ok(mesh_llm::stopped_status())
}

#[tauri::command]
pub async fn mesh_node_status(state: State<'_, AppState>) -> CmdResult<mesh_llm::MeshNodeStatus> {
    let runtime = state.mesh_llm_runtime.lock().await;
    match runtime.as_ref() {
        Some(runtime) => runtime.status().await.map_err(|error| error.to_string()),
        None => Ok(mesh_llm::stopped_status()),
    }
}

/// Read-only host-side usage: who/what is using the compute this machine is
/// sharing. Returns a zeroed snapshot when no runtime is active. No new trust
/// surface — it reads the serving node's own runtime metrics.
#[tauri::command]
pub async fn mesh_serving_usage(
    state: State<'_, AppState>,
) -> CmdResult<mesh_llm::MeshServingUsage> {
    let runtime = state.mesh_llm_runtime.lock().await;
    match runtime.as_ref() {
        Some(runtime) => runtime.serving_usage().await.map_err(|e| e.to_string()),
        None => Ok(mesh_llm::MeshServingUsage::default()),
    }
}

#[tauri::command]
pub async fn mesh_installed_models(
    state: State<'_, AppState>,
) -> CmdResult<Vec<mesh_llm::MeshModelOption>> {
    let runtime = state.mesh_llm_runtime.lock().await;
    if let Some(runtime) = runtime.as_ref() {
        return runtime
            .installed_models()
            .await
            .map_err(|error| error.to_string());
    }
    Ok(Vec::new())
}

/// Hardware-aware curated model catalog for the Share-compute picker: the
/// machine's AI memory, a recommended best fit, and every catalog model
/// ranked by fit with installed-state flags. Runs the hardware survey +
/// HF-cache scan off the async runtime (both do blocking I/O).
#[tauri::command]
pub async fn mesh_model_catalog() -> CmdResult<mesh_llm::MeshModelCatalog> {
    tokio::task::spawn_blocking(mesh_llm::model_catalog)
        .await
        .map_err(|error| format!("mesh catalog task failed: {error}"))
}

#[cfg(all(test, feature = "mesh-llm"))]
#[path = "mesh_llm_tests.rs"]
mod tests;
