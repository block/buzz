//! Integration tests for `buzz capabilities` — the machine-readable
//! security-contract report consumed by downstream automated verifiers.
//!
//! These spawn the real `buzz` binary as a subprocess (rather than calling
//! the library function in-process) so they also exercise CLI dispatch: that
//! `capabilities` is reachable, exits 0, and is unaffected by env vars,
//! exactly as an external doctor tool would observe it.

use std::process::Command;

const SYNTHETIC_NSEC: &str = "nsec1synthetic-do-not-use-0000000000000000000000000000000000";

fn run_capabilities(env_private_key: Option<&str>) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_buzz"));
    cmd.arg("capabilities");
    if let Some(key) = env_private_key {
        cmd.env("BUZZ_PRIVATE_KEY", key);
    } else {
        cmd.env_remove("BUZZ_PRIVATE_KEY");
    }
    let output = cmd.output().expect("failed to spawn buzz binary");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

fn expected_json() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "capability_revision": "secure_private_key_fd_v1",
        "private_key_fd": {
            "supported": cfg!(unix),
            "transport": "inherited_fd",
            "min_fd": 3,
            "max_fd": 1024,
            "max_key_bytes": 256,
            "closes_original_fd": true,
            "zeroizes_input": true,
            "help_env_value_redacted": true,
        },
    })
}

#[test]
fn capabilities_without_private_key_env_returns_expected_json() {
    let (code, stdout, stderr) = run_capabilities(None);
    assert_eq!(code, 0, "expected exit 0, stderr: {stderr}");

    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");
    assert_eq!(parsed, expected_json(), "unexpected capabilities JSON");
}

#[test]
fn capabilities_ignores_private_key_env_and_leaks_nothing() {
    let (code, stdout, stderr) = run_capabilities(Some(SYNTHETIC_NSEC));
    assert_eq!(code, 0, "expected exit 0, stderr: {stderr}");

    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");
    assert_eq!(
        parsed,
        expected_json(),
        "BUZZ_PRIVATE_KEY must have zero effect on capabilities output"
    );
    assert!(
        !stdout.contains(SYNTHETIC_NSEC),
        "capabilities stdout leaked the synthetic private key:\n{stdout}"
    );
    assert!(
        !stderr.contains(SYNTHETIC_NSEC),
        "capabilities stderr leaked the synthetic private key:\n{stderr}"
    );
}

#[test]
#[cfg(unix)]
fn private_key_fd_is_supported_on_unix() {
    let (code, stdout, stderr) = run_capabilities(None);
    assert_eq!(code, 0, "expected exit 0, stderr: {stderr}");
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be valid JSON");
    assert_eq!(parsed["private_key_fd"]["supported"], true);
}
