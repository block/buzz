//! Buzz-owned Codex state directory (`$NEST/.codex`).
//!
//! Codex defaults to `~/.codex` for sessions and history. Managed agents must
//! not write there — that mixes Buzz sessions into the user's personal Codex
//! sidebar / ChatGPT Remote list (#2660).

use std::fs;
use std::path::{Path, PathBuf};

use super::nest_dir;

/// Buzz-owned Codex state directory (`$NEST/.codex`).
pub fn buzz_codex_home() -> Option<PathBuf> {
    nest_dir().map(|root| root.join(".codex"))
}

/// Ensure a Buzz-owned `CODEX_HOME` exists and is seeded with auth/config from
/// the user's personal `~/.codex` when missing.
///
/// Copies (does not symlink) `auth.json` and `config.toml` so Buzz can own
/// session/history files while still inheriting an existing login. Returns the
/// directory path suitable for the `CODEX_HOME` env var.
pub fn prepare_isolated_codex_home() -> Option<PathBuf> {
    let home = buzz_codex_home()?;
    fs::create_dir_all(&home).ok()?;
    seed_codex_home_file(&home, "auth.json");
    seed_codex_home_file(&home, "config.toml");
    Some(home)
}

/// Set `CODEX_HOME` on a managed-agent spawn when the runtime is Codex.
pub fn apply_isolated_codex_home_env(
    command: &mut std::process::Command,
    runtime_id: Option<&str>,
) {
    if runtime_id != Some("codex") {
        return;
    }
    if let Some(codex_home) = prepare_isolated_codex_home() {
        command.env("CODEX_HOME", codex_home);
    }
}

fn seed_codex_home_file(codex_home: &Path, name: &str) {
    let dest = codex_home.join(name);
    if dest.exists() {
        return;
    }
    let Some(user_codex) = dirs::home_dir().map(|h| h.join(".codex")) else {
        return;
    };
    let src = user_codex.join(name);
    if src.is_file() {
        let _ = fs::copy(&src, &dest);
    }
}

#[cfg(test)]
fn seed_codex_home_from(user_codex: &Path, nest_codex: &Path) {
    let _ = fs::create_dir_all(nest_codex);
    for name in ["auth.json", "config.toml"] {
        let dest = nest_codex.join(name);
        if dest.exists() {
            continue;
        }
        let src = user_codex.join(name);
        if src.is_file() {
            let _ = fs::copy(&src, &dest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_codex_home_copies_auth_and_config_once() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user-codex");
        let nest = tmp.path().join("nest-codex");
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("auth.json"), r#"{"tokens":1}"#).unwrap();
        fs::write(user.join("config.toml"), "model = \"o3\"\n").unwrap();

        seed_codex_home_from(&user, &nest);
        assert_eq!(
            fs::read_to_string(nest.join("auth.json")).unwrap(),
            r#"{"tokens":1}"#
        );
        assert_eq!(
            fs::read_to_string(nest.join("config.toml")).unwrap(),
            "model = \"o3\"\n"
        );

        fs::write(nest.join("auth.json"), r#"{"tokens":"buzz"}"#).unwrap();
        fs::write(user.join("auth.json"), r#"{"tokens":"user"}"#).unwrap();
        seed_codex_home_from(&user, &nest);
        assert_eq!(
            fs::read_to_string(nest.join("auth.json")).unwrap(),
            r#"{"tokens":"buzz"}"#
        );
    }
}
