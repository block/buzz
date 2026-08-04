use std::path::Path;

use tauri::Manager;

use super::{canonical_dev_data_dir, patch_json_records};

pub(super) fn strip_legacy_agent_mcp_env_in_file(path: &Path) {
    patch_json_records(path, |obj| {
        let Some(serde_json::Value::Object(env_vars)) = obj.get_mut("env_vars") else {
            return false;
        };
        let stale_keys: Vec<String> = env_vars
            .keys()
            .filter(|key| crate::managed_agents::is_legacy_agent_mcp_env_key(key))
            .cloned()
            .collect();
        for key in &stale_keys {
            env_vars.remove(key);
            eprintln!(
                "buzz-desktop: project-mcp-boundary: removed legacy agent-owned env var {key:?}"
            );
        }
        !stale_keys.is_empty()
    });
}

/// Remove the retired per-agent MCP profile and generated credential values
/// before any managed-agent record is restored or deployed. Project
/// connections own this state now; the runtime filter remains defense in depth
/// for records that cannot be rewritten on this boot.
pub(super) fn strip_legacy_agent_mcp_env(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }
    for dir in dirs {
        let path = dir.join("agents/managed-agents.json");
        if path.exists() {
            strip_legacy_agent_mcp_env_in_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::test_support::{read_agents_json, write_agents_json};

    #[test]
    fn removes_profile_and_generated_credentials() {
        let dir = tempfile::tempdir().unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([{
                "name": "Cloud Agent",
                "env_vars": {
                    "BUZZ_ACP_MCP_SERVERS": "profile",
                    "BUZZ_MCP_CRM_AUTH_HEADER": "secret",
                    "buzz_mcp_case_variant": "secret-two",
                    "OPENAI_COMPAT_API_KEY": "keep"
                }
            }]),
        );
        let path = dir.path().join("agents/managed-agents.json");

        strip_legacy_agent_mcp_env_in_file(&path);

        let records = read_agents_json(dir.path());
        let env_vars = &records[0]["env_vars"];
        assert!(env_vars.get("BUZZ_ACP_MCP_SERVERS").is_none());
        assert!(env_vars.get("BUZZ_MCP_CRM_AUTH_HEADER").is_none());
        assert!(env_vars.get("buzz_mcp_case_variant").is_none());
        assert_eq!(env_vars["OPENAI_COMPAT_API_KEY"], "keep");
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([{
                "name": "Cloud Agent",
                "env_vars": {"BUZZ_MCP_CRM_TOKEN": "secret"}
            }]),
        );
        let path = dir.path().join("agents/managed-agents.json");

        strip_legacy_agent_mcp_env_in_file(&path);
        let after_first = std::fs::read_to_string(&path).unwrap();
        strip_legacy_agent_mcp_env_in_file(&path);

        assert_eq!(after_first, std::fs::read_to_string(&path).unwrap());
    }
}
