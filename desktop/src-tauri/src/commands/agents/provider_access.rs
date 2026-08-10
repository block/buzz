//! Upgrade reconciliation for provider-backed managed-agent access.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, OnceLock},
};

use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, discover_provider_candidates, load_managed_agents,
        load_personas, provider_deploy, resolve_provider_binary, save_managed_agents, BackendKind,
        ManagedAgentRecord, ManagedAgentSummary,
    },
    util::now_iso,
};

type DeployLocks = StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;

/// Per-agent provider-deploy serialization: at most one provider call may be
/// in flight per agent pubkey, so two deploys for the same agent can never
/// interleave and land out of order remotely. A module-level static rather
/// than an `AppState` field because app_state.rs is at its file-size ratchet
/// limit and may not grow, and because the map holds no community-scoped data
/// (keys are agent pubkeys, values are bare mutexes), so it needs no
/// community-switch reset.
///
/// Entries are deliberately never removed — not even on agent deletion.
/// Removing an entry while a deploy still holds its `Arc` would let a
/// recreated agent with the same pubkey mint a fresh mutex and run
/// concurrently with the in-flight holder, silently breaking serialization.
/// Growth is bounded by the number of distinct agent pubkeys seen in one app
/// run, and each entry is a few dozen bytes.
static PROVIDER_DEPLOY_LOCKS: OnceLock<DeployLocks> = OnceLock::new();

fn agent_deploy_lock(pubkey: &str) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let locks = PROVIDER_DEPLOY_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks.lock().map_err(|error| error.to_string())?;
    Ok(locks.entry(pubkey.to_string()).or_default().clone())
}

pub(super) fn needs_reconciliation_with_policy(
    record: &ManagedAgentRecord,
    owner_only_access: bool,
) -> bool {
    owner_only_access && record.backend != BackendKind::Local && record.backend_agent_id.is_some()
}

/// A provider deploy captured under the store lock: the coordinates the
/// provider call will use plus the record generation (`updated_at`) the
/// completion is bound to. Every deploy path (create, start, reconcile,
/// redeploy) snapshots one of these under the per-agent deploy lock before
/// releasing the store lock for the long-running provider call.
#[derive(Debug)]
pub(super) struct CapturedDeploy {
    pub(super) provider_id: String,
    pub(super) config: serde_json::Value,
    pub(super) cached_binary_path: Option<String>,
    pub(super) updated_at: String,
}

/// Capture the provider coordinates and configuration generation of `record`
/// for a deploy. Errors when the record is not provider-backed.
fn capture_provider_deploy(record: &ManagedAgentRecord) -> Result<CapturedDeploy, String> {
    match &record.backend {
        BackendKind::Provider { id, config } => Ok(CapturedDeploy {
            provider_id: id.clone(),
            config: config.clone(),
            cached_binary_path: record.provider_binary_path.clone(),
            updated_at: record.updated_at.clone(),
        }),
        BackendKind::Local => Err(format!(
            "agent {} is not provider-backed and cannot be deployed to a provider",
            record.pubkey
        )),
    }
}

fn collect_reconcile_pubkeys(
    records: Vec<ManagedAgentRecord>,
    owner_only_access: bool,
) -> Vec<String> {
    records
        .into_iter()
        .filter(|record| needs_reconciliation_with_policy(record, owner_only_access))
        .map(|record| record.pubkey)
        .collect()
}

/// Redeploy every existing provider agent in an owner-only access build.
///
/// The saved `backend_agent_id` only proves that some provider deployment
/// exists. A marked build sends the current owner-only payload before each
/// community UI load. Workspace apply fails closed if any provider rejects it.
/// Only the pubkeys are collected here — each deploy snapshots its own
/// payload and generation under the per-agent deploy lock, so a redeploy or
/// edit racing this loop can never make an older payload land after a newer
/// one.
pub(crate) async fn reconcile_on_workspace_apply(
    app: &AppHandle,
    state: &AppState,
) -> Result<(), String> {
    if !crate::managed_agents::owner_only_access_build() {
        return Ok(());
    }

    let pubkeys = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        collect_reconcile_pubkeys(load_managed_agents(app)?, true)
    };

    for pubkey in pubkeys {
        if let Err(error) = deploy_to_provider(app, state, &pubkey).await {
            return Err(format!(
                "provider access reconciliation failed for agent {pubkey}: {error}"
            ));
        }
    }

    Ok(())
}

/// Resolve the provider binary and run the blocking deploy call off the async
/// runtime. The outer `Err` is an infrastructure failure (binary resolution or
/// `spawn_blocking`) that never reached the provider; the inner result is the
/// provider's own answer — `Ok(backend_agent_id)` or the provider error.
async fn call_provider_deploy(
    captured: &CapturedDeploy,
    agent_json: serde_json::Value,
) -> Result<Result<String, String>, String> {
    let provider_id = &captured.provider_id;
    // Resolve via discovered candidates only. Cached path must match BOTH
    // "is a discovered candidate" AND "belongs to this provider_id". A tampered
    // record cannot redirect deploys to a different provider's binary.
    let bin_path = captured
        .cached_binary_path
        .as_deref()
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .map(|p| p.canonicalize().unwrap_or(p))
        .filter(|canonical| {
            discover_provider_candidates().iter().any(|(id, cp)| {
                id == provider_id && cp.canonicalize().ok().as_ref() == Some(canonical)
            })
        })
        .map_or_else(|| resolve_provider_binary(provider_id), Ok)?;

    let config_clone = captured.config.clone();
    tokio::task::spawn_blocking(move || provider_deploy(&bin_path, &agent_json, &config_clone))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Deploy an agent to a provider backend. Snapshots the record under the
/// per-agent deploy lock, resolves the binary, calls deploy via
/// spawn_blocking, and persists the result (backend_agent_id or last_error).
///
/// Idempotency: calling deploy on an already-deployed agent sends the same payload
/// again. Providers are expected to handle this as an update-in-place or no-op —
/// the protocol does not include an explicit `undeploy` operation (deferred to v2).
///
/// Serialization and completion binding are shared with the redeploy command
/// via [`run_deploy`]: the payload and generation are snapshotted only after
/// this call holds the agent's deploy lock (so a queued deploy always sends
/// the latest saved configuration), and the result is bound through
/// [`bind_completion`] — success is only reported as applied when the record
/// still exists, still points at the captured provider, and was not edited
/// while the call was in flight. Otherwise the honest [`CompletionOutcome`]
/// is surfaced as the `Err` — never a claim that the current record was
/// applied.
pub(super) async fn deploy_to_provider(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
) -> Result<ManagedAgentSummary, String> {
    let mut effects = CommandDeployEffects {
        app,
        state,
        target: deploy_provider_target,
    };
    run_deploy(pubkey, &mut effects).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(backend: BackendKind, backend_agent_id: Option<&str>) -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "agent", "name": "Agent", "relay_url": "", "acp_command": "",
            "agent_command": "", "agent_args": [], "mcp_command": "",
            "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
            "updated_at": "", "last_started_at": null, "last_stopped_at": null,
            "last_exit_code": null, "last_error": null
        }))
        .unwrap();
        record.backend = backend;
        record.backend_agent_id = backend_agent_id.map(str::to_string);
        record
    }

    #[test]
    fn upgrade_collects_only_existing_provider_deployments() {
        let records = vec![
            record(
                BackendKind::Provider {
                    id: "provider".into(),
                    config: serde_json::json!({"region": "test"}),
                },
                Some("existing"),
            ),
            record(
                BackendKind::Provider {
                    id: "not-deployed".into(),
                    config: serde_json::json!({}),
                },
                None,
            ),
            record(BackendKind::Local, Some("stale")),
        ];

        assert_eq!(collect_reconcile_pubkeys(records, true), vec!["agent"]);
    }

    #[test]
    fn unmarked_build_collects_no_upgrade_targets() {
        let records = vec![record(
            BackendKind::Provider {
                id: "provider".into(),
                config: serde_json::json!({}),
            },
            Some("existing"),
        )];

        assert!(collect_reconcile_pubkeys(records, false).is_empty());
    }
}

pub(super) fn redeploy_provider_target(
    pubkey: &str,
    record: &ManagedAgentRecord,
) -> Result<CapturedDeploy, String> {
    match (&record.backend, record.backend_agent_id.as_deref()) {
        (BackendKind::Provider { .. }, Some(_)) => capture_provider_deploy(record),
        (BackendKind::Provider { .. }, None) => Err(format!(
            "agent {pubkey} has not been deployed yet; deploy it before redeploying"
        )),
        _ => Err(format!(
            "agent {pubkey} is not provider-backed and cannot be redeployed"
        )),
    }
}

/// Everything a deploy flow captures under the store lock before the
/// provider call. `captured.updated_at` is the configuration generation the
/// completion is bound to.
pub(super) struct DeploySnapshot {
    pub(super) record: ManagedAgentRecord,
    pub(super) captured: CapturedDeploy,
}

/// How a finished provider call was bound to the store state found on
/// completion. Computed by [`bind_completion`].
#[derive(Debug)]
pub(super) enum CompletionOutcome {
    /// Record unchanged since the snapshot: success persisted as current.
    AppliedCurrent(Box<ManagedAgentRecord>),
    /// Record edited mid-flight: the remote facts are persisted
    /// (`backend_agent_id`, `last_started_at`) because the deploy really
    /// happened, but the running deployment carries the previously saved
    /// configuration, so nothing may imply the current record was applied —
    /// `last_error` is left untouched.
    AppliedStale,
    /// Record switched away from the captured provider backend mid-flight
    /// (to local, or to a different provider): nothing persisted — the
    /// deployment id belongs to a backend the record no longer describes.
    BackendChanged,
    /// Record deleted mid-flight: nothing persisted.
    Deleted,
    /// Provider call failed: the failure is stamped on the surviving record.
    Failed(String),
}

/// Bind a finished provider call to whatever the store holds now. Mutates
/// `records` in place; the returned bool says whether the caller must save.
/// A successful deploy is only stamped onto a record that is still backed by
/// the captured provider; otherwise the id would describe a foreign backend.
pub(super) fn bind_completion(
    records: &mut [ManagedAgentRecord],
    pubkey: &str,
    captured_provider_id: &str,
    captured_updated_at: &str,
    deploy_result: &Result<String, String>,
) -> (bool, CompletionOutcome) {
    let Some(record) = records.iter_mut().find(|record| record.pubkey == pubkey) else {
        return (false, CompletionOutcome::Deleted);
    };
    match deploy_result {
        Err(error) => {
            record.last_error = Some(error.clone());
            record.updated_at = now_iso();
            (true, CompletionOutcome::Failed(error.clone()))
        }
        Ok(backend_agent_id) => {
            let same_backend = matches!(
                &record.backend,
                BackendKind::Provider { id, .. } if id == captured_provider_id
            );
            if !same_backend {
                return (false, CompletionOutcome::BackendChanged);
            }
            let unchanged = record.updated_at == captured_updated_at;
            record.backend_agent_id = Some(backend_agent_id.clone());
            record.last_started_at = Some(now_iso());
            record.updated_at = now_iso();
            if unchanged {
                record.last_error = None;
                (
                    true,
                    CompletionOutcome::AppliedCurrent(Box::new(record.clone())),
                )
            } else {
                (true, CompletionOutcome::AppliedStale)
            }
        }
    }
}

/// Fold a bound completion into the command result: the applied record on
/// success-as-current, a user-facing error otherwise. Shared by the redeploy
/// command and [`deploy_to_provider`] (create/start/reconcile) so every
/// deploy path surfaces the same honest outcome. Error strings reach the
/// owner verbatim in a toast, so they are short sentences without pubkeys.
pub(super) fn completion_result(
    outcome: CompletionOutcome,
    deploy_result: Result<String, String>,
) -> Result<Box<ManagedAgentRecord>, String> {
    match outcome {
        CompletionOutcome::AppliedCurrent(record) => Ok(record),
        CompletionOutcome::AppliedStale => Err("Agent settings changed while the deploy was in \
             flight, so the agent is running the previous configuration. Redeploy to apply the \
             latest settings — a queued redeploy will do that automatically."
            .to_string()),
        CompletionOutcome::BackendChanged => Err("The agent's backend changed while the deploy \
             was in flight. The finished deployment may still be running on the previous \
             provider backend."
            .to_string()),
        CompletionOutcome::Deleted => Err(match deploy_result {
            Ok(_) => "The agent was deleted while the deploy was in flight. The finished \
                 deployment may still be running on the provider."
                .to_string(),
            Err(error) => error,
        }),
        CompletionOutcome::Failed(error) => Err(error),
    }
}

/// Effects a deploy orchestration runs against. The real commands wire
/// these to the on-disk store and the provider binary; tests wire an
/// in-memory store and a gated fake provider so mid-flight edits and
/// deletions can be driven deterministically.
pub(super) trait DeployEffects {
    type Success;

    /// Under the store lock: load the record and validate it as a deploy
    /// target.
    fn snapshot(&mut self, pubkey: &str) -> Result<DeploySnapshot, String>;

    /// The provider call (payload build + deploy). Must not touch the store.
    async fn deploy(&mut self, snapshot: &DeploySnapshot) -> Result<String, String>;

    /// Run `apply` on the loaded records under the store lock, saving when it
    /// returns `true`.
    fn with_store<R>(
        &mut self,
        apply: &mut dyn FnMut(&mut Vec<ManagedAgentRecord>) -> (bool, R),
    ) -> Result<R, String>;

    /// Build the success value from the freshly persisted record.
    fn success(&mut self, record: &ManagedAgentRecord) -> Result<Self::Success, String>;
}

/// The deploy orchestration shared by every provider-deploy path (create,
/// start, reconcile, redeploy): per-agent serialization, a snapshot taken
/// only after the deploy lock is held, the provider call, and a completion
/// bound to the captured configuration generation. All decision logic lives
/// here and in [`bind_completion`]; the commands and the tests differ only in
/// the [`DeployEffects`] they plug in.
pub(super) async fn run_deploy<E: DeployEffects>(
    pubkey: &str,
    effects: &mut E,
) -> Result<E::Success, String> {
    let deploy_lock = agent_deploy_lock(pubkey)?;
    let _deploy_guard = deploy_lock.lock().await;

    let snapshot = effects.snapshot(pubkey)?;

    let deploy_result = effects.deploy(&snapshot).await;

    let outcome = effects.with_store(&mut |records| {
        bind_completion(
            records,
            pubkey,
            &snapshot.captured.provider_id,
            &snapshot.captured.updated_at,
            &deploy_result,
        )
    })?;

    let record = completion_result(outcome, deploy_result)?;
    effects.success(&record)
}

/// Validate `record` as a plain deploy target (create/start/reconcile): any
/// provider-backed record qualifies, deployed before or not.
fn deploy_provider_target(
    _pubkey: &str,
    record: &ManagedAgentRecord,
) -> Result<CapturedDeploy, String> {
    capture_provider_deploy(record)
}

/// [`DeployEffects`] wired to the real store, payload builder, and provider
/// binary. `target` validates the record under the store lock —
/// [`deploy_provider_target`] for first-time deploys,
/// [`redeploy_provider_target`] for the redeploy command, which additionally
/// requires an existing deployment.
struct CommandDeployEffects<'a> {
    app: &'a AppHandle,
    state: &'a AppState,
    target: fn(&str, &ManagedAgentRecord) -> Result<CapturedDeploy, String>,
}

impl DeployEffects for CommandDeployEffects<'_> {
    type Success = ManagedAgentSummary;

    fn snapshot(&mut self, pubkey: &str) -> Result<DeploySnapshot, String> {
        let _store_guard = self
            .state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(self.app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        let captured = (self.target)(pubkey, record)?;
        Ok(DeploySnapshot {
            record: record.clone(),
            captured,
        })
    }

    async fn deploy(&mut self, snapshot: &DeploySnapshot) -> Result<String, String> {
        let agent_json = super::build_deploy_payload(self.app, self.state, &snapshot.record)?;
        call_provider_deploy(&snapshot.captured, agent_json).await?
    }

    fn with_store<R>(
        &mut self,
        apply: &mut dyn FnMut(&mut Vec<ManagedAgentRecord>) -> (bool, R),
    ) -> Result<R, String> {
        let _store_guard = self
            .state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(self.app)?;
        let (save, result) = apply(&mut records);
        if save {
            save_managed_agents(self.app, &records)?;
        }
        Ok(result)
    }

    fn success(&mut self, record: &ManagedAgentRecord) -> Result<ManagedAgentSummary, String> {
        let runtimes = self
            .state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        let personas = load_personas(self.app).unwrap_or_default();
        build_managed_agent_summary(
            self.app,
            record,
            &runtimes,
            &personas,
            &crate::managed_agents::load_global_agent_config(self.app).unwrap_or_default(),
        )
    }
}

#[tauri::command]
pub async fn redeploy_managed_agent(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ManagedAgentSummary, String> {
    let mut effects = CommandDeployEffects {
        app: &app,
        state: &state,
        target: redeploy_provider_target,
    };
    run_deploy(&pubkey, &mut effects).await
}
