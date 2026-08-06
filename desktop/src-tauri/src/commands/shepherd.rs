//! Tauri surface for importing optional Shepherd execution evidence.

use std::{path::PathBuf, process::Command};

use serde::{Deserialize, Serialize};

use crate::managed_agents::shepherd::{normalize_shepherd_export, ShepherdEvidenceEnvelope};

const MAX_COMMAND_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShepherdAdapterStatus {
    installed: bool,
    version: Option<String>,
    supported: bool,
    detail: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShepherdSettlementAction {
    Select,
    Apply,
    Discard,
}

impl ShepherdSettlementAction {
    fn command(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Apply => "apply",
            Self::Discard => "discard",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShepherdSettlementResult {
    action: String,
    source_run_ref: String,
    message: String,
}

fn parse_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| {
            part.trim_start_matches('v')
                .split('.')
                .all(|segment| !segment.is_empty() && segment.chars().all(|c| c.is_ascii_digit()))
                && part.contains('.')
        })
        .map(|part| part.trim_start_matches('v').to_string())
}

fn supported_version(version: &str) -> bool {
    version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u64>().ok())
        == Some(0)
        && version
            .split('.')
            .nth(1)
            .and_then(|minor| minor.parse::<u64>().ok())
            .is_some_and(|minor| minor >= 3)
}

/// Detect a local Shepherd installation without installing or modifying it.
#[tauri::command]
pub async fn shepherd_adapter_status() -> ShepherdAdapterStatus {
    tokio::task::spawn_blocking(
        || match Command::new("shepherd").arg("--version").output() {
            Ok(output) if output.status.success() => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let version = parse_version(&stdout).or_else(|| parse_version(&stderr));
                let supported = version.as_deref().is_some_and(supported_version);
                ShepherdAdapterStatus {
                    installed: true,
                    version,
                    supported,
                    detail: if supported {
                        "Shepherd is available for evidence import and settlement".to_string()
                    } else {
                        "Shepherd is installed, but Buzz requires version 0.3 or newer".to_string()
                    },
                }
            }
            Ok(output) => ShepherdAdapterStatus {
                installed: true,
                version: None,
                supported: false,
                detail: format!("Shepherd version probe exited with {}", output.status),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => ShepherdAdapterStatus {
                installed: false,
                version: None,
                supported: false,
                detail: "Shepherd is not installed; Buzz remains fully functional without it"
                    .to_string(),
            },
            Err(error) => ShepherdAdapterStatus {
                installed: false,
                version: None,
                supported: false,
                detail: format!("Shepherd could not be probed: {error}"),
            },
        },
    )
    .await
    .unwrap_or_else(|error| ShepherdAdapterStatus {
        installed: false,
        version: None,
        supported: false,
        detail: format!("Shepherd probe task failed: {error}"),
    })
}

fn validate_run_ref(value: String) -> Result<String, String> {
    let value = value.trim().to_string();
    if value.is_empty()
        || value.len() > 256
        || value.starts_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-/@-".contains(&byte))
    {
        return Err("sourceRunRef contains unsupported characters".to_string());
    }
    Ok(value)
}

fn bounded_output(output: &[u8]) -> String {
    let keep = output.len().min(MAX_COMMAND_OUTPUT_BYTES);
    String::from_utf8_lossy(&output[..keep]).trim().to_string()
}

/// Execute an explicitly owner-confirmed Shepherd settlement action.
#[tauri::command]
pub async fn settle_shepherd_run(
    workspace_path: String,
    source_run_ref: String,
    action: ShepherdSettlementAction,
) -> Result<ShepherdSettlementResult, String> {
    let source_run_ref = validate_run_ref(source_run_ref)?;
    let workspace = PathBuf::from(workspace_path)
        .canonicalize()
        .map_err(|error| format!("invalid Shepherd workspace: {error}"))?;
    if !workspace.is_dir() {
        return Err("Shepherd workspace must be a directory".to_string());
    }
    if !workspace.join(".vcscore").is_dir() {
        return Err("the selected directory is not an initialized Shepherd workspace".to_string());
    }
    let action_name = action.command().to_string();
    tokio::task::spawn_blocking(move || {
        let output = Command::new("shepherd")
            .args(["run", action.command(), &source_run_ref])
            .current_dir(&workspace)
            .output()
            .map_err(|error| format!("failed to launch Shepherd: {error}"))?;
        if !output.status.success() {
            let detail = bounded_output(&output.stderr)
                .replace(&workspace.display().to_string(), "<workspace>");
            return Err(if detail.is_empty() {
                format!("Shepherd {action_name} failed with {}", output.status)
            } else {
                format!("Shepherd {action_name} failed: {detail}")
            });
        }
        let message =
            bounded_output(&output.stdout).replace(&workspace.display().to_string(), "<workspace>");
        Ok(ShepherdSettlementResult {
            action: action_name,
            source_run_ref,
            message,
        })
    })
    .await
    .map_err(|error| format!("Shepherd settlement task failed: {error}"))?
}

/// Normalize a Shepherd flat JSON trace export without retaining raw payloads.
#[tauri::command]
pub fn normalize_shepherd_trace(
    export_json: String,
    source_run_ref: Option<String>,
) -> Result<ShepherdEvidenceEnvelope, String> {
    normalize_shepherd_export(&export_json, source_run_ref)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_gates_supported_versions() {
        assert_eq!(parse_version("shepherd 0.3.0"), Some("0.3.0".into()));
        assert_eq!(parse_version("shepherd v0.4.2"), Some("0.4.2".into()));
        assert!(supported_version("0.3.0"));
        assert!(!supported_version("0.2.9"));
        assert!(!supported_version("1.0.0"));
    }

    #[test]
    fn run_refs_cannot_inject_arguments_or_shell_syntax() {
        assert!(validate_run_ref("run:abc/123".into()).is_ok());
        assert!(validate_run_ref("--help".into()).is_err());
        assert!(validate_run_ref("abc; touch /tmp/pwned".into()).is_err());
    }
}
