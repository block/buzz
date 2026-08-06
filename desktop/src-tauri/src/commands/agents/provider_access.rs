//! Upgrade reconciliation for provider-backed managed-agent access.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        find_managed_agent_mut, load_managed_agents, save_managed_agents, BackendKind,
        ManagedAgentRecord,
    },
    util::now_iso,
};

pub(super) fn needs_reconciliation_with_policy(
    record: &ManagedAgentRecord,
    owner_only_access: bool,
) -> bool {
    owner_only_access && record.backend != BackendKind::Local && record.backend_agent_id.is_some()
}

#[derive(Debug)]
struct ProviderAccessTarget {
    pubkey: String,
    provider_id: String,
    config: serde_json::Value,
    cached_binary_path: Option<String>,
    agent_json: Result<serde_json::Value, String>,
}

fn collect_targets_with(
    records: Vec<ManagedAgentRecord>,
    owner_only_access: bool,
    mut build_payload: impl FnMut(&ManagedAgentRecord) -> Result<serde_json::Value, String>,
) -> Vec<ProviderAccessTarget> {
    records
        .into_iter()
        .filter(|record| needs_reconciliation_with_policy(record, owner_only_access))
        .map(|record| match record.backend.clone() {
            BackendKind::Provider { id, config } => ProviderAccessTarget {
                agent_json: build_payload(&record),
                pubkey: record.pubkey,
                provider_id: id,
                config,
                cached_binary_path: record.provider_binary_path,
            },
            BackendKind::Local => {
                unreachable!("provider access reconciliation selected a local agent")
            }
        })
        .collect()
}

/// Redeploy every existing provider agent in an owner-only access build.
///
/// The saved `backend_agent_id` only proves that some provider deployment
/// exists. A marked build sends the current owner-only payload before each
/// community UI load. Workspace apply fails closed if any provider rejects it.
pub(crate) async fn reconcile_on_workspace_apply(
    app: &AppHandle,
    state: &AppState,
) -> Result<(), String> {
    if !crate::managed_agents::owner_only_access_build() {
        return Ok(());
    }

    let targets = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        collect_targets_with(load_managed_agents(app)?, true, |record| {
            super::build_deploy_payload(app, state, record)
        })
    };

    for target in targets {
        let ProviderAccessTarget {
            pubkey,
            provider_id,
            config,
            cached_binary_path,
            agent_json,
        } = target;
        let agent_json = match agent_json {
            Ok(agent_json) => agent_json,
            Err(error) => {
                persist_failure(app, state, &pubkey, &error)?;
                return Err(format!(
                    "provider access reconciliation failed for agent {pubkey}: {error}"
                ));
            }
        };
        if let Err(error) = super::deploy_to_provider(
            app,
            state,
            &pubkey,
            &provider_id,
            &config,
            agent_json,
            cached_binary_path.as_deref(),
        )
        .await
        {
            return Err(format!(
                "provider access reconciliation failed for agent {pubkey}: {error}"
            ));
        }
    }

    Ok(())
}

fn persist_failure(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    error: &str,
) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|lock_error| lock_error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.last_error = Some(error.to_string());
    record.updated_at = now_iso();
    save_managed_agents(app, &records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(backend: BackendKind, backend_agent_id: Option<&str>) -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "agent", "name": "Agent", "relay_url": "", "acp_command": "",
            "agent_command": "", "agent_args": [], "mcp_command": "",
            "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
            "updated_at": "", "last_started_at": null, "last_stopped_at": null,
            "last_exit_code": null, "last_error": null
        }))
        .unwrap();
        record.backend = backend;
        record.backend_agent_id = backend_agent_id.map(str::to_string);
        record
    }

    #[test]
    fn upgrade_collects_existing_provider_and_builds_projected_payload() {
        let records = vec![
            record(
                BackendKind::Provider {
                    id: "provider".into(),
                    config: serde_json::json!({"region": "test"}),
                },
                Some("existing"),
            ),
            record(
                BackendKind::Provider {
                    id: "not-deployed".into(),
                    config: serde_json::json!({}),
                },
                None,
            ),
            record(BackendKind::Local, Some("stale")),
        ];

        let targets = collect_targets_with(records, true, |_| {
            Ok(serde_json::json!({"respond_to": "owner-only"}))
        });

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].pubkey, "agent");
        assert_eq!(targets[0].provider_id, "provider");
        assert_eq!(targets[0].config["region"], "test");
        assert_eq!(
            targets[0].agent_json.as_ref().unwrap()["respond_to"],
            "owner-only"
        );
    }

    #[test]
    fn unmarked_build_collects_no_upgrade_targets() {
        let records = vec![record(
            BackendKind::Provider {
                id: "provider".into(),
                config: serde_json::json!({}),
            },
            Some("existing"),
        )];

        assert!(
            collect_targets_with(records, false, |_| { Ok(serde_json::Value::Null) }).is_empty()
        );
    }
}
