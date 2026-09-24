//! Host-bound collaboration env written at spawn. These keys are reserved so
//! persona/agent `env_vars` cannot self-elevate a Main role or inject an HMAC.

use std::process::Command;

use super::types::ManagedAgentRecord;

pub(crate) const COLLAB_ROLE_ENV: &str = "BUZZ_COLLAB_ROLE";
pub(crate) const COLLAB_PROJECT_IDS_ENV: &str = "BUZZ_COLLAB_PROJECT_IDS";
pub(crate) const COLLAB_HELPER_ROOT_ENV: &str = "BUZZ_COLLAB_HELPER_ROOT";
pub(crate) const COLLAB_AUTHORITY_ENV: &str = "BUZZ_COLLAB_AUTHORITY";

const STRIP_KEYS: &[&str] = &[
    COLLAB_ROLE_ENV,
    COLLAB_PROJECT_IDS_ENV,
    COLLAB_HELPER_ROOT_ENV,
    COLLAB_AUTHORITY_ENV,
    "BUZZ_COLLAB_MCP_COMMAND",
    "BUZZ_COLLAB_HMAC_KEY",
    "BUZZ_COLLAB_TURN_PATH",
    "BUZZ_COLLAB_AGENT_PRINCIPAL",
    "BUZZ_COLLAB_AGENT_AUTHORITY",
];

const ALLOWED_ROLES: &[&str] = &["main", "requirements", "solution", "ui", "recipient"];

pub(crate) fn normalize_collaboration_role(value: &str) -> Result<String, String> {
    let trimmed = value.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if ALLOWED_ROLES.contains(&trimmed.as_str()) {
        Ok(trimmed)
    } else {
        Err(format!(
            "collaboration role {value:?} is not one of main, requirements, solution, ui, recipient"
        ))
    }
}

/// Fail closed when a role is set without at least one valid project and helper root.
pub(crate) fn is_project_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    let first = bytes[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

pub(crate) fn normalize_managed_project_ids(ids: Vec<String>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for raw in ids {
        let id = raw.trim().to_string();
        if id.is_empty() {
            return Err("managed project id cannot be empty".into());
        }
        if !is_project_id(&id) {
            return Err(
                "managed project id must be 1–64 lowercase letters, digits, or hyphens".into(),
            );
        }
        if out.iter().any(|existing| existing == &id) {
            return Err("duplicate managed project id".into());
        }
        out.push(id);
    }
    Ok(out)
}

pub(crate) fn validate_collaboration_binding(record: &ManagedAgentRecord) -> Result<(), String> {
    let role = record.collaboration_role.trim();
    if role.is_empty() {
        return Ok(());
    }
    let _ = normalize_collaboration_role(role)?;
    let ids = normalize_managed_project_ids(record.managed_project_ids.clone())?;
    if ids.is_empty() {
        return Err("collaboration role requires at least one managed project id".into());
    }
    if record.collab_helper_root.trim().is_empty() {
        return Err("collaboration role requires collab_helper_root".into());
    }
    Ok(())
}

/// Strip inherited/user collab keys, then write Desktop-owned values.
/// Must run after the user-env loop so reserved keys cannot stick.
pub(crate) fn apply_collab_host_env(command: &mut Command, record: &ManagedAgentRecord) {
    for key in STRIP_KEYS {
        command.env_remove(key);
    }
    let role = record.collaboration_role.trim();
    if role.is_empty() {
        return;
    }
    command.env(COLLAB_ROLE_ENV, role);
    command.env(
        COLLAB_PROJECT_IDS_ENV,
        record.managed_project_ids.join(","),
    );
    command.env(
        COLLAB_HELPER_ROOT_ENV,
        record.collab_helper_root.trim(),
    );
    command.env(COLLAB_AUTHORITY_ENV, "buzz-desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": "p",
            "name": "n",
            "private_key_nsec": "nsec1fake",
            "relay_url": "ws://localhost:3000",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "created_at": "now",
            "updated_at": "now",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }))
        .expect("minimal managed agent record")
    }

    #[test]
    fn unset_role_is_valid_without_project() {
        assert!(validate_collaboration_binding(&record()).is_ok());
    }

    #[test]
    fn main_role_requires_projects_and_helper() {
        let mut record = record();
        record.collaboration_role = "main".into();
        assert!(validate_collaboration_binding(&record).is_err());
        record.managed_project_ids = vec!["demo".into()];
        assert!(validate_collaboration_binding(&record).is_err());
        record.collab_helper_root = "/tmp/helper".into();
        assert!(validate_collaboration_binding(&record).is_ok());
        record.managed_project_ids = vec!["a".into(), "b".into()];
        assert!(validate_collaboration_binding(&record).is_ok());
        record.managed_project_ids = vec!["demo".into(), "demo".into()];
        assert!(validate_collaboration_binding(&record).is_err());
        record.managed_project_ids = vec!["Just-Start".into()];
        assert!(validate_collaboration_binding(&record).is_err());
        record.managed_project_ids = Vec::new();
        assert!(validate_collaboration_binding(&record).is_err());
    }

    #[test]
    fn forged_role_name_is_rejected() {
        assert_eq!(normalize_collaboration_role("Main").unwrap(), "main");
        assert!(normalize_collaboration_role("admin").is_err());
    }
}

