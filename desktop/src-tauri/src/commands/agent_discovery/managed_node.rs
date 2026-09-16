use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use std::{io::Read, io::Write};

use crate::managed_agents::{is_npm_global_install, InstallStepResult};

const MANAGED_NODE_VERSION: &str = "v24.18.0";
const MANAGED_NODE_MAX_BYTES: u64 = 90 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct ManagedNodeArtifact {
    platform: &'static str,
    filename: &'static str,
    sha256: &'static str,
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "darwin-arm64",
    filename: "node-v24.18.0-darwin-arm64.tar.gz",
    sha256: "e1a97e14c99c803e96c7339403282ea05a499c32f8d83defe9ef5ec66f979ed1",
});

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "darwin-x64",
    filename: "node-v24.18.0-darwin-x64.tar.gz",
    sha256: "dfd0dbd3e721503434df7b7205e719f61b3a3a31b2bcf9729b8b91fea240f080",
});

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "linux-x64",
    filename: "node-v24.18.0-linux-x64.tar.gz",
    sha256: "783130984963db7ba9cbd01089eaf2c2efb055c7c1693c943174b967b3050cb8",
});

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "linux-arm64",
    filename: "node-v24.18.0-linux-arm64.tar.gz",
    sha256: "6b4484c2190274175df9aa8f28e2d758a819cb1c1fe6ab481e2f95b463ab8508",
});

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "win-x64",
    filename: "node-v24.18.0-win-x64.zip",
    sha256: "0ae68406b42d7725661da979b1403ec9926da205c6770827f33aac9d8f26e821",
});

#[cfg(all(target_os = "windows", target_arch = "aarch64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "win-arm64",
    filename: "node-v24.18.0-win-arm64.zip",
    sha256: "f274669adb93b1fd0fbf8f21fd078609e9dcc84333d4f2718d2dde3f9a161a01",
});

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "windows", target_arch = "aarch64")
)))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = None;

fn managed_node_unsupported_step() -> InstallStepResult {
    InstallStepResult {
        step: "node".to_string(),
        command: "managed Node.js runtime".to_string(),
        success: false,
        stdout: String::new(),
        stderr: format!(
            "Buzz does not provide a managed Node.js runtime for {}-{} yet",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
        exit_code: None,
        hint: Some(
            "Install Node.js from https://nodejs.org, restart Buzz, then click Install again."
                .to_string(),
        ),
    }
}

fn managed_node_install_hint() -> String {
    "Buzz could not install its private Node.js runtime. Check your network and app-data directory permissions, then click Install again.".to_string()
}

fn managed_node_failed_step(stderr: String) -> InstallStepResult {
    InstallStepResult {
        step: "node".to_string(),
        command: "managed Node.js runtime".to_string(),
        success: false,
        stdout: String::new(),
        stderr,
        exit_code: None,
        hint: Some(managed_node_install_hint()),
    }
}

pub(super) fn managed_node_runtime_ready() -> bool {
    let Some(node) = crate::managed_agents::buzz_managed_node_bin_path() else {
        return false;
    };
    if !node.is_file() {
        return false;
    }
    probe_node(&node, MANAGED_NODE_VERSION, Duration::from_secs(3))
}

/// Run `executable --version` with a bounded deadline and return `true` only
/// when it exits 0 and its trimmed stdout equals `expected_version`.
///
/// Transport: stdout is redirected to a temp file so no exit path can block on
/// an inherited handle (a descendant retaining a pipe write-end would otherwise
/// prevent EOF indefinitely).
///
/// Cleanup: the child runs in its own process group on Unix (`process_group(0)`)
/// so an unconditional group SIGKILL on every exit path terminates all
/// descendants.  On Windows, `terminate_process` issues `taskkill /T /F` for
/// tree-wide cleanup.  SIGKILL to an already-dead group returns ESRCH (no-op).
pub(super) fn probe_node(
    executable: &std::path::Path,
    expected_version: &str,
    timeout: Duration,
) -> bool {
    let tmp = match tempfile::NamedTempFile::new() {
        Ok(f) => f,
        Err(_) => return false,
    };
    let out_file = match tmp.reopen() {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut cmd = std::process::Command::new(executable);
    cmd.arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(out_file))
        .stderr(std::process::Stdio::null());
    crate::util::configure_no_window(&mut cmd);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let Ok(mut child) = cmd.spawn() else {
        return false;
    };

    let deadline = std::time::Instant::now() + timeout;
    let exit_status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    kill_probe_group(child.id());
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                kill_probe_group(child.id());
                let _ = child.wait();
                return false;
            }
        }
    };

    // Group-kill unconditionally: SIGKILL to a dead group is ESRCH (no-op).
    kill_probe_group(child.id());

    if !exit_status.success() {
        return false;
    }

    let mut output = String::new();
    if std::io::Read::read_to_string(&mut tmp.as_file(), &mut output).is_err() {
        return false;
    }
    output.trim() == expected_version
}

/// Kill the probe's process group/tree unconditionally (no TERM grace — this
/// is a probe, not an agent session).  ESRCH on a dead group is fine.
fn kill_probe_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = crate::managed_agents::terminate_process(pid);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

/// Returns `true` when the managed Node runtime is absent or no longer executes —
/// meaning any existing npm adapter shims are broken and must be reinstalled.
///
/// This fires when the pinned Node version changes (e.g. v24.11.0 → v24.18.0):
/// the old dir stays on disk, shims appear installed, but they fail at run time
/// because the Node binary they reference is gone.  Treating the adapter as
/// missing forces `ensure_managed_node_runtime_blocking` to re-download Node and
/// npm to reinstall the shims.
pub(super) fn managed_node_orphaned() -> bool {
    managed_node_runtime_supported() && !managed_node_runtime_ready()
}

/// Returns `true` when an adapter at `resolved` should be invalidated.
///
/// Only a Buzz-managed shim (path under `managed_prefix`) with an orphaned
/// runtime is invalidated; external adapters are always preserved.
pub(super) fn should_invalidate_adapter(
    resolved: &std::path::Path,
    managed_prefix: &std::path::Path,
    orphaned: bool,
) -> bool {
    orphaned && resolved.starts_with(managed_prefix)
}

/// Resolve the adapter binary path, accounting for the Node-orphan case.
/// Resolves first; invalidates only managed-prefix shims when Node is orphaned.
pub(super) fn resolve_adapter_path(
    commands: &[&str],
    adapter_install_commands: &[&str],
) -> Option<std::path::PathBuf> {
    let resolved = commands
        .iter()
        .find_map(|cmd| crate::managed_agents::resolve_command(cmd));

    let needs_managed_npm = adapter_install_commands
        .iter()
        .any(|cmd| is_npm_global_install(cmd));
    if needs_managed_npm {
        if let (Some(ref path), Some(ref managed_bin)) =
            (&resolved, crate::managed_agents::buzz_managed_npm_bin_dir())
        {
            if should_invalidate_adapter(path, managed_bin, managed_node_orphaned()) {
                return None;
            }
        }
    }

    resolved
}

fn managed_node_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) fn managed_node_runtime_supported() -> bool {
    MANAGED_NODE_ARTIFACT.is_some() && crate::managed_agents::buzz_managed_node_bin_dir().is_some()
}

pub(super) fn ensure_managed_node_runtime_blocking() -> Result<(), Box<InstallStepResult>> {
    if managed_node_runtime_ready() {
        return Ok(());
    }

    let Some(artifact) = MANAGED_NODE_ARTIFACT else {
        return Err(Box::new(managed_node_unsupported_step()));
    };
    let Some(root) = crate::managed_agents::buzz_managed_node_root() else {
        return Err(Box::new(managed_node_failed_step(
            "failed to resolve Buzz app-data directory for private Node.js runtime".to_string(),
        )));
    };

    let _guard = managed_node_install_lock().lock().map_err(|_| {
        Box::new(managed_node_failed_step(
            "managed Node.js install lock poisoned".to_string(),
        ))
    })?;

    if managed_node_runtime_ready() {
        return Ok(());
    }

    install_managed_node_runtime(&root, artifact)
        .map_err(|err| Box::new(managed_node_failed_step(err)))?;
    if managed_node_runtime_ready() {
        Ok(())
    } else {
        Err(Box::new(managed_node_failed_step(
            "managed Node.js runtime did not pass readiness after install".to_string(),
        )))
    }
}

fn install_managed_node_runtime(
    root: &std::path::Path,
    artifact: ManagedNodeArtifact,
) -> Result<(), String> {
    let final_dir = root.join(MANAGED_NODE_VERSION).join(artifact.platform);
    let temp_dir = root.join(format!(
        "{}.{}.tmp",
        MANAGED_NODE_VERSION, artifact.platform
    ));
    let archive_path = root.join(format!("{}.download", artifact.filename));

    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir).map_err(|e| format!("remove stale temp dir: {e}"))?;
    }
    std::fs::create_dir_all(root).map_err(|e| format!("create runtime root: {e}"))?;
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create runtime version dir: {e}"))?;
    }

    let url = format!(
        "https://nodejs.org/dist/{MANAGED_NODE_VERSION}/{}",
        artifact.filename
    );
    download_managed_node_archive(&url, &archive_path, artifact.sha256)?;

    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("create temp dir: {e}"))?;
    extract_managed_node_archive(&archive_path, &temp_dir, artifact.filename)?;
    let _ = std::fs::remove_file(&archive_path);

    let extracted_dir = temp_dir.join(
        artifact
            .filename
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".zip"),
    );
    let source_dir = if extracted_dir.is_dir() {
        extracted_dir
    } else {
        temp_dir.clone()
    };
    verify_node_tree(&source_dir)?;

    let old_dir = final_dir.with_extension("old");
    if old_dir.exists() {
        std::fs::remove_dir_all(&old_dir).map_err(|e| format!("remove stale old dir: {e}"))?;
    }
    if final_dir.exists() {
        std::fs::rename(&final_dir, &old_dir)
            .map_err(|e| format!("stage previous runtime: {e}"))?;
    }
    if let Err(error) = std::fs::rename(&source_dir, &final_dir) {
        if old_dir.exists() {
            let _ = std::fs::rename(&old_dir, &final_dir);
        }
        return Err(format!("install runtime: {error}"));
    }
    let _ = std::fs::remove_dir_all(&old_dir);
    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}

fn download_managed_node_archive(
    url: &str,
    dest: &std::path::Path,
    expected_sha256: &str,
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5 * 60))
        .build()
        .map_err(|e| format!("build Node.js download client: {e}"))?;
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("download Node.js request failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "download Node.js HTTP {}: {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("unknown")
        ));
    }
    if let Some(total) = response.content_length() {
        if total > MANAGED_NODE_MAX_BYTES {
            return Err(format!(
                "download Node.js too large: {total} bytes (max {MANAGED_NODE_MAX_BYTES})"
            ));
        }
    }

    let mut response = response;
    let mut file =
        std::fs::File::create(dest).map_err(|e| format!("create Node.js archive: {e}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("download Node.js stream error: {e}"))?;
        if read == 0 {
            break;
        }
        downloaded += read as u64;
        if downloaded > MANAGED_NODE_MAX_BYTES {
            let _ = std::fs::remove_file(dest);
            return Err(format!(
                "download Node.js exceeded max size: {downloaded} bytes (max {MANAGED_NODE_MAX_BYTES})"
            ));
        }
        file.write_all(&buffer[..read])
            .map_err(|e| format!("write Node.js archive: {e}"))?;
        hasher.update(&buffer[..read]);
    }
    file.flush()
        .map_err(|e| format!("flush Node.js archive: {e}"))?;

    let actual = hex::encode(hasher.finalize());
    if actual != expected_sha256 {
        let _ = std::fs::remove_file(dest);
        return Err(format!(
            "download Node.js hash mismatch: expected {expected_sha256}, got {actual}"
        ));
    }
    Ok(())
}

fn extract_managed_node_archive(
    archive_path: &std::path::Path,
    dest_dir: &std::path::Path,
    filename: &str,
) -> Result<(), String> {
    if filename.ends_with(".tar.gz") {
        let file =
            std::fs::File::open(archive_path).map_err(|e| format!("open Node.js archive: {e}"))?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        validate_managed_node_archive_entries(&mut archive)?;

        let file = std::fs::File::open(archive_path)
            .map_err(|e| format!("open Node.js archive for extraction: {e}"))?;
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(dest_dir)
            .map_err(|e| format!("extract Node.js archive: {e}"))
    } else if filename.ends_with(".zip") {
        let file =
            std::fs::File::open(archive_path).map_err(|e| format!("open Node.js archive: {e}"))?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|e| format!("read Node.js zip archive: {e}"))?;
        validate_managed_node_zip_entries(&archive)?;
        extract_managed_node_zip(&mut archive, dest_dir)
    } else {
        Err(format!("unsupported managed Node.js archive: {filename}"))
    }
}

/// Validate ZIP entry names using platform-neutral string logic.
///
/// `std::path::Path` is intentionally avoided: its `is_absolute()` and
/// `Component` parsing use BUILD-HOST grammar, so `/etc/passwd` is not
/// `is_absolute()` on Windows (no drive prefix), causing the check to lie on
/// the platform this guard exists to protect.  Instead we apply pure string
/// rules that produce identical results on every host:
///
/// - Unix-rooted: starts with `/`
/// - Windows-rooted: starts with `\`, has a drive prefix (`X:`), or is UNC
///   (`\\` / `//`)
/// - Traversal: any component that is `..` when split on EITHER `/` or `\`
fn validate_managed_node_zip_entries(
    archive: &zip::ZipArchive<std::fs::File>,
) -> Result<(), String> {
    for i in 0..archive.len() {
        let name = archive
            .name_for_index(i)
            .ok_or_else(|| format!("Node.js zip entry {i}: missing name"))?;

        // Absolute-path checks (platform-neutral).
        if name.starts_with('/') || name.starts_with('\\') {
            return Err(format!("Node.js zip contains absolute path: {name}"));
        }
        // Drive prefix: one ASCII letter followed by ':'
        if name.len() >= 2 && name.as_bytes()[1] == b':' && name.as_bytes()[0].is_ascii_alphabetic()
        {
            return Err(format!("Node.js zip contains absolute path: {name}"));
        }
        // UNC prefix: // or \\ (covered by starts_with checks above for \\,
        // and // is caught by starts_with('/') then a second '/' — belt + suspenders).
        // (Already caught by the starts_with checks above; explicit for clarity.)

        // Traversal: split on both separators and check each component.
        let has_traversal = name.split(['/', '\\']).any(|component| component == "..");
        if has_traversal {
            return Err(format!("Node.js zip contains path traversal: {name}"));
        }
    }
    Ok(())
}

fn extract_managed_node_zip(
    archive: &mut zip::ZipArchive<std::fs::File>,
    dest_dir: &std::path::Path,
) -> Result<(), String> {
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Node.js zip entry {i}: {e}"))?;
        let outpath = match entry.enclosed_name() {
            Some(p) => dest_dir.join(p),
            None => {
                return Err(format!(
                    "Node.js zip contains unsafe path: {}",
                    entry.name()
                ))
            }
        };
        if entry.is_dir() {
            std::fs::create_dir_all(&outpath)
                .map_err(|e| format!("create dir in Node.js zip: {e}"))?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create parent dir in Node.js zip: {e}"))?;
            }
            let mut out = std::fs::File::create(&outpath)
                .map_err(|e| format!("create file in Node.js zip: {e}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("extract file in Node.js zip: {e}"))?;
        }
    }
    Ok(())
}

fn validate_managed_node_archive_entries<R: std::io::Read>(
    archive: &mut tar::Archive<R>,
) -> Result<(), String> {
    let entries = archive
        .entries()
        .map_err(|e| format!("read Node.js archive entries: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Node.js archive entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("Node.js archive entry path: {e}"))?;
        let path_str = path.to_string_lossy();
        if path.is_absolute() {
            return Err(format!(
                "Node.js archive contains absolute path: {path_str}"
            ));
        }
        if path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(format!(
                "Node.js archive contains path traversal: {path_str}"
            ));
        }
    }
    Ok(())
}

fn verify_node_tree(dir: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        // Windows zip layout: node.exe + npm.cmd + npm (POSIX sh shim) at archive root
        let node = dir.join("node.exe");
        let npm_cmd = dir.join("npm.cmd");
        let npm = dir.join("npm");
        if !node.is_file() {
            return Err("Node.js archive missing node.exe".to_string());
        }
        if !npm_cmd.is_file() {
            return Err("Node.js archive missing npm.cmd".to_string());
        }
        if !npm.is_file() {
            return Err("Node.js archive missing npm".to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        // Unix tarball layout: bin/node + bin/npm
        let node = dir.join("bin").join("node");
        let npm = dir.join("bin").join("npm");
        if !node.is_file() {
            return Err("Node.js archive missing bin/node".to_string());
        }
        if !npm.is_file() {
            return Err("Node.js archive missing bin/npm".to_string());
        }
        Ok(())
    }
}

// ── managed npm adapter installs ──────────────────────────────────────────────

/// Guidance text shown when the Buzz-private npm prefix is not available.
fn managed_npm_prefix_hint() -> String {
    "Buzz could not create its private Node tools directory. Check app-data directory permissions, restart Buzz, then click Install again.".to_string()
}

pub(super) fn managed_npm_command(command: &str) -> Result<Option<String>, Box<InstallStepResult>> {
    if !is_npm_global_install(command) {
        return Ok(None);
    }

    let Some(prefix) = crate::managed_agents::buzz_managed_npm_prefix() else {
        return Err(Box::new(InstallStepResult {
            step: "adapter".to_string(),
            command: command.to_string(),
            success: false,
            stdout: String::new(),
            stderr: "failed to resolve Buzz app-data directory for private npm prefix".to_string(),
            exit_code: None,
            hint: Some(managed_npm_prefix_hint()),
        }));
    };
    if let Err(error) = std::fs::create_dir_all(&prefix) {
        return Err(Box::new(InstallStepResult {
            step: "adapter".to_string(),
            command: command.to_string(),
            success: false,
            stdout: String::new(),
            stderr: format!(
                "failed to create Buzz private npm prefix '{}': {error}",
                prefix.display()
            ),
            exit_code: None,
            hint: Some(managed_npm_prefix_hint()),
        }));
    }

    let prefix_arg = shell_quote(&prefix);
    Ok(Some(rewrite_npm_global_install(command, &prefix_arg)))
}

fn rewrite_npm_global_install(command: &str, quoted_prefix: &str) -> String {
    let trimmed = command.trim_start();
    if let Some(rest) = trimmed.strip_prefix("npm install -g ") {
        format!("npm install --global --prefix {quoted_prefix} {rest}")
    } else if let Some(rest) = trimmed.strip_prefix("npm i -g ") {
        format!("npm i --global --prefix {quoted_prefix} {rest}")
    } else if let Some(rest) = trimmed.strip_prefix("npm uninstall -g ") {
        format!("npm uninstall --global --prefix {quoted_prefix} {rest}")
    } else {
        trimmed.to_string()
    }
}

fn shell_quote(path: &std::path::Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Inspect `stderr` for known npm EACCES patterns and return actionable
/// guidance if matched, or `None` when the error is unrelated.
pub(super) fn npm_eacces_hint(stderr: &str, _command: &str) -> Option<String> {
    if stderr.contains("EACCES: permission denied") || stderr.contains("npm error EACCES") {
        Some(
            "npm could not write to Buzz's private Node tools directory. Check app-data directory permissions, restart Buzz, then click Install again."
                .to_string(),
        )
    } else {
        None
    }
}

// ── end managed npm adapter installs ──────────────────────────────────────────

#[cfg(test)]
#[path = "managed_node_tests.rs"]
mod tests;
