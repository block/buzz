//! Local mesh serve→client→inference smoke test.
//!
//! Proves the full path Sprout's "Share compute" + "Run on relay mesh" pair
//! relies on, on a single box and without a relay or Nostr discovery:
//!
//!   1. Start a SERVE node hosting a GGUF (the `serve::start` path desktop
//!      uses in Share-compute mode), and read its mesh invite token.
//!   2. Start a CLIENT node joined to that serve node via the invite token
//!      (the `client::start` path an agent's "Run on relay mesh" provider
//!      uses), binding its own local OpenAI-compatible endpoint.
//!   3. Drive one chat completion against the CLIENT endpoint and assert it
//!      routed through the mesh to the serve node and produced real output.
//!
//! Serve-only proves less than the PR claims (serve + client + routing), so
//! this exercises the client hop end to end.
//!
//! This is hardware-gated and NOT a CI test — it loads a real model and runs
//! inference. It lives as an example so CI never auto-runs it.
//!
//! Usage:
//!   # default model is a ~100MB instruct model, downloaded on first run:
//!   cargo run -p buzz-relay --example mesh_serve_client_smoke
//!
//!   # or point at any local .gguf / hf model ref (e.g. the on-hardware 35B):
//!   MESH_SMOKE_MODEL=/path/to/model.gguf \
//!     cargo run -p buzz-relay --example mesh_serve_client_smoke
use std::time::Duration;

use mesh_llm_sdk::{client, serve, MeshDiscoveryMode};

/// Small, real instruct model the mesh project itself uses for CI smoke.
/// Downloaded on first run; ~100MB.
const DEFAULT_MODEL: &str = "jc-builds/SmolLM2-135M-Instruct-Q4_K_M-GGUF:Q4_K_M";

const SERVE_API_PORT: u16 = 19337;
const SERVE_CONSOLE_PORT: u16 = 13131;
const CLIENT_API_PORT: u16 = 19338;
const CLIENT_CONSOLE_PORT: u16 = 13132;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = std::env::var("MESH_SMOKE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    eprintln!("[smoke] model: {model}");
    let cache = mesh_llm_sdk::native_runtime::native_runtime_cache(None)?;
    let current = mesh_llm_sdk::native_runtime::CURRENT_MESH_VERSION;
    if !cache
        .installed()?
        .iter()
        .any(|runtime| runtime.mesh_version == current)
    {
        anyhow::bail!("MeshLLM native runtime for MeshLLM {current} is not installed; run `just mesh-e2e-hardware` to prepare it");
    }
    mesh_llm_host_runtime::initialize_host_runtime()
        .map_err(|error| anyhow::anyhow!("MeshLLM host runtime init failed: {error}"))?;
    eprintln!("[smoke] MeshLLM host runtime initialized");

    // ── 1. Serve node ────────────────────────────────────────────────────────
    let serve_cfg = serve::EmbeddedServeConfig::builder()
        .model(&model)
        .api_port(SERVE_API_PORT)
        .console_port(SERVE_CONSOLE_PORT)
        // publish so the client can join via the invite token; Mdns keeps
        // discovery local — no relay, no Nostr.
        .publish(true)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Mdns)
        // console_ui(true) is required for readiness polling at rev bd16da4
        // (serve::start polls :console_port/api/status, which only binds when
        // !headless). See mesh_serve_smoke.rs for the full note.
        .console_ui(true)
        .build();

    eprintln!("[smoke] starting serve node...");
    let serve_node = serve::start(serve_cfg).await?;
    let serve_base = serve_node.api_base_url().to_string();
    eprintln!("[smoke] serve up, api_base_url = {serve_base}");

    let invite = serve_node
        .invite_token()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("serve node produced no invite token to join"))?;
    eprintln!("[smoke] serve invite token acquired (len {})", invite.len());

    // Wait until the serve node reports the model loaded, so the client has
    // something to route to.
    let http = reqwest::Client::new();
    let serve_model_id = wait_for_model(&http, &serve_base).await?;
    eprintln!("[smoke] serve model ready: {serve_model_id}");

    // ── 2. Client node, joined to the serve node ─────────────────────────────
    let client_cfg = client::EmbeddedClientConfig::builder()
        .api_port(CLIENT_API_PORT)
        .console_port(CLIENT_CONSOLE_PORT)
        .publish(false)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Mdns)
        .join_token(&invite)
        .console_ui(true)
        .build();

    eprintln!("[smoke] starting client node joined to serve...");
    let client_node = client::start(client_cfg).await?;
    let client_base = client_node.api_base_url().to_string();
    eprintln!("[smoke] client up, api_base_url = {client_base}");

    // The served model must propagate across the mesh to the client's
    // /models view before we can route a completion through it.
    let routed_model_id = wait_for_model(&http, &client_base).await?;
    eprintln!("[smoke] client sees routed model: {routed_model_id}");

    // ── 3. One real completion, through the CLIENT (mesh hop) ─────────────────
    let chat_url = format!("{client_base}/chat/completions");
    let req = serde_json::json!({
        "model": routed_model_id,
        "messages": [{"role": "user", "content": "Reply with exactly one word: PONG"}],
        "max_tokens": 512,
        "temperature": 0.0
    });
    eprintln!("[smoke] POST {chat_url} (routes serve←client over mesh)");
    let resp = http.post(&chat_url).json(&req).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    println!("[smoke] completion status={status}");
    println!("[smoke] completion body={body}");

    // Tear down both nodes before asserting, so a failed assert still cleans up.
    let _ = client_node.stop().await;
    let _ = serve_node.stop().await;

    if !status.is_success() {
        anyhow::bail!("completion through client failed: {status}");
    }
    let json: serde_json::Value = serde_json::from_str(&body)?;
    let content = json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("");
    let finish = json["choices"][0]["finish_reason"].as_str().unwrap_or("");
    if content.trim().is_empty() {
        anyhow::bail!("completion routed but content was empty");
    }
    println!("[smoke] OK — routed completion finish_reason={finish:?} content={content:?}");
    eprintln!("[smoke] PASS: serve→client→inference proven over mesh");
    Ok(())
}

/// Poll a node's `/models` until it reports a model, returning the served id
/// (the node assigns its own id, e.g. `local-gguf/sha256-…`, not our ref).
async fn wait_for_model(http: &reqwest::Client, api_base: &str) -> anyhow::Result<String> {
    let url = format!("{api_base}/models");
    for i in 0..120 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        match http.get(&url).send().await {
            Ok(r) => {
                let body = r.text().await.unwrap_or_default();
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(id) = json["data"].get(0).and_then(|m| m["id"].as_str()) {
                        return Ok(id.to_string());
                    }
                }
                eprintln!("[smoke] waiting ({}s) {url} -> {body}", (i + 1) * 5);
            }
            Err(e) => eprintln!("[smoke] waiting ({}s) {url} err: {e}", (i + 1) * 5),
        }
    }
    anyhow::bail!("model never became visible at {api_base} within timeout")
}
