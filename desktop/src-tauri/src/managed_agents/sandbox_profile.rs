//! nono sandbox profiles for desktop-managed agents.
//!
//! Buzz installs two profiles in nono's user profile directory
//! (`$XDG_CONFIG_HOME/nono/profiles`, default `~/.config/nono/profiles`):
//!
//! - `<nest>-agent.json` — the Buzz-owned base, rewritten whenever the
//!   embedded template changes. Users should not edit it.
//! - `<nest>-agent-local.json` — the user-owned layer that extends the base.
//!   Seeded once, never overwritten. Agents run with this profile.
//!
//! `<nest>` is the nest directory name without its leading dot (`buzz`,
//! `buzz-dev`, `buzz-demo-<slug>`), so prod, dev, and demo builds never
//! rewrite each other's base. nono resolves `extends` by profile name only,
//! which is why the base must live next to the local profile rather than in
//! the app bundle.
//!
//! The macOS keychain grant lives in the seeded local profile, not the base:
//! nono merges `extends` additively, so a local profile can never remove a
//! base grant. Keeping it in the user's file lets them delete it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Base profile template. `meta.name` is rewritten per build identity.
const BASE_PROFILE_TEMPLATE: &str = include_str!("sandbox_profile.json");

/// Names of the base and local sandbox profiles for one nest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SandboxProfileNames {
    /// Buzz-owned base profile name.
    pub base: String,
    /// User-owned profile name agents are launched with.
    pub local: String,
}

/// Derive the profile names from the nest root (`~/.buzz` → `buzz-agent`).
pub(crate) fn profile_names(nest_root: &Path) -> Option<SandboxProfileNames> {
    let nest_name = nest_root.file_name()?.to_str()?.trim_start_matches('.');
    if nest_name.is_empty() {
        return None;
    }
    let base = format!("{nest_name}-agent");
    let local = format!("{base}-local");
    Some(SandboxProfileNames { base, local })
}

/// nono's user profile directory, honoring `XDG_CONFIG_HOME` like nono does
/// (on every platform, including macOS).
pub(crate) fn nono_profiles_dir() -> Option<PathBuf> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))?;
    Some(config_home.join("nono").join("profiles"))
}

/// Install the base profile and seed the local profile for the live nest.
pub(crate) fn ensure_sandbox_profiles() -> Result<SandboxProfileNames, String> {
    let (dir, names) = live_profiles()?;
    ensure_sandbox_profiles_at(&dir, &names)?;
    Ok(names)
}

/// Path of the user-owned profile for the live nest, installing both
/// profiles first so the file exists.
pub(crate) fn ensure_local_profile_path() -> Result<PathBuf, String> {
    let (dir, names) = live_profiles()?;
    ensure_sandbox_profiles_at(&dir, &names)?;
    Ok(local_profile_path(&dir, &names))
}

/// Replace the user-owned profile with the default seed. Running agents keep
/// their current sandbox until they restart.
pub(crate) fn reset_local_profile() -> Result<PathBuf, String> {
    let (dir, names) = live_profiles()?;
    reset_local_profile_at(&dir, &names)?;
    Ok(local_profile_path(&dir, &names))
}

fn live_profiles() -> Result<(PathBuf, SandboxProfileNames), String> {
    let nest = super::nest_dir().ok_or("cannot resolve the nest directory")?;
    let names = profile_names(&nest).ok_or_else(|| {
        format!(
            "cannot derive a sandbox profile name from {}",
            nest.display()
        )
    })?;
    let dir = nono_profiles_dir().ok_or("cannot resolve the nono profile directory")?;
    Ok((dir, names))
}

fn local_profile_path(profiles_dir: &Path, names: &SandboxProfileNames) -> PathBuf {
    profiles_dir.join(format!("{}.json", names.local))
}

/// Remove the local profile (a symlink is removed, not its target) and seed it
/// again.
pub(crate) fn reset_local_profile_at(
    profiles_dir: &Path,
    names: &SandboxProfileNames,
) -> Result<(), String> {
    let path = local_profile_path(profiles_dir, names);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("failed to remove {}: {e}", path.display())),
    }
    ensure_sandbox_profiles_at(profiles_dir, names)
}

/// Write the base profile when its content differs, and create the local
/// profile only when it is absent.
pub(crate) fn ensure_sandbox_profiles_at(
    profiles_dir: &Path,
    names: &SandboxProfileNames,
) -> Result<(), String> {
    fs::create_dir_all(profiles_dir)
        .map_err(|e| format!("failed to create {}: {e}", profiles_dir.display()))?;

    let base_path = profiles_dir.join(format!("{}.json", names.base));
    let base = render_base_profile(names)?;
    if fs::read(&base_path).ok().as_deref() != Some(base.as_bytes()) {
        write_replacing(&base_path, base.as_bytes())?;
    }

    let local_path = local_profile_path(profiles_dir, names);
    write_if_absent(&local_path, render_local_profile(names)?.as_bytes())
}

fn render_base_profile(names: &SandboxProfileNames) -> Result<String, String> {
    let mut profile: serde_json::Value = serde_json::from_str(BASE_PROFILE_TEMPLATE)
        .map_err(|e| format!("embedded sandbox profile is invalid JSON: {e}"))?;
    let meta = profile
        .get_mut("meta")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("embedded sandbox profile has no meta object")?;
    meta.insert("name".into(), names.base.clone().into());
    meta.insert(
        "description".into(),
        format!(
            "Buzz-managed base sandbox for desktop agents. Buzz overwrites this file; \
             edit {}.json instead.",
            names.local
        )
        .into(),
    );
    to_profile_json(&profile)
}

fn render_local_profile(names: &SandboxProfileNames) -> Result<String, String> {
    let keychain = "$HOME/Library/Keychains";
    to_profile_json(&serde_json::json!({
        "meta": {
            "name": names.local,
            "version": "1",
            "description": "Your sandbox settings for Buzz agents. Buzz never overwrites this file."
        },
        "extends": names.base,
        // Claude Code and Codex keep their login in the macOS keychain.
        // Delete this block if your agents authenticate with API keys.
        "platform_overrides": {
            "macos": {
                "filesystem": {
                    "allow": [keychain],
                    "bypass_protection": [keychain]
                }
            }
        }
    }))
}

fn to_profile_json(profile: &serde_json::Value) -> Result<String, String> {
    serde_json::to_string_pretty(profile)
        .map(|json| json + "\n")
        .map_err(|e| format!("failed to serialize sandbox profile: {e}"))
}

/// Replace `path` atomically so nono never reads a torn base profile.
fn write_replacing(path: &Path, contents: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents).map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("failed to replace {}: {e}", path.display())
    })
}

/// Create `path` with `contents` unless something (file or symlink) is
/// already there. A partially written file is removed so the next launch
/// seeds it again.
fn write_if_absent(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => return Err(format!("failed to create {}: {e}", path.display())),
    };
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|e| {
            let _ = fs::remove_file(path);
            format!("failed to write {}: {e}", path.display())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> SandboxProfileNames {
        SandboxProfileNames {
            base: "buzz-agent".into(),
            local: "buzz-agent-local".into(),
        }
    }

    fn read_json(path: &Path) -> serde_json::Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn profile_names_follow_the_nest_identity() {
        let cases = [
            ("/home/u/.buzz", "buzz-agent"),
            ("/home/u/.buzz-dev", "buzz-dev-agent"),
            ("/home/u/.buzz-demo-acme", "buzz-demo-acme-agent"),
        ];
        for (nest, base) in cases {
            let names = profile_names(Path::new(nest)).unwrap();
            assert_eq!(names.base, base);
            assert_eq!(names.local, format!("{base}-local"));
        }
        assert_eq!(profile_names(Path::new("/home/u/...")), None);
        assert_eq!(profile_names(Path::new("/")), None);
    }

    #[test]
    fn installs_base_and_local_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = dir.path().join("nono/profiles");
        ensure_sandbox_profiles_at(&profiles, &names()).unwrap();

        let base = read_json(&profiles.join("buzz-agent.json"));
        assert_eq!(base["meta"]["name"], "buzz-agent");
        assert_eq!(base["extends"], "default");
        assert_eq!(base["workdir"]["access"], "readwrite");

        let local = read_json(&profiles.join("buzz-agent-local.json"));
        assert_eq!(local["meta"]["name"], "buzz-agent-local");
        assert_eq!(local["extends"], "buzz-agent");
        assert_eq!(
            local["platform_overrides"]["macos"]["filesystem"]["bypass_protection"][0],
            "$HOME/Library/Keychains"
        );
    }

    #[test]
    fn base_profile_never_grants_the_keychain() {
        let base = render_base_profile(&names()).unwrap();
        assert!(!base.contains("bypass_protection"), "{base}");
    }

    #[test]
    fn rewrites_a_stale_base_but_never_the_local_profile() {
        let dir = tempfile::tempdir().unwrap();
        let profiles = dir.path();
        ensure_sandbox_profiles_at(profiles, &names()).unwrap();
        let base_path = profiles.join("buzz-agent.json");
        let local_path = profiles.join("buzz-agent-local.json");
        fs::write(&base_path, "{\"stale\": true}").unwrap();
        fs::write(&local_path, "{\"user\": \"edit\"}").unwrap();

        ensure_sandbox_profiles_at(profiles, &names()).unwrap();

        assert_eq!(read_json(&base_path)["meta"]["name"], "buzz-agent");
        assert_eq!(
            fs::read_to_string(&local_path).unwrap(),
            "{\"user\": \"edit\"}"
        );
        assert!(!profiles.join("buzz-agent.json.tmp").exists());
    }

    #[test]
    fn current_base_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        ensure_sandbox_profiles_at(dir.path(), &names()).unwrap();
        let base_path = dir.path().join("buzz-agent.json");
        let before = fs::metadata(&base_path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));

        ensure_sandbox_profiles_at(dir.path(), &names()).unwrap();

        assert_eq!(
            fs::metadata(&base_path).unwrap().modified().unwrap(),
            before
        );
    }

    #[test]
    fn reset_restores_the_default_local_profile() {
        let dir = tempfile::tempdir().unwrap();
        ensure_sandbox_profiles_at(dir.path(), &names()).unwrap();
        let local_path = dir.path().join("buzz-agent-local.json");
        let seeded = fs::read_to_string(&local_path).unwrap();
        fs::write(&local_path, "{ broken").unwrap();

        reset_local_profile_at(dir.path(), &names()).unwrap();

        assert_eq!(fs::read_to_string(&local_path).unwrap(), seeded);
    }

    #[cfg(unix)]
    #[test]
    fn reset_replaces_a_symlink_without_touching_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dotfiles-profile.json");
        fs::write(&target, "{}").unwrap();
        let local_path = dir.path().join("buzz-agent-local.json");
        std::os::unix::fs::symlink(&target, &local_path).unwrap();

        reset_local_profile_at(dir.path(), &names()).unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "{}");
        assert!(!fs::symlink_metadata(&local_path).unwrap().is_symlink());
        assert_eq!(read_json(&local_path)["extends"], "buzz-agent");
    }

    #[cfg(unix)]
    #[test]
    fn local_profile_symlink_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("dotfiles-profile.json");
        fs::write(&target, "{}").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("buzz-agent-local.json")).unwrap();

        ensure_sandbox_profiles_at(dir.path(), &names()).unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "{}");
    }
}
