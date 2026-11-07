//! `buzz-acp stop-hook`, the command the Claude Stop hook runs.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run_hook(dir: Option<&Path>, stdin: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    command.arg("stop-hook");
    if let Some(dir) = dir {
        command.arg("--dir").arg(dir);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn buzz-acp stop-hook");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write hook input");
    child.wait_with_output().expect("wait for stop-hook")
}

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("buzz-acp-stop-hook-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// The hook must exit 0 and print nothing: Claude Code treats a failing or
/// talkative Stop hook as an instruction.
fn assert_silent_success(output: &Output) {
    assert!(output.status.success(), "exit status {:?}", output.status);
    assert!(
        output.stdout.is_empty(),
        "stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn stop_hook_records_the_session_crons_silently() {
    let dir = temp_dir();
    let input = r#"{"session_id":"abc-1","hook_event_name":"Stop","session_crons":[{"id":"x","schedule":"0 9 * * *","recurring":true,"prompt":"p"}]}"#;
    let output = run_hook(Some(&dir), input);
    assert_silent_success(&output);
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("abc-1.json")).unwrap()).unwrap();
    assert_eq!(record["session_crons"][0]["schedule"], "0 9 * * *");

    let output = run_hook(Some(&dir), "not json");
    assert_silent_success(&output);
    let errors = std::fs::read_to_string(dir.join("stop-hook-errors.log")).unwrap();
    assert!(!errors.is_empty(), "malformed input left no trace");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stop_hook_without_a_dir_does_nothing_silently() {
    assert_silent_success(&run_hook(None, r#"{"session_id":"abc-1"}"#));
}
