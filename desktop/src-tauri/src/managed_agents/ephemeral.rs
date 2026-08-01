//! Ephemeral (temp) managed agents: owner grant, TTL, concurrent cap, parent destroy.
//!
//! Temps are local-only, definition-less workers an orchestrator agent can spawn
//! after the owner opts in. Desktop enforces grant, parent identity, cap, and
//! bounded TTL; the frontend must never be the only gate.

use chrono::{DateTime, Duration, Utc};

use super::types::ManagedAgentRecord;
use super::GlobalAgentConfig;

/// Default TTL when the spawner omits one (30 min).
pub const DEFAULT_TEMP_AGENT_TTL_SECS: u64 = 30 * 60;
/// Hard ceiling a spawner may request (4 hours).
pub const MAX_TEMP_AGENT_TTL_SECS: u64 = 4 * 60 * 60;
/// Max concurrent live temp agents per owner Desktop.
pub const MAX_CONCURRENT_TEMP_AGENTS: usize = 5;

/// Start-time guard for temp agents (grant + expiry).
pub fn assert_temp_can_start(
    pubkey: &str,
    is_ephemeral: bool,
    grant_on: bool,
    expires_at: Option<&str>,
) -> Result<(), String> {
    if !is_ephemeral {
        return Ok(());
    }
    if !grant_on {
        return Err(format!(
            "temporary agent {pubkey} cannot start while temp spawn grant is off"
        ));
    }
    let Some(expires_at) = expires_at else {
        return Err(format!("temporary agent {pubkey} has no expires_at"));
    };
    if is_expired(expires_at, Utc::now()) {
        return Err(format!("temporary agent {pubkey} has expired"));
    }
    Ok(())
}

/// Canonical temp fields revalidated under the store lock before insert.
pub struct LockedTempInsert {
    pub parent_agent_pubkey: String,
    pub expires_at: String,
    pub channel_id: String,
}

/// Re-check grant/name/cap/parent/channel/expiry under the store lock.
pub fn recheck_temp_before_insert(
    global: &GlobalAgentConfig,
    records: &[ManagedAgentRecord],
    name: &str,
    parent_agent_pubkey: Option<&str>,
    channel_id: Option<&str>,
    expires_at: Option<&str>,
    backend_is_local: bool,
    respond_to_is_allowlist: bool,
) -> Result<LockedTempInsert, String> {
    let parent_pk = parent_agent_pubkey
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_ascii_lowercase())
        .ok_or_else(|| "temporary agents require parent_agent_pubkey".to_string())?;
    assert_can_spawn_temp(global, records, &parent_pk, name)?;
    let channel = channel_id
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "temporary agents require channel_id".to_string())?;
    uuid::Uuid::parse_str(&channel).map_err(|_| format!("invalid channel UUID: {channel}"))?;
    let expires_raw = expires_at
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "temporary agents require expires_at".to_string())?;
    let expires_at = clamp_expires_at(expires_raw, Utc::now())?;
    if !backend_is_local {
        return Err("temporary agents must be local".to_string());
    }
    if !respond_to_is_allowlist {
        return Err("temporary agents require respondTo=allowlist".to_string());
    }
    Ok(LockedTempInsert {
        parent_agent_pubkey: parent_pk,
        expires_at,
        channel_id: channel,
    })
}

/// Pure helpers for temp-agent policy (unit-testable without AppHandle).
pub fn clamp_temp_ttl_secs(requested: Option<u64>) -> u64 {
    let raw = requested.unwrap_or(DEFAULT_TEMP_AGENT_TTL_SECS);
    raw.clamp(1, MAX_TEMP_AGENT_TTL_SECS)
}

pub fn expires_at_from_ttl_secs(ttl_secs: u64) -> String {
    (Utc::now() + Duration::seconds(ttl_secs as i64)).to_rfc3339()
}

/// Clamp a requested RFC3339 expiry into (now, now+MAX_TTL]. Rejects past times.
pub fn clamp_expires_at(expires_at: &str, now: DateTime<Utc>) -> Result<String, String> {
    let at = parse_expires_at(expires_at)?;
    if at <= now {
        return Err("expires_at is in the past".to_string());
    }
    let max = now + Duration::seconds(MAX_TEMP_AGENT_TTL_SECS as i64);
    Ok(if at > max {
        max.to_rfc3339()
    } else {
        at.to_rfc3339()
    })
}

pub fn parse_expires_at(expires_at: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(expires_at)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| format!("invalid expires_at: {e}"))
}

pub fn is_expired(expires_at: &str, now: DateTime<Utc>) -> bool {
    parse_expires_at(expires_at)
        .map(|at| at <= now)
        .unwrap_or(true)
}

pub fn count_live_temps(records: &[ManagedAgentRecord]) -> usize {
    records.iter().filter(|r| r.ephemeral).count()
}

pub fn find_parent<'a>(
    records: &'a [ManagedAgentRecord],
    parent_pubkey: &str,
) -> Result<&'a ManagedAgentRecord, String> {
    let normalized = parent_pubkey.to_ascii_lowercase();
    let parent = records
        .iter()
        .find(|r| r.pubkey.eq_ignore_ascii_case(&normalized))
        .ok_or_else(|| "parent agent is not a managed agent on this Desktop".to_string())?;
    if parent.ephemeral {
        return Err("temporary agents cannot spawn other temporary agents".to_string());
    }
    Ok(parent)
}

pub fn assert_can_spawn_temp(
    config: &GlobalAgentConfig,
    records: &[ManagedAgentRecord],
    parent_pubkey: &str,
    name: &str,
) -> Result<(), String> {
    if !config.allow_temp_agent_spawn {
        return Err(
            "temp agent spawn is disabled — enable it in Agents settings (owner opt-in)"
                .to_string(),
        );
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("temp agent name is required".to_string());
    }
    if records.iter().any(|r| r.name.eq_ignore_ascii_case(name)) {
        return Err(format!("an agent named '{name}' already exists"));
    }
    find_parent(records, parent_pubkey)?;
    if count_live_temps(records) >= MAX_CONCURRENT_TEMP_AGENTS {
        return Err(format!(
            "concurrent temp agent cap reached ({MAX_CONCURRENT_TEMP_AGENTS})"
        ));
    }
    Ok(())
}

pub fn assert_parent_can_destroy(
    records: &[ManagedAgentRecord],
    target_pubkey: &str,
    parent_pubkey: &str,
) -> Result<(), String> {
    let target = records
        .iter()
        .find(|r| r.pubkey.eq_ignore_ascii_case(target_pubkey))
        .ok_or_else(|| format!("agent {target_pubkey} not found"))?;
    if !target.ephemeral {
        return Err("destroy-temp only works on ephemeral agents".to_string());
    }
    let parent = target
        .parent_agent_pubkey
        .as_deref()
        .ok_or_else(|| "ephemeral agent has no parent".to_string())?;
    if !parent.eq_ignore_ascii_case(parent_pubkey) {
        return Err("only the parent agent can destroy this temp".to_string());
    }
    Ok(())
}

/// Pubkeys of ephemeral agents that should be deleted (expired or all when grant off).
pub fn temps_to_sweep(
    records: &[ManagedAgentRecord],
    kill_all: bool,
    now: DateTime<Utc>,
) -> Vec<String> {
    records
        .iter()
        .filter(|r| r.ephemeral)
        .filter(|r| {
            kill_all
                || r.expires_at
                    .as_deref()
                    .map(|e| is_expired(e, now))
                    .unwrap_or(true)
        })
        .map(|r| r.pubkey.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_defaults_and_clamps() {
        assert_eq!(clamp_temp_ttl_secs(None), DEFAULT_TEMP_AGENT_TTL_SECS);
        assert_eq!(clamp_temp_ttl_secs(Some(60)), 60);
        assert_eq!(
            clamp_temp_ttl_secs(Some(MAX_TEMP_AGENT_TTL_SECS + 1)),
            MAX_TEMP_AGENT_TTL_SECS
        );
        assert_eq!(clamp_temp_ttl_secs(Some(0)), 1);
    }

    #[test]
    fn grant_off_blocks_spawn() {
        let config = GlobalAgentConfig::default();
        let err =
            assert_can_spawn_temp(&config, &[], "aa".repeat(32).as_str(), "temp-1").unwrap_err();
        assert!(err.contains("disabled"));
    }

    #[test]
    fn parent_can_destroy_only_own_temp() {
        let parent_pk = "aa".repeat(32);
        let other_pk = "bb".repeat(32);
        let temp_pk = "cc".repeat(32);
        let mut record: ManagedAgentRecord = serde_json::from_str(
            r#"{
                "pubkey": "placeholder",
                "name": "temp-1",
                "relay_url": "",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }"#,
        )
        .unwrap();
        record.pubkey = temp_pk.clone();
        record.ephemeral = true;
        record.parent_agent_pubkey = Some(parent_pk.clone());
        let records = vec![record];

        assert!(assert_parent_can_destroy(&records, &temp_pk, &parent_pk).is_ok());
        assert!(assert_parent_can_destroy(&records, &temp_pk, &other_pk).is_err());
    }

    #[test]
    fn temp_cannot_spawn_temp() {
        let mut parent: ManagedAgentRecord = serde_json::from_str(
            r#"{
                "pubkey": "placeholder",
                "name": "parent-temp",
                "relay_url": "",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }"#,
        )
        .unwrap();
        let parent_pk = "aa".repeat(32);
        parent.pubkey = parent_pk.clone();
        parent.ephemeral = true;
        let config = GlobalAgentConfig {
            allow_temp_agent_spawn: true,
            ..Default::default()
        };
        let err = assert_can_spawn_temp(&config, &[parent], &parent_pk, "child").unwrap_err();
        assert!(err.contains("cannot spawn"));
    }

    #[test]
    fn expired_detects_past() {
        let past = (Utc::now() - Duration::minutes(5)).to_rfc3339();
        assert!(is_expired(&past, Utc::now()));
        let future = (Utc::now() + Duration::minutes(5)).to_rfc3339();
        assert!(!is_expired(&future, Utc::now()));
    }

    #[test]
    fn clamp_expires_at_caps_to_max() {
        let now = Utc::now();
        let far = (now + Duration::hours(48)).to_rfc3339();
        let clamped = clamp_expires_at(&far, now).unwrap();
        let at = parse_expires_at(&clamped).unwrap();
        let max = now + Duration::seconds(MAX_TEMP_AGENT_TTL_SECS as i64);
        assert!(at <= max + Duration::seconds(1));
    }

    #[test]
    fn clamp_expires_at_rejects_past() {
        let now = Utc::now();
        let past = (now - Duration::minutes(1)).to_rfc3339();
        assert!(clamp_expires_at(&past, now).is_err());
    }
}
