use std::process::Command;

#[test]
fn empty_mcp_config_environment_value_is_treated_as_unset() {
    let output = Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
        .env("BUZZ_ACP_MCP_CONFIG", "")
        .args(["--private-key", "not-a-valid-nostr-key"])
        .output()
        .expect("run buzz-acp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("configuration error: failed to parse nostr keys"),
        "empty optional MCP config should reach normal configuration validation: {stderr}"
    );
    assert!(
        !stderr.contains("a value is required for '--mcp-config"),
        "empty optional MCP config must not fail Clap parsing: {stderr}"
    );
}
