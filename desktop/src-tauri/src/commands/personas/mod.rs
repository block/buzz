use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use super::export_util::save_json_with_dialog;
use crate::{
    app_state::AppState,
    managed_agents::{
        agent_events::ManagedAgentEventContent, effective_agent_command, encode_persona_json,
        load_managed_agents, load_personas, load_teams, managed_agent_avatar_url,
        parse_json_persona, parse_md_persona, parse_png_persona, parse_zip_personas,
        persona_events::persona_d_tag, save_managed_agents, save_personas,
        team_events::TeamEventContent, team_persona_key, try_regenerate_nest,
        validate_persona_activation_change, validate_persona_deletion, CreatePersonaRequest,
        ManagedAgentRecord, ParsePersonaFilesResult, PersonaRecord, TeamRecord,
        UpdatePersonaRequest,
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

/// Retain a freshly authored persona event in the local store, flagged for
/// relay sync. Called inside a command's `managed_agents_store_lock`-held body
/// after `save_personas`; the background flush loop publishes it out-of-band.
///
/// The event is signed with the owner keys at call time, so its `created_at`
/// is `now` — newer than any prior retained row, clearing the upsert's
/// newer-or-equal guard. `pending_sync = 1` enqueues it for the flush loop,
/// which is the sole publisher. Best-effort: a failure here is logged and
/// swallowed so a retention hiccup never blocks the disk-authoritative write.
///
/// Unlike `retain_managed_agent_pending`, this has no projection-equality
/// short-circuit: personas have no start/stop runtime churn, so a republish
/// only happens on a genuine create/update/delete user edit (`set_persona_active`
/// does not retain, so the local-only `is_active` toggle never republishes, and
/// a byte-identical user-save republish is harmlessly NIP-33-replaced). The
/// guard is intentionally omitted.
fn retain_persona_pending(app: &AppHandle, state: &AppState, persona: &PersonaRecord) {
    use crate::managed_agents::{
        managed_agents_base_dir,
        persona_events::{build_persona_event, monotonic_created_at, persona_d_tag},
        retention::{get_retained_event, open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let d_tag = persona_d_tag(persona);
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        let (pubkey, event) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            // Monotonic created_at: read the retained head for this coordinate
            // and bump past it (NIP-AP step 3) so a same-second edit supersedes.
            let prior =
                get_retained_event(&conn, KIND_PERSONA, &keys.public_key().to_hex(), &d_tag)?
                    .map(|row| row.created_at);
            let event = build_persona_event(persona)?
                .custom_created_at(monotonic_created_at(prior))
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign persona event: {e}"))?;
            (keys.public_key().to_hex(), event)
        };
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_PERSONA,
                pubkey,
                d_tag,
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: persona-retain: {e}");
    }
}

/// Purge a deleted persona's pending row and enqueue a NIP-09 tombstone, both
/// inside the `managed_agents_store_lock`-held delete body.
///
/// PURGE IN: `delete_retained_event` removes the persona's `(30175, pubkey,
/// d_tag)` row. Running it under the same lock that serializes `retain_event`
/// closes the same-second resurrect race — a concurrent edit can't re-insert a
/// pending persona row after the tombstone is queued.
///
/// PUBLISH OUT: the kind:5 tombstone is retained at its own coordinate `(5,
/// pubkey, d_tag)` (distinct from the purged persona row) with `pending_sync =
/// 1`; the flush loop publishes it. Best-effort: a failure is logged and
/// swallowed so a retention hiccup never blocks the disk-authoritative delete.
pub(super) fn tombstone_persona_pending(app: &AppHandle, state: &AppState, d_tag: &str) {
    use crate::managed_agents::{
        managed_agents_base_dir,
        persona_events::build_persona_delete,
        retention::{
            delete_retained_event, open_retention_db, retain_event, tombstone_retention_d_tag,
            RetainedEvent,
        },
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    const KIND_DELETE: u32 = 5;

    let result = (|| -> Result<(), String> {
        let (pubkey, event) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            let pubkey = keys.public_key().to_hex();
            let event = build_persona_delete(d_tag, &pubkey)?
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign persona tombstone: {e}"))?;
            (pubkey, event)
        };
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        // Purge the persona row first so an unpublished edit can never resurrect
        // it after the tombstone publishes.
        delete_retained_event(&conn, KIND_PERSONA, &pubkey, d_tag)?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_DELETE,
                pubkey,
                // Key by the target coordinate so cross-kind d-tag tombstones
                // occupy distinct rows (F2c).
                d_tag: tombstone_retention_d_tag(KIND_PERSONA, d_tag),
                content: event.content.to_string(),
                created_at: event.created_at.as_secs() as i64,
                raw_event: event.as_json(),
                pending_sync: true,
            },
        )
    })();
    if let Err(e) = result {
        eprintln!("buzz-desktop: persona-tombstone: {e}");
    }
}

#[tauri::command]
pub async fn list_personas(app: AppHandle) -> Result<Vec<PersonaRecord>, String> {
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
) -> Result<PersonaRecord, String> {
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
        let persona = PersonaRecord {
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
            created_at: now.clone(),
            updated_at: now,
        };
        personas.push(persona.clone());
        save_personas(&app, &personas)?;
        retain_persona_pending(&app, &state, &persona);
        try_regenerate_nest(&app);
        Ok(persona)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn update_persona(
    input: UpdatePersonaRequest,
    app: AppHandle,
) -> Result<PersonaRecord, String> {
    use tauri::Manager;

    /// Profile sync params collected under the store lock for async relay publish.
    type ProfileSyncParams = Vec<(nostr::Keys, String, String, Option<String>, Option<String>)>;

    // Phase 1: synchronous save (persona record + linked agent avatar updates)
    let (result, profile_sync_params) = tokio::task::spawn_blocking({
        let app = app.clone();
        move || -> Result<(PersonaRecord, ProfileSyncParams), String> {
            let state = app.state::<AppState>();
            let display_name = trim_required(&input.display_name, "Display name")?;
            // Do not trim system_prompt: `compose_prompt` appends pack_instructions
            // verbatim (including any trailing newline), and write_back_persona_md
            // decomposes by suffix-stripping. Trimming would break that exact-suffix
            // match for the common case where instructions.md has a trailing newline.
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
                .ok_or_else(|| format!("persona {} not found", input.id))?;

            if persona.is_builtin {
                return Err("Built-in personas cannot be edited.".to_string());
            }

            // Track whether avatar changed so we can sync linked agents.
            let avatar_changed = persona.avatar_url != avatar_url;

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
            persona.updated_at = now_iso();

            save_personas(&app, &personas)?;
            let result = personas
                .into_iter()
                .find(|record| record.id == input.id)
                .ok_or_else(|| format!("persona {} disappeared unexpectedly", input.id))?;

            // For pack-backed personas, also write the edit back to the source
            // `.persona.md` so that launch sync (which reads the file) becomes a
            // no-op rather than overwriting the record we just saved.
            write_back_persona_md(&app, &result);

            retain_persona_pending(&app, &state, &result);
            try_regenerate_nest(&app);

            // If the avatar changed, propagate to linked agent records and
            // collect relay profile sync params for the async phase.
            let sync_params: ProfileSyncParams = if avatar_changed {
                let mut records = load_managed_agents(&app)?;
                let mut params: ProfileSyncParams = Vec::new();
                let mut agents_modified = false;
                let workspace_relay = crate::relay::relay_ws_url_with_override(&state);

                for record in records.iter_mut() {
                    if record.persona_id.as_deref() != Some(&result.id) {
                        continue;
                    }
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

    // Phase 2: await relay profile sync for linked agents whose avatar was
    // just updated. We await (rather than fire-and-forget) so the frontend
    // cache invalidation that follows the mutation settlement sees the fresh
    // relay profile. Best-effort — failures are logged, not surfaced.
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
                eprintln!(
                    "buzz-desktop: relay profile sync failed after persona avatar update: {e}"
                );
            }
        }
    }

    Ok(result)
}

mod writeback;
use writeback::write_back_persona_md;

#[cfg(test)]
mod inbound_tests;

#[tauri::command]
pub async fn delete_persona(id: String, app: AppHandle) -> Result<(), String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
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
        // Capture the coordinate before the record leaves the list. Only reached
        // for non-builtin, non-team personas (validate_persona_deletion rejects
        // both), so every deleted persona here is one this owner published.
        let d_tag = crate::managed_agents::persona_events::persona_d_tag(persona);

        let original_len = personas.len();
        personas.retain(|record| record.id != id);
        if personas.len() == original_len {
            return Err(format!("persona {id} not found"));
        }
        save_personas(&app, &personas)?;
        tombstone_persona_pending(&app, &state, &d_tag);

        let mut agents = load_managed_agents(&app)?;
        let mut changed_agents = false;
        let now = now_iso();
        for agent in &mut agents {
            if agent.persona_id.as_deref() == Some(id.as_str()) {
                agent.persona_id = None;
                agent.updated_at = now.clone();
                changed_agents = true;
            }
        }
        if changed_agents {
            save_managed_agents(&app, &agents)?;
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
    let event = nostr::Event::from_json(&event_json)
        .map_err(|e| format!("failed to parse inbound event: {e}"))?;

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
        let _owner = parts.next()?;
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
fn apply_inbound_persona(personas: &mut Vec<PersonaRecord>, inbound: PersonaRecord) {
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
        local.mcp_toolsets = inbound.mcp_toolsets;
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
            local.persona_ids = inbound.persona_ids;
        }
        None => teams.push(TeamRecord {
            id: d_tag,
            name: inbound.name,
            description: inbound.description,
            persona_ids: inbound.persona_ids,
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
) -> Result<PersonaRecord, String> {
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
            .ok_or_else(|| format!("persona {id} not found"))?;

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

const MAX_PNG_BYTES: usize = 10 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 5 * 1024 * 1024;
const MAX_ZIP_BYTES: usize = 100 * 1024 * 1024;

const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];
const ZIP_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
const JSON_OPEN_BRACE: u8 = 0x7B;

#[tauri::command]
pub async fn parse_persona_files(
    file_bytes: Vec<u8>,
    file_name: String,
) -> Result<ParsePersonaFilesResult, String> {
    tokio::task::spawn_blocking(move || {
        if file_bytes.len() > MAX_ZIP_BYTES {
            return Err("File is too large (max 100 MB).".to_string());
        }
        if file_bytes.is_empty() {
            return Err("File is empty.".to_string());
        }

        let first_byte = file_bytes[0];

        if file_bytes.len() >= 4 {
            let magic: [u8; 4] = file_bytes[..4]
                .try_into()
                .map_err(|_| "Failed to read file header".to_string())?;

            if magic == PNG_MAGIC {
                if file_bytes.len() > MAX_PNG_BYTES {
                    return Err("PNG file is too large (max 10 MB).".to_string());
                }
                let mut preview = parse_png_persona(&file_bytes)?;
                preview.source_file = file_name;
                return Ok(ParsePersonaFilesResult {
                    personas: vec![preview],
                    skipped: vec![],
                });
            }

            if magic == ZIP_MAGIC {
                return parse_zip_personas(&file_bytes);
            }
        }

        if first_byte == JSON_OPEN_BRACE {
            if file_bytes.len() > MAX_JSON_BYTES {
                return Err("JSON file is too large (max 5 MB).".to_string());
            }
            let mut preview = parse_json_persona(&file_bytes)?;
            preview.source_file = file_name;
            return Ok(ParsePersonaFilesResult {
                personas: vec![preview],
                skipped: vec![],
            });
        }

        // .persona.md: YAML frontmatter starts with "---"
        let lower_name = file_name.to_ascii_lowercase();
        if lower_name.ends_with(".persona.md") {
            if file_bytes.len() > MAX_JSON_BYTES {
                return Err("Markdown file is too large (max 5 MB).".to_string());
            }
            let mut preview = parse_md_persona(&file_bytes)?;
            preview.source_file = file_name;
            return Ok(ParsePersonaFilesResult {
                personas: vec![preview],
                skipped: vec![],
            });
        }

        // If it's a .md file but not .persona.md, give a specific hint.
        if lower_name.ends_with(".md") {
            return Err(
                "Only .persona.md files are supported. Rename to <name>.persona.md".to_string(),
            );
        }

        Err(
            "Unsupported file format. Expected .persona.md, .persona.png, .persona.json, or .zip"
                .to_string(),
        )
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn export_persona_to_json(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    // Load persona data under lock, then drop lock before dialog.
    //
    // NOTE: `env_vars` are deliberately NOT included in the exported card.
    // Persona cards are designed to be shareable artifacts (uploaded,
    // forked, distributed), and bundling API keys / credentials in them
    // would be a significant footgun. Users who import a card and need
    // credentials must supply them post-import via the persona dialog.
    let (display_name, system_prompt, avatar_url, runtime, model, provider, name_pool) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let personas = load_personas(&app)?;
        let persona = personas
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("persona {id} not found"))?;
        (
            persona.display_name.clone(),
            persona.system_prompt.clone(),
            persona.avatar_url.clone(),
            persona.runtime.clone(),
            persona.model.clone(),
            persona.provider.clone(),
            persona.name_pool.clone(),
        )
    };

    let json_bytes = encode_persona_json(
        &display_name,
        &system_prompt,
        avatar_url.as_deref(),
        runtime.as_deref(),
        model.as_deref(),
        provider.as_deref(),
        &name_pool,
    )?;

    let slug = crate::util::slugify(&display_name, "persona", 50);
    let filename = format!("{slug}.persona.json");
    save_json_with_dialog(&app, &filename, &json_bytes).await
}
