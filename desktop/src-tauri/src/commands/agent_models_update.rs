use super::*;

pub(crate) fn managed_agent_access_policy_changed(
    current_mode: crate::managed_agents::RespondTo,
    current_allowlist: &[String],
    prospective_mode: crate::managed_agents::RespondTo,
    prospective_allowlist: &[String],
    enforced_owner_only: bool,
) -> bool {
    // Stored policy remains portable across OSS and owner-only builds, but a
    // marked build always projects both states to the same owner-only runtime
    // gate. Do not restart a fleet merely because relay state differs in bytes
    // that this build cannot execute.
    if enforced_owner_only {
        return false;
    }
    prospective_mode != current_mode
        || (prospective_mode == crate::managed_agents::RespondTo::Allowlist
            && prospective_allowlist != current_allowlist)
}

fn ensure_access_policy_change_supported(
    record: &ManagedAgentRecord,
    access_policy_changed: bool,
) -> Result<(), String> {
    if access_policy_changed
        && record.backend != crate::managed_agents::BackendKind::Local
        && record.backend_agent_id.is_some()
    {
        return Err(
            "Access cannot be changed while this provider-backed agent is deployed because the provider protocol has no explicit stop or revocation acknowledgement. Stop or recreate the provider agent first."
                .to_string(),
        );
    }
    Ok(())
}

/// Reject an effort mutation for a non-local record. Remote effort is
/// deployment-owned (set via `policy_env` at deploy time); persisting locally
/// would make the canonical column diverge from the deployed runtime's actual
/// effort.
fn ensure_effort_change_supported(
    record: &ManagedAgentRecord,
    effort_level: &Option<Option<String>>,
) -> Result<(), String> {
    if effort_level.is_some() && record.backend != crate::managed_agents::BackendKind::Local {
        return Err(format!(
            "agent {} is not a local agent; remote effort is set at deploy time",
            record.pubkey
        ));
    }
    Ok(())
}

/// Guard/apply seam for the effort step inside `apply_record_field_updates`.
fn apply_effort_update(
    record: &mut ManagedAgentRecord,
    effort_level: Option<Option<String>>,
) -> Result<(), String> {
    ensure_effort_change_supported(record, &effort_level)?;
    if let Some(effort_override) = effort_level {
        crate::commands::agent_config::apply_picker_effort_level(record, effort_override);
    }
    Ok(())
}

/// Proof token returned by `apply_record_field_updates`. Zero-size and
/// `#[must_use]`; consumed by `stamp_record_updated_at`, so removing the
/// `apply_record_field_updates` call from `update_managed_agent` leaves
/// `applied` undefined at the timestamp site — a compile error.
#[derive(Debug)]
#[must_use]
pub(crate) struct RecordFieldsApplied(());

/// Apply the env-vars and effort steps of `update_managed_agent` to a record
/// in the correct order: env_vars FIRST (so the same-request map cannot
/// reintroduce a stale alias), then the canonical effort column write.
///
/// Returns a `RecordFieldsApplied` token that must be passed to
/// `stamp_record_updated_at`. Removing this call from `update_managed_agent`
/// leaves `applied` undefined at the timestamp site — a compile error.
///
/// Called by `update_managed_agent` inside its locked transaction and by tests.
/// Any step deleted from inside this function is directly caught by the
/// corresponding test assertion.
///
/// Mutation proofs (see `agent_models_update_tests.rs`):
///   - Deleting the `apply_effort_update` call leaves `effort_level` unchanged.
///   - Deleting `ensure_effort_change_supported` inside `apply_effort_update`
///     lets non-local writes pass `Ok(())` without mutating the column.
///   - Deleting `apply_picker_effort_level` inside `apply_effort_update`
///     leaves `effort_level == None` on a local-set request.
pub(crate) fn apply_record_field_updates(
    record: &mut ManagedAgentRecord,
    env_vars: Option<&std::collections::BTreeMap<String, String>>,
    inherit_transition: bool,
    effort_level: Option<Option<String>>,
) -> Result<RecordFieldsApplied, String> {
    // Order is load-bearing: env_vars before effort so a same-request
    // env_vars map cannot reintroduce a stale alias after the column write.
    crate::managed_agents::apply_env_vars_then_effort_transition(
        record,
        env_vars.cloned(),
        inherit_transition,
    );
    apply_effort_update(record, effort_level)?;
    Ok(RecordFieldsApplied(()))
}

/// Stamp `record.updated_at` with the current ISO timestamp, consuming the
/// `RecordFieldsApplied` proof token. Removing `apply_record_field_updates`
/// from `update_managed_agent` leaves `applied` undefined here — a compile error.
pub(crate) fn stamp_record_updated_at(
    record: &mut ManagedAgentRecord,
    _applied: RecordFieldsApplied,
) {
    record.updated_at = crate::util::now_iso();
}

/// Flush a retained managed-agent policy, preserving any earlier profile error.
pub(crate) async fn flush_managed_agent_policy(
    app: &AppHandle,
    state: &AppState,
    existing_error: Option<String>,
) -> Option<String> {
    match crate::managed_agents::persona_events::flush_active_pending_events(app, state).await {
        Ok(_) => existing_error,
        Err(error) => Some(match existing_error {
            Some(profile_error) => {
                format!("{profile_error}; managed policy sync failed: {error}")
            }
            None => format!("managed policy sync failed: {error}"),
        }),
    }
}

/// Update mutable fields on an existing managed agent record.
///
/// Most runtime config changes take effect on the next agent spawn. Access
/// policy changes stop active local pairs before saving and restart those exact
/// pairs after the relay policy is flushed.
#[tauri::command]
pub async fn update_managed_agent(
    input: UpdateManagedAgentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateManagedAgentResponse, String> {
    // Phase 1: local save (synchronous, under lock)
    let (mut summary, sync_params, rollback, access_policy_changed, access_restart_relays) = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| e.to_string())?;
        let mut records = load_managed_agents(&app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| e.to_string())?;
        let (_, exited_pubkeys) =
            sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(&app));
        for pubkey in &exited_pubkeys {
            state.clear_agent_session_caches(pubkey);
        }

        let record = find_managed_agent_mut(&mut records, &input.pubkey)?;
        let previous_record = record.clone();

        let mut name_changed = false;
        if let Some(name_update) = input.name {
            let trimmed = name_update.trim().to_string();
            if !trimmed.is_empty() && trimmed != record.name {
                record.name = trimmed;
                name_changed = true;
            }
        }
        apply_model_provider_prompt_update(
            record,
            input.model,
            input.provider,
            input.system_prompt,
        )?;
        if let Some(parallelism) = input.parallelism {
            record.parallelism = parallelism;
        }
        // turn_timeout_seconds is intentionally not applied here —
        // BUZZ_ACP_TURN_TIMEOUT is deprecated and ignored by the harness.
        // Use idle_timeout_seconds or max_turn_duration_seconds instead.
        // Store the relay override exactly as supplied (trimmed). An explicit
        // value pins the agent; empty falls back to the workspace relay at
        // read-time. A name-only edit (relay_url == None) leaves the pin intact.
        if let Some(relay_url) = input.relay_url {
            record.relay_url = relay_url.trim().to_string();
        }
        if let Some(acp_command) = input.acp_command {
            record.acp_command = acp_command;
        }
        // Harness edit: the persona's runtime is authoritative, so an explicit
        // `agent_command_override` is persisted ONLY when the user picks a
        // command that diverges from the persona, and the empty/whitespace
        // "Inherit from persona" sentinel clears the pin, the materialized
        // record runtime, AND the per-instance effort override (column here,
        // env aliases after `env_vars` is applied below). A name-only edit
        // (`agent_command == None`) leaves the pin intact. `harness_override`
        // threads the user's explicit intent — see `apply_agent_command_update`
        // and `update_time_agent_command_override` for the full resolution
        // rules.
        let mut inherit_transition = false;
        if let Some(agent_command) = input.agent_command {
            let personas = load_personas(&app).unwrap_or_default();
            inherit_transition = crate::managed_agents::apply_agent_command_update(
                record,
                &personas,
                &agent_command,
                input.harness_override,
            );
        }
        if let Some(agent_args) = input.agent_args {
            record.agent_args = agent_args;
        }
        // mcp_command is intentionally not applied here — the effective MCP
        // command is always catalog-derived (known_acp_runtime at spawn time)
        // and the per-record field is never read by the runtime.
        //
        // Apply the caller-supplied `env_vars` (validated first), then — only on
        // the pin→inherit transition — strip the record effort env aliases. The
        // order is load-bearing: stripping AFTER the env replacement is what
        // stops a same-request `env_vars` map from reintroducing a stale effort
        // alias while the instance inherits its harness. The column was already
        // cleared inside `apply_agent_command_update`. See
        // `apply_env_vars_then_effort_transition` for the pinned invariant.
        if let Some(ref env_vars) = input.env_vars {
            crate::managed_agents::validate_user_env_keys(env_vars)?;
        }

        // Native provider/model fields are authoritative. Keep the typed marker
        // derived for new records while retaining legacy typed records for
        // non-native providers.
        if record.provider.as_deref() == Some(crate::managed_agents::RELAY_MESH_PROVIDER_ID) {
            let model_ref = record
                .model
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(crate::managed_agents::RELAY_MESH_AUTO_MODEL_ID)
                .to_string();
            record.model = Some(model_ref.clone());
            record.relay_mesh = Some(crate::managed_agents::RelayMeshConfig { model_ref });
        }

        // Inbound author gate: merge patch onto current values, then validate
        // the merged state. This lets a single update switch to Allowlist AND
        // supply pubkeys atomically.
        let prospective_mode = input.respond_to.unwrap_or(record.respond_to);
        let prospective_allowlist = match input.respond_to_allowlist.as_ref() {
            Some(list) => crate::managed_agents::validate_respond_to_allowlist(list)?,
            None => record.respond_to_allowlist.clone(),
        };
        if prospective_mode == crate::managed_agents::RespondTo::Allowlist
            && prospective_allowlist.is_empty()
        {
            return Err(
                "respond-to mode 'allowlist' requires at least one pubkey in the allowlist"
                    .to_string(),
            );
        }
        let access_policy_changed = managed_agent_access_policy_changed(
            record.respond_to,
            &record.respond_to_allowlist,
            prospective_mode,
            &prospective_allowlist,
            crate::managed_agents::owner_only_access_build(),
        );
        ensure_access_policy_change_supported(record, access_policy_changed)?;

        // Revoke the currently running local gate before persisting or
        // advertising the replacement policy. Keeping this inside the same
        // store/process critical section prevents another command or a status
        // refresh from observing a saved narrow policy while the old broad
        // process is still alive. A stop failure aborts before mutation.
        let mut access_restart_relays = Vec::new();
        if access_policy_changed && record.backend == crate::managed_agents::BackendKind::Local {
            access_restart_relays =
                crate::managed_agents::managed_agent_runtime_keys(&runtimes, &record.pubkey)
                    .into_iter()
                    .map(|key| key.relay_url)
                    .collect();
            if access_restart_relays.is_empty() && record.runtime_pid.is_some() {
                access_restart_relays.push(crate::relay::effective_agent_relay_url(
                    &record.relay_url,
                    &relay_ws_url_with_override(&state),
                ));
            }
            if !access_restart_relays.is_empty() {
                crate::managed_agents::stop_managed_agent_process(&app, record, &mut runtimes)?;
            }
        }

        record.respond_to = prospective_mode;
        // Preserve the persisted allowlist across mode toggles — only replace
        // when the caller explicitly supplied a new list.
        if input.respond_to_allowlist.is_some() {
            record.respond_to_allowlist = prospective_allowlist;
        }

        // Effort + env_vars: applied together inside `apply_record_field_updates` to
        // enforce the ordering invariant (env_vars before effort column write) and
        // provide a directly-testable production seam. Effort persists inside the
        // locked transaction so an access-policy restart above snapshots and
        // launches the new effort value. Present+Some(v)=set; Present+None=clear;
        // Absent=don't touch (the dialog sends it only when effortTouched).
        // The returned token is consumed by `stamp_record_updated_at`; removing
        // this call from `update_managed_agent` leaves `applied` undefined there
        // — a compile error (the sole outer-seam proof for this call site).
        let applied = apply_record_field_updates(
            record,
            input.env_vars.as_ref(),
            inherit_transition,
            input.effort_level,
        )?;

        stamp_record_updated_at(record, applied);

        save_managed_agents(&app, &records)?;

        let record = records
            .iter()
            .find(|r| r.pubkey == input.pubkey)
            .ok_or_else(|| format!("agent {} not found", input.pubkey))?;

        // Publish the edit to the relay. After-save, inside the lock, before
        // any .await. The retention upsert hashes the opt-IN projection, so an
        // update that touched only runtime/local fields is a no-op publish.
        super::super::agents::retain_managed_agent_pending(&app, &state, record);

        let sync_params = if name_changed {
            let agent_keys = Keys::parse(&record.private_key_nsec)
                .map_err(|e| format!("failed to parse agent keys: {e}"))?;
            // Re-publish the renamed profile to the agent's effective relay:
            // an explicit per-agent relay wins; empty falls back to workspace.
            let relay_url = crate::relay::effective_agent_relay_url(
                &record.relay_url,
                &relay_ws_url_with_override(&state),
            );
            let display_name = record.name.clone();
            // Avatar fallback derives from the EFFECTIVE harness (persona-wins),
            // not the frozen snapshot, so an inherited harness picks the right
            // default avatar.
            let personas = load_personas(&app).unwrap_or_default();
            let effective_command = crate::managed_agents::record_agent_command(record, &personas);
            let avatar_url = record
                .avatar_url
                .clone()
                .or_else(|| managed_agent_avatar_url(&effective_command));
            let about = crate::managed_agents::record_effective_description(record, &personas);
            let auth_tag = record.auth_tag.clone();
            Some((
                agent_keys,
                relay_url,
                display_name,
                avatar_url,
                about,
                auth_tag,
            ))
        } else {
            None
        };

        let summary = { super::super::agents::summarize_from_disk(&app, record, &runtimes)? };
        let rollback = name_changed
            .then(|| AgentUpdateRollback::new(previous_record, record, access_policy_changed));
        (
            summary,
            sync_params,
            rollback,
            access_policy_changed,
            access_restart_relays,
        )
    }; // lock dropped here

    try_regenerate_nest(&app);

    // Phase 2: relay sync (async, outside lock). The owner-signed managed
    // policy is security-sensitive: an access reduction must replace the old
    // relay head before this command returns rather than waiting for the
    // 30-second retention sweep. The flush remains durable/best-effort; rows a
    // relay does not accept stay pending for the background retry.
    let mut profile_sync_error =
        crate::managed_agents::persona_events::flush_active_pending_events(&app, &state)
            .await
            .err()
            .map(|error| format!("managed policy sync failed: {error}"));
    if profile_sync_error.is_none()
        && crate::managed_agents::persona_events::active_pending_event(
            &app,
            &state,
            buzz_core_pkg::kind::KIND_MANAGED_AGENT,
            &summary.pubkey,
        )?
    {
        profile_sync_error = Some(
            "managed policy sync failed: relay did not accept the updated policy; retry queued"
                .to_string(),
        );
    }

    // A rename is committed only when profile sync succeeds; otherwise restore
    // the complete pre-edit record so Desktop and the relay keep one
    // authoritative name.
    if let Some((agent_keys, relay_url, display_name, avatar_url, about, auth_tag)) = sync_params {
        if let Err(sync_error) = sync_managed_agent_profile(
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
            let rollback = rollback.ok_or_else(|| {
                "missing local rollback state after relay profile sync failure".to_string()
            })?;
            rollback_failed_agent_update(&app, &state, &summary.pubkey, rollback)?;
            let restart_suffix = if access_restart_relays.is_empty() {
                String::new()
            } else {
                match super::super::agents::start_local_agent_pairs_with_preflight(
                    &app,
                    &state,
                    &summary.pubkey,
                    &access_restart_relays,
                )
                .await
                {
                    Ok(_) => String::new(),
                    Err(error) => format!(
                        " The runtime also failed to restart with the kept access policy: {error}"
                    ),
                }
            };
            let rollback_message = if access_policy_changed {
                "The access policy change was kept, but other edits were rolled back"
            } else {
                "No changes were saved"
            };
            return Err(format!(
                "Agent rename failed because its relay profile could not be updated. {rollback_message}: {sync_error}.{restart_suffix}"
            ));
        }
    }

    if !access_restart_relays.is_empty() {
        summary = super::super::agents::start_local_agent_pairs_with_preflight(
            &app,
            &state,
            &summary.pubkey,
            &access_restart_relays,
        )
        .await
        .map_err(|error| {
            format!(
                "Agent access was saved and published, but its runtime failed to restart with the new policy: {error}"
            )
        })?;
    }

    Ok(UpdateManagedAgentResponse {
        agent: summary,
        profile_sync_error: profile_sync_error.take(),
    })
}

#[cfg(test)]
#[allow(unused_must_use)]
#[path = "agent_models_update_tests.rs"]
mod tests;
