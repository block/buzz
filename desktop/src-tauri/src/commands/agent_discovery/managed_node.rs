use sha2::{Digest, Sha256};
use std::{io::Read, io::Write};

use crate::managed_agents::{is_npm_global_install, InstallStepResult};

const MANAGED_NODE_VERSION: &str = "v24.11.0";
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
    filename: "node-v24.11.0-darwin-arm64.tar.gz",
    sha256: "0be2ab2816a4fa02d1acff014a434f29f56d8d956f5af6a98b70ced6c5f4d201",
});

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "darwin-x64",
    filename: "node-v24.11.0-darwin-x64.tar.gz",
    sha256: "3884671e87f46f773832d98a0a6cabcc5ec4f637084f0f3515b69e66ea27f2f1",
});

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "linux-x64",
    filename: "node-v24.11.0-linux-x64.tar.gz",
    sha256: "b3c071cdf47aab867c3b2aa287257df12ec5d7c962bf922b32fd33226c4295fd",
});

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const MANAGED_NODE_ARTIFACT: Option<ManagedNodeArtifact> = Some(ManagedNodeArtifact {
    platform: "linux-arm64",
    filename: "node-v24.11.0-linux-arm64.tar.gz",
    sha256: "4786d00c4d259d3ff0b2328307f764ef3ced65f2d6e9502d433e68d66238509d",
});

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64")
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

fn managed_node_runtime_ready() -> bool {
    let Some(node) = crate::managed_agents::buzz_managed_node_bin_path() else {
        return false;
    };
    if !node.is_file() {
        return false;
    }
    let output = std::process::Command::new(&node)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim() == MANAGED_NODE_VERSION)
        .unwrap_or(false)
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

    let extracted_dir = temp_dir.join(artifact.filename.trim_end_matches(".tar.gz"));
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
    let response =
        reqwest::blocking::get(url).map_err(|e| format!("download Node.js request failed: {e}"))?;
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
    if !filename.ends_with(".tar.gz") {
        return Err(format!("unsupported managed Node.js archive: {filename}"));
    }
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
mod tests {
    use super::*;

    #[test]
    fn test_npm_eacces_hint_guidance_mentions_buzz_private_dir() {
        let hint = npm_eacces_hint("EACCES: permission denied", "npm install -g foo").unwrap();
        assert!(
            hint.contains("Buzz's private Node tools directory"),
            "hint: {hint}"
        );
    }

    #[test]
    fn test_rewrite_npm_install_uses_private_prefix() {
        assert_eq!(
            rewrite_npm_global_install(
                "npm install -g @agentclientprotocol/codex-acp",
                "'/tmp/Buzz Node'"
            ),
            "npm install --global --prefix '/tmp/Buzz Node' @agentclientprotocol/codex-acp"
        );
    }

    #[test]
    fn test_rewrite_npm_i_uses_private_prefix() {
        assert_eq!(
            rewrite_npm_global_install("npm i -g some-package", "'/tmp/buzz'"),
            "npm i --global --prefix '/tmp/buzz' some-package"
        );
    }

    #[test]
    fn test_rewrite_npm_uninstall_uses_private_prefix() {
        assert_eq!(
            rewrite_npm_global_install("npm uninstall -g @zed-industries/codex-acp", "'/tmp/buzz'"),
            "npm uninstall --global --prefix '/tmp/buzz' @zed-industries/codex-acp"
        );
    }

    #[test]
    fn test_rewrite_ignores_non_global_command() {
        assert_eq!(
            rewrite_npm_global_install("npm install foo", "'/tmp/buzz'"),
            "npm install foo"
        );
    }

    #[test]
    fn test_shell_quote_escapes_single_quotes() {
        assert_eq!(
            shell_quote(std::path::Path::new("/tmp/Buzz's Node")),
            "'/tmp/Buzz'\\''s Node'"
        );
    }
}
