//! The real driver against scripted ACP agents: `/bin/sh` scripts stand in for
//! `buzz-agent` so each test controls the agent side of the wire exactly. The
//! scripts rely on absolute paths because the child environment has no PATH.
use super::*;
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;

const INIT_OK: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#;
const SESSION_OK: &str = r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#;
const PROMPT_DONE: &str = r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#;

fn chunk(session: &str, text: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session,
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": text },
            },
        },
    })
    .to_string()
}

struct Script {
    _dir: tempfile::TempDir,
    program: PathBuf,
    out: PathBuf,
}

/// Write an executable agent script. `$OUT` inside the body names a scratch
/// directory the test can inspect afterwards.
fn script(body: &str) -> Script {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();
    let program = dir.path().join("fake-agent");
    let text = format!(
        "#!/bin/sh\nOUT='{}'\n{}\n",
        out.display(),
        body.trim_start()
    );
    std::fs::write(&program, text).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    Script {
        _dir: dir,
        program,
        out,
    }
}

fn echo(line: &str) -> String {
    format!("printf '%s\\n' '{}'", line.replace('\'', r"'\''"))
}

fn launch(script: &Script) -> AcpLaunch {
    AcpLaunch {
        program: script.program.clone(),
        base_url: "http://127.0.0.1:11434/v1".into(),
        model: "local-model".into(),
    }
}

fn limits() -> AcpLimits {
    AcpLimits {
        deadline: Duration::from_secs(10),
        ..AcpLimits::default()
    }
}

async fn run(
    script: &Script,
    limits: &AcpLimits,
    cancel: &CancellationToken,
) -> Result<String, AcpRunError> {
    run_tool_free_prompt(&launch(script), "system rules", "user data", limits, cancel).await
}

fn process_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

fn read_pid(path: &Path) -> i32 {
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(pid) = text.trim().parse() {
                return pid;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("pid file {} never appeared", path.display());
}

fn wait_dead(pid: i32) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if !process_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// Agent that records each request line, then completes one turn.
fn happy_agent(reply_chunks: &[String]) -> Script {
    let chunks: Vec<String> = reply_chunks.iter().map(|c| echo(c)).collect();
    script(&format!(
        r#"
/usr/bin/env > "$OUT/env"
/bin/pwd > "$OUT/cwd"
read l1; printf '%s\n' "$l1" > "$OUT/initialize"; {init}
read l2; printf '%s\n' "$l2" > "$OUT/session_new"; {session}
read l3; printf '%s\n' "$l3" > "$OUT/prompt"
{chunks}
{done}
"#,
        init = echo(INIT_OK),
        session = echo(SESSION_OK),
        chunks = chunks.join("\n"),
        done = echo(PROMPT_DONE),
    ))
}

#[tokio::test]
async fn returns_the_streamed_reply_for_this_session_only() {
    let agent = happy_agent(&[
        chunk("s1", "{\"points\":"),
        chunk("other-session", "INJECTED"),
        chunk("s1", "[]}"),
    ]);
    let text = run(&agent, &limits(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(text, "{\"points\":[]}");
}

#[tokio::test]
async fn child_gets_a_cleared_environment_isolated_cwd_and_no_tools() {
    let agent = happy_agent(&[chunk("s1", "ok")]);
    run(&agent, &limits(), &CancellationToken::new())
        .await
        .unwrap();

    let env = std::fs::read_to_string(agent.out.join("env")).unwrap();
    let vars: std::collections::HashMap<&str, &str> = env
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    assert_eq!(vars.get("BUZZ_AGENT_LOCAL_ONLY"), Some(&"1"));
    assert_eq!(vars.get("BUZZ_AGENT_LOCAL_JSON_OUTPUT"), Some(&"1"));
    assert_eq!(vars.get("BUZZ_AGENT_THINKING_EFFORT"), Some(&"none"));
    assert_eq!(
        vars.get("OPENAI_COMPAT_BASE_URL"),
        Some(&"http://127.0.0.1:11434/v1")
    );
    assert_eq!(vars.get("BUZZ_AGENT_MODEL"), Some(&"local-model"));
    assert_eq!(vars.get("BUZZ_AGENT_NO_HINTS"), Some(&"1"));
    assert_eq!(vars.get("BUZZ_AGENT_MAX_ROUNDS"), Some(&"1"));
    // Nothing from the parent survives: the test process always has PATH.
    assert!(std::env::var_os("PATH").is_some());
    for inherited in [
        "PATH",
        "USER",
        "OPENAI_COMPAT_API_KEY",
        "ANTHROPIC_API_KEY",
        "HTTPS_PROXY",
    ] {
        assert!(
            !vars.contains_key(inherited),
            "{inherited} leaked into the child"
        );
    }
    let expected: std::collections::HashSet<String> =
        child_env(&launch(&agent), &limits(), Path::new(vars["HOME"]))
            .into_iter()
            .map(|(key, _)| key)
            .collect();
    let actual: std::collections::HashSet<String> = vars
        .keys()
        .map(|key| key.to_string())
        // /bin/sh itself exports these.
        .filter(|key| {
            !matches!(
                key.as_str(),
                "PWD" | "SHLVL" | "_" | "OLDPWD" | "__CF_USER_TEXT_ENCODING"
            )
        })
        .collect();
    assert_eq!(actual, expected);

    // The working directory is a fresh private folder, also HOME, and is
    // removed after the run.
    let cwd = std::fs::read_to_string(agent.out.join("cwd")).unwrap();
    let cwd = cwd.trim();
    assert_eq!(vars["HOME"], cwd);
    assert!(cwd.contains("buzz-catch-up-"), "{cwd}");
    assert!(!Path::new(cwd).exists(), "work folder must be removed");

    let session_new: Value =
        serde_json::from_str(&std::fs::read_to_string(agent.out.join("session_new")).unwrap())
            .unwrap();
    assert_eq!(session_new["method"], "session/new");
    assert_eq!(session_new["params"]["mcpServers"], json!([]));
    assert_eq!(session_new["params"]["cwd"], cwd);
    assert_eq!(session_new["params"]["systemPrompt"], "system rules");

    let prompt: Value =
        serde_json::from_str(&std::fs::read_to_string(agent.out.join("prompt")).unwrap()).unwrap();
    assert_eq!(prompt["params"]["sessionId"], "s1");
    assert_eq!(prompt["params"]["prompt"][0]["text"], "user data");
}

#[tokio::test]
async fn permission_requests_are_cancelled_and_other_requests_refused() {
    let permission = r#"{"jsonrpc":"2.0","id":"perm-1","method":"session/request_permission","params":{"sessionId":"s1"}}"#;
    let read_file = r#"{"jsonrpc":"2.0","id":"fs-1","method":"fs/read_text_file","params":{"path":"/etc/passwd"}}"#;
    let agent = script(&format!(
        r#"
read l1; {init}
read l2; {session}
read l3
{permission}
read answer1; printf '%s\n' "$answer1" > "$OUT/answer1"
{read_file}
read answer2; printf '%s\n' "$answer2" > "$OUT/answer2"
{chunk}
{done}
"#,
        init = echo(INIT_OK),
        session = echo(SESSION_OK),
        permission = echo(permission),
        read_file = echo(read_file),
        chunk = echo(&chunk("s1", "done")),
        done = echo(PROMPT_DONE),
    ));
    let text = run(&agent, &limits(), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(text, "done");

    let answer1: Value =
        serde_json::from_str(&std::fs::read_to_string(agent.out.join("answer1")).unwrap()).unwrap();
    assert_eq!(answer1["id"], "perm-1");
    assert_eq!(answer1["result"]["outcome"]["outcome"], "cancelled");
    let answer2: Value =
        serde_json::from_str(&std::fs::read_to_string(agent.out.join("answer2")).unwrap()).unwrap();
    assert_eq!(answer2["id"], "fs-1");
    assert!(answer2["error"].is_object());
    assert!(answer2.get("result").is_none());
}

/// Agent that backgrounds a grandchild, records both pids, then hangs.
fn hanging_agent() -> Script {
    script(&format!(
        r#"
/bin/sleep 60 &
printf '%s\n' "$!" > "$OUT/grandchild"
printf '%s\n' "$$" > "$OUT/agent"
read l1; {init}
read l2; {session}
read l3
/bin/sleep 60
"#,
        init = echo(INIT_OK),
        session = echo(SESSION_OK),
    ))
}

#[tokio::test]
async fn cancellation_kills_the_whole_process_tree() {
    let agent = hanging_agent();
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let grandchild_file = agent.out.join("grandchild");
    let agent_file = agent.out.join("agent");
    let pids = tokio::task::spawn_blocking(move || {
        let pids = (read_pid(&agent_file), read_pid(&grandchild_file));
        std::thread::sleep(Duration::from_millis(200));
        trigger.cancel();
        pids
    });
    let started = Instant::now();
    let result = run(&agent, &limits(), &cancel).await;
    assert_eq!(result, Err(AcpRunError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(5));
    let (agent_pid, grandchild_pid) = pids.await.unwrap();
    assert!(wait_dead(agent_pid), "agent survived cancellation");
    assert!(
        wait_dead(grandchild_pid),
        "grandchild survived cancellation"
    );
}

#[tokio::test]
async fn deadline_kills_the_whole_process_tree() {
    let agent = hanging_agent();
    let limits = AcpLimits {
        deadline: Duration::from_millis(600),
        ..AcpLimits::default()
    };
    let result = run(&agent, &limits, &CancellationToken::new()).await;
    assert_eq!(result, Err(AcpRunError::TimedOut));
    let agent_pid = read_pid(&agent.out.join("agent"));
    let grandchild_pid = read_pid(&agent.out.join("grandchild"));
    assert!(wait_dead(agent_pid), "agent survived the deadline");
    assert!(
        wait_dead(grandchild_pid),
        "grandchild survived the deadline"
    );
}

#[tokio::test]
async fn already_cancelled_requests_never_spawn() {
    let agent = happy_agent(&[chunk("s1", "ok")]);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        run(&agent, &limits(), &cancel).await,
        Err(AcpRunError::Cancelled)
    );
    assert!(
        !agent.out.join("env").exists(),
        "the agent must not have run"
    );
}

#[tokio::test]
async fn reply_line_and_total_output_are_bounded() {
    let small_reply = AcpLimits {
        max_reply_bytes: 16,
        ..limits()
    };
    let agent = happy_agent(&[chunk("s1", "0123456789"), chunk("s1", "0123456789")]);
    let error = run(&agent, &small_reply, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AcpRunError::Failed(m) if m.contains("size limit")),
        "{error:?}"
    );

    let short_lines = AcpLimits {
        max_line_bytes: 120,
        ..limits()
    };
    let agent = happy_agent(&[chunk("s1", &"x".repeat(200))]);
    let error = run(&agent, &short_lines, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AcpRunError::Failed(m) if m.contains("size limit")),
        "{error:?}"
    );

    let small_stdout = AcpLimits {
        max_stdout_bytes: 150,
        ..limits()
    };
    let agent = happy_agent(&[chunk("s1", "a"), chunk("s1", "b"), chunk("s1", "c")]);
    let error = run(&agent, &small_stdout, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AcpRunError::Failed(m) if m.contains("size limit")),
        "{error:?}"
    );
}

#[tokio::test]
async fn non_end_turn_stops_and_empty_replies_fail() {
    for (stop, expected) in [
        ("max_tokens", "cut off"),
        ("max_turn_requests", "tools"),
        ("refusal", "stopped unexpectedly"),
    ] {
        let done = format!(r#"{{"jsonrpc":"2.0","id":3,"result":{{"stopReason":"{stop}"}}}}"#);
        let agent = script(&format!(
            "read l1; {}\nread l2; {}\nread l3; {}\n{}\n",
            echo(INIT_OK),
            echo(SESSION_OK),
            echo(&chunk("s1", "partial")),
            echo(&done)
        ));
        let error = run(&agent, &limits(), &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AcpRunError::Failed(m) if m.contains(expected)),
            "{stop}: {error:?}"
        );
    }

    let agent = happy_agent(&[]);
    let error = run(&agent, &limits(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AcpRunError::Failed(m) if m.contains("no reply")),
        "{error:?}"
    );
}

#[tokio::test]
async fn startup_failure_surfaces_the_agents_last_stderr_line() {
    let agent = script(
        "printf '%s\\n' 'config: OPENAI_COMPAT_API_KEY must not be set in local-only mode' >&2\nexit 1",
    );
    let error = run(&agent, &limits(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AcpRunError::Failed(m) if m.contains("must not be set in local-only mode")),
        "{error:?}"
    );
}

#[tokio::test]
async fn agent_error_responses_fail_the_run() {
    let agent = script(&format!(
        "read l1; {}\nread l2; {}\n",
        echo(INIT_OK),
        echo(
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"session/new: cwd must be an absolute path"}}"#
        ),
    ));
    let error = run(&agent, &limits(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AcpRunError::Failed(m) if m.contains("cwd must be")),
        "{error:?}"
    );
}
