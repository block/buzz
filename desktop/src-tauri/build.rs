// Shared schema, included from the same source the runtime command parses with,
// so the build-time validation below and the runtime parse cannot drift.
include!("src/commands/reconnect_hook_config.rs");

use base64::Engine as _;

fn main() {
    println!("cargo:rerun-if-env-changed=BUZZ_RELAY_URL");
    println!("cargo:rerun-if-env-changed=BUZZ_RELAY_HTTP");
    println!("cargo:rerun-if-env-changed=BUZZ_UPDATER_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=BUZZ_UPDATER_ENDPOINT");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_BUZZ_AGENT_PROVIDER");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_BUZZ_AGENT_MODEL");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_AGENT_ENV");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_RELAY_RECONNECT_CMD");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT");
    println!("cargo:rustc-check-cfg=cfg(buzz_updater_enabled)");

    if let Ok(relay_url) = std::env::var("BUZZ_RELAY_URL") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_RELAY_URL={relay_url}");
    }

    if let Ok(relay_http) = std::env::var("BUZZ_RELAY_HTTP") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_RELAY_HTTP={relay_http}");
    }

    if let Ok(provider) = std::env::var("BUZZ_BUILD_BUZZ_AGENT_PROVIDER") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_BUZZ_AGENT_PROVIDER={provider}");
    }

    if let Ok(model) = std::env::var("BUZZ_BUILD_BUZZ_AGENT_MODEL") {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_BUZZ_AGENT_MODEL={model}");
    }

    // Generic KEY=VALUE pairs to inject into every spawned agent process.
    // Newline-delimited; each line must be non-empty and contain exactly one
    // `=` separator with a non-empty key.  OSS builds leave this unset.
    // The validated value is base64-encoded before emitting so the single-line
    // Cargo build-script output carries all pairs (Cargo output is line-oriented;
    // a raw multiline value would be silently truncated to the first line).
    if let Ok(raw) = std::env::var("BUZZ_BUILD_AGENT_ENV") {
        for (line_no, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let eq = line.find('=').unwrap_or_else(|| {
                panic!(
                    "BUZZ_BUILD_AGENT_ENV line {}: missing '=' separator in {:?}",
                    line_no + 1,
                    line
                )
            });
            let key = &line[..eq];
            if key.is_empty() {
                panic!(
                    "BUZZ_BUILD_AGENT_ENV line {}: key must not be empty in {:?}",
                    line_no + 1,
                    line
                );
            }
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_AGENT_ENV={encoded}");
    }

    if let Ok(val) = std::env::var("BUZZ_BUILD_RELAY_RECONNECT_CMD") {
        let parsed: serde_json::Value = serde_json::from_str(&val)
            .unwrap_or_else(|e| panic!("BUZZ_BUILD_RELAY_RECONNECT_CMD is not valid JSON: {e}"));
        serde_json::from_value::<ReconnectHookConfig>(parsed).unwrap_or_else(|e| {
            panic!("BUZZ_BUILD_RELAY_RECONNECT_CMD doesn't match ReconnectHookConfig: {e}")
        });
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_RELAY_RECONNECT_CMD={val}");
    }

    // Presence-only flag: when set (any non-empty value), observer-feed archive
    // defaults to ON for the current identity on first run.  OSS builds leave
    // this unset → default OFF.  No JSON validation needed — the command only
    // checks `.is_some()`.
    if std::env::var("BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT").is_ok() {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_OBSERVER_ARCHIVE_DEFAULT=1");
    }

    let updater_public_key = std::env::var("BUZZ_UPDATER_PUBLIC_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let updater_endpoint = std::env::var("BUZZ_UPDATER_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if updater_public_key.is_some() && updater_endpoint.is_some() {
        println!("cargo:rustc-cfg=buzz_updater_enabled");
    }

    tauri_build::build()
}
