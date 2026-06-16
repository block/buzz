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

/// Run a fixed-argv command (`argv[0]` + `argv[1..]`) with a wall-clock cap.
///
/// `std::process::Command::output()` blocks until the child exits — a wedged
/// `warp-cli` (the exact degraded-transport case this hook targets) would hang
/// forever, pinning the blocking-pool thread and leaving the frontend `invoke`
/// unresolved. So we spawn, poll `try_wait()` every 500ms, and kill+reap on the
/// deadline. Modeled on `media_transcode.rs` `run_ffmpeg_with_timeout`.
///
/// stdout/stderr are piped and read only after the child exits. The pipe-buffer
/// deadlock noted there (a child blocking on write() when the ~64 KiB OS pipe
/// fills) does not apply: `warp-cli` emits a few lines, far below the buffer.
fn run_with_timeout(
    argv: &[String],
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    let mut child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = child.stdout.take().map_or_else(Vec::new, |mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                });
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr: Vec::new(),
                });
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // reap zombie
                    return Err(format!("timed out after {}ms", timeout.as_millis()));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => return Err(format!("wait failed: {e}")),
        }
    }
}

/// Runs the configured steps then polls the readiness probe. Every failure is
/// logged and swallowed — the caller treats the hook as best-effort.
fn run_hook(config: &ReconnectHookConfig) {
    let cap = std::time::Duration::from_millis(config.timeout_ms);

    // Run each step sequentially (fixed-argv, no shell). Each step is capped at
    // `timeout_ms` so a hung child can't stall the whole hook — on cap it's
    // killed, logged, and we move on, same non-fatal contract as a spawn error.
    for step in &config.steps {
        if step.is_empty() {
            continue;
        }
        match run_with_timeout(step, cap) {
            Ok(o) if !o.status.success() => {
                eprintln!("[relay_reconnect_hook] step {:?} exited {}", step, o.status);
            }
            Err(e) => {
                eprintln!("[relay_reconnect_hook] step {:?} failed: {e}", step);
            }
            _ => {}
        }
    }

    // Poll readiness probe until match or timeout.
    if config.ready_probe.is_empty() {
        return;
    }
    let deadline = std::time::Instant::now() + cap;
    while std::time::Instant::now() < deadline {
        // Cap each probe at the time left to the deadline, so one wedged probe
        // can't push the total probe phase past `timeout_ms`.
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match run_with_timeout(&config.ready_probe, remaining) {
            Ok(output) if String::from_utf8_lossy(&output.stdout).contains(&config.ready_match) => {
                return;
            }
            Err(e) => {
                eprintln!("[relay_reconnect_hook] probe failed: {e}");
            }
            _ => {}
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
