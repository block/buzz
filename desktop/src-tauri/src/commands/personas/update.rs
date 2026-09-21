//! The persona edit command surface: `update_persona` (best-effort enqueue)
//! and the `update_persona_with` seam that `update_persona_and_publish` reuses
//! to await relay acceptance for the same save.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        apply_persona_behavior, effective_agent_command, load_managed_agents, load_personas,
        managed_agent_avatar_url, save_personas, try_regenerate_nest,
        validate_agent_definition_text, AgentDefinition, ManagedAgentRecord, UpdatePersonaRequest,
    },
    util::now_iso,
};

use super::{normalize_description, pending, retain_persona_pending, trim_optional, trim_required};

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

#[derive(Debug, PartialEq, Eq)]
struct LinkedProfileUpdate {
    /// Whether this update changed bytes in the managed-agent record.
    record_changed: bool,
    /// Whether this instance needs a complete kind:0 replacement event.
    profile_sync_required: bool,
    /// Avatar to publish with the complete kind:0 replacement event.
    profile_avatar: Option<String>,
}

/// Apply the persisted portion of a persona identity edit to one linked
/// instance and resolve the avatar for the complete kind:0 replacement.
///
/// Description-only edits deliberately leave the record unchanged, but still
/// need a non-empty avatar projection for legacy records whose `avatar_url`
/// has not yet been backfilled. The persona avatar is authoritative there;
/// the effective command icon is the final fallback.
fn prepare_linked_profile_update(
    record: &mut ManagedAgentRecord,
    persona: &AgentDefinition,
    renamed: bool,
    avatar_changed: bool,
    about_changed: bool,
) -> LinkedProfileUpdate {
    let mut record_changed = renamed;
    if avatar_changed {
        let effective_cmd = effective_agent_command(
            record.persona_id.as_deref(),
            std::slice::from_ref(persona),
            record.agent_command_override.as_deref(),
        );
        record.avatar_url = persona
            .avatar_url
            .clone()
            .or_else(|| managed_agent_avatar_url(&effective_cmd));
        record_changed = true;
    }

    let effective_cmd = effective_agent_command(
        record.persona_id.as_deref(),
        std::slice::from_ref(persona),
        record.agent_command_override.as_deref(),
    );
    let profile_avatar = record
        .avatar_url
        .clone()
        .or_else(|| persona.avatar_url.clone())
        .or_else(|| managed_agent_avatar_url(&effective_cmd));

    LinkedProfileUpdate {
        record_changed,
        profile_sync_required: record_changed || about_changed,
        profile_avatar,
    }
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

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LinkedBehaviorApplied {
    pub(crate) policy_pubkeys: Vec<String>,
    pub(crate) runtime_pubkeys: Vec<String>,
}

/// Apply a definition behavior replacement to every linked instance. The
/// returned proof is consumed by the command's atomic save/policy/restart path;
/// removing this production call leaves that path without its required input.
pub(crate) fn apply_linked_persona_behavior(
    records: &mut [ManagedAgentRecord],
    persona: &AgentDefinition,
) -> Result<LinkedBehaviorApplied, String> {
    let respond_to = match persona.respond_to.as_deref() {
        Some(value) => crate::managed_agents::RespondTo::parse_wire(value)?,
        None => crate::managed_agents::RespondTo::default(),
    };
    let allowlist = if respond_to == crate::managed_agents::RespondTo::Allowlist {
        persona.respond_to_allowlist.clone()
    } else {
        Vec::new()
    };
    let parallelism = persona
        .parallelism
        .unwrap_or(crate::managed_agents::DEFAULT_AGENT_PARALLELISM);
    let session_policy = persona.session_policy;
    let mut applied = LinkedBehaviorApplied::default();

    for record in records
        .iter_mut()
        .filter(|record| record.persona_id.as_deref() == Some(&persona.id))
    {
        let runtime_changed = record.respond_to != respond_to
            || record.respond_to_allowlist != allowlist
            || record.parallelism != parallelism
            || record.session_policy != session_policy;
        if runtime_changed {
            record.respond_to = respond_to;
            record.respond_to_allowlist.clone_from(&allowlist);
            record.parallelism = parallelism;
            record.session_policy = session_policy;
            record.updated_at = now_iso();
        }
        // A repeated Profile save is also the durable recovery action after an
        // ambiguous timeout or failed rollback. Always prove policy delivery
        // and restart active local pairs, even when disk already holds the
        // requested values; otherwise a 10→3 failure followed by retrying 3
        // could incorrectly no-op while the live harness still runs at 10.
        applied.policy_pubkeys.push(record.pubkey.clone());
        if record.backend == crate::managed_agents::BackendKind::Local {
            applied.runtime_pubkeys.push(record.pubkey.clone());
        } else if runtime_changed && record.backend_agent_id.is_some() {
            return Err(format!(
                "Behavior cannot be changed while provider-backed agent {} is deployed because the provider protocol cannot acknowledge a runtime restart. Stop or recreate it first.",
                record.pubkey
            ));
        }
    }
    Ok(applied)
}

#[derive(Debug)]
struct LinkedBehaviorTransition {
    previous_personas: Vec<AgentDefinition>,
    previous_records: Vec<ManagedAgentRecord>,
    committed_persona: AgentDefinition,
    committed_records: Vec<ManagedAgentRecord>,
    policy_pubkeys: Vec<String>,
    runtime_pairs: Vec<(String, Vec<String>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RollbackOutcome {
    Applied,
    Superseded,
    /// The old policy could not be queued transactionally, so local state was
    /// restored to the committed generation. The caller must realign runtime
    /// to that committed generation before returning the failure.
    Committed,
}

fn linked_record_config_matches(left: &ManagedAgentRecord, right: &ManagedAgentRecord) -> bool {
    left.name == right.name
        && left.display_name == right.display_name
        && left.avatar_url == right.avatar_url
        && left.respond_to == right.respond_to
        && left.respond_to_allowlist == right.respond_to_allowlist
        && left.parallelism == right.parallelism
        && left.session_policy == right.session_policy
}

/// Restore only the fields owned by this persona edit, and only while the
/// committed generation is still current. Runtime receipts and unrelated
/// records are deliberately preserved.
fn apply_linked_behavior_rollback_if_current(
    current_personas: &mut [AgentDefinition],
    current_records: &mut [ManagedAgentRecord],
    transition: &LinkedBehaviorTransition,
) -> Result<RollbackOutcome, String> {
    let current_persona = current_personas
        .iter_mut()
        .find(|persona| persona.id == transition.committed_persona.id)
        .ok_or_else(|| "updated agent definition disappeared during rollback".to_string())?;
    if current_persona != &transition.committed_persona {
        return Ok(RollbackOutcome::Superseded);
    }

    for pubkey in &transition.policy_pubkeys {
        let current = current_records
            .iter()
            .find(|record| &record.pubkey == pubkey)
            .ok_or_else(|| format!("linked agent {pubkey} disappeared during rollback"))?;
        let committed = transition
            .committed_records
            .iter()
            .find(|record| &record.pubkey == pubkey)
            .ok_or_else(|| format!("committed agent {pubkey} is missing from rollback proof"))?;
        if !linked_record_config_matches(current, committed) {
            return Ok(RollbackOutcome::Superseded);
        }
    }

    let previous_persona = transition
        .previous_personas
        .iter()
        .find(|persona| persona.id == transition.committed_persona.id)
        .ok_or_else(|| "previous agent definition is missing from rollback proof".to_string())?;
    *current_persona = previous_persona.clone();

    for pubkey in &transition.policy_pubkeys {
        let current = current_records
            .iter_mut()
            .find(|record| &record.pubkey == pubkey)
            .ok_or_else(|| format!("linked agent {pubkey} disappeared during rollback"))?;
        let previous = transition
            .previous_records
            .iter()
            .find(|record| &record.pubkey == pubkey)
            .ok_or_else(|| format!("previous agent {pubkey} is missing from rollback proof"))?;
        current.name.clone_from(&previous.name);
        current.display_name.clone_from(&previous.display_name);
        current.avatar_url.clone_from(&previous.avatar_url);
        current.respond_to = previous.respond_to;
        current
            .respond_to_allowlist
            .clone_from(&previous.respond_to_allowlist);
        current.parallelism = previous.parallelism;
        current.session_policy = previous.session_policy;
        current.updated_at = now_iso();
    }
    Ok(RollbackOutcome::Applied)
}

fn rollback_linked_behavior<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    transition: &LinkedBehaviorTransition,
) -> Result<RollbackOutcome, String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut current_personas = load_personas(app)?;
    let mut current_records = load_managed_agents(app)?;
    let outcome = apply_linked_behavior_rollback_if_current(
        &mut current_personas,
        &mut current_records,
        transition,
    )?;
    if outcome == RollbackOutcome::Superseded {
        return Ok(outcome);
    }
    crate::managed_agents::save_agent_definitions_and_instances(
        app,
        &current_personas,
        &current_records,
    )?;
    let previous_policies = transition
        .previous_records
        .iter()
        .filter(|record| transition.policy_pubkeys.contains(&record.pubkey))
        .collect::<Vec<_>>();
    if let Err(error) =
        crate::commands::agents::try_retain_managed_agents_pending(app, state, &previous_policies)
    {
        // The retention transaction did not change. Restore the committed
        // generation locally so disk and the still-pending policy agree.
        let committed = transition
            .committed_records
            .iter()
            .filter(|record| transition.policy_pubkeys.contains(&record.pubkey))
            .collect::<Vec<_>>();
        *current_personas
            .iter_mut()
            .find(|persona| persona.id == transition.committed_persona.id)
            .ok_or_else(|| "updated agent definition disappeared during recovery".to_string())? =
            transition.committed_persona.clone();
        for record in committed {
            let current = current_records
                .iter_mut()
                .find(|candidate| candidate.pubkey == record.pubkey)
                .ok_or_else(|| {
                    format!("linked agent {} disappeared during recovery", record.pubkey)
                })?;
            current.name.clone_from(&record.name);
            current.display_name.clone_from(&record.display_name);
            current.avatar_url.clone_from(&record.avatar_url);
            current.respond_to = record.respond_to;
            current
                .respond_to_allowlist
                .clone_from(&record.respond_to_allowlist);
            current.parallelism = record.parallelism;
            current.session_policy = record.session_policy;
            current.updated_at = now_iso();
        }
        crate::managed_agents::save_agent_definitions_and_instances(
            app,
            &current_personas,
            &current_records,
        )?;
        eprintln!("buzz-desktop: managed policy rollback could not be queued: {error}");
        return Ok(RollbackOutcome::Committed);
    }
    try_regenerate_nest(app);
    Ok(outcome)
}

fn restart_linked_behavior_pairs(
    app: &AppHandle,
    pairs: &[(String, Vec<String>)],
) -> Result<(), String> {
    restart_linked_behavior_pairs_with(pairs, |pubkey, relay_url| {
        crate::managed_agents::restart_managed_agent_runtime(
            pubkey.to_string(),
            relay_url.to_string(),
            app.clone(),
        )
        .map(|_| ())
    })
}

/// Production-entered restart seam. Tests inject a recorder/failure to prove
/// every active `(agent, relay)` pair is actually visited; the real wrapper
/// above injects the Tauri runtime restart command.
fn restart_linked_behavior_pairs_with(
    pairs: &[(String, Vec<String>)],
    mut restart: impl FnMut(&str, &str) -> Result<(), String>,
) -> Result<(), String> {
    for (pubkey, relays) in pairs {
        for relay_url in relays {
            restart(pubkey, relay_url)?;
        }
    }
    Ok(())
}

/// Strictly enqueue every changed kind:30177 projection. The unified store is
/// already on the new generation when this runs; a transactional enqueue
/// failure restores the previous file generation before returning an error.
fn retain_linked_policies_or_rollback<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    policy_pubkeys: &[String],
    records: &[ManagedAgentRecord],
    previous_personas: &[AgentDefinition],
    previous_records: &[ManagedAgentRecord],
) -> Result<Vec<ManagedAgentRecord>, String> {
    if policy_pubkeys.is_empty() {
        return Ok(Vec::new());
    }
    let committed_records = policy_pubkeys
        .iter()
        .map(|pubkey| {
            records
                .iter()
                .find(|record| &record.pubkey == pubkey)
                .cloned()
                .ok_or_else(|| format!("linked agent {pubkey} disappeared during save"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let policy_records = committed_records.iter().collect::<Vec<_>>();
    if let Err(error) =
        crate::commands::agents::try_retain_managed_agents_pending(app, state, &policy_records)
    {
        crate::managed_agents::save_agent_definitions_and_instances(
            app,
            previous_personas,
            previous_records,
        )?;
        return Err(format!(
            "agent definition update was rolled back because managed policy enqueue failed: {error}"
        ));
    }
    Ok(committed_records)
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
    let (result, retained, profile_sync_params, behavior_transition) =
        tokio::task::spawn_blocking({
            let app = app.clone();
            move ||
              -> Result<
            (
                AgentDefinition,
                R,
                ProfileSyncParams,
                Option<LinkedBehaviorTransition>,
            ),
            String,
        > {
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
            let previous_personas = personas.clone();
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
            let behavior_update_requested = input.behavior.is_some();
            apply_persona_behavior(persona, input.behavior)?;
            persona.updated_at = now_iso();

            let result = persona.clone();
            let previous_records = load_managed_agents(&app)?;
            let mut records = previous_records.clone();
            let behavior_applied = if behavior_update_requested {
                apply_linked_persona_behavior(&mut records, &result)?
            } else {
                LinkedBehaviorApplied::default()
            };

            let mut params: ProfileSyncParams = Vec::new();
            let workspace_relay = crate::relay::relay_ws_url_with_override(&state);
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
            let mut profile_record_changed = false;
            if avatar_changed || name_changed || about_changed {
                for record in records.iter_mut() {
                    if record.persona_id.as_deref() != Some(&result.id) {
                        continue;
                    }
                    let update = prepare_linked_profile_update(
                        record,
                        &result,
                        renamed.contains(&record.pubkey),
                        avatar_changed,
                        about_changed,
                    );
                    profile_record_changed |= update.record_changed;
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
            }

            let instances_changed = profile_record_changed
                || !behavior_applied.policy_pubkeys.is_empty()
                || !renamed.is_empty();
            if instances_changed {
                crate::managed_agents::save_agent_definitions_and_instances(
                    &app, &personas, &records,
                )?;
            } else {
                save_personas(&app, &personas)?;
            }

            let retained = match retain(&app, &state, &result) {
                Ok(retained) => retained,
                Err(error) => {
                    if instances_changed {
                        crate::managed_agents::save_agent_definitions_and_instances(
                            &app,
                            &previous_personas,
                            &previous_records,
                        )?;
                    }
                    return Err(error);
                }
            };

            let mut policy_pubkeys = behavior_applied.policy_pubkeys;
            for pubkey in renamed {
                if !policy_pubkeys.contains(&pubkey) {
                    policy_pubkeys.push(pubkey);
                }
            }
            let committed_records = retain_linked_policies_or_rollback(
                &app,
                &state,
                &policy_pubkeys,
                &records,
                &previous_personas,
                &previous_records,
            )?;

            let mut runtime_pairs = Vec::new();
            if !behavior_applied.runtime_pubkeys.is_empty() {
                let runtimes = state
                    .managed_agent_processes
                    .lock()
                    .map_err(|error| error.to_string())?;
                for pubkey in &behavior_applied.runtime_pubkeys {
                    let mut relays =
                        crate::managed_agents::managed_agent_runtime_keys(&runtimes, pubkey)
                            .into_iter()
                            .map(|key| key.relay_url)
                            .collect::<Vec<_>>();
                    let record = records
                        .iter()
                        .find(|record| &record.pubkey == pubkey)
                        .ok_or_else(|| format!("linked agent {pubkey} disappeared during save"))?;
                    if relays.is_empty() && record.runtime_pid.is_some() {
                        relays.push(crate::relay::effective_agent_relay_url(
                            &record.relay_url,
                            &workspace_relay,
                        ));
                    }
                    if !relays.is_empty() {
                        runtime_pairs.push((pubkey.clone(), relays));
                    }
                }
            }

            try_regenerate_nest(&app);
            let transition = (!policy_pubkeys.is_empty()).then_some(LinkedBehaviorTransition {
                previous_personas,
                previous_records,
                committed_persona: result.clone(),
                committed_records,
                policy_pubkeys,
                runtime_pairs,
            });

            Ok((result, retained, params, transition))
        }
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    if let Some(transition) = behavior_transition {
        let state = app.state::<AppState>();
        let mut policy_error =
            crate::managed_agents::persona_events::flush_active_pending_events(&app, &state)
                .await
                .err()
                .map(|error| format!("managed policy sync failed: {error}"));
        if policy_error.is_none() {
            for pubkey in &transition.policy_pubkeys {
                if crate::managed_agents::persona_events::active_pending_event(
                    &app,
                    &state,
                    buzz_core_pkg::kind::KIND_MANAGED_AGENT,
                    pubkey,
                )? {
                    policy_error = Some(
                        "managed policy sync failed: relay did not accept the updated policy; retry queued"
                            .to_string(),
                    );
                    break;
                }
            }
        }
        if let Some(error) = policy_error {
            let rollback = rollback_linked_behavior(&app, &state, &transition)?;
            if rollback != RollbackOutcome::Superseded {
                let _ = crate::managed_agents::persona_events::flush_active_pending_events(
                    &app, &state,
                )
                .await;
            }
            return Err(match rollback {
                RollbackOutcome::Applied => {
                    format!("Agent definition update was rolled back because {error}")
                }
                RollbackOutcome::Superseded => format!(
                    "Agent definition update failed because {error}; a newer edit superseded this save and was preserved"
                ),
                RollbackOutcome::Committed => {
                    let runtime_recovery = restart_linked_behavior_pairs(
                        &app,
                        &transition.runtime_pairs,
                    )
                    .err()
                    .map(|restart| format!(" Runtime realignment also failed: {restart}"))
                    .unwrap_or_default();
                    format!(
                        "Agent definition update failed because {error}; the committed configuration was preserved.{runtime_recovery}"
                    )
                }
            });
        }

        if let Err(error) = restart_linked_behavior_pairs(&app, &transition.runtime_pairs) {
            let rollback = rollback_linked_behavior(&app, &state, &transition)?;
            return match rollback {
                RollbackOutcome::Superseded => Err(format!(
                    "Agent runtime restart failed: {error}. A newer edit superseded this save and was preserved"
                )),
                RollbackOutcome::Applied => {
                    let _ = crate::managed_agents::persona_events::flush_active_pending_events(
                        &app, &state,
                    )
                    .await;
                    let restore_error =
                        restart_linked_behavior_pairs(&app, &transition.runtime_pairs)
                            .err()
                            .map(|restore| {
                                format!(" Previous runtime recovery also failed: {restore}")
                            })
                            .unwrap_or_default();
                    Err(format!(
                        "Agent definition update was rolled back because its runtime failed to restart: {error}.{restore_error}"
                    ))
                }
                RollbackOutcome::Committed => {
                    let _ = crate::managed_agents::persona_events::flush_active_pending_events(
                        &app, &state,
                    )
                    .await;
                    let recovery_error =
                        restart_linked_behavior_pairs(&app, &transition.runtime_pairs)
                            .err()
                            .map(|restore| {
                                format!(" Runtime realignment also failed: {restore}")
                            })
                            .unwrap_or_default();
                    Err(format!(
                        "Agent runtime restart failed: {error}. The committed configuration was preserved.{recovery_error}"
                    ))
                }
            };
        }
    }

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
