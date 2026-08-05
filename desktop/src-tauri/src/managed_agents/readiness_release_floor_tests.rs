use std::collections::BTreeMap;

use super::{agent_readiness, effective_agent_env_with_floor};
use crate::managed_agents::discovery::known_acp_runtime_exact;

#[test]
fn empty_persisted_config_is_ready_with_baked_databricks_release_defaults() {
    let record: crate::managed_agents::types::ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{
                "pubkey": "{}",
                "name": "fresh-starter",
                "relay_url": "",
                "acp_command": "buzz-acp",
                "agent_command": "buzz-agent",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }}"#,
        "aa".repeat(32)
    ))
    .unwrap();
    let baked = BTreeMap::from([
        ("BUZZ_AGENT_PROVIDER".into(), "databricks_v2".into()),
        ("DATABRICKS_HOST".into(), "https://dbc.example.com".into()),
        ("DATABRICKS_MODEL".into(), "goose-claude-4-6-sonnet".into()),
    ]);
    let runtime = known_acp_runtime_exact("buzz-agent");

    let effective =
        effective_agent_env_with_floor(&record, &[], runtime, &Default::default(), baked);

    assert!(agent_readiness(&effective).is_ready());
    assert!(record.provider.is_none());
    assert!(record.model.is_none());
    assert!(record.env_vars.is_empty());
}
