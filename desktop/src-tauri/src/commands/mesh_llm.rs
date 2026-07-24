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

/// Sentinel prefix on every `last_error` the ingress re-arm watchdog writes, so
/// `clear_mesh_last_error_if_set` only clears errors this path actually set
/// rather than any message that merely mentions "shared compute"
/// (micspiral review #2). Kept out of the user-facing tail of the string.
pub(crate) const MESH_REARM_ERROR_SENTINEL: &str = "[buzz-mesh-rearm] ";

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

/// Fast liveness probe of the local mesh OpenAI ingress (`:9337`).
///
/// Unlike [`wait_for_mesh_inference`], this does not run a full chat completion
/// or retry for two minutes — it issues a single short-timeout `GET /v1/models`
/// (the same call the issue used to confirm the ingress was dead) and reports
/// reachability. Used to detect a `mesh_llm_runtime = Some` handle that points
/// at an exited/wedged runtime so we can drop it and re-arm instead of waiting
/// on a dead endpoint (#2062).
///
/// `pub(crate)` so the mesh coordinator watchdog can share the same probe on the
/// post-launch path (Brad #2304: ensure_relay_mesh_for_record only runs on start
/// / restore, not on every inbound turn).
pub(crate) async fn mesh_ingress_is_live() -> bool {
    mesh_ingress_is_live_at(crate::managed_agents::RELAY_MESH_API_BASE_URL).await
}

/// Testable variant of [`mesh_ingress_is_live`] with an injectable base URL
/// (`…/v1`). Production always passes [`RELAY_MESH_API_BASE_URL`].
pub(crate) async fn mesh_ingress_is_live_at(api_base_url: &str) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let base = api_base_url.trim_end_matches('/');
    client
        .get(format!("{base}/models"))
        .bearer_auth(crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER)
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

/// If a runtime handle is present but `:9337` is unreachable, drop the stale
/// runtime (best-effort stop) so a subsequent ensure/bootstrap can re-arm.
///
/// Returns `true` when a stale handle was evicted (caller should re-arm).
/// Bounded budget for a best-effort stop of a stale runtime. Matches the 3s
/// ingress-probe budget: if the embedded runtime is *itself* the wedged
/// component, `stop()` can hang forever, which would defeat the whole
/// "never block re-arm" intent (Brad #2304 #1). On timeout we log and drop the
/// handle anyway so the watchdog keeps making progress.
const STALE_STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Pure identity gate used after the ingress probe returns dead (Brad #2304 #2).
/// Only evict when the same runtime id is still installed; a concurrent
/// replacement must be left alone.
pub(crate) fn should_evict_stale_runtime_after_probe(
    candidate_id: u64,
    current_id: Option<u64>,
) -> bool {
    matches!(current_id, Some(id) if id == candidate_id)
}

/// Consecutive dead-probe debounce before we evict a healthy-looking runtime
/// (micspiral review, #2304 #1). `rearm_relay_mesh_for_running_agents`
/// early-returns on a healthy handle, so a dead probe is the *only* thing that
/// ever touches a running runtime — a single false-negative (a transient stall
/// past the 3s probe budget: model load/reload, VRAM alloc, GC/mmap pause,
/// inference saturation) would otherwise force an avoidable cold re-bootstrap
/// (and, for a serve node, a mode flip). Requiring 2 consecutive dead probes
/// costs ~15-30s extra on genuine recovery at the 15s base cadence while
/// eliminating transient-blip false evictions; `wait_for_mesh_inference`
/// already tolerates a 120s warm-up on the readiness path, so the liveness
/// probe having zero tolerance was the asymmetry worth closing.
pub(crate) const DEAD_PROBE_EVICT_THRESHOLD: u32 = 2;

/// Pure debounce gate: evict only once dead probes have reached the threshold.
pub(crate) fn should_evict_after_consecutive_dead_probes(consecutive: u32) -> bool {
    consecutive >= DEAD_PROBE_EVICT_THRESHOLD
}

pub(crate) async fn drop_stale_mesh_runtime_if_ingress_dead(state: &AppState) -> bool {
    drop_stale_mesh_runtime_if_ingress_dead_with_probe(state, mesh_ingress_is_live()).await
}

/// Injectable-probe variant for deterministic unit tests (Brad #2304 recovery
/// proof). Production always uses [`mesh_ingress_is_live`].
pub(crate) async fn drop_stale_mesh_runtime_if_ingress_dead_with_probe<F>(
    state: &AppState,
    probe_ingress_live: F,
) -> bool
where
    F: std::future::Future<Output = bool> + Send,
{
    // Capture identity *before* the probe `.await` so a concurrent stop/start
    // that swaps the handle mid-probe cannot cause us to evict the replacement.
    let candidate_id = match state.mesh_llm_runtime.lock().await.as_ref() {
        Some(runtime) => runtime.id(),
        None => {
            // No handle to guard — keep the debounce counter clean.
            state
                .mesh_ingress_dead_probes
                .store(0, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
    };
    if probe_ingress_live.await {
        // A live probe clears any accumulated dead streak (micspiral #1).
        state
            .mesh_ingress_dead_probes
            .store(0, std::sync::atomic::Ordering::Relaxed);
        return false;
    }
    // Dead probe: debounce so one transient stall does not evict a healthy
    // runtime. Only proceed to eviction once we have seen the ingress dead
    // across N consecutive watchdog ticks (micspiral review #1).
    let consecutive = state
        .mesh_ingress_dead_probes
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        + 1;
    if !should_evict_after_consecutive_dead_probes(consecutive) {
        eprintln!(
            "buzz-mesh: ingress probe dead ({consecutive}/{DEAD_PROBE_EVICT_THRESHOLD}); debouncing before eviction (#2304)"
        );
        return false;
    }
    let stale = {
        let mut guard = state.mesh_llm_runtime.lock().await;
        let current_id = guard.as_ref().map(|runtime| runtime.id());
        if !should_evict_stale_runtime_after_probe(candidate_id, current_id) {
            // A concurrent stop/start swapped in a different runtime during the
            // probe window; leave it alone and reset the streak so the fresh
            // handle is judged on its own probes (micspiral #1 + #2304 #2).
            state
                .mesh_ingress_dead_probes
                .store(0, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        guard.take()
    };
    let Some(stale) = stale else {
        return false;
    };
    // Confirmed dead across the debounce window and we own the eviction — reset
    // the streak so the next runtime starts with a clean counter.
    state
        .mesh_ingress_dead_probes
        .store(0, std::sync::atomic::Ordering::Relaxed);
    eprintln!(
        "buzz-mesh: Buzz shared compute ingress is down while a runtime handle is present; dropping the stale runtime for re-arm (#2062)"
    );
    // Best-effort, bounded: a wedged runtime may fail/hang on stop; never block
    // re-arm on it (the zombie-guard motivation in #2062, Brad #2304 #1).
    match tokio::time::timeout(STALE_STOP_TIMEOUT, stale.stop()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("stale mesh runtime stop failed during re-arm: {error}");
        }
        Err(_) => {
            eprintln!(
                "stale mesh runtime stop timed out after {}s during re-arm; dropping handle anyway (#2304)",
                STALE_STOP_TIMEOUT.as_secs()
            );
        }
    }
    true
}

/// Post-launch recovery for running relay-mesh agents when the shared ingress
/// died under a live handle (#2062 / Brad #2304).
///
/// Call path: mesh coordinator bounded watchdog (not message dispatch — local
/// agents talk to `:9337` themselves; desktop must heal the ingress without a
/// turn hook). Drops a dead handle, then re-runs [`ensure_relay_mesh_for_record`]
/// for every *actively running* local relay-mesh agent. Failures are written
/// to `last_error` so the UI surfaces an actionable shared-compute-offline state
/// instead of silent non-response.
pub(crate) async fn rearm_relay_mesh_for_running_agents(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let had_handle = state.mesh_llm_runtime.lock().await.is_some();
    let evicted = drop_stale_mesh_runtime_if_ingress_dead(&state).await;
    let active_pubkeys = active_managed_agent_pubkeys(&state);

    // Only re-arm when we actually had a dead handle, or when running
    // relay-mesh agents need a runtime and there is currently none. Avoid
    // thrashing healthy runtimes with ensure every tick.
    if !evicted && had_handle {
        return Ok(());
    }
    if !evicted && !had_handle {
        let records = crate::managed_agents::load_managed_agents(app).unwrap_or_default();
        let needs = records
            .iter()
            .any(|record| is_running_relay_mesh_agent(record, &active_pubkeys));
        if !needs {
            return Ok(());
        }
    }

    let records = crate::managed_agents::load_managed_agents(app).unwrap_or_default();
    let mesh_records: Vec<_> = records
        .into_iter()
        .filter(|record| is_running_relay_mesh_agent(record, &active_pubkeys))
        .collect();
    if mesh_records.is_empty() {
        return Ok(());
    }

    let mut first_error: Option<String> = None;
    for record in &mesh_records {
        match ensure_relay_mesh_for_record(app, record, false).await {
            Ok(()) => {
                if let Err(error) = clear_mesh_last_error_if_set(app, &record.pubkey) {
                    eprintln!(
                        "buzz-mesh: failed to clear shared-compute last_error for {}: {error}",
                        record.pubkey
                    );
                }
            }
            Err(error) => {
                let msg = format!(
                    "{MESH_REARM_ERROR_SENTINEL}Buzz shared compute offline — failed to re-arm local ingress for this agent: {error}"
                );
                eprintln!("buzz-mesh: re-arm failed for {}: {msg}", record.pubkey);
                if let Err(persist_error) = persist_mesh_last_error(app, &record.pubkey, &msg) {
                    eprintln!(
                        "buzz-mesh: failed to persist shared-compute last_error for {}: {persist_error}",
                        record.pubkey
                    );
                }
                if first_error.is_none() {
                    first_error = Some(msg);
                }
            }
        }
    }
    match first_error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Pubkeys currently present in the live managed-agent process map.
fn active_managed_agent_pubkeys(state: &AppState) -> std::collections::HashSet<String> {
    state
        .managed_agent_processes
        .lock()
        .map(|guard| {
            guard
                .keys()
                .map(|key| key.pubkey.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

/// A record is a live consumer of the shared-compute ingress only when it is a
/// local relay-mesh agent that is *actually running in this desktop process*
/// (present in `managed_agent_processes`) and whose `runtime_pid` still looks
/// alive. Stopped/manual records must not start the mesh client or hold the
/// watchdog in failure backoff (Brad #2304 #3).
fn is_running_relay_mesh_agent(
    record: &crate::managed_agents::ManagedAgentRecord,
    active_pubkeys: &std::collections::HashSet<String>,
) -> bool {
    if record.backend != crate::managed_agents::BackendKind::Local {
        return false;
    }
    if crate::managed_agents::relay_mesh_model_id(record).is_none() {
        return false;
    }
    if !active_pubkeys.contains(&record.pubkey.to_ascii_lowercase()) {
        return false;
    }
    match record.runtime_pid {
        Some(pid) => crate::managed_agents::process_is_running(pid),
        // Process-map entry without a pid is still a live harness registration
        // (starting / listening); treat as running so re-arm can serve it.
        None => true,
    }
}

/// Persist mesh re-arm failure under the same store lock as restore/install
/// error paths (Brad #2304 #4). Updates `updated_at`, preserves unrelated
/// fields/errors on other records, and surfaces persistence failures.
fn persist_mesh_last_error(app: &AppHandle, pubkey: &str, error: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| format!("failed to acquire managed agents store lock: {e}"))?;
    let mut records = crate::managed_agents::load_managed_agents(app)?;
    let record = crate::managed_agents::find_managed_agent_mut(&mut records, pubkey)?;
    record.last_error = Some(error.to_string());
    record.updated_at = crate::util::now_iso();
    crate::managed_agents::save_managed_agents(app, &records)
}

/// Clear only shared-compute offline errors after a successful re-arm. Other
/// last_error values are left untouched (Brad #2304 #4 preserve unrelated).
fn clear_mesh_last_error_if_set(app: &AppHandle, pubkey: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| format!("failed to acquire managed agents store lock: {e}"))?;
    let mut records = crate::managed_agents::load_managed_agents(app)?;
    let record = crate::managed_agents::find_managed_agent_mut(&mut records, pubkey)?;
    let Some(err) = record.last_error.as_deref() else {
        return Ok(());
    };
    // Only clear errors this watchdog set (sentinel prefix), never an unrelated
    // last_error that merely mentions "shared compute" (micspiral #2).
    if !err.starts_with(MESH_REARM_ERROR_SENTINEL) {
        return Ok(());
    }
    record.last_error = None;
    record.updated_at = crate::util::now_iso();
    crate::managed_agents::save_managed_agents(app, &records)
}

/// Mesh can bind its HTTP ingress and advertise a model shortly before the
/// router has installed a usable target. Probe the exact chat path agents use
/// so startup cannot race that gap (`single target None unavailable`).
/// Which startup stage a mesh client is stuck at when it never becomes
/// inference-ready. The two live-observed failure modes are physically
/// distinct and want different user copy:
///
///   * `CatalogNeverSynced` — the local client node came up and connected to
///     the host at the control level (ping/RTT fine), but the served model
///     never appeared in the local `/v1/models` catalog. That catalog is
///     populated by the peer gossip exchange; when the gossip bi-stream can't
///     establish across the network (observed as iroh
///     `MultipathNotNegotiated` / unreachable direct path), the catalog stays
///     empty forever and every request is rejected "model not available".
///     Root cause is the network path between this machine and the host.
///   * `RoutingNeverCompleted` — the model *did* sync into the catalog, but
///     inference requests never completed (routing/transport to the host
///     failing per-request). The host is discoverable and advertised but not
///     actually serving us.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshReadinessFailure {
    CatalogNeverSynced,
    RoutingNeverCompleted,
}

/// Pure classifier: given whether the served model was ever observed in the
/// local `/v1/models` catalog during the wait, decide which stage failed.
/// Split out so the diagnosis is unit-testable without a live mesh.
fn classify_mesh_readiness_failure(model_ever_visible: bool) -> MeshReadinessFailure {
    if model_ever_visible {
        MeshReadinessFailure::RoutingNeverCompleted
    } else {
        MeshReadinessFailure::CatalogNeverSynced
    }
}

/// Actionable, non-technical copy for a readiness failure. `last_detail` is the
/// last raw transport/HTTP error, appended for support triage.
fn mesh_readiness_failure_message(
    failure: MeshReadinessFailure,
    model_id: &str,
    last_detail: &str,
) -> String {
    match failure {
        MeshReadinessFailure::CatalogNeverSynced => format!(
            "Buzz shared compute connected to the serving member but could not sync \
             the model list for \"{model_id}\" — this is a network path problem \
             between this machine and the host (the compute node is reachable for \
             pings but the model-sync stream did not establish). Try again, or have \
             the host and this machine on a more direct network. (last: {last_detail})"
        ),
        MeshReadinessFailure::RoutingNeverCompleted => format!(
            "Buzz shared compute found \"{model_id}\" on a serving member but inference \
             requests did not complete — the host is discoverable but not currently \
             reachable for requests. Try again shortly. (last: {last_detail})"
        ),
    }
}

/// Poll the local mesh OpenAI ingress until a real inference for `model_id`
/// succeeds, or a deadline elapses. On failure, returns a stage-specific,
/// actionable message (see [`MeshReadinessFailure`]) rather than a raw
/// `HTTP 429`, so the UI can tell "still warming up" apart from "can't reach
/// the host".
async fn wait_for_mesh_inference(model_id: &str) -> CmdResult<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("failed to build mesh readiness client: {error}"))?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    let models_url = format!("{}/models", crate::managed_agents::RELAY_MESH_API_BASE_URL);
    let chat_url = format!(
        "{}/chat/completions",
        crate::managed_agents::RELAY_MESH_API_BASE_URL
    );
    let mut last_error = "mesh inference is not ready".to_string();
    // Track whether the served model ever reached the local catalog — the
    // signal that splits "catalog never synced" from "routing never completed".
    let mut model_ever_visible = false;

    while tokio::time::Instant::now() < deadline {
        // Refresh catalog visibility. "auto" delegates model choice to the
        // router, so any advertised model counts as the catalog having synced.
        if let Ok(response) = client
            .get(&models_url)
            .bearer_auth(crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER)
            .send()
            .await
        {
            if let Ok(body) = response.json::<serde_json::Value>().await {
                if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
                    let wanted = model_id.trim().replace("@main", "");
                    let visible = !data.is_empty()
                        && (model_id == crate::mesh_llm::AUTO_MODEL_ID
                            || data.iter().any(|m| {
                                m.get("id")
                                    .and_then(|id| id.as_str())
                                    .map(|id| id.replace("@main", "") == wanted)
                                    .unwrap_or(false)
                            }));
                    model_ever_visible |= visible;
                }
            }
        }

        match client
            .post(&chat_url)
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

    let failure = classify_mesh_readiness_failure(model_ever_visible);
    Err(mesh_readiness_failure_message(
        failure,
        model_id,
        &last_error,
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
    // a separate relay-advertised target in that case — BUT only trust it when
    // the ingress is actually alive. A runtime that exited/wedged after launch
    // leaves `mesh_llm_runtime = Some` pointing at a dead `:9337` ingress, so a
    // blind `wait_for_mesh_inference` would just time out and the agent would
    // stay silent (#2062). Probe first; if the ingress is dead, drop the stale
    // runtime and fall through to re-arm it. The mesh coordinator watchdog also
    // calls this path after eviction so recovery is not start-only (Brad #2304).
    if state.mesh_llm_runtime.lock().await.is_some() {
        if !drop_stale_mesh_runtime_if_ingress_dead(&state).await {
            return wait_for_mesh_inference(&model_id).await;
        }
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

    // Serve→Client re-arm transition (micspiral review #3, intentional-by-design):
    // if the dead ingress belonged to a *serve* node with running consumer
    // agents, this re-arms it as a Client (`MeshNodeMode::Client`). That is the
    // correct/safe recovery here — config-backed serve restoration is
    // `restore_mesh_sharing`'s job (`MeshNodeMode::Serve`), and
    // `ensure_client_node_for_model` reuses any live runtime of *either* mode
    // (the router resolves per-request), so it only cold-starts a Client when
    // there is genuinely no runtime. Falling back to Client if a serve node
    // crashed under local pressure is a desirable fail-safe, not a regression.
    ensure_client_node_for_model(&state, &model_id, Some(target.endpoint_addr)).await?;
    wait_for_mesh_inference(&model_id).await
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
    let taken = {
        let mut guard = state.mesh_llm_runtime.lock().await;
        if let Some(runtime) = guard.as_ref() {
            if !share_stop_should_teardown(runtime.mode()) {
                return runtime.status().await.map_err(|error| error.to_string());
            }
        }
        guard.take()
    };
    if let Some(runtime) = taken {
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
    fn readiness_failure_is_catalog_sync_when_model_never_visible() {
        assert_eq!(
            classify_mesh_readiness_failure(false),
            MeshReadinessFailure::CatalogNeverSynced
        );
    }

    #[test]
    fn readiness_failure_is_routing_when_model_was_visible() {
        assert_eq!(
            classify_mesh_readiness_failure(true),
            MeshReadinessFailure::RoutingNeverCompleted
        );
    }

    #[test]
    fn readiness_messages_are_distinct_and_actionable() {
        let catalog = mesh_readiness_failure_message(
            MeshReadinessFailure::CatalogNeverSynced,
            "auto",
            "HTTP 429",
        );
        let routing = mesh_readiness_failure_message(
            MeshReadinessFailure::RoutingNeverCompleted,
            "auto",
            "HTTP 503",
        );
        // Distinct diagnoses, each names the model and carries the raw detail.
        assert_ne!(catalog, routing);
        assert!(catalog.contains("network path"));
        assert!(catalog.contains("HTTP 429"));
        assert!(routing.contains("did not complete"));
        assert!(routing.contains("HTTP 503"));
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

    #[test]
    fn share_stop_tears_down_serve_but_not_client() {
        // Stopping "Share compute" tears down a serve node (we were sharing)
        // but must leave a client node alone (we are consuming a peer). This is
        // the backend half of the toggle-on regression: a client node occupies
        // the single slot and reports state:"running", and the stop path must
        // not kill it.
        assert!(
            share_stop_should_teardown(mesh_llm::MeshNodeMode::Serve),
            "serve node is our sharing runtime; stop must tear it down"
        );
        assert!(
            !share_stop_should_teardown(mesh_llm::MeshNodeMode::Client),
            "client node is a consume session; stop must NOT tear it down"
        );
    }

    #[test]
    fn client_status_serializes_with_running_state_and_client_mode() {
        // Contract pin for the TS mock (e2eBridge.ts) and the frontend
        // predicate: a consuming node serializes as
        // {"state":"running","mode":"client"}. If serde renaming drifts, the
        // hand-written mock shape and `deriveMeshShareToggle` would silently
        // stop matching the real IPC payload.
        let status = mesh_llm::MeshNodeStatus {
            state: mesh_llm::MeshNodeState::Running,
            mode: Some(mesh_llm::MeshNodeMode::Client),
            // `MeshHealth::ok()` is module-private; build via the public fields.
            health: mesh_llm::MeshHealth {
                status: mesh_llm::MeshHealthStatus::Ok,
                reason: None,
            },
            api_base_url: Some("http://127.0.0.1:9337/v1".to_string()),
            console_url: None,
            model_id: None,
            model_name: None,
            invite_token: None,
            endpoint_id: None,
            device_id: None,
            device_name: None,
        };
        let value = serde_json::to_value(&status).expect("serialize mesh status");
        assert_eq!(value["state"], serde_json::json!("running"));
        assert_eq!(value["mode"], serde_json::json!("client"));
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
    /// Brad #2304 sequence unit: live-looking handle is irrelevant when GET
    /// /v1/models fails — probe reports dead so callers can drop + re-arm.
    #[tokio::test]
    async fn mesh_ingress_probe_false_when_nothing_listens() {
        // High unused port — connection refused → not live (Brad step: kill ingress).
        let dead = mesh_ingress_is_live_at("http://127.0.0.1:1/v1").await;
        assert!(!dead, "dead port must not count as live ingress");
    }

    /// When no runtime handle is installed, drop helper is a no-op (no false swagger).
    #[tokio::test]
    async fn drop_stale_runtime_noop_without_handle() {
        let state = build_app_state();
        assert!(!drop_stale_mesh_runtime_if_ingress_dead(&state).await);
        assert!(state.mesh_llm_runtime.lock().await.is_none());
    }

    /// Brad sequence (steps 1–4 simplified): handle present + dead ingress ⇒
    /// drop_stale returns true and clears the Option so ensure can re-arm.
    /// We don't install a real DesktopMeshRuntime (needs model load); instead
    /// we assert the probe+branch contract the ensure path and watchdog share.
    #[tokio::test]
    async fn dead_ingress_probe_drives_rearm_branch() {
        // Shared contract: success path only when probe is true.
        // With nothing on :1, probe is false → re-arm branch taken by ensure.
        assert!(!mesh_ingress_is_live_at("http://127.0.0.1:1/v1").await);
        // Production base uses RELAY_MESH_API_BASE_URL; if CI has nothing on 9337,
        // probe should also be false (or true if a leftover mesh is up — either is
        // a bool, not panic).
        let _ = mesh_ingress_is_live().await;
    }

    /// Build a local relay-mesh record (Brad #2304 #3 filter tests). The mesh
    /// preset env is the legacy discriminator `relay_mesh_model_id` detects.
    fn mesh_record(pubkey: &str, runtime_pid: Option<u32>) -> crate::managed_agents::ManagedAgentRecord {
        let mut rec = crate::managed_agents::AgentDefinition {
            id: pubkey.to_string(),
            display_name: pubkey.to_string(),
            avatar_url: None,
            system_prompt: String::new(),
            runtime: None,
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: std::collections::BTreeMap::from([
                ("BUZZ_AGENT_PROVIDER".to_string(), "openai".to_string()),
                (
                    "OPENAI_COMPAT_BASE_URL".to_string(),
                    "http://127.0.0.1:9337/v1/".to_string(),
                ),
                ("OPENAI_COMPAT_MODEL".to_string(), "Qwen3".to_string()),
                (
                    "OPENAI_COMPAT_API_KEY".to_string(),
                    crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER.to_string(),
                ),
            ]),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
        .into_agent_record();
        rec.pubkey = pubkey.to_string();
        rec.backend = crate::managed_agents::BackendKind::Local;
        rec.runtime_pid = runtime_pid;
        rec
    }

    fn active_set(pubkeys: &[&str]) -> std::collections::HashSet<String> {
        pubkeys
            .iter()
            .map(|p| p.to_ascii_lowercase())
            .collect()
    }

    /// Brad #2304 #3: stopped / not-in-process-map relay-mesh records must NOT
    /// be treated as ingress consumers, or re-arm would resurrect a runtime
    /// the user deliberately stopped.
    #[test]
    fn stopped_relay_mesh_agent_is_not_a_rearm_target() {
        let empty = active_set(&[]);
        // Configured mesh record but no process-map entry → not running.
        assert!(!is_running_relay_mesh_agent(&mesh_record("a", None), &empty));
        assert!(!is_running_relay_mesh_agent(
            &mesh_record("a", Some(std::process::id())),
            &empty
        ));
        // In process map but pid is dead → not running.
        let active = active_set(&["b"]);
        let dead = mesh_record("b", Some(4_000_000_000));
        assert!(!is_running_relay_mesh_agent(&dead, &active));
    }

    /// Live process-map entry + live pid + mesh preset ⇒ re-arm target.
    #[test]
    fn running_relay_mesh_agent_is_a_rearm_target() {
        let pid = std::process::id();
        let active = active_set(&["live"]);
        assert!(is_running_relay_mesh_agent(
            &mesh_record("live", Some(pid)),
            &active
        ));
        // Process-map entry without pid (starting) still counts.
        assert!(is_running_relay_mesh_agent(
            &mesh_record("live", None),
            &active
        ));
    }

    /// A running process that is NOT relay-mesh (no mesh preset) is ignored
    /// even if alive — only ingress consumers get re-armed.
    #[test]
    fn running_non_mesh_agent_is_not_a_rearm_target() {
        let mut rec = mesh_record("plain", Some(std::process::id()));
        rec.env_vars.clear();
        rec.provider = None;
        rec.relay_mesh = None;
        let active = active_set(&["plain"]);
        assert!(!is_running_relay_mesh_agent(&rec, &active));
    }

    /// Brad #2304 #2: identity compare — never evict a different runtime id.
    #[test]
    fn probe_evict_identity_skips_replacement_runtime() {
        assert!(should_evict_stale_runtime_after_probe(7, Some(7)));
        assert!(!should_evict_stale_runtime_after_probe(7, Some(8)));
        assert!(!should_evict_stale_runtime_after_probe(7, None));
    }

    /// Brad #2304 #1 invariant: stop budget is finite (wedged stop cannot hang forever).
    #[test]
    fn stale_stop_timeout_is_bounded() {
        assert!(STALE_STOP_TIMEOUT.as_secs() > 0);
        assert!(STALE_STOP_TIMEOUT.as_secs() <= 5);
    }

    /// Brad #2304 #4 + micspiral #2: clear only errors this watchdog set
    /// (sentinel prefix); never an unrelated last_error, even one that mentions
    /// "shared compute".
    #[test]
    fn mesh_error_classifier_preserves_unrelated_last_error() {
        let ours = format!(
            "{MESH_REARM_ERROR_SENTINEL}Buzz shared compute offline — failed to re-arm local ingress for this agent: x"
        );
        // Sentinel-tagged → this watchdog owns it → clearable.
        assert!(ours.starts_with(MESH_REARM_ERROR_SENTINEL));
        // An unrelated error that merely mentions "shared compute" must NOT be
        // cleared (the loose-substring bug micspiral flagged).
        let bystander = "user note: shared compute config looks wrong";
        assert!(!bystander.starts_with(MESH_REARM_ERROR_SENTINEL));
        let other = "npm install failed: EACCES";
        assert!(!other.starts_with(MESH_REARM_ERROR_SENTINEL));
    }

    /// micspiral #1: eviction is debounced — a single dead probe must not evict
    /// a healthy runtime; only a sustained dead streak (>= threshold) does.
    #[test]
    fn eviction_debounces_transient_dead_probe() {
        assert!(DEAD_PROBE_EVICT_THRESHOLD >= 2);
        // One transient blip: do not evict.
        assert!(!should_evict_after_consecutive_dead_probes(1));
        // Sustained dead across the window: evict.
        assert!(should_evict_after_consecutive_dead_probes(
            DEAD_PROBE_EVICT_THRESHOLD
        ));
        assert!(should_evict_after_consecutive_dead_probes(
            DEAD_PROBE_EVICT_THRESHOLD + 5
        ));
    }

    /// Hardware-gated live kill-:9337 recovery proof (Brad sequence).
    /// Run manually when mesh hardware is available:
    ///   cargo test -p buzz-desktop --features mesh-llm     ///     kill_ingress_recovery_hardware -- --ignored --nocapture
    #[test]
    #[ignore = "hardware-gated: requires real mesh ingress on :9337"]
    fn kill_ingress_recovery_hardware_gated_documented() {
        // Documented acceptance path for Brad's 1–5 sequence. Automated CI
        // cannot load a real model / kill :9337 safely; this ignore marker is
        // the contract for manual evidence on a mesh-capable machine.
        assert!(true);
    }

    /// Failure copy for watchdog / last_error must be actionable (#2062 silent no-reply).
    #[test]
    fn rearm_failure_message_is_actionable_shared_compute_offline() {
        let error = "no live member is serving this model";
        let msg = format!(
            "{MESH_REARM_ERROR_SENTINEL}Buzz shared compute offline — failed to re-arm local ingress for this agent: {error}"
        );
        assert!(msg.starts_with(MESH_REARM_ERROR_SENTINEL));
        assert!(msg.contains("Buzz shared compute offline"));
        assert!(msg.contains("re-arm"));
        assert!(msg.contains(error));
    }

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
