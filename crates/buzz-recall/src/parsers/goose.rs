#![deny(unsafe_code)]
//! Parser for Block Goose agent session files (JSON / YAML).

use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use buzz_plugins::{RawDocument, RecallPlugin, SessionTurn};
use crate::normalizer::normalize_and_redact_transcript;

/// Recall plugin for Goose sessions.
#[derive(Debug, Clone, Default)]
pub struct GooseRecallPlugin {
    custom_root: Option<PathBuf>,
}

impl GooseRecallPlugin {
    /// Creates a new Goose recall plugin.
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Creates a plugin with a custom root directory override.
    pub fn with_custom_root(root: impl AsRef<Path>) -> Self {
        Self {
            custom_root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// Candidate directory paths for Goose sessions.
    pub fn candidate_dirs(&self) -> Vec<PathBuf> {
        if let Some(ref root) = self.custom_root {
            return vec![root.clone()];
        }

        let mut dirs_list = Vec::new();
        if let Some(config) = dirs::config_dir() {
            dirs_list.push(config.join("goose").join("sessions"));
        }
        if let Some(home) = dirs::home_dir() {
            dirs_list.push(home.join(".config").join("goose").join("sessions"));
            dirs_list.push(home.join(".local").join("share").join("goose").join("sessions"));
        }

        dirs_list
    }

    /// Parses a single Goose session file (JSON or YAML).
    pub fn parse_file(path: &Path, workspace_path: &Path) -> Option<RawDocument> {
        let content = fs::read_to_string(path).ok()?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let json_val = if ext == "yaml" || ext == "yml" {
            serde_yaml::from_str::<serde_json::Value>(&content).ok()?
        } else {
            serde_json::from_str::<serde_json::Value>(&content).ok()?
        };

        let mut turns = Vec::new();

        let messages = if let Some(arr) = json_val.as_array() {
            arr.clone()
        } else if let Some(arr) = json_val.get("messages").and_then(|m| m.as_array()) {
            arr.clone()
        } else {
            Vec::new()
        };

        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let mut text_parts = Vec::new();
            let mut tool_calls = Vec::new();

            if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                text_parts.push(content_str.to_string());
            } else if let Some(blocks) = msg.get("content").and_then(|c| c.as_array()) {
                for b in blocks {
                    if let Some(t) = b.get("text").and_then(|txt| txt.as_str()) {
                        text_parts.push(t.to_string());
                    } else if b.get("type").and_then(|t| t.as_str()) == Some("toolRequest") {
                        tool_calls.push(b.clone());
                    }
                }
            }

            if let Some(invocations) = msg.get("tool_invocations").and_then(|ti| ti.as_array()) {
                tool_calls.extend(invocations.iter().cloned());
            }

            let combined_text = text_parts.join("\n");
            match role {
                "user" => {
                    if !combined_text.is_empty() {
                        turns.push(SessionTurn::user(combined_text));
                    }
                }
                "assistant" => {
                    let mut turn = SessionTurn::assistant(combined_text, None);
                    turn.tool_calls = tool_calls;
                    turns.push(turn);
                }
                _ => {}
            }
        }

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

        let session_id = json_val
            .get("id")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("goose_session")
                    .to_string()
            });

        let metadata = serde_json::json!({
            "agent_name": "goose",
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
impl RecallPlugin for GooseRecallPlugin {
    fn name(&self) -> &'static str {
        "goose"
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
                    if ext == "json" || ext == "yaml" || ext == "yml" {
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
    async fn test_goose_parser() {
        let temp = tempdir().unwrap();
        let sessions_dir = temp.path().join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_file = sessions_dir.join("goose_001.json");

        let json = serde_json::json!({
            "id": "goose_001",
            "description": "Refactor Goose plugins",
            "messages": [
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "Set up goose test harness"}]
                },
                {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "Setting up mock session files."}],
                    "tool_invocations": [{"name": "developer__shell", "command": "cargo check"}]
                }
            ]
        });

        fs::write(&session_file, json.to_string()).unwrap();

        let plugin = GooseRecallPlugin::with_custom_root(&sessions_dir);
        assert!(plugin.detect());

        let docs = plugin.ingest_sessions(Path::new("/workspace/orbit")).await;
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.metadata["agent_name"], "goose");
        assert_eq!(doc.metadata["session_id"], "goose_001");
        assert!(doc.content.contains("Set up goose test harness"));
        assert!(doc.content.contains("Setting up mock session files."));
    }
}
