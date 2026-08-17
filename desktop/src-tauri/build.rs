// Shared schema, included from the same source the runtime command parses with,
// so the build-time validation below and the runtime parse cannot drift.
include!("src/commands/reconnect_hook_config.rs");
// Same source of truth the runtime filters with, so a baked build env cannot
// carry a reserved key the runtime believes it already rejected.
include!("src/managed_agents/reserved_env_keys.rs");
include!("src/managed_agents/binary_identity.rs");

use base64::Engine as _;

const REQUIRE_HEARTBEAT_SIDECAR_ENV: &str = "BUZZ_BUILD_REQUIRE_HEARTBEAT_PREFLIGHT_SIDECAR";
const HEARTBEAT_MACOS_TEAM_ENV: &str = "BUZZ_BUILD_HEARTBEAT_HARNESS_MACOS_TEAM_IDENTIFIER";
const SOURCE_REVISION_ENV: &str = "BUZZ_BUILD_SOURCE_REVISION";
const HEARTBEAT_CAPABILITY_COMMAND: &str = "heartbeat-preflight-capability";
const HEARTBEAT_CAPABILITY_KIND: &str = "buzz_acp_heartbeat_preflight_capability";
const HEARTBEAT_CAPABILITY_PROTOCOL_VERSION: u32 = 1;
const HEARTBEAT_BUILD_CAPABILITY: &str = "buzz-acp-source-witness-gateway-v1";

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct HeartbeatCapability {
    kind: String,
    protocol_version: u32,
    build_capability: String,
}

fn exact_heartbeat_capability() -> HeartbeatCapability {
    HeartbeatCapability {
        kind: HEARTBEAT_CAPABILITY_KIND.to_string(),
        protocol_version: HEARTBEAT_CAPABILITY_PROTOCOL_VERSION,
        build_capability: HEARTBEAT_BUILD_CAPABILITY.to_string(),
    }
}

fn read_heartbeat_capability(path: &std::path::Path) -> Result<HeartbeatCapability, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect heartbeat capability attestation: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(
            "heartbeat capability attestation must be a non-empty regular non-symlink file".into(),
        );
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read heartbeat capability attestation: {error}"))?;
    if bytes.len() > 4_096 {
        return Err("heartbeat capability attestation exceeds 4 KiB".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("heartbeat capability attestation is invalid: {error}"))
}

fn probe_heartbeat_capability(path: &std::path::Path) -> Result<HeartbeatCapability, String> {
    use std::process::{Command, Stdio};

    let mut child = Command::new(path)
        .arg(HEARTBEAT_CAPABILITY_COMMAND)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot run packaged buzz-acp capability probe: {error}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("cannot wait for packaged buzz-acp probe: {error}"))?
            .is_some()
        {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("packaged buzz-acp capability probe timed out".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot read packaged buzz-acp probe: {error}"))?;
    if !output.status.success() || output.stdout.len() > 4_096 || !output.stderr.is_empty() {
        return Err("packaged buzz-acp capability probe failed closed".into());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("packaged buzz-acp capability is invalid: {error}"))
}

fn verify_packaged_heartbeat_sidecar(
    path: &std::path::Path,
    attestation_path: &std::path::Path,
    target: &str,
) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect packaged buzz-acp: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err("packaged buzz-acp must be a non-empty regular non-symlink file".into());
    }
    let attested = read_heartbeat_capability(attestation_path)?;
    let exact = exact_heartbeat_capability();
    if attested != exact {
        return Err("packaged buzz-acp attestation lacks the exact heartbeat capability".into());
    }

    let host = std::env::var("HOST")
        .map_err(|_| "HOST unavailable while verifying packaged buzz-acp".to_string())?;
    if host == target {
        let probed = probe_heartbeat_capability(path)?;
        if probed != exact || probed != attested {
            return Err("packaged buzz-acp probe does not match its attestation".into());
        }
    }
    Ok(())
}

fn embed_bundled_buzz_acp_digest() {
    println!("cargo:rerun-if-env-changed={REQUIRE_HEARTBEAT_SIDECAR_ENV}");
    println!("cargo:rerun-if-env-changed={HEARTBEAT_MACOS_TEAM_ENV}");
    println!("cargo:rerun-if-env-changed={SOURCE_REVISION_ENV}");
    let explicitly_required = std::env::var_os(REQUIRE_HEARTBEAT_SIDECAR_ENV).is_some();
    let Ok(target) = std::env::var("TARGET") else {
        if explicitly_required {
            panic!("TARGET unavailable for required packaged heartbeat sidecar");
        }
        println!("cargo:warning=TARGET unavailable; designated heartbeat agents will fail closed");
        return;
    };
    let suffix = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let path = std::path::PathBuf::from("binaries").join(format!("buzz-acp-{target}{suffix}"));
    let attestation_path = std::path::PathBuf::from("binaries").join(format!(
        "buzz-acp-{target}{suffix}.heartbeat-preflight-capability.json"
    ));
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rerun-if-changed={}", attestation_path.display());
    let attested_build = match std::fs::symlink_metadata(&attestation_path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => panic!(
            "cannot inspect heartbeat capability attestation at {}: {error}",
            attestation_path.display()
        ),
    };
    let verification_required = explicitly_required || attested_build;
    if verification_required && !attested_build {
        panic!(
            "required packaged heartbeat sidecar has no capability attestation at {}",
            attestation_path.display()
        );
    }
    if target.contains("apple-darwin") {
        match std::env::var(HEARTBEAT_MACOS_TEAM_ENV) {
            Ok(team)
                if team.len() == 10
                    && team
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()) =>
            {
                println!(
                    "cargo:rustc-env=BUZZ_DESKTOP_HEARTBEAT_HARNESS_MACOS_TEAM_IDENTIFIER={team}"
                );
            }
            Ok(_) => panic!(
                "{HEARTBEAT_MACOS_TEAM_ENV} must be one 10-character uppercase ASCII TeamIdentifier"
            ),
            Err(_) if explicitly_required => panic!(
                "{HEARTBEAT_MACOS_TEAM_ENV} is required for a designated-heartbeat macOS build"
            ),
            Err(_) => println!(
                "cargo:warning=no macOS heartbeat-harness TeamIdentifier pin; designated heartbeat agents will fail closed"
            ),
        }
    }
    match std::env::var(SOURCE_REVISION_ENV) {
        Ok(revision)
            if matches!(revision.len(), 40 | 64)
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) =>
        {
            println!("cargo:rustc-env=BUZZ_DESKTOP_SOURCE_REVISION={revision}");
        }
        Ok(_) => {
            panic!("{SOURCE_REVISION_ENV} must be one lowercase 40- or 64-hex source revision")
        }
        Err(_) if explicitly_required => {
            panic!("{SOURCE_REVISION_ENV} is required for a designated-heartbeat build")
        }
        Err(_) => {}
    }
    match std::fs::read(&path) {
        Ok(bytes) => {
            let digest = executable_identity_sha256(&bytes)
                .unwrap_or_else(|error| panic!("cannot identify bundled buzz-acp: {error}"));
            if verification_required {
                verify_packaged_heartbeat_sidecar(&path, &attestation_path, &target)
                    .unwrap_or_else(|error| panic!("packaged heartbeat sidecar rejected: {error}"));
                let after = std::fs::read(&path)
                    .unwrap_or_else(|error| panic!("cannot re-read packaged buzz-acp: {error}"));
                let after_digest = executable_identity_sha256(&after).unwrap_or_else(|error| {
                    panic!("cannot re-identify packaged buzz-acp: {error}")
                });
                if after_digest != digest {
                    panic!("packaged buzz-acp changed during capability verification");
                }
            } else if bytes.is_empty() {
                println!(
                    "cargo:warning=bundled buzz-acp is an unverified build placeholder; designated heartbeat agents will fail closed"
                );
            }
            println!("cargo:rustc-env=BUZZ_DESKTOP_BUNDLED_BUZZ_ACP_SHA256={digest}");
        }
        Err(error) if verification_required => panic!(
            "required packaged buzz-acp is unavailable at {}: {error}",
            path.display()
        ),
        Err(error) => println!(
            "cargo:warning=cannot pin bundled buzz-acp at {} ({error}); designated heartbeat agents will fail closed",
            path.display()
        ),
    }
}

fn main() {
    embed_bundled_buzz_acp_digest();
    println!("cargo:rerun-if-env-changed=BUZZ_RELAY_URL");
    println!("cargo:rerun-if-env-changed=BUZZ_RELAY_HTTP");
    println!("cargo:rerun-if-env-changed=BUZZ_UPDATER_PUBLIC_KEY");
    println!("cargo:rerun-if-env-changed=BUZZ_UPDATER_ENDPOINT");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_BUZZ_AGENT_PROVIDER");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_BUZZ_AGENT_MODEL");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_AGENT_ENV");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_RELAY_RECONNECT_CMD");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY");
    println!("cargo:rerun-if-env-changed=BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY");
    println!("cargo:rustc-check-cfg=cfg(buzz_updater_enabled)");

    // Explicit owner-only agent-access capability. Release packaging sets this
    // presence-only marker; OSS/custom builds leave agent access configurable.
    if std::env::var("BUZZ_BUILD_AGENT_ACCESS_OWNER_ONLY").is_ok() {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_AGENT_ACCESS_OWNER_ONLY=1");
    }

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
            // The baked env is written into every spawned agent's environment
            // LAST (see `managed_agents/runtime.rs`), after Buzz sets the
            // access gates and identity vars. A baked reserved key would
            // therefore silently override the gate the UI promises, so reject
            // it at build time instead of shipping a binary that bypasses its
            // own enforcement.
            if is_reserved_env_key(key) {
                panic!(
                    "BUZZ_BUILD_AGENT_ENV line {}: `{}` is reserved by Buzz and cannot be baked \
                     into a build (it would override Buzz's own identity/access env)",
                    line_no + 1,
                    key
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

    // Presence-only release capability: internal desktop builds opt into
    // auto-connecting their configured default relay on first run. OSS builds
    // leave this unset and retain explicit community selection.
    if std::env::var("BUZZ_BUILD_AUTO_CONNECT_DEFAULT_RELAY").is_ok() {
        println!("cargo:rustc-env=BUZZ_DESKTOP_BUILD_AUTO_CONNECT_DEFAULT_RELAY=1");
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

    // Cargo test executables get no embedded Windows manifest (tauri_build
    // attaches one to bin targets only), so the loader binds comctl32 v5, which
    // lacks TaskDialogIndirect (statically imported via tauri-plugin-dialog/rfd)
    // and debug test exes die at load with STATUS_ENTRYPOINT_NOT_FOUND. Declaring
    // the Common Controls v6 dependency makes link.exe emit a side-by-side
    // <exe>.manifest that the loader honors for manifest-less executables;
    // binaries with an embedded manifest (the real app) ignore it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }

    tauri_build::try_build(
        tauri_build::Attributes::new().plugin(
            "websocket",
            tauri_build::InlinedPlugin::new()
                .commands(&["connect", "send", "disconnect", "disconnect_all"])
                .default_permission(tauri_build::DefaultPermissionRule::AllowAllCommands),
        ),
    )
    .expect("failed to build Tauri application");
}
