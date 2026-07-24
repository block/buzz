//! Buzz-owned Codex state directory under the nest (`~/.buzz/.codex`).
//!
//! Managed Codex agents must not write sessions into the user's personal
//! `~/.codex` store — that surfaces `.buzz` in Codex Desktop / ChatGPT Remote
//! (#2660). Auth is handed off via a symlink (or copy) of `auth.json` from the
//! user store so login still works.

use std::{fs, path::Path, path::PathBuf, process::Command};

use super::discovery::KnownAcpRuntime;
use super::nest::nest_dir;

/// Returns the Buzz-owned Codex home, creating it and handing off auth when needed.
pub fn isolated_codex_home() -> Option<PathBuf> {
    let nest = nest_dir()?;
    let codex_home = nest.join(".codex");
    if let Err(error) = fs::create_dir_all(&codex_home) {
        eprintln!(
            "buzz-desktop: failed to create {}: {error}",
            codex_home.display()
        );
        return None;
    }
    handoff_user_codex_auth(&codex_home);
    Some(codex_home)
}

/// Set `CODEX_HOME` on a spawn command when the runtime is Codex.
pub fn apply_isolated_codex_home(command: &mut Command, runtime: Option<&KnownAcpRuntime>) {
    if runtime.is_none_or(|runtime| runtime.id != "codex") {
        return;
    }
    if let Some(codex_home) = isolated_codex_home() {
        command.env("CODEX_HOME", codex_home);
    }
}

fn handoff_user_codex_auth(codex_home: &Path) {
    let Some(user_home) = dirs::home_dir() else {
        return;
    };
    let user_codex = user_home.join(".codex");
    // Already isolated / same path — nothing to link.
    if user_codex == codex_home {
        return;
    }
    let src = user_codex.join("auth.json");
    let dst = codex_home.join("auth.json");
    if !src.is_file() || dst.exists() {
        return;
    }
    if let Err(error) = link_or_copy(&src, &dst) {
        eprintln!(
            "buzz-desktop: failed to hand off Codex auth {} → {}: {error}",
            src.display(),
            dst.display()
        );
    }
}

fn link_or_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        match std::os::unix::fs::symlink(src, dst) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            Err(_) => {
                // Fall through to copy (e.g. filesystem without symlink support).
            }
        }
    }
    fs::copy(src, dst).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_or_copy_writes_auth_handoff() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("auth.json");
        let dst_dir = tmp.path().join(".buzz").join(".codex");
        let dst = dst_dir.join("auth.json");
        fs::write(&src, b"{\"tokens\":[]}").unwrap();
        fs::create_dir_all(&dst_dir).unwrap();
        link_or_copy(&src, &dst).unwrap();
        assert!(dst.is_file() || dst.is_symlink());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "{\"tokens\":[]}");
    }
}
