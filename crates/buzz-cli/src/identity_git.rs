//! Git launcher for enterprise agents. A fresh assertion for each invocation,
//! not the expiring token inherited when an agent process started.
use crate::error::CliError;

pub(crate) async fn run(
    args: Vec<String>,
    keys: &nostr::Keys,
    relay: &str,
) -> Result<(), CliError> {
    let binary = std::env::var_os("BUZZ_NIP_FI_GIT_BINARY").unwrap_or_else(|| "git".into());
    let mut command = std::process::Command::new(binary);
    command.args(args);
    let base = std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .map(|s| s.parse::<usize>())
        .transpose()
        .map_err(|_| CliError::Usage("invalid Git configuration count".into()))?
        .unwrap_or(0);
    buzz_ws_client::identity_git::configure(&mut command, keys, relay, base)
        .await
        .map_err(|e| CliError::Auth(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut command = tokio::process::Command::from(command);
    command.kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|_| CliError::Other("could not start git".into()))?;
    match tokio::time::timeout(std::time::Duration::from_secs(300), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(_)) => Err(CliError::Other("git command failed".into())),
        Ok(Err(_)) => Err(CliError::Other("could not wait for git".into())),
        Err(_) => {
            if let Some(pid) = child.id() {
                #[cfg(unix)]
                nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGKILL,
                )
                .map_err(|_| {
                    CliError::Other("could not stop timed-out git process group".into())
                })?;
                #[cfg(windows)]
                {
                    let status = tokio::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .status()
                        .await
                        .map_err(|_| {
                            CliError::Other("could not stop timed-out git process tree".into())
                        })?;
                    if !status.success() {
                        return Err(CliError::Other(
                            "could not stop timed-out git process tree".into(),
                        ));
                    }
                }
            }
            child
                .wait()
                .await
                .map_err(|_| CliError::Other("could not stop timed-out git".into()))?;
            Err(CliError::Other("git command timed out".into()))
        }
    }
}
