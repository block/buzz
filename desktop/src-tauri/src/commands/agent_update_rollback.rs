use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        load_managed_agents, save_managed_agents, try_regenerate_nest, ManagedAgentRecord,
    },
};

#[derive(Debug)]
pub(super) struct AgentUpdateRollback {
    attempted_record: ManagedAgentRecord,
    previous_record: ManagedAgentRecord,
}

impl AgentUpdateRollback {
    pub(super) fn new(previous_record: ManagedAgentRecord, attempted: &ManagedAgentRecord) -> Self {
        Self {
            attempted_record: attempted.clone(),
            previous_record,
        }
    }
}

fn copy_runtime_state(from: &ManagedAgentRecord, to: &mut ManagedAgentRecord) {
    to.runtime_pid = from.runtime_pid;
    to.backend = from.backend.clone();
    to.backend_agent_id.clone_from(&from.backend_agent_id);
    to.provider_binary_path
        .clone_from(&from.provider_binary_path);
    to.last_started_at.clone_from(&from.last_started_at);
    to.last_stopped_at.clone_from(&from.last_stopped_at);
    to.last_exit_code = from.last_exit_code;
    to.last_error.clone_from(&from.last_error);
    to.last_error_code = from.last_error_code;
}

fn same_configuration(left: &ManagedAgentRecord, right: &ManagedAgentRecord) -> bool {
    let mut normalized_left = left.clone();
    copy_runtime_state(right, &mut normalized_left);
    normalized_left.updated_at.clone_from(&right.updated_at);
    normalized_left == *right
}

fn restore_agent_update(
    records: &mut [ManagedAgentRecord],
    pubkey: &str,
    rollback: AgentUpdateRollback,
) -> Result<(), String> {
    let current = records
        .iter_mut()
        .find(|record| record.pubkey == pubkey)
        .ok_or_else(|| format!("agent {pubkey} not found while rolling back failed rename"))?;

    if !same_configuration(current, &rollback.attempted_record) {
        return Err(format!(
            "agent {pubkey} changed again before the failed rename could be rolled back"
        ));
    }

    let runtime_changed = {
        let mut attempted_with_current_runtime = rollback.attempted_record.clone();
        copy_runtime_state(current, &mut attempted_with_current_runtime);
        attempted_with_current_runtime != rollback.attempted_record
    };
    let mut restored = rollback.previous_record;
    copy_runtime_state(current, &mut restored);
    if runtime_changed {
        restored.updated_at.clone_from(&current.updated_at);
    }
    *current = restored;
    Ok(())
}

pub(super) fn rollback_failed_agent_update(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    rollback: AgentUpdateRollback,
) -> Result<(), String> {
    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(app)?;
        restore_agent_update(&mut records, pubkey, rollback)?;
        save_managed_agents(app, &records)?;
        let restored = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found after failed rename rollback"))?;
        super::agents::retain_managed_agent_pending(app, state, restored);
    }
    try_regenerate_nest(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, updated_at: &str) -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_str(
            r#"{
                "pubkey": "abcd1234",
                "name": "test-agent",
                "private_key_nsec": "nsec1fake",
                "relay_url": "wss://localhost:3000",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "model": null,
                "provider": null,
                "env_vars": {},
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_started_at": null,
                "last_stopped_at": null,
                "last_exit_code": null,
                "last_error": null
            }"#,
        )
        .expect("sample managed agent record");
        record.name = name.to_string();
        record.updated_at = updated_at.to_string();
        record
    }

    #[test]
    fn failed_profile_sync_restores_the_entire_agent_update() {
        let previous = record("Old name", "before");
        let mut attempted = previous.clone();
        attempted.name = "New name".to_string();
        attempted.model = Some("new-model".to_string());
        attempted.updated_at = "attempt".to_string();
        let rollback = AgentUpdateRollback::new(previous, &attempted);
        let mut records = vec![attempted];

        restore_agent_update(&mut records, "abcd1234", rollback)
            .expect("matching attempted update rolls back");

        assert_eq!(records[0].name, "Old name");
        assert_eq!(records[0].model, None);
        assert_eq!(records[0].updated_at, "before");
    }

    #[test]
    fn failed_profile_sync_does_not_overwrite_a_newer_agent_update() {
        let previous = record("Old name", "before");
        let mut attempted = previous.clone();
        attempted.name = "New name".to_string();
        attempted.updated_at = "attempt".to_string();
        let rollback = AgentUpdateRollback::new(previous, &attempted);
        let mut newer = attempted;
        newer.name = "Newest name".to_string();
        newer.updated_at = "newer".to_string();
        let mut records = vec![newer];

        let error = restore_agent_update(&mut records, "abcd1234", rollback)
            .expect_err("a concurrent update must not be overwritten");

        assert!(error.contains("changed again"));
        assert_eq!(records[0].name, "Newest name");
        assert_eq!(records[0].updated_at, "newer");
    }

    #[test]
    fn failed_profile_sync_preserves_runtime_churn_while_restoring_configuration() {
        let previous = record("Old name", "before");
        let mut attempted = previous.clone();
        attempted.name = "New name".to_string();
        attempted.model = Some("new-model".to_string());
        attempted.updated_at = "attempt".to_string();
        let rollback = AgentUpdateRollback::new(previous, &attempted);
        let mut churned = attempted;
        churned.runtime_pid = None;
        churned.last_stopped_at = Some("stopped".to_string());
        churned.last_exit_code = Some(1);
        churned.last_error = Some("harness exited".to_string());
        churned.updated_at = "runtime-change".to_string();
        let mut records = vec![churned];

        restore_agent_update(&mut records, "abcd1234", rollback)
            .expect("runtime-only churn must not prevent rollback");

        assert_eq!(records[0].name, "Old name");
        assert_eq!(records[0].model, None);
        assert_eq!(records[0].last_stopped_at.as_deref(), Some("stopped"));
        assert_eq!(records[0].last_exit_code, Some(1));
        assert_eq!(records[0].last_error.as_deref(), Some("harness exited"));
        assert_eq!(records[0].updated_at, "runtime-change");
    }
}
