#![deny(unsafe_code)]
//! Parser for Claude Code project and session transcripts.

use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use buzz_plugins::{RawDocument, RecallPlugin, SessionTurn};
use crate::normalizer::normalize_and_redact_transcript;

/// Recall plugin for Claude Code CLI and IDE transcripts.
#[derive(Debug, Clone, Default)]
pub struct ClaudeCodeRecallPlugin {
    custom_root: Option<PathBuf>,
}

impl ClaudeCodeRecallPlugin {
    /// Creates a new Claude Code plugin.
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Creates a plugin with a custom root directory override.
    pub fn with_custom_root(root: impl AsRef<Path>) -> Self {
        Self {
            custom_root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// Candidate directory paths for Claude Code sessions.
    pub fn candidate_dirs(&self) -> Vec<PathBuf> {
        if let Some(ref root) = self.custom_root {
            return vec![root.clone()];
        }

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        vec![
            home.join(".claude").join("projects"),
            home.join(".claude").join("sessions"),
        ]
    }

    /// Parses a single Claude Code JSON or JSONL session file.
    pub fn parse_file(path: &Path, workspace_path: &Path) -> Option<RawDocument> {
        let content = fs::read_to_string(path).ok()?;
        let mut turns = Vec::new();

        // 1. Try JSON document parsing
        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) {
            let messages = if let Some(arr) = json_val.as_array() {
                arr.clone()
            } else if let Some(arr) = json_val.get("messages").and_then(|m| m.as_array()) {
                arr.clone()
            } else if let Some(arr) = json_val.get("history").and_then(|h| h.as_array()) {
                arr.clone()
            } else {
                Vec::new()
            };

            for msg in messages {
                let role_str = msg.get("role").or_else(|| msg.get("type")).and_then(|r| r.as_str()).unwrap_or("");
                let mut text_parts = Vec::new();
                let mut tool_calls = Vec::new();
                let mut tool_results = Vec::new();

                if let Some(c_str) = msg.get("content").and_then(|c| c.as_str()) {
                    text_parts.push(c_str.to_string());
                } else if let Some(blocks) = msg.get("content").and_then(|c| c.as_array()) {
                    for b in blocks {
                        let b_type = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if b_type == "text" {
                            if let Some(t) = b.get("text").and_then(|txt| txt.as_str()) {
                                text_parts.push(t.to_string());
                            }
                        } else if b_type == "tool_use" {
                            tool_calls.push(b.clone());
                        } else if b_type == "tool_result" {
                            tool_results.push(b.clone());
                        }
                    }
                }

                let combined_text = text_parts.join("\n");
                match role_str {
                    "user" | "human" => {
                        if !combined_text.is_empty() {
                            turns.push(SessionTurn::user(combined_text));
                        }
                    }
                    "assistant" => {
                        let mut turn = SessionTurn::assistant(combined_text, None);
                        turn.tool_calls = tool_calls;
                        turns.push(turn);
                    }
                    "tool" | "tool_result" => {
                        turns.push(SessionTurn::tool(combined_text, tool_calls, tool_results));
                    }
                    _ => {}
                }
            }
        } else {
            // 2. Fallback to line-by-line JSONL
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    let role = val.get("role").or_else(|| val.get("type")).and_then(|r| r.as_str()).unwrap_or("");
                    let text = val.get("text").or_else(|| val.get("content")).and_then(|t| t.as_str()).unwrap_or("");
                    if role == "user" && !text.is_empty() {
                        turns.push(SessionTurn::user(text));
                    } else if role == "assistant" && !text.is_empty() {
                        turns.push(SessionTurn::assistant(text, None));
                    }
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

        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("claude_session")
            .to_string();

        let metadata = serde_json::json!({
            "agent_name": "claude_code",
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
impl RecallPlugin for ClaudeCodeRecallPlugin {
    fn name(&self) -> &'static str {
        "claude_code"
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
                .max_depth(6)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if path.is_file() {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext == "json" || ext == "jsonl" {
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
    async fn test_claude_code_parser() {
        let temp = tempdir().unwrap();
        let proj_dir = temp.path().join("projects").join("my-project");
        fs::create_dir_all(&proj_dir).unwrap();
        let session_file = proj_dir.join("session_101.json");

        let json = serde_json::json!({
            "sessionId": "session_101",
            "messages": [
                {
                    "role": "user",
                    "content": [{"type": "text", "text": "Refactor error handling"}]
                },
                {
                    "role": "assistant",
                    "content": [
                        {"type": "text", "text": "I'll update thiserror variants."},
                        {"type": "tool_use", "name": "edit_file", "input": {"path": "src/error.rs"}}
                    ]
                }
            ]
        });

        fs::write(&session_file, json.to_string()).unwrap();

        let plugin = ClaudeCodeRecallPlugin::with_custom_root(temp.path().join("projects"));
        assert!(plugin.detect());

        let docs = plugin.ingest_sessions(Path::new("/workspace/orbit")).await;
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.metadata["agent_name"], "claude_code");
        assert_eq!(doc.turns.len(), 2);
        assert!(doc.content.contains("Refactor error handling"));
        assert!(doc.content.contains("I'll update thiserror variants."));
    }
}
