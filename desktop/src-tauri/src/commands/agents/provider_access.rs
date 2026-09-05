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
    (owner_only_access || record.provider_policy_pending)
        && record.backend != BackendKind::Local
        && record.backend_agent_id.is_some()
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

/// Redeploy existing provider agents whose access policy requires enforcement.
///
/// Owner-only builds refresh every existing deployment before each community UI
/// load. All builds also retry records whose saved policy has not yet been
/// acknowledged by a successful provider deployment. Workspace apply fails
/// closed if any selected provider rejects the current policy.
pub(crate) async fn reconcile_on_workspace_apply(
    app: &AppHandle,
    state: &AppState,
) -> Result<(), String> {
    let owner_only_access = crate::managed_agents::owner_only_access_build();
    let targets = {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        // Resolve each disk row through the relay-primary overlay before
        // selecting targets: the redeploy below is a final-use boundary, so
        // both the selection predicate (backend, backend_agent_id, pending
        // flag) and the payload handed to the provider must read the
        // authoritative config, not raw disk bytes a newer relay head has
        // superseded.
        let records = load_managed_agents(app)?
            .iter()
            .map(|record| {
                crate::managed_agents::private_config_overlay::resolved_local_record(state, record)
            })
            .collect::<Result<Vec<_>, _>>()?;
        collect_targets_with(records, owner_only_access, |record| {
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
            None,
            None,
            None,
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

pub(crate) fn persist_failure(
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
    fn unmarked_build_collects_only_pending_targets() {
        let mut pending = record(
            BackendKind::Provider {
                id: "pending-provider".into(),
                config: serde_json::json!({}),
            },
            Some("existing-pending"),
        );
        pending.pubkey = "pending-agent".into();
        pending.provider_policy_pending = true;
        let ordinary = record(
            BackendKind::Provider {
                id: "ordinary-provider".into(),
                config: serde_json::json!({}),
            },
            Some("existing-ordinary"),
        );

        let targets = collect_targets_with(vec![ordinary, pending], false, |record| {
            Ok(serde_json::json!({"pubkey": record.pubkey}))
        });

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].pubkey, "pending-agent");
        assert_eq!(targets[0].provider_id, "pending-provider");
        assert_eq!(
            targets[0].agent_json.as_ref().unwrap()["pubkey"],
            "pending-agent"
        );
    }

    #[test]
    fn pending_policy_requires_an_existing_provider_deployment() {
        let mut undeployed = record(
            BackendKind::Provider {
                id: "provider".into(),
                config: serde_json::json!({}),
            },
            None,
        );
        undeployed.provider_policy_pending = true;
        let mut local = record(BackendKind::Local, Some("stale-provider-id"));
        local.provider_policy_pending = true;

        assert!(collect_targets_with(vec![undeployed, local], false, |_| {
            Ok(serde_json::Value::Null)
        })
        .is_empty());
    }

    // ── Workspace reconcile: relay-overlay resolve before target selection ──
    //
    // The production wiring (`reconcile_on_workspace_apply` resolving every
    // disk row through `resolved_local_record` before `collect_targets_with`)
    // needs a live `AppHandle`, so its presence is pinned by
    // `write_site_resolve_guard` in `private_config_overlay.rs`. This test
    // proves the fold itself: both the selection predicate and the redeploy
    // inputs read the resolved records, not raw disk rows.

    /// Carl round-9 P1 regression (stale-disk A / overlay B at workspace
    /// provider-access reconciliation): a relay head that migrated the
    /// backend must drive BOTH selection and the payload — the raw disk row's
    /// provider must neither be redeployed (head says local) nor keep stale
    /// policy inputs (head says a different provider).
    #[test]
    fn reconcile_selects_and_deploys_relay_resolved_records_not_raw_disk() {
        use crate::managed_agents::private_config_overlay::{
            test_relay_payload, PrivateConfigOverlay,
        };

        // Disk row A: provider on disk, but the relay head migrated it to
        // local — reconciliation must not select it at all.
        let migrated_pubkey = "aa".repeat(32);
        let mut migrated_disk = record(
            BackendKind::Provider {
                id: "stale-provider".into(),
                config: serde_json::json!({}),
            },
            Some("existing"),
        );
        migrated_disk.pubkey = migrated_pubkey.clone();

        // Disk row B: provider on disk AND on the relay head, but the head
        // carries newer policy inputs — the redeploy payload must read them.
        let repolicied_pubkey = "bb".repeat(32);
        let mut repolicied_disk = record(
            BackendKind::Provider {
                id: "stale-provider".into(),
                config: serde_json::json!({"region": "stale"}),
            },
            Some("existing"),
        );
        repolicied_disk.pubkey = repolicied_pubkey.clone();
        repolicied_disk.system_prompt = Some("stale disk prompt".into());

        let mut overlay = PrivateConfigOverlay::default();
        overlay
            .insert(test_relay_payload(&migrated_pubkey))
            .unwrap(); // backend: local
        let mut repolicied_head = test_relay_payload(&repolicied_pubkey);
        repolicied_head.config.backend = serde_json::json!({"type":"provider","id":"relay-provider","config":{"region":"relay"}});
        repolicied_head.config.backend_agent_id = Some("existing".into());
        overlay.insert(repolicied_head).unwrap();

        let resolved: Vec<_> = [&migrated_disk, &repolicied_disk]
            .into_iter()
            .map(|record| overlay.resolve_local_record(record))
            .collect();
        let targets = collect_targets_with(resolved, true, |record| {
            Ok(serde_json::json!({"system_prompt": record.system_prompt}))
        });

        assert_eq!(
            targets.len(),
            1,
            "the head-migrated-to-local row must not be selected"
        );
        assert_eq!(targets[0].pubkey, repolicied_pubkey);
        assert_eq!(targets[0].provider_id, "relay-provider");
        assert_eq!(targets[0].config["region"], "relay");
        assert_eq!(
            targets[0].agent_json.as_ref().unwrap()["system_prompt"],
            "relay prompt",
            "the redeploy payload must be built from the resolved record"
        );

        // NEGATIVE CONTROL: raw disk rows select the migrated agent and keep
        // the stale provider — proving the resolve, not the fixtures.
        let raw = collect_targets_with(vec![migrated_disk, repolicied_disk], true, |_| {
            Ok(serde_json::Value::Null)
        });
        assert_eq!(raw.len(), 2);
        assert!(raw.iter().all(|t| t.provider_id == "stale-provider"));
    }
}
