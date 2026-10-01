#![deny(unsafe_code)]
//! Parser for ZCode conversation and session records.

use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use buzz_plugins::{RawDocument, RecallPlugin, SessionTurn};
use crate::normalizer::normalize_and_redact_transcript;

/// Recall plugin for ZCode conversations.
#[derive(Debug, Clone, Default)]
pub struct ZCodeRecallPlugin {
    custom_root: Option<PathBuf>,
}

impl ZCodeRecallPlugin {
    /// Creates a new ZCode recall plugin.
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Creates a plugin with a custom root directory override.
    pub fn with_custom_root(root: impl AsRef<Path>) -> Self {
        Self {
            custom_root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// Candidate directory paths for ZCode conversations.
    pub fn candidate_dirs(&self) -> Vec<PathBuf> {
        if let Some(ref root) = self.custom_root {
            return vec![root.clone()];
        }

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        vec![
            home.join(".zcode").join("conversations"),
            home.join(".config").join("zcode").join("conversations"),
        ]
    }

    /// Parses a single ZCode conversation JSON file.
    pub fn parse_file(path: &Path, workspace_path: &Path) -> Option<RawDocument> {
        let content = fs::read_to_string(path).ok()?;
        let json_val = serde_json::from_str::<serde_json::Value>(&content).ok()?;
        let mut turns = Vec::new();

        // 1. Format: {"history": [{"speaker": "human"|"agent", "utterance": "..."}]}
        if let Some(history) = json_val.get("history").and_then(|h| h.as_array()) {
            for item in history {
                let speaker = item.get("speaker").or_else(|| item.get("role")).and_then(|s| s.as_str()).unwrap_or("");
                let text = item.get("utterance").or_else(|| item.get("text")).or_else(|| item.get("content")).and_then(|t| t.as_str()).unwrap_or("");
                let actions = item.get("actions").and_then(|a| a.as_array()).cloned().unwrap_or_default();

                if (speaker == "human" || speaker == "user") && !text.is_empty() {
                    turns.push(SessionTurn::user(text));
                } else if (speaker == "agent" || speaker == "assistant") && (!text.is_empty() || !actions.is_empty()) {
                    let mut turn = SessionTurn::assistant(text, None);
                    turn.tool_calls = actions;
                    turns.push(turn);
                }
            }
        }
        // 2. Format: {"messages": [{"sender": "user"|"agent", "text": "..."}]}
        else if let Some(msgs) = json_val.get("messages").and_then(|m| m.as_array()) {
            for msg in msgs {
                let sender = msg.get("sender").or_else(|| msg.get("role")).and_then(|s| s.as_str()).unwrap_or("");
                let text = msg.get("text").or_else(|| msg.get("content")).and_then(|t| t.as_str()).unwrap_or("");
                if (sender == "user" || sender == "human") && !text.is_empty() {
                    turns.push(SessionTurn::user(text));
                } else if (sender == "agent" || sender == "assistant") && !text.is_empty() {
                    turns.push(SessionTurn::assistant(text, None));
                }
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
            .get("conversation_id")
            .or_else(|| json_val.get("id"))
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("zcode_conv")
                    .to_string()
            });

        let metadata = serde_json::json!({
            "agent_name": "zcode",
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
impl RecallPlugin for ZCodeRecallPlugin {
    fn name(&self) -> &'static str {
        "zcode"
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
                if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Some(doc) = Self::parse_file(path, workspace_path) {
                        documents.push(doc);
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
    async fn test_zcode_parser() {
        let temp = tempdir().unwrap();
        let conv_dir = temp.path().join("conversations");
        fs::create_dir_all(&conv_dir).unwrap();
        let conv_file = conv_dir.join("zc_456.json");

        let json = serde_json::json!({
            "conversation_id": "zc_456",
            "history": [
                {"speaker": "human", "utterance": "Configure PostgreSQL replication slot"},
                {"speaker": "agent", "utterance": "Creating logical replication slot for CDC stream.", "actions": [{"tool": "db_exec"}]}
            ]
        });

        fs::write(&conv_file, json.to_string()).unwrap();

        let plugin = ZCodeRecallPlugin::with_custom_root(&conv_dir);
        assert!(plugin.detect());

        let docs = plugin.ingest_sessions(Path::new("/workspace/orbit")).await;
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.metadata["agent_name"], "zcode");
        assert_eq!(doc.metadata["session_id"], "zc_456");
        assert!(doc.content.contains("Configure PostgreSQL replication slot"));
        assert!(doc.content.contains("Creating logical replication slot"));
    }
}
