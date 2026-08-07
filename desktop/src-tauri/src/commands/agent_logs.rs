use std::collections::BTreeMap;
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        latest_managed_agent_log_path, load_managed_agents, read_log_tail, redact_env_values_in,
        redact_secrets_with, BackendKind, ManagedAgentLogResponse,
    },
};

/// Scrub the bounded log tail that `get_managed_agent_log` is about to hand
/// to the rapid-iteration UI.
///
/// Harnesses write diagnostics to a file that the desktop then tails at
/// read time; spawn-time stderr scrubbing cannot see lines the ACP/MCP child
/// writes directly, so we redo both passes here before returning any
/// bytes to JS:
///
/// 1. shape-based (`redact_secrets_with([])`) catches well-known token
///    prefixes (`nsec1`, `sprt_tok_`, `ghp_`, …) even when no env var
///    ever carried them — e.g. an installer echoing a remote URL.
/// 2. value-based (`redact_env_values_in`) walks every value on the
///    resolved `ManagedAgentRecord.env_vars` and replaces literal
///    occurrences in the tail. This is the only step that catches an
///    API key the user actually configured for *this* agent.
///
/// `BTreeMap` keeps ordering deterministic for tests; we never read it
/// back, the only thing we need is "every value, as a `&str` slice".
pub(crate) fn redact_managed_agent_log_tail(
    content: &str,
    env_vars: &BTreeMap<String, String>,
) -> String {
    let shape = redact_secrets_with(content, &[]);
    redact_env_values_in(&shape, env_vars)
}

#[tauri::command]
pub async fn get_managed_agent_log(
    pubkey: String,
    line_count: Option<u32>,
    app: AppHandle,
) -> Result<ManagedAgentLogResponse, String> {
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        if record.backend != BackendKind::Local {
            return Err("logs are not available for remote agents".to_string());
        }

        let log_path = latest_managed_agent_log_path(&app, &pubkey)?;
        let raw = read_log_tail(&log_path, line_count.unwrap_or(120) as usize)?;
        let content = redact_managed_agent_log_tail(&raw, &record.env_vars);
        let content = redact_secrets_with(
            &content,
            &[
                record.private_key_nsec.as_str(),
                record.auth_tag.as_deref().unwrap_or(""),
            ],
        );
        Ok(ManagedAgentLogResponse {
            content,
            log_path: log_path.display().to_string(),
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::redact_managed_agent_log_tail;

    /// Representative-shaped text only — no real credentials. A desktop
    /// bug that drops a literal env value, a Nostr secret, or a GitHub
    /// token into the UI must surface as a regression here.
    #[test]
    fn scrubs_literal_env_values_and_preserves_neutral_lines() {
        let mut env = BTreeMap::new();
        env.insert(
            "ANTHROPIC_API_KEY".to_string(),
            "unit-test-secret-value".to_string(),
        );
        env.insert("EMPTY".to_string(), String::new());

        let tail = "\
[ready] harness online
model=demo chat=local
auth=unit-test-secret-value failed; retrying
nonce=42 status=ok
";

        let redacted = redact_managed_agent_log_tail(tail, &env);

        // The secret the user configured for this agent must be gone.
        assert!(
            !redacted.contains("unit-test-secret-value"),
            "literal env value leaked through: {redacted}",
        );
        assert!(redacted.contains("[REDACTED]"), "{redacted}");
        // Surrounding context — and an unrelated short token — stays.
        assert!(redacted.contains("retrying"), "{redacted}");
        assert!(redacted.contains("nonce=42"), "{redacted}");
        // Empty env entries are not scrubbed (would replace empty string).
        assert!(!redacted.contains("[REDACTED][REDACTED]"), "{redacted}");
    }

    /// Shape-based scrubbing also runs, so a token that never passed
    /// through `ManagedAgentRecord.env_vars` (e.g. embedded in a remote
    /// URL an installer echoes) is still removed before the tail reaches
    /// the UI panel.
    #[test]
    fn scrubs_known_secret_shapes_even_when_env_map_is_empty() {
        let env = BTreeMap::new();
        // Representative shapes only — these are deliberately invalid
        // (wrong length / checksum) test fixtures, not real credentials.
        let tail = "\
[git] cloning https://ghp_NOTAREALTOKENxxxxxxxx@github.com/owner/repo now
[nostr] identity=nsec1NOTAREALSECRETxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
";

        let redacted = redact_managed_agent_log_tail(tail, &env);

        assert!(
            !redacted.contains("ghp_NOTAREALTOKEN"),
            "github token prefix leaked: {redacted}",
        );
        assert!(
            !redacted.contains("nsec1NOTAREALSECRET"),
            "nsec leaked: {redacted}",
        );
        assert!(redacted.contains("[REDACTED]"), "{redacted}");
        // Surrounding context survives — we only narrow the secret span.
        assert!(redacted.contains("cloning"), "{redacted}");
        assert!(redacted.contains(" now"), "{redacted}");
        assert!(redacted.contains("identity="), "{redacted}");
    }
}
