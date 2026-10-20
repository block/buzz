//! Process-level coverage for the `messages send` empty-content guard.
//!
//! The guard fires before any relay call, so these tests need no
//! infrastructure. They bind the production wiring end to end: argv parsing,
//! the `--content -` sentinel, the guard, and the exit-code/stderr contract.
//! Replacing the guard's sentinel derivation with a constant, or removing the
//! guard, fails at least one of these.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `buzz messages send --channel <uuid> --content -` with `stdin_body`.
///
/// Returns the exit code and stderr. The relay URL is deliberately
/// unreachable: a send that clears the guard must fail at the network layer,
/// not silently succeed.
fn send_with_stdin(stdin_body: &str, extra: &[&str]) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .args([
            "messages",
            "send",
            "--channel",
            "123e4567-e89b-12d3-a456-426614174000",
            "--content",
            "-",
        ])
        .args(extra)
        .env("BUZZ_RELAY_URL", "http://127.0.0.1:9")
        // A syntactically valid secp256k1 secret key; the zero scalar is not.
        .env("BUZZ_PRIVATE_KEY", format!("{}1", "0".repeat(63)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the buzz binary");

    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin_body.as_bytes())
        .expect("write stdin");
    // Dropping the handle closes the pipe so the child sees EOF.

    let output = child.wait_with_output().expect("wait for buzz");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn empty_stdin_exits_1_with_the_stdin_diagnosis() {
    let (code, stderr) = send_with_stdin("\n", &[]);
    assert_eq!(code, Some(1), "expected exit 1, stderr: {stderr}");
    assert!(
        stderr.contains("upstream pipeline step"),
        "the stdin path must name the likely cause, got: {stderr}"
    );
}

#[test]
fn whitespace_only_stdin_exits_1() {
    let (code, stderr) = send_with_stdin("   ", &[]);
    assert_eq!(code, Some(1), "expected exit 1, stderr: {stderr}");
}

#[test]
fn allow_empty_passes_the_guard_and_reaches_the_relay() {
    let (code, stderr) = send_with_stdin("", &["--allow-empty"]);
    assert_eq!(
        code,
        Some(2),
        "the send must clear the guard and fail at the network layer, stderr: {stderr}"
    );
    assert!(
        !stderr.contains("upstream pipeline step"),
        "--allow-empty must not produce the empty-content diagnosis: {stderr}"
    );
}
