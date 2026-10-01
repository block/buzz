#![deny(unsafe_code)]
//! Parser for Cursor AI IDE workspace storage and composer session transcripts.

use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use buzz_plugins::{RawDocument, RecallPlugin, SessionTurn};
use crate::normalizer::normalize_and_redact_transcript;

/// Recall plugin for Cursor IDE composer and chat sessions.
#[derive(Debug, Clone, Default)]
pub struct CursorRecallPlugin {
    custom_root: Option<PathBuf>,
}

impl CursorRecallPlugin {
    /// Creates a new Cursor recall plugin.
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Creates a plugin with a custom root directory override.
    pub fn with_custom_root(root: impl AsRef<Path>) -> Self {
        Self {
            custom_root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// Candidate directory paths for Cursor workspaceStorage.
    pub fn candidate_dirs(&self) -> Vec<PathBuf> {
        if let Some(ref root) = self.custom_root {
            return vec![root.clone()];
        }

        let mut dirs_list = Vec::new();
        if let Some(home) = dirs::home_dir() {
            dirs_list.push(home.join(".cursor").join("User").join("workspaceStorage"));
        }
        if let Some(appdata) = dirs::config_dir() {
            dirs_list.push(appdata.join("Cursor").join("User").join("workspaceStorage"));
        }

        dirs_list
    }

    /// Parses a single Cursor session or workspaceStorage JSON file.
    pub fn parse_file(path: &Path, workspace_path: &Path) -> Option<RawDocument> {
        let content = fs::read_to_string(path).ok()?;
        let json_val = serde_json::from_str::<serde_json::Value>(&content).ok()?;
        let mut turns = Vec::new();

        // 1. Format: {"conversation": [{"text": "...", "type": 1}, {"text": "...", "type": 2}]}
        if let Some(conv) = json_val.get("conversation").and_then(|c| c.as_array()) {
            for item in conv {
                let text = item.get("text").and_then(|t| t.as_str()).unwrap_or("");
                let turn_type = item.get("type").and_then(|t| t.as_i64()).unwrap_or(0);
                if turn_type == 1 && !text.is_empty() {
                    turns.push(SessionTurn::user(text));
                } else if (turn_type == 2 || turn_type == 3) && !text.is_empty() {
                    turns.push(SessionTurn::assistant(text, None));
                }
            }
        }
        // 2. Format: {"tabs": [{"bubbles": [{"type": "user"|"ai", "text": "..."}]}]}
        else if let Some(tabs) = json_val.get("tabs").and_then(|t| t.as_array()) {
            for tab in tabs {
                if let Some(bubbles) = tab.get("bubbles").and_then(|b| b.as_array()) {
                    for bubble in bubbles {
                        let b_type = bubble.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        let text = bubble.get("text").or_else(|| bubble.get("rawText")).and_then(|t| t.as_str()).unwrap_or("");
                        if (b_type == "user" || b_type == "human") && !text.is_empty() {
                            turns.push(SessionTurn::user(text));
                        } else if (b_type == "ai" || b_type == "bot" || b_type == "assistant") && !text.is_empty() {
                            turns.push(SessionTurn::assistant(text, None));
                        }
                    }
                }
            }
        }
        // 3. Format: {"messages": [{"sender": "user"|"bot", "text": "..."}]}
        else if let Some(msgs) = json_val.get("messages").and_then(|m| m.as_array()) {
            for msg in msgs {
                let sender = msg.get("sender").or_else(|| msg.get("role")).and_then(|s| s.as_str()).unwrap_or("");
                let text = msg.get("text").or_else(|| msg.get("content")).and_then(|t| t.as_str()).unwrap_or("");
                if (sender == "user" || sender == "human") && !text.is_empty() {
                    turns.push(SessionTurn::user(text));
                } else if (sender == "bot" || sender == "ai" || sender == "assistant") && !text.is_empty() {
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
            .get("composerId")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("cursor_session")
                    .to_string()
            });

        let metadata = serde_json::json!({
            "agent_name": "cursor",
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
impl RecallPlugin for CursorRecallPlugin {
    fn name(&self) -> &'static str {
        "cursor"
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
                .max_depth(5)
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
    async fn test_cursor_parser() {
        let temp = tempdir().unwrap();
        let ws_dir = temp.path().join("workspaceStorage").join("ws-abc-123");
        fs::create_dir_all(&ws_dir).unwrap();
        let composer_file = ws_dir.join("composer.json");

        let json = serde_json::json!({
            "composerId": "comp-789",
            "conversation": [
                {"type": 1, "text": "Fix memory leak in websocket listener"},
                {"type": 2, "text": "Identified unbounded channel receiver. Applying backpressure."}
            ]
        });

        fs::write(&composer_file, json.to_string()).unwrap();

        let plugin = CursorRecallPlugin::with_custom_root(temp.path().join("workspaceStorage"));
        assert!(plugin.detect());

        let docs = plugin.ingest_sessions(Path::new("/workspace/orbit")).await;
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.metadata["agent_name"], "cursor");
        assert_eq!(doc.metadata["session_id"], "comp-789");
        assert!(doc.content.contains("Fix memory leak in websocket listener"));
        assert!(doc.content.contains("Identified unbounded channel receiver"));
    }
}
