use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::app_state::AppState;

const INGRESS_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const INGRESS_CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
const INGRESS_RELEASE_TIMEOUT: Duration = Duration::from_secs(3);
const STALE_STOP_TIMEOUT: Duration = Duration::from_secs(12);
const DEAD_PROBE_EVICT_THRESHOLD: u32 = 2;

/// Sentinel prefix on errors owned by this recovery path. Recovery clears only
/// these errors and never an unrelated agent failure.
pub(crate) const MESH_REARM_ERROR_SENTINEL: &str = "[buzz-mesh-rearm] ";

/// App-scoped recovery coordination. The runtime id binds a dead-probe streak
/// to one specific handle, while `rearm_lock` prevents overlapping watchdog
/// passes from starting competing replacements.
pub struct MeshRecoveryState {
    probe_runtime_id: AtomicU64,
    dead_probes: AtomicU32,
    rearm_lock: tokio::sync::Mutex<()>,
}

impl Default for MeshRecoveryState {
    fn default() -> Self {
        Self {
            probe_runtime_id: AtomicU64::new(0),
            dead_probes: AtomicU32::new(0),
            rearm_lock: tokio::sync::Mutex::new(()),
        }
    }
}

impl MeshRecoveryState {
    fn reset_probe_streak(&self) {
        self.probe_runtime_id.store(0, Ordering::Relaxed);
        self.dead_probes.store(0, Ordering::Relaxed);
    }

    fn record_dead_probe(&self, runtime_id: u64) -> u32 {
        if self.probe_runtime_id.swap(runtime_id, Ordering::Relaxed) != runtime_id {
            self.dead_probes.store(1, Ordering::Relaxed);
            return 1;
        }
        self.dead_probes.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MeshIngressProbe {
    Live,
    PortClosed,
    Unhealthy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MeshRecoveryUrgency {
    Foreground,
    Watchdog,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MeshRuntimeRecovery {
    Absent,
    Live,
    Debouncing,
    Evicted,
    ReleasePending,
    Replaced,
    RestartRequired,
}

/// Probe the contract agents actually consume. A successful `/v1/models`
/// response proves the local OpenAI ingress is accepting requests; the
/// management console on `:3131` is intentionally not part of this health
/// decision.
pub(crate) async fn probe_mesh_ingress() -> MeshIngressProbe {
    probe_mesh_ingress_at(crate::managed_agents::RELAY_MESH_API_BASE_URL).await
}

pub(crate) async fn mesh_ingress_is_live_at(api_base_url: &str) -> bool {
    probe_mesh_ingress_at(api_base_url).await == MeshIngressProbe::Live
}

pub(crate) async fn probe_mesh_ingress_at(api_base_url: &str) -> MeshIngressProbe {
    let base = api_base_url.trim_end_matches('/');
    let client = match reqwest::Client::builder()
        .timeout(INGRESS_PROBE_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(_) => return MeshIngressProbe::Unhealthy,
    };
    if client
        .get(format!("{base}/models"))
        .bearer_auth(crate::managed_agents::RELAY_MESH_API_KEY_PLACEHOLDER)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
    {
        return MeshIngressProbe::Live;
    }
    if ingress_port_is_bound(api_base_url).await {
        MeshIngressProbe::Unhealthy
    } else {
        MeshIngressProbe::PortClosed
    }
}

async fn ingress_port_is_bound(api_base_url: &str) -> bool {
    let Ok(url) = url::Url::parse(api_base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let Some(port) = url.port_or_known_default() else {
        return false;
    };
    matches!(
        tokio::time::timeout(
            INGRESS_CONNECT_TIMEOUT,
            tokio::net::TcpStream::connect((host, port)),
        )
        .await,
        Ok(Ok(_))
    )
}

async fn wait_for_ingress_release() -> bool {
    let deadline = tokio::time::Instant::now() + INGRESS_RELEASE_TIMEOUT;
    loop {
        if !ingress_port_is_bound(crate::managed_agents::RELAY_MESH_API_BASE_URL).await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn should_evict_after_probe(
    urgency: MeshRecoveryUrgency,
    probe: MeshIngressProbe,
    consecutive: u32,
) -> bool {
    urgency == MeshRecoveryUrgency::Foreground && probe == MeshIngressProbe::PortClosed
        || consecutive >= DEAD_PROBE_EVICT_THRESHOLD
}

fn requires_process_restart(
    mode: crate::mesh_llm::MeshNodeMode,
    startup_in_progress: bool,
) -> bool {
    startup_in_progress || mode == crate::mesh_llm::MeshNodeMode::Serve
}

/// Probe and, when justified, remove one stale runtime. A closed port is
/// decisive for a foreground agent start; watchdog and ambiguous/unhealthy
/// ports require consecutive failures to avoid restarting on a transient load
/// spike.
pub(crate) async fn recover_stale_mesh_runtime(
    state: &AppState,
    urgency: MeshRecoveryUrgency,
) -> MeshRuntimeRecovery {
    let (candidate_id, startup_in_progress, candidate_mode) =
        match state.mesh_llm_runtime.lock().await.as_ref() {
            Some(runtime) => (runtime.id(), runtime.is_starting().await, runtime.mode()),
            None => {
                state.mesh_recovery.reset_probe_streak();
                // A cancelled SDK startup can outlive its Buzz-side task briefly
                // because the embedded runtime runs on its own thread. Never start
                // a replacement merely because the tracked handle is gone: first
                // prove the old ingress is either still useful or has released the
                // port. This closes the port-conflict loop in #2304.
                return match probe_mesh_ingress().await {
                    MeshIngressProbe::Live => MeshRuntimeRecovery::Live,
                    MeshIngressProbe::PortClosed => MeshRuntimeRecovery::Absent,
                    MeshIngressProbe::Unhealthy => MeshRuntimeRecovery::ReleasePending,
                };
            }
        };
    let probe = probe_mesh_ingress().await;
    if probe == MeshIngressProbe::Live {
        state.mesh_recovery.reset_probe_streak();
        return MeshRuntimeRecovery::Live;
    }
    let consecutive = state.mesh_recovery.record_dead_probe(candidate_id);
    if startup_in_progress && consecutive < DEAD_PROBE_EVICT_THRESHOLD {
        eprintln!(
            "buzz-mesh: ingress is not live on the first startup probe; allowing the supervised SDK task one watchdog interval"
        );
        return MeshRuntimeRecovery::Debouncing;
    }
    if !should_evict_after_probe(urgency, probe, consecutive) {
        eprintln!(
            "buzz-mesh: ingress probe {probe:?} ({consecutive}/{DEAD_PROBE_EVICT_THRESHOLD}); debouncing"
        );
        return MeshRuntimeRecovery::Debouncing;
    }

    // Never replace a serving runtime in-process. Its native listeners and
    // model host are process-owned; stopping it here and then cold-starting a
    // client silently disables Share Compute and can race ports 9337/3131.
    // Pending client startups have the same ownership problem because the SDK
    // has not yielded a shutdown handle yet. In both cases, process restart is
    // the only boundary that preserves the configured role safely.
    if requires_process_restart(candidate_mode, startup_in_progress) {
        state.mesh_recovery.reset_probe_streak();
        return MeshRuntimeRecovery::RestartRequired;
    }

    let stale = {
        let mut guard = state.mesh_llm_runtime.lock().await;
        if guard.as_ref().map(|runtime| runtime.id()) != Some(candidate_id) {
            state.mesh_recovery.reset_probe_streak();
            return MeshRuntimeRecovery::Replaced;
        }
        guard.take()
    };
    let Some(stale) = stale else {
        return MeshRuntimeRecovery::Replaced;
    };
    state.mesh_recovery.reset_probe_streak();

    match tokio::time::timeout(STALE_STOP_TIMEOUT, stale.stop()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("buzz-mesh: stale runtime stop failed: {error:#}"),
        Err(_) => eprintln!(
            "buzz-mesh: stale runtime stop exceeded {}s; waiting for port release",
            STALE_STOP_TIMEOUT.as_secs()
        ),
    }
    if wait_for_ingress_release().await {
        MeshRuntimeRecovery::Evicted
    } else {
        eprintln!(
            "buzz-mesh: old runtime still owns the local ingress; deferring replacement to avoid a port-conflict loop"
        );
        MeshRuntimeRecovery::ReleasePending
    }
}

/// Post-launch recovery for actively running relay-mesh agents.
pub(crate) async fn rearm_relay_mesh_for_running_agents(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _rearm_guard = state.mesh_recovery.rearm_lock.lock().await;
    let runtime_mode = state
        .mesh_llm_runtime
        .lock()
        .await
        .as_ref()
        .map(|runtime| runtime.mode());
    let recovery = recover_stale_mesh_runtime(&state, MeshRecoveryUrgency::Watchdog).await;
    let active_pubkeys = active_managed_agent_pubkeys(&state);
    // Mesh participation is resolved through the same definition-authoritative
    // path as spawn/restore (#1968): definition → global fallback. A linked
    // instance's own bytes never contribute.
    let personas = crate::managed_agents::load_personas(app).unwrap_or_default();
    let global = crate::managed_agents::load_global_agent_config(app).unwrap_or_default();

    match recovery {
        MeshRuntimeRecovery::Live
        | MeshRuntimeRecovery::Debouncing
        | MeshRuntimeRecovery::Replaced => return Ok(()),
        MeshRuntimeRecovery::RestartRequired => {
            if runtime_mode == Some(crate::mesh_llm::MeshNodeMode::Serve) {
                eprintln!(
                    "buzz-mesh: serving ingress failed; restarting Buzz to restore Share Compute without changing roles"
                );
                app.request_restart();
                return Ok(());
            }
            let records = crate::managed_agents::load_managed_agents(app).unwrap_or_default();
            if !records.iter().any(|record| {
                running_relay_mesh_model_id(record, &active_pubkeys, &personas, &global).is_some()
            }) {
                // A foreground save may still be bringing up its first ingress.
                // Only an already-running consumer justifies an automatic app
                // relaunch from the background watchdog.
                return Ok(());
            }
            eprintln!(
                "buzz-mesh: supervised client startup lost its ingress before the SDK exposed a shutdown handle; restarting Buzz"
            );
            app.request_restart();
            return Ok(());
        }
        MeshRuntimeRecovery::ReleasePending => {
            return Err(format!(
                "{MESH_REARM_ERROR_SENTINEL}old local mesh ingress is still shutting down"
            ));
        }
        MeshRuntimeRecovery::Absent => {
            let records = crate::managed_agents::load_managed_agents(app).unwrap_or_default();
            if !records.iter().any(|record| {
                running_relay_mesh_model_id(record, &active_pubkeys, &personas, &global).is_some()
            }) {
                return Ok(());
            }
        }
        MeshRuntimeRecovery::Evicted => {}
    }

    let records = crate::managed_agents::load_managed_agents(app).unwrap_or_default();
    let mesh_records: Vec<_> = records
        .into_iter()
        .filter_map(|record| {
            running_relay_mesh_model_id(&record, &active_pubkeys, &personas, &global)
                .map(|mesh_model_id| (record, mesh_model_id))
        })
        .collect();
    let mut first_error = None;
    for (record, mesh_model_id) in &mesh_records {
        match crate::commands::mesh_llm::ensure_relay_mesh_for_record(
            app,
            Some(mesh_model_id.as_str()),
            false,
        )
        .await
        {
            Ok(()) => {
                if let Err(error) = clear_mesh_last_error_if_set(app, &record.pubkey) {
                    eprintln!("buzz-mesh: failed to clear recovery error: {error}");
                }
            }
            Err(error) => {
                let message = format!(
                    "{MESH_REARM_ERROR_SENTINEL}Buzz shared compute offline — failed to re-arm local ingress for this agent: {error}"
                );
                if let Err(persist_error) = persist_mesh_last_error(app, &record.pubkey, &message) {
                    eprintln!("buzz-mesh: failed to persist recovery error: {persist_error}");
                }
                first_error.get_or_insert(message);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn active_managed_agent_pubkeys(state: &AppState) -> HashSet<String> {
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

/// Effective mesh model for a record that is actively running, or `None`
/// when the record is not a running relay-mesh consumer. Resolution goes
/// through `resolve_effective_relay_mesh_model_id` (definition → global
/// fallback, #1968) so the watchdog agrees with spawn/restore about which
/// agents are mesh-backed.
fn running_relay_mesh_model_id(
    record: &crate::managed_agents::ManagedAgentRecord,
    active_pubkeys: &HashSet<String>,
    personas: &[crate::managed_agents::AgentDefinition],
    global: &crate::managed_agents::GlobalAgentConfig,
) -> Option<String> {
    let running = record.backend == crate::managed_agents::BackendKind::Local
        && active_pubkeys.contains(&record.pubkey.to_ascii_lowercase())
        && record
            .runtime_pid
            .is_none_or(crate::managed_agents::process_is_running);
    if !running {
        return None;
    }
    crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
        record, personas, global,
    )
}

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

fn clear_mesh_last_error_if_set(app: &AppHandle, pubkey: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| format!("failed to acquire managed agents store lock: {e}"))?;
    let mut records = crate::managed_agents::load_managed_agents(app)?;
    let record = crate::managed_agents::find_managed_agent_mut(&mut records, pubkey)?;
    if !record
        .last_error
        .as_deref()
        .is_some_and(|error| error.starts_with(MESH_REARM_ERROR_SENTINEL))
    {
        return Ok(());
    }
    record.last_error = None;
    record.updated_at = crate::util::now_iso();
    crate::managed_agents::save_managed_agents(app, &records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_record(
        pubkey: &str,
        runtime_pid: Option<u32>,
    ) -> crate::managed_agents::ManagedAgentRecord {
        let mut record = crate::managed_agents::AgentDefinition {
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
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
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
        record.pubkey = pubkey.to_string();
        record.backend = crate::managed_agents::BackendKind::Local;
        record.runtime_pid = runtime_pid;
        record
    }

    fn active_set(pubkeys: &[&str]) -> HashSet<String> {
        pubkeys
            .iter()
            .map(|pubkey| pubkey.to_ascii_lowercase())
            .collect()
    }

    #[tokio::test]
    async fn closed_port_is_distinct_from_unhealthy_bound_port() {
        assert_eq!(
            probe_mesh_ingress_at("http://127.0.0.1:1/v1").await,
            MeshIngressProbe::PortClosed
        );
    }

    #[test]
    fn foreground_closed_port_evicts_without_debounce() {
        assert!(should_evict_after_probe(
            MeshRecoveryUrgency::Foreground,
            MeshIngressProbe::PortClosed,
            1
        ));
        assert!(!should_evict_after_probe(
            MeshRecoveryUrgency::Watchdog,
            MeshIngressProbe::PortClosed,
            1
        ));
    }

    #[test]
    fn probe_streak_is_scoped_to_runtime_identity() {
        let state = MeshRecoveryState::default();
        assert_eq!(state.record_dead_probe(7), 1);
        assert_eq!(state.record_dead_probe(7), 2);
        assert_eq!(state.record_dead_probe(8), 1);
    }

    #[test]
    fn explicit_stop_budget_accommodates_sdk_forced_shutdown() {
        assert!(STALE_STOP_TIMEOUT >= Duration::from_secs(10));
        assert!(STALE_STOP_TIMEOUT <= Duration::from_secs(15));
    }

    #[test]
    fn failed_serving_runtime_requires_process_restart_instead_of_client_fallback() {
        assert!(requires_process_restart(
            crate::mesh_llm::MeshNodeMode::Serve,
            false
        ));
        assert!(requires_process_restart(
            crate::mesh_llm::MeshNodeMode::Client,
            true
        ));
        assert!(!requires_process_restart(
            crate::mesh_llm::MeshNodeMode::Client,
            false
        ));
    }

    #[test]
    fn only_running_relay_mesh_agents_trigger_rearm() {
        let personas: Vec<crate::managed_agents::AgentDefinition> = Vec::new();
        let global = crate::managed_agents::GlobalAgentConfig::default();

        let empty = active_set(&[]);
        assert!(running_relay_mesh_model_id(
            &mesh_record("stopped", Some(std::process::id())),
            &empty,
            &personas,
            &global,
        )
        .is_none());

        let active = active_set(&["live"]);
        assert_eq!(
            running_relay_mesh_model_id(
                &mesh_record("live", Some(std::process::id())),
                &active,
                &personas,
                &global,
            )
            .as_deref(),
            Some("Qwen3")
        );
        assert_eq!(
            running_relay_mesh_model_id(&mesh_record("live", None), &active, &personas, &global)
                .as_deref(),
            Some("Qwen3")
        );

        let mut non_mesh = mesh_record("plain", Some(std::process::id()));
        non_mesh.env_vars.clear();
        non_mesh.relay_mesh = None;
        assert!(running_relay_mesh_model_id(
            &non_mesh,
            &active_set(&["plain"]),
            &personas,
            &global,
        )
        .is_none());
    }

    #[test]
    fn recovery_error_sentinel_does_not_match_unrelated_errors() {
        assert!(
            format!("{MESH_REARM_ERROR_SENTINEL}offline").starts_with(MESH_REARM_ERROR_SENTINEL)
        );
        assert!(!"user note: shared compute config".starts_with(MESH_REARM_ERROR_SENTINEL));
    }
}
