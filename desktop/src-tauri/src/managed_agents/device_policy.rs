//! Host/observer separation belongs to the device, never synchronized definitions.

pub(crate) mod model;
pub(crate) mod sync;
pub(crate) mod unique_names;
use model::{active_policy, load_policy, DeviceAgentPolicy};
use serde::Serialize;
use tauri::{AppHandle, Manager};

use super::storage::{atomic_write_json_restricted, managed_agents_base_dir};
use crate::app_state::AppState;

/// Return the policy fixed for this Desktop process, including cached read errors.
pub(crate) fn active<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<DeviceAgentPolicy, String> {
    let state = app.state::<AppState>();
    active_policy(&state.agent_device_policy, || {
        let path = managed_agents_base_dir(app)?.join("agent-device-policy.json");
        let policy = load_policy(&path)?;
        tracing::info!(
            client_only = policy.client_only,
            unique_names = policy.unique_names,
            preferred_identities = policy.preferred_agents.len(),
            "loaded device agent policy"
        );
        Ok(policy)
    })
    .cloned()
}

/// Invalid policy disables automatic management while leaving chat available.
pub(crate) fn is_client_only<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    active(app).map_or(true, |policy| policy.client_only)
}

/// Retain old queues and keep this device's templates local in unique-name mode.
pub(crate) fn pauses_sync<R: tauri::Runtime>(app: &AppHandle<R>) -> bool {
    active(app).map_or(true, |policy| policy.client_only || policy.unique_names)
}

/// Check both identity and name before local management or execution.
pub(crate) fn require_record<R: tauri::Runtime>(
    app: &AppHandle<R>,
    record: &super::ManagedAgentRecord,
) -> Result<(), String> {
    active(app)?.require_local_agent(
        &record.name,
        Some(&record.pubkey),
        record.persona_id.as_deref(),
    )
}

/// A presentation filter only; callers retain the complete persistent store.
pub(crate) fn can_host_record<R: tauri::Runtime>(
    app: &AppHandle<R>,
    record: &super::ManagedAgentRecord,
) -> bool {
    require_record(app, record).is_ok()
}

/// Protect a stable remote definition as well as its current name.
pub(crate) fn require_persona<R: tauri::Runtime>(
    app: &AppHandle<R>,
    id: &str,
) -> Result<(), String> {
    let policy = active(app)?;
    let personas = super::load_personas(app)?;
    let name = personas
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.display_name.as_str())
        .unwrap_or("");
    policy.require_local_agent(name, None, Some(id))?;
    for record in super::load_managed_agents(app)?
        .iter()
        .filter(|record| record.persona_id.as_deref() == Some(id))
    {
        require_record(app, record)?;
    }
    Ok(())
}

/// Team edits cannot cascade into protected remote definitions.
pub(crate) fn require_team<R: tauri::Runtime>(app: &AppHandle<R>, id: &str) -> Result<(), String> {
    require_hosting(app)?;
    if let Some(team) = super::load_teams(app)?.iter().find(|team| team.id == id) {
        for persona_id in &team.persona_ids {
            require_persona(app, persona_id)?;
        }
        if team.source_dir.is_some() {
            let key = super::team_persona_key(team);
            for persona in super::load_personas(app)?
                .iter()
                .filter(|p| p.source_team.as_deref() == Some(key))
            {
                require_persona(app, &persona.id)?;
            }
        }
    }
    Ok(())
}

/// Generate an agent key only on a hosting device. The identity login/pairing
/// keys use their independent paths and are unaffected.
pub(crate) fn generate_agent_keys<R: tauri::Runtime>(
    app: &AppHandle<R>,
    name: &str,
    persona_id: Option<&str>,
) -> Result<nostr::Keys, String> {
    active(app)?.require_local_agent(name, None, persona_id)?;
    Ok(nostr::Keys::generate())
}

/// Native guard shared by all key creation and execution paths.
pub(crate) fn require_hosting<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    active(app)?.require_hosting()
}

/// Profile archive actions must preserve the protected remote identity too.
pub(crate) fn require_identity_archive<R: tauri::Runtime>(
    app: &AppHandle<R>,
    target: &str,
) -> Result<(), String> {
    let policy = active(app)?;
    if !policy.client_only && !policy.unique_names {
        return Ok(());
    }
    let key = nostr::PublicKey::parse(target)
        .map_err(|e| e.to_string())?
        .to_hex();
    if policy
        .preferred_agents
        .iter()
        .any(|agent| agent.pubkey.eq_ignore_ascii_case(&key))
    {
        return Err("Manage this protected identity on the device that hosts it.".into());
    }
    Ok(())
}

/// Batch catalog imports can rename whole teams; unique-name mode imports individually.
pub(crate) fn require_full_hosting<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let policy = active(app)?;
    policy.require_hosting()?;
    if policy.unique_names {
        return Err("This device keeps agent definitions local. Import agents individually with unique names; catalog sharing and team imports require unrestricted hosting.".into());
    }
    Ok(())
}

/// Saved preference and effective policy are separate until Desktop restarts.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAgentPolicyStatus {
    active_client_only: bool,
    active_unique_names: bool,
    saved: DeviceAgentPolicy,
    restart_required: bool,
    load_error: Option<String>,
}

/// Read this device's hosting setting without changing any agent definition.
#[tauri::command]
pub fn get_agent_device_policy(app: AppHandle) -> Result<DeviceAgentPolicyStatus, String> {
    let current = active(&app);
    let saved = load_policy(&managed_agents_base_dir(&app)?.join("agent-device-policy.json"));
    let load_error = current.as_ref().err().or(saved.as_ref().err()).cloned();
    let active_client_only = current.as_ref().map_or(true, |policy| policy.client_only);
    let active_unique_names = current.as_ref().is_ok_and(|policy| policy.unique_names);
    let saved = saved.unwrap_or_else(|_| DeviceAgentPolicy {
        client_only: true,
        ..Default::default()
    });
    let restart_required = current.as_ref() != Ok(&saved);
    Ok(DeviceAgentPolicyStatus {
        active_client_only,
        active_unique_names,
        saved,
        restart_required,
        load_error,
    })
}

/// Persist an atomic, device-only preference. Execution changes after restart.
#[tauri::command]
pub fn set_agent_device_policy(
    policy: DeviceAgentPolicy,
    app: AppHandle,
) -> Result<DeviceAgentPolicyStatus, String> {
    // Freeze the old policy before saving, even if no execution path has run yet.
    // A malformed old file remains fail-closed until restart, but is recoverable here.
    let current = active(&app);
    if current.as_ref().is_ok_and(|old| {
        old.unique_names && policy.unique_names && old.preferred_agents != policy.preferred_agents
    }) {
        return Err("The protected remote identities cannot be removed while unique-name hosting is enabled.".into());
    }
    let bytes = serde_json::to_vec_pretty(&policy).map_err(|error| error.to_string())?;
    if bytes.len() > 65_536 {
        return Err("Agent device policy exceeds 64 KiB".into());
    }
    let path = managed_agents_base_dir(&app)?.join("agent-device-policy.json");
    atomic_write_json_restricted(&path, &bytes)?;
    get_agent_device_policy(app)
}

#[cfg(test)]
mod tests;
