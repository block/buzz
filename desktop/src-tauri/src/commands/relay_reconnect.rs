//! Configurable transport-reconnect hook.
//!
//! When the build-time env var `BUZZ_BUILD_RELAY_RECONNECT_CMD` is set (internal
//! builds), this command runs an ordered sequence of subprocess steps followed by
//! a readiness poll before the frontend fires the relay WebSocket reconnect.
//!
//! OSS builds (env var unset) get a pure no-op — zero WARP knowledge compiled in.

// Single source of truth for the config schema, shared with build.rs via
// `include!`. See reconnect_hook_config.rs for why this is shared, not a module.
include!("reconnect_hook_config.rs");

#[tauri::command]
pub async fn relay_reconnect_hook() -> Result<(), String> {
    let Some(config_str) = option_env!("BUZZ_DESKTOP_BUILD_RELAY_RECONNECT_CMD") else {
        return Ok(()); // OSS build — no-op
    };

    // Safe: build.rs already validated this parses correctly against the same schema.
    let config: ReconnectHookConfig = serde_json::from_str(config_str)
        .map_err(|e| format!("reconnect hook config parse error: {e}"))?;

    // spawn_blocking because the desktop Tauri crate doesn't enable tokio's
    // `process` feature; std::process::Command + thread::sleep are synchronous
    // and must not run on an async worker. The whole hook is non-fatal — a join
    // failure logs and returns Ok so the frontend's relay reconnect still fires.
    if let Err(e) = tokio::task::spawn_blocking(move || run_hook(&config)).await {
        eprintln!("[relay_reconnect_hook] task join failed: {e}");
    }

    Ok(())
}

/// Runs the configured steps then polls the readiness probe. Every failure is
/// logged and swallowed — the caller treats the hook as best-effort.
fn run_hook(config: &ReconnectHookConfig) {
    // Run each step sequentially (fixed-argv, no shell).
    for step in &config.steps {
        if step.is_empty() {
            continue;
        }
        match std::process::Command::new(&step[0]).args(&step[1..]).output() {
            Ok(o) if !o.status.success() => {
                eprintln!("[relay_reconnect_hook] step {:?} exited {}", step, o.status);
            }
            Err(e) => {
                eprintln!("[relay_reconnect_hook] failed to spawn {:?}: {e}", step);
            }
            _ => {}
        }
    }

    // Poll readiness probe until match or timeout.
    if config.ready_probe.is_empty() {
        return;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(config.timeout_ms);
    while std::time::Instant::now() < deadline {
        if let Ok(output) = std::process::Command::new(&config.ready_probe[0])
            .args(&config.ready_probe[1..])
            .output()
        {
            if String::from_utf8_lossy(&output.stdout).contains(&config.ready_match) {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
