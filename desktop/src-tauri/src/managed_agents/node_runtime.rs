//! Doctor check for the bundled ACP bridges' Node.js runtime requirement.
//!
//! The bundled bridges are shell shims that `exec node` (see
//! `desktop/scripts/lib/acp-node-wrapper.sh`); without a suitable Node.js on
//! the spawn PATH the first agent session dies with a bare exit 127. This
//! check surfaces the requirement in the Doctor panel ahead of time. The
//! manifest (`resources/acp/node-runtime.json`) is written at staging time by
//! `desktop/scripts/prepare-acp-tools-resource.sh`, one entry per npm-sourced
//! bridge, each carrying its own required Node major.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const NODE_RUNTIME_FIX_URL: &str = "https://nodejs.org/en/download";
const NODE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// On-disk shape of `resources/acp/node-runtime.json`. Each tool carries its
/// own required Node major so bridges with different engine ranges are
/// checked independently.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeRuntimeManifest {
    #[serde(default)]
    tools: Vec<NodeRuntimeTool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeRuntimeTool {
    binary: String,
    #[serde(default)]
    node_engine: Option<String>,
    required_node_major: u32,
}

impl NodeRuntimeTool {
    fn requirement_label(&self) -> String {
        self.node_engine
            .clone()
            .unwrap_or_else(|| format!(">={}", self.required_node_major))
    }
}

/// Wire type for the Doctor panel (snake_case JSON like the rest of the
/// commands surface).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NodeRuntimeCheck {
    pub status: NodeRuntimeCheckStatus,
    pub message: String,
    pub manifest_path: String,
    pub node_path: Option<String>,
    pub node_version: Option<String>,
    pub requirements: Vec<NodeRuntimeRequirement>,
    pub fix_url: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeRuntimeCheckStatus {
    Pass,
    Warn,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NodeRuntimeRequirement {
    pub binary: String,
    pub requirement: String,
    pub verdict: NodeRequirementVerdict,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeRequirementVerdict {
    Satisfied,
    Unmet,
    Unknown,
}

enum NodeRuntimeManifestState {
    /// No manifest next to the bundled tools dir: no npm-sourced bridges are
    /// bundled, so the check stays silent.
    Missing,
    Invalid {
        path: PathBuf,
        error: String,
    },
    Loaded {
        path: PathBuf,
        manifest: NodeRuntimeManifest,
    },
}

fn load_node_runtime_manifest(bundled_bin_dir: Option<&Path>) -> NodeRuntimeManifestState {
    let Some(path) = bundled_bin_dir.and_then(super::acp_tools::node_runtime_manifest_path) else {
        return NodeRuntimeManifestState::Missing;
    };
    let contents = match std::fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return NodeRuntimeManifestState::Missing;
        }
        Err(error) => {
            return NodeRuntimeManifestState::Invalid {
                path,
                error: format!("failed to read manifest: {error}"),
            };
        }
    };
    match serde_json::from_slice::<NodeRuntimeManifest>(&contents) {
        Ok(manifest) => NodeRuntimeManifestState::Loaded { path, manifest },
        Err(error) => NodeRuntimeManifestState::Invalid {
            path,
            error: format!("failed to parse manifest JSON: {error}"),
        },
    }
}

/// Surface the bundled ACP bridges' Node.js runtime requirement in the Doctor
/// panel instead of letting the first session spawn die with a bare exit 127.
/// Returns `None` when no npm-sourced bridges are bundled; an unreadable
/// manifest warns instead of silently hiding a packaging break.
pub(crate) async fn run_node_runtime_check() -> Option<NodeRuntimeCheck> {
    let bundled_dir = super::acp_tools::bundled_acp_tools_dir();
    let (manifest_path, manifest) = match load_node_runtime_manifest(bundled_dir.as_deref()) {
        NodeRuntimeManifestState::Missing => return None,
        NodeRuntimeManifestState::Invalid { path, error } => {
            return Some(NodeRuntimeCheck {
                status: NodeRuntimeCheckStatus::Warn,
                message: format!(
                    "Bundled ACP bridge Node.js manifest is unreadable; bridge runtime \
                     requirements cannot be verified ({error})"
                ),
                manifest_path: path.display().to_string(),
                node_path: None,
                node_version: None,
                requirements: Vec::new(),
                fix_url: NODE_RUNTIME_FIX_URL.to_string(),
            });
        }
        NodeRuntimeManifestState::Loaded { path, manifest } => (path, manifest),
    };
    if manifest.tools.is_empty() {
        return None;
    }

    // Resolve node from the same augmented PATH the agent spawn and the CLI
    // auth probes use, so this check cannot disagree with what the bundled
    // wrapper shims will find at spawn time. When no augmented PATH can be
    // built, spawned children inherit the process PATH — mirror that too.
    let path_value = super::readiness::cli_probe::augmented_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    let node_path = resolve_node_from_path_value(&path_value);
    let node_version = match node_path.as_deref() {
        Some(path) => query_node_version(path).await,
        None => None,
    };

    Some(build_node_runtime_check(
        &manifest_path,
        &manifest.tools,
        node_path,
        node_version,
    ))
}

fn resolve_node_from_path_value(path_value: &str) -> Option<String> {
    let file_name = super::discovery::executable_basename("node");
    std::env::split_paths(path_value)
        .map(|dir| dir.join(&file_name))
        .find(|candidate| super::discovery::is_executable_file(candidate))
        .map(|path| path.display().to_string())
}

async fn query_node_version(node_path: &str) -> Option<String> {
    let mut command = tokio::process::Command::new(node_path);
    command
        .args(["-p", "process.versions.node"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = tokio::time::timeout(NODE_PROBE_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(String::from)
}

fn parse_node_major(version: &str) -> Option<u32> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

fn node_requirement_summary<'a>(tools: impl IntoIterator<Item = &'a NodeRuntimeTool>) -> String {
    tools
        .into_iter()
        .map(|tool| format!("{} needs Node.js {}", tool.binary, tool.requirement_label()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn build_node_runtime_check(
    manifest_path: &Path,
    tools: &[NodeRuntimeTool],
    node_path: Option<String>,
    node_version: Option<String>,
) -> NodeRuntimeCheck {
    let node_major = node_version.as_deref().and_then(parse_node_major);
    let unmet: Vec<&NodeRuntimeTool> = match node_major {
        Some(major) => tools
            .iter()
            .filter(|tool| major < tool.required_node_major)
            .collect(),
        None => Vec::new(),
    };

    let (status, message) = if node_path.is_none() {
        (
            NodeRuntimeCheckStatus::Warn,
            format!(
                "Node.js was not found on PATH; bundled ACP bridges require it ({})",
                node_requirement_summary(tools)
            ),
        )
    } else if node_major.is_none() {
        (
            NodeRuntimeCheckStatus::Warn,
            format!(
                "Could not determine the Node.js version; bundled ACP bridges require it ({})",
                node_requirement_summary(tools)
            ),
        )
    } else if unmet.is_empty() {
        (
            NodeRuntimeCheckStatus::Pass,
            format!(
                "Node.js {} satisfies the bundled ACP bridge requirements",
                node_version.as_deref().unwrap_or("unknown")
            ),
        )
    } else {
        (
            NodeRuntimeCheckStatus::Warn,
            format!(
                "Node.js {} is too old for bundled ACP bridges: {}",
                node_version.as_deref().unwrap_or("unknown"),
                node_requirement_summary(unmet.iter().copied())
            ),
        )
    };

    let requirements = tools
        .iter()
        .map(|tool| NodeRuntimeRequirement {
            binary: tool.binary.clone(),
            requirement: tool.requirement_label(),
            verdict: match node_major {
                Some(major) if major >= tool.required_node_major => {
                    NodeRequirementVerdict::Satisfied
                }
                Some(_) => NodeRequirementVerdict::Unmet,
                None => NodeRequirementVerdict::Unknown,
            },
        })
        .collect();

    NodeRuntimeCheck {
        status,
        message,
        manifest_path: manifest_path.display().to_string(),
        node_path,
        node_version,
        requirements,
        fix_url: NODE_RUNTIME_FIX_URL.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_node_runtime_check, load_node_runtime_manifest, parse_node_major,
        NodeRequirementVerdict, NodeRuntimeCheckStatus, NodeRuntimeManifestState, NodeRuntimeTool,
    };
    use std::path::Path;

    fn tool(binary: &str, node_engine: Option<&str>, required_node_major: u32) -> NodeRuntimeTool {
        NodeRuntimeTool {
            binary: binary.to_string(),
            node_engine: node_engine.map(str::to_string),
            required_node_major,
        }
    }

    #[test]
    fn parse_node_major_handles_plain_and_v_prefixed_versions() {
        assert_eq!(parse_node_major("22.11.0"), Some(22));
        assert_eq!(parse_node_major("v20.1.2"), Some(20));
        assert_eq!(parse_node_major(" 18.0.0 \n"), Some(18));
        assert_eq!(parse_node_major("not-a-version"), None);
        assert_eq!(parse_node_major(""), None);
    }

    #[test]
    fn requirement_label_falls_back_to_required_major() {
        assert_eq!(tool("codex-acp", None, 22).requirement_label(), ">=22");
        assert_eq!(
            tool("claude-agent-acp", Some(">=18"), 18).requirement_label(),
            ">=18"
        );
    }

    #[test]
    fn node_missing_warns_with_unknown_verdicts() {
        let tools = vec![tool("claude-agent-acp", Some(">=18"), 18)];
        let check =
            build_node_runtime_check(Path::new("/acp/node-runtime.json"), &tools, None, None);
        assert_eq!(check.status, NodeRuntimeCheckStatus::Warn);
        assert!(
            check.message.contains("not found on PATH"),
            "{}",
            check.message
        );
        assert!(check
            .message
            .contains("claude-agent-acp needs Node.js >=18"));
        assert_eq!(check.requirements.len(), 1);
        assert_eq!(
            check.requirements[0].verdict,
            NodeRequirementVerdict::Unknown
        );
    }

    #[test]
    fn unparseable_version_warns() {
        let tools = vec![tool("codex-acp", None, 20)];
        let check = build_node_runtime_check(
            Path::new("/acp/node-runtime.json"),
            &tools,
            Some("/usr/local/bin/node".to_string()),
            Some("garbage".to_string()),
        );
        assert_eq!(check.status, NodeRuntimeCheckStatus::Warn);
        assert!(
            check.message.contains("Could not determine"),
            "{}",
            check.message
        );
        assert_eq!(
            check.requirements[0].verdict,
            NodeRequirementVerdict::Unknown
        );
    }

    #[test]
    fn old_node_warns_listing_only_unmet_tools() {
        let tools = vec![
            tool("claude-agent-acp", Some(">=18"), 18),
            tool("codex-acp", Some(">=22"), 22),
        ];
        let check = build_node_runtime_check(
            Path::new("/acp/node-runtime.json"),
            &tools,
            Some("/usr/local/bin/node".to_string()),
            Some("20.10.0".to_string()),
        );
        assert_eq!(check.status, NodeRuntimeCheckStatus::Warn);
        assert!(
            check.message.contains("codex-acp needs Node.js >=22"),
            "{}",
            check.message
        );
        assert!(
            !check.message.contains("claude-agent-acp"),
            "satisfied tools must not appear in the warn summary: {}",
            check.message
        );
        assert_eq!(
            check.requirements[0].verdict,
            NodeRequirementVerdict::Satisfied
        );
        assert_eq!(check.requirements[1].verdict, NodeRequirementVerdict::Unmet);
    }

    #[test]
    fn new_enough_node_passes() {
        let tools = vec![
            tool("claude-agent-acp", Some(">=18"), 18),
            tool("codex-acp", Some(">=22"), 22),
        ];
        let check = build_node_runtime_check(
            Path::new("/acp/node-runtime.json"),
            &tools,
            Some("/usr/local/bin/node".to_string()),
            Some("22.11.0".to_string()),
        );
        assert_eq!(check.status, NodeRuntimeCheckStatus::Pass);
        assert!(check
            .requirements
            .iter()
            .all(|req| req.verdict == NodeRequirementVerdict::Satisfied),);
    }

    #[test]
    fn manifest_missing_when_no_bin_dir_or_file() {
        assert!(matches!(
            load_node_runtime_manifest(None),
            NodeRuntimeManifestState::Missing
        ));
        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        assert!(matches!(
            load_node_runtime_manifest(Some(&bin_dir)),
            NodeRuntimeManifestState::Missing
        ));
    }

    #[test]
    fn manifest_invalid_when_unparseable() {
        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        std::fs::write(temp.path().join("node-runtime.json"), "not json").expect("write manifest");
        let NodeRuntimeManifestState::Invalid { error, .. } =
            load_node_runtime_manifest(Some(&bin_dir))
        else {
            panic!("expected Invalid state");
        };
        assert!(error.contains("parse"), "{error}");
    }

    #[test]
    fn manifest_loads_the_staging_script_shape() {
        // Mirrors the exact JSON prepare-acp-tools-resource.sh writes.
        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).expect("bin dir");
        std::fs::write(
            temp.path().join("node-runtime.json"),
            r#"{
  "tools": [
    {
      "id": "claude-agent-acp",
      "binary": "claude-agent-acp",
      "nodeEngine": ">=18.0.0",
      "requiredNodeMajor": 18
    },
    {
      "id": "codex-acp",
      "binary": "codex-acp",
      "nodeEngine": ">=22",
      "requiredNodeMajor": 22
    }
  ]
}
"#,
        )
        .expect("write manifest");
        let NodeRuntimeManifestState::Loaded { manifest, .. } =
            load_node_runtime_manifest(Some(&bin_dir))
        else {
            panic!("expected Loaded state");
        };
        assert_eq!(manifest.tools.len(), 2);
        assert_eq!(manifest.tools[0].binary, "claude-agent-acp");
        assert_eq!(manifest.tools[0].requirement_label(), ">=18.0.0");
        assert_eq!(manifest.tools[1].required_node_major, 22);
    }
}
