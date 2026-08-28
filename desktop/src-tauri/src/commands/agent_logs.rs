use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    app_state::AppState,
    managed_agents::{
        build_managed_agent_summary, latest_managed_agent_log_path, load_global_agent_config,
        load_managed_agents, load_personas, managed_agent_log_path, managed_agent_runtime_log_path,
        read_log_tail, workspace_pair_key, BackendKind, ManagedAgentLogResponse,
    },
};

const USAGE_LOG_SAMPLE_LINES: usize = 20_000;

#[derive(Debug, Default, PartialEq, Eq)]
struct ParsedAgentUsage {
    prompt_count: u64,
    prompt_bytes: u64,
    peak_prompt_bytes: u64,
    session_start_count: u64,
    large_prompt_count: u64,
    retry_count: u64,
    quota_limit_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
/// Bounded local usage and effective runtime metadata for one managed agent.
///
/// Prompt measurements are derived from harness logs and are estimates rather
/// than provider billing data. No prompt or message content is serialized.
pub struct AgentUsageSummary {
    pubkey: String,
    name: String,
    model: Option<String>,
    parallelism: u32,
    is_running: bool,
    prompt_count: u64,
    prompt_bytes: u64,
    estimated_prompt_tokens: u64,
    peak_prompt_bytes: u64,
    session_start_count: u64,
    large_prompt_count: u64,
    retry_count: u64,
    quota_limit_count: u64,
}

fn parse_u64_field(line: &str, field: &str) -> Option<u64> {
    let prefix = format!("{field}=");
    line.split_whitespace()
        .find_map(|part| part.strip_prefix(&prefix))
        .and_then(|value| value.trim_end_matches([',', ';']).parse().ok())
}

fn parse_agent_usage(content: &str) -> ParsedAgentUsage {
    let mut usage = ParsedAgentUsage::default();

    for line in content.lines() {
        if line.contains("prompt prepared") && !line.contains("large prompt prepared") {
            if let Some(bytes) = parse_u64_field(line, "prompt_bytes") {
                usage.prompt_count += 1;
                usage.prompt_bytes = usage.prompt_bytes.saturating_add(bytes);
                usage.peak_prompt_bytes = usage.peak_prompt_bytes.max(bytes);
            }
            if line.contains("is_new_session=true") {
                usage.session_start_count += 1;
            }
        }
        if line.contains("large prompt prepared") {
            usage.large_prompt_count += 1;
        }
        if line.contains("requeueing failed batch with backoff")
            || line.contains("requeued for retry")
        {
            usage.retry_count += 1;
        }
        if line.contains("provider usage limit reached") {
            usage.quota_limit_count += 1;
        }
    }

    usage
}

fn select_usage_log_path(pair_path: Option<PathBuf>, legacy_path: PathBuf) -> PathBuf {
    pair_path
        .filter(|path| path.exists())
        .unwrap_or(legacy_path)
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
        Ok(ManagedAgentLogResponse {
            content: read_log_tail(&log_path, line_count.unwrap_or(120) as usize)?,
            log_path: log_path.display().to_string(),
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Return bounded, per-agent prompt-cost proxies from local harness logs.
///
/// ACP adapters do not all expose exact provider token counts. Prompt bytes
/// are therefore reported as a transparent input-size proxy, with the
/// conventional bytes/4 token estimate clearly labeled as an estimate in the
/// UI. The bounded tail avoids loading unbounded historical logs.
#[tauri::command]
pub async fn get_agent_usage_dashboard(app: AppHandle) -> Result<Vec<AgentUsageSummary>, String> {
    tokio::task::spawn_blocking(move || {
        let agent_contexts = {
            let state = app.state::<AppState>();
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let records = load_managed_agents(&app)?;
            let personas = load_personas(&app).unwrap_or_default();
            let global = load_global_agent_config(&app).unwrap_or_default();
            let runtimes = state
                .managed_agent_processes
                .lock()
                .map_err(|error| error.to_string())?;

            records
                .iter()
                .filter(|record| {
                    !record.pubkey.is_empty() && matches!(&record.backend, BackendKind::Local)
                })
                .map(|record| {
                    let summary =
                        build_managed_agent_summary(&app, record, &runtimes, &personas, &global)?;
                    let legacy_log_path = managed_agent_log_path(&app, &record.pubkey)?;
                    let pair_log_path = workspace_pair_key(&app, record)
                        .and_then(|key| managed_agent_runtime_log_path(&app, &key).ok());
                    let log_path = select_usage_log_path(pair_log_path, legacy_log_path);
                    Ok((summary, log_path))
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        let mut summaries = Vec::new();

        for (summary, log_path) in agent_contexts {
            let content = read_log_tail(&log_path, USAGE_LOG_SAMPLE_LINES)?;
            let usage = parse_agent_usage(&content);

            summaries.push(AgentUsageSummary {
                pubkey: summary.pubkey,
                name: summary.name,
                model: summary.model,
                parallelism: summary.parallelism,
                is_running: summary.status == "running",
                prompt_count: usage.prompt_count,
                prompt_bytes: usage.prompt_bytes,
                estimated_prompt_tokens: usage.prompt_bytes.saturating_add(3) / 4,
                peak_prompt_bytes: usage.peak_prompt_bytes,
                session_start_count: usage.session_start_count,
                large_prompt_count: usage.large_prompt_count,
                retry_count: usage.retry_count,
                quota_limit_count: usage.quota_limit_count,
            });
        }

        summaries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(summaries)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_usage_summarizes_prompt_cost_and_failures() {
        let log = r#"
INFO pool::prompt: prompt prepared prompt_bytes=12000 prompt_blocks=5 is_new_session=true
INFO pool::prompt: prompt prepared prompt_bytes=800 prompt_blocks=2 is_new_session=false
WARN pool::prompt: large prompt prepared prompt_bytes=52000
WARN requeueing failed batch with backoff retry_count=1
ERROR dead-lettering batch immediately — provider usage limit reached
"#;

        assert_eq!(
            parse_agent_usage(log),
            ParsedAgentUsage {
                prompt_count: 2,
                prompt_bytes: 12_800,
                peak_prompt_bytes: 12_000,
                session_start_count: 1,
                large_prompt_count: 1,
                retry_count: 1,
                quota_limit_count: 1,
            }
        );
    }

    #[test]
    fn parse_agent_usage_ignores_malformed_prompt_measurements() {
        let usage = parse_agent_usage(
            "INFO prompt prepared prompt_bytes=unknown is_new_session=true\nINFO unrelated",
        );
        assert_eq!(usage.prompt_count, 0);
        assert_eq!(usage.prompt_bytes, 0);
        assert_eq!(usage.session_start_count, 1);
    }

    #[test]
    fn usage_log_prefers_existing_pair_scope_and_falls_back_to_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let pair = temp.path().join("pair.log");
        let legacy = temp.path().join("legacy.log");

        assert_eq!(
            select_usage_log_path(Some(pair.clone()), legacy.clone()),
            legacy
        );

        std::fs::write(&pair, "pair usage").unwrap();
        assert_eq!(select_usage_log_path(Some(pair.clone()), legacy), pair);
    }
}
