//! Pi-specific native launcher setup.
//!
//! `pi-acp` does not currently consume ACP `session/new.systemPrompt`, but it
//! does let callers replace the `pi` executable through
//! `PI_ACP_PI_COMMAND`. For Pi sessions, Buzz points that variable at a
//! private launcher which adds `--system-prompt <file>` and the canonical Buzz
//! `--skill <directory>` before forwarding the adapter's RPC/session arguments
//! unchanged.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::ffi::OsStr;

use uuid::Uuid;

pub(crate) const PI_ACP_PI_COMMAND_ENV: &str = "PI_ACP_PI_COMMAND";

/// Files backing the Pi launcher for one `buzz-acp` process.
///
/// The guard must live as long as the ACP pool because `pi-acp` may start or
/// restore Pi subprocesses after its own initialization.
pub(crate) struct PiLaunchOverride {
    directory: PathBuf,
    launcher: PathBuf,
}

impl PiLaunchOverride {
    /// Prepare a Pi launcher when the configured ACP adapter is `pi-acp`.
    ///
    /// Returns the prompt that still needs ordinary ACP delivery. For Pi, the
    /// base prompt moves into Pi's native system role and is therefore removed
    /// from first-turn user framing. Other adapters receive it unchanged.
    pub(crate) fn prepare(
        agent_command: &str,
        base_prompt: Option<String>,
        managed_skills_dir: &Path,
        inherited_pi_command_is_set: bool,
    ) -> io::Result<(Option<Self>, Option<String>)> {
        if crate::config::normalize_agent_command_identity(agent_command) != "pi-acp" {
            return Ok((None, base_prompt));
        }

        if inherited_pi_command_is_set {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "PI_ACP_PI_COMMAND is managed by Buzz; unset it before starting a managed Pi agent",
            ));
        }

        // Buzz owns PI_ACP_PI_COMMAND and always uses it to point pi-acp at
        // this generated launcher. The launcher resolves the ordinary `pi`
        // command from Buzz's effective PATH.
        let prepared = Self::create("pi", base_prompt.as_deref(), managed_skills_dir)?;
        Ok((Some(prepared), None))
    }

    pub(crate) fn launcher_path(&self) -> &Path {
        &self.launcher
    }

    fn create(
        pi_command: &str,
        prompt: Option<&str>,
        managed_skills_dir: &Path,
    ) -> io::Result<Self> {
        let directory = std::env::temp_dir().join(format!(
            "buzz-acp-pi-launcher-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        create_private_directory(&directory)?;

        let prompt_path = directory.join("SYSTEM.md");
        let launcher = directory.join(launcher_file_name());
        // Construct the cleanup guard before either file write. Any later `?`
        // drops it, so a partial setup cannot strand the private prompt file.
        let prepared = Self {
            directory,
            launcher,
        };

        if let Some(prompt) = prompt {
            write_private_file(&prompt_path, prompt.as_bytes(), false)?;
        }

        let script = launcher_script(
            pi_command,
            prompt.map(|_| prompt_path.as_path()),
            managed_skills_dir,
        )?;
        write_private_file(&prepared.launcher, script.as_bytes(), true)?;

        Ok(prepared)
    }
}

impl Drop for PiLaunchOverride {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.directory) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    path = %self.directory.display(),
                    %error,
                    "failed to remove temporary Pi launcher"
                );
            }
        }
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
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
    file.write_all(content)?;
    file.sync_all()
}

#[cfg(unix)]
fn launcher_file_name() -> &'static str {
    "pi-with-buzz-context"
}

#[cfg(windows)]
fn launcher_file_name() -> &'static str {
    "pi-with-buzz-context.cmd"
}

#[cfg(not(any(unix, windows)))]
fn launcher_file_name() -> &'static str {
    "pi-with-buzz-context"
}

#[cfg(unix)]
fn launcher_script(
    pi_command: &str,
    prompt_path: Option<&Path>,
    managed_skills_dir: &Path,
) -> io::Result<String> {
    let system_prompt_arg = match prompt_path {
        Some(prompt_path) => format!(" --system-prompt {}", shell_quote(prompt_path.as_os_str())?),
        None => String::new(),
    };
    Ok(format!(
        "#!/bin/sh\nexec {}{} --skill {} \"$@\"\n",
        shell_quote(OsStr::new(pi_command))?,
        system_prompt_arg,
        shell_quote(managed_skills_dir.as_os_str())?,
    ))
}

#[cfg(unix)]
fn shell_quote(value: &OsStr) -> io::Result<String> {
    let value = value.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Pi launcher paths must be valid UTF-8",
        )
    })?;
    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}

#[cfg(windows)]
fn launcher_script(
    pi_command: &str,
    prompt_path: Option<&Path>,
    managed_skills_dir: &Path,
) -> io::Result<String> {
    let system_prompt_arg = match prompt_path {
        Some(prompt_path) => {
            let prompt_path = prompt_path.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Pi launcher paths must be valid UTF-8",
                )
            })?;
            format!(" --system-prompt \"{}\"", batch_escape(prompt_path))
        }
        None => String::new(),
    };
    let managed_skills_dir = managed_skills_dir.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Pi skill paths must be valid UTF-8",
        )
    })?;
    Ok(format!(
        "@echo off\r\n\"{}\"{} --skill \"{}\" %*\r\nexit /b %ERRORLEVEL%\r\n",
        batch_escape(pi_command),
        system_prompt_arg,
        batch_escape(managed_skills_dir),
    ))
}

#[cfg(windows)]
fn batch_escape(value: &str) -> String {
    value.replace('%', "%%").replace('"', "\"\"")
}

#[cfg(not(any(unix, windows)))]
fn launcher_script(
    _pi_command: &str,
    _prompt_path: Option<&Path>,
    _managed_skills_dir: &Path,
) -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Pi launch overrides are unsupported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_pi_adapter_keeps_base_prompt_for_acp_delivery() {
        let base = Some("Buzz base".to_string());
        let (prepared, remaining) =
            PiLaunchOverride::prepare("goose", base.clone(), Path::new("/unused/skills"), true)
                .expect("prepare");
        assert!(prepared.is_none());
        assert_eq!(remaining, base);
    }

    #[test]
    fn pi_adapter_rejects_inherited_pi_command() {
        let error = PiLaunchOverride::prepare(
            "pi-acp",
            Some("Buzz base".to_string()),
            Path::new("/unused/skills"),
            true,
        )
        .err()
        .expect("inherited PI_ACP_PI_COMMAND must be rejected");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("managed by Buzz"));
    }

    #[test]
    fn disabled_base_prompt_still_creates_pi_skills_launcher() {
        let (prepared, remaining) =
            PiLaunchOverride::prepare("pi-acp", None, Path::new("/unused/skills"), false)
                .expect("prepare");
        let prepared = prepared.expect("Pi skills launcher");
        assert!(remaining.is_none());
        assert!(!prepared.directory.join("SYSTEM.md").exists());

        #[cfg(unix)]
        assert!(fs::read_to_string(prepared.launcher_path())
            .expect("read launcher")
            .contains("--skill '/unused/skills'"));
    }

    #[test]
    fn pi_adapter_moves_buzz_base_out_of_ordinary_acp_delivery() {
        let base = crate::scope::SessionPolicy::Thread
            .append_session_model(include_str!("base_prompt.md"));
        let (prepared, remaining) = PiLaunchOverride::prepare(
            "/opt/bin/pi-acp",
            Some(base.clone()),
            Path::new("/buzz/.agents/skills"),
            false,
        )
        .expect("prepare");
        let prepared = prepared.expect("Pi launcher");

        assert!(remaining.is_none());
        assert_eq!(
            fs::read_to_string(prepared.directory.join("SYSTEM.md")).expect("read prompt"),
            base
        );
        assert!(base.contains("each thread gets its own"));

        #[cfg(unix)]
        assert!(fs::read_to_string(prepared.launcher_path())
            .expect("read launcher")
            .contains("exec 'pi'"));
    }

    #[cfg(unix)]
    #[test]
    fn pi_launcher_replaces_system_prompt_and_forwards_adapter_args() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let fixture_dir =
            std::env::temp_dir().join(format!("buzz-acp-pi-system-prompt-test-{}", Uuid::new_v4()));
        create_private_directory(&fixture_dir).expect("create fixture dir");
        let capture_path = fixture_dir.join("args.txt");
        let fake_pi = fixture_dir.join("fake-pi");
        let managed_skills_dir = fixture_dir.join("managed skills");
        let fake_script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\n",
            shell_quote(capture_path.as_os_str()).expect("quote capture path")
        );
        write_private_file(&fake_pi, fake_script.as_bytes(), true).expect("write fake pi");

        let prepared = PiLaunchOverride::create(
            fake_pi.to_str().expect("UTF-8 fake Pi path"),
            Some("Buzz base\n\n## Session Model\nThread scoped"),
            &managed_skills_dir,
        )
        .expect("prepare Pi launcher");
        let prompt_path = prepared.directory.join("SYSTEM.md");

        let status = Command::new(prepared.launcher_path())
            .args(["--mode", "rpc", "--session", "/tmp/session.jsonl"])
            .status()
            .expect("run launcher");
        assert!(status.success());
        assert_eq!(
            fs::read_to_string(&capture_path).expect("read captured args"),
            format!(
                "--system-prompt\n{}\n--skill\n{}\n--mode\nrpc\n--session\n/tmp/session.jsonl\n",
                prompt_path.display(),
                managed_skills_dir.display(),
            )
        );
        assert_eq!(
            fs::read_to_string(&prompt_path).expect("read system prompt"),
            "Buzz base\n\n## Session Model\nThread scoped"
        );
        assert_eq!(
            fs::metadata(&prompt_path)
                .expect("prompt metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(prepared.launcher_path())
                .expect("launcher metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&prepared.directory)
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        drop(prepared);
        assert!(!prompt_path.exists());
        fs::remove_dir_all(fixture_dir).expect("remove fixture dir");
    }
}
