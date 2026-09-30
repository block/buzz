//! Read OpenClaw Gateway session-store totals when ACP `usage_update` is missing.
//!
//! Community last-miles (`buzz-acp@captain` etc.) share a host with the Gateway.
//! OpenClaw only emits ACP `usage_update { used, size }` when
//! `buildSessionUsageSnapshot` sees `totalTokensFresh === true` *at emit time*
//! (`src/acp/translator.presentation.ts`). A persistence race can leave the
//! wire silent while `~/.openclaw/agents/<id>/sessions/sessions.json` already
//! holds fresh `totalTokens` — proven on dohula Captain (no usage_update; store
//! had totalTokens + totalTokensFresh).
//!
//! This module is a best-effort, sync filesystem lookup. It never talks to the
//! Gateway RPC (no private auth). Failures are silent; the caller keeps the
//! existing "no usage → no 44200" behavior.

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::parse_openclaw_gateway_agent_id;

/// Absolute used-token proxy for one Gateway session key, when the store entry
/// is fresh enough to trust (mirrors OpenClaw's ACP emit gate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenClawStoreUsage {
    pub used: u64,
    pub size: Option<u64>,
}

/// Resolve `agents/<id>/sessions/sessions.json` under the OpenClaw state dir.
pub(crate) fn resolve_sessions_json_path(agent_id: &str) -> PathBuf {
    resolve_openclaw_state_dir().join("agents").join(agent_id).join("sessions").join("sessions.json")
}

fn resolve_openclaw_state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("OPENCLAW_STATE_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    let home = std::env::var("OPENCLAW_HOME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".openclaw")
}

/// Look up fresh `totalTokens` for `session_key` in the agent's sessions.json.
///
/// Returns `None` when the agent id cannot be parsed, the file is missing /
/// unreadable, the key is absent, or the entry is not fresh (`totalTokensFresh`
/// must be explicitly `true`, matching OpenClaw's emit gate).
pub(crate) fn lookup_fresh_used_tokens(session_key: &str) -> Option<OpenClawStoreUsage> {
    let agent_id = parse_openclaw_gateway_agent_id(session_key)?;
    let path = resolve_sessions_json_path(&agent_id);
    lookup_fresh_used_tokens_in_store(&path, session_key)
}

pub(crate) fn lookup_fresh_used_tokens_in_store(
    store_path: &Path,
    session_key: &str,
) -> Option<OpenClawStoreUsage> {
    let raw = fs::read_to_string(store_path).ok()?;
    let root: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let entry = root.get(session_key)?;
    parse_fresh_usage_entry(entry)
}

fn parse_fresh_usage_entry(entry: &serde_json::Value) -> Option<OpenClawStoreUsage> {
    // OpenClaw only emits usage_update when totalTokensFresh === true.
    if entry.get("totalTokensFresh") != Some(&serde_json::Value::Bool(true)) {
        return None;
    }
    let total = json_nonneg_u64(entry.get("totalTokens"))?;
    let size = json_nonneg_u64(entry.get("contextTokens")).filter(|&n| n > 0);
    // Match OpenClaw's wire `used = min(totalTokens, contextTokens)` when size
    // is known; otherwise publish the raw total (still useful for 44200 chips).
    let used = match size {
        Some(size) => total.min(size),
        None => total,
    };
    Some(OpenClawStoreUsage { used, size })
}

fn json_nonneg_u64(v: Option<&serde_json::Value>) -> Option<u64> {
    let v = v?;
    if let Some(n) = v.as_u64() {
        return Some(n);
    }
    let f = v.as_f64()?;
    if f.is_finite() && f >= 0.0 {
        Some(f.floor() as u64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    fn write_store(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("sessions.json");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn lookup_reads_fresh_total_tokens_capped_to_context() {
        let dir = tempfile::tempdir().unwrap();
        let key = "agent:captain:buzz:ch:7cce05ee-cd4b-4f67-a2e4-58ac63f97dae";
        let path = write_store(
            dir.path(),
            &format!(
                r#"{{
                  "{key}": {{
                    "sessionId": "abc",
                    "updatedAt": 1,
                    "totalTokens": 131246,
                    "totalTokensFresh": true,
                    "contextTokens": 1000000
                  }}
                }}"#
            ),
        );
        let usage = lookup_fresh_used_tokens_in_store(&path, key).expect("fresh");
        assert_eq!(usage.used, 131246);
        assert_eq!(usage.size, Some(1_000_000));
    }

    #[test]
    fn lookup_rejects_stale_or_missing_fresh_flag() {
        let dir = tempfile::tempdir().unwrap();
        let key = "agent:mo:buzz:ch:deadbeef";
        let path = write_store(
            dir.path(),
            &format!(
                r#"{{
                  "{key}": {{
                    "totalTokens": 99,
                    "totalTokensFresh": false,
                    "contextTokens": 1000
                  }}
                }}"#
            ),
        );
        assert!(lookup_fresh_used_tokens_in_store(&path, key).is_none());
    }

    #[test]
    fn lookup_rejects_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_store(dir.path(), r#"{"agent:other:buzz":{"totalTokens":1,"totalTokensFresh":true}}"#);
        assert!(lookup_fresh_used_tokens_in_store(&path, "agent:captain:buzz").is_none());
    }

    #[test]
    fn resolve_path_joins_state_dir_agents_sessions() {
        let path = resolve_sessions_json_path("captain");
        assert!(path.ends_with(Path::new("agents/captain/sessions/sessions.json")));
    }
}
