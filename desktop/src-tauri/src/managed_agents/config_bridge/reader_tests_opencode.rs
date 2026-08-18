//! OpenCode config-bridge tests — split out of `reader_tests.rs` to keep it
//! under the 1000-line file-size ratchet.
//!
//! Included as `mod opencode_tests` inside `reader_tests.rs`, so `use super::*`
//! gives access to all helpers and types from that module.

use super::*;

static OPENCODE_CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn with_opencode_config<T>(path: &Path, body: impl FnOnce() -> T) -> T {
    let _guard = OPENCODE_CONFIG_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let prior = std::env::var_os("OPENCODE_CONFIG");
    std::env::set_var("OPENCODE_CONFIG", path);
    let output = body();
    match prior {
        Some(value) => std::env::set_var("OPENCODE_CONFIG", value),
        None => std::env::remove_var("OPENCODE_CONFIG"),
    }
    output
}

/// End-to-end wiring guard for the whole point of the OpenCode entry: the
/// harness takes no `--model` flag and reads no model env var, so unless the
/// bridge reaches its config file the model field is blank in the panel.
#[test]
fn opencode_surface_takes_its_model_from_the_config_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = dir.path().join("opencode.jsonc");
    std::fs::write(
        &config,
        r#"{
            // real OpenCode configs are JSONC with comments and trailing commas
            "$schema": "https://opencode.ai/config.json",
            "model": "anthropic/claude-sonnet-4-5",
            "mcp": { "filesystem": { "type": "local" } },
        }"#,
    )
    .expect("write config");

    let record = test_record();
    let runtime = &KnownAcpRuntime {
        id: "opencode",
        label: "OpenCode",
        commands: &["opencode"],
        model_env_var: None,
        provider_env_var: None,
        supports_acp_native_config: false,
        thinking_env_var: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        required_normalized_fields: &[],
        config_file_path: Some("~/.config/opencode/opencode.json"),
        config_file_format: Some("json"),
        ..*test_runtime()
    };

    let surface = with_opencode_config(&config, || {
        read_config_surface(&record, Some(runtime), None, &no_tiers(), None)
    });

    let model = surface.normalized.model.expect("model field");
    assert_eq!(model.value.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(model.origin, ConfigOrigin::ConfigFile);
    // Nothing can write it back — no env var, no ACP model switching.
    assert!(matches!(model.write_via, ConfigWriteMechanism::ReadOnly));

    let provider = surface.normalized.provider.expect("provider field");
    assert_eq!(provider.value.as_deref(), Some("anthropic"));

    assert_eq!(surface.sources.config_file, ConfigTierStatus::Available);
    assert_eq!(
        surface.sources.config_file_path.as_deref().map(Path::new),
        Some(config.as_path()),
        "the reported path must be the file actually read, not the static default"
    );
    assert_eq!(
        surface
            .extensions
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>(),
        vec!["filesystem"]
    );
}
