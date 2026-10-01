#![deny(unsafe_code)]
//! Desktop Agent Integration & 1-Click Harness Hub Tauri IPC commands.
//!
//! Exposes:
//! - `detect_agent_harnesses`: Scans workstation for 9 coding agent harnesses.
//! - `connect_harness`: Injects Orbit MCP server (`buzz-mcp`) and universal skill into harness configuration.
//! - `disconnect_harness`: Removes Orbit MCP server registration and skill cleanly.

use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Status of an AI coding agent harness on the developer's system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessStatus {
    /// Configured and connected to Orbit central brain.
    Connected,
    /// Detected on the machine but not yet wired to Orbit.
    Detected,
    /// Not found on the developer's system.
    NotInstalled,
}

/// Metadata and status for an agent harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessInfo {
    /// Unique identifier: "antigravity", "claude_code", "cursor", "codex", "goose", "opencode", "zcode", "agy_cli", "kimi".
    pub id: String,
    /// Display name.
    pub name: String,
    /// Current connection status.
    pub status: HarnessStatus,
    /// Target MCP configuration file path.
    pub config_path: String,
    /// Target skill or instruction file path.
    pub skill_path: String,
    /// Short description of the coding agent.
    pub description: String,
    /// Lucide icon name or identifier.
    pub icon: String,
    /// Documentation or setup link.
    pub docs_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFormat {
    JsonMcp,
    GooseYaml,
}

struct HarnessSpec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    icon: &'static str,
    docs_url: &'static str,
    format: ConfigFormat,
    detect_dirs: &'static [&'static str],
    config_rel: &'static str,
    skill_rel: &'static str,
}

const SUPPORTED_HARNESSES: &[HarnessSpec] = &[
    HarnessSpec {
        id: "antigravity",
        name: "Antigravity IDE",
        description: "Google DeepMind pair programming assistant with deep reasoning and agentic workflows.",
        icon: "sparkles",
        docs_url: "https://antigravity.google",
        format: ConfigFormat::JsonMcp,
        detect_dirs: &[".gemini", ".gemini/config", ".gemini/antigravity"],
        config_rel: ".gemini/config/mcp_config.json",
        skill_rel: ".gemini/antigravity/skills/orbit/SKILL.md",
    },
    HarnessSpec {
        id: "claude_code",
        name: "Claude Code",
        description: "Anthropic's agentic CLI tool for autonomous software engineering in the terminal.",
        icon: "terminal",
        docs_url: "https://docs.anthropic.com/en/docs/agents-and-tools/claude-code/overview",
        format: ConfigFormat::JsonMcp,
        detect_dirs: &[".claude"],
        config_rel: ".claude/mcp_config.json",
        skill_rel: ".claude/skills/orbit-memory/SKILL.md",
    },
    HarnessSpec {
        id: "cursor",
        name: "Cursor",
        description: "AI-first code editor with inline code generation, agent terminal, and MCP support.",
        icon: "code",
        docs_url: "https://cursor.com",
        format: ConfigFormat::JsonMcp,
        detect_dirs: &[".cursor"],
        config_rel: ".cursor/mcp.json",
        skill_rel: ".cursor/rules/orbit-memory.md",
    },
    HarnessSpec {
        id: "codex",
        name: "Codex",
        description: "OpenAI Codex CLI and terminal code agent engine.",
        icon: "file-code",
        docs_url: "https://openai.com",
        format: ConfigFormat::JsonMcp,
        detect_dirs: &[".codex"],
        config_rel: ".codex/config.json",
        skill_rel: ".codex/instructions.md",
    },
    HarnessSpec {
        id: "goose",
        name: "Goose",
        description: "Block's open source extensible AI agent for autonomous software engineering.",
        icon: "feather",
        docs_url: "https://block.github.io/goose",
        format: ConfigFormat::GooseYaml,
        detect_dirs: &[".config/goose", ".goose"],
        config_rel: ".config/goose/config.yaml",
        skill_rel: ".goose/skills/orbit.yaml",
    },
    HarnessSpec {
        id: "opencode",
        name: "OpenCode",
        description: "Open source terminal and IDE AI coding copilot with agent loop.",
        icon: "cpu",
        docs_url: "https://github.com",
        format: ConfigFormat::JsonMcp,
        detect_dirs: &[".opencode"],
        config_rel: ".opencode/mcp.json",
        skill_rel: ".opencode/skills/orbit.md",
    },
    HarnessSpec {
        id: "zcode",
        name: "ZCode",
        description: "High-performance Zed AI coding assistant and agent environment.",
        icon: "layers",
        docs_url: "https://zed.dev",
        format: ConfigFormat::JsonMcp,
        detect_dirs: &[".zcode"],
        config_rel: ".zcode/mcp_config.json",
        skill_rel: ".zcode/instructions/orbit.md",
    },
    HarnessSpec {
        id: "agy_cli",
        name: "AGY CLI",
        description: "Antigravity developer CLI supporting multi-agent collaboration and MCP bridges.",
        icon: "command",
        docs_url: "https://antigravity.google",
        format: ConfigFormat::JsonMcp,
        detect_dirs: &[".gemini"],
        config_rel: ".gemini/config/mcp_config.json",
        skill_rel: ".gemini/skills/orbit.md",
    },
    HarnessSpec {
        id: "kimi",
        name: "Kimi",
        description: "Moonshot AI Kimi coding assistant with multi-turn reasoning and agent tools.",
        icon: "message-square",
        docs_url: "https://moonshot.cn",
        format: ConfigFormat::JsonMcp,
        detect_dirs: &[".kimi"],
        config_rel: ".kimi/mcp.json",
        skill_rel: ".kimi/rules/orbit.md",
    },
];

/// Resolves user home directory with fallback to current directory.
pub fn resolve_home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Resolves path to the `buzz-mcp` binary or falls back to "buzz-mcp".
pub fn resolve_buzz_mcp_command() -> String {
    let home = resolve_home_dir();
    let orbit_bin = home.join(".orbit").join("bin").join(if cfg!(windows) { "buzz-mcp.exe" } else { "buzz-mcp" });
    if orbit_bin.exists() {
        return orbit_bin.to_string_lossy().to_string();
    }
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let candidate = parent.join(if cfg!(windows) { "buzz-mcp.exe" } else { "buzz-mcp" });
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    "buzz-mcp".to_string()
}

/// Canonical Universal Orbit Skill markdown content.
pub fn canonical_orbit_skill_markdown() -> &'static str {
    r#"---
name: orbit-memory
description: Centralized Orbit persistent memory, bi-temporal knowledge graph, and cross-agent context synchronization.
---

# Orbit Centralized Brain Skill

You are connected to the central Orbit persistent memory and knowledge graph (`~/.orbit/brain/`).
Always use the `orbit.*` tools to maintain architectural continuity across coding sessions and peer AI agents.

## Core Tool Usage Patterns

1. **At Session Start**:
   Call `orbit.get_project_context(path=".")` to load the active architectural decisions, tech stack, and conventions before planning changes.

2. **Before Modifying Code**:
   Call `orbit.get_file_history(file_path="<target-file>")` to see previous decisions, changes, and architectural rationale affecting that file.

3. **Before Making Major Decisions**:
   Call `orbit.search_context(query="<architectural query>")` to search past decisions, dense semantic memories, and documentation across all agents.

4. **After Completing Significant Tasks**:
   Call `orbit.store_memory(content="<decision or architectural fact>", tags=["architecture", "decision"])` to record state so other agents (Cursor, Claude, Antigravity, etc.) stay synchronized.

5. **When Obsoleting Decisions**:
   Call `orbit.mark_decision(id="<decision-id>", state="superseded", note="<reason>")` to keep the bi-temporal knowledge graph accurate.
"#
}

/// Canonical Universal Orbit Skill YAML content for Goose.
pub fn canonical_orbit_skill_yaml() -> &'static str {
    r#"name: orbit-memory
description: Centralized Orbit persistent memory and knowledge graph synchronization.
instructions: |
  You are connected to the central Orbit persistent memory and knowledge graph.
  1. At session start, call orbit.get_project_context(path=".") to load active architectural decisions.
  2. Before modifying code, call orbit.get_file_history(file_path="...") to review past changes.
  3. Before making decisions, call orbit.search_context(query="...") to search memories.
  4. After completing tasks, call orbit.store_memory(...) to persist state for peer agents.
  5. When obsoleting decisions, call orbit.mark_decision(...) to update the knowledge graph.
"#
}

/// Internal detector logic parameterized by user root.
pub fn detect_harnesses_with_root(root: &Path) -> Vec<HarnessInfo> {
    SUPPORTED_HARNESSES
        .iter()
        .map(|spec| {
            let config_path = root.join(spec.config_rel);
            let skill_path = root.join(spec.skill_rel);

            let has_detect_dir = spec
                .detect_dirs
                .iter()
                .any(|d| root.join(d).exists());

            let is_connected = is_harness_connected(&config_path, spec.format);
            let is_detected = is_connected || has_detect_dir || config_path.exists();

            let status = if is_connected {
                HarnessStatus::Connected
            } else if is_detected {
                HarnessStatus::Detected
            } else {
                HarnessStatus::NotInstalled
            };

            HarnessInfo {
                id: spec.id.to_string(),
                name: spec.name.to_string(),
                status,
                config_path: config_path.to_string_lossy().to_string(),
                skill_path: skill_path.to_string_lossy().to_string(),
                description: spec.description.to_string(),
                icon: spec.icon.to_string(),
                docs_url: Some(spec.docs_url.to_string()),
            }
        })
        .collect()
}

fn is_harness_connected(config_path: &Path, format: ConfigFormat) -> bool {
    if !config_path.exists() {
        return false;
    }
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    match format {
        ConfigFormat::JsonMcp => {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(servers) = json.get("mcpServers").and_then(|s| s.as_object()) {
                    return servers.contains_key("orbit");
                }
            }
            false
        }
        ConfigFormat::GooseYaml => {
            if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                if let Some(extensions) = yaml.get("extensions").and_then(|e| e.as_mapping()) {
                    let key = serde_yaml::Value::String("orbit".to_string());
                    return extensions.contains_key(&key);
                }
            }
            false
        }
    }
}

/// Injects Orbit MCP server and universal skill into harness configuration.
pub fn connect_harness_with_root(root: &Path, harness_id: &str) -> Result<bool, String> {
    let spec = SUPPORTED_HARNESSES
        .iter()
        .find(|s| s.id == harness_id)
        .ok_or_else(|| format!("Unknown harness ID: {harness_id}"))?;

    let config_path = root.join(spec.config_rel);
    let skill_path = root.join(spec.skill_rel);

    // 1. Ensure config directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory {}: {e}", parent.display()))?;
    }

    // 2. Read and modify configuration
    let mcp_cmd = resolve_buzz_mcp_command();
    match spec.format {
        ConfigFormat::JsonMcp => {
            let mut json: serde_json::Value = if config_path.exists() {
                let content = fs::read_to_string(&config_path)
                    .map_err(|e| format!("Failed to read {}: {e}", config_path.display()))?;
                serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
            } else {
                serde_json::json!({})
            };

            if !json.is_object() {
                json = serde_json::json!({});
            }

            let obj = json.as_object_mut().expect("json is object");
            if !obj.contains_key("mcpServers") || !obj["mcpServers"].is_object() {
                obj.insert("mcpServers".to_string(), serde_json::json!({}));
            }

            let servers = obj
                .get_mut("mcpServers")
                .and_then(|s| s.as_object_mut())
                .expect("mcpServers is object");

            servers.insert(
                "orbit".to_string(),
                serde_json::json!({
                    "command": mcp_cmd,
                    "args": ["--stdio"]
                }),
            );

            let serialized = serde_json::to_string_pretty(&json)
                .map_err(|e| format!("Failed to serialize config JSON: {e}"))?;
            fs::write(&config_path, serialized)
                .map_err(|e| format!("Failed to write {}: {e}", config_path.display()))?;
        }
        ConfigFormat::GooseYaml => {
            let mut yaml: serde_yaml::Value = if config_path.exists() {
                let content = fs::read_to_string(&config_path)
                    .map_err(|e| format!("Failed to read {}: {e}", config_path.display()))?;
                serde_yaml::from_str(&content).unwrap_or_else(|_| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
            } else {
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
            };

            if !yaml.is_mapping() {
                yaml = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
            }

            let root_map = yaml.as_mapping_mut().expect("yaml is mapping");
            let ext_key = serde_yaml::Value::String("extensions".to_string());
            if !root_map.contains_key(&ext_key) || !root_map[&ext_key].is_mapping() {
                root_map.insert(ext_key.clone(), serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
            }

            let ext_map = root_map
                .get_mut(&ext_key)
                .and_then(|e| e.as_mapping_mut())
                .expect("extensions is mapping");

            let mut orbit_entry = serde_yaml::Mapping::new();
            orbit_entry.insert(
                serde_yaml::Value::String("name".to_string()),
                serde_yaml::Value::String("orbit".to_string()),
            );
            orbit_entry.insert(
                serde_yaml::Value::String("cmd".to_string()),
                serde_yaml::Value::String(mcp_cmd),
            );
            orbit_entry.insert(
                serde_yaml::Value::String("args".to_string()),
                serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("--stdio".to_string())]),
            );
            orbit_entry.insert(
                serde_yaml::Value::String("type".to_string()),
                serde_yaml::Value::String("stdio".to_string()),
            );

            ext_map.insert(
                serde_yaml::Value::String("orbit".to_string()),
                serde_yaml::Value::Mapping(orbit_entry),
            );

            let serialized = serde_yaml::to_string(&yaml)
                .map_err(|e| format!("Failed to serialize config YAML: {e}"))?;
            fs::write(&config_path, serialized)
                .map_err(|e| format!("Failed to write {}: {e}", config_path.display()))?;
        }
    }

    // 3. Inject Universal Orbit Skill definition
    if let Some(parent) = skill_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create skill directory {}: {e}", parent.display()))?;
    }

    let skill_content = if spec.format == ConfigFormat::GooseYaml {
        canonical_orbit_skill_yaml()
    } else {
        canonical_orbit_skill_markdown()
    };

    fs::write(&skill_path, skill_content)
        .map_err(|e| format!("Failed to write skill file {}: {e}", skill_path.display()))?;

    Ok(true)
}

/// Safely removes Orbit MCP registration and skill definition.
pub fn disconnect_harness_with_root(root: &Path, harness_id: &str) -> Result<bool, String> {
    let spec = SUPPORTED_HARNESSES
        .iter()
        .find(|s| s.id == harness_id)
        .ok_or_else(|| format!("Unknown harness ID: {harness_id}"))?;

    let config_path = root.join(spec.config_rel);
    let skill_path = root.join(spec.skill_rel);

    // 1. Remove from configuration
    if config_path.exists() {
        match spec.format {
            ConfigFormat::JsonMcp => {
                if let Ok(content) = fs::read_to_string(&config_path) {
                    if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(servers) = json.get_mut("mcpServers").and_then(|s| s.as_object_mut()) {
                            servers.remove("orbit");
                            if let Ok(serialized) = serde_json::to_string_pretty(&json) {
                                let _ = fs::write(&config_path, serialized);
                            }
                        }
                    }
                }
            }
            ConfigFormat::GooseYaml => {
                if let Ok(content) = fs::read_to_string(&config_path) {
                    if let Ok(mut yaml) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                        if let Some(extensions) = yaml.get_mut("extensions").and_then(|e| e.as_mapping_mut()) {
                            let key = serde_yaml::Value::String("orbit".to_string());
                            extensions.remove(&key);
                            if let Ok(serialized) = serde_yaml::to_string(&yaml) {
                                let _ = fs::write(&config_path, serialized);
                            }
                        }
                    }
                }
            }
        }
    }

    // 2. Remove injected skill file if it exists
    if skill_path.exists() {
        let _ = fs::remove_file(&skill_path);
    }

    Ok(true)
}

// ── Tauri Commands ─────────────────────────────────────────────────────────────

/// Scans workstation for installed AI coding agent harnesses and returns their status.
#[tauri::command]
pub async fn detect_agent_harnesses() -> Result<Vec<HarnessInfo>, String> {
    let home = resolve_home_dir();
    Ok(detect_harnesses_with_root(&home))
}

/// Connects an agent harness to the centralized Orbit central brain.
/// Injects `buzz-mcp` into the agent's MCP config and copies the canonical Orbit Skill.
#[tauri::command]
pub async fn connect_harness(harness_id: String) -> Result<bool, String> {
    let home = resolve_home_dir();
    connect_harness_with_root(&home, &harness_id)
}

/// Disconnects an agent harness from Orbit central brain.
#[tauri::command]
pub async fn disconnect_harness(harness_id: String) -> Result<bool, String> {
    let home = resolve_home_dir();
    disconnect_harness_with_root(&home, &harness_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_tc_f08_001_connect_harness_idempotent() {
        let tmp = tempdir().expect("tempdir");
        let root = tmp.path();

        // 1. Initial detection: Antigravity and Claude should be NotInstalled
        let initial = detect_harnesses_with_root(root);
        assert_eq!(initial.len(), 9);
        let claude = initial.iter().find(|h| h.id == "claude_code").unwrap();
        assert_eq!(claude.status, HarnessStatus::NotInstalled);

        // 2. Connect Claude Code
        let res = connect_harness_with_root(root, "claude_code");
        assert_eq!(res, Ok(true));

        // Verify config written
        let claude_config = root.join(".claude/mcp_config.json");
        assert!(claude_config.exists());
        let content = fs::read_to_string(&claude_config).expect("read claude config");
        let json: serde_json::Value = serde_json::from_str(&content).expect("parse json");
        assert!(json["mcpServers"]["orbit"].is_object());
        assert_eq!(json["mcpServers"]["orbit"]["args"][0], "--stdio");

        // Verify skill written
        let claude_skill = root.join(".claude/skills/orbit-memory/SKILL.md");
        assert!(claude_skill.exists());
        let skill_text = fs::read_to_string(&claude_skill).expect("read claude skill");
        assert!(skill_text.contains("orbit.get_project_context"));
        assert!(skill_text.contains("orbit.store_memory"));

        // Status should now be Connected
        let detected = detect_harnesses_with_root(root);
        let claude_after = detected.iter().find(|h| h.id == "claude_code").unwrap();
        assert_eq!(claude_after.status, HarnessStatus::Connected);
    }

    #[test]
    fn test_tc_f08_002_reconnect_no_duplicates_or_corruption() {
        let tmp = tempdir().expect("tempdir");
        let root = tmp.path();

        // Existing config with other developer servers
        let claude_dir = root.join(".claude");
        fs::create_dir_all(&claude_dir).unwrap();
        let initial_json = serde_json::json!({
            "mcpServers": {
                "custom-tools": {
                    "command": "node",
                    "args": ["custom.js"]
                }
            },
            "userPreference": "developer-mode"
        });
        fs::write(claude_dir.join("mcp_config.json"), serde_json::to_string_pretty(&initial_json).unwrap()).unwrap();

        // Status should be Detected
        let detected = detect_harnesses_with_root(root);
        let claude = detected.iter().find(|h| h.id == "claude_code").unwrap();
        assert_eq!(claude.status, HarnessStatus::Detected);

        // First connect
        assert_eq!(connect_harness_with_root(root, "claude_code"), Ok(true));
        // Second connect (reconnect)
        assert_eq!(connect_harness_with_root(root, "claude_code"), Ok(true));

        // Verify no duplicate keys or broken JSON
        let content = fs::read_to_string(claude_dir.join("mcp_config.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse json");

        // Existing developer configs preserved
        assert_eq!(parsed["userPreference"], "developer-mode");
        assert!(parsed["mcpServers"]["custom-tools"].is_object());
        // Orbit server present exactly once
        assert!(parsed["mcpServers"]["orbit"].is_object());
        assert_eq!(parsed["mcpServers"].as_object().unwrap().len(), 2);

        // Status is Connected
        let detected_again = detect_harnesses_with_root(root);
        let claude_conn = detected_again.iter().find(|h| h.id == "claude_code").unwrap();
        assert_eq!(claude_conn.status, HarnessStatus::Connected);

        // Test Disconnect
        assert_eq!(disconnect_harness_with_root(root, "claude_code"), Ok(true));
        let content_after = fs::read_to_string(claude_dir.join("mcp_config.json")).unwrap();
        let parsed_after: serde_json::Value = serde_json::from_str(&content_after).unwrap();
        assert!(!parsed_after["mcpServers"].as_object().unwrap().contains_key("orbit"));
        assert!(parsed_after["mcpServers"]["custom-tools"].is_object());

        // Skill removed
        assert!(!root.join(".claude/skills/orbit-memory/SKILL.md").exists());
    }

    #[test]
    fn test_goose_yaml_connect_and_disconnect() {
        let tmp = tempdir().expect("tempdir");
        let root = tmp.path();

        let goose_dir = root.join(".config").join("goose");
        fs::create_dir_all(&goose_dir).unwrap();

        // Connect Goose
        assert_eq!(connect_harness_with_root(root, "goose"), Ok(true));
        let cfg_path = goose_dir.join("config.yaml");
        assert!(cfg_path.exists());

        let content = fs::read_to_string(&cfg_path).unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
        assert!(yaml["extensions"]["orbit"].is_mapping());

        let skill_path = root.join(".goose/skills/orbit.yaml");
        assert!(skill_path.exists());

        // Reconnect
        assert_eq!(connect_harness_with_root(root, "goose"), Ok(true));

        // Disconnect
        assert_eq!(disconnect_harness_with_root(root, "goose"), Ok(true));
        let content_after = fs::read_to_string(&cfg_path).unwrap();
        let yaml_after: serde_yaml::Value = serde_yaml::from_str(&content_after).unwrap();
        assert!(yaml_after["extensions"].as_mapping().unwrap().is_empty());
        assert!(!skill_path.exists());
    }
}
