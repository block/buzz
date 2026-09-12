//! The persona edit command surface: `update_persona` (best-effort enqueue)
//! and the `update_persona_with` seam that `update_persona_and_publish` reuses
//! to await relay acceptance for the same save.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        apply_persona_behavior, effective_agent_command, load_managed_agents, load_personas,
        managed_agent_avatar_url, save_managed_agents, save_personas, try_regenerate_nest,
        validate_agent_definition_text, AgentDefinition, ManagedAgentRecord, UpdatePersonaRequest,
    },
    util::now_iso,
};

use super::{normalize_description, pending, propagate, retain_persona_pending, trim_optional, trim_required};

use propagate::{propagate_persona_name_rename, propagate_persona_respond_to};

#[cfg(test)]
mod name_propagation_tests;

/// Return value of the `update_persona` command. Uses flatten so all
/// `AgentDefinition` fields appear at the top level of the JSON response —
/// backward-compatible with callers that already destructure a raw persona object.
#[derive(Debug, serde::Serialize)]
pub struct UpdatePersonaResult {
    #[serde(flatten)]
    persona: AgentDefinition,
}


/// Profile sync params collected under the store lock for async relay publish:
/// (agent keys, relay url, display name, avatar url, kind:0 about, auth tag).
type ProfileSyncParams = Vec<(
    nostr::Keys,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
)>;


#[tauri::command]
pub async fn update_persona(
    input: UpdatePersonaRequest,
    app: AppHandle,
) -> Result<UpdatePersonaResult, String> {
    let (persona, ()) = update_persona_with(input, app, |app, state, persona| {
        retain_persona_pending(app, state, persona);
        // F2: immediately refresh any shared 30178 heads that include this
        // persona as a member. Best-effort inside retain so a hiccup cannot
        // fail the persona edit itself.
        crate::commands::refresh_team_catalog_heads_for_persona(app, state, &persona.id);
        Ok(())
    })
    .await?;
    Ok(UpdatePersonaResult { persona })
}

/// Save an edited persona, hand the saved record to `retain` while the store
/// lock is still held, then sync the relay profiles of linked agent instances.
///
/// `retain` is the only difference between the two update commands:
/// [`update_persona`] enqueues best-effort, while
/// [`sharing::update_persona_and_publish`] prepares a strict publication and
/// returns the event so the caller can await relay acceptance.
pub(super) async fn update_persona_with<R: Send + 'static>(
    input: UpdatePersonaRequest,
    app: AppHandle,
    retain: impl FnOnce(&AppHandle, &AppState, &AgentDefinition) -> Result<R, String> + Send + 'static,
) -> Result<(AgentDefinition, R), String> {
    use tauri::Manager;

    // Phase 1: synchronous save (persona record + linked agent avatar updates)
    let (result, retained, profile_sync_params) = tokio::task::spawn_blocking({
        let app = app.clone();
        move || -> Result<(AgentDefinition, R, ProfileSyncParams), String> {
            let state = app.state::<AppState>();
            let display_name = trim_required(&input.display_name, "Display name")?;
            let system_prompt = input.system_prompt.clone();
            validate_agent_definition_text(&display_name, &system_prompt)?;
            let description = normalize_description(input.description)?;
            let avatar_url = trim_optional(input.avatar_url);
            let runtime = trim_optional(input.runtime);
            let model = trim_optional(input.model);
            let provider = trim_optional(input.provider);

            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let mut personas = load_personas(&app)?;
            pending::project_active_persona_sharing(&app, &state, &mut personas);
            let persona = personas
                .iter_mut()
                .find(|record| record.id == input.id)
                .ok_or_else(|| format!("agent {} not found", input.id))?;

            // Track what changed so we can propagate to linked agent records.
            let avatar_changed = persona.avatar_url != avatar_url;
            let name_changed = persona.display_name != display_name;
            let old_display_name = persona.display_name.clone();
            // The kind:0 `about` is the authored description, so a
            // description edit changes what should be published.
            let old_about =
                crate::managed_agents::effective_agent_description(persona.description.as_deref());
            let new_about =
                crate::managed_agents::effective_agent_description(description.as_deref());
            let about_changed = old_about != new_about;

            persona.display_name = display_name;
            persona.avatar_url = avatar_url;
            persona.description = description;
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
            let behavior_present = input.behavior.is_some();
            apply_persona_behavior(persona, input.behavior)?;
            persona.updated_at = now_iso();

            let result = persona.clone();
            save_personas(&app, &personas)?;

            let retained = retain(&app, &state, &result)?;
            try_regenerate_nest(&app);


            // Propagate definition edits that instances must mirror (avatar,
            // display name, respond-to gate) and collect relay profile sync
            // params for the async phase.
            //
            // Also enter when the definition already carries an explicit gate:
            // unrelated edits (prompt, model, …) must still reconcile that gate
            // onto instances so a prior broken owner-only instance can heal
            // without toggling the behavior control.
            let sync_params: ProfileSyncParams = if avatar_changed
                || name_changed
                || about_changed
                || behavior_present
                || result.respond_to.is_some()

            {
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

                // Explicit definition gate only — unset leaves instances alone.
                if result.respond_to.is_some()
                    && propagate_persona_respond_to(&mut records, &result.id, &result)? > 0
                {
                    agents_modified = true;
                }

                for record in records.iter_mut() {
                    if record.persona_id.as_deref() != Some(&result.id) {
                        continue;
                    }
                    let was_renamed = renamed.contains(&record.pubkey);
                    let update = prepare_linked_profile_update(
                        record,
                        &result,
                        was_renamed,
                        avatar_changed,
                        about_changed,
                    );

                    agents_modified = agents_modified || update.record_changed;
                    if update.profile_sync_required {
                        if let Ok(agent_keys) = nostr::Keys::parse(&record.private_key_nsec) {
                            let relay_url = crate::relay::effective_agent_relay_url(
                                &record.relay_url,
                                &workspace_relay,
                            );
                            params.push((
                                agent_keys,
                                relay_url,
                                record.name.clone(),
                                update.profile_avatar,
                                new_about.clone(),
                                record.auth_tag.clone(),
                            ));
                        }
                    }
                }

                if agents_modified {
                    save_managed_agents(&app, &records)?;
                    // Keep retained kind:30177 identity records in lockstep with
                    // the rename (#2423): `record.name` is part of the published
                    // identity projection, so skipping this strands the relay on
                    // the stale name→pubkey binding until the next boot reconcile.
                    // Avatar-only edits are excluded — the avatar is not in the
                    // projection, so retaining would be a guaranteed no-op.
                    for record in records.iter().filter(|r| renamed.contains(&r.pubkey)) {
                        crate::commands::agents::retain_managed_agent_pending(&app, &state, record);
                    }
                }

                params
            } else {
                Vec::new()
            };

            Ok((result, retained, sync_params))
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    // Phase 2: await relay profile sync for linked agents whose avatar,
    // display_name, or effective description (kind:0 about) was just
    // updated. We await (rather than fire-and-forget)
    // so the frontend cache invalidation that follows the mutation settlement
    // sees the fresh relay profile. Best-effort — failures are logged, not surfaced.
    if !profile_sync_params.is_empty() {
        let state = app.state::<AppState>();
        for (agent_keys, relay_url, display_name, avatar_url, about, auth_tag) in
            profile_sync_params
        {
            if let Err(e) = crate::relay::sync_managed_agent_profile(
                &state,
                &relay_url,
                &agent_keys,
                &display_name,
                avatar_url.as_deref(),
                about.as_deref(),
                auth_tag.as_deref(),
            )
            .await
            {
                eprintln!("buzz-desktop: relay profile sync failed after persona update: {e}");
            }
        }
    }

    Ok((result, retained))
}
