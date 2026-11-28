//! Explicit CLI credential source; never discovered or invoked by default.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use nostr::Keys;
use tokio::io::AsyncReadExt;
use zeroize::Zeroizing;

use crate::error::CliError;

/// A user explicitly selects the installed CLI and the item to retrieve.
pub struct CredentialCommand {
    pub executable: String,
    pub service: String,
    pub account: String,
}

impl CredentialCommand {
    pub(crate) fn from_args(values: Vec<String>) -> Result<Self, CliError> {
        let [executable, service, account]: [String; 3] = values.try_into().map_err(|_| {
            CliError::Auth("--credential-command requires a path, service and account".into())
        })?;
        Ok(Self {
            executable,
            service,
            account,
        })
    }

    pub(super) async fn load(&self) -> Result<Keys, CliError> {
        let executable = Path::new(&self.executable);
        if !executable.is_absolute() {
            return Err(CliError::Auth(
                "--credential-command requires an absolute executable path".into(),
            ));
        }
        // Windows can implicitly run batch files through cmd.exe; accept only
        // native executables there so that the item names stay literal args.
        #[cfg(windows)]
        if !executable
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            return Err(CliError::Auth(
                "--credential-command requires an .exe on Windows".into(),
            ));
        }
        let read = async {
            let unavailable = |_| CliError::Auth("credential command key retrieval failed".into());
            let mut command = tokio::process::Command::new(executable);
            // Provider commands must not receive the parent application's
            // credentials (including inherited desktop tokens). Pass only
            // basic OS paths and session discovery variables.
            command.env_clear();
            for name in [
                "PATH",
                "HOME",
                "TMPDIR",
                "TMP",
                "TEMP",
                "SystemRoot",
                "USERPROFILE",
                "APPDATA",
                "LOCALAPPDATA",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_RUNTIME_DIR",
                "DBUS_SESSION_BUS_ADDRESS",
            ] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
            let mut child = command
                .args(["get", &self.service, &self.account])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(unavailable)?;
            // A hex or nsec key fits well within this cap, even with whitespace.
            // Keep the raw secret bounded, off the logs, and zeroize it on drop.
            const MAX_KEY_BYTES: usize = 128;
            let mut output = Zeroizing::new([0; MAX_KEY_BYTES + 1]);
            let mut stdout = child
                .stdout
                .take()
                .ok_or_else(|| CliError::Auth("credential command key retrieval failed".into()))?;
            let mut used = 0;
            while used < output.len() {
                let count = stdout
                    .read(&mut output[used..])
                    .await
                    .map_err(unavailable)?;
                if count == 0 {
                    break;
                }
                used += count;
            }
            if used > MAX_KEY_BYTES || !child.wait().await.map_err(unavailable)?.success() {
                return Err(CliError::Auth(
                    "credential command key retrieval failed".into(),
                ));
            }
            let key = std::str::from_utf8(&output[..used])
                .map_err(|_| CliError::Key("invalid Nostr key from credential command".into()))?;
            Keys::parse(key.trim())
                .map_err(|_| CliError::Key("invalid Nostr key from credential command".into()))
        };
        tokio::time::timeout(Duration::from_secs(15), read)
            .await
            .map_err(|_| CliError::Auth("credential command key retrieval timed out".into()))?
    }
}
