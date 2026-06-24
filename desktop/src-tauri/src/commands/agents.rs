use nostr::{Keys, ToBech32};
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, discover_provider_candidates, ensure_persona_is_active,
        find_managed_agent_mut, invoke_provider, load_managed_agents, load_personas,
        managed_agent_avatar_url, managed_agent_log_path, managed_agents_base_dir,
        normalize_agent_args, provider_deploy, read_log_tail, resolve_provider_binary,
        save_managed_agents, start_managed_agent_process, stop_managed_agent_process,
        sync_managed_agent_processes, try_regenerate_nest, validate_provider_config, BackendKind,
        BackendProviderInfo, CreateManagedAgentRequest, CreateManagedAgentResponse,
        ManagedAgentLogResponse, ManagedAgentRecord, ManagedAgentSummary, RelayMeshConfig,
        DEFAULT_ACP_COMMAND, DEFAULT_AGENT_PARALLELISM, DEFAULT_AGENT_TURN_TIMEOUT_SECONDS,
    },
    relay::{relay_ws_url_with_override, sync_managed_agent_profile},
    util::now_iso,
};

/// Read the workspace owner's pubkey hex from app state without holding the
/// lock for longer than necessary. Used to populate `BUZZ_ACP_AGENT_OWNER`
/// as a fallback for legacy agent records that have no NIP-OA `auth_tag`.
fn workspace_owner_hex(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
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
        return Err("relay mesh modelRef is required".to_string());
    }
    if backend != &BackendKind::Local {
        return Err("relay mesh agents must use the local backend".to_string());
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

async fn start_local_agent_with_preflight(
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
    start_managed_agent_process(app, record, &mut runtimes, Some(owner_hex))?;
    save_managed_agents(app, &records)?;
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    let personas = load_personas(app).unwrap_or_default();
    build_managed_agent_summary(app, record, &runtimes, &personas)
}

/// Build the standard agent JSON payload for provider deploy calls.
///
/// Fails closed if the agent points at a `persona_id` we can't load — persona
/// env_vars typically hold API credentials, and silently deploying with an
/// empty map would surface as an opaque 401 from the provider.
fn build_deploy_payload(
    app: &AppHandle,
    state: &AppState,
    record: &ManagedAgentRecord,
) -> Result<serde_json::Value, String> {
    // Merge persona env_vars + agent env_vars for provider deploy. Same
    // precedence as local spawn: persona first, agent overrides last. Without
    // this, provider-backed agents wouldn't receive credentials saved on the
    // persona or the agent itself.
    let persona_env =
        crate::managed_agents::resolve_persona_env(app, record.persona_id.as_deref())?;
    let merged_env = crate::managed_agents::merged_user_env(&persona_env, &record.env_vars);

    // Resolve effective model/provider from the persona's structured fields.
    // Agent record's model takes precedence (user override via UI).
    let personas = load_personas(app)
        .map_err(|e| format!("failed to load personas for deploy payload resolution: {e}"))?;
    let (effective_model, effective_provider) = if let Some(ref pid) = record.persona_id {
        let persona = personas.iter().find(|p| p.id == *pid);
        let model = record
            .model
            .clone()
            .or_else(|| persona.and_then(|p| p.model.clone()));
        let provider = persona.and_then(|p| p.provider.clone());
        (model, provider)
    } else {
        (record.model.clone(), None)
    };

    // Resolve the effective harness (persona-wins, override-honored) so the
    // remote provider runs the same harness a local spawn would — derive args
    // from it rather than the frozen record snapshot.
    let effective_command = crate::managed_agents::effective_agent_command(
        record.persona_id.as_deref(),
        &personas,
        record.agent_command_override.as_deref(),
    );
    let effective_args =
        crate::managed_agents::normalize_agent_args(&effective_command, record.agent_args.clone());

    Ok(serde_json::json!({
        "name": &record.name,
        // Resolve the per-agent pin against the active workspace relay here:
        // this payload crosses the host boundary to a remote provider harness
        // that has no notion of the desktop's workspace, so the blank→workspace
        // fallback (otherwise applied at read-time in `effective_agent_relay_url`)
        // must be materialized into a concrete URL before serializing.
        "relay_url": crate::relay::effective_agent_relay_url(
            &record.relay_url,
            &relay_ws_url_with_override(state),
        ),
        "private_key_nsec": &record.private_key_nsec,
        "auth_tag": &record.auth_tag,
        "agent_command": &effective_command,
        "agent_args": &effective_args,
        "system_prompt": &record.system_prompt,
        "model": effective_model,
        "provider": effective_provider,
        "turn_timeout_seconds": record.turn_timeout_seconds,
        "idle_timeout_seconds": record.idle_timeout_seconds,
        "max_turn_duration_seconds": record.max_turn_duration_seconds,
        "parallelism": record.parallelism,
        // Inbound author gate. Providers that don't yet read these fall back
        // to the harness default (`owner-only`) — no protocol break.
        "respond_to": record.respond_to,
        "respond_to_allowlist": &record.respond_to_allowlist,
        // Merged persona + agent env vars. Providers that don't read this
        // field will simply ignore it — no protocol break.
        "env_vars": merged_env,
    }))
}

/// Persist a deploy-preparation error (currently: persona env resolution
/// failure inside `build_deploy_payload`) into the agent's `last_error`
/// so a refresh shows the cause. Mirrors what `deploy_to_provider` does
/// on its own failures — without this, an agent created with an invalid
/// persona_id would appear as `not_deployed` with no recorded reason.
fn persist_create_deploy_error(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    error: &str,
) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(app)?;
    let rec = records
        .iter_mut()
        .find(|r| r.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    rec.last_error = Some(error.to_string());
    rec.updated_at = now_iso();
    save_managed_agents(app, &records)
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

#[tauri::command]
pub fn list_managed_agents(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ManagedAgentSummary>, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    if sync_managed_agent_processes(&mut records, &mut runtimes) {
        save_managed_agents(&app, &records)?;
    }

    let personas = load_personas(&app).unwrap_or_default();
    records
        .iter()
        .map(|record| build_managed_agent_summary(&app, record, &runtimes, &personas))
        .collect()
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
    // start with a list that will crash it on launch.
    let respond_to_allowlist =
        crate::managed_agents::validate_respond_to_allowlist(&input.respond_to_allowlist)?;
    if input.respond_to == crate::managed_agents::RespondTo::Allowlist
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

        if sync_managed_agent_processes(&mut records, &mut runtimes) {
            save_managed_agents(&app, &records)?;
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
        let owner_keys = state.keys.lock().map_err(|e| e.to_string())?;
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

        if sync_managed_agent_processes(&mut records, &mut runtimes) {
            save_managed_agents(&app, &records)?;
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

        let mcp_command = input
            .mcp_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(
                || match crate::managed_agents::known_acp_runtime(&agent_command) {
                    Some(p) => p.mcp_command.unwrap_or("").to_string(),
                    None => String::new(),
                },
            );

        // For pack-backed personas, resolve the installed pack path and the
        // persona's internal name (slug). ACP's resolve_persona_by_name()
        // matches on this internal name, NOT display_name.
        let pack_metadata: Option<(std::path::PathBuf, String)> =
            requested_persona_id.as_deref().and_then(|pid| {
                let persona = personas.iter().find(|p| p.id == pid)?;
                let team_id = persona.source_team.as_deref()?;
                let slug = persona.source_team_persona_slug.as_deref()?;
                let base = managed_agents_base_dir(&app).ok()?;
                let team_path = base.join("teams").join(team_id);
                // Use the validated slug stored during import — no need to
                // re-resolve the pack. The slug is [a-zA-Z0-9_-]+ by construction.
                Some((team_path, slug.to_owned()))
            });

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

        let record = crate::managed_agents::ManagedAgentRecord {
            pubkey: pubkey.clone(),
            name: name.clone(),
            persona_id: requested_persona_id.clone(),
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
            turn_timeout_seconds: input
                .turn_timeout_seconds
                .filter(|seconds| *seconds > 0)
                .unwrap_or(DEFAULT_AGENT_TURN_TIMEOUT_SECONDS),
            // 0 or None → harness uses its own default (320s idle, 3600s max), and the CLI also clamps 0 → minimum.
            idle_timeout_seconds: input.idle_timeout_seconds.filter(|s| *s > 0),
            max_turn_duration_seconds: input.max_turn_duration_seconds.filter(|s| *s > 0),
            parallelism: input
                .parallelism
                .filter(|count| (1..=32).contains(count))
                .unwrap_or(DEFAULT_AGENT_PARALLELISM),
            system_prompt: input
                .system_prompt
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            model: input
                .model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            mcp_toolsets: input
                .mcp_toolsets
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            // Provider agents are managed externally — force false.
            start_on_app_launch: if input.backend != BackendKind::Local {
                false
            } else {
                input.start_on_app_launch
            },
            runtime_pid: None,
            backend: input.backend.clone(),
            backend_agent_id: None,
            provider_binary_path,
            // Team-backed personas: record path + internal slug so the runtime
            // can resolve team config at startup. Must be the slug (e.g., "lep"),
            // NOT the display_name — ACP's resolve_persona_by_name() matches slugs.
            persona_team_dir: pack_metadata.as_ref().map(|(path, _)| path.clone()),
            persona_name_in_team: pack_metadata.as_ref().map(|(_, name)| name.clone()),
            env_vars: input.env_vars.clone(),
            created_at: now_iso(),
            updated_at: now_iso(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            respond_to: input.respond_to,
            respond_to_allowlist: respond_to_allowlist.clone(),
            relay_mesh: relay_mesh.clone(),
        };

        records.push(record);

        save_managed_agents(&app, &records)?;

        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| "created agent disappeared unexpectedly".to_string())?;
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
    let profile_sync_error = (sync_managed_agent_profile(
        &state,
        &resolved_relay_url,
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
                build_deploy_payload(&app, &state, rec)
            };
            // The agent was already persisted in Phase 3 — converting a
            // persona-resolution failure into `spawn_error` (rather than
            // unwinding) keeps the record on disk and surfaces the cause
            // in the agent's last_error / UI status. We persist the same
            // error string into `last_error` so a refresh after restart
            // still shows *why* deploy never happened, matching what
            // `deploy_to_provider` does on its own failures.
            match agent_json {
                Err(e) => {
                    if let Err(persist_err) = persist_create_deploy_error(&app, &state, &pubkey, &e)
                    {
                        eprintln!(
                            "buzz-desktop: failed to persist deploy-prep error for {pubkey}: {persist_err}"
                        );
                    }
                    Some(e)
                }
                Ok(json) => {
                    match deploy_to_provider(&app, &state, &pubkey, id, config, json, None).await {
                        Ok(()) => spawn_error,
                        Err(e) => Some(e),
                    }
                }
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
pub(crate) struct ProfileReconcileData {
    pub(crate) private_key_nsec: String,
    pub(crate) name: String,
    pub(crate) relay_url: String,
    /// Expected avatar URL for the published profile. `None` for legacy records
    /// that predate the `avatar_url` field — these will be backfilled from the
    /// relay's existing kind:0 profile on first reconciliation.
    pub(crate) avatar_url: Option<String>,
    pub(crate) auth_tag: Option<String>,
    /// The agent's pubkey (hex). Needed to update the persisted record during
    /// avatar backfill migration.
    pub(crate) pubkey: String,
    /// The agent's command (e.g. "goose"). Used as fallback when no profile
    /// exists on the relay during avatar backfill.
    pub(crate) agent_command: String,
    /// Persona ID if this agent was created from a persona. Used during avatar
    /// backfill to recover the correct avatar from the persona record when the
    /// relay profile has been corrupted.
    pub(crate) persona_id: Option<String>,
}

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

        if sync_managed_agent_processes(&mut records, &mut runtimes) {
            save_managed_agents(&app, &records)?;
        }

        let record = find_managed_agent_mut(&mut records, &pubkey)?;

        // Resolve the effective harness for the avatar-fallback derivation in
        // profile reconcile (the create-time snapshot may be empty or stale for
        // a persona-inherited harness).
        let reconcile_personas = load_personas(&app).unwrap_or_default();
        let reconcile_effective_command = crate::managed_agents::effective_agent_command(
            record.persona_id.as_deref(),
            &reconcile_personas,
            record.agent_command_override.as_deref(),
        );

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
    if result.is_ok() {
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

/// Resolve the avatar to backfill for a legacy agent record (pre-PR-921, no
/// stored `avatar_url`).
///
/// Priority: the persona's avatar wins, because the old reconciliation code
/// could have overwritten the relay's kind:0 `picture` with the command default
/// — making the relay an unreliable source for persona-backed agents. Only fall
/// back to the relay's `picture`, then the command icon, for agents with no
/// persona avatar to recover from.
fn resolve_legacy_avatar(
    persona_avatar: Option<String>,
    relay_picture: Option<String>,
    agent_command: &str,
) -> String {
    persona_avatar
        .or(relay_picture)
        .or_else(|| managed_agent_avatar_url(agent_command))
        .unwrap_or_default()
}

/// Reconcile an agent's kind:0 profile on the relay.
///
/// Queries the relay for the agent's existing profile and re-publishes if missing
/// or stale (display_name or picture mismatch). This is fire-and-forget — errors
/// are returned to the caller for logging but never block agent startup.
///
/// For legacy records (pre-PR-921) where `avatar_url` is `None`, this function
/// backfills via `resolve_legacy_avatar` — preferring the persona record's avatar
/// over the relay's `picture`, since the old code may have corrupted the relay
/// profile — and persists the updated record. After backfill, normal
/// reconciliation proceeds.
///
/// Query and publish target the relay returned by `effective_agent_relay_url`
/// for every agent regardless of backend: an explicit per-agent `relay_url`
/// wins, and a blank one falls back to the active workspace relay. This keeps
/// reconciliation following the session's relay for never-pinned agents while
/// honoring a deliberate pin wherever it points.
pub(crate) async fn reconcile_agent_profile(
    state: &AppState,
    app: &AppHandle,
    agent_pubkey: &str,
    data: &ProfileReconcileData,
) -> Result<(), String> {
    use crate::relay::{query_agent_profile, sync_managed_agent_profile};

    // An explicit per-agent relay wins; an empty one falls back to the active
    // workspace relay. Resolved once and used for both the read and write-back.
    let relay_url = crate::relay::effective_agent_relay_url(
        &data.relay_url,
        &relay_ws_url_with_override(state),
    );

    // Query the relay for the agent's existing kind:0 profile.
    let existing = query_agent_profile(state, &relay_url, agent_pubkey).await?;

    // Resolve the expected avatar — backfilling for legacy records that have no
    // stored avatar_url yet.
    let expected_avatar = match data.avatar_url.as_deref() {
        Some(url) => url.to_string(),
        None => {
            // Legacy record: the relay profile may have been corrupted by the
            // old reconciliation code (it overwrote the persona avatar with the
            // command default), so the persona record is the authoritative source.
            let persona_avatar = data.persona_id.as_ref().and_then(|pid| {
                load_personas(app)
                    .ok()?
                    .into_iter()
                    .find(|p| p.id == *pid)?
                    .avatar_url
            });

            let backfilled = resolve_legacy_avatar(
                persona_avatar,
                existing.as_ref().and_then(|info| info.picture.clone()),
                &data.agent_command,
            );

            // Persist the backfilled avatar so this migration only runs once.
            if !backfilled.is_empty() {
                let _store_guard = state
                    .managed_agents_store_lock
                    .lock()
                    .map_err(|e| e.to_string())?;
                let mut records = load_managed_agents(app)?;
                if let Some(record) = records.iter_mut().find(|r| r.pubkey == data.pubkey) {
                    record.avatar_url = Some(backfilled.clone());
                    save_managed_agents(app, &records)?;
                }
            }

            backfilled
        }
    };

    if expected_avatar.is_empty() {
        return Ok(());
    }

    if !profile_needs_sync(existing.as_ref(), &data.name, Some(&expected_avatar)) {
        return Ok(());
    }

    let agent_keys = Keys::parse(&data.private_key_nsec)
        .map_err(|e| format!("failed to parse agent keys: {e}"))?;

    sync_managed_agent_profile(
        state,
        &relay_url,
        &agent_keys,
        &data.name,
        Some(&expected_avatar),
        data.auth_tag.as_deref(),
    )
    .await
}

/// Decide whether a published profile is missing or stale relative to the
/// expected name and avatar. A missing profile always needs sync; a present
/// one is stale when either the display name or picture diverges.
fn profile_needs_sync(
    existing: Option<&crate::relay::AgentProfileInfo>,
    expected_name: &str,
    expected_avatar: Option<&str>,
) -> bool {
    match existing {
        None => true,
        Some(info) => {
            let name_matches = info.display_name.as_deref() == Some(expected_name);
            let picture_matches = info.picture.as_deref() == expected_avatar;
            !name_matches || !picture_matches
        }
    }
}

#[tauri::command]
pub fn stop_managed_agent(
    pubkey: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ManagedAgentSummary, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    if sync_managed_agent_processes(&mut records, &mut runtimes) {
        save_managed_agents(&app, &records)?;
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
    save_managed_agents(&app, &records)?;
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    let personas = load_personas(&app).unwrap_or_default();
    build_managed_agent_summary(&app, record, &runtimes, &personas)
}

#[tauri::command]
pub fn delete_managed_agent(
    pubkey: String,
    force_remote_delete: Option<bool>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
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

        if sync_managed_agent_processes(&mut records, &mut runtimes) {
            save_managed_agents(&app, &records)?;
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
            // For local agents: kills the process. For remote agents: no-op (the frontend
            // sends !shutdown via WebSocket before calling delete). Either way, safe.
            stop_managed_agent_process(&app, record, &mut runtimes)?;
        }
        let initial_len = records.len();
        records.retain(|record| record.pubkey != pubkey);
        if records.len() == initial_len {
            return Err(format!("agent {pubkey} not found"));
        }
        save_managed_agents(&app, &records)?;
    }
    try_regenerate_nest(&app);
    Ok(())
}

#[tauri::command]
pub fn get_managed_agent_log(
    pubkey: String,
    line_count: Option<u32>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ManagedAgentLogResponse, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let records = load_managed_agents(&app)?;
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found"))?;
    if record.backend != BackendKind::Local {
        return Err("logs are not available for remote agents".to_string());
    }

    let log_path = managed_agent_log_path(&app, &pubkey)?;
    Ok(ManagedAgentLogResponse {
        content: read_log_tail(&log_path, line_count.unwrap_or(120) as usize)?,
        log_path: log_path.display().to_string(),
    })
}

// ── New backend-provider commands ────────────────────────────────────────────

#[tauri::command]
pub fn discover_backend_providers() -> Vec<BackendProviderInfo> {
    discover_provider_candidates()
        .into_iter()
        .map(|(id, path)| BackendProviderInfo {
            id,
            binary_path: path.display().to_string(),
        })
        .collect()
}

#[tauri::command]
pub async fn probe_backend_provider(binary_path: String) -> Result<serde_json::Value, String> {
    // Validate that the requested path is actually a discovered buzz-backend-* binary.
    // This prevents arbitrary binary execution via a compromised frontend or IPC.
    let candidates = discover_provider_candidates();
    let path = std::path::PathBuf::from(&binary_path);
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("binary not found: {binary_path}: {e}"))?;
    let is_known = candidates
        .iter()
        .any(|(_, p)| p.canonicalize().ok().as_ref() == Some(&canonical));
    if !is_known {
        return Err(format!(
            "binary '{binary_path}' is not a discovered buzz-backend-* provider"
        ));
    }
    // request_id is for provider-side logging — not validated in the response
    // (stdin→stdout is 1:1 per process invocation).
    let request = serde_json::json!({
        "op": "info",
        "request_id": uuid::Uuid::new_v4().to_string(),
    });
    tokio::task::spawn_blocking(move || {
        invoke_provider(&canonical, &request, std::time::Duration::from_secs(10))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

// Remote agent shutdown is handled entirely by the frontend:
// 1. Frontend sends "!shutdown" @mention via WebSocket (signed by user's key)
// 2. Harness sees it, exits gracefully, sets presence to "offline"
// 3. Desktop's existing presence polling sees "offline" — UI updates automatically
// No backend Tauri command needed. Presence IS the status.

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
