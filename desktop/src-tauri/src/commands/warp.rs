use std::io;
use std::process::{Command, Output};

type CmdResult<T> = Result<T, String>;

const WARP_CLI_CANDIDATES: &[&str] = &[
    "warp-cli",
    "/Applications/Cloudflare WARP.app/Contents/Resources/warp-cli",
];

fn handle_warp_cli_output(command: &str, args: &[&str], output: Output) -> CmdResult<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = stderr.trim();
    let details = if details.is_empty() {
        stdout.trim()
    } else {
        details
    };

    if details.is_empty() {
        Err(format!("{command} {} failed.", args.join(" ")))
    } else {
        Err(format!("{command} {} failed: {details}", args.join(" ")))
    }
}

fn run_warp_cli(args: &[&str]) -> CmdResult<()> {
    for command in WARP_CLI_CANDIDATES {
        let output = match Command::new(command).args(args).output() {
            Ok(output) => output,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Failed to run {command}: {error}")),
        };

        return handle_warp_cli_output(command, args, output);
    }

    Err("Cloudflare WARP CLI is not installed or is not on PATH.".to_string())
}

#[tauri::command]
pub async fn connect_warp_vpn() -> CmdResult<()> {
    tokio::task::spawn_blocking(|| run_warp_cli(&["connect"]))
        .await
        .map_err(|error| format!("Failed to run WARP command: {error}"))?
}

#[tauri::command]
pub async fn refresh_warp_access() -> CmdResult<()> {
    tokio::task::spawn_blocking(|| run_warp_cli(&["debug", "access-reauth"]))
        .await
        .map_err(|error| format!("Failed to run WARP command: {error}"))?
}
