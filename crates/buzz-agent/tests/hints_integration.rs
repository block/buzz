//! Integration tests for AGENTS.md / SKILL.md hint loading.
//!
//! Uses the same subprocess + capturing-LLM pattern as `regressions.rs`.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

// ─── Fake LLM ────────────────────────────────────────────────────────────────

struct CapturingLlm {
    url: String,
    captured: Arc<Mutex<Vec<Value>>>,
}

async fn spawn_capturing_llm(responses: Vec<Value>) -> CapturingLlm {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
    let captured: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let cap2 = captured.clone();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let queue = queue.clone();
            let captured = cap2.clone();
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
                let headers = &buf[..header_end];
                let mut body_len = 0usize;
                for line in headers.split(|b| *b == b'\n') {
                    let line = std::str::from_utf8(line).unwrap_or("");
                    if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        body_len = rest.trim().trim_end_matches('\r').parse().unwrap_or(0);
                    }
                }
                while buf.len() < header_end + body_len {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                if let Ok(req) = serde_json::from_slice::<Value>(&buf[header_end..]) {
                    captured.lock().await.push(req);
                }
                let body = queue
                    .lock()
                    .await
                    .pop_front()
                    .unwrap_or_else(|| json!({ "error": "no canned response" }));
                let body_s = serde_json::to_string(&body).unwrap();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body_s.len(),
                    body_s,
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    CapturingLlm { url, captured }
}

// ─── Harness ─────────────────────────────────────────────────────────────────

struct Harness {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: i64,
}

impl Harness {
    async fn spawn_with_env(base_url: &str, extra: &[(&str, &str)]) -> Self {
        let bin = env!("CARGO_BIN_EXE_buzz-agent");
        let mut cmd = tokio::process::Command::new(bin);
        cmd.env("BUZZ_AGENT_PROVIDER", "openai")
            .env("OPENAI_COMPAT_API_KEY", "test")
            .env("OPENAI_COMPAT_MODEL", "fake-model")
            .env("OPENAI_COMPAT_BASE_URL", base_url)
            .env("BUZZ_AGENT_LLM_TIMEOUT_SECS", "5")
            .env("BUZZ_AGENT_TOOL_TIMEOUT_SECS", "5")
            .env("BUZZ_AGENT_MAX_ROUNDS", "8")
            .env("BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS", "2");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = cmd.spawn().expect("spawn buzz-agent");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    async fn send(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.write(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        id
    }

    async fn write(&mut self, msg: Value) {
        let mut s = serde_json::to_string(&msg).unwrap();
        s.push('\n');
        self.stdin.write_all(s.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(15), self.stdout.read_line(&mut line))
            .await
            .expect("recv timeout")
            .expect("read line");
        assert!(n > 0, "agent EOF");
        serde_json::from_str(&line).expect("non-JSON line")
    }

    async fn recv_until<F: FnMut(&Value) -> bool>(&mut self, mut pred: F) -> Value {
        loop {
            let v = self.recv().await;
            if pred(&v) {
                return v;
            }
        }
    }

    async fn shutdown(mut self) {
        drop(self.stdin);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
        let _ = self.child.start_kill();
    }
}

fn openai_text(content: &str) -> Value {
    json!({
        "id": "cc-1", "object": "chat.completion", "model": "fake-model",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop",
        }],
    })
}

async fn init_session(h: &mut Harness, cwd: &str) -> String {
    h.send(
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .await;
    let _ = h.recv().await;
    h.send("session/new", json!({"cwd": cwd, "mcpServers": []}))
        .await;
    let r = h
        .recv_until(|v| v.get("result").is_some() || v.get("error").is_some())
        .await;
    r["result"]["sessionId"]
        .as_str()
        .expect("sessionId")
        .to_owned()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// AGENTS.md in cwd is loaded into the system prompt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hints_loaded_from_cwd_agents_md() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path();
    let marker = "BUZZ_HINTS_MARKER_42";
    std::fs::write(cwd.join("AGENTS.md"), marker).unwrap();

    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn_with_env(&llm.url, &[]).await;
    let sid = init_session(&mut h, cwd.to_str().unwrap()).await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;

    let captured = llm.captured.lock().await;
    assert!(!captured.is_empty(), "no LLM request captured");
    let system = captured[0]["messages"][0]["content"].as_str().unwrap_or("");
    assert!(
        system.contains(marker),
        "system prompt does not contain AGENTS.md marker: {system}"
    );
    h.shutdown().await;
}

/// BUZZ_AGENT_NO_HINTS=1 suppresses hint loading.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hints_suppressed_with_env_var() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path();
    let marker = "SUPPRESS_CHECK_MARKER_99";
    std::fs::write(cwd.join("AGENTS.md"), marker).unwrap();

    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn_with_env(&llm.url, &[("BUZZ_AGENT_NO_HINTS", "1")]).await;
    let sid = init_session(&mut h, cwd.to_str().unwrap()).await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;

    let captured = llm.captured.lock().await;
    assert!(!captured.is_empty(), "no LLM request captured");
    let system = captured[0]["messages"][0]["content"].as_str().unwrap_or("");
    assert!(
        !system.contains(marker),
        "system prompt should NOT contain marker when hints disabled: {system}"
    );
    h.shutdown().await;
}

/// SKILL.md files in .agents/skills/ are loaded into the system prompt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skills_loaded_from_agents_skills_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path();
    let skill_dir = cwd.join(".agents/skills/test-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\ndescription: A test skill\n---\nSKILL_BODY_MARKER_77\n",
    )
    .unwrap();

    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn_with_env(&llm.url, &[]).await;
    let sid = init_session(&mut h, cwd.to_str().unwrap()).await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;

    let captured = llm.captured.lock().await;
    assert!(!captured.is_empty(), "no LLM request captured");
    let system = captured[0]["messages"][0]["content"].as_str().unwrap_or("");
    assert!(
        system.contains("test-skill"),
        "system prompt missing skill name: {system}"
    );
    assert!(
        system.contains("SKILL_BODY_MARKER_77"),
        "system prompt missing skill body: {system}"
    );
    h.shutdown().await;
}

/// AGENTS.md files at git root and subdirectory are both loaded, root first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn git_root_hints_included() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    std::fs::create_dir(root.join(".git")).unwrap();
    std::fs::write(root.join("AGENTS.md"), "ROOT_HINT_MARKER_11").unwrap();
    let sub = root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("AGENTS.md"), "SUB_HINT_MARKER_22").unwrap();

    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h = Harness::spawn_with_env(&llm.url, &[]).await;
    let sid = init_session(&mut h, sub.to_str().unwrap()).await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;

    let captured = llm.captured.lock().await;
    assert!(!captured.is_empty(), "no LLM request captured");
    let system = captured[0]["messages"][0]["content"].as_str().unwrap_or("");
    assert!(
        system.contains("ROOT_HINT_MARKER_11"),
        "system prompt missing root hint: {system}"
    );
    assert!(
        system.contains("SUB_HINT_MARKER_22"),
        "system prompt missing sub hint: {system}"
    );
    let root_pos = system.find("ROOT_HINT_MARKER_11").unwrap();
    let sub_pos = system.find("SUB_HINT_MARKER_22").unwrap();
    assert!(
        root_pos < sub_pos,
        "root hint should appear before sub hint in system prompt"
    );
    h.shutdown().await;
}

/// ~/AGENTS.md (global) is loaded before CWD AGENTS.md when HOME is set.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_agents_md_loaded() {
    let home_tmp = tempfile::TempDir::new().unwrap();
    let cwd_tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(home_tmp.path().join("AGENTS.md"), "GLOBAL_HINT_MARKER_55").unwrap();
    std::fs::write(cwd_tmp.path().join("AGENTS.md"), "LOCAL_HINT_MARKER_66").unwrap();

    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h =
        Harness::spawn_with_env(&llm.url, &[("HOME", home_tmp.path().to_str().unwrap())]).await;
    let sid = init_session(&mut h, cwd_tmp.path().to_str().unwrap()).await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;

    let captured = llm.captured.lock().await;
    assert!(!captured.is_empty(), "no LLM request captured");
    let system = captured[0]["messages"][0]["content"].as_str().unwrap_or("");
    assert!(
        system.contains("GLOBAL_HINT_MARKER_55"),
        "system prompt missing global hint: {system}"
    );
    assert!(
        system.contains("LOCAL_HINT_MARKER_66"),
        "system prompt missing local hint: {system}"
    );
    let global_pos = system.find("GLOBAL_HINT_MARKER_55").unwrap();
    let local_pos = system.find("LOCAL_HINT_MARKER_66").unwrap();
    assert!(
        global_pos < local_pos,
        "global hint should appear before local hint in system prompt"
    );
    h.shutdown().await;
}

/// Global skills from ~/.agents/skills/ are loaded; project-level wins on name conflict.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_skills_loaded_and_project_wins() {
    let home_tmp = tempfile::TempDir::new().unwrap();
    let cwd_tmp = tempfile::TempDir::new().unwrap();

    let global_only_dir = home_tmp.path().join(".agents/skills/global-only");
    std::fs::create_dir_all(&global_only_dir).unwrap();
    std::fs::write(
        global_only_dir.join("SKILL.md"),
        "---\nname: global-only\ndescription: A global skill\n---\nGLOBAL_SKILL_BODY_88\n",
    )
    .unwrap();

    let global_shared_dir = home_tmp.path().join(".agents/skills/shared-name");
    std::fs::create_dir_all(&global_shared_dir).unwrap();
    std::fs::write(
        global_shared_dir.join("SKILL.md"),
        "---\nname: shared-name\ndescription: Global version\n---\nGLOBAL_SHARED_BODY_LOSE\n",
    )
    .unwrap();

    let project_shared_dir = cwd_tmp.path().join(".agents/skills/shared-name");
    std::fs::create_dir_all(&project_shared_dir).unwrap();
    std::fs::write(
        project_shared_dir.join("SKILL.md"),
        "---\nname: shared-name\ndescription: Project version\n---\nPROJECT_SHARED_BODY_WIN\n",
    )
    .unwrap();

    let llm = spawn_capturing_llm(vec![openai_text("done")]).await;
    let mut h =
        Harness::spawn_with_env(&llm.url, &[("HOME", home_tmp.path().to_str().unwrap())]).await;
    let sid = init_session(&mut h, cwd_tmp.path().to_str().unwrap()).await;

    let p = h
        .send(
            "session/prompt",
            json!({"sessionId": sid, "prompt": [{"type":"text","text":"go"}]}),
        )
        .await;
    let _ = h.recv_until(|v| v["id"] == json!(p)).await;

    let captured = llm.captured.lock().await;
    assert!(!captured.is_empty(), "no LLM request captured");
    let system = captured[0]["messages"][0]["content"].as_str().unwrap_or("");
    assert!(
        system.contains("global-only"),
        "system prompt missing global-only skill name: {system}"
    );
    assert!(
        system.contains("GLOBAL_SKILL_BODY_88"),
        "system prompt missing global-only skill body: {system}"
    );
    assert!(
        system.contains("PROJECT_SHARED_BODY_WIN"),
        "system prompt missing project skill body: {system}"
    );
    assert!(
        !system.contains("GLOBAL_SHARED_BODY_LOSE"),
        "system prompt should NOT contain shadowed global skill body: {system}"
    );
    h.shutdown().await;
}
