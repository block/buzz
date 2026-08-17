use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::{BackendKind, ManagedAgentRecord, DEFAULT_ACP_COMMAND};

pub(crate) const MIN_HEARTBEAT_PREFLIGHT_INTERVAL_SECONDS: u64 = 10;
pub(crate) const MAX_HEARTBEAT_PREFLIGHT_INTERVAL_SECONDS: u64 = 86_400;

/// Exact durable policy authority for one managed agent. This contains no
/// connector credentials; it only pins the owner-controlled policy file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatPreflightDesignation {
    /// Absolute path of the owner-controlled policy file.
    #[serde(alias = "policyFile")]
    pub policy_file: PathBuf,
    /// Lowercase SHA-256 of the exact policy bytes.
    #[serde(alias = "policySha256")]
    pub policy_sha256: String,
    /// Owner-selected positive cadence enforced by both Desktop and harness.
    #[serde(alias = "heartbeatIntervalSeconds")]
    pub heartbeat_interval_seconds: u64,
}

impl HeartbeatPreflightDesignation {
    /// Validate the pinned policy file and its exact target before save/spawn.
    /// The harness repeats these checks on every heartbeat.
    pub(crate) fn validate_for_agent(&self, agent_pubkey: &str) -> Result<(), String> {
        use sha2::{Digest, Sha256};

        if !self.policy_file.is_absolute() {
            return Err("heartbeat preflight policy file must be an absolute path".into());
        }
        if self.policy_file.to_str().is_none() {
            return Err("heartbeat preflight policy file must be valid UTF-8".into());
        }
        if self.policy_sha256.len() != 64
            || !self
                .policy_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(
                "heartbeat preflight policy sha256 must be exactly 64 lowercase hex characters"
                    .into(),
            );
        }
        if !(MIN_HEARTBEAT_PREFLIGHT_INTERVAL_SECONDS..=MAX_HEARTBEAT_PREFLIGHT_INTERVAL_SECONDS)
            .contains(&self.heartbeat_interval_seconds)
        {
            return Err(format!(
                "heartbeat preflight interval must be between {MIN_HEARTBEAT_PREFLIGHT_INTERVAL_SECONDS} and {MAX_HEARTBEAT_PREFLIGHT_INTERVAL_SECONDS} seconds"
            ));
        }
        let metadata = std::fs::symlink_metadata(&self.policy_file).map_err(|error| {
            format!(
                "heartbeat preflight policy {} is unavailable: {error}",
                self.policy_file.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(format!(
                "heartbeat preflight policy {} must be a regular non-symlink file",
                self.policy_file.display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o022 != 0 {
                return Err(format!(
                    "heartbeat preflight policy {} is group/world-writable",
                    self.policy_file.display()
                ));
            }
        }
        let bytes = std::fs::read(&self.policy_file).map_err(|error| {
            format!(
                "heartbeat preflight policy {} is unreadable: {error}",
                self.policy_file.display()
            )
        })?;
        if bytes.len() > 64 * 1024 {
            return Err("heartbeat preflight policy exceeds 64 KiB".into());
        }
        let actual = hex::encode(Sha256::digest(&bytes));
        if actual != self.policy_sha256 {
            return Err("heartbeat preflight policy does not match its pinned digest".into());
        }
        #[derive(Deserialize)]
        struct TargetSelector {
            target_agent_pubkey: String,
            heartbeat_interval_seconds: u64,
        }
        let selector: TargetSelector = serde_json::from_slice(&bytes)
            .map_err(|error| format!("heartbeat preflight policy is invalid JSON: {error}"))?;
        if selector.target_agent_pubkey != agent_pubkey {
            return Err("heartbeat preflight policy targets a different managed agent".into());
        }
        if selector.heartbeat_interval_seconds != self.heartbeat_interval_seconds {
            return Err("heartbeat preflight policy cadence does not match its designation".into());
        }
        Ok(())
    }
}

/// Validate the complete Desktop-owned designation boundary. A designated
/// record must use the local backend and the bundled `buzz-acp` harness; a
/// custom ACP command could ignore the required policy environment entirely.
pub(crate) fn validate_heartbeat_preflight_configuration(
    designation: Option<&HeartbeatPreflightDesignation>,
    backend: &BackendKind,
    acp_command: &str,
    agent_pubkey: &str,
) -> Result<(), String> {
    let Some(designation) = designation else {
        return Ok(());
    };
    if backend != &BackendKind::Local {
        return Err(
            "heartbeat-preflight-designated agents are local-only until remote providers implement an equivalent durable policy authority"
                .to_string(),
        );
    }
    if acp_command != DEFAULT_ACP_COMMAND {
        return Err(
            "heartbeat-preflight-designated agents must use the bundled buzz-acp harness"
                .to_string(),
        );
    }
    designation.validate_for_agent(agent_pubkey)
}

/// Apply ACP-command and designation patches as one security-sensitive unit.
/// Returns true when an existing process must be stopped before the updated
/// record is persisted, so no process using the prior gate can survive.
pub(crate) fn apply_heartbeat_preflight_update(
    record: &mut ManagedAgentRecord,
    acp_command_update: Option<String>,
    designation_update: Option<Option<HeartbeatPreflightDesignation>>,
) -> Result<bool, String> {
    let prospective_acp_command = acp_command_update
        .as_deref()
        .unwrap_or(record.acp_command.as_str());
    let prospective_designation = designation_update
        .as_ref()
        .map_or(record.heartbeat_preflight.as_ref(), Option::as_ref);
    validate_heartbeat_preflight_configuration(
        prospective_designation,
        &record.backend,
        prospective_acp_command,
        &record.pubkey,
    )?;

    let must_stop = requires_process_stop(
        record.heartbeat_preflight.as_ref(),
        designation_update.as_ref(),
        &record.acp_command,
        acp_command_update.as_deref(),
    );

    if let Some(acp_command) = acp_command_update {
        record.acp_command = acp_command;
    }
    if let Some(designation) = designation_update {
        record.heartbeat_preflight = designation;
    }
    Ok(must_stop)
}

fn requires_process_stop(
    current: Option<&HeartbeatPreflightDesignation>,
    update: Option<&Option<HeartbeatPreflightDesignation>>,
    current_acp_command: &str,
    acp_command_update: Option<&str>,
) -> bool {
    let prospective = update.map_or(current, Option::as_ref);
    update.is_some_and(|update| update.as_ref() != current)
        || (prospective.is_some()
            && acp_command_update.is_some_and(|command| command != current_acp_command))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_harness_is_rejected_for_designated_record() {
        let designation = HeartbeatPreflightDesignation {
            policy_file: "/owner/policy.json".into(),
            policy_sha256: "a".repeat(64),
            heartbeat_interval_seconds: 3_600,
        };
        let error = validate_heartbeat_preflight_configuration(
            Some(&designation),
            &BackendKind::Local,
            "custom-acp",
            &"b".repeat(64),
        )
        .expect_err("custom ACP must not bypass the heartbeat gate");
        assert!(error.contains("bundled buzz-acp"));
    }

    #[test]
    fn add_change_and_remove_each_require_old_process_shutdown() {
        let first = HeartbeatPreflightDesignation {
            policy_file: "/owner/first.json".into(),
            policy_sha256: "a".repeat(64),
            heartbeat_interval_seconds: 3_600,
        };
        let second = HeartbeatPreflightDesignation {
            policy_file: "/owner/second.json".into(),
            policy_sha256: "b".repeat(64),
            heartbeat_interval_seconds: 3_600,
        };
        assert!(requires_process_stop(
            None,
            Some(&Some(first.clone())),
            DEFAULT_ACP_COMMAND,
            None,
        ));
        assert!(requires_process_stop(
            Some(&first),
            Some(&Some(second)),
            DEFAULT_ACP_COMMAND,
            None,
        ));
        assert!(requires_process_stop(
            Some(&first),
            Some(&None),
            DEFAULT_ACP_COMMAND,
            None,
        ));
        assert!(!requires_process_stop(
            Some(&first),
            Some(&Some(first.clone())),
            DEFAULT_ACP_COMMAND,
            None,
        ));
    }
}
