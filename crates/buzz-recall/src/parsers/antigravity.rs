#![deny(unsafe_code)]
//! Parser for Google Antigravity IDE and AGY brain session transcripts.

use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::warn;
use walkdir::WalkDir;
use buzz_plugins::{RawDocument, RecallPlugin, SessionTurn};
use crate::normalizer::normalize_and_redact_transcript;

/// Recall plugin for Google Antigravity IDE transcripts.
#[derive(Debug, Clone, Default)]
pub struct AntigravityRecallPlugin {
    custom_root: Option<PathBuf>,
}

impl AntigravityRecallPlugin {
    /// Creates a new plugin scanning default system locations.
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Creates a plugin with a custom root directory override (useful for testing).
    pub fn with_custom_root(root: impl AsRef<Path>) -> Self {
        Self {
            custom_root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// Returns candidate search directories for Antigravity transcripts.
    pub fn candidate_dirs(&self) -> Vec<PathBuf> {
        if let Some(ref root) = self.custom_root {
            return vec![root.clone()];
        }

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        vec![
            home.join(".gemini").join("antigravity-ide").join("brain"),
            home.join(".gemini").join("antigravity").join("brain"),
        ]
    }

    /// Parses a single `transcript.jsonl` file into a `RawDocument`.
    pub fn parse_file(path: &Path, workspace_path: &Path) -> Option<RawDocument> {
        let content = fs::read_to_string(path).ok()?;
        let mut turns = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                let source = val.get("source").and_then(|s| s.as_str()).unwrap_or("");
                let step_type = val.get("type").and_then(|s| s.as_str()).unwrap_or("");
                let text = val.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
                let thinking = val.get("thinking").and_then(|t| t.as_str()).map(|s| s.to_string());
                let tool_calls = val.get("tool_calls").and_then(|t| t.as_array()).cloned().unwrap_or_default();

                if source == "USER_EXPLICIT" || step_type == "USER_INPUT" {
                    if !text.is_empty() {
                        turns.push(SessionTurn::user(text));
                    }
                } else if source == "MODEL" || step_type == "PLANNER_RESPONSE" {
                    let mut turn = SessionTurn::assistant(text, thinking);
                    turn.tool_calls = tool_calls;
                    turns.push(turn);
                } else if step_type == "GENERIC" || source == "TOOL" {
                    if !text.is_empty() {
                        turns.push(SessionTurn::tool(text, Vec::new(), Vec::new()));
                    }
                }
            }
        }

        if turns.is_empty() {
            return None;
        }

        let normalized_content = normalize_and_redact_transcript(&turns);
        let mtime_ns = fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        let session_id = path
            .parent()
            .and_then(|p| {
                if p.file_name().and_then(|n| n.to_str()) == Some("logs") {
                    p.parent()
                        .and_then(|p2| p2.parent())
                        .and_then(|p3| p3.file_name())
                        .and_then(|n| n.to_str())
                } else {
                    p.file_name().and_then(|n| n.to_str())
                }
            })
            .unwrap_or("antigravity_session")
            .to_string();

        let metadata = serde_json::json!({
            "agent_name": "antigravity",
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
            normalized_content,
            mtime_ns,
            metadata,
            turns,
        ))
    }
}

#[async_trait]
impl RecallPlugin for AntigravityRecallPlugin {
    fn name(&self) -> &'static str {
        "antigravity"
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
                if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some("transcript.jsonl") {
                    if let Some(doc) = Self::parse_file(path, workspace_path) {
                        documents.push(doc);
                    } else {
                        warn!("Antigravity transcript at {:?} had no usable turns", path);
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
    async fn test_antigravity_parser() {
        let temp = tempdir().unwrap();
        let session_dir = temp.path().join("brain").join("session-123");
        fs::create_dir_all(&session_dir).unwrap();
        let transcript_file = session_dir.join("transcript.jsonl");

        let lines = [
            serde_json::json!({
                "step_index": 0,
                "source": "USER_EXPLICIT",
                "type": "USER_INPUT",
                "content": "Please inspect the orbit codebase."
            }),
            serde_json::json!({
                "step_index": 1,
                "source": "MODEL",
                "type": "PLANNER_RESPONSE",
                "thinking": "Analyzing project structure and crates.",
                "content": "I have inspected the project structure.",
                "tool_calls": [{"name": "run_command", "args": {"CommandLine": "cargo test"}}]
            }),
        ];

        let content = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
        fs::write(&transcript_file, content).unwrap();

        let plugin = AntigravityRecallPlugin::with_custom_root(temp.path().join("brain"));
        assert!(plugin.detect());

        let docs = plugin.ingest_sessions(Path::new("/workspace/orbit")).await;
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.turns.len(), 2);
        assert!(doc.content.contains("### User"));
        assert!(doc.content.contains("Please inspect the orbit codebase."));
        assert!(doc.content.contains("Analyzing project structure"));
        assert_eq!(doc.metadata["agent_name"], "antigravity");
        assert_eq!(doc.metadata["session_id"], "session-123");
    }
}
