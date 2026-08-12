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

pub(super) mod deploy_receipt;
pub(super) mod payload_digest;

use deploy_receipt::AmbiguousDeployReceipt;
use payload_digest::{deploy_input_digest, digest_parts};

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
/// provider call will use, the record generation (`updated_at`), and the
/// digest of the fully resolved deploy input the completion is bound to.
/// Every deploy path (create, start, reconcile, redeploy) snapshots one of
/// these under the per-agent deploy lock before releasing the store lock for
/// the long-running provider call.
///
/// Two bindings, not one, and deliberately conservative — a completion is only
/// reported as current when *both* hold:
///
/// * `updated_at` catches an owner edit to the agent row even when the edited
///   field does not reach the provider, so a deploy is never reported as
///   having applied an edit it did not carry.
/// * `payload_digest` catches everything the agent row does not cover. The
///   payload is re-resolved from the live persona, team, global agent config,
///   effective harness descriptor, workspace relay URL and owner pubkey, and
///   the provider config rides beside it — all of which can change with the
///   agent row untouched.
#[derive(Debug)]
pub(super) struct CapturedDeploy {
    pub(super) provider_id: String,
    pub(super) config: serde_json::Value,
    pub(super) cached_binary_path: Option<String>,
    pub(super) updated_at: String,
    pub(super) payload_digest: String,
}

/// Capture the provider coordinates, configuration generation and resolved
/// payload digest of `record` for a deploy. Errors when the record is not
/// provider-backed.
fn capture_provider_deploy(
    record: &ManagedAgentRecord,
    payload: &serde_json::Value,
) -> Result<CapturedDeploy, String> {
    match &record.backend {
        BackendKind::Provider { id, config } => Ok(CapturedDeploy {
            provider_id: id.clone(),
            config: config.clone(),
            cached_binary_path: record.provider_binary_path.clone(),
            updated_at: record.updated_at.clone(),
            payload_digest: digest_parts(id, config, payload)?,
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
    payload: &serde_json::Value,
) -> Result<CapturedDeploy, String> {
    match (&record.backend, record.backend_agent_id.as_deref()) {
        (BackendKind::Provider { .. }, Some(_)) => capture_provider_deploy(record, payload),
        (BackendKind::Provider { .. }, None) => Err(format!(
            "agent {pubkey} has not been deployed yet; deploy it before redeploying"
        )),
        _ => Err(format!(
            "agent {pubkey} is not provider-backed and cannot be redeployed"
        )),
    }
}

/// Everything a deploy flow captures under the store lock before the provider
/// call: the provider coordinates, the generation and digest the completion is
/// bound to, and the fully resolved payload that will be sent.
///
/// The payload is resolved here — under the same lock, in the same instant as
/// the digest — rather than after the lock is released, so the bytes on the
/// wire and the digest the completion compares against can never come from two
/// different reads of the persona, team or global config.
pub(super) struct DeploySnapshot {
    pub(super) captured: CapturedDeploy,
    pub(super) payload: serde_json::Value,
}

/// What one store transaction did: the value `apply` returned, whether a save
/// was performed, and the error if the save was attempted and failed.
///
/// A failed save is *not* folded into the outer `Err`, because when the
/// provider call already succeeded the difference matters: the outer `Err` is
/// "nothing happened", while a save error is "something happened remotely and
/// this app did not record it" — the ambiguous-success case that owes a
/// durable receipt.
pub(super) struct StoreOutcome<R> {
    pub(super) result: R,
    pub(super) saved: bool,
    pub(super) save_error: Option<String>,
}

/// How a finished provider call was bound to the store state found on
/// completion. Computed by [`bind_completion`].
#[derive(Debug)]
pub(super) enum CompletionOutcome {
    /// Record and its fully resolved deploy input both unchanged since the
    /// snapshot: success persisted as current.
    AppliedCurrent(Box<ManagedAgentRecord>),
    /// The deploy input moved mid-flight — the agent row was edited, or an
    /// input outside it (persona, team, global config, relay URL, owner,
    /// provider config) resolved differently. The remote facts are persisted
    /// (`backend_agent_id`, `last_started_at`) because the deploy really
    /// happened, but what the provider received is not what the store now
    /// describes, so nothing may imply the current configuration was sent —
    /// `last_error` is left untouched.
    AppliedStale,
    /// The deploy succeeded but the payload could not be re-resolved on
    /// completion, so currency could not be *proved* either way. The remote
    /// facts are persisted; nothing claims currency. Carries the resolution
    /// error.
    AppliedUnverified(String),
    /// Record switched away from the captured provider backend mid-flight
    /// (to local, or to a different provider): nothing persisted — the
    /// deployment id belongs to a backend the record no longer describes.
    BackendChanged,
    /// Record deleted mid-flight: nothing persisted.
    Deleted,
    /// Provider call failed and the surviving record is still the one that
    /// failed: the failure is stamped on it.
    Failed(String),
    /// Provider call failed, but the record in the store is no longer the
    /// deploy that failed — a different backend, or a moved generation.
    /// Nothing is stamped: `last_error` on a record describes *that* record's
    /// last attempt, and this attempt was of something else.
    FailedUnstamped(String),
}

/// Re-resolves the full deploy input of a record found on completion and
/// digests it, so inputs that live outside the agent row (persona, team,
/// global config, relay URL, owner) are compared too. Supplied by the
/// [`DeployEffects`] impl, which knows how to reach them.
pub(super) type ResolveDigest<'a> = dyn Fn(&ManagedAgentRecord) -> Result<String, String> + 'a;

/// The completion body [`DeployEffects::with_store`] runs against the loaded
/// records, returning whether the caller must save.
pub(super) type CompletionApply<'a, R> =
    dyn FnMut(&mut Vec<ManagedAgentRecord>, &ResolveDigest<'_>) -> (bool, R) + 'a;

/// Whether the record found on completion is still the deploy that was sent.
#[derive(Debug)]
enum Currency {
    /// Same generation and same fully resolved deploy input.
    Current,
    /// The agent row, or an input resolved outside it, moved.
    Moved,
    /// The deploy input could not be re-resolved, so neither can be proved.
    Unverified(String),
}

/// Compare the record in the store against the captured deploy. Must be called
/// before the completion mutates `record.updated_at`.
fn currency(
    record: &ManagedAgentRecord,
    captured: &CapturedDeploy,
    resolve_digest: &ResolveDigest<'_>,
) -> Currency {
    if record.updated_at != captured.updated_at {
        return Currency::Moved;
    }
    match resolve_digest(record) {
        Ok(digest) if digest == captured.payload_digest => Currency::Current,
        Ok(_) => Currency::Moved,
        Err(error) => Currency::Unverified(error),
    }
}

/// Bind a finished provider call to whatever the store holds now. Mutates
/// `records` in place; the returned bool says whether the caller must save.
///
/// Every stamp — success *and* failure — is gated on the surviving record
/// still being the deploy this completion belongs to: same provider backend,
/// same generation, same re-resolved deploy input. A success stamped onto a
/// foreign backend would describe a deployment that backend never made; a
/// failure stamped onto a moved record would report an attempt of a
/// configuration that was never attempted.
///
/// `resolve_digest` re-resolves the full deploy input for the record found on
/// completion, so inputs that live outside the agent row are compared too.
pub(super) fn bind_completion(
    records: &mut [ManagedAgentRecord],
    pubkey: &str,
    captured: &CapturedDeploy,
    resolve_digest: &ResolveDigest<'_>,
    deploy_result: &Result<String, String>,
) -> (bool, CompletionOutcome) {
    let Some(record) = records.iter_mut().find(|record| record.pubkey == pubkey) else {
        return (false, CompletionOutcome::Deleted);
    };
    let same_backend = matches!(
        &record.backend,
        BackendKind::Provider { id, .. } if *id == captured.provider_id
    );
    match deploy_result {
        Err(error) => {
            if !same_backend {
                return (false, CompletionOutcome::FailedUnstamped(error.clone()));
            }
            match currency(record, captured, resolve_digest) {
                Currency::Current => {
                    record.last_error = Some(error.clone());
                    record.updated_at = now_iso();
                    (true, CompletionOutcome::Failed(error.clone()))
                }
                Currency::Moved | Currency::Unverified(_) => {
                    (false, CompletionOutcome::FailedUnstamped(error.clone()))
                }
            }
        }
        Ok(backend_agent_id) => {
            if !same_backend {
                return (false, CompletionOutcome::BackendChanged);
            }
            let currency = currency(record, captured, resolve_digest);
            record.backend_agent_id = Some(backend_agent_id.clone());
            record.last_started_at = Some(now_iso());
            record.updated_at = now_iso();
            match currency {
                Currency::Current => {
                    record.last_error = None;
                    (
                        true,
                        CompletionOutcome::AppliedCurrent(Box::new(record.clone())),
                    )
                }
                Currency::Moved => (true, CompletionOutcome::AppliedStale),
                Currency::Unverified(error) => (true, CompletionOutcome::AppliedUnverified(error)),
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
             flight, so the provider was sent the previous settings. Redeploy to send the latest \
             ones — a queued redeploy will do that automatically."
            .to_string()),
        CompletionOutcome::AppliedUnverified(error) => Err(format!(
            "The settings were sent, but Buzz could not re-read them afterwards to confirm the \
             provider got the current ones: {error}"
        )),
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
        // The provider's own error either way. It is what the owner needs to
        // act on, and it reaches them verbatim in a toast whether or not the
        // moved record was a legitimate place to stamp it.
        CompletionOutcome::Failed(error) | CompletionOutcome::FailedUnstamped(error) => Err(error),
    }
}

/// Effects a deploy orchestration runs against. The real commands wire
/// these to the on-disk store and the provider binary; tests wire an
/// in-memory store and a gated fake provider so mid-flight edits and
/// deletions can be driven deterministically.
pub(super) trait DeployEffects {
    type Success;

    /// Under the store lock: load the record, resolve its deploy payload, and
    /// validate it as a deploy target.
    fn snapshot(&mut self, pubkey: &str) -> Result<DeploySnapshot, String>;

    /// The provider call. Must not touch the store — the payload it sends was
    /// resolved in [`DeployEffects::snapshot`].
    async fn deploy(&mut self, snapshot: &DeploySnapshot) -> Result<String, String>;

    /// Run `apply` on the loaded records under the store lock, saving when it
    /// returns `true`. `apply`'s second argument re-resolves a record's full
    /// deploy input digest, so the completion can compare inputs that live
    /// outside the agent row.
    fn with_store<R>(
        &mut self,
        apply: &mut CompletionApply<'_, R>,
    ) -> Result<StoreOutcome<R>, String>;

    /// Persist a durable record of a deploy the provider accepted and the
    /// store failed to save.
    fn record_ambiguous_success(&mut self, receipt: &AmbiguousDeployReceipt) -> Result<(), String>;

    /// Drop any ambiguous-success receipt for `pubkey`: the store has just
    /// recorded a known outcome for this agent, so the ambiguity is resolved.
    fn clear_ambiguous_success(&mut self, pubkey: &str);

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

    let captured = &snapshot.captured;
    let store = effects.with_store(&mut |records, resolve_digest| {
        bind_completion(records, pubkey, captured, resolve_digest, &deploy_result)
    })?;
    let StoreOutcome {
        result: outcome,
        saved,
        save_error,
    } = store;

    if let Some(save_error) = save_error {
        return Err(unrecorded_completion(
            effects,
            pubkey,
            captured,
            &deploy_result,
            save_error,
        ));
    }
    if saved {
        effects.clear_ambiguous_success(pubkey);
    }

    let record = completion_result(outcome, deploy_result)?;
    effects.success(&record)
}

/// The store write that should have recorded a finished provider call failed.
///
/// When the provider call *also* failed, the only thing lost is a local
/// `last_error` stamp and the provider's error is what the owner needs, so it
/// is surfaced with the save failure appended. When the provider call
/// succeeded, this is the ambiguous-success case: something is running (or
/// converging) remotely that this app has no record of asking for, so a
/// durable receipt is written before the error is returned. Returns the
/// message for the owner.
fn unrecorded_completion<E: DeployEffects>(
    effects: &mut E,
    pubkey: &str,
    captured: &CapturedDeploy,
    deploy_result: &Result<String, String>,
    save_error: String,
) -> String {
    let backend_agent_id = match deploy_result {
        Err(error) => {
            return format!("{error} (and the result could not be saved: {save_error})");
        }
        Ok(backend_agent_id) => backend_agent_id.clone(),
    };

    let receipt = AmbiguousDeployReceipt {
        pubkey: pubkey.to_string(),
        provider_id: captured.provider_id.clone(),
        backend_agent_id,
        payload_digest: captured.payload_digest.clone(),
        captured_updated_at: captured.updated_at.clone(),
        save_error: save_error.clone(),
        observed_at: now_iso(),
    };
    match effects.record_ambiguous_success(&receipt) {
        Ok(()) => format!(
            "The provider accepted the deploy but Buzz could not save the result: {save_error}. \
             The agent may be deployed with the settings that were just sent while this app's \
             record does not show it — a receipt of what was sent was saved so it can be \
             reconciled later."
        ),
        Err(receipt_error) => format!(
            "The provider accepted the deploy but Buzz could not save the result: {save_error}. \
             The agent may be deployed with the settings that were just sent while this app's \
             record does not show it, and the receipt of what was sent could not be saved \
             either: {receipt_error}"
        ),
    }
}

/// Validate `record` as a plain deploy target (create/start/reconcile): any
/// provider-backed record qualifies, deployed before or not.
fn deploy_provider_target(
    _pubkey: &str,
    record: &ManagedAgentRecord,
    payload: &serde_json::Value,
) -> Result<CapturedDeploy, String> {
    capture_provider_deploy(record, payload)
}

/// [`DeployEffects`] wired to the real store, payload builder, and provider
/// binary. `target` validates the record under the store lock —
/// [`deploy_provider_target`] for first-time deploys,
/// [`redeploy_provider_target`] for the redeploy command, which additionally
/// requires an existing deployment.
struct CommandDeployEffects<'a> {
    app: &'a AppHandle,
    state: &'a AppState,
    target: fn(&str, &ManagedAgentRecord, &serde_json::Value) -> Result<CapturedDeploy, String>,
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
        // Read side of the ambiguous-success receipt: a deploy for an agent
        // whose last one was never recorded says so in the app log, where it
        // is legible next to whatever this deploy goes on to do.
        for receipt in deploy_receipt::read_deploy_receipts(self.app)
            .iter()
            .filter(|receipt| receipt.pubkey == pubkey)
        {
            eprintln!(
                "buzz-desktop: agent {pubkey} has an unrecorded deploy from {} (provider {}, \
                 deployment {}, input digest {}) — the provider may be running it; this deploy \
                 supersedes it",
                receipt.observed_at,
                receipt.provider_id,
                receipt.backend_agent_id,
                receipt.payload_digest
            );
        }
        let payload = super::build_deploy_payload(self.app, self.state, record)?;
        let captured = (self.target)(pubkey, record, &payload)?;
        Ok(DeploySnapshot { captured, payload })
    }

    async fn deploy(&mut self, snapshot: &DeploySnapshot) -> Result<String, String> {
        call_provider_deploy(&snapshot.captured, snapshot.payload.clone()).await?
    }

    fn with_store<R>(
        &mut self,
        apply: &mut CompletionApply<'_, R>,
    ) -> Result<StoreOutcome<R>, String> {
        let _store_guard = self
            .state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(self.app)?;
        // Copies of two shared references, so the resolver borrows nothing
        // that the save below needs back. Resolving under the store lock is
        // deliberate: it reads personas, teams and global config, none of
        // which take this lock, and it must see the same store state the
        // completion is about to stamp.
        let (app, state) = (self.app, self.state);
        let resolve_digest = move |record: &ManagedAgentRecord| -> Result<String, String> {
            let payload = super::build_deploy_payload(app, state, record)?;
            deploy_input_digest(record, &payload)
        };
        let (save, result) = apply(&mut records, &resolve_digest);
        let save_error = if save {
            save_managed_agents(self.app, &records).err()
        } else {
            None
        };
        Ok(StoreOutcome {
            result,
            saved: save && save_error.is_none(),
            save_error,
        })
    }

    fn record_ambiguous_success(&mut self, receipt: &AmbiguousDeployReceipt) -> Result<(), String> {
        deploy_receipt::write_deploy_receipt(self.app, receipt)
    }

    fn clear_ambiguous_success(&mut self, pubkey: &str) {
        deploy_receipt::clear_deploy_receipt(self.app, pubkey);
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
