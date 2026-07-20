use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::{
    app_state::AppState,
    managed_agents::{
        agent_events::ManagedAgentEventContent, apply_persona_behavior, current_instance_id,
        delete_agent_key, effective_agent_command, load_managed_agents, load_personas, load_teams,
        managed_agent_avatar_url, persona_events::persona_d_tag, save_managed_agents,
        save_personas, stop_managed_agent_process, sync_managed_agent_processes,
        team_events::TeamEventContent, try_regenerate_nest, validate_persona_activation_change,
        validate_persona_deletion, AgentDefinition, CreatePersonaRequest, ManagedAgentRecord,
        TeamRecord, UpdatePersonaRequest,
    },
    util::now_iso,
};

fn trim_required(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(trimmed.to_string())
}

fn trim_optional(value: Option<String>) -> Option<String> {
    value.and_then(|candidate| {
        let trimmed = candidate.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

mod pending;
pub(in crate::commands) use pending::retain_persona_pending;
pub(super) use pending::tombstone_persona_pending;

#[tauri::command]
pub async fn list_personas(app: AppHandle) -> Result<Vec<AgentDefinition>, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        load_personas(&app)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn create_persona(
    input: CreatePersonaRequest,
    app: AppHandle,
) -> Result<AgentDefinition, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let display_name = trim_required(&input.display_name, "Display name")?;
        // System prompt optional: core memory is auto-injected. Empty is valid.
        let system_prompt = input.system_prompt.trim().to_string();
        let avatar_url = trim_optional(input.avatar_url);
        let runtime = trim_optional(input.runtime);
        let model = trim_optional(input.model);
        let provider = trim_optional(input.provider);
        let now = now_iso();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut personas = load_personas(&app)?;
        let name_pool: Vec<String> = input
            .name_pool
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        crate::managed_agents::validate_user_env_keys(&input.env_vars)?;
        let mut persona = AgentDefinition {
            id: Uuid::new_v4().to_string(),
            display_name,
            avatar_url,
            system_prompt,
            runtime,
            model,
            provider,
            name_pool,
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: None,
            env_vars: input.env_vars,
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: now.clone(),
            updated_at: now,
        };
        apply_persona_behavior(&mut persona, input.behavior)?;
        personas.push(persona.clone());
        save_personas(&app, &personas)?;
        retain_persona_pending(&app, &state, &persona);
        try_regenerate_nest(&app);
        Ok(persona)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Return value of the `update_persona` command. Uses flatten so all
/// `AgentDefinition` fields appear at the top level of the JSON response —
/// backward-compatible with callers that already destructure a raw persona object.
#[derive(Debug, serde::Serialize)]
pub struct UpdatePersonaResult {
    #[serde(flatten)]
    persona: AgentDefinition,
}

/// Propagate a persona definition's display_name rename to linked agent instances.
/// Only instances whose current `name` equals `old_display_name` are updated;
/// pool-named instances (e.g. "Birch", "Compass") keep their individualised name.
/// Updates both `record.name` (relay display name) and `record.display_name`.
/// Returns the pubkeys of the records that were renamed.
fn propagate_persona_name_rename(
    records: &mut [ManagedAgentRecord],
    persona_id: &str,
    old_display_name: &str,
    new_display_name: &str,
) -> Vec<String> {
    let mut renamed = Vec::new();
    for record in records.iter_mut() {
        if record.persona_id.as_deref() != Some(persona_id) {
            continue;
        }
        if record.name != old_display_name {
            continue; // pool-named instance — keep its individualised name
        }
        record.name = new_display_name.to_string();
        record.display_name = Some(new_display_name.to_string());
        renamed.push(record.pubkey.clone());
    }
    renamed
}

#[tauri::command]
pub async fn update_persona(
    input: UpdatePersonaRequest,
    app: AppHandle,
) -> Result<UpdatePersonaResult, String> {
    use tauri::Manager;

    /// Profile sync params collected under the store lock for async relay publish.
    type ProfileSyncParams = Vec<(nostr::Keys, String, String, Option<String>, Option<String>)>;

    // Phase 1: synchronous save (persona record + linked agent avatar updates)
    let (result, profile_sync_params) = tokio::task::spawn_blocking({
        let app = app.clone();
        move || -> Result<(AgentDefinition, ProfileSyncParams), String> {
            let state = app.state::<AppState>();
            let display_name = trim_required(&input.display_name, "Display name")?;
            let system_prompt = input.system_prompt.clone();
            let avatar_url = trim_optional(input.avatar_url);
            let runtime = trim_optional(input.runtime);
            let model = trim_optional(input.model);
            let provider = trim_optional(input.provider);

            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let mut personas = load_personas(&app)?;
            let persona = personas
                .iter_mut()
                .find(|record| record.id == input.id)
                .ok_or_else(|| format!("agent {} not found", input.id))?;

            // Track what changed so we can propagate to linked agent records.
            let avatar_changed = persona.avatar_url != avatar_url;
            let name_changed = persona.display_name != display_name;
            let old_display_name = persona.display_name.clone();

            persona.display_name = display_name;
            persona.avatar_url = avatar_url;
            persona.system_prompt = system_prompt;
            persona.runtime = runtime;
            persona.model = model;
            persona.provider = provider;
            persona.name_pool = input
                .name_pool
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if let Some(env_vars) = input.env_vars {
                crate::managed_agents::validate_user_env_keys(&env_vars)?;
                persona.env_vars = env_vars;
            }
            apply_persona_behavior(persona, input.behavior)?;
            persona.updated_at = now_iso();

            let result = persona.clone();
            save_personas(&app, &personas)?;

            retain_persona_pending(&app, &state, &result);
            try_regenerate_nest(&app);

            // If the avatar or display_name changed, propagate to linked agent
            // records and collect relay profile sync params for the async phase.
            let sync_params: ProfileSyncParams = if avatar_changed || name_changed {
                let mut records = load_managed_agents(&app)?;
                let mut params: ProfileSyncParams = Vec::new();
                let mut agents_modified = false;
                let workspace_relay = crate::relay::relay_ws_url_with_override(&state);

                // Propagate the display_name rename to instances that still
                // carry the old definition display_name (pool-named instances
                // keep their individualised name) in one pass; the loop below
                // only decides which records need a relay profile sync.
                let renamed: Vec<String> = if name_changed {
                    propagate_persona_name_rename(
                        &mut records,
                        &result.id,
                        &old_display_name,
                        &result.display_name,
                    )
                } else {
                    Vec::new()
                };

                for record in records.iter_mut() {
                    if record.persona_id.as_deref() != Some(&result.id) {
                        continue;
                    }
                    let mut record_changed = renamed.contains(&record.pubkey);

                    if avatar_changed {
                        // Update the persisted avatar so reconciliation on next
                        // start agrees with what we're about to publish.
                        // When the persona avatar is cleared, fall back to the
                        // command-default icon so the record never stores `None`
                        // (which reconcile_agent_profile treats as "un-migrated").
                        let effective_cmd = effective_agent_command(
                            record.persona_id.as_deref(),
                            std::slice::from_ref(&result),
                            record.agent_command_override.as_deref(),
                        );
                        record.avatar_url = result
                            .avatar_url
                            .clone()
                            .or_else(|| managed_agent_avatar_url(&effective_cmd));
                        record_changed = true;
                    }

                    if record_changed {
                        agents_modified = true;
                        if let Ok(agent_keys) = nostr::Keys::parse(&record.private_key_nsec) {
                            let relay_url = crate::relay::effective_agent_relay_url(
                                &record.relay_url,
                                &workspace_relay,
                            );
                            params.push((
                                agent_keys,
                                relay_url,
                                record.name.clone(),
                                record.avatar_url.clone(),
                                record.auth_tag.clone(),
                            ));
                        }
                    }
                }

                if agents_modified {
                    save_managed_agents(&app, &records)?;
                }

                params
            } else {
                Vec::new()
            };

            Ok((result, sync_params))
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    // Phase 2: await relay profile sync for linked agents whose avatar or
    // display_name was just updated. We await (rather than fire-and-forget)
    // so the frontend cache invalidation that follows the mutation settlement
    // sees the fresh relay profile. Best-effort — failures are logged, not surfaced.
    if !profile_sync_params.is_empty() {
        let state = app.state::<AppState>();
        for (agent_keys, relay_url, display_name, avatar_url, auth_tag) in profile_sync_params {
            if let Err(e) = crate::relay::sync_managed_agent_profile(
                &state,
                &relay_url,
                &agent_keys,
                &display_name,
                avatar_url.as_deref(),
                auth_tag.as_deref(),
            )
            .await
            {
                eprintln!("buzz-desktop: relay profile sync failed after persona update: {e}");
            }
        }
    }

    Ok(UpdatePersonaResult { persona: result })
}

#[cfg(test)]
mod delete_cascade_tests;
#[cfg(test)]
mod inbound_tests;
#[cfg(test)]
mod name_propagation_tests;

/// Return pubkeys of every managed agent whose definition is the given persona.
///
/// Pure helper used by `delete_persona` to determine which agent records to
/// cascade-delete. Extracted so the filtering logic can be unit-tested without
/// a full Tauri `AppHandle`.
fn collect_cascade_pubkeys(agents: &[ManagedAgentRecord], persona_id: &str) -> Vec<String> {
    agents
        .iter()
        .filter(|a| a.persona_id.as_deref() == Some(persona_id))
        .map(|a| a.pubkey.clone())
        .collect()
}

/// Names of cascade agents that are provider-deployed: non-local backend with
/// a live `backend_agent_id`.
///
/// Pure helper used by `delete_persona`'s pre-flight: the cascade is refused
/// while any exist, because deleting the local record would orphan the remote
/// deployment. Mirrors `delete_managed_agent`'s `force_remote_delete` guard.
fn collect_remote_deployed(
    agents: &[ManagedAgentRecord],
    cascade: &std::collections::HashSet<String>,
) -> Vec<String> {
    agents
        .iter()
        .filter(|a| {
            cascade.contains(&a.pubkey)
                && a.backend != crate::managed_agents::BackendKind::Local
                && a.backend_agent_id.is_some()
        })
        .map(|a| a.name.clone())
        .collect()
}

/// Remove cascade agents from `agents` and persist via the injectable `save`.
///
/// Extracted from `delete_persona` so unit tests can inject a failing save and
/// verify retry-safety without a full `AppHandle` mock: if `save` returns `Err`,
/// this function propagates it before the keyring deletions and tombstones that
/// appear after the `?` in the call site — nothing is destroyed and the command
/// is safe to retry.
fn commit_cascade_agents(
    agents: &mut Vec<ManagedAgentRecord>,
    cascade: &std::collections::HashSet<String>,
    save: impl FnOnce(&[ManagedAgentRecord]) -> Result<(), String>,
) -> Result<(), String> {
    agents.retain(|a| !cascade.contains(&a.pubkey));
    save(agents)
}

#[tauri::command]
pub async fn delete_persona(id: String, app: AppHandle) -> Result<(), String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();

        {
            // Store lock held across all three phases.
            // Lock ordering: store lock (acquired here) → process lock (per-agent in Phase 2).
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;

            // Load and validate the persona before any destructive work.
            let mut personas = load_personas(&app)?;
            let persona = personas
                .iter()
                .find(|record| record.id == id)
                .ok_or_else(|| format!("persona {id} not found"))?;
            let referenced_by_team = load_teams(&app)?.iter().any(|team| {
                team.persona_ids
                    .iter()
                    .any(|persona_id| persona_id == id.as_str())
            });
            validate_persona_deletion(persona, referenced_by_team)?;
            // Capture the coordinate before the record might leave the list. Only
            // reached for non-builtin, non-team personas (both rejected above),
            // so every deleted persona here is one this owner published.
            let d_tag = crate::managed_agents::persona_events::persona_d_tag(persona);

            // ── Phase 1: Stage ─────────────────────────────────────────────
            //
            // Load agents, sync process state, and build the cascade set. Lock
            // ordering: store lock (held) → process lock (acquired for sync,
            // then released before Phase 2 stops). Every fallible read/lock is
            // here; an error leaves all state intact and the command is retryable.
            let mut agents = load_managed_agents(&app)?;
            {
                let mut runtimes = state
                    .managed_agent_processes
                    .lock()
                    .map_err(|error| error.to_string())?;
                let (sync_changed, exited_pubkeys) = sync_managed_agent_processes(
                    &mut agents,
                    &mut runtimes,
                    &current_instance_id(&app),
                );
                if sync_changed {
                    save_managed_agents(&app, &agents)?;
                }
                for pk in &exited_pubkeys {
                    state.clear_session_cache(pk);
                }
                // runtimes drops here (process lock released before Phase 2).
            }

            // Build the cascade set. HashSet for O(1) membership in Phase 3.
            let cascade: std::collections::HashSet<String> =
                collect_cascade_pubkeys(&agents, &id).into_iter().collect();

            // Remote-agent pre-flight: refuse the cascade before any destructive
            // work while any target is provider-deployed. Nothing in
            // create_managed_agent forbids a persona-linked provider agent, so
            // this must be a runtime guard, not an assumed invariant.
            let remote_deployed = collect_remote_deployed(&agents, &cascade);
            if !remote_deployed.is_empty() {
                return Err(format!(
                    "persona {id} has provider-deployed agent instances ({}); delete those agent instances first",
                    remote_deployed.join(", ")
                ));
            }

            // ── Phase 2: Stop ───────────────────────────────────────────────
            //
            // Best-effort stop each running cascade instance. Lock ordering:
            // store lock (held) → process lock acquired per-agent and released
            // between stops so the process lock is not held across the full poll
            // cycle (stop_managed_agent_process polls 100ms×10 before SIGKILL).
            //
            // Per-agent stop errors are swallowed — these records are deleted in
            // Phase 3 regardless. Intentional difference from delete_managed_agent
            // (single-agent, fatal on stop failure); here the cascade is multi-agent
            // and deletion must proceed even if one instance cannot be stopped.
            for pk in &cascade {
                if let Some(rec) = agents.iter_mut().find(|a| a.pubkey == *pk) {
                    let mut runtimes = state
                        .managed_agent_processes
                        .lock()
                        .map_err(|error| error.to_string())?;
                    if let Err(e) = stop_managed_agent_process(&app, rec, &mut runtimes) {
                        eprintln!("buzz-desktop: delete_persona: failed to stop agent {pk}: {e}");
                    }
                    // runtimes drops here (per-agent, process lock not held across stops).
                }
            }

            // ── Phase 3: Commit ─────────────────────────────────────────────
            //
            // Disk-authoritative writes first, side effects strictly after.
            // commit_cascade_agents is an injectable seam so unit tests can
            // verify retry-safety: a failing save propagates before any keyring
            // deletion or tombstone occurs.
            //
            // Failure semantics:
            //   agent save fails   → nothing destroyed; full cascade retries cleanly
            //   persona save fails → cascade agents gone, persona survives; a retry
            //                        finds an empty cascade and proceeds cleanly
            // Keys and tombstones are enqueued only after their records leave disk.
            if !cascade.is_empty() {
                commit_cascade_agents(&mut agents, &cascade, |recs| {
                    save_managed_agents(&app, recs)
                })?;
            }

            let original_len = personas.len();
            personas.retain(|record| record.id != id);
            if personas.len() == original_len {
                return Err(format!("persona {id} not found"));
            }
            save_personas(&app, &personas)?;

            // Side effects — strictly after records leave disk.
            for pk in &cascade {
                state.clear_session_cache(pk);
                // Remove nsec from keyring after the record is gone.
                delete_agent_key(pk);
                super::agents::tombstone_managed_agent_pending(&app, &state, pk);
                super::agents::archive_managed_agent_pending(&app, &state, pk);
            }
            tombstone_persona_pending(&app, &state, &d_tag);

            // _store_guard drops here, before try_regenerate_nest.
        }

        try_regenerate_nest(&app);

        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Apply an inbound kind:30175 persona event from the relay onto the local
/// store. The frontend's live subscription invokes this per event for our own
/// authored coordinate so Device B inherits Device A's edits.
///
/// Retention is a sync channel that writes INTO `personas.json`, never an
/// authoritative read source — `load_personas` is untouched, so every agent
/// keeps resolving its persona by UUID and keeps its provider keys.
///
/// MATCH KEY (single source of truth, both directions): an inbound event
/// matches the local record whose `persona_d_tag(record)` equals the event's
/// d-tag. Reusing the same derivation the outbound path uses guarantees the
/// inbound key can never drift from the outbound key — in particular, an
/// in-app persona (`source_team_persona_slug == None`) whose d-tag IS its
/// `id` matches its existing UUID row instead of minting a duplicate.
///
/// On match: patch ONLY the projected fields; preserve local `id`, `env_vars`,
/// `source_team`, and `created_at`. On no match: insert the parsed record as-is
/// — `persona_from_event` already sets `id = d_tag`, so an in-app persona reuses
/// its d-tag as the id and a re-received event stays idempotent (no duplicate).
///
/// The retention store decides whether the inbound event wins over a pending
/// local edit (`retain_inbound_event`): `personas.json` is only patched when the
/// retain reports [`InboundOutcome::Applied`], so an equal-second collision with
/// a pending local edit leaves the local record — and its queued publish —
/// untouched.
#[tauri::command]
pub async fn reconcile_inbound_persona_event(
    event_json: String,
    app: AppHandle,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || reconcile_inbound_persona_event_blocking(event_json, app))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

fn reconcile_inbound_persona_event_blocking(
    event_json: String,
    app: AppHandle,
) -> Result<(), String> {
    use crate::managed_agents::{
        agent_events::managed_agent_content_from_event,
        load_managed_agents, load_teams, managed_agents_base_dir,
        persona_events::persona_from_event,
        retention::{open_retention_db, retain_inbound_event, InboundOutcome, RetainedEvent},
        save_managed_agents, save_teams,
        team_events::team_content_from_event,
    };
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_MANAGED_AGENT, KIND_PERSONA, KIND_TEAM};
    use nostr::JsonUtil;

    let state = app.state::<AppState>();
    let event = parse_verified_inbound_event(&event_json)?;

    // The live filter subscribes to 30175/30176/30177 (upserts) plus kind:5
    // (NIP-09 deletions). d-tags are NOT unique across kinds, so every path
    // below dispatches on kind FIRST and only ever touches its own store — a
    // cross-kind d-tag collision can never link a team to a persona or agent.
    let kind = event.kind.as_u16() as u32;

    // kind:5 deletion: a tombstone removes the local record at the coordinate
    // in its `a` tag (`<target_kind>:<owner>:<d_tag>`). Handled before the
    // upsert dispatch because its coordinate and retention key differ.
    if kind == KIND_DELETION {
        return reconcile_inbound_tombstone(&event, &app, &state);
    }

    if !matches!(kind, KIND_PERSONA | KIND_TEAM | KIND_MANAGED_AGENT) {
        return Ok(());
    }

    // The d-tag identifies the record within its kind. Persona derives it from
    // the parsed record (`persona_d_tag`); team/agent carry it as the event's
    // d-tag directly. The persona is parsed once here and reused in the apply
    // branch below — team/agent content is parsed in-branch since their d-tag
    // comes from the event tag, not the content.
    let inbound_persona = (kind == KIND_PERSONA)
        .then(|| persona_from_event(&event))
        .transpose()?;
    let d_tag = match &inbound_persona {
        Some(persona) => persona_d_tag(persona),
        None => event_d_tag(&event)?,
    };

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    // Resolve inbound vs. any pending local edit before touching the store.
    let conn = open_retention_db(&managed_agents_base_dir(&app)?.join("retention.db"))?;
    let outcome = retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind,
            pubkey: event.pubkey.to_hex(),
            d_tag: d_tag.clone(),
            content: event.content.to_string(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )?;
    if outcome == InboundOutcome::Skipped {
        return Ok(());
    }

    match kind {
        KIND_PERSONA => {
            let mut personas = load_personas(&app)?;
            // `inbound_persona` is `Some` for KIND_PERSONA (set above).
            apply_inbound_persona(
                &mut personas,
                inbound_persona.expect("persona parsed above"),
            );
            save_personas(&app, &personas)?;
        }
        KIND_TEAM => {
            let mut teams = load_teams(&app)?;
            apply_inbound_team(&mut teams, d_tag, team_content_from_event(&event)?);
            save_teams(&app, &teams)?;
        }
        KIND_MANAGED_AGENT => {
            let mut agents = load_managed_agents(&app)?;
            apply_inbound_managed_agent(
                &mut agents,
                &d_tag,
                managed_agent_content_from_event(&event)?,
            );
            save_managed_agents(&app, &agents)?;
        }
        _ => unreachable!("kind gated above"),
    }
    try_regenerate_nest(&app);

    // Signal the live UI to refetch agents data — inbound relay events otherwise
    // land on disk silently, leaving the Agents tab stale until restart.
    let _ = app.emit("agents-data-changed", ());

    Ok(())
}

/// Parse an inbound wire event and enforce the signature gate. Everything
/// downstream trusts `event.pubkey` (ownership routing, tombstone scoping,
/// behavioral-quad application), so a forged pubkey must die here — the
/// TS-side owner filter reads the same attacker-controlled field and is no
/// defense.
fn parse_verified_inbound_event(event_json: &str) -> Result<nostr::Event, String> {
    use nostr::JsonUtil;
    let event = nostr::Event::from_json(event_json)
        .map_err(|e| format!("failed to parse inbound event: {e}"))?;
    event
        .verify()
        .map_err(|e| format!("inbound event failed signature verification: {e}"))?;
    Ok(event)
}

/// Parse a NIP-09 `a`-tag coordinate `<kind>:<owner_pubkey>:<d_tag>` into its
/// target kind and d-tag. Returns `None` if the tag is absent or malformed, so
/// the caller no-ops on a tombstone it can't route.
fn parse_deletion_coordinate(event: &nostr::Event) -> Option<(u32, String)> {
    event.tags.iter().find_map(|tag| {
        let values: Vec<&str> = tag.as_slice().iter().map(|s| s.as_str()).collect();
        if values.first() != Some(&"a") {
            return None;
        }
        let coord = values.get(1)?;
        // `<kind>:<owner>:<d_tag>` — d_tag may itself contain ':' so split at
        // most twice and keep the remainder as the d_tag.
        let mut parts = coord.splitn(3, ':');
        let kind: u32 = parts.next()?.parse().ok()?;
        let owner = parts.next()?;
        // NIP-09 scoping: only the record's author may tombstone it. The
        // signature gate upstream proves `event.pubkey`; requiring the
        // coordinate owner to match closes the other half — a validly
        // signed kind:5 naming ANOTHER owner's coordinate must no-op.
        if owner != event.pubkey.to_hex() {
            return None;
        }
        let d_tag = parts.next()?;
        Some((kind, d_tag.to_string()))
    })
}

/// Apply an inbound kind:5 NIP-09 deletion: remove the local record at the
/// tombstone's target coordinate, scoped per-kind. Mirrors the upsert spine —
/// retention resolution under the store lock, then a per-kind store mutation —
/// but removes rather than patches. Unknown/malformed coordinates no-op.
fn reconcile_inbound_tombstone(
    event: &nostr::Event,
    app: &AppHandle,
    state: &AppState,
) -> Result<(), String> {
    use crate::managed_agents::{
        load_managed_agents, load_teams, managed_agents_base_dir,
        retention::{
            open_retention_db, retain_inbound_event, tombstone_retention_d_tag, InboundOutcome,
            RetainedEvent,
        },
        save_managed_agents, save_teams,
    };
    use buzz_core_pkg::kind::{KIND_DELETION, KIND_MANAGED_AGENT, KIND_PERSONA, KIND_TEAM};
    use nostr::JsonUtil;

    let Some((target_kind, target_d_tag)) = parse_deletion_coordinate(event) else {
        return Ok(()); // no routable coordinate — nothing to delete
    };
    if !matches!(target_kind, KIND_PERSONA | KIND_TEAM | KIND_MANAGED_AGENT) {
        return Ok(()); // deletion for a kind we don't track locally
    }

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    // Resolve against the retained tombstone row (keyed by the target
    // coordinate, F2c) so a re-received tombstone or one older than a pending
    // local edit is a no-op.
    let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
    let outcome = retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_DELETION,
            pubkey: event.pubkey.to_hex(),
            d_tag: tombstone_retention_d_tag(target_kind, &target_d_tag),
            content: event.content.to_string(),
            created_at: event.created_at.as_secs() as i64,
            raw_event: event.as_json(),
            pending_sync: false,
        },
    )?;
    if outcome == InboundOutcome::Skipped {
        return Ok(());
    }

    // Remove the local record using the SAME per-kind match rule the apply fns
    // use: persona by `persona_d_tag`, team by `id`, managed-agent by `pubkey`.
    match target_kind {
        KIND_PERSONA => {
            let mut personas = load_personas(app)?;
            personas.retain(|record| persona_d_tag(record) != target_d_tag);
            save_personas(app, &personas)?;
        }
        KIND_TEAM => {
            let mut teams = load_teams(app)?;
            teams.retain(|record| record.id != target_d_tag);
            save_teams(app, &teams)?;
        }
        KIND_MANAGED_AGENT => {
            let mut agents = load_managed_agents(app)?;
            agents.retain(|record| record.pubkey != target_d_tag);
            save_managed_agents(app, &agents)?;
        }
        _ => unreachable!("target kind gated above"),
    }
    try_regenerate_nest(app);

    // Refresh the live UI on inbound deletion — a removal is as user-visible as
    // an upsert and the Agents tab must drop the tombstoned record without restart.
    let _ = app.emit("agents-data-changed", ());

    Ok(())
}

/// Extract the `d` tag value from an event, the match key for team (= team id)
/// and managed-agent (= agent pubkey) inbound reconcile.
fn event_d_tag(event: &nostr::Event) -> Result<String, String> {
    event
        .tags
        .iter()
        .find_map(|tag| {
            let values: Vec<&str> = tag.as_slice().iter().map(|s| s.as_str()).collect();
            (values.first() == Some(&"d"))
                .then(|| values.get(1).map(|s| s.to_string()))
                .flatten()
        })
        .ok_or_else(|| "inbound event missing d-tag".to_string())
}

/// Merge a parsed inbound persona into the local set: patch the matching record
/// in place, or push it when none matches.
///
/// The match key is `persona_d_tag` — the same derivation the outbound path
/// uses — so the inbound and outbound keys can never drift. On match, only the
/// projected fields are overwritten; local `id`, `env_vars`, `source_team`, and
/// `created_at` survive. On no match, the parsed record is inserted as-is; since
/// `persona_from_event` sets `id = d_tag`, an in-app persona reuses its d-tag as
/// the id and a re-received event stays idempotent (no duplicate row).
fn apply_inbound_persona(personas: &mut Vec<AgentDefinition>, inbound: AgentDefinition) {
    let d_tag = persona_d_tag(&inbound);
    match personas
        .iter_mut()
        .find(|record| persona_d_tag(record) == d_tag)
    {
        Some(local) => {
            local.display_name = inbound.display_name;
            local.avatar_url = inbound.avatar_url;
            local.system_prompt = inbound.system_prompt;
            local.runtime = inbound.runtime;
            local.model = inbound.model;
            local.provider = inbound.provider;
            local.name_pool = inbound.name_pool;
            local.respond_to = inbound.respond_to;
            local.respond_to_allowlist = inbound.respond_to_allowlist;
            local.parallelism = inbound.parallelism;
            local.updated_at = inbound.updated_at;
        }
        None => personas.push(inbound),
    }
}

/// Merge an inbound kind:30177 managed-agent projection into the local set.
///
/// Matches the local record whose `pubkey` equals the event's d-tag (the d-tag
/// IS the agent pubkey — see `build_agent_event`). On match, overwrite ONLY the
/// 10 projected fields; every secret (`private_key_nsec`, `auth_tag`,
/// `env_vars`, `backend`), the harness pins (`agent_command`,
/// `agent_command_override`), and all runtime/local fields are preserved
/// untouched. The projection type carries none of them, so they cannot be
/// reached here even if a foreign event tried to inject them.
///
/// No match is a no-op: managed agents carry device-local secrets and are never
/// minted from a relay event — an agent that does not already exist locally has
/// no secret key to run with, so inserting a secretless shell would be useless
/// and misleading. This diverges from the persona path, which DOES insert on no
/// match (personas are secretless definitions). Flagged in the reconcile docs.
fn apply_inbound_managed_agent(
    agents: &mut [ManagedAgentRecord],
    d_tag: &str,
    inbound: ManagedAgentEventContent,
) {
    if let Some(local) = agents.iter_mut().find(|record| record.pubkey == d_tag) {
        local.name = inbound.name;
        // Mirror of the slimmed writer (agent_event_content): a
        // definition-linked event omits the definition quad because those
        // fields resolve through the kind:30175 definition — absent means
        // "not carried", never "clear". Definition-less events still carry
        // the quad and apply it unconditionally (including clears).
        let definition_linked = inbound.persona_id.is_some();
        local.persona_id = inbound.persona_id;
        if !definition_linked {
            local.system_prompt = inbound.system_prompt;
            local.model = inbound.model;
            local.provider = inbound.provider;
            local.persona_source_version = inbound.persona_source_version;
        }
        local.parallelism = inbound.parallelism;
        local.respond_to = inbound.respond_to;
        local.respond_to_allowlist = inbound.respond_to_allowlist;
    }
}

/// Merge an inbound kind:30176 team projection into the local set.
///
/// Matches the local record whose `id` equals the event's d-tag (the d-tag IS
/// the team id — see `build_team_event`). On match, overwrite ONLY the three
/// shared fields (`name`, `description`, `persona_ids`); install-specific local
/// fields (`source_dir`, `is_symlink`, `symlink_target`, `is_builtin`,
/// `version`, `created_at`) are preserved. On no match, insert a fresh record
/// reusing the d-tag as the id so a re-received event stays idempotent —
/// symmetric to the persona path, since a team (like a persona) is a secretless
/// definition that another device may legitimately learn about from the relay.
fn apply_inbound_team(teams: &mut Vec<TeamRecord>, d_tag: String, inbound: TeamEventContent) {
    match teams.iter_mut().find(|record| record.id == d_tag) {
        Some(local) => {
            local.name = inbound.name;
            local.description = inbound.description;
            // `None` means the event came from a client that predates
            // always-publish — its true value is unknown, so preserve
            // local. Only `Some` (including the explicit-clear variants)
            // overwrites. See `TeamEventContent` for the wire rules.
            if let Some(instructions) = inbound.instructions {
                local.instructions = instructions;
            }
            if let Some(persona_ids) = inbound.persona_ids {
                local.persona_ids = persona_ids;
            }
        }
        None => teams.push(TeamRecord {
            id: d_tag,
            name: inbound.name,
            description: inbound.description,
            // Fresh insert has no local value to preserve; `None` from a
            // pre-fix client simply means no known value.
            instructions: inbound.instructions.unwrap_or_default(),
            persona_ids: inbound.persona_ids.unwrap_or_default(),
            is_builtin: false,
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: now_iso(),
            updated_at: now_iso(),
        }),
    }
}

#[tauri::command]
pub async fn set_persona_active(
    id: String,
    active: bool,
    app: AppHandle,
) -> Result<AgentDefinition, String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut personas = load_personas(&app)?;
        let persona = personas
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| format!("agent {id} not found"))?;

        let referenced_by_managed_agent = !active
            && load_managed_agents(&app)?
                .iter()
                .any(|agent| agent.persona_id.as_deref() == Some(id.as_str()));
        let referenced_by_team = !active
            && load_teams(&app)?.iter().any(|team| {
                team.persona_ids
                    .iter()
                    .any(|persona_id| persona_id == id.as_str())
            });

        validate_persona_activation_change(
            persona,
            active,
            referenced_by_managed_agent,
            referenced_by_team,
        )?;

        if persona.is_active == active {
            return Ok(persona.clone());
        }

        persona.is_active = active;
        persona.updated_at = now_iso();

        let updated = persona.clone();
        save_personas(&app, &personas)?;
        try_regenerate_nest(&app);
        Ok(updated)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

pub(crate) const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];
mod snapshot;
pub use snapshot::encode_agent_snapshot_for_send;
pub use snapshot::export_agent_snapshot;
pub(crate) use snapshot::import::{
    decode_snapshot_from_bytes, resolve_snapshot_import_behavior, MAX_SNAPSHOT_JSON_BYTES,
    MAX_SNAPSHOT_PNG_BYTES,
};
pub use snapshot::{confirm_agent_snapshot_import, preview_agent_snapshot_import};
