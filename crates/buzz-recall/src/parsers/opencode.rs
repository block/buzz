#![deny(unsafe_code)]
//! Parser for OpenCode session logs (Markdown and JSON formats).

use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use buzz_plugins::{RawDocument, RecallPlugin, SessionTurn, TurnRole};
use crate::normalizer::normalize_and_redact_transcript;

/// Recall plugin for OpenCode sessions.
#[derive(Debug, Clone, Default)]
pub struct OpenCodeRecallPlugin {
    custom_root: Option<PathBuf>,
}

impl OpenCodeRecallPlugin {
    /// Creates a new OpenCode recall plugin.
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Creates a plugin with a custom root directory override.
    pub fn with_custom_root(root: impl AsRef<Path>) -> Self {
        Self {
            custom_root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// Candidate directory paths for OpenCode sessions.
    pub fn candidate_dirs(&self) -> Vec<PathBuf> {
        if let Some(ref root) = self.custom_root {
            return vec![root.clone()];
        }

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        vec![
            home.join(".opencode").join("sessions"),
            home.join(".config").join("opencode").join("sessions"),
        ]
    }

    /// Parses a markdown-formatted OpenCode session.
    fn parse_markdown(content: &str) -> Vec<SessionTurn> {
        let mut turns = Vec::new();
        let mut current_role: Option<TurnRole> = None;
        let mut current_lines = Vec::new();

        for line in content.lines() {
            let lower = line.trim().to_lowercase();
            if lower.starts_with("# user") || lower.starts_with("## user") {
                if let Some(role) = current_role {
                    let text = current_lines.join("\n").trim().to_string();
                    if !text.is_empty() {
                        turns.push(SessionTurn {
                            role,
                            content: text,
                            thinking: None,
                            tool_calls: Vec::new(),
                            tool_results: Vec::new(),
                            timestamp: None,
                        });
                    }
                    current_lines.clear();
                }
                current_role = Some(TurnRole::User);
            } else if lower.starts_with("# assistant") || lower.starts_with("## assistant") || lower.starts_with("# agent") {
                if let Some(role) = current_role {
                    let text = current_lines.join("\n").trim().to_string();
                    if !text.is_empty() {
                        turns.push(SessionTurn {
                            role,
                            content: text,
                            thinking: None,
                            tool_calls: Vec::new(),
                            tool_results: Vec::new(),
                            timestamp: None,
                        });
                    }
                    current_lines.clear();
                }
                current_role = Some(TurnRole::Assistant);
            } else {
                current_lines.push(line);
            }
        }

        if let Some(role) = current_role {
            let text = current_lines.join("\n").trim().to_string();
            if !text.is_empty() {
                turns.push(SessionTurn {
                    role,
                    content: text,
                    thinking: None,
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                    timestamp: None,
                });
            }
        }

        turns
    }

    /// Parses a single OpenCode session file.
    pub fn parse_file(path: &Path, workspace_path: &Path) -> Option<RawDocument> {
        let content = fs::read_to_string(path).ok()?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let turns = if ext == "md" || ext == "markdown" {
            Self::parse_markdown(&content)
        } else if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) {
            let mut parsed_turns = Vec::new();

            let turns_array = json_val
                .get("turns")
                .or_else(|| json_val.get("messages"))
                .and_then(|t| t.as_array());

            if let Some(arr) = turns_array {
                for item in arr {
                    let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    let prompt = item.get("prompt").or_else(|| item.get("content")).and_then(|p| p.as_str()).unwrap_or("");
                    let response = item.get("response").and_then(|r| r.as_str()).unwrap_or("");

                    if role == "user" || !prompt.is_empty() {
                        if !prompt.is_empty() {
                            parsed_turns.push(SessionTurn::user(prompt));
                        }
                    }
                    if role == "assistant" || !response.is_empty() {
                        if !response.is_empty() {
                            parsed_turns.push(SessionTurn::assistant(response, None));
                        }
                    }
                }
            }
            parsed_turns
        } else {
            Self::parse_markdown(&content)
        };

        if turns.is_empty() {
            return None;
        }

        let normalized = normalize_and_redact_transcript(&turns);
        let mtime_ns = fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("opencode_session")
            .to_string();

        let metadata = serde_json::json!({
            "agent_name": "opencode",
            "session_id": session_id,
            "turn_count": turns.len(),
            "policy": {
                "local_only": true,
                "cloud_sync": false
            }
        });

        Some(RawDocument::new(
            path.to_string_lossy().to_string(),
            workspace_path.to_string_lossy().to_string(),
            normalized,
            mtime_ns,
            metadata,
            turns,
        ))
    }
}

#[async_trait]
impl RecallPlugin for OpenCodeRecallPlugin {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn detect(&self) -> bool {
        self.candidate_dirs().iter().any(|p| p.exists())
    }

    fn default_location(&self) -> Option<PathBuf> {
        self.candidate_dirs().into_iter().next()
    }

    async fn ingest_sessions(&self, workspace_path: &Path) -> Vec<RawDocument> {
        let mut documents = Vec::new();

        for candidate_dir in self.candidate_dirs() {
            if !candidate_dir.exists() {
                continue;
            }

            for entry in WalkDir::new(&candidate_dir)
                .max_depth(4)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext == "json" || ext == "md" || ext == "markdown" {
                        if let Some(doc) = Self::parse_file(path, workspace_path) {
                            documents.push(doc);
                        }
                    }
                }
            }
        }

        documents
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_opencode_markdown_parser() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_file = sessions_dir.join("session_1.md");

        let md = "# User\nBuild an AST chunker for Rust code\n\n# Assistant\nImplemented using tree-sitter-rust queries.";
        fs::write(&session_file, md).unwrap();

        let plugin = OpenCodeRecallPlugin::with_custom_root(&sessions_dir);
        assert!(plugin.detect());

        let docs = plugin.ingest_sessions(Path::new("/workspace/orbit")).await;
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.metadata["agent_name"], "opencode");
        assert_eq!(doc.turns.len(), 2);
        assert!(doc.content.contains("Build an AST chunker for Rust code"));
        assert!(doc.content.contains("Implemented using tree-sitter-rust queries."));
    }
}
