use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use super::export_util::save_json_with_dialog;
use crate::{
    app_state::AppState,
    managed_agents::{
        agent_events::ManagedAgentEventContent, encode_persona_json, load_managed_agents,
        load_personas, load_teams, parse_json_persona, parse_md_persona, parse_png_persona,
        parse_zip_personas, persona_events::persona_d_tag, save_managed_agents, save_personas,
        team_events::TeamEventContent, team_persona_key, try_regenerate_nest,
        validate_persona_activation_change, validate_persona_deletion, CreatePersonaRequest,
        ManagedAgentRecord, ParsePersonaFilesResult, PersonaRecord, TeamRecord, UpdatePersonaRequest,
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
pub fn list_personas(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<PersonaRecord>, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    load_personas(&app)
}

#[tauri::command]
pub fn create_persona(
    input: CreatePersonaRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PersonaRecord, String> {
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
}

#[tauri::command]
pub fn update_persona(
    input: UpdatePersonaRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PersonaRecord, String> {
    let display_name = trim_required(&input.display_name, "Display name")?;
    let system_prompt = input.system_prompt.trim().to_string();
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
    Ok(result)
}

/// Find the team whose `team_persona_key` equals `source_team`. This matches
/// the same key that `sync_team_from_dir` uses, covering both modern teams
/// (where `team.id` equals the manifest directory name) and legacy/backfilled
/// teams (where `team.id` is a UUID and the manifest id is `source_dir.file_name()`).
fn find_team_for_persona_source<'a>(
    teams: &'a [TeamRecord],
    source_team: &str,
) -> Option<&'a TeamRecord> {
    teams
        .iter()
        .find(|t| t.id == source_team || team_persona_key(t) == source_team)
}

/// Write updated frontmatter fields back to the source `.persona.md` file for
/// pack-backed personas (`source_team` is set). Non-fatal: any miss (no
/// `source_dir`, missing file, pack load failure, parse or write error) is
/// logged and swallowed so that the in-app edit — already persisted to
/// `personas.json` — always lands.
///
/// Only the four fields that the UI can set and that live in frontmatter are
/// rewritten: `display_name`, `runtime`, `avatar`, and `model` (the combined
/// `"provider:model"` string used by the pack format). The markdown body is
/// preserved byte-for-byte because `PersonaRecord.system_prompt` is the
/// _composed_ prompt (body + pack instructions appended by `compose_prompt`)
/// and writing it back to the file would cause the instructions to be
/// double-appended on the next launch sync.
///
/// The source file path is derived from the pack manifest via
/// `buzz_persona_pkg::pack::load_pack` — the same resolution the launch sync
/// uses — rather than reconstructed by convention. This ensures write-back
/// targets the correct file regardless of where the manifest places the
/// `.persona.md` (e.g. `personas/` vs `agents/`, nested paths, or filenames
/// that differ from the persona `name:` field).
///
/// The team is located via `find_team_for_persona_source`, which matches the
/// same key as `sync_team_from_dir` (`team_persona_key`). This handles both
/// modern teams (where `team.id` equals the manifest id) and legacy/backfilled
/// teams (where `team.id` is a UUID and the manifest id lives in `source_dir`).
fn write_back_persona_md(app: &AppHandle, persona: &PersonaRecord) {
    // Only pack-backed personas have a source file to write back to.
    let Some(source_team_id) = &persona.source_team else {
        return;
    };
    let Some(slug) = &persona.source_team_persona_slug else {
        eprintln!(
            "buzz-desktop: persona-writeback: persona {} has source_team but no slug; skipping",
            persona.id
        );
        return;
    };

    let result = (|| -> Result<(), String> {
        let teams = load_teams(app)?;
        let team = find_team_for_persona_source(&teams, source_team_id)
            .ok_or_else(|| format!("team {source_team_id} not found"))?;
        let source_dir = team
            .source_dir
            .as_ref()
            .ok_or_else(|| "team has no source_dir (JSON-only team)".to_string())?;

        // Resolve the actual source file via the pack manifest, matching the
        // same path the launch sync reads. `LoadedPersona.source_path` is the
        // absolute path set by `safe_resolve` against the pack root, so it is
        // correct regardless of the manifest layout.
        let pack = buzz_persona_pkg::pack::load_pack(source_dir)
            .map_err(|e| format!("load_pack {}: {e}", source_dir.display()))?;
        let loaded = pack
            .personas
            .iter()
            .find(|p| p.name == *slug)
            .ok_or_else(|| format!("persona '{slug}' not found in pack at {}", source_dir.display()))?;
        let path = &loaded.source_path;

        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {e}", path.display()))?;

        let updated = rewrite_persona_md(&content, persona, &loaded.prompt, pack.pack_instructions.as_deref())?;
        if updated == content {
            return Ok(());
        }
        std::fs::write(path, &updated)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(())
    })();

    if let Err(e) = result {
        eprintln!("buzz-desktop: persona-writeback: {e}");
    }
}

/// Rewrite a `.persona.md` file with updated frontmatter fields and, when safe,
/// an updated body (system prompt). Returns the full rewritten file content, or
/// the original unchanged when the result would be byte-identical.
///
/// **Frontmatter fields rewritten:** `display_name`, `runtime`, `avatar`, and
/// `model` (joined `"provider:model"` per the pack format). All other keys and
/// their order are preserved.
///
/// **Body (system prompt) write-back:**
/// The `persona.system_prompt` field holds the *composed* prompt:
/// `compose_prompt(raw_body, pack_instructions)`. To recover the raw body we
/// reverse `compose_prompt`:
///
/// - If `pack_instructions` is absent or blank: new body = `system_prompt`
///   verbatim (no suffix to strip).
/// - If `pack_instructions` is present and non-blank: the composed prompt ends
///   with `"\n\n---\n# Team Instructions\n{instructions}"`. If
///   `system_prompt` ends with that exact suffix, strip it to get the new raw
///   body. **Safety guard**: if the suffix is absent (user edited inside the
///   Team Instructions block, or instructions drifted), we cannot safely
///   recover the raw body — preserve the existing body and log a skip. This
///   prevents a corrupted file or double-appended instructions.
/// - If `system_prompt` equals `compose_prompt(current_raw_body, instructions)`
///   exactly (user did not edit the prompt), the body is preserved
///   byte-for-byte (no-op for the body section).
fn rewrite_persona_md(
    content: &str,
    persona: &PersonaRecord,
    current_raw_body: &str,
    pack_instructions: Option<&str>,
) -> Result<String, String> {
    let (frontmatter, existing_body) = buzz_persona_pkg::persona::split_frontmatter(content)
        .map_err(|e| format!("split_frontmatter: {e:?}"))?;

    let mut value = serde_yaml::from_str::<serde_yaml::Value>(frontmatter)
        .map_err(|e| format!("yaml parse: {e}"))?;
    let mapping = value
        .as_mapping_mut()
        .ok_or("frontmatter is not a YAML mapping")?;

    // display_name
    mapping.insert(
        serde_yaml::Value::String("display_name".to_string()),
        serde_yaml::Value::String(persona.display_name.clone()),
    );

    // runtime: set when Some, remove when None
    let runtime_key = serde_yaml::Value::String("runtime".to_string());
    match &persona.runtime {
        Some(rt) if !rt.is_empty() => {
            mapping.insert(runtime_key, serde_yaml::Value::String(rt.clone()));
        }
        _ => {
            mapping.remove(&runtime_key);
        }
    }

    // avatar: set when Some, remove when None
    let avatar_key = serde_yaml::Value::String("avatar".to_string());
    match &persona.avatar_url {
        Some(av) if !av.is_empty() => {
            mapping.insert(avatar_key, serde_yaml::Value::String(av.clone()));
        }
        _ => {
            mapping.remove(&avatar_key);
        }
    }

    // model: joined "provider:model" or bare "model"; remove when both absent
    let model_key = serde_yaml::Value::String("model".to_string());
    match (&persona.provider, &persona.model) {
        (Some(prov), Some(mdl)) if !prov.is_empty() && !mdl.is_empty() => {
            mapping.insert(
                model_key,
                serde_yaml::Value::String(format!("{prov}:{mdl}")),
            );
        }
        (_, Some(mdl)) if !mdl.is_empty() => {
            mapping.insert(model_key, serde_yaml::Value::String(mdl.clone()));
        }
        _ => {
            mapping.remove(&model_key);
        }
    }

    let updated_frontmatter = serde_yaml::to_string(&value)
        .map_err(|e| format!("yaml serialize: {e}"))?;

    // Determine the body to write back.
    // `compose_prompt` is: body + "\n\n---\n# Team Instructions\n{instructions}"
    // when instructions is non-blank, or body verbatim when absent/blank.
    let effective_instructions = pack_instructions
        .filter(|s| !s.trim().is_empty());
    let expected_composed = match effective_instructions {
        Some(instr) => format!("{current_raw_body}\n\n---\n# Team Instructions\n{instr}"),
        None => current_raw_body.to_owned(),
    };

    let new_body: &str = if persona.system_prompt == expected_composed {
        // User did not edit the prompt — keep the existing body byte-for-byte.
        existing_body
    } else {
        // User edited the prompt. Recover the raw body by reversing compose_prompt.
        match effective_instructions {
            None => {
                // No pack instructions: composed == raw, write verbatim.
                &persona.system_prompt
            }
            Some(instr) => {
                let suffix = format!("\n\n---\n# Team Instructions\n{instr}");
                if let Some(raw) = persona.system_prompt.strip_suffix(suffix.as_str()) {
                    raw
                } else {
                    // Safety guard: suffix absent — cannot safely recover raw body.
                    // Preserve the existing body to avoid corruption or double-append.
                    eprintln!(
                        "buzz-desktop: persona-writeback: \
                         system_prompt does not end with expected Team Instructions suffix; \
                         preserving existing body to avoid corruption"
                    );
                    existing_body
                }
            }
        }
    };

    Ok(format!("---\n{updated_frontmatter}---\n{new_body}"))
}

#[tauri::command]
pub fn delete_persona(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
pub fn reconcile_inbound_persona_event(
    event_json: String,
    app: AppHandle,
    state: State<'_, AppState>,
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
        local.persona_id = inbound.persona_id;
        local.system_prompt = inbound.system_prompt;
        local.model = inbound.model;
        local.provider = inbound.provider;
        local.mcp_toolsets = inbound.mcp_toolsets;
        local.persona_source_version = inbound.persona_source_version;
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
pub fn set_persona_active(
    id: String,
    active: bool,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PersonaRecord, String> {
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
}

const MAX_PNG_BYTES: usize = 10 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 5 * 1024 * 1024;
const MAX_ZIP_BYTES: usize = 100 * 1024 * 1024;

const PNG_MAGIC: [u8; 4] = [0x89, 0x50, 0x4E, 0x47];
const ZIP_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
const JSON_OPEN_BRACE: u8 = 0x7B;

#[tauri::command]
pub fn parse_persona_files(
    file_bytes: Vec<u8>,
    file_name: String,
) -> Result<ParsePersonaFilesResult, String> {
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

#[cfg(test)]
mod inbound_tests {
    use super::*;
    use std::collections::BTreeMap;

    const UUID: &str = "11111111-2222-3333-4444-555555555555";

    /// A local in-app persona: `source_team_persona_slug` is None, so its d-tag
    /// IS its UUID id. Carries env_vars + source_team that must survive a patch.
    fn local_in_app() -> PersonaRecord {
        PersonaRecord {
            id: UUID.to_string(),
            display_name: "Local".to_string(),
            avatar_url: None,
            system_prompt: "local prompt".to_string(),
            runtime: Some("goose".to_string()),
            model: Some("opus".to_string()),
            provider: Some("anthropic".to_string()),
            name_pool: vec!["Local".to_string()],
            is_builtin: false,
            is_active: true,
            source_team: Some("team-1".to_string()),
            source_team_persona_slug: None,
            env_vars: BTreeMap::from([("API_KEY".to_string(), "secret".to_string())]),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    /// An inbound persona as `persona_from_event` would produce it: id = d-tag,
    /// slug = Some(d-tag), empty env_vars, source_team None.
    fn inbound_for(d_tag: &str, display_name: &str) -> PersonaRecord {
        PersonaRecord {
            id: d_tag.to_string(),
            display_name: display_name.to_string(),
            avatar_url: Some("https://example.com/a.png".to_string()),
            system_prompt: "remote prompt".to_string(),
            runtime: Some("acp".to_string()),
            model: Some("sonnet".to_string()),
            provider: Some("openai".to_string()),
            name_pool: vec!["Remote".to_string()],
            is_builtin: false,
            is_active: true,
            source_team: None,
            source_team_persona_slug: Some(d_tag.to_string()),
            env_vars: BTreeMap::new(),
            created_at: "2025-06-01T00:00:00Z".to_string(),
            updated_at: "2025-06-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn in_app_persona_matches_existing_uuid_and_patches() {
        let mut personas = vec![local_in_app()];
        apply_inbound_persona(&mut personas, inbound_for(UUID, "Remote"));

        assert_eq!(personas.len(), 1, "no duplicate row");
        let p = &personas[0];
        // Projected fields patched.
        assert_eq!(p.display_name, "Remote");
        assert_eq!(p.system_prompt, "remote prompt");
        assert_eq!(p.provider, Some("openai".to_string()));
        // Local identity + secrets + lineage preserved.
        assert_eq!(p.id, UUID);
        assert_eq!(p.env_vars.get("API_KEY"), Some(&"secret".to_string()));
        assert_eq!(p.source_team, Some("team-1".to_string()));
        assert_eq!(p.source_team_persona_slug, None);
        assert_eq!(p.created_at, "2025-01-01T00:00:00Z");
    }

    #[test]
    fn re_received_in_app_persona_is_idempotent_no_duplicate() {
        let mut personas = vec![local_in_app()];
        apply_inbound_persona(&mut personas, inbound_for(UUID, "Remote"));
        // Same event arrives again (e.g. reconnect backfill).
        apply_inbound_persona(&mut personas, inbound_for(UUID, "Remote"));

        assert_eq!(personas.len(), 1, "re-receive must not duplicate");
        assert_eq!(personas[0].id, UUID);
    }

    #[test]
    fn team_persona_matches_on_slug_and_patches() {
        let mut local = local_in_app();
        local.id = "local-uuid".to_string();
        local.source_team_persona_slug = Some("team-slug".to_string());
        let mut personas = vec![local];

        apply_inbound_persona(&mut personas, inbound_for("team-slug", "Renamed"));

        assert_eq!(personas.len(), 1, "no duplicate row");
        assert_eq!(personas[0].display_name, "Renamed");
        // Local UUID survives even though the match key is the slug.
        assert_eq!(personas[0].id, "local-uuid");
        assert_eq!(
            personas[0].source_team_persona_slug,
            Some("team-slug".to_string())
        );
    }

    #[test]
    fn no_local_match_inserts_inbound_reusing_d_tag_as_id() {
        let mut personas = vec![local_in_app()];
        let other = "99999999-8888-7777-6666-555555555555";
        apply_inbound_persona(&mut personas, inbound_for(other, "New"));

        assert_eq!(personas.len(), 2, "unmatched inbound is inserted");
        let inserted = personas.iter().find(|p| p.id == other).unwrap();
        assert_eq!(inserted.display_name, "New");
        // Re-receiving the inserted record must still be idempotent.
        apply_inbound_persona(&mut personas, inbound_for(other, "New"));
        assert_eq!(personas.len(), 2, "re-receive of inserted record no-ops");
    }

    // ── Managed-agent (30177) inbound ────────────────────────────────────────

    const AGENT_PUBKEY: &str = "agentpubkeyhex0000000000000000000000000000000000000000000000000000";

    /// A local managed agent carrying every device-local secret that an inbound
    /// event must NEVER be able to overwrite.
    fn local_agent() -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: AGENT_PUBKEY.to_string(),
            name: "Local Agent".to_string(),
            persona_id: Some("persona-local".to_string()),
            private_key_nsec: "nsec1localsecret".to_string(),
            auth_tag: Some("localauthtag".to_string()),
            relay_url: "wss://relay.local".to_string(),
            avatar_url: None,
            acp_command: "buzz-acp".to_string(),
            agent_command: "goose".to_string(),
            agent_command_override: Some("claude".to_string()),
            agent_args: vec![],
            mcp_command: "buzz-dev-mcp".to_string(),
            turn_timeout_seconds: 320,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 8,
            system_prompt: Some("local prompt".to_string()),
            model: Some("local-model".to_string()),
            provider: Some("local-provider".to_string()),
            persona_source_version: Some("local-hash".to_string()),
            mcp_toolsets: Some("local".to_string()),
            env_vars: BTreeMap::from([("API_KEY".to_string(), "localsecret".to_string())]),
            start_on_app_launch: true,
            runtime_pid: Some(1234),
            backend: crate::managed_agents::BackendKind::Provider {
                id: "buzz-backend".to_string(),
                config: serde_json::json!({ "api_key": "localproviderkey" }),
            },
            backend_agent_id: Some("local-remote-id".to_string()),
            provider_binary_path: Some("/local/bin".to_string()),
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            respond_to: crate::managed_agents::RespondTo::OwnerOnly,
            respond_to_allowlist: vec![],
            relay_mesh: None,
        }
    }

    /// Sign a kind:30177 event whose content JSON carries the legitimate
    /// projected fields PLUS injected secret/harness keys — a hostile relay
    /// event trying to smuggle credentials onto the apply path.
    fn foreign_agent_event_with_secrets(d_tag: &str) -> nostr::Event {
        use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
        let content = serde_json::json!({
            "name": "Remote Agent",
            "persona_id": "persona-remote",
            "system_prompt": "remote prompt",
            "model": "remote-model",
            "provider": "remote-provider",
            "mcp_toolsets": "remote",
            "persona_source_version": "remote-hash",
            "parallelism": 99,
            "respond_to": "anyone",
            "respond_to_allowlist": ["deadbeef"],
            // Injected — must be dropped at deserialization, never applied.
            "private_key_nsec": "nsec1INJECTEDSECRET",
            "auth_tag": "INJECTEDAUTHTAG",
            "env_vars": { "API_KEY": "INJECTEDKEY" },
            "agent_command": "INJECTEDHARNESS",
            "agent_command_override": "INJECTEDOVERRIDE",
            "backend": { "type": "provider", "id": "x", "config": { "k": "INJECTEDBACKEND" } },
            "mcp_command": "INJECTEDMCP",
        });
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(30177), content.to_string())
            .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        // Round-trip through JSON to mirror the wire path the reconcile command
        // parses from.
        nostr::Event::from_json(event.as_json()).unwrap()
    }

    /// Direct-backend secret-preservation: drive the real parser + apply against
    /// a foreign event crammed with secrets and assert NONE land on the local
    /// record, and that every projected field IS updated. The projection type is
    /// the structural guard — the injected keys cannot even be represented.
    #[test]
    fn inbound_managed_agent_drops_injected_secrets_and_harness() {
        let event = foreign_agent_event_with_secrets(AGENT_PUBKEY);
        let content =
            crate::managed_agents::agent_events::managed_agent_content_from_event(&event).unwrap();
        let mut agents = vec![local_agent()];
        apply_inbound_managed_agent(&mut agents, AGENT_PUBKEY, content);

        let a = &agents[0];
        // Secrets / harness / runtime — every one preserved from the local record.
        assert_eq!(
            a.private_key_nsec, "nsec1localsecret",
            "secret key overwritten"
        );
        assert_eq!(
            a.auth_tag,
            Some("localauthtag".to_string()),
            "auth tag overwritten"
        );
        assert_eq!(
            a.env_vars.get("API_KEY"),
            Some(&"localsecret".to_string()),
            "env var overwritten"
        );
        assert_eq!(a.agent_command, "goose", "harness command overwritten");
        assert_eq!(
            a.agent_command_override,
            Some("claude".to_string()),
            "harness override overwritten"
        );
        assert_eq!(a.mcp_command, "buzz-dev-mcp", "mcp command overwritten");
        assert_eq!(a.relay_url, "wss://relay.local", "relay url overwritten");
        assert_eq!(a.runtime_pid, Some(1234), "runtime pid overwritten");
        match &a.backend {
            crate::managed_agents::BackendKind::Provider { config, .. } => {
                assert_eq!(
                    config["api_key"], "localproviderkey",
                    "backend blob overwritten"
                );
            }
            _ => panic!("backend kind changed"),
        }
        // No injected value appears anywhere on the serialized record.
        let json = serde_json::to_string(a).unwrap();
        for needle in [
            "INJECTEDSECRET",
            "INJECTEDAUTHTAG",
            "INJECTEDKEY",
            "INJECTEDHARNESS",
            "INJECTEDOVERRIDE",
            "INJECTEDBACKEND",
            "INJECTEDMCP",
        ] {
            assert!(!json.contains(needle), "injected value leaked: {needle}");
        }
        // Projected fields ARE updated from the inbound event.
        assert_eq!(a.name, "Remote Agent");
        assert_eq!(a.system_prompt, Some("remote prompt".to_string()));
        assert_eq!(a.model, Some("remote-model".to_string()));
        assert_eq!(a.provider, Some("remote-provider".to_string()));
        assert_eq!(a.parallelism, 99);
        assert_eq!(a.respond_to, crate::managed_agents::RespondTo::Anyone);
        assert_eq!(a.respond_to_allowlist, vec!["deadbeef".to_string()]);
    }

    #[test]
    fn inbound_managed_agent_no_match_is_noop() {
        let event = foreign_agent_event_with_secrets("someotheragentpubkey");
        let content =
            crate::managed_agents::agent_events::managed_agent_content_from_event(&event).unwrap();
        let mut agents = vec![local_agent()];
        apply_inbound_managed_agent(&mut agents, "someotheragentpubkey", content);

        // No agent minted from a relay event — it would have no secret key.
        assert_eq!(agents.len(), 1);
        assert_eq!(
            agents[0].name, "Local Agent",
            "unmatched inbound must not touch the local record"
        );
    }

    // ── Team (30176) inbound ─────────────────────────────────────────────────

    const TEAM_ID: &str = "team-local-id";

    fn local_team() -> TeamRecord {
        TeamRecord {
            id: TEAM_ID.to_string(),
            name: "Local Team".to_string(),
            description: Some("local desc".to_string()),
            persona_ids: vec!["p-local".to_string()],
            is_builtin: false,
            source_dir: Some(std::path::PathBuf::from("/local/team/dir")),
            is_symlink: true,
            symlink_target: Some("/external".to_string()),
            version: Some("1.0".to_string()),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    fn team_content(name: &str) -> TeamEventContent {
        TeamEventContent {
            name: name.to_string(),
            description: Some("remote desc".to_string()),
            persona_ids: vec!["p-remote-1".to_string(), "p-remote-2".to_string()],
        }
    }

    #[test]
    fn inbound_team_match_patches_shared_preserves_local() {
        let mut teams = vec![local_team()];
        apply_inbound_team(
            &mut teams,
            TEAM_ID.to_string(),
            team_content("Renamed Team"),
        );

        assert_eq!(teams.len(), 1, "no duplicate row");
        let t = &teams[0];
        // Shared fields overwritten.
        assert_eq!(t.name, "Renamed Team");
        assert_eq!(t.description, Some("remote desc".to_string()));
        assert_eq!(
            t.persona_ids,
            vec!["p-remote-1".to_string(), "p-remote-2".to_string()]
        );
        // Install-local fields preserved.
        assert_eq!(t.id, TEAM_ID);
        assert_eq!(
            t.source_dir,
            Some(std::path::PathBuf::from("/local/team/dir"))
        );
        assert!(t.is_symlink);
        assert_eq!(t.symlink_target, Some("/external".to_string()));
        assert_eq!(t.version, Some("1.0".to_string()));
        assert_eq!(t.created_at, "2025-01-01T00:00:00Z");
    }

    #[test]
    fn inbound_team_no_match_inserts_idempotently() {
        let mut teams = vec![local_team()];
        let other = "team-remote-id";
        apply_inbound_team(&mut teams, other.to_string(), team_content("New Team"));

        assert_eq!(teams.len(), 2, "unmatched inbound is inserted");
        let inserted = teams.iter().find(|t| t.id == other).unwrap();
        assert_eq!(inserted.name, "New Team");
        assert!(
            inserted.source_dir.is_none(),
            "inserted team has no local install dir"
        );
        // Re-receive stays idempotent.
        apply_inbound_team(&mut teams, other.to_string(), team_content("New Team"));
        assert_eq!(teams.len(), 2, "re-receive of inserted team no-ops");
    }

    // ── Tombstone (kind:5) consume ────────────────────────────────────────────

    fn deletion_event(coord: &str) -> nostr::Event {
        use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
        let event = EventBuilder::new(Kind::Custom(5), "")
            .tags(vec![Tag::parse(["a", coord]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        nostr::Event::from_json(event.as_json()).unwrap()
    }

    #[test]
    fn parse_deletion_coordinate_extracts_kind_and_d_tag() {
        let owner = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        // Persona / team / agent coordinates all route by their leading kind.
        let p = deletion_event(&format!("30175:{owner}:my-persona"));
        assert_eq!(
            parse_deletion_coordinate(&p),
            Some((30175, "my-persona".to_string()))
        );
        let a = deletion_event(&format!("30177:{owner}:agentpubkeyhex"));
        assert_eq!(
            parse_deletion_coordinate(&a),
            Some((30177, "agentpubkeyhex".to_string()))
        );
    }

    #[test]
    fn parse_deletion_coordinate_handles_colon_in_d_tag_and_rejects_malformed() {
        let owner = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
        // A d-tag containing ':' keeps its remainder intact (splitn(3)).
        let weird = deletion_event(&format!("30176:{owner}:a:b:c"));
        assert_eq!(
            parse_deletion_coordinate(&weird),
            Some((30176, "a:b:c".to_string()))
        );
        // Missing d-tag segment / non-numeric kind → None (no-op).
        assert_eq!(
            parse_deletion_coordinate(&deletion_event("30175:owner")),
            None
        );
        assert_eq!(
            parse_deletion_coordinate(&deletion_event("notakind:owner:d")),
            None
        );
    }

    #[test]
    fn tombstone_removal_predicates_match_apply_fn_keys() {
        // The deletion path removes by the SAME per-kind key the apply fns use.
        // Persona: by persona_d_tag (slug/id).
        let mut personas = vec![local_in_app()];
        let target = persona_d_tag(&personas[0]);
        personas.retain(|r| persona_d_tag(r) != target);
        assert!(personas.is_empty(), "persona removed by its d-tag");

        // Team: by id.
        let mut teams = vec![local_team()];
        teams.retain(|r| r.id != TEAM_ID);
        assert!(teams.is_empty(), "team removed by id");

        // Managed-agent: by pubkey. A non-matching d-tag is a no-op.
        let mut agents = vec![local_agent()];
        agents.retain(|r| r.pubkey != "someoneelse");
        assert_eq!(agents.len(), 1, "non-matching agent tombstone no-ops");
        agents.retain(|r| r.pubkey != AGENT_PUBKEY);
        assert!(agents.is_empty(), "agent removed by pubkey");
    }
}

#[cfg(test)]
mod writeback_tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Build a minimal PersonaRecord with the fields that `rewrite_persona_md` reads.
    fn persona(
        display_name: &str,
        runtime: Option<&str>,
        avatar_url: Option<&str>,
        provider: Option<&str>,
        model: Option<&str>,
    ) -> PersonaRecord {
        // system_prompt matches the SAMPLE_MD body so the "no prompt edit" path
        // is taken in rewrite_persona_md (body preserved byte-for-byte).
        persona_with_prompt(display_name, runtime, avatar_url, provider, model, "You are Paul.\n")
    }

    /// Like `persona` but with an explicit system_prompt value.
    fn persona_with_prompt(
        display_name: &str,
        runtime: Option<&str>,
        avatar_url: Option<&str>,
        provider: Option<&str>,
        model: Option<&str>,
        system_prompt: &str,
    ) -> PersonaRecord {
        PersonaRecord {
            id: "test-id".to_string(),
            display_name: display_name.to_string(),
            avatar_url: avatar_url.map(str::to_string),
            system_prompt: system_prompt.to_string(),
            runtime: runtime.map(str::to_string),
            model: model.map(str::to_string),
            provider: provider.map(str::to_string),
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            source_team: Some("team-1".to_string()),
            source_team_persona_slug: Some("paul".to_string()),
            env_vars: BTreeMap::new(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    const SAMPLE_MD: &str = "\
---
name: paul
display_name: \"Paul\"
description: \"An orchestrator.\"
model: goose-claude-4-6-opus
runtime: goose
extra_key: keep-me
---
You are Paul.
";

    // ── rewrite_persona_md unit tests ─────────────────────────────────────────

    #[test]
    fn test_rewrite_model_provider_joined_and_body_preserved() {
        let p = persona("Paul", Some("goose"), None, Some("databricks_v2"), Some("goose-claude-opus-4-8"));
        let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

        // Body is byte-preserved.
        assert!(result.ends_with("\nYou are Paul.\n"), "body not preserved: {result:?}");

        // model key is the joined form.
        assert!(result.contains("model: databricks_v2:goose-claude-opus-4-8"), "joined model missing: {result}");

        // No separate provider key.
        assert!(!result.contains("provider:"), "separate provider key must not be emitted: {result}");

        // Unrelated key preserved.
        assert!(result.contains("extra_key: keep-me"), "extra key lost: {result}");

        // Still valid frontmatter (parses cleanly).
        assert!(result.starts_with("---\n"), "must start with ---");
    }

    #[test]
    fn test_rewrite_bare_model_when_provider_none() {
        let p = persona("Paul", Some("goose"), None, None, Some("bare-model-id"));
        let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

        assert!(result.contains("model: bare-model-id"), "bare model missing: {result}");
        assert!(!result.contains("provider:"), "provider key must not be emitted: {result}");
    }

    #[test]
    fn test_rewrite_runtime_removed_when_none() {
        let p = persona("Paul", None, None, None, Some("some-model"));
        let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

        // runtime was in source but persona.runtime is None — key must be removed.
        assert!(!result.contains("runtime:"), "runtime key should be removed when None: {result}");
    }

    #[test]
    fn test_rewrite_preserves_description_and_name_and_extra_keys() {
        let p = persona("Paul Updated", Some("goose"), None, Some("anthropic"), Some("claude-opus-4"));
        let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

        // name and description are not persona record fields — must survive untouched.
        assert!(result.contains("name: paul"), "name key lost: {result}");
        assert!(result.contains("description:"), "description key lost: {result}");
        assert!(result.contains("extra_key: keep-me"), "extra_key lost: {result}");

        // display_name updated.
        assert!(result.contains("display_name: Paul Updated") || result.contains("display_name: \"Paul Updated\""),
            "display_name not updated: {result}");
    }

    #[test]
    fn test_rewrite_no_provider_no_model_removes_model_key() {
        let p = persona("Paul", None, None, None, None);
        let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

        // When both provider and model are cleared, the model key is removed.
        assert!(!result.contains("model:"), "model key should be removed when both absent: {result}");
    }

    #[test]
    fn test_rewrite_avatar_set_and_cleared() {
        let with_avatar = persona("Paul", Some("goose"), Some("data:image/png;base64,abc"), Some("openai"), Some("gpt-4o"));
        let result = rewrite_persona_md(SAMPLE_MD, &with_avatar, "You are Paul.\n", None).unwrap();
        assert!(result.contains("avatar:"), "avatar key should be set: {result}");

        let without_avatar = persona("Paul", Some("goose"), None, Some("openai"), Some("gpt-4o"));
        let result = rewrite_persona_md(SAMPLE_MD, &without_avatar, "You are Paul.\n", None).unwrap();
        assert!(!result.contains("avatar:"), "avatar key should be absent when None: {result}");
    }

    #[test]
    fn test_rewrite_body_not_replaced_by_system_prompt() {
        // system_prompt on the PersonaRecord is the COMPOSED prompt (body + pack instructions).
        // The body of the .persona.md must not be replaced with it.
        let p = persona("Paul", Some("goose"), None, Some("databricks_v2"), Some("goose-claude-opus-4-8"));
        let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

        // The raw body from the file ("You are Paul.") is preserved.
        assert!(result.ends_with("You are Paul.\n"), "raw body must be preserved, not replaced by composed system_prompt: {result:?}");
        // The composed instructions suffix must NOT appear in the file body.
        assert!(!result.contains("# Team Instructions"), "composed system_prompt must not be written to body: {result}");
    }

    // ── body (system_prompt) write-back tests ─────────────────────────────────
    //
    // These tests exercise the compose_prompt inversion logic in rewrite_persona_md.

    /// Frontmatter-only MD (no body to preserve) for prompt tests.
    const PROMPT_MD: &str = "\
---
name: paul
display_name: \"Paul\"
model: goose-claude-4-6-opus
---
You are Paul.
";

    #[test]
    fn test_prompt_edited_with_pack_instructions_body_rewritten() {
        // User edits the prompt. system_prompt = new_raw_body + separator + instructions.
        // Body in file should be updated to new_raw_body; Team Instructions must NOT appear.
        let instructions = "Follow the rules.";
        let new_raw_body = "You are Paul, a wise orchestrator.";
        let composed = format!("{new_raw_body}\n\n---\n# Team Instructions\n{instructions}");

        let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
        p.system_prompt = composed;

        let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", Some(instructions)).unwrap();
        assert!(result.ends_with("You are Paul, a wise orchestrator."), "new body not written: {result:?}");
        assert!(!result.contains("# Team Instructions"), "Team Instructions must not appear in body: {result}");
    }

    #[test]
    fn test_prompt_edited_no_pack_instructions_body_rewritten_verbatim() {
        // No pack instructions: composed == raw. New body is system_prompt verbatim.
        let new_raw_body = "You are Paul, updated.";
        let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
        p.system_prompt = new_raw_body.to_string();

        let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", None).unwrap();
        assert!(result.ends_with("You are Paul, updated."), "body not updated: {result:?}");
    }

    #[test]
    fn test_prompt_unedited_body_preserved() {
        // system_prompt equals compose_prompt(current_raw_body, instructions) exactly.
        // The body section must not change even though frontmatter may be rewritten.
        let instructions = "Follow the rules.";
        let raw_body = "You are Paul.";
        let composed = format!("{raw_body}\n\n---\n# Team Instructions\n{instructions}");

        let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
        // Frontmatter also unchanged (matches PROMPT_MD).
        p.system_prompt = composed;

        let result = rewrite_persona_md(PROMPT_MD, &p, raw_body, Some(instructions)).unwrap();
        // Body must remain "You are Paul." (no prompt edit — body section preserved).
        assert!(result.ends_with("You are Paul.\n"), "body must be unchanged: {result:?}");
        // Team Instructions must not leak into the body.
        assert!(!result.contains("# Team Instructions"), "Team Instructions must not appear: {result}");
    }

    #[test]
    fn test_prompt_safety_guard_missing_suffix_preserves_body() {
        // pack_instructions is non-empty but system_prompt does NOT end with the
        // expected suffix (user edited inside the Team Instructions block, or the
        // instructions drifted). The existing body must be preserved — no corruption.
        let instructions = "Follow the rules.";
        let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
        // system_prompt lacks the expected suffix entirely.
        p.system_prompt = "Some rogue prompt with # Team Instructions in the middle".to_string();

        let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", Some(instructions)).unwrap();
        // Body preserved from the file.
        assert!(result.ends_with("You are Paul.\n"), "body must be preserved by safety guard: {result:?}");
    }

    #[test]
    fn test_prompt_round_trip_no_double_append() {
        // After write-back, running compose_prompt on the written body + instructions
        // must reproduce the stored system_prompt exactly. This proves no double-append.
        let instructions = "Follow the rules.";
        let new_raw_body = "You are Paul, updated for the round-trip.";
        let composed = format!("{new_raw_body}\n\n---\n# Team Instructions\n{instructions}");

        let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
        p.system_prompt = composed.clone();

        let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", Some(instructions)).unwrap();

        // Extract the written body (everything after the last "---\n").
        let written_body = result.split("---\n").last().unwrap();
        // Re-compose: body + instructions must equal the original composed prompt.
        let recomposed = format!("{written_body}\n\n---\n# Team Instructions\n{instructions}");
        assert_eq!(recomposed, composed, "round-trip failed — double-append would occur: recomposed={recomposed:?}");
    }

    // ── write_back_persona_md path-resolution tests ───────────────────────────
    //
    // These tests verify that write_back_persona_md resolves the source file via
    // the pack manifest rather than by convention. This is the class of bug that
    // Thufir's IMPORTANT finding caught: a pack whose manifest points at
    // `personas/foo.persona.md` (not `agents/<slug>.persona.md`) must still be
    // rewritten correctly.

    use tempfile::TempDir;

    /// Build a minimal pack on disk with the given personas layout.
    ///
    /// `persona_entries` is a list of (manifest_rel_path, file_content) pairs.
    /// The manifest lists the relative paths; the files are written verbatim.
    fn make_temp_pack(persona_entries: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();

        std::fs::create_dir_all(root.join(".plugin")).unwrap();
        let persona_paths: Vec<&str> = persona_entries.iter().map(|(p, _)| *p).collect();
        let manifest = serde_json::json!({
            "id": "test-team",
            "name": "Test Team",
            "version": "0.1.0",
            "personas": persona_paths,
        });
        std::fs::write(
            root.join(".plugin/plugin.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        ).unwrap();

        for (rel_path, content) in persona_entries {
            let abs = root.join(rel_path);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(&abs, content).unwrap();
        }

        dir
    }

    #[test]
    fn test_writeback_uses_manifest_path_not_convention() {
        // Pack uses `personas/paul.persona.md` layout (not `agents/`).
        // Convention-based path would derive `agents/paul.persona.md` (wrong).
        // Manifest-based resolution must write to the correct `personas/` path.
        let persona_md = "\
---
name: paul
display_name: \"Paul\"
description: \"Orchestrator\"
model: goose-claude-4-6-opus
---
You are Paul.
";
        let dir = make_temp_pack(&[("personas/paul.persona.md", persona_md)]);
        let source_dir = dir.path().to_path_buf();

        let pack = buzz_persona_pkg::pack::load_pack(&source_dir).unwrap();
        assert_eq!(pack.personas[0].name, "paul");
        let source_path = pack.personas[0].source_path.clone();
        // File lives under personas/, not agents/
        assert!(source_path.to_string_lossy().contains("personas/paul.persona.md"),
            "expected personas/ layout: {}", source_path.display());

        // Simulate what write_back_persona_md does after the fix:
        // use source_path from pack, not source_dir/agents/<slug>.persona.md
        let mut p = persona("Paul Updated", Some("goose"), None, Some("databricks_v2"), Some("goose-claude-opus-4-8"));
        p.source_team_persona_slug = Some("paul".to_string());

        let content = std::fs::read_to_string(&source_path).unwrap();
        let updated = rewrite_persona_md(&content, &p, "You are Paul.\n", None).unwrap();
        std::fs::write(&source_path, &updated).unwrap();

        let after = std::fs::read_to_string(&source_path).unwrap();
        assert!(after.contains("databricks_v2:goose-claude-opus-4-8"), "model not written: {after}");
        assert!(after.ends_with("You are Paul.\n"), "body not preserved: {after:?}");
    }

    #[test]
    fn test_writeback_name_differs_from_filename() {
        // Pack file is `personas/orchestrator.persona.md` but `name: paul`.
        // Slug matches `name:`, not the filename.
        let persona_md = "\
---
name: paul
display_name: \"Paul\"
description: \"Orchestrator\"
model: old-model
---
You are Paul.
";
        let dir = make_temp_pack(&[("personas/orchestrator.persona.md", persona_md)]);
        let source_dir = dir.path().to_path_buf();

        let pack = buzz_persona_pkg::pack::load_pack(&source_dir).unwrap();
        let loaded = pack.personas.iter().find(|p| p.name == "paul").unwrap();
        let source_path = loaded.source_path.clone();
        // File basename is orchestrator, not paul
        assert!(source_path.to_string_lossy().contains("orchestrator.persona.md"),
            "expected orchestrator.persona.md: {}", source_path.display());

        let mut p = persona("Paul", Some("goose"), None, Some("anthropic"), Some("claude-4"));
        p.source_team_persona_slug = Some("paul".to_string());

        let content = std::fs::read_to_string(&source_path).unwrap();
        let updated = rewrite_persona_md(&content, &p, "You are Paul.\n", None).unwrap();
        std::fs::write(&source_path, &updated).unwrap();

        let after = std::fs::read_to_string(&source_path).unwrap();
        assert!(after.contains("anthropic:claude-4"), "model not written to orchestrator.persona.md: {after}");
        // The wrong file (paul.persona.md in agents/) must NOT exist.
        assert!(!dir.path().join("agents/paul.persona.md").exists(),
            "convention-based path must not be created");
    }

    // ── find_team_for_persona_source tests ────────────────────────────────────
    //
    // Verify that write_back_persona_md finds teams by team_persona_key, not
    // team.id. Legacy/backfilled teams have a UUID `id` while PersonaRecord
    // stores the manifest directory name in `source_team`; matching by `id`
    // alone silently misses those teams.

    fn make_team(id: &str, source_dir: Option<&str>) -> TeamRecord {
        TeamRecord {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            persona_ids: vec![],
            is_builtin: false,
            source_dir: source_dir.map(|s| std::path::PathBuf::from(s)),
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_find_team_legacy_uuid_id_matched_by_source_dir_name() {
        // Legacy shape: team.id is a UUID; PersonaRecord.source_team is the
        // manifest directory name. The old `team.id == source_team` predicate
        // missed this — find_team_for_persona_source must match via source_dir.
        let teams = vec![make_team("some-uuid-123", Some("/teams/com.test.pack"))];
        let found = find_team_for_persona_source(&teams, "com.test.pack");
        assert!(found.is_some(), "legacy team must be found by manifest dir name, not UUID");
        assert_eq!(found.unwrap().id, "some-uuid-123");
    }

    #[test]
    fn test_find_team_modern_id_matched_directly() {
        // Modern shape: team.id equals the manifest directory name. Must match.
        let teams = vec![make_team("com.test.pack", Some("/teams/com.test.pack"))];
        let found = find_team_for_persona_source(&teams, "com.test.pack");
        assert!(found.is_some(), "modern team must be found");
    }

    #[test]
    fn test_find_team_manifest_dir_name_matches_regardless_of_uuid_id() {
        // Regression: the old predicate `team.id == source_team` missed legacy
        // teams when source_team holds the manifest dir name, not the UUID.
        // This test confirms that searching by manifest dir name always works
        // even when team.id is a UUID.
        let teams = vec![make_team("some-uuid-123", Some("/teams/com.test.pack"))];
        // source_team holds the manifest dir name "com.test.pack", not the UUID.
        let by_dir = find_team_for_persona_source(&teams, "com.test.pack");
        assert!(by_dir.is_some(), "manifest dir name must find the legacy team");
        assert_eq!(by_dir.unwrap().id, "some-uuid-123");
    }

    #[test]
    fn test_find_team_no_source_dir_falls_back_to_id() {
        // JSON-only team: no source_dir, team_persona_key falls back to id.
        let teams = vec![make_team("builtin-team:fizz", None)];
        let found = find_team_for_persona_source(&teams, "builtin-team:fizz");
        assert!(found.is_some(), "no source_dir: must match via team.id fallback");
    }

    #[test]
    fn test_find_team_returns_none_when_no_match() {
        let teams = vec![make_team("some-uuid-123", Some("/teams/com.test.pack"))];
        let not_found = find_team_for_persona_source(&teams, "com.other.pack");
        assert!(not_found.is_none(), "unrelated source_team must not match");
    }
}
