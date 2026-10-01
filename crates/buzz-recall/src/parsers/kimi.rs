#![deny(unsafe_code)]
//! Parser for Moonshot Kimi chat and session logs.

use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use buzz_plugins::{RawDocument, RecallPlugin, SessionTurn, TurnRole};
use crate::normalizer::normalize_and_redact_transcript;

/// Recall plugin for Kimi chats and session logs.
#[derive(Debug, Clone, Default)]
pub struct KimiRecallPlugin {
    custom_root: Option<PathBuf>,
}

impl KimiRecallPlugin {
    /// Creates a new Kimi recall plugin.
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Creates a plugin with a custom root directory override.
    pub fn with_custom_root(root: impl AsRef<Path>) -> Self {
        Self {
            custom_root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// Candidate directory paths for Kimi chats.
    pub fn candidate_dirs(&self) -> Vec<PathBuf> {
        if let Some(ref root) = self.custom_root {
            return vec![root.clone()];
        }

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        vec![
            home.join(".kimi").join("chats"),
            home.join(".kimi").join("sessions"),
        ]
    }

    /// Parses a single Kimi chat JSON file.
    pub fn parse_file(path: &Path, workspace_path: &Path) -> Option<RawDocument> {
        let content = fs::read_to_string(path).ok()?;
        let json_val = serde_json::from_str::<serde_json::Value>(&content).ok()?;
        let mut turns = Vec::new();

        let messages = if let Some(arr) = json_val.as_array() {
            arr.clone()
        } else if let Some(arr) = json_val.get("messages").or_else(|| json_val.get("turns")).and_then(|m| m.as_array()) {
            arr.clone()
        } else {
            Vec::new()
        };

        for msg in messages {
            let role = msg.get("role").or_else(|| msg.get("sender")).and_then(|r| r.as_str()).unwrap_or("");
            let text = msg.get("content").or_else(|| msg.get("text")).and_then(|c| c.as_str()).unwrap_or("");

            match role {
                "user" | "human" => {
                    if !text.is_empty() {
                        turns.push(SessionTurn::user(text));
                    }
                }
                "assistant" | "kimi" | "bot" => {
                    if !text.is_empty() {
                        turns.push(SessionTurn::assistant(text, None));
                    }
                }
                "system" => {
                    if !text.is_empty() {
                        turns.push(SessionTurn {
                            role: TurnRole::System,
                            content: text.to_string(),
                            thinking: None,
                            tool_calls: Vec::new(),
                            tool_results: Vec::new(),
                            timestamp: None,
                        });
                    }
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
            .get("chat_id")
            .or_else(|| json_val.get("session_id"))
            .or_else(|| json_val.get("id"))
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("kimi_chat")
                    .to_string()
            });

        let metadata = serde_json::json!({
            "agent_name": "kimi",
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
impl RecallPlugin for KimiRecallPlugin {
    fn name(&self) -> &'static str {
        "kimi"
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
    async fn test_kimi_parser() {
        let temp = tempdir().unwrap();
        let chats_dir = temp.path().join("chats");
        fs::create_dir_all(&chats_dir).unwrap();
        let chat_file = chats_dir.join("kimi_001.json");

        let json = serde_json::json!({
            "chat_id": "kimi_001",
            "messages": [
                {"role": "user", "content": "Explain 2-hop graph neighborhood expansion."},
                {"role": "assistant", "content": "2-hop expansion queries 1st degree neighbors and then traverses edges from those neighbors."}
            ]
        });

        fs::write(&chat_file, json.to_string()).unwrap();

        let plugin = KimiRecallPlugin::with_custom_root(&chats_dir);
        assert!(plugin.detect());

        let docs = plugin.ingest_sessions(Path::new("/workspace/orbit")).await;
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.metadata["agent_name"], "kimi");
        assert_eq!(doc.metadata["session_id"], "kimi_001");
        assert!(doc.content.contains("Explain 2-hop graph neighborhood expansion."));
        assert!(doc.content.contains("2-hop expansion queries 1st degree neighbors"));
    }
}
