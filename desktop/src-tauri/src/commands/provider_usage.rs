use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_FRAME_BYTES: usize = 256 * 1024;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ProviderUsageId {
    Codex,
    Claude,
    Grok,
}

impl ProviderUsageId {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "grok" => Some(Self::Grok),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Grok => "grok",
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
/// A provider shown in the provider-allowance experiment picker.
pub struct ProviderUsageCapability {
    id: &'static str,
    name: &'static str,
    availability: &'static str,
    detail: &'static str,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
/// One independently resetting personal-allowance window.
pub struct ProviderUsageWindow {
    id: String,
    label: String,
    used_percent: u64,
    remaining_percent: u64,
    resets_at: Option<i64>,
    duration_minutes: Option<u64>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
/// Optional provider totals that are not tied to one allowance window.
pub struct ProviderUsageTotals {
    credit_balance: Option<String>,
    reset_credits_available: Option<u64>,
    lifetime_tokens: Option<u64>,
    latest_daily_tokens: Option<u64>,
    latest_daily_date: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
/// A normalized local personal-allowance snapshot safe to expose over IPC.
pub struct ProviderUsageSnapshot {
    provider: &'static str,
    vendor: &'static str,
    product: &'static str,
    source: &'static str,
    plan_type: Option<String>,
    windows: Vec<ProviderUsageWindow>,
    totals: ProviderUsageTotals,
    fetched_at: u64,
}

#[tauri::command]
/// Lists supported and explicitly unsupported personal-allowance adapters.
pub async fn list_provider_usage_capabilities() -> Vec<ProviderUsageCapability> {
    let codex_availability = if crate::managed_agents::resolve_command("codex").is_some() {
        ("available", "Uses your existing local Codex sign-in")
    } else {
        ("not_installed", "Install and sign in to the Codex CLI")
    };

    vec![
        ProviderUsageCapability {
            id: ProviderUsageId::Codex.as_str(),
            name: "Codex",
            availability: codex_availability.0,
            detail: codex_availability.1,
        },
        ProviderUsageCapability {
            id: ProviderUsageId::Claude.as_str(),
            name: "Claude",
            availability: "unsupported",
            detail: "No supported standalone personal allowance reader yet",
        },
        ProviderUsageCapability {
            id: ProviderUsageId::Grok.as_str(),
            name: "Grok",
            availability: "unsupported",
            detail: "Consumer allowance is available in Grok Settings",
        },
    ]
}

#[tauri::command]
/// Reads a normalized personal-allowance snapshot for a selected provider.
pub async fn get_provider_usage(provider: String) -> Result<ProviderUsageSnapshot, String> {
    let provider =
        ProviderUsageId::parse(&provider).ok_or_else(|| "provider_usage_unknown".to_string())?;
    match provider {
        ProviderUsageId::Codex => {
            let codex_path = crate::managed_agents::resolve_command("codex")
                .ok_or_else(|| "codex_not_installed".to_string())?;
            tokio::task::spawn_blocking(move || read_codex_provider_usage(&codex_path))
                .await
                .map_err(|_| "codex_usage_task_failed".to_string())?
        }
        ProviderUsageId::Claude => Err("claude_usage_unsupported".to_string()),
        ProviderUsageId::Grok => Err("grok_usage_unsupported".to_string()),
    }
}

fn read_codex_provider_usage(
    codex_path: &std::path::Path,
) -> Result<ProviderUsageSnapshot, String> {
    let mut command = Command::new(codex_path);
    command
        .args(["app-server", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // App-server stderr can contain local configuration details. Only
        // stable error codes cross the command boundary.
        .stderr(Stdio::null());
    if let Some(workdir) = crate::managed_agents::default_agent_workdir() {
        command.current_dir(workdir);
    }
    if let Some(path) = crate::managed_agents::login_shell_path() {
        command.env("PATH", path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Keep launchers and native descendants together so every exit path
        // can reap the complete local app-server process tree.
        command.process_group(0);
    }
    crate::util::configure_no_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|_| "codex_app_server_start_failed".to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| finish_with_error(&mut child, "codex_app_server_stdin_unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| finish_with_error(&mut child, "codex_app_server_stdout_unavailable"))?;

    write_message(&mut child, &mut stdin, &initialize_request())?;

    let (sender, receiver) = mpsc::channel();
    let mut reader = Some(std::thread::spawn(move || {
        read_bounded_jsonl(stdout, sender)
    }));
    let deadline = Instant::now() + RESPONSE_TIMEOUT;

    if let Err(code) = wait_for_response(&receiver, deadline, 1) {
        stop_child(&mut child);
        join_reader(&mut reader);
        return Err(code);
    }

    let account_requests = post_initialize_requests();
    for message in &account_requests[..2] {
        if let Err(code) = write_message(&mut child, &mut stdin, message) {
            join_reader(&mut reader);
            return Err(code);
        }
    }

    let rate_limits = match wait_for_response(&receiver, deadline, 2) {
        Ok(result) => result,
        Err(code) => {
            stop_child(&mut child);
            join_reader(&mut reader);
            return Err(code);
        }
    };
    // Token totals are supplemental. Older app-server versions and some
    // accounts expose rate limits without account/usage/read; keep the valid
    // allowance snapshot and leave optional totals empty in that case.
    let token_usage = if write_message(&mut child, &mut stdin, &account_requests[2]).is_ok() {
        wait_for_optional_response(&receiver, deadline, 3)
    } else {
        json!({})
    };

    stop_child(&mut child);
    join_reader(&mut reader);
    normalize_usage(&rate_limits, &token_usage, unix_timestamp())
}

fn join_reader(reader: &mut Option<std::thread::JoinHandle<()>>) {
    if let Some(reader) = reader.take() {
        let _ = reader.join();
    }
}

fn write_message(child: &mut Child, stdin: &mut impl Write, message: &Value) -> Result<(), String> {
    writeln!(stdin, "{message}")
        .and_then(|_| stdin.flush())
        .map_err(|_| finish_with_error(child, "codex_app_server_write_failed"))
}

fn initialize_request() -> Value {
    json!({
        "method": "initialize",
        "id": 1,
        "params": {
            "clientInfo": {
                "name": "buzz_desktop",
                "title": "Buzz Desktop",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "optOutNotificationMethods": [
                    "thread/started",
                    "item/agentMessage/delta"
                ]
            }
        }
    })
}

fn post_initialize_requests() -> [Value; 3] {
    [
        json!({"method": "initialized", "params": {}}),
        json!({"method": "account/rateLimits/read", "id": 2, "params": null}),
        json!({"method": "account/usage/read", "id": 3, "params": null}),
    ]
}

fn read_bounded_jsonl(stdout: impl Read, sender: mpsc::Sender<Result<String, String>>) {
    let mut reader = BufReader::new(stdout);
    let mut total = 0_usize;
    let mut frame = Vec::new();
    loop {
        let buffer = match reader.fill_buf() {
            Ok(buffer) => buffer,
            Err(_) => {
                let _ = sender.send(Err("codex_app_server_read_failed".to_string()));
                break;
            }
        };
        if buffer.is_empty() {
            if !frame.is_empty() {
                let _ = sender.send(Err("codex_app_server_read_failed".to_string()));
            }
            break;
        }

        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let bytes = newline.map_or(buffer.len(), |position| position + 1);
        total = total.saturating_add(bytes);
        if frame.len().saturating_add(bytes) > MAX_FRAME_BYTES || total > MAX_TOTAL_BYTES {
            let _ = sender.send(Err("codex_usage_response_too_large".to_string()));
            break;
        }
        frame.extend_from_slice(&buffer[..bytes]);
        reader.consume(bytes);

        if newline.is_none() {
            continue;
        }
        while matches!(frame.last(), Some(b'\n' | b'\r')) {
            frame.pop();
        }
        let completed = std::mem::take(&mut frame);
        match String::from_utf8(completed) {
            Ok(line) => {
                if sender.send(Ok(line)).is_err() {
                    break;
                }
            }
            Err(_) => {
                let _ = sender.send(Err("codex_app_server_read_failed".to_string()));
                break;
            }
        }
    }
}

fn wait_for_response(
    receiver: &mpsc::Receiver<Result<String, String>>,
    deadline: Instant,
    expected_id: u64,
) -> Result<Value, String> {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err("codex_usage_timeout".to_string());
        }
        match receiver.recv_timeout(deadline.saturating_duration_since(now)) {
            Ok(Ok(line)) => {
                let message = serde_json::from_str::<Value>(&line)
                    .map_err(|_| "codex_usage_invalid_response".to_string())?;
                if message.get("id").and_then(Value::as_u64) != Some(expected_id) {
                    continue;
                }
                if let Some(error_code) = response_error_code(&message) {
                    return Err(error_code);
                }
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| "codex_usage_invalid_response".to_string());
            }
            Ok(Err(code)) => return Err(code),
            Err(mpsc::RecvTimeoutError::Timeout) => return Err("codex_usage_timeout".to_string()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("codex_app_server_closed".to_string())
            }
        }
    }
}

fn wait_for_optional_response(
    receiver: &mpsc::Receiver<Result<String, String>>,
    deadline: Instant,
    expected_id: u64,
) -> Value {
    wait_for_response(receiver, deadline, expected_id).unwrap_or_else(|_| json!({}))
}

fn response_error_code(message: &Value) -> Option<String> {
    let error = message.get("error")?;
    let detail = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if detail.contains("auth") || detail.contains("login") {
        return Some("codex_not_authenticated".to_string());
    }
    if detail.contains("experimental") || detail.contains("method") {
        return Some("codex_usage_protocol_unsupported".to_string());
    }
    Some("codex_usage_unavailable".to_string())
}

fn normalize_usage(
    rate_limits_result: &Value,
    token_usage_result: &Value,
    fetched_at: u64,
) -> Result<ProviderUsageSnapshot, String> {
    let bucket_map = rate_limits_result
        .get("rateLimitsByLimitId")
        .and_then(Value::as_object);
    let snapshots: Vec<(&str, &Value)> = match bucket_map {
        Some(map) if !map.is_empty() => {
            map.iter().map(|(id, value)| (id.as_str(), value)).collect()
        }
        _ => vec![(
            "codex",
            rate_limits_result
                .get("rateLimits")
                .ok_or_else(|| "codex_usage_invalid_response".to_string())?,
        )],
    };

    let mut windows = Vec::new();
    for (fallback_id, snapshot) in &snapshots {
        let limit_id = snapshot
            .get("limitId")
            .and_then(Value::as_str)
            .unwrap_or(fallback_id);
        let limit_name = snapshot
            .get("limitName")
            .and_then(Value::as_str)
            .unwrap_or(limit_id);
        for (window_id, default_label) in [("primary", "Primary"), ("secondary", "Secondary")] {
            let Some(window) = snapshot.get(window_id).filter(|value| !value.is_null()) else {
                continue;
            };
            windows.push(normalize_window(
                limit_id,
                limit_name,
                window_id,
                default_label,
                window,
            )?);
        }
    }
    if windows.is_empty() {
        return Err("codex_usage_limit_unavailable".to_string());
    }

    let first_snapshot = snapshots.first().map(|(_, snapshot)| *snapshot);
    let latest_bucket = token_usage_result
        .get("dailyUsageBuckets")
        .and_then(Value::as_array)
        .and_then(|buckets| {
            buckets.iter().max_by_key(|bucket| {
                bucket
                    .get("startDate")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            })
        });

    Ok(ProviderUsageSnapshot {
        provider: ProviderUsageId::Codex.as_str(),
        vendor: "openai",
        product: "codex",
        source: "personalAllowance",
        plan_type: rate_limits_result
            .get("rateLimits")
            .and_then(|snapshot| snapshot.get("planType"))
            .and_then(Value::as_str)
            .or_else(|| {
                first_snapshot
                    .and_then(|snapshot| snapshot.get("planType"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned),
        windows,
        totals: ProviderUsageTotals {
            credit_balance: rate_limits_result
                .get("rateLimits")
                .and_then(|snapshot| snapshot.get("credits"))
                .and_then(|credits| credits.get("balance"))
                .and_then(Value::as_str)
                .or_else(|| {
                    first_snapshot
                        .and_then(|snapshot| snapshot.get("credits"))
                        .and_then(|credits| credits.get("balance"))
                        .and_then(Value::as_str)
                })
                .map(str::to_owned),
            reset_credits_available: rate_limits_result
                .get("rateLimitResetCredits")
                .and_then(|credits| credits.get("availableCount"))
                .and_then(Value::as_u64),
            lifetime_tokens: token_usage_result
                .get("summary")
                .and_then(|summary| summary.get("lifetimeTokens"))
                .and_then(Value::as_u64),
            latest_daily_tokens: latest_bucket
                .and_then(|bucket| bucket.get("tokens"))
                .and_then(Value::as_u64),
            latest_daily_date: latest_bucket
                .and_then(|bucket| bucket.get("startDate"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        fetched_at,
    })
}

fn normalize_window(
    limit_id: &str,
    limit_name: &str,
    window_id: &str,
    default_label: &str,
    window: &Value,
) -> Result<ProviderUsageWindow, String> {
    let used_percent = window
        .get("usedPercent")
        .and_then(Value::as_i64)
        .filter(|value| (0..=100).contains(value))
        .map(|value| value as u64)
        .ok_or_else(|| "codex_usage_invalid_response".to_string())?;
    let duration_minutes = window.get("windowDurationMins").and_then(Value::as_u64);
    let window_label = match duration_minutes {
        Some(300) => "5-hour".to_string(),
        Some(1_440) => "Daily".to_string(),
        Some(10_080) => "Weekly".to_string(),
        Some(minutes) if minutes % 1_440 == 0 => format!("{}-day", minutes / 1_440),
        Some(minutes) if minutes % 60 == 0 => format!("{}-hour", minutes / 60),
        _ => default_label.to_string(),
    };
    let label = if limit_name.eq_ignore_ascii_case("codex") {
        window_label
    } else {
        format!("{limit_name} · {window_label}")
    };
    Ok(ProviderUsageWindow {
        id: format!("{limit_id}:{window_id}"),
        label,
        used_percent,
        remaining_percent: 100 - used_percent,
        resets_at: window.get("resetsAt").and_then(Value::as_i64),
        duration_minutes,
    })
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn finish_with_error(child: &mut Child, code: &str) -> String {
    stop_child(child);
    code.to_string()
}

fn stop_child(child: &mut Child) {
    let _ = crate::managed_agents::terminate_process(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_before_account_reads_without_experimental_flag() {
        assert_eq!(
            initialize_request().get("id").and_then(Value::as_u64),
            Some(1)
        );
        assert!(initialize_request()
            .pointer("/params/capabilities/experimentalApi")
            .is_none());
        let requests = post_initialize_requests();
        assert_eq!(
            requests[0].get("method").and_then(Value::as_str),
            Some("initialized")
        );
        assert_eq!(requests[1].get("id").and_then(Value::as_u64), Some(2));
        assert_eq!(requests[2].get("id").and_then(Value::as_u64), Some(3));
    }

    #[test]
    fn normalizes_all_multi_bucket_windows() {
        let rate_limits = json!({
            "rateLimits": {
                "primary": {"usedPercent": 38},
                "credits": {"balance": "0"},
                "planType": "pro"
            },
            "rateLimitsByLimitId": {
                "codex": {
                    "limitId": "codex",
                    "limitName": "Codex",
                    "primary": {
                        "usedPercent": 38,
                        "windowDurationMins": 300,
                        "resetsAt": 1785258777
                    },
                    "secondary": {
                        "usedPercent": 52,
                        "windowDurationMins": 10080,
                        "resetsAt": 1785658777
                    }
                },
                "review": {
                    "limitId": "review",
                    "limitName": "Code review",
                    "primary": {"usedPercent": 12}
                }
            },
            "rateLimitResetCredits": {"availableCount": 3}
        });
        let tokens = json!({
            "summary": {"lifetimeTokens": 13597623776_u64},
            "dailyUsageBuckets": [
                {"startDate": "2026-07-23", "tokens": 373817016},
                {"startDate": "2026-07-24", "tokens": 61038450}
            ]
        });

        let usage = normalize_usage(&rate_limits, &tokens, 123).unwrap();
        assert_eq!(usage.provider, "codex");
        assert_eq!(usage.plan_type.as_deref(), Some("pro"));
        assert_eq!(usage.windows.len(), 3);
        assert_eq!(usage.windows[0].remaining_percent, 62);
        assert_eq!(usage.windows[1].remaining_percent, 48);
        assert_eq!(usage.windows[0].label, "5-hour");
        assert_eq!(usage.windows[1].label, "Weekly");
        assert_eq!(usage.windows[2].label, "Code review · Primary");
        assert_eq!(usage.totals.credit_balance.as_deref(), Some("0"));
        assert_eq!(usage.totals.reset_credits_available, Some(3));
        assert_eq!(usage.totals.lifetime_tokens, Some(13_597_623_776));
        assert_eq!(usage.totals.latest_daily_tokens, Some(61_038_450));
        assert_eq!(usage.fetched_at, 123);
    }

    #[test]
    fn keeps_rate_limits_when_optional_token_usage_fails() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(json!({
                "id": 3,
                "error": {
                    "code": -32601,
                    "message": "Method not found"
                }
            })
            .to_string()))
            .unwrap();
        let token_usage =
            wait_for_optional_response(&receiver, Instant::now() + Duration::from_secs(1), 3);
        let usage = normalize_usage(
            &json!({
                "rateLimits": {
                    "primary": {"usedPercent": 38},
                    "planType": "pro"
                }
            }),
            &token_usage,
            123,
        )
        .unwrap();

        assert_eq!(usage.windows[0].remaining_percent, 62);
        assert_eq!(usage.plan_type.as_deref(), Some("pro"));
        assert_eq!(usage.totals.lifetime_tokens, None);
        assert_eq!(usage.totals.latest_daily_tokens, None);
    }

    #[test]
    fn normalizes_map_only_rate_limits() {
        let usage = normalize_usage(
            &json!({
                "rateLimitsByLimitId": {
                    "codex": {
                        "limitId": "codex",
                        "limitName": "Codex",
                        "planType": "pro",
                        "primary": {
                            "usedPercent": 25,
                            "windowDurationMins": 300
                        }
                    }
                }
            }),
            &json!({}),
            123,
        )
        .unwrap();

        assert_eq!(usage.windows.len(), 1);
        assert_eq!(usage.windows[0].remaining_percent, 75);
        assert_eq!(usage.plan_type.as_deref(), Some("pro"));
    }

    #[test]
    fn rejects_out_of_range_percentage() {
        let rate_limits = json!({
            "rateLimits": {"primary": {"usedPercent": 140}}
        });
        assert_eq!(
            normalize_usage(&rate_limits, &json!({}), 1).unwrap_err(),
            "codex_usage_invalid_response"
        );
    }

    #[test]
    fn maps_auth_errors_without_forwarding_details() {
        let response = json!({
            "id": 2,
            "error": {
                "code": -32000,
                "message": "Login required for alice@example.com"
            }
        });
        assert_eq!(
            response_error_code(&response).as_deref(),
            Some("codex_not_authenticated")
        );
    }

    #[test]
    fn response_reader_correlates_ids_and_ignores_notifications() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(json!({"method": "account/updated"}).to_string()))
            .unwrap();
        sender
            .send(Ok(json!({"id": 2, "result": {"ok": true}}).to_string()))
            .unwrap();
        assert_eq!(
            wait_for_response(&receiver, Instant::now() + Duration::from_secs(1), 2).unwrap(),
            json!({"ok": true})
        );
    }

    #[test]
    fn response_reader_rejects_malformed_json() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Ok("{not-json".to_string())).unwrap();
        assert_eq!(
            wait_for_response(&receiver, Instant::now() + Duration::from_secs(1), 1).unwrap_err(),
            "codex_usage_invalid_response"
        );
    }

    #[test]
    fn bounded_reader_rejects_oversized_frame() {
        let oversized = vec![b'a'; MAX_FRAME_BYTES + 1];
        let (sender, receiver) = mpsc::channel();
        read_bounded_jsonl(oversized.as_slice(), sender);
        assert_eq!(
            receiver.recv().unwrap().unwrap_err(),
            "codex_usage_response_too_large"
        );
    }
}
