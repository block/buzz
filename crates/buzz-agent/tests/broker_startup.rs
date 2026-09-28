//! The real executable consumes broker startup before ACP or provider creation.
use serde_json::json;
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn agent() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_buzz-agent"));
    cmd.env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", std::env::var_os("HOME").unwrap_or_default())
        .env("BUZZ_AGENT_PROVIDER", "openai-compat")
        .env("OPENAI_COMPAT_BASE_URL", "http://127.0.0.1:1")
        .env("OPENAI_COMPAT_API_KEY", "synthetic")
        .env("BUZZ_AGENT_MODEL", "synthetic-model")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn frame() -> Vec<u8> {
    let data = serde_json::to_vec(&json!({"port":1234,
        "secret":"synthetic-broker-capability-at-least-32-chars", "host":"https://model.example"}))
    .unwrap();
    let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
    bytes.extend(data);
    bytes
}

#[test]
fn private_bootstrap_preserves_real_acp_initialize() {
    let mut cmd = agent();
    cmd.env("BUZZ_SANDBOX_AUTH_BROKER", "stdin-v1");
    let mut child = cmd.spawn().unwrap();
    let mut input = frame();
    input.extend(
        serde_json::to_vec(&json!({"jsonrpc":"2.0", "id":1,
        "method":"initialize", "params":{"protocolVersion":1,"clientCapabilities":{}}}))
        .unwrap(),
    );
    input.push(b'\n');
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let messages: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        messages
            .iter()
            .any(|v| v["id"] == 1 && v.get("result").is_some()),
        "{messages:?}"
    );
}

#[test]
fn legacy_environment_and_truncated_bootstrap_fail_closed() {
    for (marker, input, expected) in [
        (
            r#"{"secret":"private-legacy-secret"}"#,
            vec![],
            "must be delivered through stdin-v1",
        ),
        ("stdin-v1", vec![0, 0], "invalid broker startup frame"),
    ] {
        let mut cmd = agent();
        cmd.env("BUZZ_SANDBOX_AUTH_BROKER", marker);
        let mut child = cmd.spawn().unwrap();
        child.stdin.take().unwrap().write_all(&input).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("private-legacy-secret"));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn initialized_broker_still_rejects_wrong_provider_host() {
    let mut cmd = agent();
    cmd.env("BUZZ_SANDBOX_AUTH_BROKER", "stdin-v1")
        .env("BUZZ_AGENT_PROVIDER", "databricks")
        .env("DATABRICKS_HOST", "https://other.example");
    let mut child = cmd.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&frame()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("broker provider mismatch"));
}
