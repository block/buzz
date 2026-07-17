use nostr::{Keys, ToBech32};
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, current_instance_id, discover_provider_candidates,
        ensure_persona_is_active, find_managed_agent_mut, load_managed_agents, load_personas,
        load_teams, managed_agent_avatar_url, managed_agents_base_dir, normalize_agent_args,
        provider_deploy, resolve_provider_binary, save_managed_agents, start_managed_agent_process,
        stop_managed_agent_process, sync_managed_agent_processes, try_regenerate_nest,
        validate_provider_config, BackendKind, CreateManagedAgentRequest,
        CreateManagedAgentResponse, ManagedAgentRecord, ManagedAgentSummary, RelayMeshConfig,
        DEFAULT_ACP_COMMAND, DEFAULT_AGENT_PARALLELISM, DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
    },
    relay::{relay_ws_url_with_override, sync_managed_agent_profile},
    util::now_iso,
};

/// Read the workspace owner's pubkey hex from app state without holding the
/// lock for longer than necessary. Used to populate `BUZZ_ACP_AGENT_OWNER`
/// as a fallback for legacy agent records that have no NIP-OA `auth_tag`.
pub(super) fn workspace_owner_hex(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

/// Retain a freshly authored managed-agent event in the local store, flagged
/// for relay sync. MUST be called inside the `managed_agents_store_lock`-held
/// body after `save_managed_agents`, NEVER across an `.await`: it acquires
/// `state.keys` and a retention-db connection, both `std::sync` guards, and
/// drops them before returning.
///
/// Owner-authored, mirroring `commands::personas::retain_persona_pending`: the
/// owner keys sign, the d_tag is the agent's pubkey, so the coordinate is
/// `30177:<owner>:<agent_pubkey>`. The event content is the opt-IN
/// [`agent_event_content`] projection — the retention upsert's content-equality
/// guard compares this projection, so an operational start/stop that mutates
/// only runtime fields produces an identical row and never re-enqueues a
/// publish. Best-effort: a failure here is logged and swallowed so a retention
/// hiccup never blocks the disk-authoritative write.
pub(super) fn retain_managed_agent_pending(
    app: &AppHandle,
    state: &AppState,
    record: &ManagedAgentRecord,
) {
    use crate::managed_agents::{
        agent_events::{agent_event_content, build_agent_event},
        persona_events::monotonic_created_at,
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        // The published content is the opt-IN projection JSON, independent of
        // signing and created_at. Compute it once to drive the no-republish
        // guard without signing twice.
        let content = serde_json::to_string(&agent_event_content(record))
            .map_err(|e| format!("failed to serialize managed-agent content: {e}"))?;
        let (owner_pubkey, event) = {
            let keys = state.signing_keys()?;
            let owner_pubkey = keys.public_key().to_hex();
            let existing =
                get_retained_event(&conn, KIND_MANAGED_AGENT, &owner_pubkey, &record.pubkey)?;
            // Skip re-publishing when the projection is unchanged: a start/stop
            // or any edit that touched only excluded runtime/local fields
            // produces an identical projection, so it is a no-op — operational
            // churn never re-enqueues a publish.
            if existing.as_ref().is_some_and(|row| row.content == content) {
                return Ok(());
            }
            // Monotonic created_at: bump past the retained head (NIP-AP step 3).
            let event = build_agent_event(record)?
                .custom_created_at(monotonic_created_at(existing.map(|row| row.created_at)))
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign managed-agent event: {e}"))?;
            (owner_pubkey, event)
        };
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_MANAGED_AGENT,
                pubkey: owner_pubkey,
                d_tag: record.pubkey.clone(),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: agent-retain: {e}");
    }
}

/// Purge a deleted agent's pending row and enqueue a NIP-09 tombstone, both
/// inside the `managed_agents_store_lock`-held delete body and NEVER across an
/// `.await`.
///
/// Mirrors `commands::personas::tombstone_persona_pending`: the agent row at
/// `(30177, owner, agent_pubkey)` is purged first so an unpublished edit can
/// never resurrect it after the tombstone publishes, then the kind:5 tombstone
/// is retained at its own `(5, owner, agent_pubkey)` coordinate with
/// `pending_sync = 1`. The `d_tag` is the agent's pubkey. Best-effort: a
/// failure is logged and swallowed so a retention hiccup never blocks the
/// disk-authoritative delete.
pub(super) fn tombstone_managed_agent_pending(
    app: &AppHandle,
    state: &AppState,
    agent_pubkey: &str,
) {
    use crate::managed_agents::{
        agent_events::build_agent_delete,
        retention::{
            delete_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
            RetainedEvent,
        },
    };
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let result = (|| -> Result<(), String> {
        let (owner_pubkey, event) = {
            let keys = state.signing_keys()?;
            let owner_pubkey = keys.public_key().to_hex();
            let event = build_agent_delete(agent_pubkey, &owner_pubkey)?
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign managed-agent tombstone: {e}"))?;
            (owner_pubkey, event)
        };
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        delete_retained_event(&conn, KIND_MANAGED_AGENT, &owner_pubkey, agent_pubkey)?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_DELETE,
                pubkey: owner_pubkey,
                // Key by the target coordinate so cross-kind d-tag tombstones
                // occupy distinct rows (F2c).
                d_tag: tombstone_retention_d_tag(KIND_MANAGED_AGENT, agent_pubkey),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: agent-tombstone: {e}");
    }
}

fn normalize_relay_mesh(
    config: Option<&RelayMeshConfig>,
    backend: &BackendKind,
) -> Result<Option<RelayMeshConfig>, String> {
    let Some(config) = config else {
        return Ok(None);
    };

    let model_ref = config.model_ref.trim();
    if model_ref.is_empty() {
        return Err("Buzz shared compute model is required".to_string());
    }
    if backend != &BackendKind::Local {
        return Err("Buzz shared compute agents must use the local backend".to_string());
    }

    Ok(Some(RelayMeshConfig {
        model_ref: model_ref.to_string(),
    }))
}

fn trim_to_optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_created_avatar_url(
    requested_avatar_url: Option<&str>,
    persona_avatar_url: Option<String>,
    agent_command: &str,
) -> Option<String> {
    requested_avatar_url
        .and_then(trim_to_optional_string)
        .or_else(|| {
            persona_avatar_url
                .as_deref()
                .and_then(trim_to_optional_string)
        })
        .or_else(|| managed_agent_avatar_url(agent_command))
}

#[cfg(feature = "mesh-llm")]
async fn ensure_relay_mesh_for_record(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    allow_fresh_create_start: bool,
) -> Result<(), String> {
    crate::commands::ensure_relay_mesh_for_record(app, record, allow_fresh_create_start).await
}

#[cfg(not(feature = "mesh-llm"))]
async fn ensure_relay_mesh_for_record(
    _app: &AppHandle,
    _record: &ManagedAgentRecord,
    _allow_fresh_create_start: bool,
) -> Result<(), String> {
    Ok(())
}

pub(super) async fn start_local_agent_with_preflight(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    owner_hex: &str,
    allow_fresh_create_start: bool,
) -> Result<ManagedAgentSummary, String> {
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

    ensure_relay_mesh_for_record(app, &record_snapshot, allow_fresh_create_start).await?;

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
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
        if let Some(persona) = personas.iter().find(|p| p.id == persona_id) {
            crate::managed_agents::persona_events::apply_persona_snapshot(record, persona);
            record.updated_at = crate::util::now_iso();
        }
    }
    start_managed_agent_process(app, record, &mut runtimes, Some(owner_hex))?;
    save_managed_agents(app, &records)?;
    if let Some(saved_record) = records.iter().find(|r| r.pubkey == pubkey) {
        retain_managed_agent_pending(app, state, saved_record);
    }
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    build_managed_agent_summary(app, record, &runtimes, &personas)
}

/// Deploy an agent to a provider backend. Resolves the binary, calls deploy via
/// spawn_blocking, and persists the result (backend_agent_id or last_error).
///
/// Idempotency: calling deploy on an already-deployed agent sends the same payload
/// again. Providers are expected to handle this as an update-in-place or no-op —
/// the protocol does not include an explicit `undeploy` operation (deferred to v2).
///
/// Returns Ok(()) on success, Err(message) on failure. Either way the record is
/// updated and saved before returning.
async fn deploy_to_provider(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    provider_id: &str,
    config: &serde_json::Value,
    agent_json: serde_json::Value,
    cached_binary_path: Option<&str>,
) -> Result<(), String> {
    // Resolve via discovered candidates only. Cached path must match BOTH
    // "is a discovered candidate" AND "belongs to this provider_id". A tampered
    // record cannot redirect deploys to a different provider's binary.
    let bin_path = cached_binary_path
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .map(|p| p.canonicalize().unwrap_or(p))
        .filter(|canonical| {
            discover_provider_candidates().iter().any(|(id, cp)| {
                id == provider_id && cp.canonicalize().ok().as_ref() == Some(canonical)
            })
        })
        .map_or_else(|| resolve_provider_binary(provider_id), Ok)?;

    let config_clone = config.clone();
    let deploy_result =
        tokio::task::spawn_blocking(move || provider_deploy(&bin_path, &agent_json, &config_clone))
            .await
            .map_err(|e| format!("spawn_blocking failed: {e}"))?;

    // Persist result under lock.
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(app)?;
    let rec = records
        .iter_mut()
        .find(|r| r.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;

    match deploy_result {
        Ok(backend_agent_id) => {
            rec.backend_agent_id = Some(backend_agent_id);
            rec.last_started_at = Some(now_iso());
            rec.updated_at = now_iso();
            rec.last_error = None;
        }
        Err(ref e) => {
            rec.last_error = Some(e.clone());
            rec.updated_at = now_iso();
            save_managed_agents(app, &records)?;
            return Err(e.clone());
        }
    }
    save_managed_agents(app, &records)?;
    Ok(())
}

// Async so the blocking body (disk reads of agent/persona records, per-agent
// process-liveness syscalls, and a possible save) runs on Tauri's worker pool
// via spawn_blocking instead of the main UI thread — it was a beachball on the
// agents menu mount and after every start/stop/edit refetch. State is re-derived
// from the owned AppHandle inside the closure because `State<'_, _>` is borrowed
// and `std::sync::MutexGuard` is not `Send`.
#[tauri::command]
pub async fn list_managed_agents(app: AppHandle) -> Result<Vec<ManagedAgentSummary>, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
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
            state.clear_session_cache(pubkey);
        }

        let personas = load_personas(&app).unwrap_or_default();
        records
            .iter()
            .map(|record| build_managed_agent_summary(&app, record, &runtimes, &personas))
            .collect()
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn create_managed_agent(
    input: CreateManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CreateManagedAgentResponse, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("agent name is required".to_string());
    }
    let requested_persona_id = input
        .persona_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
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

    // Snapshot the workspace owner pubkey for the legacy-record auth_tag
    // fallback. Computed outside the records lock to keep lock ordering simple.
    let owner_hex = workspace_owner_hex(&state)?;

    // ── Phase 1: generate keys (sync lock) ────────────────────────────────────
    let (agent_keys, private_key_nsec, pubkey, resolved_relay_url, input) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
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
            state.clear_session_cache(pubkey);
        }
        if let Some(persona_id) = requested_persona_id.as_deref() {
            let personas = load_personas(&app)?;
            ensure_persona_is_active(&personas, persona_id)?;
        }
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        if records.iter().any(|record| record.pubkey == pubkey) {
            return Err(format!("agent {pubkey} already exists"));
        }
        let private_key_nsec = keys
            .secret_key()
            .to_bech32()
            .map_err(|error| format!("failed to encode private key: {error}"))?;

        // Store the relay override exactly as supplied (trimmed). An explicit
        // value pins the agent; empty stays empty and resolves to the active
        // workspace relay at read-time. Uniform for Local and Provider.
        let resolved_relay_url = input
            .relay_url
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string();

        (keys, private_key_nsec, pubkey, resolved_relay_url, input)
    };

    // ── Pre-Phase 2: validate provider config BEFORE any side effects ────────
    if let BackendKind::Provider { ref config, ref id } = input.backend {
        validate_provider_config(config)?;
        // Validate via discovered candidates — not raw resolve_command.
        resolve_provider_binary(id)?;
    }

    let relay_mesh = normalize_relay_mesh(input.relay_mesh.as_ref(), &input.backend)?;

    // ── Phase 2: compute NIP-OA auth tag (sync) ──────────────────────────────
    // Agents authenticate via the auth tag in their kind:0 profile event.
    // No tokens are minted. Fail closed: bad auth tag → don't create agent.
    let auth_tag = {
        let owner_keys = state.signing_keys()?;
        // Bridge nostr 0.37 → 0.36 (buzz-sdk) via hex round-trip.
        let compat_owner = nostr::Keys::parse(&owner_keys.secret_key().to_secret_hex())
            .map_err(|e| format!("failed to bridge owner keys: {e}"))?;
        let compat_agent = nostr::PublicKey::from_hex(&agent_keys.public_key().to_hex())
            .map_err(|e| format!("failed to bridge agent pubkey: {e}"))?;
        let tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(&compat_owner, &compat_agent, "")
            .map_err(|e| format!("failed to compute NIP-OA auth tag: {e}"))?;
        Some(tag)
    };

    // ── Phase 3: save record (sync lock) ───────────────────────────────────────
    let (agent, resolved_avatar_url) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
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
            state.clear_session_cache(pubkey);
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

        let record = crate::managed_agents::ManagedAgentRecord {
            pubkey: pubkey.clone(),
            name: name.clone(),
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
            source_team: None,
            source_team_persona_slug: None,
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
        retain_managed_agent_pending(&app, &state, record);
        let personas = load_personas(&app).unwrap_or_default();
        (
            build_managed_agent_summary(&app, record, &runtimes, &personas)?,
            resolved_avatar_url,
        )
    };

    // ── Phase 3b: local spawn (async preflight outside store lock) ───────────
    let mut spawn_error = None;
    let agent = if input.spawn_after_create && input.backend == BackendKind::Local {
        match start_local_agent_with_preflight(&app, &state, &pubkey, &owner_hex, true).await {
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
                let personas = load_personas(&app).unwrap_or_default();
                build_managed_agent_summary(&app, record, &runtimes, &personas)?
            }
        }
    } else {
        agent
    };

    try_regenerate_nest(&app);

    // ── Phase 4: sync agent profile on relay (async, outside lock) ───────────
    // Use the avatar persisted on the record so the published profile and any
    // later reconciliation agree on the same value.
    let profile_relay_url = crate::relay::effective_agent_relay_url(
        &resolved_relay_url,
        &relay_ws_url_with_override(&state),
    );
    let profile_sync_error = (sync_managed_agent_profile(
        &state,
        &profile_relay_url,
        &agent_keys,
        &name,
        resolved_avatar_url.as_deref(),
        auth_tag.as_deref(),
    )
    .await)
        .err();

    // ── Phase 5: provider deploy (async, outside lock) ───────────────────────
    let spawn_error = if input.spawn_after_create && input.backend != BackendKind::Local {
        if let BackendKind::Provider { ref id, ref config } = input.backend {
            // Read the saved record to build the deploy payload (record has the
            // canonical field values after Phase 3 normalization).
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
            match deploy_to_provider(&app, &state, &pubkey, id, config, agent_json, None).await {
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
        let personas = load_personas(&app).unwrap_or_default();
        build_managed_agent_summary(&app, record, &runtimes, &personas)?
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
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ManagedAgentSummary, String> {
    // Snapshot the workspace owner pubkey for the legacy auth_tag fallback.
    // Read outside the records lock to keep lock ordering simple.
    let owner_hex = workspace_owner_hex(&state)?;
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
            state.clear_session_cache(pubkey);
        }

        let record = find_managed_agent_mut(&mut records, &pubkey)?;

        // Resolve the effective harness for the avatar-fallback derivation in
        // profile reconcile (the create-time snapshot may be empty or stale for
        // a persona-inherited harness).
        let reconcile_personas = load_personas(&app).unwrap_or_default();
        let reconcile_effective_command =
            crate::managed_agents::record_agent_command(record, &reconcile_personas);

        let reconcile = ProfileReconcileData {
            private_key_nsec: record.private_key_nsec.clone(),
            name: record.name.clone(),
            relay_url: record.relay_url.clone(),
            avatar_url: record.avatar_url.clone(),
            auth_tag: record.auth_tag.clone(),
            pubkey: record.pubkey.clone(),
            agent_command: reconcile_effective_command,
            persona_id: record.persona_id.clone(),
        };

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
            start_local_agent_with_preflight(&app, &state, &pubkey, &owner_hex, false).await
        }
        StartTarget::Provider {
            backend: BackendKind::Provider { id, config },
            cached_binary_path,
            agent_json,
        } => {
            deploy_to_provider(
                &app,
                &state,
                &pubkey,
                &id,
                &config,
                agent_json,
                cached_binary_path.as_deref(),
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
            let personas = load_personas(&app).unwrap_or_default();
            build_managed_agent_summary(&app, record, &runtimes, &personas)
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
            .managed_agent_profile_reconcile_enabled
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
) -> Result<ManagedAgentSummary, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
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
            state.clear_session_cache(pubkey);
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
            stop_managed_agent_process(&app, record, &mut runtimes)?;
        }
        state.clear_session_cache(&pubkey);
        save_managed_agents(&app, &records)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        let personas = load_personas(&app).unwrap_or_default();
        build_managed_agent_summary(&app, record, &runtimes, &personas)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

// Async so the blocking body (disk reads/writes, process termination, keyring
// delete, nest regeneration) runs off the main UI thread via spawn_blocking.
#[tauri::command]
pub async fn delete_managed_agent(
    pubkey: String,
    force_remote_delete: Option<bool>,
    app: AppHandle,
) -> Result<(), String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        {
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let mut records = load_managed_agents(&app)?;
            let mut runtimes = state
                .managed_agent_processes
                .lock()
                .map_err(|error| error.to_string())?;

            let (sync_changed, exited_pubkeys) = sync_managed_agent_processes(
                &mut records,
                &mut runtimes,
                &current_instance_id(&app),
            );
            if sync_changed {
                save_managed_agents(&app, &records)?;
            }
            for pubkey in &exited_pubkeys {
                state.clear_session_cache(pubkey);
            }

            // Guard: reject deletion of deployed remote agents unless explicitly forced.
            // This turns "don't orphan remote infra" from a UI convention into a backend
            // invariant — a buggy or compromised IPC caller cannot silently orphan a live
            // remote deployment. The frontend sends force_remote_delete: true only after
            // the user confirms the orphan warning.
            if let Some(record) = records.iter().find(|r| r.pubkey == pubkey) {
                if record.backend != BackendKind::Local
                    && record.backend_agent_id.is_some()
                    && !force_remote_delete.unwrap_or(false)
                {
                    return Err(
                        "cannot delete a deployed remote agent without force_remote_delete: true"
                            .to_string(),
                    );
                }
            }

            if let Some(record) = records.iter_mut().find(|record| record.pubkey == pubkey) {
                stop_managed_agent_process(&app, record, &mut runtimes)?;
            }
            state.clear_session_cache(&pubkey);
            let initial_len = records.len();
            records.retain(|record| record.pubkey != pubkey);
            if records.len() == initial_len {
                return Err(format!("agent {pubkey} not found"));
            }
            save_managed_agents(&app, &records)?;
            // Remove the agent's nsec from the keyring after the record is gone.
            crate::managed_agents::delete_agent_key(&pubkey);
            // Tombstone-after-validation: only reached past the deployed-remote
            // guard above and a confirmed removal — never orphan a live remote
            // deployment's relay record. Inside the lock, before the block closes
            // (no .await here). Every agent published, so every delete tombstones.
            tombstone_managed_agent_pending(&app, &state, &pubkey);
        }
        try_regenerate_nest(&app);
        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

// Remote agent shutdown is handled entirely by the frontend:
// 1. Frontend sends "!shutdown" @mention via WebSocket (signed by user's key)
// 2. Harness sees it, exits gracefully, sets presence to "offline"
// 3. Desktop's existing presence polling sees "offline" — UI updates automatically
// No backend Tauri command needed. Presence IS the status.

#[path = "agents_deploy.rs"]
mod deploy;
use deploy::build_deploy_payload;
#[cfg(test)]
use deploy::deploy_payload_json;
#[cfg(test)]
pub(crate) use deploy::resolve_deploy_model_provider;

#[path = "agents_profile.rs"]
mod profile;
#[cfg(test)]
use profile::{profile_needs_sync, resolve_legacy_avatar};
pub(crate) use profile::{reconcile_agent_profile, ProfileReconcileData};

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
