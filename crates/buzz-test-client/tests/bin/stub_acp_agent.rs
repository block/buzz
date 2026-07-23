//! Minimal ACP-speaking stub agent for harness integration tests.
//!
//! Speaks newline-delimited JSON-RPC 2.0 over stdio — just enough of the
//! Agent Client Protocol for `buzz-acp` to initialize, create a session, and
//! run prompt turns. Each `session/prompt` appends one line to the file named
//! by `STUB_TURN_LOG`, then immediately ends the turn — the test counts lines
//! to observe exactly how many turns the harness dispatched. The stub never
//! contacts the relay: thread participation is recorded by the harness at
//! dispatch time, so turn counts alone prove admission/suppression behavior.

use std::io::{BufRead, Write};

fn respond(
    stdout: &mut std::io::StdoutLock<'_>,
    id: &serde_json::Value,
    result: serde_json::Value,
) {
    let msg = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    writeln!(stdout, "{msg}").expect("write response");
    stdout.flush().expect("flush stdout");
}

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let turn_log = std::env::var("STUB_TURN_LOG").ok();
    let mut turn: u64 = 0;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            Ok(_) => continue,
            Err(_) => break,
        };
        let msg: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

        let Some(id) = id else {
            continue; // notification — nothing to answer
        };

        match method {
            "initialize" => respond(
                &mut out,
                &id,
                serde_json::json!({ "protocolVersion": 2, "agentCapabilities": {} }),
            ),
            "session/new" => respond(
                &mut out,
                &id,
                serde_json::json!({ "sessionId": "stub-session-1" }),
            ),
            "session/prompt" => {
                turn += 1;
                if let Some(path) = &turn_log {
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .expect("open turn log");
                    writeln!(f, "turn {turn}").expect("append turn log");
                }
                respond(
                    &mut out,
                    &id,
                    serde_json::json!({ "stopReason": "end_turn" }),
                );
            }
            // Anything else the harness asks for (config options, custom
            // extensions): empty success keeps it moving; it tolerates both
            // empty results and method-not-found for optional calls.
            _ => respond(&mut out, &id, serde_json::json!({})),
        }
    }
}
