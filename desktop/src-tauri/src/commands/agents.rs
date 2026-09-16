use nostr::Keys;
use tauri::{AppHandle, State};

use super::managed_agent_definition::validate_create_definition;

use crate::{
    app_state::AppState,
    managed_agents::{
        bestie_assignment::{recover_pending_assignment_cleanup, with_agent_assignments_cleared},
        build_managed_agent_summary, current_instance_id, ensure_persona_is_active,
        find_managed_agent_mut, load_managed_agents, load_personas, load_teams,
        managed_agents_base_dir, normalize_agent_args, resolve_provider_binary,
        save_managed_agents, start_managed_agent_process, stop_managed_agent_process,
        stop_managed_agent_workspace_pair, sync_managed_agent_processes, try_regenerate_nest,
        validate_provider_config, BackendKind, CreateManagedAgentRequest,
        CreateManagedAgentResponse, ManagedAgentRecord, ManagedAgentSummary, RelayMeshConfig,
        DEFAULT_ACP_COMMAND, DEFAULT_AGENT_PARALLELISM, DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
    },
    relay::relay_ws_url_with_override,
    util::now_iso,
};

/// Read the workspace owner pubkey without holding the lock. Used to populate `BUZZ_ACP_AGENT_OWNER`
/// as a fallback for legacy agent records that have no NIP-OA `auth_tag`.
pub(super) fn workspace_owner_hex(state: &AppState) -> Result<String, String> {
    Ok(state.identity_public_key()?.to_hex())
}

#[path = "agents_pending.rs"]
mod pending;
#[cfg(test)]
use pending::build_agent_archive_request;
pub(crate) use pending::{finish_managed_agent_pending, prepare_managed_agent_pending};
#[path = "agents_deletion_witness.rs"]
pub(crate) mod deletion_witness;

/// Build a summary from fresh disk state (personas, teams, global config).
/// For one-shot command paths only — the 5s list poll calls
/// `build_managed_agent_summary` directly with stores loaded once per call,
/// not once per record.
pub(super) fn summarize_from_disk<R: tauri::Runtime>(
    app: &AppHandle<R>,
    record: &ManagedAgentRecord,
    runtimes: &std::collections::HashMap<
        crate::managed_agents::ManagedAgentRuntimeKey,
        crate::managed_agents::ManagedAgentPairRuntime,
    >,
) -> Result<ManagedAgentSummary, String> {
    build_managed_agent_summary(
        app,
        record,
        runtimes,
        &load_personas(app).unwrap_or_default(),
        &load_teams(app).unwrap_or_default(),
        &crate::managed_agents::load_global_agent_config(app).unwrap_or_default(),
    )
}

#[path = "agents_create_fields.rs"]
mod create_fields;
use create_fields::{normalize_relay_mesh, resolve_created_avatar_url, trim_to_optional_string};

#[cfg(feature = "mesh-llm")]
async fn ensure_relay_mesh_for_record(
    app: &AppHandle,
    model_id: Option<&str>,
    allow_fresh_create_start: bool,
) -> Result<(), String> {
    crate::commands::ensure_relay_mesh_for_record(app, model_id, allow_fresh_create_start).await
}

#[cfg(not(feature = "mesh-llm"))]
async fn ensure_relay_mesh_for_record(
    _app: &AppHandle,
    _model_id: Option<&str>,
    _allow_fresh_create_start: bool,
) -> Result<(), String> {
    Ok(())
}

pub(super) async fn start_local_agent_pairs_with_preflight(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    relay_urls: &[String],
) -> Result<ManagedAgentSummary, String> {
    let operation = crate::user_operation::UserOperationScope::capture(state)?;
    start_local_agent_pairs_in_scope(app, state, pubkey, relay_urls, &operation).await
}

pub(super) async fn start_local_agent_pairs_in_scope(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    relay_urls: &[String],
    operation: &crate::user_operation::UserOperationScope,
) -> Result<ManagedAgentSummary, String> {
    let record_snapshot = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        load_managed_agents(app)?
            .into_iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?
    };
    if record_snapshot.backend != BackendKind::Local {
        return Err(format!("agent {pubkey} is not a local agent"));
    }
    let personas_for_preflight = {
        let _store = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let _admission = operation.admit(state)?;
        load_personas(app).unwrap_or_default()
    };
    let global_for_preflight =
        crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    let mesh_model_id =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            &record_snapshot,
            &personas_for_preflight,
            &global_for_preflight,
        );
    ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), false).await?;

    let retention = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let _admission = operation.admit(state)?;
        let mut records = load_managed_agents(app)?;
        let record = find_managed_agent_mut(&mut records, pubkey)?;
        let personas = load_personas(app).unwrap_or_default();
        if let Some(persona_id) = record.persona_id.clone() {
            if let Some(persona) = personas.iter().find(|persona| persona.id == persona_id) {
                crate::managed_agents::persona_events::apply_persona_snapshot(record, persona);
                record.updated_at = crate::util::now_iso();
            }
        }
        save_managed_agents(app, &records)?;
        match records.iter().find(|record| record.pubkey == pubkey) {
            Some(record) => prepare_managed_agent_pending(app, state, record),
            None => Ok(None),
        }
    };
    finish_managed_agent_pending(app, state, retention).await;

    let mut errors = Vec::new();
    for relay_url in relay_urls {
        if let Err(error) = crate::managed_agents::start_managed_agent_runtime_pair_in_scope(
            pubkey.to_string(),
            relay_url.clone(),
            app.clone(),
            operation,
        ) {
            errors.push(format!("{relay_url}: {error}"));
        }
    }
    if !errors.is_empty() {
        return Err(format!(
            "failed to restart one or more managed-agent runtime pairs: {}",
            errors.join("; ")
        ));
    }

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let _admission = operation.admit(state)?;
    let records = load_managed_agents(app)?;
    let runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    summarize_from_disk(app, record, &runtimes)
}

pub(super) async fn start_local_agent_with_preflight(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    allow_fresh_create_start: bool,
    operation: &crate::user_operation::UserOperationScope,
    replay_floor_unix: Option<u64>,
) -> Result<ManagedAgentSummary, String> {
    let workspace_relay_url =
        crate::relay::bind_expected_relay_scope(None, operation.relay_url.clone())?;
    let workspace_owner =
        crate::relay::bind_expected_signer(None, operation.owner_pubkey.to_hex())?;
    let record_snapshot = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(app)?;
        records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .cloned()
            .ok_or_else(|| format!("agent {pubkey} not found"))?
    };

    if record_snapshot.backend != BackendKind::Local {
        return Err(format!("agent {pubkey} is not a local agent"));
    }

    // Preflight against the same resolution spawn uses — `resolve_effective_config`
    // (definition → global fallback). A linked instance's own `provider`/`model`/
    // `relay_mesh` bytes never contribute: this reads the CURRENT definition
    // directly, so a definition edit that flips `provider` to/from relay-mesh
    // between saves is reflected here without needing a prospective re-snapshot;
    // for a global-inherited blank definition, it also folds in the global
    // default, which record-byte sniffing could never see.
    let personas = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let _admission = operation.admit(state)?;
        load_personas(app).unwrap_or_default()
    };
    let global = crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    let mesh_model_id =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            &record_snapshot,
            &personas,
            &global,
        );
    ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), allow_fresh_create_start).await?;

    let (summary, retention) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let _admission = operation.admit(state)?;
        let mut records = load_managed_agents(app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let record = find_managed_agent_mut(&mut records, pubkey)?;
        if record.backend != BackendKind::Local {
            return Err(format!("agent {pubkey} is no longer a local agent"));
        }
        // Re-snapshot the persona onto the record at every spawn so the agent always
        // starts with the current persona config (system_prompt, model, provider,
        // runtime). This clears the "out of date" drift badge without requiring a
        // delete+recreate. See `apply_persona_snapshot` for the precedence and
        // env-override self-heal rules.
        // Load personas once: used for snapshot application below and summary build
        // at the end — avoids a second disk read for the same file in the same call.
        let personas = load_personas(app).unwrap_or_default();
        if let Some(persona_id) = record.persona_id.clone() {
            match personas.iter().find(|p| p.id == persona_id) {
                Some(persona) => {
                    crate::managed_agents::persona_events::apply_persona_snapshot(record, persona);
                    record.updated_at = crate::util::now_iso();
                }
                None => {
                    return Err(
                        crate::managed_agents::effective_config::ORPHANED_INSTANCE_ERROR
                            .to_string(),
                    );
                }
            }
        }
        start_managed_agent_process(
            app,
            record,
            &mut runtimes,
            Some(workspace_owner.as_str()),
            &workspace_relay_url,
            replay_floor_unix,
        )?;
        save_managed_agents(app, &records)?;
        let retention = match records.iter().find(|r| r.pubkey == pubkey) {
            Some(record) => prepare_managed_agent_pending(app, state, record),
            None => Ok(None),
        };
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        let summary = build_managed_agent_summary(
            app,
            record,
            &runtimes,
            &personas,
            &load_teams(app).unwrap_or_default(),
            &crate::managed_agents::load_global_agent_config(app).unwrap_or_default(),
        )?;
        (summary, retention)
    };
    finish_managed_agent_pending(app, state, retention).await;
    Ok(summary)
}

pub(crate) use provider_deploy::deploy_to_provider;

// Async so the blocking body (disk reads of agent/persona records, per-agent
// process-liveness syscalls, and a possible save) runs on Tauri's worker pool
// via spawn_blocking instead of the main UI thread — it was a beachball on the
// agents menu mount and after every start/stop/edit refetch. State is re-derived
// from the owned AppHandle inside the closure because `State<'_, _>` is borrowed
// and `std::sync::MutexGuard` is not `Send`.
#[tauri::command]
pub async fn list_managed_agents<R: tauri::Runtime>(
    app: AppHandle<R>,
    operation: crate::user_operation::UserOperationScope,
) -> Result<Vec<ManagedAgentSummary>, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        list_managed_agents_in_scope(&app, &state, &operation)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Blocking list core: reconciliation and persona loading can both persist state.
fn list_managed_agents_in_scope<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    operation: &crate::user_operation::UserOperationScope,
) -> Result<Vec<ManagedAgentSummary>, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let _admission = operation.admit(state)?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    let (sync_changed, exited_pubkeys) =
        sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(app));
    if sync_changed {
        save_managed_agents(app, &records)?;
    }
    for pubkey in &exited_pubkeys {
        state.clear_agent_session_caches(pubkey);
    }

    let personas = load_personas(app).unwrap_or_default();
    // One disk read for the whole list — build_managed_agent_summary takes
    // teams and config as parameters precisely so this poll-every-5s call
    // does not re-read them per record.
    let teams = load_teams(app).unwrap_or_default();
    let global_config = crate::managed_agents::load_global_agent_config(app).unwrap_or_default();
    records
        .iter()
        .map(|record| {
            build_managed_agent_summary(app, record, &runtimes, &personas, &teams, &global_config)
        })
        .collect()
}

/// Create's proof/definition preparation phase, completed before store writes.
pub(crate) async fn prepare_create_agent<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    owner: &crate::owner_authorization::OwnerAuthorizationScope,
    persona_id: Option<&str>,
) -> Result<crate::owner_authorization::AuthorizedAgent, String> {
    let agent = crate::owner_authorization::prepare_agent(&owner.signer).await?;
    owner.check_current(state)?;
    // load_personas merges and SAVES definitions. Defer it (not just explicit
    // housekeeping) until proof success, then recheck scope under the store lock.
    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let _admission = owner.admit(state)?;
        if let Some(persona_id) = persona_id {
            ensure_persona_is_active(&load_personas(app)?, persona_id)?;
        }
    }
    Ok(agent)
}

#[tauri::command]
pub async fn create_managed_agent(
    input: CreateManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CreateManagedAgentResponse, String> {
    crate::owner_authorization::require_owned_workspace(&state)?;
    let owner = crate::owner_authorization::OwnerAuthorizationScope::capture(&state)?;
    let name = input.name.trim().to_string();
    let requested_persona_id = input
        .persona_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    validate_create_definition(&name, requested_persona_id.as_deref(), &input)?;
    if let Some(parallelism) = input.parallelism {
        if !(1..=32).contains(&parallelism) {
            return Err("parallelism must be between 1 and 32".to_string());
        }
    }
    crate::managed_agents::validate_user_env_keys(&input.env_vars)?;

    // Validate & normalize the respond-to allowlist BEFORE any side effects.
    // The harness has its own validator (buzz-acp/src/config.rs) but we want
    // to catch malformed input at the boundary so the agent never tries to
    // start with a list that will crash it on launch. The mode/allowlist
    // pairing (and the definition-default fallback) is resolved later at the
    // mint site via `resolve_mint_behavioral_defaults`, where the linked
    // definition is in hand.
    let respond_to_allowlist =
        crate::managed_agents::validate_respond_to_allowlist(&input.respond_to_allowlist)?;
    if input.respond_to == Some(crate::managed_agents::RespondTo::Allowlist)
        && respond_to_allowlist.is_empty()
    {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".to_string(),
        );
    }

    let resolved_relay_url = input
        .relay_url
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_string();

    // ── Pre-Phase 2: validate provider config BEFORE any side effects ────────
    if let BackendKind::Provider { ref config, ref id } = input.backend {
        validate_provider_config(config)?;
        // Validate via discovered candidates — not raw resolve_command.
        resolve_provider_binary(id)?;
    }

    let relay_mesh = normalize_relay_mesh(input.relay_mesh.as_ref(), &input.backend)?;

    // Proof preparation is asynchronous, outside all store/SQLite locks.
    let crate::owner_authorization::AuthorizedAgent {
        keys: agent_keys,
        private_key_nsec,
        pubkey,
        auth_tag,
    } = prepare_create_agent(&app, &state, &owner, requested_persona_id.as_deref()).await?;
    owner.check_current(&state)?;

    // ── Phase 3: save record (sync lock) ───────────────────────────────────────
    let (agent, resolved_avatar_url, profile_about, retention) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let _admission = owner.admit(&state)?;
        if let Some(persona_id) = requested_persona_id.as_deref() {
            ensure_persona_is_active(&load_personas(&app)?, persona_id)?;
        }
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }

        // Guard against a duplicate pubkey appearing between phase 1 and phase 3
        // (extremely unlikely but safe to check).
        if records.iter().any(|record| record.pubkey == pubkey) {
            return Err(format!("agent {pubkey} already exists"));
        }
        // Provider config was already validated in Pre-Phase 2; cache the discovered binary path for deploy_to_provider.
        let provider_binary_path = if let BackendKind::Provider { ref id, .. } = input.backend {
            // Use resolve_provider_binary (discovered candidates only).
            resolve_provider_binary(id)
                .ok()
                .map(|p| p.display().to_string())
        } else {
            None
        };

        // Load personas once for harness/pack/avatar resolution below.
        let personas = load_personas(&app).unwrap_or_default();

        // Harness resolution: the persona's runtime is authoritative. A
        // persona-backed create stores an `agent_command_override` ONLY when the
        // user deliberately picked a divergent runtime (`harness_override`) —
        // e.g. AddChannelBotDialog's runtime selector. A divergence WITHOUT that
        // flag is a missing-runtime fallback from `resolvePersonaRuntime`, not a
        // pin, and must inherit so it doesn't freeze on the fallback harness once
        // the persona's runtime is installed. A persona-less create always
        // preserves the picked command as a real pin.
        let agent_command_override = crate::managed_agents::create_time_agent_command_override(
            requested_persona_id.as_deref(),
            &personas,
            input.agent_command.as_deref(),
            input.harness_override,
        );
        // The create-time snapshot used for arg/mcp/avatar derivations and
        // legacy reconcile. Authoritative spawn resolution re-derives this via
        // `effective_agent_command` at use-time.
        let agent_command = crate::managed_agents::effective_agent_command(
            requested_persona_id.as_deref(),
            &personas,
            agent_command_override.as_deref(),
        );
        let agent_args = normalize_agent_args(
            &agent_command,
            input
                .agent_args
                .iter()
                .map(|arg| arg.trim().to_string())
                .filter(|arg| !arg.is_empty())
                .collect::<Vec<_>>(),
        );

        // Derive MCP command exclusively from the runtime catalog — the
        // per-record field is never read at spawn time so user-supplied input
        // is silently discarded. Always sourcing from the catalog ensures
        // new agents pick up the correct value without any stored override.
        let mcp_command = match crate::managed_agents::known_acp_runtime(&agent_command) {
            Some(p) => p.mcp_command.unwrap_or("").to_string(),
            None => String::new(),
        };

        let team_id = input
            .team_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(team_id) = &team_id {
            if !load_teams(&app)?.iter().any(|team| &team.id == team_id) {
                return Err(format!("team {team_id} not found"));
            }
        }

        // Resolve the avatar URL once at creation and persist it on the record.
        // Explicit input wins, then the persona's own avatar, then the runtime
        // fallback. Storing it lets reconciliation compare against what was
        // actually published instead of re-deriving it.
        let persona_avatar_url = requested_persona_id.as_ref().and_then(|persona_id| {
            personas
                .iter()
                .find(|persona| persona.id == *persona_id)?
                .avatar_url
                .clone()
        });
        let resolved_avatar_url = resolve_created_avatar_url(
            input.avatar_url.as_deref(),
            persona_avatar_url,
            &agent_command,
        );

        // Pin the persona config onto the record at create. After this, spawn
        // and deploy read these snapshotted fields, never the live persona, so
        // the agent stays on the config it was created with across restarts;
        // delete+respawn re-runs create and rewrites the snapshot. env_vars are
        // NOT pinned: `record.env_vars` holds agent-level overrides only
        // (input.env_vars), and the live persona env is merged underneath at
        // read time (spawn / readiness / deploy) so persona credential edits
        // refresh on the next spawn like prompt/model/provider already do.
        let linked_persona = requested_persona_id.as_deref().and_then(|pid| {
            load_personas(&app)
                .ok()?
                .into_iter()
                .find(|persona| persona.id == pid)
        });
        let persona_snapshot = linked_persona
            .as_ref()
            .map(crate::managed_agents::persona_events::persona_snapshot);
        let snapshot_prompt = persona_snapshot
            .as_ref()
            .and_then(|s| s.system_prompt.clone());
        let snapshot_model = persona_snapshot.as_ref().and_then(|s| s.model.clone());
        let snapshot_provider = persona_snapshot.as_ref().and_then(|s| s.provider.clone());
        let snapshot_source_version = persona_snapshot.as_ref().map(|s| s.source_version.clone());
        let effective_provider = snapshot_provider
            .or_else(|| input.provider.as_deref().and_then(trim_to_optional_string));
        let mut effective_model =
            snapshot_model.or_else(|| input.model.as_deref().and_then(trim_to_optional_string));
        if effective_provider.as_deref() == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID)
            && effective_model.is_none()
        {
            effective_model = Some(crate::managed_agents::RELAY_MESH_AUTO_MODEL_ID.to_string());
        }

        // Mint-time behavioral quad: explicit input wins, then the linked
        // definition's NIP-AP defaults, then client defaults. The ONLY parse
        // point for definition behavioral strings — fails loudly on a bad
        // mode/range instead of minting an agent the author didn't describe.
        let minted = crate::managed_agents::resolve_mint_behavioral_defaults(
            input.respond_to,
            respond_to_allowlist.clone(),
            input.parallelism,
            linked_persona.as_ref(),
        )?;
        let record = ManagedAgentRecord {
            pubkey: pubkey.clone(),
            name: name.clone(),
            description: None,
            persona_id: requested_persona_id.clone(),
            team_id,
            private_key_nsec: private_key_nsec.clone(),
            auth_tag: auth_tag.clone(),
            relay_url: resolved_relay_url.clone(),
            avatar_url: resolved_avatar_url.clone(),
            acp_command: input
                .acp_command
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_ACP_COMMAND)
                .to_string(),
            agent_command,
            agent_command_override,
            agent_args,
            mcp_command,
            // BUZZ_ACP_TURN_TIMEOUT is deprecated and ignored by the harness;
            // store the schema default only. Use idle_timeout_seconds or
            // max_turn_duration_seconds for actual turn-length control.
            turn_timeout_seconds: DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
            // 0 or None → harness uses its own default (320s idle, 3600s max), and the CLI also clamps 0 → minimum.
            idle_timeout_seconds: input.idle_timeout_seconds.filter(|s| *s > 0),
            max_turn_duration_seconds: input.max_turn_duration_seconds.filter(|s| *s > 0),
            parallelism: minted.parallelism.unwrap_or(DEFAULT_AGENT_PARALLELISM),
            session_policy: linked_persona
                .as_ref()
                .map(|persona| persona.session_policy)
                .unwrap_or_default(),
            system_prompt: snapshot_prompt.or_else(|| {
                input
                    .system_prompt
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            }),
            model: effective_model.clone(),
            provider: effective_provider.clone(),
            persona_source_version: snapshot_source_version,
            // Provider agents are managed externally — force false.
            start_on_app_launch: if input.backend != BackendKind::Local {
                false
            } else {
                input.start_on_app_launch
            },
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: input.backend.clone(),
            backend_agent_id: None,
            provider_policy_pending: false,
            provider_binary_path,
            persona_team_dir: None,
            persona_name_in_team: None,
            env_vars: input.env_vars.clone(),
            created_at: now_iso(),
            updated_at: now_iso(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: minted.respond_to,
            respond_to_allowlist: minted.respond_to_allowlist.clone(),
            display_name: None,
            slug: None,
            runtime: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            definition_respond_to: None,
            definition_respond_to_allowlist: Vec::new(),
            definition_parallelism: None,
            relay_mesh: if effective_provider.as_deref()
                == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID)
            {
                effective_model
                    .clone()
                    .map(|model_ref| RelayMeshConfig { model_ref })
            } else {
                relay_mesh.clone()
            },
            effort_level: None,
        };

        records.push(record);

        save_managed_agents(&app, &records)?;

        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| "created agent disappeared unexpectedly".to_string())?;
        // Publish the agent to the relay. Inside the Phase-3 lock, after save,
        // before any .await — owner-authored, every agent (Will's ruling: no
        // is_builtin/persona-membership gate).
        let retention = prepare_managed_agent_pending(&app, &state, record);
        // Effective owner-authored description for the kind:0 `about`.
        let profile_about = crate::managed_agents::record_effective_description(record, &personas);
        (
            summarize_from_disk(&app, record, &runtimes)?,
            resolved_avatar_url,
            profile_about,
            retention,
        )
    };
    finish_managed_agent_pending(&app, &state, retention).await;

    // ── Phase 3b: local spawn (async preflight outside store lock) ───────────
    let mut spawn_error = None;
    let agent = if input.spawn_after_create && input.backend == BackendKind::Local {
        match start_local_agent_with_preflight(&app, &state, &pubkey, true, &owner.operation, None)
            .await
        {
            Ok(agent) => agent,
            Err(error) => {
                let _store_guard = state
                    .managed_agents_store_lock
                    .lock()
                    .map_err(|e| e.to_string())?;
                let mut records = load_managed_agents(&app)?;
                let runtimes = state
                    .managed_agent_processes
                    .lock()
                    .map_err(|e| e.to_string())?;
                let record = find_managed_agent_mut(&mut records, &pubkey)?;
                record.updated_at = now_iso();
                record.last_error = Some(error.clone());
                save_managed_agents(&app, &records)?;
                spawn_error = Some(error);
                let record = records
                    .iter()
                    .find(|record| record.pubkey == pubkey)
                    .ok_or_else(|| "created agent disappeared unexpectedly".to_string())?;
                summarize_from_disk(&app, record, &runtimes)?
            }
        }
    } else {
        agent
    };

    try_regenerate_nest(&app);

    // ── Phase 4: sync agent profile on relay (async, outside lock) ───────────
    // Use the avatar persisted on the record so the published profile and any
    // later reconciliation agree on the same value.
    let mut profile_sync_error = profile::publish_agent_profile_with_about(
        &state,
        &owner.relay_base,
        &agent_keys,
        &name,
        resolved_avatar_url.as_deref(),
        profile_about.as_deref(),
        auth_tag.as_deref(),
    )
    .await;
    profile_sync_error =
        super::agent_models::flush_managed_agent_policy(&app, &state, profile_sync_error).await;

    let spawn_error = if input.spawn_after_create && input.backend != BackendKind::Local {
        if let BackendKind::Provider { ref id, ref config } = input.backend {
            let agent_json = {
                let _g = state
                    .managed_agents_store_lock
                    .lock()
                    .map_err(|e| e.to_string())?;
                let records = load_managed_agents(&app)?;
                let rec = records
                    .iter()
                    .find(|r| r.pubkey == pubkey)
                    .ok_or_else(|| "agent disappeared".to_string())?;
                build_deploy_payload(&app, &state, rec)?
            };
            match deploy_to_provider(
                &app, &state, &pubkey, id, config, agent_json, None, None, None, None,
            )
            .await
            {
                Ok(()) => spawn_error,
                Err(e) => Some(e),
            }
        } else {
            spawn_error
        }
    } else {
        spawn_error
    };

    // Rebuild summary if provider deploy may have updated backend_agent_id.
    let final_agent = if input.backend != BackendKind::Local && spawn_error.is_none() {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let records = load_managed_agents(&app)?;
        let runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey)
            .ok_or_else(|| "agent disappeared".to_string())?;
        summarize_from_disk(&app, record, &runtimes)?
    } else {
        agent
    };

    Ok(CreateManagedAgentResponse {
        agent: final_agent,
        private_key_nsec,
        profile_sync_error,
        spawn_error,
    })
}

/// Data needed for background profile reconciliation after agent start.
#[tauri::command]
pub async fn start_managed_agent(
    pubkey: String,
    expected_relay_url: Option<String>,
    expected_signer_pubkey: Option<String>,
    replay_floor_unix: Option<u64>,
    app: AppHandle,
    state: State<'_, AppState>,
    operation: crate::user_operation::UserOperationScope,
) -> Result<ManagedAgentSummary, String> {
    crate::relay::assert_expected_signer(
        expected_signer_pubkey.as_deref(),
        &operation.owner_pubkey.to_hex(),
    )?;
    let reconcile_relay = crate::relay::bind_expected_relay_scope(
        expected_relay_url.as_deref(),
        operation.relay_url.clone(),
    )?;
    enum StartTarget {
        Local,
        Provider {
            backend: BackendKind,
            cached_binary_path: Option<String>,
            agent_json: serde_json::Value,
        },
    }

    // Collect backend info under lock; async preflight/spawn happens below.
    // Also snapshot profile reconciliation data for the background task.
    let (target, reconcile_data) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let _admission = operation.admit(&state)?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }

        let record = find_managed_agent_mut(&mut records, &pubkey)?;

        // Resolve the effective harness for the avatar-fallback derivation in
        // profile reconcile (the create-time snapshot may be empty or stale for
        // a persona-inherited harness).
        let reconcile_personas = load_personas(&app).unwrap_or_default();
        let mut reconcile = profile_reconcile_data(record, &reconcile_personas);
        // Pin the startup relay (the bound, caller-validated read) so the
        // fire-and-forget task can never resolve a post-switch workspace.
        // Mirrors `load_pending_profile_reconciliations`.
        reconcile.target_relay_url = Some(crate::relay::effective_agent_relay_url(
            &record.relay_url,
            reconcile_relay.as_str(),
        ));

        let target = if record.backend == BackendKind::Local {
            StartTarget::Local
        } else {
            StartTarget::Provider {
                backend: record.backend.clone(),
                cached_binary_path: record.provider_binary_path.clone(),
                agent_json: build_deploy_payload(&app, &state, record)?,
            }
        };

        (target, reconcile)
    };

    let result = match target {
        StartTarget::Local => {
            start_local_agent_with_preflight(
                &app,
                &state,
                &pubkey,
                false,
                &operation,
                replay_floor_unix,
            )
            .await
        }
        StartTarget::Provider {
            backend: BackendKind::Provider { id, config },
            cached_binary_path,
            agent_json,
        } => {
            // The caller's captured scope is asserted INSIDE deploy_to_provider
            // against the payload rebuilt after the deploy lock — the exact
            // payload invoked — so a switch racing the lock wait cannot deploy
            // the agent into the new tenant on behalf of a stale callback.
            // The replay floor rides along so a publish-first mention send's
            // remote harness replays past the already-published message, same
            // as the local spawn path.
            deploy_to_provider(
                &app,
                &state,
                &pubkey,
                &id,
                &config,
                agent_json,
                cached_binary_path.as_deref(),
                expected_relay_url.as_deref(),
                expected_signer_pubkey.as_deref(),
                replay_floor_unix,
            )
            .await?;

            // Return updated summary.
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|e| e.to_string())?;
            let records = load_managed_agents(&app)?;
            let runtimes = state
                .managed_agent_processes
                .lock()
                .map_err(|e| e.to_string())?;
            let record = records
                .iter()
                .find(|r| r.pubkey == pubkey)
                .ok_or_else(|| format!("agent {pubkey} not found"))?;
            summarize_from_disk(&app, record, &runtimes)
        }
        StartTarget::Provider { backend, .. } => Err(format!(
            "agent {pubkey} has unsupported backend kind: {backend:?}"
        )),
    };

    // ── Profile reconciliation (fire-and-forget) ────────────────────────────
    // On successful start, spawn a background task to ensure the agent's kind:0
    // profile is published on the relay. This self-heals cases where the initial
    // profile sync at creation time failed silently. For legacy records (pre-PR-921)
    // with no persisted avatar, this also backfills the avatar from the relay.
    if result.is_ok()
        && state
            .managed_agent_profile_reconcile_enabled()
            .load(std::sync::atomic::Ordering::Acquire)
    {
        let reconcile_pubkey = pubkey.clone();
        let reconcile_app = app.clone();
        tauri::async_runtime::spawn(async move {
            use tauri::Manager;
            let state = reconcile_app.state::<AppState>();
            if let Err(e) =
                reconcile_agent_profile(&state, &reconcile_app, &reconcile_pubkey, &reconcile_data)
                    .await
            {
                eprintln!(
                    "buzz-desktop: profile reconciliation failed for agent {reconcile_pubkey}: {e}"
                );
            }
        });
    }

    result
}

#[tauri::command]
pub async fn stop_managed_agent(
    pubkey: String,
    app: AppHandle,
    operation: crate::user_operation::UserOperationScope,
) -> Result<ManagedAgentSummary, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let _admission = operation.admit(&state)?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;

        let (sync_changed, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        if sync_changed {
            save_managed_agents(&app, &records)?;
        }
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }

        {
            let record = find_managed_agent_mut(&mut records, &pubkey)?;
            // Remote agents are stopped via !shutdown @mention from the frontend,
            // not via this backend command. Reject the call.
            if record.backend != BackendKind::Local {
                return Err(
                    "remote agents are stopped via !shutdown message, not this command".to_string(),
                );
            }
            // Pair-scoped: stops only the active workspace's pair; delete and
            // the config-restart flows still drain every pair.
            stop_managed_agent_workspace_pair(&app, record, &mut runtimes)?;
        }
        save_managed_agents(&app, &records)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        summarize_from_disk(&app, record, &runtimes)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

// Async so the blocking body (disk reads/writes, process termination, keyring
// delete, nest regeneration) runs off the main UI thread via spawn_blocking.
fn run_managed_agent_deletion<T>(
    base_dir: &std::path::Path,
    pubkey: &str,
    records: &mut Vec<ManagedAgentRecord>,
    delete: impl FnOnce(&mut Vec<ManagedAgentRecord>) -> Result<T, String>,
) -> Result<T, String> {
    recover_pending_assignment_cleanup(base_dir, |pending_pubkey| {
        records
            .iter()
            .any(|record| record.pubkey.eq_ignore_ascii_case(pending_pubkey))
    })?;
    with_agent_assignments_cleared(base_dir, pubkey, || delete(records))
}

#[path = "agents_deletion.rs"]
mod deletion;

#[tauri::command]
pub async fn delete_managed_agent(
    pubkey: String,
    force_remote_delete: Option<bool>,
    app: AppHandle,
) -> Result<(), String> {
    deletion::delete_managed_agent_with(pubkey, force_remote_delete, app).await
}

// Remote agent shutdown is handled entirely by the frontend:
// 1. Frontend sends "!shutdown" @mention via WebSocket (signed by user's key)
// 2. Harness sees it, exits gracefully, sets presence to "offline"
// 3. Desktop's existing presence polling sees "offline" — UI updates automatically
// No backend Tauri command needed. Presence IS the status.
#[path = "agents_deploy.rs"]
mod deploy;
pub(super) mod provider_access;
mod provider_deploy;
pub(super) use deploy::build_deploy_payload;
#[cfg(test)]
use deploy::{deploy_payload_json, DeployProjections};
#[cfg(test)]
use deploy::{ensure_remote_provider_supported, resolve_deploy_model_provider};

#[path = "agents_profile.rs"]
mod profile;
pub(crate) use profile::*;
#[cfg(test)]
use profile::{profile_needs_sync, resolve_legacy_avatar};

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agents_deletion_tests.rs"]
mod deletion_tests;

#[cfg(test)]
#[path = "agents_operation_tests.rs"]
mod operation_tests;
