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

use super::{pending, retain_persona_pending, trim_optional, trim_required};

#[cfg(test)]
mod behavior_cascade_tests;
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

/// Profile sync params collected under the store lock for async relay publish.
type ProfileSyncParams = Vec<(nostr::Keys, String, String, Option<String>, Option<String>)>;

/// Propagate a persona definition's behavioral-group edit to linked agent
/// instances. Discrimination rule (mirrors the pool-name rule in
/// [`propagate_persona_name_rename`]): an instance whose `respond_to` AND
/// `respond_to_allowlist` still equal the PRE-edit definition values was
/// inheriting → it adopts the new definition value; an instance carrying a
/// different value holds an explicit instance-level override → preserved.
/// The allowlist is part of the discriminant because mode alone cannot
/// express a same-mode allowlist pin: an instance pinned to `allowlist` +
/// `[X]` under a definition also in `allowlist` would otherwise read as
/// "still inheriting" and lose its pin. Every linked instance's
/// definition-mirror fields (`definition_respond_to` &c.) refresh
/// regardless, so future mint/inspect paths see the current definition bytes.
///
/// A definition sitting in `allowlist` with an empty allowlist is skipped
/// (mirrors refresh, gates do not change): `resolve_mint_behavioral_defaults`
/// and `apply_persona_behavior` both reject that state at mint, so the
/// cascade must not manufacture a record neither of them would ever produce.
/// Skip, don't fail — the definition state is reachable through the
/// person-picker (mode persists, members don't; issue #2501 defect 1) and a
/// hard error here would wedge every other edit on the persona.
///
/// Returns `true` when at least one linked record was touched (caller must
/// persist the records store).
/// What a behavior cascade did, so the caller can republish only what it
/// must. `linked` means at least one record belongs to this persona (mirror
/// fields always refresh); `adopted` names the records whose effective gate
/// actually changed.
#[derive(Debug, Default)]
struct BehaviorCascade {
    linked: bool,
    adopted: Vec<String>,
}

fn propagate_persona_behavior(
    records: &mut [ManagedAgentRecord],
    persona_id: &str,
    old_mode: crate::managed_agents::RespondTo,
    old_allowlist: &[String],
    persona: &AgentDefinition,
) -> Result<BehaviorCascade, String> {
    use crate::managed_agents::RespondTo;

    // Parse before touching any record, so a bogus definition mode cannot
    // half-apply mirror refreshes either (fail-loudly contract shared with
    // `resolve_mint_behavioral_defaults`).
    let new_mode = match persona.respond_to.as_deref() {
        Some(wire) => RespondTo::parse_wire(wire)?,
        None => RespondTo::default(),
    };

    // Effective allowlists as `apply_persona_behavior` stores them on
    // records: non-allowlist modes store an empty list. Comparing against
    // the definition's raw list would break inheritance detection whenever a
    // non-allowlist definition carries residual allowlist entries, and
    // writing the raw list would break the NEXT edit's detection the same
    // way.
    let inherited_allowlist: &[String] = if old_mode == RespondTo::Allowlist {
        old_allowlist
    } else {
        &[]
    };
    let adopted_allowlist: &[String] = if new_mode == RespondTo::Allowlist {
        &persona.respond_to_allowlist
    } else {
        &[]
    };
    let definition_adoptable =
        new_mode != RespondTo::Allowlist || !persona.respond_to_allowlist.is_empty();

    let mut outcome = BehaviorCascade::default();
    for record in records.iter_mut() {
        if record.persona_id.as_deref() != Some(persona_id) {
            continue;
        }
        outcome.linked = true;

        // Read the pre-fix marker BEFORE the mirror refresh overwrites it.
        let mirrors_unset = record.definition_respond_to.is_none();

        record.definition_respond_to = persona.respond_to.clone();
        record.definition_respond_to_allowlist = persona.respond_to_allowlist.clone();
        record.definition_parallelism = persona.parallelism;

        // An instance minted before this cascade existed, against a
        // definition already edited during the buggy era, matches neither the
        // old mode nor the old allowlist -- so it reads as a deliberate pin
        // and stays desynced forever. The mirror fields are written ONLY by
        // this cascade, so an unset `definition_respond_to` is a reliable
        // "pre-fix record" marker; combined with a gate still at the mint
        // default it is inheritance, not a pin. A dialog pin is non-default
        // by construction, and before this fix the dialog could not write an
        // instance-level gate at all (that is defect 1 of #2501), so the
        // default gate on a pre-fix record cannot be a deliberate choice.
        // Reported with store bytes by @xtranger51 on #4115.
        let pre_fix_default = mirrors_unset
            && record.respond_to == RespondTo::default()
            && record.respond_to_allowlist.is_empty();

        let still_inheriting = pre_fix_default
            || (record.respond_to == old_mode
                && same_allowlist(&record.respond_to_allowlist, inherited_allowlist));

        if still_inheriting {
            // Parallelism is not part of the unsafe state. Adopting the
            // respond_to gate from a definition sitting in Allowlist with an
            // empty allowlist would cascade a state the mint path rejects, so
            // that is gated -- but freezing an inheriting instance's pool
            // width as a side effect of the same condition is a separate,
            // unasked-for behaviour change, and `behavior_changed` fires on a
            // parallelism-only edit (review on #4115).
            if let Some(parallelism) = persona.parallelism {
                record.parallelism = parallelism;
            }

            if definition_adoptable
                && (record.respond_to != new_mode
                    || record.respond_to_allowlist.as_slice() != adopted_allowlist)
            {
                // Adopt the new definition value as the instance's effective
                // gate. `None` on the definition means "no explicit mode":
                // the harness default (owner-only) applies.
                record.respond_to = new_mode;
                record.respond_to_allowlist = adopted_allowlist.to_vec();
                outcome.adopted.push(record.pubkey.clone());
            }
        }
    }
    Ok(outcome)
}

/// Whether two allowlists hold the same principals.
///
/// Order-insensitive on purpose: nothing guarantees a stable ordering out of
/// the person-picker, and a positional comparison reads an instance holding
/// the same pubkeys in a different order as pinned -- silently stranding it
/// on the old gate forever (review on #4115). Sorted rather than set-wise so
/// a genuine duplicate still counts as a difference.
fn same_allowlist(left: &[String], right: &[String]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left: Vec<&str> = left.iter().map(String::as_str).collect();
    let mut right: Vec<&str> = right.iter().map(String::as_str).collect();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

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
            // Pre-edit behavioral signature — the definition value linked
            // instances were minted against. Used post-save to cascade
            // behavior edits ONLY to instances that were still inheriting
            // (record.respond_to == pre-edit definition value), never to
            // instances carrying an explicit instance-level override.
            let old_respond_to = persona.respond_to.clone();
            let old_respond_to_allowlist = persona.respond_to_allowlist.clone();
            let old_parallelism = persona.parallelism;

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

            // Cascade behavior edits to linked instance records that were
            // still inheriting the definition value. Discrimination rule
            // (mirrors the pool-name rule for display_name): an instance
            // whose record.respond_to already equals the OLD definition value
            // was inheriting → adopt the new value + mirrors. An instance
            // whose record.respond_to differs is an explicit override →
            // preserve it; only the definition mirror fields refresh. The
            // empty-allowlist-ignores-mode asymmetry in apply_persona_behavior
            // (non-allowlist modes store an empty list) is preserved verbatim.
            let behavior_changed = old_respond_to != result.respond_to
                || old_respond_to_allowlist != result.respond_to_allowlist
                || old_parallelism != result.parallelism;
            if behavior_changed {
                let mut records = load_managed_agents(&app)?;
                let old_mode = old_respond_to
                    .as_deref()
                    .and_then(|wire| crate::managed_agents::RespondTo::parse_wire(wire).ok())
                    .unwrap_or_default();
                let cascade = propagate_persona_behavior(
                    &mut records,
                    &result.id,
                    old_mode,
                    &old_respond_to_allowlist,
                    &result,
                )?;
                if cascade.linked {
                    save_managed_agents(&app, &records)?;
                    // The behavioral triple is part of the published kind:30177
                    // projection (`agent_event_content`), so without this the
                    // relay's retained record stays stale until the next boot
                    // reconcile -- the same reason the rename cascade retains
                    // (#2423). Mirror-only refreshes are excluded: they do not
                    // change the projection, so retaining would be a no-op.
                    // Reported by @xtranger51 on #4115.
                    for record in records
                        .iter()
                        .filter(|r| cascade.adopted.contains(&r.pubkey))
                    {
                        crate::commands::agents::retain_managed_agent_pending(&app, &state, record);
                    }
                }
            }

            let retained = retain(&app, &state, &result)?;
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

    Ok((result, retained))
}
