//! Integration tests for the PKCE OAuth token source.
//!
//! No browser dance — we cover the silent-refresh and cache-hit paths
//! against a stubbed OIDC server (axum). The interactive browser flow is
//! exercised manually via the `sprout-agent auth databricks` subcommand
//! (see `lib.rs::auth_subcommand`).
//!
//! The second test module (further down) is an ACP-level envelope
//! regression: it spawns the real `sprout-agent` binary with
//! `DATABRICKS_TOKEN` set and a stub HTTP server, then asserts the wire
//! shape we send to Databricks.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::Form;
use axum::{routing::get, routing::post, Json, Router};
use serde::Deserialize;
use serde_json::json;
use sprout_agent::auth::{PkceOAuthConfig, PkceOAuthTokenSource, TokenSource};
use tempfile::TempDir;

#[derive(Deserialize)]
struct TokenForm {
    grant_type: String,
    #[allow(dead_code)]
    refresh_token: Option<String>,
}

/// Boot a stub OIDC server that:
///   - serves discovery at `/.well-known/oauth-authorization-server`
///   - issues a fresh access token for every `refresh_token` request
///   - counts how many refresh hits it gets
async fn spawn_oidc() -> (String, Arc<AtomicU64>) {
    let counter = Arc::new(AtomicU64::new(0));
    let counter_for_token = counter.clone();

    // Bind first so we know our own base URL before building the router.
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let base_for_discovery = base.clone();

    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(move || {
                let base = base_for_discovery.clone();
                async move {
                    Json(json!({
                        "authorization_endpoint": format!("{base}/authorize"),
                        "token_endpoint": format!("{base}/token"),
                    }))
                }
            }),
        )
        .route(
            "/token",
            post(move |Form(form): Form<TokenForm>| {
                let counter = counter_for_token.clone();
                async move {
                    let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    assert_eq!(form.grant_type, "refresh_token");
                    Json(json!({
                        "access_token": format!("fresh-token-{n}"),
                        "refresh_token": "rotated-refresh",
                        "expires_in": 3600,
                    }))
                }
            }),
        );

    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, counter)
}

/// Cache key construction matches the auth module: sha256(discovery|client|scopes).
fn cache_path_for(cache_dir: &std::path::Path, cfg: &PkceOAuthConfig) -> std::path::PathBuf {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(cfg.discovery_url.as_bytes());
    h.update(b"|");
    h.update(cfg.client_id.as_bytes());
    h.update(b"|");
    h.update(cfg.scopes.join(",").as_bytes());
    let hash = hex::encode(h.finalize());
    cache_dir
        .join(&cfg.cache_namespace)
        .join(format!("{hash}.json"))
}

/// Write a token file the engine should pick up on construction.
fn seed_cache(path: &std::path::Path, body: serde_json::Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&body).unwrap()).unwrap();
}

#[tokio::test]
async fn cache_hit_short_circuits_network() {
    let tmp = TempDir::new().unwrap();

    let (base, refresh_counter) = spawn_oidc().await;
    let cfg = PkceOAuthConfig {
        discovery_url: format!("{base}/.well-known/oauth-authorization-server"),
        client_id: "test-client".into(),
        scopes: vec!["a".into(), "b".into()],
        cache_namespace: "databricks".into(),
        cache_dir_override: Some(tmp.path().to_path_buf()),
    };

    // Seed an unexpired token in the cache.
    let future = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let path = cache_path_for(tmp.path(), &cfg);
    seed_cache(
        &path,
        json!({
            "access_token": "cached-token",
            "refresh_token": "rt",
            "expires_at": future,
        }),
    );

    let src = PkceOAuthTokenSource::new(cfg).unwrap();
    let bearer = src.bearer().await.unwrap();
    assert_eq!(bearer, "cached-token");
    assert_eq!(
        refresh_counter.load(Ordering::SeqCst),
        0,
        "no refresh should fire"
    );
}

#[tokio::test]
async fn expired_cache_silently_refreshes() {
    let tmp = TempDir::new().unwrap();

    let (base, refresh_counter) = spawn_oidc().await;
    let cfg = PkceOAuthConfig {
        discovery_url: format!("{base}/.well-known/oauth-authorization-server"),
        client_id: "test-client".into(),
        scopes: vec!["a".into()],
        cache_namespace: "databricks".into(),
        cache_dir_override: Some(tmp.path().to_path_buf()),
    };

    // Seed an already-expired token with a refresh_token.
    let path = cache_path_for(tmp.path(), &cfg);
    seed_cache(
        &path,
        json!({
            "access_token": "stale",
            "refresh_token": "valid-refresh",
            "expires_at": 1u64, // way in the past
        }),
    );

    let src = PkceOAuthTokenSource::new(cfg).unwrap();
    let bearer = src.bearer().await.unwrap();
    assert_eq!(bearer, "fresh-token-1");
    assert_eq!(refresh_counter.load(Ordering::SeqCst), 1);

    // A second call should hit the in-memory cache and skip the network.
    let bearer2 = src.bearer().await.unwrap();
    assert_eq!(bearer2, "fresh-token-1");
    assert_eq!(refresh_counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn refreshed_token_is_persisted_to_disk() {
    let tmp = TempDir::new().unwrap();

    let (base, _) = spawn_oidc().await;
    let cfg = PkceOAuthConfig {
        discovery_url: format!("{base}/.well-known/oauth-authorization-server"),
        client_id: "test-client".into(),
        scopes: vec!["a".into()],
        cache_namespace: "databricks".into(),
        cache_dir_override: Some(tmp.path().to_path_buf()),
    };

    let path = cache_path_for(tmp.path(), &cfg);
    seed_cache(
        &path,
        json!({
            "access_token": "stale",
            "refresh_token": "valid-refresh",
            "expires_at": 1u64,
        }),
    );

    let src = PkceOAuthTokenSource::new(cfg).unwrap();
    let _ = src.bearer().await.unwrap();

    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(on_disk["access_token"], "fresh-token-1");
    assert_eq!(on_disk["refresh_token"], "rotated-refresh");
    assert!(on_disk["expires_at"].is_u64());
}

// ────────────────────────────────────────────────────────────────────────────
// ACP-level envelope regression test.
//
// Boots the real sprout-agent binary with `DATABRICKS_TOKEN` set (so the
// OAuth dance is skipped) pointed at a stub HTTP server that captures every
// inbound request. Asserts the wire-level shape Databricks model serving
// requires: path is `/serving-endpoints/<model>/invocations`, Authorization
// is `Bearer <token>`, and the JSON body has *no* top-level `"model"`. This
// locks in the DRY envelope behavior so a refactor of `post_openai` can't
// silently break Databricks.
// ────────────────────────────────────────────────────────────────────────────

use std::collections::VecDeque;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Debug)]
struct CapturedRequest {
    path: String,
    authorization: Option<String>,
    body: serde_json::Value,
}

async fn spawn_capturing_server(
    responses: Vec<serde_json::Value>,
) -> (String, Arc<Mutex<Vec<CapturedRequest>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
    let captured = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
    let cap_for_task = captured.clone();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let queue = queue.clone();
            let captured = cap_for_task.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                    if buf.len() > 4_000_000 {
                        return;
                    }
                }
                let header_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
                let (request_line, rest) = header_str.split_once('\n').unwrap_or(("", ""));
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                let mut authorization = None;
                let mut body_len = 0usize;
                for line in rest.lines() {
                    // Split case-insensitively on the colon but keep the value's case intact.
                    let Some((name, value)) = line.split_once(':') else {
                        continue;
                    };
                    let value = value.trim().trim_end_matches('\r').to_string();
                    match name.trim().to_ascii_lowercase().as_str() {
                        "authorization" => authorization = Some(value),
                        "content-length" => body_len = value.parse().unwrap_or(0),
                        _ => {}
                    }
                }
                while buf.len() < header_end + body_len {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let body: serde_json::Value =
                    serde_json::from_slice(&buf[header_end..header_end + body_len])
                        .unwrap_or(json!(null));
                captured.lock().await.push(CapturedRequest {
                    path,
                    authorization,
                    body,
                });
                let body = queue
                    .lock()
                    .await
                    .pop_front()
                    .unwrap_or_else(|| json!({ "error": "no canned response" }));
                let body_s = serde_json::to_string(&body).unwrap();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body_s.len(),
                    body_s,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (url, captured)
}

struct AgentHarness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
}

impl Drop for AgentHarness {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl AgentHarness {
    async fn spawn_databricks(base_url: &str, model: &str) -> Self {
        let bin = env!("CARGO_BIN_EXE_sprout-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("SPROUT_AGENT_PROVIDER", "databricks")
            .env("DATABRICKS_HOST", base_url)
            .env("DATABRICKS_MODEL", model)
            .env("DATABRICKS_TOKEN", "test-bearer")
            .env("SPROUT_AGENT_LLM_TIMEOUT_SECS", "5")
            .env("SPROUT_AGENT_TOOL_TIMEOUT_SECS", "5")
            .env("SPROUT_AGENT_MAX_ROUNDS", "2")
            .env("SPROUT_AGENT_MCP_INIT_TIMEOUT_SECS", "2")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().expect("spawn sprout-agent");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    async fn send(&mut self, method: &str, params: serde_json::Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        let mut s = serde_json::to_string(
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .unwrap();
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
        id
    }

    async fn recv_for(&mut self, want_id: i64) -> serde_json::Value {
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(Duration::from_secs(15), self.stdout.read_line(&mut line))
                .await
                .expect("recv timeout")
                .expect("read line");
            assert!(n > 0, "agent EOF");
            let v: serde_json::Value = serde_json::from_str(&line).expect("non-JSON line");
            if v.get("id") == Some(&json!(want_id)) {
                return v;
            }
        }
    }
}

#[tokio::test]
async fn databricks_envelope_routes_through_serving_endpoints_and_strips_model() {
    // One canned chat-completions-shaped response → assistant says "ok"
    // with end_turn so the agent loop exits cleanly.
    let canned = vec![json!({
        "id": "x",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "ok" },
            "finish_reason": "stop"
        }]
    })];
    let (base, captured) = spawn_capturing_server(canned).await;

    let model = "goose-claude-4-6-sonnet";
    let mut h = AgentHarness::spawn_databricks(&base, model).await;
    h.send(
        "initialize",
        json!({ "protocolVersion": 1, "clientCapabilities": {} }),
    )
    .await;
    h.recv_for(1).await;
    h.send("session/new", json!({ "cwd": "/tmp", "mcpServers": [] }))
        .await;
    let r = h.recv_for(2).await;
    let sid = r["result"]["sessionId"].as_str().unwrap().to_string();
    h.send(
        "session/prompt",
        json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "say ok" }] }),
    )
    .await;
    let _ = h.recv_for(3).await;

    let reqs = captured.lock().await;
    assert_eq!(reqs.len(), 1, "expected exactly one LLM request");
    let req = &reqs[0];

    assert_eq!(
        req.path,
        format!("/serving-endpoints/{model}/invocations"),
        "Databricks must route to serving-endpoints/{{model}}/invocations"
    );
    assert_eq!(
        req.authorization.as_deref(),
        Some("Bearer test-bearer"),
        "Authorization must be the static DATABRICKS_TOKEN as a Bearer"
    );
    assert!(
        req.body.get("model").is_none(),
        "request body must NOT include `model` (Databricks rejects it): {:?}",
        req.body
    );
    // Sanity: the rest of the chat envelope should still be there.
    assert!(
        req.body
            .get("messages")
            .and_then(|v| v.as_array())
            .is_some(),
        "request body should keep the chat `messages` field"
    );
}
