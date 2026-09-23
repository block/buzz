//! End-to-end mesh + agent permutation harness — fully headless, no desktop
//! app, no keychain. Proves the whole chain the UI exercises:
//!
//!   share compute (serve node) → agent env preset → ACP agent → inference
//!
//! Permutations:
//!   P1 explicit-model chat  — agent pinned to the served model id replies.
//!   P2 virtual-mesh chat   — one served model exercises single-model fallback.
//!   P3 generous output ceiling succeeds (Mesh #1350); oversized input still
//!      fails context admission via the real HTTP router, before inference.
//!   P4 agentic tool use     — agent + buzz-dev-mcp writes a file on disk.
//!
//! The serve node is the same `mesh_llm_sdk::serve` path Share-compute uses
//! (publish off, mdns, loopback). The agent legs spawn the real
//! `buzz-agent` binary with the exact env vars the relay-mesh preset ships.
//!
//! Hardware-gated, not CI. Run:
//!   cargo build --release -p buzz-agent -p buzz-dev-mcp
//!   cargo run -p buzz-relay --example mesh_agent_e2e
//! Env: MESH_E2E_MODEL overrides the served model ref.
use std::process::Stdio;
use std::time::Duration;

use mesh_llm_sdk::{serve, MeshDiscoveryMode};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

// Qwen3-8B: cached GGUF *and* complete layer package on this class of
// machine, so the serve node starts in seconds. Qwen3-30B-A3B works too but
// mesh-llm serves it from layer packages and will download them on first
// serve (~7GB) — fine in the app (progress UI), too slow for a smoke.
const DEFAULT_MODEL: &str = "unsloth/Qwen3-8B-GGUF:Q4_K_M";
const API_PORT: u16 = 19437;
const CONSOLE_PORT: u16 = 13231;

fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        // Same fix the desktop ships: mesh-llm futures need >2MiB stacks.
        .thread_stack_size(8 * 1024 * 1024)
        .build()?
        .block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let model = std::env::var("MESH_E2E_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    mesh_llm_host_runtime::initialize_host_runtime()
        .await
        .map_err(|e| anyhow::anyhow!("host runtime init: {e}"))?;

    eprintln!("[e2e] starting serve node with {model} (loading may take a minute)...");
    let cfg = serve::EmbeddedServeConfig::builder()
        .model(&model)
        .api_port(API_PORT)
        .console_port(CONSOLE_PORT)
        .publish(false)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Mdns)
        .console_ui(true) // readiness poll needs the console bound
        .startup_timeout(Duration::from_secs(300))
        .build();
    let node = serve::start(cfg)
        .await
        .map_err(|e| anyhow::anyhow!("serve start: {e}"))?;
    let base = node.api_base_url().to_string();

    // Wait for the model to be loaded + resolvable, capture its served id.
    let http = reqwest::Client::new();
    let mut served_id = String::new();
    for _ in 0..120 {
        if let Ok(resp) = http.get(format!("{base}/models")).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(id) = json["data"].get(0).and_then(|m| m["id"].as_str()) {
                    served_id = id.to_string();
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    anyhow::ensure!(!served_id.is_empty(), "model never appeared in /models");
    eprintln!("[e2e] node up, served id = {served_id}");

    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut record = |name: &str, ok: bool, detail: String| {
        if ok {
            pass += 1;
            eprintln!("[e2e] PASS {name}: {detail}");
        } else {
            fail += 1;
            eprintln!("[e2e] FAIL {name}: {detail}");
        }
    };

    // P1: explicit model id.
    let r = agent_chat(
        &base,
        &served_id,
        None,
        "Reply with exactly one word: PONG",
        &[],
    )
    .await;
    match r {
        Ok(text) => record(
            "P1 explicit-model chat",
            text.to_uppercase().contains("PONG"),
            text,
        ),
        Err(e) => record("P1 explicit-model chat", false, e.to_string()),
    }

    // P2: the virtual `mesh` model — exactly what apply_relay_mesh_env now puts
    // on the wire for shared compute. With one served model there is no
    // committee, so this proves MeshLLM's degrade_to_single_model path.
    let r = agent_chat(
        &base,
        "mesh",
        None,
        "Reply with exactly one word: PONG",
        &[],
    )
    .await;
    match r {
        Ok(text) => record(
            "P2 virtual-mesh chat (degrades to single model)",
            text.to_uppercase().contains("PONG"),
            text,
        ),
        Err(e) => record(
            "P2 virtual-mesh chat (degrades to single model)",
            false,
            e.to_string(),
        ),
    }

    // Mesh v0.76.0-rc9 (9f192c9), routing_rank.rs:43-58 reserves
    // min(completion ceiling, 512) decode headroom, NOT the entire ceiling
    // (#1350). buzz-agent llm.rs sends max_completion_tokens on the chat API.
    // Keep the 150k ceiling as a positive regression, plus a negative input
    // fit probe below. Do not accept generic service failures as context errors.
    let r = agent_chat(
        &base,
        "mesh",
        Some("150000"),
        "Reply with exactly one word: PONG",
        &[],
    )
    .await;
    match r {
        Ok(text) => record("P3a generous output ceiling", text.trim() == "PONG", text),
        Err(e) => record("P3a generous output ceiling", false, e.to_string()),
    }
    let r = oversized_input_probe(&http, &base, &served_id).await;
    match r {
        Ok(detail) => record("P3b oversized input rejected", true, detail),
        Err(e) => record("P3b oversized input rejected", false, e.to_string()),
    }

    // P4: agentic tool use via buzz-dev-mcp — write a real file inside the
    // isolated ACP working directory. The MCP sandbox intentionally rejects
    // nonexistent absolute paths outside that root.
    let marker_name = format!("mesh-e2e-{}.txt", std::process::id());
    let prompt = format!(
        "Call dev__shell exactly once with command `{}` and no workdir override. This is the only authorized operation. Then confirm in text; do not run any other tools.", marker_command(&marker_name)
    );
    let mcp = vec![("dev".to_string(), repo_bin("buzz-dev-mcp")?)];
    let (r, marker) =
        agent_chat_with_marker(&base, "mesh", None, &prompt, &mcp, &marker_name).await;
    let file_ok = std::fs::read_to_string(&marker)
        .map(|c| c == "BUZZ_OK")
        .unwrap_or(false);
    match r {
        Ok(text) => record(
            "P4 agentic tool use",
            file_ok,
            if file_ok {
                format!("file written; agent said: {text}")
            } else {
                format!("no file at {}; agent said: {text}", marker.display())
            },
        ),
        Err(e) => record("P4 agentic tool use", false, format!("agent error: {e}")),
    }
    let _ = std::fs::remove_file(&marker);

    eprintln!("[e2e] {pass} passed, {fail} failed");
    if fail > 0 {
        eprintln!("[e2e] FAIL: {fail} permutation(s) failed");
        exit_without_native_destructors(1);
    }
    eprintln!("[e2e] PASS: share-compute → agent → inference proven end to end");
    exit_without_native_destructors(0);
}

/// Replace the process image so libc does not run llama.cpp's crashing Metal
/// `atexit` handlers (mesh-console issue #8). `std::process::exit` is not enough:
/// it skips Rust drops but still runs native C/C++ finalizers.
fn exit_without_native_destructors(code: i32) -> ! {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let program = if code == 0 { "true" } else { "false" };
        let error = std::process::Command::new(program).exec();
        eprintln!("[e2e] failed to exec {program}: {error}");
    }
    std::process::exit(code)
}

fn repo_bin(name: &str) -> anyhow::Result<String> {
    let path = std::env::current_dir()?.join("target/release").join(name);
    anyhow::ensure!(
        path.exists(),
        "{} missing — cargo build --release -p {name}",
        path.display()
    );
    Ok(path.to_string_lossy().into_owned())
}

/// Spawn the real buzz-agent with relay-mesh preset env and drive one ACP
/// session/prompt over stdio. Returns the concatenated agent message text,
/// or Err carrying the agent's error message.
async fn agent_chat(
    base: &str,
    model: &str,
    max_output_tokens: Option<&str>,
    prompt: &str,
    mcp_servers: &[(String, String)],
) -> anyhow::Result<String> {
    let (result, _) =
        agent_chat_in_isolated_home(base, model, max_output_tokens, prompt, mcp_servers, None)
            .await;
    result
}

async fn agent_chat_with_marker(
    base: &str,
    model: &str,
    max_output_tokens: Option<&str>,
    prompt: &str,
    mcp_servers: &[(String, String)],
    marker_name: &str,
) -> (anyhow::Result<String>, std::path::PathBuf) {
    let (result, home) = agent_chat_in_isolated_home(
        base,
        model,
        max_output_tokens,
        prompt,
        mcp_servers,
        Some(marker_name),
    )
    .await;
    (result, home.join(marker_name))
}

async fn agent_chat_in_isolated_home(
    base: &str,
    model: &str,
    max_output_tokens: Option<&str>,
    prompt: &str,
    mcp_servers: &[(String, String)],
    marker_name: Option<&str>,
) -> (anyhow::Result<String>, std::path::PathBuf) {
    let agent = match repo_bin("buzz-agent") {
        Ok(agent) => agent,
        Err(error) => return (Err(error), std::path::PathBuf::new()),
    };
    // Isolated HOME: no skills, no AGENTS.md chain, no keychain, tiny prompt.
    // Fresh per leg: stale markers and prior agent state can never pass P4.
    // Retain the directory for diagnostics (also when P4 fails).
    let home = match tempfile::Builder::new().prefix("mesh-e2e-home-").tempdir() {
        Ok(dir) => dir.keep(),
        Err(error) => return (Err(error.into()), std::path::PathBuf::new()),
    };
    eprintln!("[e2e] isolated home: {}", home.display());

    let mut command = Command::new(&agent);
    command
        .current_dir(&home)
        .kill_on_drop(true)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        // The transport subset of apply_relay_mesh_env(): provider, base URL,
        // model, key, and chat API. Not BUZZ_AGENT_REQUIRE_REPLY, which needs
        // Buzz's publish tools to mean anything.
        .env("BUZZ_AGENT_PROVIDER", "openai")
        .env("BUZZ_AGENT_MODEL", model)
        .env("OPENAI_COMPAT_BASE_URL", base)
        .env("OPENAI_COMPAT_MODEL", model)
        .env("OPENAI_COMPAT_API_KEY", "buzz-mesh-local")
        .env("OPENAI_COMPAT_API", "chat")
        .env("BUZZ_AGENT_MAX_OUTPUT_TOKENS", "4096")
        // No BUZZ_AGENT_THINKING_EFFORT: apply_relay_mesh_env() deliberately
        // leaves it unset so each model's chat template picks its own default.
        // Pinning a value here would test a config the product does not ship.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    // P3a exercises a generous ceiling, not an admission reservation.
    if let Some(value) = max_output_tokens {
        command.env("BUZZ_AGENT_MAX_OUTPUT_TOKENS", value);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return (Err(error.into()), home),
    };

    let result = drive_acp(&mut child, prompt, mcp_servers, &home, marker_name).await;
    let _ = child.kill().await;
    (result, home)
}

async fn drive_acp(
    child: &mut Child,
    prompt: &str,
    mcp_servers: &[(String, String)],
    cwd: &std::path::Path,
    marker_name: Option<&str>,
) -> anyhow::Result<String> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdout"))?;
    let mut lines = BufReader::new(stdout).lines();

    let mcp_json: Vec<serde_json::Value> = mcp_servers
        .iter()
        .map(|(name, command)| {
            serde_json::json!({ "name": name, "command": command, "args": [], "env": [] })
        })
        .collect();

    let send = |v: serde_json::Value| format!("{v}\n");
    stdin
        .write_all(
            send(serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": 1, "clientCapabilities": {} }
            }))
            .as_bytes(),
        )
        .await?;
    let mut session_id: Option<String> = None;
    let mut agent_text = String::new();
    let mut marker_approved = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);

    loop {
        let line = tokio::time::timeout_at(deadline, lines.next_line())
            .await
            .map_err(|_| anyhow::anyhow!("agent timed out; text so far: {agent_text}"))??
            .ok_or_else(|| anyhow::anyhow!("agent closed stdout; text so far: {agent_text}"))?;
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        // Requests have their own id namespace; handle before response ids.
        if msg["method"] == "session/request_permission" {
            let response = permission_response(
                &msg,
                session_id.as_deref(),
                marker_name,
                &mut marker_approved,
            );
            eprintln!(
                "[e2e] permission: {} -> {}",
                msg["params"]["toolCall"], response["result"]
            );
            stdin.write_all(send(response).as_bytes()).await?;
            continue;
        }
        if msg.get("method").is_some() && msg.get("id").is_some() {
            stdin
                .write_all(
                    send(serde_json::json!({
                        "jsonrpc": "2.0", "id": msg["id"],
                        "error": {"code": -32601, "message": "Unsupported harness method"}
                    }))
                    .as_bytes(),
                )
                .await?;
            continue;
        }
        // Collect any streamed agent text from session/update notifications.
        if msg.get("method").and_then(|m| m.as_str()) == Some("session/update") {
            if msg["params"]["sessionId"].as_str() == session_id.as_deref() {
                let update = &msg["params"]["update"];
                if matches!(
                    update["sessionUpdate"].as_str(),
                    Some("tool_call" | "tool_call_update")
                ) {
                    eprintln!("[e2e] tool update: {update}");
                }
                collect_text(update, &mut agent_text);
            }
            continue;
        }
        match msg.get("id").and_then(|i| i.as_i64()) {
            Some(1) => {
                anyhow::ensure!(msg.get("error").is_none(), "initialize failed: {msg}");
                anyhow::ensure!(
                    msg["result"]["protocolVersion"] == 1,
                    "expected ACP v1: {msg}"
                );
                stdin
        .write_all(
            send(serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "method": "session/new",
                "params": {
                    "cwd": cwd.to_string_lossy(),
                    "mcpServers": mcp_json,
                    "systemPrompt": "You are a terse test agent. Follow instructions exactly."
                }
            }))
            .as_bytes(),
        )
        .await?;
            }
            Some(2) => {
                if let Some(err) = msg.get("error") {
                    anyhow::bail!("session/new failed: {err}");
                }
                let sid = msg["result"]["sessionId"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("session/new: no sessionId: {msg}"))?
                    .to_string();
                stdin
                    .write_all(
                        send(serde_json::json!({
                            "jsonrpc": "2.0", "id": 3, "method": "session/prompt",
                            "params": {
                                "sessionId": sid,
                                "prompt": [ { "type": "text", "text": prompt } ]
                            }
                        }))
                        .as_bytes(),
                    )
                    .await?;
                session_id = Some(sid);
            }
            Some(3) => {
                anyhow::ensure!(session_id.is_some(), "prompt response before session");
                if let Some(err) = msg.get("error") {
                    anyhow::bail!("session/prompt failed: {err}");
                }
                anyhow::ensure!(
                    msg["result"]["stopReason"] == "end_turn",
                    "unexpected prompt stop: {msg}; text: {agent_text}"
                );
                anyhow::ensure!(
                    marker_name.is_none() || marker_approved,
                    "marker permission was never approved; text: {agent_text}"
                );
                return Ok(agent_text.trim().to_string());
            }
            _ => {}
        }
    }
}

/// Only user-visible assistant chunks count, never reasoning or tool content.
fn collect_text(value: &serde_json::Value, out: &mut String) {
    if value["sessionUpdate"] == "agent_message_chunk" && value["content"]["type"] == "text" {
        if let Some(text) = value["content"]["text"].as_str() {
            out.push_str(text);
        }
    }
}

fn marker_command(name: &str) -> String {
    format!("printf %s BUZZ_OK > {name}")
}

/// Test-client-only grant: one exact shell invocation, in a fresh cwd.
/// No command parsing, prefix matches, persistent grants, or alternate tools.
fn permission_response(
    request: &serde_json::Value,
    session: Option<&str>,
    marker: Option<&str>,
    approved: &mut bool,
) -> serde_json::Value {
    let p = &request["params"];
    let tool = &p["toolCall"]; // We negotiate v1 only; fail closed on other shapes.
    let input = &tool["rawInput"];
    let safe_marker = marker.filter(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    });
    let exact_input = safe_marker.is_some_and(|name| {
        input.as_object().is_some_and(|o| {
            o.keys()
                .all(|k| k == "command" || k == "timeout_ms" || k == "workdir")
                // MCP treats absent/null workdir as its default cwd; no overrides.
                && (input.get("workdir").is_none() || input["workdir"].is_null())
                && input["command"] == marker_command(name)
                && (input.get("timeout_ms").is_none()
                    || input["timeout_ms"]
                        .as_u64()
                        .is_some_and(|n| n > 0 && n <= 120_000))
        })
    });
    let allow = !*approved
        && session.is_some()
        && p["sessionId"].as_str() == session
        && tool["title"] == "dev__shell"
        && exact_input;
    let option = p["options"]
        .as_array()
        .and_then(|options| options.iter().find(|o| o["kind"] == "allow_once"))
        .and_then(|o| o["optionId"].as_str());
    let outcome = if let Some(id) = option.filter(|_| allow) {
        *approved = true;
        serde_json::json!({"outcome": "selected", "optionId": id})
    } else {
        serde_json::json!({"outcome": "cancelled"})
    };
    serde_json::json!({"jsonrpc": "2.0", "id": request["id"], "result": {"outcome": outcome}})
}

async fn oversized_input_probe(
    http: &reqwest::Client,
    base: &str,
    model: &str,
) -> anyhow::Result<String> {
    // Use advertised runtime capacity, not native model capacity. Explicit
    // model routing avoids conflating MoA selection failure with context fit.
    let models: serde_json::Value = http
        .get(format!("{base}/models"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let entry = models["data"]
        .as_array()
        .and_then(|a| a.iter().find(|m| m["id"] == model))
        .ok_or_else(|| anyhow::anyhow!("served model absent: {models}"))?;
    let metadata = &entry["metadata"];
    let context = metadata["max_context_length"]
        .as_u64()
        .or_else(|| metadata["context_length"].as_u64())
        .ok_or_else(|| anyhow::anyhow!("no advertised runtime context: {entry}"))?;
    // Bound allocation/request size; fail rather than guess for huge models.
    anyhow::ensure!(
        context > 0 && context <= 262_144,
        "unsupported smoke context: {context}"
    );
    // rc9 estimates serialized input bytes / 4, retains ALL prompt tokens.
    let body = serde_json::json!({"model": model, "stream": false,
        "max_completion_tokens": 1,
        "messages": [{"role": "user", "content": "x".repeat((context as usize + 1024) * 4)}]});
    let response = http
        .post(format!("{base}/chat/completions"))
        .timeout(Duration::from_secs(30))
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await?;
    anyhow::ensure!(
        status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            && text.contains("no context-compatible target"),
        "expected context-fit 503 for runtime context {context}, got {status}: {}",
        text.chars().take(2048).collect::<String>()
    );
    Ok(format!("runtime context {context}; {status}: {text}"))
}

#[cfg(test)]
#[path = "mesh_agent_e2e/tests.rs"]
mod tests;
