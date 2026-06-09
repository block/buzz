use tauri::{AppHandle, Manager, State};

use crate::{app_state::AppState, managed_agents::RELAY_MESH_API_BASE_URL, mesh_llm, relay};

const RELAY_MESH_RUNTIME_NO_TARGET: &str =
    "relay mesh client start requires a concrete serve target; reopen the agent with Run on relay mesh selected to refresh its target";

pub type CmdResult<T> = Result<T, String>;

#[tauri::command]
pub async fn mesh_availability(
    state: State<'_, AppState>,
) -> CmdResult<mesh_llm::MeshAvailability> {
    match relay::query_relay(&state, &[mesh_llm::mesh_status_filter()]).await {
        Ok(events) => Ok(mesh_llm::availability_from_events(events)),
        Err(error) => Ok(mesh_llm::MeshAvailability::unavailable(error)),
    }
}

#[tauri::command]
pub async fn mesh_start_node(
    app: AppHandle,
    state: State<'_, AppState>,
    request: mesh_llm::StartMeshNodeRequest,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    let mut runtime = state.mesh_llm_runtime.lock().await;
    if runtime.is_some() {
        return Err("mesh node is already running".to_string());
    }

    let started = mesh_llm::DesktopMeshRuntime::start(request)
        .await
        .map_err(|error| error.to_string())?;
    let status = started
        .status()
        .await
        .map_err(|error| format!("mesh node started but status probe failed: {error}"))?;
    *runtime = Some(started);
    drop(runtime);
    mesh_llm::publish_current_status_once(&app, "start").await;
    Ok(status)
}

#[tauri::command]
pub async fn mesh_ensure_client_node(
    state: State<'_, AppState>,
    request: mesh_llm::EnsureMeshClientRequest,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    ensure_client_node_for_model(&state, request.model_id, request.endpoint_addr).await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareRelayMeshClientRequest {
    pub model_id: String,
    pub target: mesh_llm::MeshServeTarget,
}

/// Fresh-create preflight for relay-mesh agents. Starts/dials the local mesh
/// client and sends the paired connect-request through the Rust coordinator so
/// fresh-created and saved relay-mesh agents use the same signaling path.
#[tauri::command]
pub async fn mesh_prepare_relay_mesh_client(
    app: AppHandle,
    state: State<'_, AppState>,
    request: PrepareRelayMeshClientRequest,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    prepare_relay_mesh_client(&app, &state, &request.model_id, request.target).await
}

pub(crate) async fn prepare_relay_mesh_client(
    app: &AppHandle,
    state: &AppState,
    model_id: &str,
    target: mesh_llm::MeshServeTarget,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    let target_pubkey = normalize_pubkey(target.reporter_pubkey.as_deref())
        .ok_or_else(|| "Selected relay mesh target is missing its reporter pubkey.".to_string())?;
    let status =
        ensure_client_node_for_model(state, model_id, Some(target.endpoint_addr.clone())).await?;
    let self_pubkey = workspace_pubkey(state)?;
    if self_pubkey == target_pubkey {
        return Ok(status);
    }
    let self_addr = status
        .invite_token
        .as_deref()
        .ok_or_else(|| "Local mesh client did not publish an endpoint address.".to_string())?;
    crate::mesh_llm::start_client(
        app,
        crate::mesh_llm::RelayMeshConnectRequest {
            target_pubkey: &target_pubkey,
            peer_endpoint_addr: &target.endpoint_addr,
            self_endpoint_addr: self_addr,
            peer_endpoint_id: target.endpoint_id.as_deref(),
            self_endpoint_id: status.endpoint_id.as_deref(),
        },
    )
    .await?;
    Ok(status)
}

fn normalize_pubkey(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(normalized)
    } else {
        None
    }
}

fn workspace_pubkey(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

/// Join a peer by endpoint addr without naming a model. Used by the runtime
/// coordinator's call-me-now responder and the initiator's same-attempt dial:
/// the responder side of a hole-punch just needs both ends dialing, and the
/// mesh-llm router resolves per-model routability per request afterward.
///
/// Dials into the running runtime if one exists; otherwise starts a client
/// node with the addr as its join token. Model-agnostic on purpose.
pub(crate) async fn ensure_client_node_for_model_dial_only(
    state: &AppState,
    endpoint_addr: &str,
) -> CmdResult<()> {
    let addr = endpoint_addr.trim();
    if addr.is_empty() {
        return Err("endpoint_addr is required to dial".to_string());
    }
    {
        let runtime = state.mesh_llm_runtime.lock().await;
        if let Some(runtime) = runtime.as_ref() {
            return runtime
                .dial_endpoint_addr(addr)
                .await
                .map_err(|error| format!("mesh dial failed: {error}"));
        }
    }
    let start = mesh_llm::StartMeshNodeRequest {
        mode: mesh_llm::MeshNodeMode::Client,
        model_id: None,
        max_vram_gb: None,
        join_token: Some(addr.to_string()),
    };
    let mut runtime = state.mesh_llm_runtime.lock().await;
    if runtime.is_some() {
        // Lost a race; dial into the now-present runtime instead.
        if let Some(runtime) = runtime.as_ref() {
            return runtime
                .dial_endpoint_addr(addr)
                .await
                .map_err(|error| format!("mesh dial failed: {error}"));
        }
    }
    let started = mesh_llm::DesktopMeshRuntime::start(start)
        .await
        .map_err(|error| format!("mesh client failed to start: {error}"))?;
    *runtime = Some(started);
    Ok(())
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
        join_token: Some(join_token),
    };
    let mut runtime = state.mesh_llm_runtime.lock().await;
    if runtime.is_some() {
        return Err("mesh node changed while starting relay mesh client".to_string());
    }
    let started = mesh_llm::DesktopMeshRuntime::start(start)
        .await
        .map_err(|error| format!("mesh client failed to start: {error}"))?;
    let status = started
        .status()
        .await
        .map_err(|error| format!("mesh client started but status probe failed: {error}"))?;
    *runtime = Some(started);
    Ok(status)
}

/// Re-resolve a live serve target's dial pointer for a saved relay-mesh agent.
///
/// The serve target's `endpoint_addr` is live discovery state — it comes from
/// the peer's replaceable kind:30621 status event and rotates when the peer's
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
    let events = relay::query_relay(state, &[mesh_llm::mesh_status_filter()]).await?;
    Ok(pick_serve_target_for_model(
        mesh_llm::availability_from_events(events).serve_targets,
        model_id,
    ))
}

/// Pure target-selection used by `resolve_mesh_bootstrap_target`: the first
/// gossiped serve target that hosts `model_id`. Returns the full target so the
/// caller has the reporter pubkey (to address the paired connect-request) as
/// well as the dial pointer. Split out so the matching rule is unit-testable
/// without a relay round-trip.
fn pick_serve_target_for_model(
    targets: Vec<mesh_llm::MeshServeTarget>,
    model_id: &str,
) -> Option<mesh_llm::MeshServeTarget> {
    targets
        .into_iter()
        .find(|target| target.model_id == model_id)
}

/// Decide whether a relay-mesh agent may start, and bring up its local mesh
/// client when needed.
///
/// Fresh create (`allow_fresh_create_start`) has just run the client-start flow
/// from the dialog, so it spawns as-is. For a saved/manual start the serve
/// target's dial pointer was never persisted (it is live discovery state), so
/// re-resolve a current bootstrap target from the relay's gossiped targets,
/// bring up the local client node, then publish a paired connect-request
/// (kind:24621) through the runtime coordinator so the peer dials back — the
/// hole-punch needs *both* ends dialing. This is the fix for saved agents
/// flaking on restart: before, saved-start did a one-sided dial and never told
/// the peer to dial back. The two failure modes get distinct, actionable copy:
/// a relay query failure ("could not refresh targets") is not the same as a
/// relay that answered with no live target for this model ("peer offline").
/// Non relay-mesh records are a no-op.
pub(crate) async fn ensure_relay_mesh_for_record(
    app: &AppHandle,
    record: &crate::managed_agents::ManagedAgentRecord,
    allow_fresh_create_start: bool,
) -> Result<(), String> {
    if allow_fresh_create_start {
        return Ok(());
    }
    let state = app.state::<AppState>();
    let Some(model_id) = crate::managed_agents::relay_mesh_model_id(record) else {
        return Ok(());
    };
    let target = match resolve_mesh_bootstrap_target(&state, &model_id).await {
        Ok(Some(target)) => target,
        Ok(None) => {
            return Err(format!(
                "relay mesh agents cannot be started from saved state because no live serve target is available for this model. Start serving on a mesh peer, or create a new agent with Run on relay mesh selected to refresh the target for {RELAY_MESH_API_BASE_URL}."
            ));
        }
        Err(error) => {
            return Err(format!(
                "could not refresh relay mesh serve targets to start this agent: {error}"
            ));
        }
    };

    // Bring up the local client node (and dial the peer). Its status carries
    // our own invite token — the addr we advertise to the peer in the 24621.
    let status =
        ensure_client_node_for_model(&state, &model_id, Some(target.endpoint_addr.clone())).await?;

    // Publish the paired connect-request so the peer dials *us* back. Needs the
    // target's reporter pubkey (whom to address) and our invite token (where to
    // dial). If either is missing we have already done the one-sided dial above
    // — no worse than the old behavior — so degrade rather than fail the start.
    if let (Some(target_pubkey), Some(self_addr)) = (
        normalize_pubkey(target.reporter_pubkey.as_deref()),
        status.invite_token.as_deref(),
    ) {
        if target_pubkey != workspace_pubkey(&state)? {
            if let Err(error) = crate::mesh_llm::start_client(
                app,
                crate::mesh_llm::RelayMeshConnectRequest {
                    target_pubkey: &target_pubkey,
                    peer_endpoint_addr: &target.endpoint_addr,
                    self_endpoint_addr: self_addr,
                    peer_endpoint_id: target.endpoint_id.as_deref(),
                    self_endpoint_id: status.endpoint_id.as_deref(),
                },
            )
            .await
            {
                // Non-fatal: the one-sided dial may still punch on a favorable NAT.
                // Surface the reason without blocking the agent's spawn.
                eprintln!("sprout-mesh: saved-start connect-request failed: {error}");
            }
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshDialEndpointRequest {
    pub endpoint_addr: String,
}

#[tauri::command]
pub async fn mesh_dial_endpoint_addr(
    state: State<'_, AppState>,
    request: MeshDialEndpointRequest,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    let endpoint_addr = request.endpoint_addr.trim();
    if endpoint_addr.is_empty() {
        return Err("endpointAddr is required".to_string());
    }
    let runtime = state.mesh_llm_runtime.lock().await;
    let Some(runtime) = runtime.as_ref() else {
        return Err("mesh node is not running".to_string());
    };
    runtime
        .dial_endpoint_addr(endpoint_addr)
        .await
        .map_err(|error| format!("mesh dial failed: {error}"))?;
    runtime.status().await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn mesh_status_report_payload(
    state: State<'_, AppState>,
) -> CmdResult<Option<serde_json::Value>> {
    let runtime = state.mesh_llm_runtime.lock().await;
    match runtime.as_ref() {
        Some(runtime) => runtime
            .status_report_payload()
            .await
            .map(Some)
            .map_err(|error| error.to_string()),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn mesh_stop_node(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    let runtime = state.mesh_llm_runtime.lock().await.take();
    if let Some(runtime) = runtime {
        runtime.stop().await.map_err(|error| error.to_string())?;
    }
    mesh_llm::publish_stopped_status_once(&app, "stop").await;
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

#[tauri::command]
pub fn mesh_agent_preset(
    request: mesh_llm::MeshAgentPresetRequest,
) -> CmdResult<mesh_llm::MeshAgentPreset> {
    mesh_llm::agent_preset(request)
}

#[cfg(all(test, feature = "mesh-llm"))]
mod tests {
    use super::*;
    use crate::app_state::build_app_state;

    fn target(model_id: &str, endpoint_addr: &str) -> mesh_llm::MeshServeTarget {
        mesh_llm::MeshServeTarget {
            model_id: model_id.to_string(),
            model_name: None,
            endpoint_addr: endpoint_addr.to_string(),
            node_name: None,
            capacity: None,
            reporter_pubkey: None,
            endpoint_id: None,
            device_id: None,
            device_name: None,
        }
    }

    #[test]
    fn pick_serve_target_returns_first_match_for_model() {
        let targets = vec![
            target("model-a", "addr-a"),
            target("model-b", "addr-b1"),
            target("model-b", "addr-b2"),
        ];
        // Matches by model id, returns the first such target (full struct, so
        // the caller has the reporter pubkey as well as the dial pointer).
        assert_eq!(
            pick_serve_target_for_model(targets, "model-b").map(|t| t.endpoint_addr),
            Some("addr-b1".to_string())
        );
    }

    #[test]
    fn pick_serve_target_none_when_model_not_hosted() {
        let targets = vec![target("model-a", "addr-a")];
        // No live target serves this model -> caller falls closed.
        assert_eq!(pick_serve_target_for_model(targets, "model-missing"), None);
    }

    #[tokio::test]
    async fn cold_client_preflight_requires_explicit_target() {
        let state = build_app_state();
        let error = ensure_client_node_for_model(&state, "demo/model", None)
            .await
            .expect_err("cold relay-mesh preflight must not auto-pick a target");
        assert_eq!(error, RELAY_MESH_RUNTIME_NO_TARGET);
    }

    /// Acceptance-critical regression for dropping the serve-vs-client guard.
    ///
    /// Before this change, `ensure_client_node_for_model` hard-errored whenever
    /// the running runtime was in `Serve` mode ("stop sharing before using
    /// relay mesh as a client"). That forbade the exact thing a user should be
    /// able to do: host model A while pointing an agent at a different model B
    /// through the same `9337` ingress.
    ///
    /// This test starts a real serve runtime and asserts that a follow-up
    /// preflight for a *different* model and no explicit target still reuses the
    /// existing runtime. Cold starts without a target are rejected before mesh-llm
    /// startup; running runtimes are already joined to whatever target the
    /// frontend selected earlier.
    ///
    /// Hardware-gated (`#[ignore]`): loads a real model. Run with:
    ///   cargo test -p sprout-desktop --features mesh-llm \
    ///     ensure_serve_runtime_serves_other_model -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "loads a real model; run manually with --ignored"]
    async fn ensure_serve_runtime_serves_other_model() {
        const HOSTED_MODEL: &str = "jc-builds/SmolLM2-135M-Instruct-Q4_K_M-GGUF:Q4_K_M";
        const OTHER_MODEL: &str = "some/other-model-not-hosted-locally:Q4_K_M";

        let state = build_app_state();

        // Start a serve runtime hosting HOSTED_MODEL — this is the "Share
        // compute" path.
        let serve = mesh_llm::DesktopMeshRuntime::start(mesh_llm::StartMeshNodeRequest {
            mode: mesh_llm::MeshNodeMode::Serve,
            model_id: Some(HOSTED_MODEL.to_string()),
            max_vram_gb: None,
            join_token: None,
        })
        .await
        .expect("serve runtime should start");

        let serve_status = serve.status().await.expect("serve status");
        let serve_base = serve_status.api_base_url.clone();
        assert_eq!(serve_status.mode, Some(mesh_llm::MeshNodeMode::Serve));

        {
            let mut runtime = state.mesh_llm_runtime.lock().await;
            *runtime = Some(serve);
        }

        // Preflight for a DIFFERENT model with no explicit target. Old code:
        // Err(...sharing compute...). New code: reuse the running ingress.
        let status = ensure_client_node_for_model(&state, OTHER_MODEL, None)
            .await
            .expect("serve runtime must not reject a different-model preflight");

        // It returns the SAME running node — agents keep using A's 9337, and
        // the router decides routability for OTHER_MODEL per request.
        assert_eq!(
            status.mode,
            Some(mesh_llm::MeshNodeMode::Serve),
            "preflight should reuse the existing serve runtime, not spin up a client"
        );
        assert_eq!(
            status.api_base_url, serve_base,
            "agent must be pointed at the existing serve node's ingress"
        );

        // Clean up the runtime.
        let taken = state.mesh_llm_runtime.lock().await.take();
        if let Some(runtime) = taken {
            let _ = runtime.stop().await;
        }
    }
}
