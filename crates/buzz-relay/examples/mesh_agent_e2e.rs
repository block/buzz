//! End-to-end mesh + agent permutation harness — fully headless, no desktop
//! app, no keychain. Proves the whole chain the UI exercises:
//!
//!   share compute (serve node) → agent env preset → ACP agent → inference
//!
//! Permutations:
//!   P1 explicit-model chat  — agent pinned to the served model id replies.
//!   P2 auto-model chat      — agent sends `model: "auto"`; mesh router picks.
//!   P3 context-fit regression — an oversized output budget (150k tokens)
//!      must FAIL with the router's context error (proves the router's fit
//!      gate — the failure mode the 1024 preset cap protects against).
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
        "1024",
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

    // P2: auto — router picks the model.
    let r = agent_chat(
        &base,
        "auto",
        "1024",
        "Reply with exactly one word: PONG",
        &[],
    )
    .await;
    match r {
        Ok(text) => record(
            "P2 auto-model chat",
            text.to_uppercase().contains("PONG"),
            text,
        ),
        Err(e) => record("P2 auto-model chat", false, e.to_string()),
    }

    // P3: regression — an output budget no served model's context can hold
    // must be rejected by the router with the context-fit error (the failure
    // mode that broke relay-mesh agents when buzz-agent's default 32768
    // budget met a 32k-context model). 150k output: passes buzz-agent's own
    // config validation (must stay under its 200k max_context_tokens) but
    // with the router's +25% margin overflows even 128k-context models like
    // GLM-4.7-Flash.
    let r = agent_chat(
        &base,
        &served_id,
        "150000",
        "Reply with exactly one word: PONG",
        &[],
    )
    .await;
    match r {
        Ok(text) => record(
            "P3 oversized-budget must fail",
            false,
            format!("unexpectedly succeeded: {text}"),
        ),
        Err(e) => {
            let msg = e.to_string();
            let is_context_503 = msg.contains("503")
                || msg.contains("service_unavailable")
                || msg.contains("context-compatible");
            record("P3 oversized-budget must fail", is_context_503, msg);
        }
    }

    // P4: agentic tool use via buzz-dev-mcp — write a real file inside the
    // isolated ACP working directory. The MCP sandbox intentionally rejects
    // nonexistent absolute paths outside that root.
    let marker_name = format!("mesh-e2e-{}.txt", std::process::id());
    let prompt = format!(
        "Use your developer tools to create {marker_name} in the current working directory containing exactly the text BUZZ_OK (no quotes, no newline commentary). Then confirm."
    );
    let mcp = vec![("dev".to_string(), repo_bin("buzz-dev-mcp")?)];
    let (r, marker) =
        agent_chat_with_marker(&base, &served_id, "1024", &prompt, &mcp, &marker_name).await;
    let file_ok = std::fs::read_to_string(&marker)
        .map(|c| c.contains("BUZZ_OK"))
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
        Err(e) => record("P4 agentic tool use", file_ok, format!("agent error: {e}")),
    }
    let _ = std::fs::remove_file(&marker);

    eprintln!("[e2e] {pass} passed, {fail} failed");
    if fail > 0 {
        anyhow::bail!("{fail} permutation(s) failed");
    }
    eprintln!("[e2e] PASS: share-compute → agent → inference proven end to end");
    // ggml teardown aborts in C++ static destructors; skip them (issue #8).
    std::process::exit(0);
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
    max_output_tokens: &str,
    prompt: &str,
    mcp_servers: &[(String, String)],
) -> anyhow::Result<String> {
    let (result, _) =
        agent_chat_in_isolated_home(base, model, max_output_tokens, prompt, mcp_servers).await;
    result
}

async fn agent_chat_with_marker(
    base: &str,
    model: &str,
    max_output_tokens: &str,
    prompt: &str,
    mcp_servers: &[(String, String)],
    marker_name: &str,
) -> (anyhow::Result<String>, std::path::PathBuf) {
    let (result, home) =
        agent_chat_in_isolated_home(base, model, max_output_tokens, prompt, mcp_servers).await;
    (result, home.join(marker_name))
}

async fn agent_chat_in_isolated_home(
    base: &str,
    model: &str,
    max_output_tokens: &str,
    prompt: &str,
    mcp_servers: &[(String, String)],
) -> (anyhow::Result<String>, std::path::PathBuf) {
    let agent = match repo_bin("buzz-agent") {
        Ok(agent) => agent,
        Err(error) => return (Err(error), std::path::PathBuf::new()),
    };
    // Isolated HOME: no skills, no AGENTS.md chain, no keychain, tiny prompt.
    let home = std::env::temp_dir().join(format!("mesh-e2e-home-{}", std::process::id()));
    if let Err(error) = std::fs::create_dir_all(&home) {
        return (Err(error.into()), home);
    }

    let mut child = match Command::new(&agent)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        // Exactly the relay-mesh preset env (preset.rs).
        .env("BUZZ_AGENT_PROVIDER", "openai")
        .env("BUZZ_AGENT_MODEL", model)
        .env("OPENAI_COMPAT_BASE_URL", base)
        .env("OPENAI_COMPAT_MODEL", model)
        .env("OPENAI_COMPAT_API_KEY", "buzz-mesh-local")
        .env("OPENAI_COMPAT_API", "chat")
        .env("BUZZ_AGENT_MAX_OUTPUT_TOKENS", max_output_tokens)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return (Err(error.into()), home),
    };

    let result = drive_acp(&mut child, prompt, mcp_servers, &home).await;
    let _ = child.kill().await;
    (result, home)
}

async fn drive_acp(
    child: &mut Child,
    prompt: &str,
    mcp_servers: &[(String, String)],
    cwd: &std::path::Path,
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

    let mut session_id: Option<String> = None;
    let mut agent_text = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);

    loop {
        let line = tokio::time::timeout_at(deadline, lines.next_line())
            .await
            .map_err(|_| anyhow::anyhow!("agent timed out; text so far: {agent_text}"))??
            .ok_or_else(|| anyhow::anyhow!("agent closed stdout; text so far: {agent_text}"))?;
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        // Collect any streamed agent text from session/update notifications.
        if msg.get("method").and_then(|m| m.as_str()) == Some("session/update") {
            collect_text(&msg["params"]["update"], &mut agent_text);
            continue;
        }
        match msg.get("id").and_then(|i| i.as_i64()) {
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
                return Ok(agent_text.trim().to_string());
            }
            _ => {}
        }
    }
}

/// Recursively harvest "text" string fields out of a session/update payload.
fn collect_text(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == "text" {
                    if let Some(s) = v.as_str() {
                        out.push_str(s);
                    }
                } else {
                    collect_text(v, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text(item, out);
            }
        }
        _ => {}
    }
}
