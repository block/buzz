//! Tiny fake MCP server for integration tests.
//!
//! Reads JSON-RPC line frames on stdin and replies on stdout. Driven by
//! environment variables so tests can simulate misbehavior:
//!
//!   FAKE_MCP_HANG_INIT=1     — never reply to `initialize` (init timeout)
//!   FAKE_MCP_HANG_TOOLS=1    — never reply to `tools/list` (list timeout)
//!   FAKE_MCP_TOOL_COUNT=N    — return N tools (default: 1)
//!   FAKE_MCP_HUGE_DESC=1     — every tool description is 100 KB
//!   FAKE_MCP_DESC_SIZE=N     — every tool description is N bytes (overrides HUGE_DESC)
//!   FAKE_MCP_TOOL_DELAY=N    — `tools/call` sleeps N seconds before replying
//!                              (use a large value, e.g. 999, to simulate hang)
//!   FAKE_MCP_RESULT_SIZE=N   — `tools/call` returns an N-byte text result
//!                              (default: the literal "ok"); grows history
//!   FAKE_MCP_PID_FILE=path   — write the child PID to `path` on startup
//!                              (for tests that want to verify the child died)
//!   FAKE_MCP_SPAWN_GRANDCHILD=1
//!                            — on `tools/call`, spawn a `sleep 999`
//!                              grandchild before hanging. Its PID is
//!                              written to FAKE_MCP_GRANDCHILD_PID_FILE
//!                              so a test can verify the entire process
//!                              tree dies on timeout.
//!   FAKE_MCP_GRANDCHILD_PID_FILE=path
//!                            — path to write the grandchild PID to.
//!   FAKE_MCP_STOP_HOOK=1     — expose a `_Stop` hook tool
//!   FAKE_MCP_STOP_TEXT=text  — `_Stop` returns this text (default: "keep going")
//!   FAKE_MCP_STOP_DELAY=N    — `_Stop` sleeps N seconds before replying
//!                              (use a large value to simulate hang/timeout)
//!   FAKE_MCP_STOP_COUNT=N    — `_Stop` returns STOP_TEXT for the first N
//!                              invocations; empty string thereafter. If
//!                              unset, every call returns STOP_TEXT.
//!   FAKE_MCP_POSTCOMPACT_HOOK=1
//!                            — expose a `_PostCompact` hook tool
//!   FAKE_MCP_POSTCOMPACT_TEXT=text
//!                            — `_PostCompact` returns this (default: "")

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn env_flag(k: &str) -> bool {
    std::env::var(k).map(|v| v != "0").unwrap_or(false)
}

fn env_usize(k: &str, default: usize) -> usize {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(k: &str, default: u64) -> u64 {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn write_response(id: Value, result: Value) {
    let msg = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    let mut s = serde_json::to_string(&msg).expect("serialize");
    s.push('\n');
    let mut out = std::io::stdout().lock();
    out.write_all(s.as_bytes()).expect("write");
    out.flush().expect("flush");
}

fn hang_forever() -> ! {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

fn make_tools(
    count: usize,
    desc: &str,
    include_stop_hook: bool,
    include_post_compact_hook: bool,
) -> Vec<Value> {
    let mut tools: Vec<Value> = (0..count)
        .map(|i| {
            json!({
                "name": format!("tool_{i}"),
                "description": desc,
                "inputSchema": { "type": "object", "properties": {} },
            })
        })
        .collect();
    if include_stop_hook {
        tools.push(json!({
            "name": "_Stop",
            "description": "stop hook",
            "inputSchema": { "type": "object", "properties": {} },
        }));
    }
    if include_post_compact_hook {
        tools.push(json!({
            "name": "_PostCompact",
            "description": "post compact hook",
            "inputSchema": { "type": "object", "properties": {} },
        }));
    }
    tools
}

fn main() {
    // Optional: write our own PID so a test can later check the process is gone.
    if let Ok(path) = std::env::var("FAKE_MCP_PID_FILE") {
        let pid = std::process::id().to_string();
        let _ = std::fs::write(&path, pid);
    }

    let hang_init = env_flag("FAKE_MCP_HANG_INIT");
    let hang_tools = env_flag("FAKE_MCP_HANG_TOOLS");
    let tool_count = env_usize("FAKE_MCP_TOOL_COUNT", 1);
    // FAKE_MCP_DESC_SIZE wins over FAKE_MCP_HUGE_DESC when set.
    let desc: String = if let Some(n) = std::env::var("FAKE_MCP_DESC_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        "x".repeat(n)
    } else if env_flag("FAKE_MCP_HUGE_DESC") {
        "x".repeat(100_000)
    } else {
        "fake tool".to_owned()
    };
    let tool_delay_secs = env_u64("FAKE_MCP_TOOL_DELAY", 0);
    // Tool-call result text size in bytes (default: the literal "ok"). Lets a
    // test grow session history by a controlled amount via a tool result.
    let result_size = env_u64("FAKE_MCP_RESULT_SIZE", 0) as usize;
    let stop_hook = env_flag("FAKE_MCP_STOP_HOOK");
    let stop_text = std::env::var("FAKE_MCP_STOP_TEXT").unwrap_or_else(|_| "keep going".to_owned());
    let stop_delay_secs = env_u64("FAKE_MCP_STOP_DELAY", 0);
    // 0 means "unset" → unlimited; any positive value caps the number of
    // calls that return STOP_TEXT before flipping to empty string.
    let stop_count_limit: usize = env_usize("FAKE_MCP_STOP_COUNT", usize::MAX);
    let mut stop_calls_seen: usize = 0;
    let post_compact_hook = env_flag("FAKE_MCP_POSTCOMPACT_HOOK");
    let post_compact_text = std::env::var("FAKE_MCP_POSTCOMPACT_TEXT").unwrap_or_default();

    // Use a channel-based stdin reader so notifications (which carry no id)
    // are captured even while the main thread is sleeping during a tool call.
    let cancel_log_path = std::env::var("FAKE_MCP_CANCEL_LOG").ok();
    let (tx, rx) = std::sync::mpsc::channel::<(Value, Option<Value>)>();
    let cancel_log_for_thread = cancel_log_path.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let lines = stdin.lock().lines();
        for line in lines {
            let line = match line {
                Ok(l) => l,
                Err(_) => return,
            };
            if line.trim().is_empty() {
                continue;
            }
            let msg: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
            let id = msg.get("id").cloned();
            // Notifications carry no id. Log cancellations if configured.
            if id.is_none() || id == Some(Value::Null) {
                if method == "notifications/cancelled" {
                    if let Some(ref path) = cancel_log_for_thread {
                        use std::io::Write as _;
                        if let Ok(mut f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                        {
                            let _ = writeln!(f, "{}", line.trim());
                        }
                    }
                }
                continue;
            }
            // Send requests (with id) to the main processing loop.
            let _ = tx.send((msg, id));
        }
    });

    while let Ok((msg, id_opt)) = rx.recv() {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = id_opt.unwrap_or(Value::Null);

        match method {
            "initialize" => {
                if hang_init {
                    hang_forever();
                }
                write_response(
                    id,
                    json!({
                        "protocolVersion": "2025-06-18",
                        "capabilities": { "tools": {} },
                        "serverInfo": { "name": "fake-mcp", "version": "0.0.0" },
                    }),
                );
            }
            "tools/list" => {
                if hang_tools {
                    hang_forever();
                }
                write_response(
                    id,
                    json!({
                        "tools": make_tools(tool_count, &desc, stop_hook, post_compact_hook)
                    }),
                );
            }
            "tools/call" => {
                // Signal that the request was received (for tests that
                // need to wait until the call is in-flight before cancelling).
                // Write the request id so tests can correlate with cancel.
                if let Ok(path) = std::env::var("FAKE_MCP_CALL_RECEIVED") {
                    let id_str = serde_json::to_string(&id).unwrap_or_else(|_| "?".into());
                    let _ = std::fs::write(&path, id_str);
                }
                let called_name = msg
                    .get("params")
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                // Optionally spawn a long-sleeping grandchild so the test
                // can verify process-group killing reaches the whole tree.
                if env_flag("FAKE_MCP_SPAWN_GRANDCHILD") {
                    let child = std::process::Command::new("sleep")
                        .arg("999")
                        .spawn()
                        .expect("spawn grandchild");
                    if let Ok(path) = std::env::var("FAKE_MCP_GRANDCHILD_PID_FILE") {
                        let _ = std::fs::write(&path, child.id().to_string());
                    }
                    std::mem::forget(child);
                }
                if called_name == "_Stop" {
                    if stop_delay_secs > 0 {
                        std::thread::sleep(std::time::Duration::from_secs(stop_delay_secs));
                    }
                    // Once we exceed the configured count, return empty
                    // text so the agent treats it as no objection. This
                    // lets a test exercise the "objected then cleared"
                    // path without relying on the rejection budget.
                    let payload = if stop_calls_seen < stop_count_limit {
                        stop_text.clone()
                    } else {
                        String::new()
                    };
                    stop_calls_seen = stop_calls_seen.saturating_add(1);
                    write_response(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": payload }],
                            "isError": false,
                        }),
                    );
                    continue;
                }
                if called_name == "_PostCompact" {
                    write_response(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": post_compact_text }],
                            "isError": false,
                        }),
                    );
                    continue;
                }
                if tool_delay_secs > 0 {
                    std::thread::sleep(std::time::Duration::from_secs(tool_delay_secs));
                }
                let result_text = if result_size > 0 {
                    "x".repeat(result_size)
                } else {
                    "ok".to_owned()
                };
                write_response(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": result_text }],
                        "isError": false,
                    }),
                );
            }
            _ => {
                // Unknown method: respond with an error so rmcp doesn't hang.
                let err = json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32601, "message": format!("method not found: {method}") },
                });
                let mut s = serde_json::to_string(&err).unwrap();
                s.push('\n');
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(s.as_bytes());
                let _ = out.flush();
            }
        }
    }
}
