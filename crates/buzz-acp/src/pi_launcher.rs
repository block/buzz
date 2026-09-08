//! Native Pi system prompts. Each ACP process owns a launcher; each session
//! owns an immutable prompt file, also used when pi-acp restores its subprocess.

mod native;
#[cfg(test)]
mod tests;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) use native::try_run;
pub(crate) const PI_ACP_PI_COMMAND_ENV: &str = "PI_ACP_PI_COMMAND";
const LAUNCH_MODE: &str = "--internal-pi-launch";

/// Private files for one adapter process, never shared across pool workers.
pub(crate) struct PiLaunchOverride {
    directory: PathBuf,
    launcher: PathBuf,
}

impl PiLaunchOverride {
    pub(crate) fn prepare(agent_command: &str) -> io::Result<Option<Arc<Self>>> {
        if crate::config::normalize_agent_command_identity(agent_command) != "pi-acp" {
            return Ok(None);
        }
        if std::env::var_os(PI_ACP_PI_COMMAND_ENV).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "PI_ACP_PI_COMMAND is managed by Buzz; unset it before starting a managed Pi agent",
            ));
        }
        Self::create(&std::env::current_exe()?).map(Some)
    }

    pub(crate) fn create(executable: &Path) -> io::Result<Arc<Self>> {
        let directory = std::env::temp_dir().join(format!(
            "buzz-acp-pi-launcher-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        create_private_directory(&directory)?;
        let prepared = Arc::new(Self {
            launcher: directory.join(if cfg!(windows) {
                "pi-with-buzz-context.cmd"
            } else {
                "pi-with-buzz-context"
            }),
            directory,
        });
        // Preserve the buzz-acp personality when current_exe resolves to Sprig.
        #[cfg(unix)]
        let executable = {
            let alias = prepared.directory.join("buzz-acp");
            std::os::unix::fs::symlink(executable, &alias)?;
            alias
        };
        write_private_file(
            &prepared.launcher,
            launcher_script(executable.as_ref(), &prepared.directory)?.as_bytes(),
            true,
        )?;
        Ok(prepared)
    }

    pub(crate) fn launcher_path(&self) -> &Path {
        &self.launcher
    }

    /// Called while the ACP client is exclusively borrowed for session/new.
    /// The pending pointer is only for new sessions; restores use their ID.
    pub(crate) fn begin(self: &Arc<Self>, prompt: &str) -> io::Result<PendingSession> {
        let token = Uuid::new_v4().to_string();
        let path = self.directory.join(format!("{token}.md"));
        write_private_file(&path, prompt.as_bytes(), false)?;
        let snapshot = PiSessionPrompt {
            _launcher: Arc::clone(self),
            path,
            mapping: None,
        };
        write_private_file(&self.directory.join("pending"), token.as_bytes(), false)?;
        Ok(PendingSession {
            snapshot: Some(snapshot),
            launcher: Arc::clone(self),
        })
    }
}

/// Keeps the prompt available for reload and restore until Buzz retires the session.
pub(crate) struct PiSessionPrompt {
    _launcher: Arc<PiLaunchOverride>,
    path: PathBuf,
    mapping: Option<PathBuf>,
}

pub(crate) struct PendingSession {
    snapshot: Option<PiSessionPrompt>,
    launcher: Arc<PiLaunchOverride>,
}

impl PendingSession {
    pub(crate) fn finish(mut self, session_id: &str) -> io::Result<PiSessionPrompt> {
        let id = parse_id(session_id)?;
        let mut snapshot = self
            .snapshot
            .take()
            .ok_or_else(|| io::Error::other("Pi snapshot already consumed"))?;
        let token = snapshot
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| io::Error::other("invalid Pi snapshot path"))?;
        let mapping = self.launcher.directory.join(format!("session-{id}"));
        write_private_file(&mapping, token.as_bytes(), false)?;
        snapshot.mapping = Some(mapping);
        fs::remove_file(self.launcher.directory.join("pending"))?;
        Ok(snapshot)
    }
}

impl Drop for PendingSession {
    fn drop(&mut self) {
        remove_file(&self.launcher.directory.join("pending"));
    }
}

impl Drop for PiSessionPrompt {
    fn drop(&mut self) {
        if let Some(mapping) = &self.mapping {
            remove_file(mapping);
        }
        remove_file(&self.path);
    }
}

impl Drop for PiLaunchOverride {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.directory) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(path = %self.directory.display(), %error, "failed to remove temporary Pi launcher");
            }
        }
    }
}

fn remove_file(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != io::ErrorKind::NotFound {
            tracing::warn!(path = %path.display(), %error, "failed to remove temporary Pi prompt file");
        }
    }
}

fn parse_id(value: &str) -> io::Result<Uuid> {
    Uuid::parse_str(value).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Pi session/snapshot ID must be a UUID",
        )
    })
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn write_private_file(path: &Path, content: &[u8], executable: bool) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o700 } else { 0o600 });
    }
    #[cfg(not(unix))]
    let _ = executable;
    let mut file = options.open(path)?;
    if let Err(error) = file.write_all(content).and_then(|()| file.sync_all()) {
        drop(file);
        remove_file(path);
        return Err(error);
    }
    Ok(())
}

#[cfg(unix)]
fn launcher_script(executable: &Path, directory: &Path) -> io::Result<String> {
    fn quote(path: &Path) -> io::Result<String> {
        let value = path
            .to_str()
            .ok_or_else(|| io::Error::other("Pi launcher paths must be valid UTF-8"))?;
        Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
    }
    Ok(format!(
        "#!/bin/sh\nexec {} {LAUNCH_MODE} {} \"$@\"\n",
        quote(executable)?,
        quote(directory)?
    ))
}

#[cfg(windows)]
fn launcher_script(executable: &Path, directory: &Path) -> io::Result<String> {
    fn quote(path: &Path) -> io::Result<String> {
        let value = path
            .to_str()
            .ok_or_else(|| io::Error::other("Pi launcher paths must be valid UTF-8"))?;
        Ok(format!(
            "\"{}\"",
            value.replace('%', "%%").replace('"', "\"\"")
        ))
    }
    Ok(format!(
        "@echo off\r\n{} {LAUNCH_MODE} {} %*\r\nexit /b %ERRORLEVEL%\r\n",
        quote(executable)?,
        quote(directory)?
    ))
}

#[cfg(not(any(unix, windows)))]
fn launcher_script(_: &Path, _: &Path) -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Pi launch overrides are unsupported on this platform",
    ))
}
