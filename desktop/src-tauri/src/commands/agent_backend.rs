//! Backend swap for an existing managed agent (local ↔ execution node, and
//! the legacy provider path where the dev flag surfaces it).
//!
//! A backend change is a lifecycle transition, not a field write: the old
//! body is torn down first — the local process is stopped, or the old node's
//! workload is removed with a confirmed receipt through the same converging
//! cleanup `delete_managed_agent` uses (`WorkloadNotFound` converges to
//! success; an unreachable node fails the swap instead of leaving a zombie
//! workload) — and only then is the new backend persisted. Execution-node
//! targets are deployed afterwards by the frontend through the existing
//! authoritative `deploy_managed_agent_to_execution_node`, which waits for
//! the receipt and sets `backend_agent_id` — mirroring the create-then-deploy
//! shape in `createAndDeployExecutionNodeAgent.ts`.

use tauri::{AppHandle, State};

use super::execution_nodes::{
    managed_agent_execution_target, remove_execution_workload_for_managed_agent,
};
use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, current_instance_id, find_managed_agent_mut,
        load_global_agent_config, load_managed_agents, load_personas, resolve_provider_binary,
        save_managed_agents, stop_managed_agent_process, sync_managed_agent_processes,
        validate_provider_config, BackendKind, ChangeManagedAgentBackendRequest,
        ManagedAgentRecord, ManagedAgentSummary,
    },
    util::now_iso,
};

/// The side effects a validated backend swap must perform, resolved before
/// any of them run so a guard failure can never half-apply a transition.
#[derive(Debug)]
pub(crate) struct PlannedBackendTransition {
    /// The backend the plan was computed against. Re-checked before persist:
    /// the store lock is released across the remote-teardown await, and a
    /// record another call already moved must not be overwritten.
    pub(crate) previous_backend: BackendKind,
    /// Leave-local teardown: stop every runtime pair before flipping.
    pub(crate) stop_local: bool,
    /// Leave-node teardown: `(node_id, workload_id)` to remove, receipt-confirmed.
    pub(crate) remove_execution_workload: Option<(String, String)>,
    /// Runtime id to persist for execution-node targets (deploy requires one).
    pub(crate) persist_runtime: Option<String>,
}

/// Pure decision core of the backend swap — guards and teardown planning,
/// no IO. `Ok(None)` means the requested backend equals the stored one and
/// the command is a no-op.
pub(crate) fn plan_backend_transition(
    record: &ManagedAgentRecord,
    new_backend: &BackendKind,
    requested_runtime: Option<&str>,
    relay_mesh_model: Option<&str>,
    force: bool,
) -> Result<Option<PlannedBackendTransition>, String> {
    if record.backend == *new_backend {
        return Ok(None);
    }

    // Mesh guard: relay-mesh agents dial a locally served model, so their
    // body must stay on this machine. Same message as create's
    // `normalize_relay_mesh`. Resolved from the EFFECTIVE config by the
    // caller so a persona-inherited relay-mesh provider is caught too.
    if *new_backend != BackendKind::Local && relay_mesh_model.is_some() {
        return Err("Buzz shared compute agents must use the local backend".to_string());
    }

    let persist_runtime = match new_backend {
        BackendKind::ExecutionNode { node_id } => {
            if node_id.trim().is_empty() {
                return Err("execution node id is required".to_string());
            }
            // The remote deploy payload requires a persisted runtime id
            // (`WorkloadSpec::agent`); a local-born agent usually has none.
            let runtime = requested_runtime
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    record
                        .runtime
                        .clone()
                        .filter(|value| !value.trim().is_empty())
                });
            Some(runtime.ok_or_else(|| {
                "select a runtime for this agent before moving it to an execution node".to_string()
            })?)
        }
        _ => None,
    };

    let (stop_local, remove_execution_workload) = match &record.backend {
        BackendKind::Local => (true, None),
        BackendKind::ExecutionNode { .. } => (false, managed_agent_execution_target(record)?),
        BackendKind::Provider { .. } => {
            // The provider protocol has no undeploy (deferred to v2), so a
            // deployed body can only be orphaned. Make that a deliberate,
            // confirmed choice — same invariant as delete's
            // `force_remote_delete`.
            if record.backend_agent_id.is_some() && !force {
                return Err(
                    "cannot change the backend of a deployed provider agent without force: true"
                        .to_string(),
                );
            }
            (false, None)
        }
    };

    Ok(Some(PlannedBackendTransition {
        previous_backend: record.backend.clone(),
        stop_local,
        remove_execution_workload,
        persist_runtime,
    }))
}

/// Move a managed agent to a different backend.
///
/// Teardown-then-adopt: the old body is stopped/removed before the new
/// backend is persisted, so a failure leaves the record on its previous
/// backend with the old body either still present (remote teardown failed)
/// or cleanly stopped. The command never deploys the new body itself —
/// execution-node targets are deployed by the frontend via the existing
/// authoritative `deploy_managed_agent_to_execution_node`, provider targets
/// via the normal Deploy action, and local targets start on demand.
#[tauri::command]
pub async fn change_managed_agent_backend(
    input: ChangeManagedAgentBackendRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ManagedAgentSummary, String> {
    // Validate a provider target BEFORE any side effects (mirrors the create
    // command's Pre-Phase 2 check).
    if let BackendKind::Provider { ref config, ref id } = input.backend {
        validate_provider_config(config)?;
        resolve_provider_binary(id)?;
    }

    // Serialize with every other remote execution transition (deploy, delete,
    // workload lifecycle) so two swaps cannot interleave their teardown and
    // persist phases.
    let _execution_guard = state.managed_agent_execution_transition.lock().await;

    // ── Phase 1: snapshot + plan (sync, under store lock) ────────────────────
    let plan = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == input.pubkey)
            .ok_or_else(|| format!("agent {} not found", input.pubkey))?;
        let personas = load_personas(&app).unwrap_or_default();
        let global = load_global_agent_config(&app).unwrap_or_default();
        let relay_mesh_model =
            crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
                record, &personas, &global,
            );
        plan_backend_transition(
            record,
            &input.backend,
            input.runtime.as_deref(),
            relay_mesh_model.as_deref(),
            input.force,
        )?
    };

    // ── Phase 2: remote teardown (async, store lock released) ────────────────
    // A node that cannot confirm the removal fails the swap here, before any
    // local mutation — never leave a zombie workload behind a record that
    // claims a different backend. `WorkloadNotFound` converges to success.
    if let Some(plan) = &plan {
        if let Some((node_id, workload_id)) = &plan.remove_execution_workload {
            remove_execution_workload_for_managed_agent(&state, node_id, workload_id).await?;
        }
    }

    // ── Phase 3: local teardown + persist (sync, under store lock) ───────────
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
        state.clear_agent_session_caches(pubkey);
    }

    if let Some(plan) = plan {
        let record = find_managed_agent_mut(&mut records, &input.pubkey)?;
        // The store lock was released across the teardown await; refuse to
        // persist over a record another call already moved.
        if record.backend != plan.previous_backend {
            return Err(
                "the agent's backend changed while the swap was in progress; retry".to_string(),
            );
        }
        if plan.stop_local {
            // Drains every runtime pair (all communities) — the body must be
            // fully down before the record claims a remote backend.
            stop_managed_agent_process(&app, record, &mut runtimes)?;
        }
        record.backend = input.backend.clone();
        record.backend_agent_id = None;
        record.provider_binary_path = match &input.backend {
            BackendKind::Provider { id, .. } => resolve_provider_binary(id)
                .ok()
                .map(|path| path.display().to_string()),
            _ => None,
        };
        if let Some(runtime) = plan.persist_runtime {
            record.runtime = Some(runtime);
        }
        // Remote agents are managed externally and never auto-start with the
        // desktop (mirrors create).
        if input.backend != BackendKind::Local {
            record.start_on_app_launch = false;
        }
        record.last_error = None;
        record.last_error_code = None;
        record.updated_at = now_iso();
        save_managed_agents(&app, &records)?;
        if let Some(saved) = records.iter().find(|record| record.pubkey == input.pubkey) {
            super::agents::retain_managed_agent_pending(&app, &state, saved);
        }
    }

    let record = records
        .iter()
        .find(|record| record.pubkey == input.pubkey)
        .ok_or_else(|| format!("agent {} not found", input.pubkey))?;
    let personas = load_personas(&app).unwrap_or_default();
    build_managed_agent_summary(
        &app,
        record,
        &runtimes,
        &personas,
        &load_global_agent_config(&app).unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> ManagedAgentRecord {
        serde_json::from_str(&format!(
            r#"{{
                "pubkey": "{}",
                "name": "test-agent",
                "private_key_nsec": "nsec1fake",
                "relay_url": "wss://localhost:3000",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_started_at": null,
                "last_stopped_at": null,
                "last_exit_code": null,
                "last_error": null
            }}"#,
            "a".repeat(64)
        ))
        .expect("sample record")
    }

    fn node_backend(id: &str) -> BackendKind {
        BackendKind::ExecutionNode {
            node_id: id.to_string(),
        }
    }

    #[test]
    fn same_backend_is_a_no_op() {
        let mut record = sample_record();
        assert!(
            plan_backend_transition(&record, &BackendKind::Local, None, None, false)
                .unwrap()
                .is_none()
        );

        record.backend = node_backend("node-1");
        assert!(
            plan_backend_transition(&record, &node_backend("node-1"), None, None, false)
                .unwrap()
                .is_none(),
            "same-node selection must not tear anything down"
        );
    }

    #[test]
    fn relay_mesh_agents_must_stay_local() {
        let record = sample_record();
        let err = plan_backend_transition(
            &record,
            &node_backend("node-1"),
            Some("goose"),
            Some("Qwen3"),
            false,
        )
        .unwrap_err();
        assert!(err.contains("local backend"), "{err}");
    }

    #[test]
    fn local_to_node_stops_local_and_persists_the_requested_runtime() {
        let record = sample_record();
        let plan =
            plan_backend_transition(&record, &node_backend("node-1"), Some("goose"), None, false)
                .unwrap()
                .expect("swap plan");
        assert!(plan.stop_local);
        assert_eq!(plan.remove_execution_workload, None);
        assert_eq!(plan.persist_runtime.as_deref(), Some("goose"));
        assert_eq!(plan.previous_backend, BackendKind::Local);
    }

    #[test]
    fn node_target_without_any_runtime_is_refused() {
        // A local-born agent has no persisted runtime, and the remote deploy
        // payload requires one — refuse with an actionable error instead of
        // persisting a backend that can never deploy.
        let record = sample_record();
        let err = plan_backend_transition(&record, &node_backend("node-1"), None, None, false)
            .unwrap_err();
        assert!(err.contains("runtime"), "{err}");

        // A blank requested runtime falls back to the record's stored one.
        let mut record = sample_record();
        record.runtime = Some("claude".to_string());
        let plan =
            plan_backend_transition(&record, &node_backend("node-1"), Some("  "), None, false)
                .unwrap()
                .expect("swap plan");
        assert_eq!(plan.persist_runtime.as_deref(), Some("claude"));
    }

    /// A stored workload id must be a UUID (`WorkloadId::new` validates), so
    /// the fixture uses one — matching what a real deploy persists.
    const STORED_WORKLOAD_ID: &str = "0f9f5698-3a02-4c8e-9c9f-8f9e35b3a001";

    #[test]
    fn node_to_local_removes_the_stored_workload_first() {
        let mut record = sample_record();
        record.backend = node_backend("node-1");
        record.backend_agent_id = Some(STORED_WORKLOAD_ID.to_string());
        let plan = plan_backend_transition(&record, &BackendKind::Local, None, None, false)
            .unwrap()
            .expect("swap plan");
        assert!(!plan.stop_local);
        assert_eq!(
            plan.remove_execution_workload,
            Some(("node-1".to_string(), STORED_WORKLOAD_ID.to_string()))
        );
        assert_eq!(plan.persist_runtime, None);
    }

    #[test]
    fn node_to_node_removes_from_the_old_node_and_keeps_the_stored_runtime() {
        let mut record = sample_record();
        record.backend = node_backend("node-1");
        record.backend_agent_id = Some(STORED_WORKLOAD_ID.to_string());
        record.runtime = Some("goose".to_string());
        let plan = plan_backend_transition(&record, &node_backend("node-2"), None, None, false)
            .unwrap()
            .expect("swap plan");
        assert_eq!(
            plan.remove_execution_workload,
            Some(("node-1".to_string(), STORED_WORKLOAD_ID.to_string()))
        );
        assert_eq!(plan.persist_runtime.as_deref(), Some("goose"));
    }

    #[test]
    fn node_without_stored_workload_id_still_converges_on_the_stable_identity() {
        // Deploy is a remote side effect followed by a local projection, so a
        // save failure can leave `backend_agent_id` empty even though the node
        // accepted the workload. The swap must still address the same
        // workload — same stable fallback delete uses.
        let mut record = sample_record();
        record.backend = node_backend("node-1");
        let plan = plan_backend_transition(&record, &BackendKind::Local, None, None, false)
            .unwrap()
            .expect("swap plan");
        let (node_id, workload_id) = plan
            .remove_execution_workload
            .expect("stable workload target");
        assert_eq!(node_id, "node-1");
        assert!(!workload_id.is_empty());
    }

    #[test]
    fn deployed_provider_source_requires_force() {
        let mut record = sample_record();
        record.backend = BackendKind::Provider {
            id: "blox".to_string(),
            config: serde_json::json!({}),
        };
        record.backend_agent_id = Some("remote-1".to_string());

        let err =
            plan_backend_transition(&record, &BackendKind::Local, None, None, false).unwrap_err();
        assert!(err.contains("force"), "{err}");

        let plan = plan_backend_transition(&record, &BackendKind::Local, None, None, true)
            .unwrap()
            .expect("forced swap plan");
        assert!(!plan.stop_local);
        assert_eq!(plan.remove_execution_workload, None);
    }

    #[test]
    fn undeployed_provider_source_needs_no_force() {
        let mut record = sample_record();
        record.backend = BackendKind::Provider {
            id: "blox".to_string(),
            config: serde_json::json!({}),
        };
        assert!(
            plan_backend_transition(&record, &BackendKind::Local, None, None, false)
                .unwrap()
                .is_some()
        );
    }
}
