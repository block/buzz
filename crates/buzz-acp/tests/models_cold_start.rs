//! Exercise the shipped models subcommand across the old cold-start cutoff.
#![cfg(unix)]

use std::{path::PathBuf, process::Command};

#[test]
fn models_probe_accepts_cold_start_longer_than_ten_seconds() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    for (key, _) in std::env::vars().filter(|(key, _)| key.starts_with("BUZZ_")) {
        command.env_remove(key);
    }
    let output = command
        .args([
            "models",
            "--json",
            "--agent-command",
            "python3",
            "--agent-args",
        ])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cold_models_agent.py"))
        .output()
        .expect("models command starts");
    assert!(
        output.status.success(),
        "cold-start discovery failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("models returns JSON");
    assert_eq!(response["agent"]["name"], "cold-probe");
    assert_eq!(response["unstable"]["currentModelId"], "cold-model");
    assert_eq!(
        response["unstable"]["availableModels"][0]["modelId"],
        "cold-model"
    );
}
