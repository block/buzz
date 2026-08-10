//! Per-run filesystem isolation for local managed-agent process trees.
//!
//! macOS Seatbelt is applied to the outer `buzz-acp` process, so every agent,
//! MCP server, shell, and background descendant inherits the same boundary.
//! The run root is fresh for every spawn and removed only after the process
//! tree has exited and the runtime entry is dropped.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Serialize;

use super::FilesystemIsolationProfile;

pub const ISOLATION_ATTESTATION_ENV: &str = "BUZZ_FILESYSTEM_ISOLATION_ATTESTATION";
pub const ISOLATION_RUN_ROOT_ENV: &str = "BUZZ_FILESYSTEM_ISOLATION_RUN_ROOT";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FilesystemIsolationAttestation {
    pub version: u8,
    pub enforcement: &'static str,
    pub identity_pubkey: String,
    pub run_id: String,
    pub run_root: PathBuf,
    pub allowed_read_roots: Vec<PathBuf>,
    pub allowed_write_roots: Vec<PathBuf>,
    pub denied_roots: Vec<PathBuf>,
}

#[derive(Debug)]
pub struct FilesystemIsolationRun {
    root: PathBuf,
    base: PathBuf,
    #[allow(dead_code)] // retained for black-box receipt inspection in tests
    pub attestation: FilesystemIsolationAttestation,
}

impl FilesystemIsolationRun {
    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for FilesystemIsolationRun {
    fn drop(&mut self) {
        if let Err(error) = remove_run_root(&self.base, &self.root) {
            eprintln!(
                "buzz-desktop: failed to remove isolated run root {}: {error}",
                self.root.display()
            );
        }
    }
}

/// Build the outer command and fresh run-root guard for one isolated spawn.
///
/// The returned `Command` launches the existing ACP harness through the host
/// boundary. Callers configure stdio and environment on it exactly as they do
/// for an unisolated harness, then retain the guard for the process lifetime.
pub fn isolated_agent_command(
    profile: &FilesystemIsolationProfile,
    identity_pubkey: &str,
    acp_command: &Path,
) -> Result<(Command, FilesystemIsolationRun), String> {
    let FilesystemIsolationProfile::Ephemeral { read_only_roots } = profile;
    validate_identity(identity_pubkey)?;

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (read_only_roots, acp_command);
        return Err(
            "ephemeral filesystem isolation is currently supported only on macOS".to_string(),
        );
    }

    #[cfg(target_os = "macos")]
    {
        let sandbox_exec = Path::new("/usr/bin/sandbox-exec");
        if !sandbox_exec.is_file() {
            return Err("macOS filesystem isolation requires /usr/bin/sandbox-exec".to_string());
        }

        let (base, run_id, root) = create_run_root(identity_pubkey)?;
        let result = (|| {
            let home = root.join("home");
            let temp = root.join("tmp");
            create_private_dir(&home)?;
            create_private_dir(&temp)?;

            let denied_roots = denied_roots()?;
            let mut allowed_read_roots = system_read_roots();
            allowed_read_roots.extend(validate_read_only_roots(read_only_roots, &denied_roots)?);
            allowed_read_roots.extend(executable_read_roots(acp_command)?);
            allowed_read_roots.push(root.clone());
            normalize_paths(&mut allowed_read_roots);

            let allowed_write_roots = vec![root.clone()];
            let attestation = FilesystemIsolationAttestation {
                version: 1,
                enforcement: "macos_seatbelt_process_tree",
                identity_pubkey: identity_pubkey.to_ascii_lowercase(),
                run_id,
                run_root: root.clone(),
                allowed_read_roots: allowed_read_roots.clone(),
                allowed_write_roots: allowed_write_roots.clone(),
                denied_roots,
            };
            let profile_text = seatbelt_profile(&attestation)?;

            let mut command = Command::new(sandbox_exec);
            command
                .arg("-p")
                .arg(profile_text)
                .arg(acp_command)
                .current_dir(&root)
                .env("HOME", &home)
                .env("TMPDIR", &temp)
                .env("XDG_CACHE_HOME", home.join(".cache"))
                .env("XDG_CONFIG_HOME", home.join(".config"))
                .env("XDG_DATA_HOME", home.join(".local/share"))
                .env(ISOLATION_RUN_ROOT_ENV, &root)
                .env(
                    ISOLATION_ATTESTATION_ENV,
                    serde_json::to_string(&attestation).map_err(|error| {
                        format!("failed to serialize isolation receipt: {error}")
                    })?,
                );

            Ok((
                command,
                FilesystemIsolationRun {
                    root: root.clone(),
                    base: base.clone(),
                    attestation,
                },
            ))
        })();

        if result.is_err() {
            let _ = remove_run_root(&base, &root);
        }
        result
    }
}

/// Validate an owner-authored profile without creating a run root.
pub fn validate_filesystem_isolation_profile(
    profile: &FilesystemIsolationProfile,
) -> Result<(), String> {
    let FilesystemIsolationProfile::Ephemeral { read_only_roots } = profile;
    validate_read_only_roots(read_only_roots, &denied_roots()?).map(|_| ())
}

fn validate_identity(identity_pubkey: &str) -> Result<(), String> {
    if identity_pubkey.len() == 64 && identity_pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("filesystem isolation requires an exact 64-character agent pubkey".to_string())
    }
}

fn create_run_root(identity_pubkey: &str) -> Result<(PathBuf, String, PathBuf), String> {
    let base = std::env::temp_dir().join("buzz-agent-runs");
    if base.exists() {
        let metadata = base
            .symlink_metadata()
            .map_err(|error| format!("failed to inspect isolation root: {error}"))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!("refusing unsafe isolation base {}", base.display()));
        }
    } else {
        create_private_dir(&base)?;
    }

    let run_id = uuid::Uuid::new_v4().simple().to_string();
    let root = base.join(format!(
        "{}-{run_id}",
        &identity_pubkey.to_ascii_lowercase()[..16]
    ));
    create_private_dir(&root)?;
    Ok((base, run_id, root))
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir(path).map_err(|error| {
        format!(
            "failed to create isolated directory {}: {error}",
            path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!(
                "failed to protect isolated directory {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn denied_roots() -> Result<Vec<PathBuf>, String> {
    let home = dirs::home_dir().ok_or_else(|| {
        "filesystem isolation requires a resolvable home directory for fail-closed denial"
            .to_string()
    })?;
    let mut roots = vec![home.join(".buzz")];
    if let Some(nest) = super::nest_dir() {
        roots.push(nest);
    }
    normalize_paths(&mut roots);
    Ok(roots)
}

fn validate_read_only_roots(
    roots: &[PathBuf],
    denied_roots: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let home = dirs::home_dir();
    let mut validated = Vec::with_capacity(roots.len());
    for root in roots {
        if !root.is_absolute() {
            return Err(format!(
                "filesystem isolation read root must be absolute: {}",
                root.display()
            ));
        }
        let canonical = root.canonicalize().map_err(|error| {
            format!(
                "filesystem isolation read root must exist ({}): {error}",
                root.display()
            )
        })?;
        let metadata = canonical.symlink_metadata().map_err(|error| {
            format!(
                "failed to inspect read root {}: {error}",
                canonical.display()
            )
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(format!(
                "filesystem isolation read root must be a real directory: {}",
                canonical.display()
            ));
        }
        if canonical == Path::new("/") || home.as_ref().is_some_and(|path| canonical == *path) {
            return Err(format!(
                "filesystem isolation refuses broad read root {}",
                canonical.display()
            ));
        }
        if denied_roots
            .iter()
            .any(|denied| canonical.starts_with(denied) || denied.starts_with(&canonical))
        {
            return Err(format!(
                "filesystem isolation read root overlaps protected Buzz data: {}",
                canonical.display()
            ));
        }
        validated.push(canonical);
    }
    normalize_paths(&mut validated);
    Ok(validated)
}

#[cfg(target_os = "macos")]
fn system_read_roots() -> Vec<PathBuf> {
    [
        "/System",
        "/usr",
        "/bin",
        "/sbin",
        "/Library",
        "/private/etc",
        "/private/var/db",
        "/dev",
        "/opt",
    ]
    .into_iter()
    .map(PathBuf::from)
    .filter(|path| path.exists())
    .collect()
}

#[cfg(target_os = "macos")]
fn executable_read_roots(command: &Path) -> Result<Vec<PathBuf>, String> {
    let canonical = command.canonicalize().map_err(|error| {
        format!(
            "failed to resolve ACP command {} for isolation: {error}",
            command.display()
        )
    })?;
    let parent = canonical.parent().ok_or_else(|| {
        format!(
            "ACP command has no parent directory: {}",
            canonical.display()
        )
    })?;
    Ok(vec![parent.to_path_buf()])
}

#[cfg(target_os = "macos")]
fn seatbelt_profile(attestation: &FilesystemIsolationAttestation) -> Result<String, String> {
    let mut profile = String::from("(version 1)\n(allow default)\n(deny file-read* file-write*)\n");
    for root in &attestation.allowed_read_roots {
        profile.push_str(&format!(
            "(allow file-read* (subpath \"{}\"))\n",
            escape_seatbelt_path(root)?
        ));
    }
    for root in &attestation.allowed_write_roots {
        profile.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n",
            escape_seatbelt_path(root)?
        ));
    }
    for device in ["/dev/null", "/dev/zero", "/dev/random", "/dev/urandom"] {
        profile.push_str(&format!(
            "(allow file-read* file-write* (literal \"{device}\"))\n"
        ));
    }
    Ok(profile)
}

#[cfg(target_os = "macos")]
fn escape_seatbelt_path(path: &Path) -> Result<String, String> {
    let raw = path
        .to_str()
        .ok_or_else(|| format!("isolation path is not valid UTF-8: {}", path.display()))?;
    if raw.contains(['\n', '\r', '\0']) {
        return Err(format!(
            "isolation path contains invalid bytes: {}",
            path.display()
        ));
    }
    Ok(raw.replace('\\', "\\\\").replace('"', "\\\""))
}

fn normalize_paths(paths: &mut Vec<PathBuf>) {
    paths.sort();
    paths.dedup();
}

fn remove_run_root(base: &Path, root: &Path) -> Result<(), String> {
    if root.parent() != Some(base) || !root.starts_with(base) {
        return Err(format!(
            "refusing to remove unscoped path {}",
            root.display()
        ));
    }
    match root.symlink_metadata() {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(root)
                .map_err(|error| format!("failed to remove {}: {error}", root.display()))
        }
        Ok(_) => Err(format!(
            "refusing to remove non-directory run root {}",
            root.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("failed to inspect {}: {error}", root.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broad_and_buzz_overlapping_read_roots_fail_closed() {
        let denied = denied_roots().unwrap();
        assert!(validate_read_only_roots(&[PathBuf::from("/")], &denied).is_err());
        if let Some(home) = dirs::home_dir() {
            assert!(validate_read_only_roots(std::slice::from_ref(&home), &denied).is_err());
            let buzz = home.join(".buzz");
            if buzz.is_dir() {
                assert!(validate_read_only_roots(&[buzz], &denied).is_err());
            }
        }
    }

    #[test]
    fn invalid_identity_is_rejected_before_creating_a_run_root() {
        let profile = FilesystemIsolationProfile::Ephemeral {
            read_only_roots: Vec::new(),
        };
        let error =
            isolated_agent_command(&profile, "not-a-pubkey", Path::new("/bin/sh")).unwrap_err();
        assert!(error.contains("exact 64-character"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn seatbelt_receipt_never_allows_home_or_shared_buzz_root() {
        let profile = FilesystemIsolationProfile::Ephemeral {
            read_only_roots: Vec::new(),
        };
        let (_command, run) =
            isolated_agent_command(&profile, &"ab".repeat(32), Path::new("/bin/sh")).unwrap();
        let home = dirs::home_dir().unwrap();
        assert!(!run.attestation.allowed_read_roots.contains(&home));
        assert!(!run
            .attestation
            .allowed_read_roots
            .iter()
            .any(|root| root == &home.join(".buzz")));
        assert!(run.attestation.denied_roots.contains(&home.join(".buzz")));
        assert!(run.root().starts_with(std::env::temp_dir()));
    }
}
