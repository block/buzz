//! Exercise the executable entry point, not a test copy of its argument logic.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use uuid::Uuid;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let result =
            Self(std::env::temp_dir().join(format!("buzz-pi-native-test-{}", Uuid::new_v4())));
        fs::create_dir(&result.0).unwrap();
        result
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
        command
            .arg("--internal-pi-launch")
            .arg(&self.0)
            .current_dir(&self.0);
        command
    }
    fn fake_pi(&self) {
        let script = self.0.join("pi");
        fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        fs::set_permissions(script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn snapshot(&self, name: &str) -> PathBuf {
        let token = Uuid::new_v4();
        let path = self.0.join(format!("{token}.md"));
        fs::write(&path, name).unwrap();
        fs::write(self.0.join("pending"), token.to_string()).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn native_pi_exec_forwards_arguments_and_selects_original_restore_snapshot() {
    let fixture = Fixture::new();
    fixture.fake_pi();
    let original = fixture.snapshot("<base>first</base>");
    let output = fixture
        .command()
        .env("PATH", &fixture.0)
        .args(["--mode", "rpc", "--no-themes"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        args,
        format!(
            "--system-prompt\n{}\n--skill\n{}\n--mode\nrpc\n--no-themes\n",
            original.display(),
            fs::canonicalize(&fixture.0)
                .unwrap()
                .join(".agents/skills")
                .display()
        )
    );

    let id = Uuid::new_v4();
    fs::write(
        fixture.0.join(format!("session-{id}")),
        original.file_stem().unwrap().to_str().unwrap(),
    )
    .unwrap();
    let session = fixture.0.join("arbitrary-name.jsonl");
    fs::write(
        &session,
        format!("{{\"type\":\"session\",\"id\":\"{id}\"}}\n"),
    )
    .unwrap();
    fixture.snapshot("<base>another session</base>");
    let output = fixture
        .command()
        .env("PATH", &fixture.0)
        .args(["--mode", "rpc", "--session"])
        .arg(&session)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stdout)
        .unwrap()
        .starts_with(&format!("--system-prompt\n{}\n", original.display())));
    fs::remove_file(&original).unwrap();
    let output = fixture
        .command()
        .env("PATH", &fixture.0)
        .args(["--mode", "rpc", "--session"])
        .arg(&session)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("snapshot is missing"));
}

#[test]
fn native_pi_terminal_login_does_not_require_a_session_prompt() {
    let fixture = Fixture::new();
    fixture.fake_pi();
    let output = fixture.command().env("PATH", &fixture.0).output().unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8(output.stdout)
        .unwrap()
        .contains("--system-prompt"));
}

/// No model calls: inspect Pi's actual live state through its RPC HTML export.
/// Run with Pi on PATH: cargo test -p buzz-acp --test pi_native_launcher -- --ignored
#[test]
#[ignore = "requires the Pi CLI on PATH"]
fn real_pi_exports_the_native_system_prompt() {
    use base64::Engine;
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let fixture = Fixture::new();
    let prompt = "<base>\nplatform test\n</base>\n\n<agent-instructions>\nprofile test\n</agent-instructions>\n\n<core-memory>\nmemory test\n</core-memory>";
    let snapshot = fixture.snapshot(prompt);
    let session_id = Uuid::new_v4();
    fs::write(
        fixture.0.join(format!("session-{session_id}")),
        snapshot.file_stem().unwrap().to_str().unwrap(),
    )
    .unwrap();
    let transcript = fixture.0.join("session.jsonl");
    let timestamp = "2026-01-01T00:00:00.000Z";
    fs::write(&transcript, format!("{}\n{}\n",
        serde_json::json!({"type":"session", "version":3, "id":session_id, "timestamp":timestamp, "cwd":fixture.0}),
        serde_json::json!({"type":"message", "id":"00000001", "parentId":null, "timestamp":timestamp,
            "message":{"role":"user", "content":[{"type":"text", "text":"<context>\nfixture turn\n</context>"}], "timestamp":1767225600000u64}})
    )).unwrap();
    let export = fixture.0.join("live.html");
    let mut child = fixture
        .command()
        .env("PI_CODING_AGENT_DIR", fixture.0.join("agent"))
        .args([
            "--mode",
            "rpc",
            "--offline",
            "--no-extensions",
            "--no-context-files",
            "--no-skills",
        ])
        .arg("--session")
        .arg(&transcript)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().take(100) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    writeln!(
        stdin,
        "{}",
        serde_json::json!({"id":"export-test", "type":"export_html", "outputPath":export})
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(line)) => {
                let value: serde_json::Value = serde_json::from_str(&line).unwrap();
                if value["id"] == "export-test" {
                    break Some(value);
                }
            }
            _ => break None,
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    drop(rx);
    reader.join().unwrap();
    let result = result.expect("Pi did not answer export_html within 30 seconds");
    assert_eq!(result["success"], true, "{result}");
    let html = fs::read_to_string(export).unwrap();
    let start = html.find("id=\"session-data\"").unwrap();
    let data = html[start..]
        .split_once('>')
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0
        .trim();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    let data: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
    let actual = data["systemPrompt"].as_str().unwrap();
    assert!(actual.starts_with(prompt), "{actual}");
    for tag in ["<base>", "<agent-instructions>", "<core-memory>"] {
        assert_eq!(actual.matches(tag).count(), 1);
    }
}
