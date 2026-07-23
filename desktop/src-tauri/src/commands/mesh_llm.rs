use std::path::PathBuf;

use tauri::{AppHandle, Manager, State};

use crate::{app_state::AppState, mesh_llm, relay};

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct MeshSharingConfig {
    enabled: bool,
    model_id: String,
    max_vram_gb: Option<u64>,
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

pub type CmdResult<T> = Result<T, String>;

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

async fn query_mesh_discovery_events(state: &AppState) -> Result<Vec<nostr::Event>, String> {
    let mut events = relay::query_relay(state, &[mesh_llm::relay_membership_filter()]).await?;
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
        let page = relay::query_relay(state, &[status_filter.clone()]).await?;
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

pub(crate) async fn restore_mesh_sharing(app: &AppHandle, state: &AppState) -> CmdResult<()> {
    let Some(config) = load_mesh_sharing_config(app)? else {
        return Ok(());
    };
    if !config.enabled || config.model_id.trim().is_empty() {
        return Ok(());
    }
    let mut runtime = state.mesh_llm_runtime.lock().await;
    if runtime.is_some() {
        return Ok(());
    }
    let request = mesh_llm::StartMeshNodeRequest {
        mode: mesh_llm::MeshNodeMode::Serve,
        model_id: Some(config.model_id),
        max_vram_gb: config.max_vram_gb,
        join_token: None,
        trusted_owner_ids: Some(resolve_trusted_owner_ids_or_self_only(state).await),
    };
    let started = mesh_llm::DesktopMeshRuntime::start(request)
        .await
        .map_err(|error| format!("failed to restore Share Compute: {error}"))?;
    *runtime = Some(started);
    drop(runtime);
    mesh_llm::publish_current_status_once(app, "restore").await;
    Ok(())
}

#[tauri::command]
pub async fn mesh_start_node(
    app: AppHandle,
    state: State<'_, AppState>,
    mut request: mesh_llm::StartMeshNodeRequest,
) -> CmdResult<mesh_llm::MeshNodeStatus> {
    // Frontend requests never carry a roster; resolve it here so every
    // UI-started node enforces the member allowlist.
    if request.trusted_owner_ids.is_none() {
        request.trusted_owner_ids = Some(resolve_trusted_owner_ids_or_self_only(&state).await);
    }
    let mut runtime = state.mesh_llm_runtime.lock().await;
    if runtime.is_some() {
        return Err("mesh node is already running".to_string());
    }

    let saved_request = request.clone();
    let started = mesh_llm::DesktopMeshRuntime::start(request)
        .await
        .map_err(|error| error.to_string())?;
    let status = started
        .status()
        .await
        .map_err(|error| format!("mesh node started but status probe failed: {error}"))?;
    *runtime = Some(started);
    drop(runtime);
    if saved_request.mode == mesh_llm::MeshNodeMode::Serve {
        if let Some(model_id) = saved_request.model_id.as_deref() {
            save_mesh_sharing_config(
                &app,
                &MeshSharingConfig {
                    enabled: true,
                    model_id: model_id.to_string(),
                    max_vram_gb: saved_request.max_vram_gb,
                },
            )?;
        }
    }
    mesh_llm::publish_current_status_once(&app, "start").await;
    Ok(status)
}

/// Mesh can bind its HTTP ingress and advertise a model shortly before the
/// router has installed a usable target. Probe the exact chat path agents use
/// so startup cannot race that gap (`single target None unavailable`).
async fn wait_for_mesh_inference(model_id: &str) -> CmdResult<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("failed to build mesh readiness client: {error}"))?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    let mut last_error = "mesh inference is not ready".to_string();
    while tokio::time::Instant::now() < deadline {
        match client
            .post(format!(
                "{}/chat/completions",
                crate::managed_agents::RELAY_MESH_API_BASE_URL
            ))
            .bearer_auth(crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER)
            .json(&serde_json::json!({
                "model": model_id,
                "messages": [{"role": "user", "content": "Reply OK"}],
                "max_tokens": 1,
                "stream": false
            }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                last_error = format!("HTTP {status}: {body}");
            }
            Err(error) => last_error = error.to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Err(format!(
        "Buzz shared compute did not become inference-ready for {model_id}: {last_error}"
    ))
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
        trusted_owner_ids: Some(resolve_trusted_owner_ids_or_self_only(state).await),
    };
    let mut runtime = state.mesh_llm_runtime.lock().await;
    if runtime.is_some() {
        return Err("mesh node changed while starting Buzz shared compute client".to_string());
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
    record: &crate::managed_agents::ManagedAgentRecord,
    _allow_fresh_create_start: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(model_id) = crate::managed_agents::relay_mesh_model_id(record) else {
        return Ok(());
    };
    // A local serve/client runtime already owns the OpenAI ingress and its
    // router can resolve both `auto` and explicit remote models. Do not require
    // a separate relay-advertised target in that case.
    if state.mesh_llm_runtime.lock().await.is_some() {
        return wait_for_mesh_inference(&model_id).await;
    }
    let target = match resolve_mesh_bootstrap_target(&state, &model_id).await {
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

    ensure_client_node_for_model(&state, &model_id, Some(target.endpoint_addr)).await?;
    wait_for_mesh_inference(&model_id).await
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
    save_mesh_sharing_config(
        &app,
        &MeshSharingConfig {
            enabled: false,
            model_id: String::new(),
            max_vram_gb: None,
        },
    )?;
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
            endpoint_id: None,
            device_id: None,
            device_name: None,
        }
    }

    #[test]
    fn mesh_status_cursor_uses_relay_composite_tiebreak() {
        let event = nostr::EventBuilder::new(nostr::Kind::TextNote, "status")
            .custom_created_at(nostr::Timestamp::from(1_234))
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign test status");
        let mut filter = mesh_llm::mesh_status_filter();

        let cursor = advance_mesh_status_cursor(&mut filter, std::slice::from_ref(&event))
            .expect("advance status cursor");

        assert_eq!(cursor, (1_234, event.id.to_hex()));
        assert_eq!(filter["until"], serde_json::json!(1_234));
        assert_eq!(filter["before_id"], serde_json::json!(event.id.to_hex()));
        assert_eq!(
            filter["limit"],
            serde_json::json!(mesh_llm::MESH_STATUS_PAGE_SIZE)
        );
    }

    #[test]
    fn pick_serve_target_returns_first_match_for_model() {
        let targets = vec![
            target("model-a", "addr-a"),
            target("model-b", "addr-b1"),
            target("model-b", "addr-b2"),
        ];
        // Matches by model id and returns the first such target.
        assert_eq!(
            pick_serve_target_for_model(targets, "model-b").map(|t| t.endpoint_addr),
            Some("addr-b1".to_string())
        );
    }

    #[test]
    fn pick_serve_target_normalizes_main_revision() {
        let targets = vec![target("org/model@main:q4", "addr")];
        assert_eq!(
            pick_serve_target_for_model(targets, "org/model:q4").map(|target| target.endpoint_addr),
            Some("addr".to_string())
        );
    }

    #[test]
    fn pick_serve_target_auto_takes_any_live_target() {
        let targets = vec![target("model-a", "addr-a"), target("model-b", "addr-b")];
        // "auto" delegates model choice to the mesh router; any live target
        // is a valid bootstrap peer (first one wins).
        assert_eq!(
            pick_serve_target_for_model(targets, crate::mesh_llm::AUTO_MODEL_ID)
                .map(|t| t.endpoint_addr),
            Some("addr-a".to_string())
        );
        // But auto with zero live targets still falls closed.
        assert_eq!(
            pick_serve_target_for_model(Vec::new(), crate::mesh_llm::AUTO_MODEL_ID),
            None
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
    /// Buzz shared compute as a client"). That forbade exactly what a user should be
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
    ///   cargo test -p buzz-desktop --features mesh-llm \
    ///     ensure_serve_runtime_serves_other_model -- --ignored --nocapture
    #[test]
    #[ignore = "loads a real model; run manually with --ignored"]
    fn ensure_serve_runtime_serves_other_model() {
        std::thread::Builder::new()
            .name("mesh-hardware-acceptance".to_string())
            .stack_size(mesh_llm::MESH_WORKER_STACK_SIZE)
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_stack_size(mesh_llm::MESH_WORKER_STACK_SIZE)
                    .enable_all()
                    .build()
                    .expect("build mesh acceptance runtime");
                runtime.block_on(async {
                    const HOSTED_MODEL: &str = "jc-builds/SmolLM2-135M-Instruct-Q4_K_M-GGUF:Q4_K_M";
                    const OTHER_MODEL: &str = "some/other-model-not-hosted-locally:Q4_K_M";

                    let state = build_app_state();

                    // Start a serve runtime hosting HOSTED_MODEL — this is the "Share
                    // compute" path.
                    let serve =
                        mesh_llm::DesktopMeshRuntime::start(mesh_llm::StartMeshNodeRequest {
                            mode: mesh_llm::MeshNodeMode::Serve,
                            model_id: Some(HOSTED_MODEL.to_string()),
                            max_vram_gb: None,
                            join_token: None,
                            trusted_owner_ids: None,
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
                });
            })
            .expect("spawn mesh acceptance thread")
            .join()
            .expect("mesh acceptance thread panicked");
    }
}
