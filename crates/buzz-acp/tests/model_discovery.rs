//! Exercise the real CLI/ACP exchange without login or a relay.
#![cfg(unix)]
use std::process::Command;

fn discover(model: Option<&str>, mode: &str) -> serde_json::Value {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/model_discovery.py"
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_buzz-acp"));
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("BUZZ_ACP_AGENT_COMMAND", "python3")
        .env("BUZZ_ACP_AGENT_ARGS", fixture)
        .env("CATALOG_FIXTURE_MODE", mode)
        .args(["models", "--json"]);
    if let Some(model) = model {
        command.args(["--model", model]);
    }
    let output = command.output().expect("start model discovery");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["stable"]["configOptions"][0]["options"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    json
}

#[test]
fn retired_model_keeps_replacement_catalog_without_default_effort() {
    let result = discover(Some("retired-model"), "ok");
    assert!(result["stable"]["effortOption"].is_null());
}

#[test]
fn rejected_model_keeps_replacement_catalog_without_default_effort() {
    let result = discover(Some("model-b"), "reject");
    assert!(result["stable"]["effortOption"].is_null());
}

#[test]
fn switch_without_options_does_not_reuse_default_model_effort() {
    let result = discover(Some("model-b"), "no-options");
    assert!(result["stable"]["effortOption"].is_null());
}

#[test]
fn selected_model_returns_its_own_effort_and_initial_probe_still_works() {
    let initial = discover(None, "ok");
    assert_eq!(initial["stable"]["effortOption"]["currentValue"], "medium");
    let selected = discover(Some("model-b"), "ok");
    assert_eq!(
        selected["stable"]["configOptions"][0]["currentValue"],
        "model-b"
    );
    assert_eq!(selected["stable"]["effortOption"]["currentValue"], "high");
}
