use tauri::{AppHandle, State};
use uuid::Uuid;

use super::export_util::save_json_with_dialog;
use crate::{
    app_state::AppState,
    managed_agents::{
        encode_persona_json, load_managed_agents, load_personas, load_teams, parse_json_persona,
        parse_md_persona, parse_png_persona, parse_zip_personas, persona_events::persona_d_tag,
        save_managed_agents, save_personas, try_regenerate_nest,
        validate_persona_activation_change, validate_persona_deletion, CreatePersonaRequest,
        ParsePersonaFilesResult, PersonaRecord, UpdatePersonaRequest,
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
        persona_events::{build_persona_event, persona_d_tag},
        retention::{open_retention_db, retain_event, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    let result = (|| -> Result<(), String> {
        let (pubkey, event) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            let event = build_persona_event(persona)?
                .sign_with_keys(&keys)
                .map_err(|e| format!("failed to sign persona event: {e}"))?;
            (keys.public_key().to_hex(), event)
        };
        let conn = open_retention_db(&managed_agents_base_dir(app)?.join("retention.db"))?;
        retain_event(
            &conn,
            &RetainedEvent {
                kind: KIND_PERSONA,
                pubkey,
                d_tag: persona_d_tag(persona),
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
fn tombstone_persona_pending(app: &AppHandle, state: &AppState, d_tag: &str) {
    use crate::managed_agents::{
        managed_agents_base_dir,
        persona_events::build_persona_delete,
        retention::{delete_retained_event, open_retention_db, retain_event, RetainedEvent},
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
                d_tag: d_tag.to_string(),
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
    let system_prompt = trim_required(&input.system_prompt, "System prompt")?;
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
    let system_prompt = trim_required(&input.system_prompt, "System prompt")?;
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
    retain_persona_pending(&app, &state, &result);
    try_regenerate_nest(&app);
    Ok(result)
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
        managed_agents_base_dir,
        persona_events::persona_from_event,
        retention::{open_retention_db, retain_inbound_event, InboundOutcome, RetainedEvent},
    };
    use buzz_core_pkg::kind::KIND_PERSONA;
    use nostr::JsonUtil;

    let event = nostr::Event::from_json(&event_json)
        .map_err(|e| format!("failed to parse inbound event: {e}"))?;

    // Per-kind scoping: the live filter subscribes to 30175/30176/30177, but
    // only persona (30175) inbound is implemented here. Team (30176) and
    // managed-agent (30177) reconcile is a tracked follow-up; no-op for now so
    // a d-tag shared across kinds can never cross-link a team and a persona.
    // follow-up: team/agent inbound parse + patch (30176/30177).
    if event.kind.as_u16() as u32 != KIND_PERSONA {
        return Ok(());
    }

    let inbound = persona_from_event(&event)?;
    let d_tag = persona_d_tag(&inbound);

    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    // Resolve inbound vs. any pending local edit before touching personas.json.
    let conn = open_retention_db(&managed_agents_base_dir(&app)?.join("retention.db"))?;
    let outcome = retain_inbound_event(
        &conn,
        &RetainedEvent {
            kind: KIND_PERSONA,
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

    let mut personas = load_personas(&app)?;
    apply_inbound_persona(&mut personas, inbound);
    save_personas(&app, &personas)?;
    try_regenerate_nest(&app);

    Ok(())
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

// ---------------------------------------------------------------------------
// Import / Export
// ---------------------------------------------------------------------------

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
}
