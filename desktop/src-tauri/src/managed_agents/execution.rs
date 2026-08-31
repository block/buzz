//! Native transition seam for authenticated host commands. The relay receiver/UI
//! is deliberately not enabled yet. Only destination-local provisioned configs
//! are supported: no source workspace, environment, loopback endpoint or key copy.
use buzz_core_pkg::host_execution::{Action, Command, Outcome};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

use super::{
    execution_ledger::{Begin, Entry, Ledger},
    *,
};

#[derive(Serialize)]
pub(crate) struct LocalExecutionConfig {
    pub runtime: String,
    pub revision: String,
}

pub(super) fn config_revision(
    record: &ManagedAgentRecord,
    personas: &[AgentDefinition],
    global: &GlobalAgentConfig,
    teams: &[TeamRecord],
    descriptor: &super::readiness::EffectiveHarnessDescriptor,
) -> Result<String, String> {
    // Hash in memory; never persist/return these secret-bearing input bytes.
    let bytes = serde_json::to_vec(&(
        env!("CARGO_PKG_VERSION"),
        record,
        personas,
        global,
        teams,
        &descriptor.command,
        &descriptor.args,
        &descriptor.env,
    ))
    .map_err(|_| "cannot fingerprint destination config")?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(crate) fn local_execution_config(
    app: &AppHandle,
    record: &ManagedAgentRecord,
) -> Result<LocalExecutionConfig, String> {
    execution_config(app, record, true)
}

fn execution_config(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    remote: bool,
) -> Result<LocalExecutionConfig, String> {
    local_execution_prerequisites(record)?;
    let personas = load_personas(app)?;
    let teams = load_teams(app)?;
    let global = load_global_agent_config(app)?;
    let descriptor = resolve_effective_harness_descriptor(record, &personas, &global)?;
    let runtime = known_acp_runtime(&descriptor.command);
    let revision = config_revision(record, &personas, &global, &teams, &descriptor)?;
    if !remote {
        // Ordinary local launch keeps its existing readiness/custom-runtime and
        // agents-everywhere semantics. Spawn still rechecks this exact revision.
        return Ok(LocalExecutionConfig {
            runtime: runtime.map_or("custom", |runtime| runtime.id).into(),
            revision,
        });
    }
    let runtime = runtime.ok_or("destination runtime is not in the Rust catalog")?;
    let effective = effective_config::resolve_effective_config(record, &personas, &global)
        .require_resolved()?;
    // Destination-local mesh preflight needs its own readiness gate. Do not
    // mistake a source machine's loopback endpoint for a portable provider.
    if effective.relay_mesh_model_id().is_some() {
        return Err("host command mesh preflight is not supported yet".into());
    }
    if !matches!(
        agent_readiness(&resolve_effective_agent_env(
            record,
            &personas,
            Some(runtime),
            &global
        )),
        AgentReadiness::Ready
    ) {
        return Err("destination agent configuration is not ready".into());
    }
    Ok(LocalExecutionConfig {
        runtime: runtime.id.into(),
        revision,
    })
}

// Shared by inspection/advertisement and the launch preflight. Only key
// availability is checked, exactly as at spawn; no key parsing/export is needed.
fn local_execution_prerequisites(record: &ManagedAgentRecord) -> Result<(), String> {
    if record.backend != BackendKind::Local {
        return Err("destination agent is not locally provisioned".into());
    }
    if let Some(error) = super::storage::spawn_key_refusal(record) {
        return Err(error);
    }
    Ok(())
}

/// Remote execution requires an explicit verified owner attestation, even for
/// legacy local records. A local key in the store is not a remote launch grant.
pub(crate) fn execution_agent_owner(
    record: &ManagedAgentRecord,
    owner: &str,
) -> Result<(), String> {
    let agent =
        nostr::PublicKey::from_hex(&record.pubkey).map_err(|_| "invalid provisioned agent")?;
    let tag = record
        .auth_tag
        .as_deref()
        .ok_or("destination agent needs owner setup")?;
    let issuer = buzz_sdk_pkg::nip_oa::verify_auth_tag(tag, &agent)
        .map_err(|_| "invalid destination ownership")?;
    if issuer.to_hex() != owner {
        return Err("destination agent belongs to another owner".into());
    }
    Ok(())
}

fn ledger(app: &AppHandle, key: &ManagedAgentRuntimeKey, owner: &str) -> Result<Ledger, String> {
    if !buzz_core_pkg::host_execution::hex_id(owner, 64) {
        return Err("invalid execution owner".into());
    }
    Ledger::open(
        &managed_agents_base_dir(app)?.join("execution-ledger"),
        &format!("{owner}__{}", key.runtime_id()),
    )
}

pub(super) fn legacy_spawn_guard(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    relay: &str,
    owner: Option<&str>,
) -> Result<Ledger, String> {
    let owner = owner.ok_or("managed launch requires an owner")?;
    let key = ManagedAgentRuntimeKey::new(&record.pubkey, relay)?;
    let ledger = ledger(app, &key, owner)?;
    if ledger.is_fenced() {
        return Err(
            "placement is controlled by a durable execution operation; explicit Start required"
                .into(),
        );
    }
    Ok(ledger)
}

/// Called only after signature, live registration, destination and runtime
/// compatibility checks. Caller retains owner authority, never a host-only login.
pub(crate) fn execute_host_operation(
    app: &AppHandle,
    owner: &str,
    command_id: &str,
    request: &Command,
    compatible_runtime: bool,
) -> Result<Entry, String> {
    execute_operation(
        app,
        owner,
        command_id,
        request,
        compatible_runtime,
        Admission::Remote,
    )
}

enum Admission<'a> {
    Remote,
    // Exact predecessor captured by the explicit local action; rechecked while
    // holding both the transition lock and OS journal lock, never auto-reconcile.
    Local { predecessor: &'a str },
}

fn local_start_predecessor(entry: &Entry) -> Result<(), String> {
    match (&entry.request.action, &entry.outcome) {
        (Action::Stop { .. }, Outcome::Stopped) | (Action::Start { .. }, Outcome::Rejected) => {
            Ok(())
        }
        _ => Err(
            "Previous execution is not proven stopped or rejected; replacement remains blocked"
                .into(),
        ),
    }
}

impl Admission<'_> {
    fn validate_predecessor(&self, ledger: &Ledger, request: &Command) -> Result<(), String> {
        if let Admission::Local { predecessor } = self {
            let current = ledger
                .current()
                .ok_or("local Start predecessor disappeared")?;
            if current.command_id != *predecessor || !matches!(request.action, Action::Start { .. })
            {
                return Err("local Start predecessor changed; refresh runtime status".into());
            }
            local_start_predecessor(current)?;
        }
        Ok(())
    }

    fn admits_record(&self, record: &ManagedAgentRecord, owner: &str, relay: &str) -> bool {
        match self {
            Self::Local { .. } => local_execution_prerequisites(record).is_ok(),
            Self::Remote => {
                buzz_core_pkg::relay::normalize_relay_url(&record.relay_url)
                    .ok()
                    .as_deref()
                    == Some(relay)
                    && execution_agent_owner(record, owner).is_ok()
            }
        }
    }
}

fn execute_operation(
    app: &AppHandle,
    owner: &str,
    command_id: &str,
    request: &Command,
    compatible_runtime: bool,
    admission: Admission<'_>,
) -> Result<Entry, String> {
    let state = app.state::<crate::app_state::AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|_| "runtime transition lock unavailable")?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|_| "agent store lock unavailable")?;
    if state
        .shutdown_started
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("desktop is shutting down".into());
    }
    crate::relay::assert_expected_signer(
        Some(owner),
        &state.signing_keys()?.public_key().to_hex(),
    )?;
    crate::relay::assert_expected_relay_scope(
        Some(&request.relay),
        &crate::relay::relay_api_base_url_with_override(&state),
    )?;
    let key = ManagedAgentRuntimeKey::new(&request.agent, &request.relay)?;
    let mut ledger = ledger(app, &key, owner)?;
    // Retry is resolved before inspecting current config/process state. Config
    // drift after success must not turn an ACK-loss retry into another launch.
    if let Some(entry) = ledger.replay(command_id, request)? {
        return Ok(entry);
    }
    // Historical commands may only read the immutable ledger. An expired
    // request without prior intent never reaches launch or Stop.
    if request.expires_at <= nostr::Timestamp::now().as_secs() {
        return Err("execution command expired without a recorded outcome".into());
    }
    admission.validate_predecessor(&ledger, request)?;
    let mut records = load_managed_agents(app)?;
    if matches!(request.action, Action::Start { .. })
        && !records
            .iter()
            .any(|r| r.pubkey == request.agent && admission.admits_record(r, owner, &request.relay))
    {
        ledger.begin(command_id, request)?;
        return ledger.finish(&request.operation, Outcome::Rejected);
    }
    let record = find_managed_agent_mut(&mut records, &request.agent)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|_| "runtime lock unavailable")?;
    if let Action::Stop { run } = &request.action {
        // Validate before persisting a new Stop fence. A stale clicked run must
        // neither kill nor take ownership of a newer local placement.
        let selected = runtimes
            .get(&key)
            .ok_or("selected run is not tracked; stop outcome unknown")?;
        if !exact_generation_matches(&selected.start_nonce, run) {
            return Err("selected generation is no longer current".into());
        }
    }
    if let Begin::Replay(entry) = ledger.begin(command_id, request)? {
        return Ok(entry);
    }
    match &request.action {
        Action::Start { runtime, revision } => {
            // No adoption or teardown of a peer, even one whose root has exited.
            // A legacy receipt (including corrupt data) is an unresolved conflict.
            let receipt_path = managed_agents_base_dir(app)?
                .join("agent-pids")
                .join(format!("{}.json", key.runtime_id()));
            let preflight = execution_config(app, record, matches!(admission, Admission::Remote));
            let compatible = preflight
                .is_ok_and(|config| config.runtime == *runtime && config.revision == *revision);
            if !compatible_runtime
                || !compatible
                || runtimes.contains_key(&key)
                || receipt_path
                    .try_exists()
                    .map_err(|_| "cannot inspect prior receipt")?
            {
                return ledger.finish(&request.operation, Outcome::Rejected);
            }
            let process = match super::runtime::spawn_agent_child_for_run(
                app,
                record,
                &key.relay_url,
                false,
                Some(owner),
                Some((&request.operation, revision)),
            ) {
                Ok(process) => process,
                // spawn_agent_child_for_run returns Err only before a child is
                // created (including OS spawn failure); after spawn it always
                // returns the retained Child. Never classify post-spawn receipt,
                // startup timeout or root-exit failures as definite rejection.
                Err(_) => return ledger.finish(&request.operation, Outcome::Rejected),
            };
            let now = crate::util::now_iso();
            let receipt = ManagedAgentRuntimeReceipt {
                key: key.clone(),
                pid: process.child.id(),
                desktop_instance_id: current_instance_id(app),
                started_at: now.clone(),
                run_id: Some(process.start_nonce.clone()),
            };
            if write_agent_runtime_receipt(app, &receipt).is_err() {
                // Preserve a possibly surviving child in memory, even if durable
                // observation failed. Never report definite failure after spawn.
                runtimes.insert(key, ManagedAgentPairRuntime::starting(process));
                return ledger.finish(&request.operation, Outcome::Unknown);
            }
            // Process observation only: not Ready, not online, not an LLM turn.
            record.updated_at = now.clone();
            record.last_started_at = Some(now);
            record.last_stopped_at = None;
            record.last_error = None;
            record.runtime_pid = None;
            runtimes.insert(key, ManagedAgentPairRuntime::starting(process));
            if save_managed_agents(app, &records).is_err() {
                return ledger.finish(&request.operation, Outcome::Unknown);
            }
            ledger.finish(&request.operation, Outcome::Spawned)
        }
        Action::Stop { run } => {
            let Some(runtime) = runtimes.get_mut(&key) else {
                // Absence, legacy PID receipts and presence expiry are not proof.
                return ledger.finish(&request.operation, Outcome::Unknown);
            };
            if !exact_generation_matches(&runtime.start_nonce, run) {
                // A delayed selected-run Stop must not kill this newer peer.
                return ledger.finish(&request.operation, Outcome::Unknown);
            }
            let actual = runtime.start_nonce.clone();
            if stop_selected_generation(&mut runtime.child, &actual, run).is_err() {
                return ledger.finish(&request.operation, Outcome::Unknown);
            }
            // Root exit alone cannot certify separately grouped descendants.
            // Only a supported, authenticated same-generation owned-work proof
            // permits replacement. Missing/invalid evidence remains fenced.
            let proof_path = stop_proof_path(&runtime.log_path, run);
            let successful_root = runtime
                .child
                .try_wait()
                .ok()
                .flatten()
                .is_some_and(|status| status.success());
            let outcome = if successful_root
                && verified_stop_proof(&proof_path, &key.pubkey, &key.relay_url, run)
            {
                Outcome::Stopped
            } else {
                Outcome::RootExited
            };
            let result = ledger.finish(&request.operation, outcome)?;
            let _ = std::fs::remove_file(proof_path);
            runtimes.remove(&key);
            remove_agent_runtime_receipt(app, &key);
            state.clear_agent_session_cache(&key);
            record.runtime_pid = None;
            record.updated_at = crate::util::now_iso();
            record.last_stopped_at = Some(record.updated_at.clone());
            save_managed_agents(app, &records)?;
            Ok(result)
        }
    }
}

/// Ordinary Desktop Stop enters the same generation fence, proof and ledger
/// authority as a host Stop. The clicked nonce is mandatory, never read afresh.
pub(super) fn stop_local_selected_run(
    app: &AppHandle,
    pubkey: &str,
    relay: &str,
    selected_run: Option<&str>,
) -> Result<(), String> {
    let run = selected_run
        .filter(|run| buzz_core_pkg::host_execution::hex_id(run, 32))
        .ok_or("Exact Stop unsupported without a selected run; refresh runtime status")?;
    let state = app.state::<crate::app_state::AppState>();
    let owner = state.signing_keys()?.public_key().to_hex();
    let key = ManagedAgentRuntimeKey::new(pubkey, relay)?;
    let operation = hex::encode(Sha256::digest(format!(
        "buzz.desktop.stop.v1\n{owner}\n{}\n{pubkey}\n{run}",
        key.relay_url
    )))[..32]
        .to_owned();
    // Reuse the first immutable local request (including deadline) on retry.
    let prior = ledger(app, &key, &owner)?.operation(&operation).cloned();
    let (command_id, request) = match prior {
        Some(entry) => (entry.command_id, entry.request),
        None => {
            let request = Command {
                v: 1,
                operation,
                relay: key.relay_url,
                agent: pubkey.into(),
                expires_at: nostr::Timestamp::now().as_secs()
                    + buzz_core_pkg::host_execution::COMMAND_TTL,
                action: Action::Stop { run: run.into() },
            };
            let bytes = serde_json::to_vec(&request).map_err(|_| "invalid local Stop")?;
            (hex::encode(Sha256::digest(bytes)), request)
        }
    };
    let entry = execute_host_operation(app, &owner, &command_id, &request, false)?;
    if entry.outcome != Outcome::Stopped {
        return Err(format!(
            "Selected Stop unconfirmed ({:?}); replacement remains blocked",
            entry.outcome
        ));
    }
    Ok(())
}

/// Explicit ordinary Start after an exact Stop or a definite rejected Start
/// uses the same journal, with ordinary local admission rather than remote
/// provisioning grants. Automatic reconciliation never calls this.
pub(crate) fn start_after_exact_stop(
    app: &AppHandle,
    pubkey: &str,
    relay: &str,
    owner: &str,
) -> Result<bool, String> {
    let key = ManagedAgentRuntimeKey::new(pubkey, relay)?;
    let prior = ledger(app, &key, owner)?.current().cloned();
    let Some(prior) = prior else {
        return Ok(false);
    };
    local_start_predecessor(&prior)?;
    let records = load_managed_agents(app)?;
    let record = records
        .iter()
        .find(|r| r.pubkey == pubkey)
        .ok_or("agent not found")?;
    let config = execution_config(app, record, false)?;
    let request = Command {
        v: 1,
        operation: uuid::Uuid::new_v4().simple().to_string(),
        relay: key.relay_url,
        agent: pubkey.into(),
        expires_at: nostr::Timestamp::now().as_secs() + buzz_core_pkg::host_execution::COMMAND_TTL,
        action: Action::Start {
            runtime: config.runtime,
            revision: config.revision,
        },
    };
    let bytes = serde_json::to_vec(&request).map_err(|_| "invalid local Start")?;
    let result = execute_operation(
        app,
        owner,
        &hex::encode(Sha256::digest(bytes)),
        &request,
        true,
        Admission::Local {
            predecessor: &prior.command_id,
        },
    )?;
    if result.outcome != Outcome::Spawned {
        return Err("Explicit Start not confirmed; inspect destination setup".into());
    }
    Ok(true)
}

pub(super) fn stop_proof_path(log: &std::path::Path, run: &str) -> std::path::PathBuf {
    log.with_extension(format!("stop-{run}.json"))
}

fn verified_stop_proof(path: &std::path::Path, agent: &str, relay: &str, run: &str) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(4097).read_to_end(&mut bytes).is_err() || bytes.len() > 4096 {
        return false;
    }
    serde_json::from_slice::<buzz_core_pkg::owned_stop::Proof>(&bytes)
        .is_ok_and(|proof| buzz_core_pkg::owned_stop::verify(&proof, agent, relay, run).is_ok())
}

fn stop_selected_generation(
    child: &mut std::process::Child,
    actual: &str,
    expected: &str,
) -> Result<(), String> {
    if !exact_generation_matches(actual, expected) {
        return Err("selected generation is no longer current".into());
    }
    super::runtime::terminate_exact_owned_group(child)
}

#[cfg(all(test, unix))]
#[path = "execution_stop_process_tests.rs"]
mod stop_process_tests;

fn exact_generation_matches(actual: &str, expected: &str) -> bool {
    buzz_core_pkg::host_execution::hex_id(expected, 32) && actual == expected
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyless_record_is_not_advertised_as_locally_provisioned() {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "synthetic-agent", "name": "test-agent", "private_key_nsec": "",
            "relay_url": "wss://relay.example", "acp_command": "buzz-acp",
            "agent_command": "buzz-agent", "agent_args": [], "mcp_command": "",
            "turn_timeout_seconds": 320, "created_at": "", "updated_at": ""
        }))
        .unwrap();
        assert_eq!(
            local_execution_prerequisites(&record).unwrap_err(),
            super::super::storage::spawn_key_refusal(&record).unwrap()
        );
        // Availability only: this preflight must not inspect or export key bytes.
        record.private_key_nsec = "synthetic-nonempty-placeholder".into();
        assert!(local_execution_prerequisites(&record).is_ok());
        // A legacy pin and absent attestation do not revoke ordinary local
        // community-pair authority. They still deny remote destination Start.
        let local = Admission::Local {
            predecessor: "unused",
        };
        assert!(local.admits_record(&record, &"aa".repeat(32), "wss://other.example"));
        assert!(!Admission::Remote.admits_record(&record, &"aa".repeat(32), "wss://other.example"));
        assert!(!Admission::Remote.admits_record(&record, &"aa".repeat(32), "wss://relay.example"));
    }

    #[test]
    fn stop_fences_successor_and_malformed_or_legacy_generation() {
        assert!(exact_generation_matches(&"aa".repeat(16), &"aa".repeat(16)));
        assert!(!exact_generation_matches(
            &"aa".repeat(16),
            &"bb".repeat(16)
        ));
        assert!(!exact_generation_matches("", ""));
        assert!(!exact_generation_matches("legacy", "legacy"));
    }
}

#[cfg(test)]
#[path = "execution_local_recovery_tests.rs"]
mod local_recovery_tests;
