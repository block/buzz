//! Local mesh-serve inference smoke test.
//!
//! Serves a GGUF model through the same `mesh_llm_sdk::serve` path Sprout
//! desktop uses in "Share compute" mode, then drives one chat completion
//! against the node's local OpenAI-compatible endpoint. No mesh publish, no
//! auto-join, no Nostr discovery — pure single-node serve-and-self-consume,
//! which is exactly the loopback variant we can prove on one box.
//!
//! Usage:
//!   cargo run -p sprout-relay --example mesh_serve_smoke -- <path-to.gguf>
use std::time::Duration;

use mesh_llm_sdk::{serve, MeshDiscoveryMode};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: mesh_serve_smoke <path-to.gguf>"))?;
    eprintln!("[smoke] serving model: {model}");

    let config = serve::EmbeddedServeConfig::builder()
        .model(&model)
        .api_port(19337)
        .console_port(13131)
        .publish(false)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Mdns)
        // NOTE: console_ui(true) is required here, not cosmetic. In mesh rev
        // bd16da4, serve::start always polls `:console_port/api/status` to
        // confirm readiness, but the console HTTP server only binds when
        // !headless (i.e. console_ui == true). With console_ui(false) the
        // poll never succeeds and startup times out after 30s. Desktop's
        // Share-compute path sets console_ui(false) — likely hits the same
        // wall. Flagged to the team.
        .console_ui(true)
        .build();

    let node = serve::start(config).await?;
    let base = node.api_base_url().to_string();
    eprintln!("[smoke] node up, api_base_url = {base}");

    // Poll until the model reports loaded/ready (give it generous time — first
    // load of a 17GB GGUF into Metal can take a while).
    let http = reqwest::Client::new();
    let models_url = format!("{base}/models");
    let mut model_id = String::new();
    for i in 0..120 {
        tokio::time::sleep(Duration::from_secs(5)).await;
        match http.get(&models_url).send().await {
            Ok(r) => {
                let body = r.text().await.unwrap_or_default();
                // The serve node assigns its own id (e.g. local-gguf/sha256-…),
                // not the file path we passed — pull it from /models.
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(id) = json["data"].get(0).and_then(|m| m["id"].as_str()) {
                        model_id = id.to_string();
                        eprintln!(
                            "[smoke] /models after {}s, served id = {model_id}",
                            (i + 1) * 5
                        );
                        break;
                    }
                }
                eprintln!("[smoke] waiting ({}s) /models -> {body}", (i + 1) * 5);
            }
            Err(e) => eprintln!("[smoke] waiting ({}s) /models err: {e}", (i + 1) * 5),
        }
    }
    if model_id.is_empty() {
        anyhow::bail!("model never became ready within timeout");
    }

    // One real completion — use the server's own model id.
    let chat_url = format!("{base}/chat/completions");
    let req = serde_json::json!({
        "model": model_id,
        "messages": [
            {"role": "user", "content": "Reply with exactly one word: PONG"}
        ],
        "max_tokens": 512,
        "temperature": 0.0
    });
    eprintln!("[smoke] POST {chat_url}");
    let resp = http.post(&chat_url).json(&req).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    println!("[smoke] completion status={status}");
    println!("[smoke] completion body={body}");

    node.stop().await?;
    if !status.is_success() {
        anyhow::bail!("completion request failed: {status}");
    }
    Ok(())
}
