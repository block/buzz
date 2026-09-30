use std::sync::atomic::Ordering;

use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    managed_agents::{
        bestie_assignment::{
            assignment_matches, clear_assignment, get_assignment,
            recover_pending_assignment_cleanup, replace_assignment, BestieAssignment,
        },
        load_managed_agents, managed_agents_base_dir,
        retention::{active_retention_scope, open_retention_db, RetentionScope},
        BackendKind, ManagedAgentRecord,
    },
    models::ChannelInfo,
};

fn canonical_relay(relay_url: &str) -> Result<String, String> {
    buzz_core_pkg::relay::normalize_relay_url(relay_url).map_err(|error| error.to_string())
}

fn assert_expected_scope(
    scope: &RetentionScope,
    expected_relay_url: Option<&str>,
    expected_signer_pubkey: Option<&str>,
) -> Result<(), String> {
    if let Some(expected) = expected_relay_url {
        if canonical_relay(expected)? != canonical_relay(&scope.relay_url)? {
            return Err("active community changed while resolving Bestie".to_string());
        }
    }
    if let Some(expected) = expected_signer_pubkey {
        if expected.trim().to_ascii_lowercase() != scope.owner_keys.public_key().to_hex() {
            return Err("active identity changed while resolving Bestie".to_string());
        }
    }
    Ok(())
}

fn validate_agent_pubkey(pubkey: &str) -> Result<String, String> {
    let normalized = pubkey.trim().to_ascii_lowercase();
    if normalized.len() != 64
        || !normalized
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("Bestie agent pubkey must be 64 hexadecimal characters".to_string());
    }
    Ok(normalized)
}

fn require_eligible_local_agent(
    records: &[ManagedAgentRecord],
    pubkey: &str,
) -> Result<(), String> {
    let record = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(pubkey))
        .ok_or_else(|| "assigned Bestie agent no longer exists on this device".to_string())?;
    if record.backend != BackendKind::Local {
        return Err("only a local managed agent can be your Bestie".to_string());
    }
    Ok(())
}

fn recover_pending_cleanup(app: &AppHandle, records: &[ManagedAgentRecord]) -> Result<(), String> {
    recover_pending_assignment_cleanup(&managed_agents_base_dir(app)?, |pending_pubkey| {
        records
            .iter()
            .any(|record| record.pubkey.eq_ignore_ascii_case(pending_pubkey))
    })
}

#[tauri::command]
pub fn get_bestie_assignment(
    expected_relay_url: Option<String>,
    expected_signer_pubkey: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<BestieAssignment>, String> {
    let scope = active_retention_scope(&app, &state)?;
    assert_expected_scope(
        &scope,
        expected_relay_url.as_deref(),
        expected_signer_pubkey.as_deref(),
    )?;
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let records = load_managed_agents(&app)?;
    recover_pending_cleanup(&app, &records)?;
    let conn = open_retention_db(&scope.db_path)?;
    get_assignment(&conn)
}

#[tauri::command]
pub fn assign_bestie(
    agent_pubkey: String,
    expected_relay_url: Option<String>,
    expected_signer_pubkey: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BestieAssignment, String> {
    let pubkey = validate_agent_pubkey(&agent_pubkey)?;
    let scope = active_retention_scope(&app, &state)?;
    assert_expected_scope(
        &scope,
        expected_relay_url.as_deref(),
        expected_signer_pubkey.as_deref(),
    )?;
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let records = load_managed_agents(&app)?;
    recover_pending_cleanup(&app, &records)?;
    require_eligible_local_agent(&records, &pubkey)?;
    let mut conn = open_retention_db(&scope.db_path)?;
    replace_assignment(&mut conn, &pubkey)
}

#[tauri::command]
pub fn clear_bestie_assignment(
    expected_relay_url: Option<String>,
    expected_signer_pubkey: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let scope = active_retention_scope(&app, &state)?;
    assert_expected_scope(
        &scope,
        expected_relay_url.as_deref(),
        expected_signer_pubkey.as_deref(),
    )?;
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let records = load_managed_agents(&app)?;
    recover_pending_cleanup(&app, &records)?;
    let mut conn = open_retention_db(&scope.db_path)?;
    clear_assignment(&mut conn)
}

#[tauri::command]
pub async fn resolve_bestie_conversation(
    expected_relay_url: Option<String>,
    expected_signer_pubkey: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ChannelInfo, String> {
    let generation = state.workspace_apply_generation.load(Ordering::Acquire);
    let scope = active_retention_scope(&app, &state)?;
    assert_expected_scope(
        &scope,
        expected_relay_url.as_deref(),
        expected_signer_pubkey.as_deref(),
    )?;
    let assignment = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(&app)?;
        recover_pending_cleanup(&app, &records)?;
        let conn = open_retention_db(&scope.db_path)?;
        let assignment = get_assignment(&conn)?
            .ok_or_else(|| "choose an agent before opening Bestie".to_string())?;
        require_eligible_local_agent(&records, &assignment.agent_pubkey)?;
        assignment
    };

    let owner_pubkey = scope.owner_keys.public_key().to_hex();
    let channel = super::dms::open_dm_with_scope(
        vec![assignment.agent_pubkey.clone()],
        Some(&scope.relay_url),
        Some(&owner_pubkey),
        &state,
    )
    .await?;

    if state.workspace_apply_generation.load(Ordering::Acquire) != generation {
        return Err("active workspace changed while resolving Bestie".to_string());
    }
    let current_scope = active_retention_scope(&app, &state)?;
    assert_expected_scope(&current_scope, Some(&scope.relay_url), Some(&owner_pubkey))?;
    let conn = open_retention_db(&scope.db_path)?;
    if !assignment_matches(&conn, &assignment.agent_pubkey)? {
        return Err("Bestie assignment changed while opening the conversation".to_string());
    }
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_scope_accepts_runtime_equivalences() {
        assert_eq!(
            canonical_relay(" WSS://LOCALHOST:443/ ")
                .unwrap_or_else(|error| panic!("canonical relay: {error}")),
            "wss://127.0.0.1"
        );
    }

    #[test]
    fn pubkeys_are_normalized_and_validated() {
        assert_eq!(
            validate_agent_pubkey(&"A".repeat(64))
                .unwrap_or_else(|error| panic!("valid pubkey: {error}")),
            "a".repeat(64)
        );
        assert!(validate_agent_pubkey("short").is_err());
    }
}
