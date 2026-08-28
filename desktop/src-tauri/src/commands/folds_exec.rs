//! Tauri command backing the Accumulator settings panel.
//!
//! The panel is a thin control plane over the bundled `buzz` CLI: every
//! action runs `buzz folds <verb> …` with the desktop identity's key and the
//! active workspace relay, and returns the CLI's output verbatim for the UI
//! to render. All fold state lives on the relay as the user's own signed
//! events — this command holds none.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nostr::ToBech32;
use serde::Serialize;
use tauri::State;

use crate::app_state::AppState;
use crate::managed_agents::resolve_command;

/// Verbs the panel may pass through. Everything runs under the `folds`
/// group (prepended here, never caller-supplied), so the desktop identity's
/// key is only ever handed to the accumulator control plane.
const ALLOWED_VERBS: &[&str] = &[
    "set", "list", "get", "delete", "estimate", "run", "artifact", "share",
];

const MAX_ARGS: usize = 64;
const MAX_ARG_LEN: usize = 16 * 1024;

/// `folds run` invokes a model subprocess (the fold runner's own timeout is
/// 600s); every other verb is relay I/O only.
const RUN_TIMEOUT: Duration = Duration::from_secs(660);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldsCliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

fn validate_args(args: &[String]) -> Result<(), String> {
    let Some(verb) = args.first() else {
        return Err("folds command requires a verb".to_string());
    };
    if !ALLOWED_VERBS.contains(&verb.as_str()) {
        return Err(format!("unsupported folds verb: {verb}"));
    }
    if args.len() > MAX_ARGS {
        return Err(format!("too many arguments ({} > {MAX_ARGS})", args.len()));
    }
    for arg in args {
        if arg.len() > MAX_ARG_LEN {
            return Err(format!(
                "argument too long ({} > {MAX_ARG_LEN} bytes)",
                arg.len()
            ));
        }
        if arg.contains('\0') {
            return Err("argument contains a NUL byte".to_string());
        }
    }
    Ok(())
}

/// Resolve the bundled `buzz` CLI: exe-sibling sidecar first (DMG install),
/// then the general command resolver (dev builds, PATH installs).
fn resolve_buzz_cli() -> Result<std::path::PathBuf, String> {
    std::env::current_exe()
        .map(|path| path.with_file_name(format!("buzz{}", std::env::consts::EXE_SUFFIX)))
        .ok()
        .filter(|path| path.exists())
        .or_else(|| resolve_command("buzz"))
        .ok_or_else(|| "buzz CLI not found".to_string())
}

fn read_pipe_lossy(pipe: Option<impl Read>) -> String {
    let Some(mut pipe) = pipe else {
        return String::new();
    };
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).to_string()
}

fn run_folds_cli_blocking(
    args: Vec<String>,
    nsec: String,
    relay_url: String,
) -> Result<FoldsCliOutput, String> {
    validate_args(&args)?;
    let cli_path = resolve_buzz_cli()?;
    let timeout = if args[0] == "run" {
        RUN_TIMEOUT
    } else {
        QUERY_TIMEOUT
    };

    let mut command = Command::new(&cli_path);
    command.arg("folds").args(&args);
    command.env("BUZZ_PRIVATE_KEY", &nsec);
    command.env("BUZZ_RELAY_URL", &relay_url);
    // The fold runner shells out to the user's model CLI (`claude` by
    // default); the augmented PATH makes that resolvable from a DMG launch.
    if let Some(path) = crate::managed_agents::readiness::cli_probe::augmented_path() {
        command.env("PATH", path);
    }
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    crate::util::configure_no_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to run buzz folds: {error}"))?;

    // Drain the pipes on background threads so a chatty run can't deadlock
    // on a full pipe while we poll for exit below.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || read_pipe_lossy(stdout_pipe));
    let stderr_thread = std::thread::spawn(move || read_pipe_lossy(stderr_pipe));

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Err(format!("buzz folds timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("failed to wait for buzz folds: {error}"));
            }
        }
    };

    Ok(FoldsCliOutput {
        stdout: stdout_thread.join().unwrap_or_default(),
        stderr: stderr_thread.join().unwrap_or_default(),
        exit_code: status.code().unwrap_or(-1),
    })
}

/// Run `buzz folds <args…>` as the desktop identity against the active
/// workspace relay. Nonzero exits are returned as data (the CLI's stderr
/// carries the actionable message); `Err` is reserved for launch failures,
/// timeouts, and validation.
#[tauri::command]
pub async fn run_folds_cli(
    args: Vec<String>,
    state: State<'_, AppState>,
) -> Result<FoldsCliOutput, String> {
    validate_args(&args)?;
    let nsec = state
        .signing_keys()?
        .secret_key()
        .to_bech32()
        .map_err(|error| format!("failed to encode signing key: {error}"))?;
    let relay_url = crate::relay::relay_ws_url_with_override(&state);
    tokio::task::spawn_blocking(move || run_folds_cli_blocking(args, nsec, relay_url))
        .await
        .map_err(|error| format!("folds task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rejects_empty_and_unknown_verbs() {
        assert!(validate_args(&[]).is_err());
        assert!(validate_args(&args(&["frobnicate"])).is_err());
        // Group escape attempts are just unknown verbs.
        assert!(validate_args(&args(&["--help"])).is_err());
        assert!(validate_args(&args(&["mem"])).is_err());
    }

    #[test]
    fn accepts_every_folds_verb() {
        for verb in ALLOWED_VERBS {
            assert!(validate_args(&args(&[verb])).is_ok(), "verb {verb}");
        }
        assert!(validate_args(&args(&["estimate", "my-fold", "--limit", "50"])).is_ok());
    }

    #[test]
    fn rejects_oversized_and_nul_arguments() {
        assert!(validate_args(&args(&["run", &"x".repeat(MAX_ARG_LEN + 1)])).is_err());
        assert!(validate_args(&args(&["get", "bad\0name"])).is_err());
        let mut many = vec!["list".to_string()];
        many.extend(std::iter::repeat_n("x".to_string(), MAX_ARGS));
        assert!(validate_args(&many).is_err());
    }
}
