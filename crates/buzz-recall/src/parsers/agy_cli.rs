#![deny(unsafe_code)]
//! Parser for AGY CLI execution transcripts and command histories.

use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use buzz_plugins::{RawDocument, RecallPlugin, SessionTurn};
use crate::normalizer::normalize_and_redact_transcript;

/// Recall plugin for AGY CLI execution transcripts.
#[derive(Debug, Clone, Default)]
pub struct AgyCliRecallPlugin {
    custom_root: Option<PathBuf>,
}

impl AgyCliRecallPlugin {
    /// Creates a new AGY CLI recall plugin.
    pub fn new() -> Self {
        Self { custom_root: None }
    }

    /// Creates a plugin with a custom root directory override.
    pub fn with_custom_root(root: impl AsRef<Path>) -> Self {
        Self {
            custom_root: Some(root.as_ref().to_path_buf()),
        }
    }

    /// Candidate directory paths for AGY CLI transcripts.
    pub fn candidate_dirs(&self) -> Vec<PathBuf> {
        if let Some(ref root) = self.custom_root {
            return vec![root.clone()];
        }

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        vec![
            home.join(".gemini").join("transcripts"),
            home.join(".agy").join("transcripts"),
        ]
    }

    /// Parses an AGY CLI transcript file (JSON or JSONL).
    pub fn parse_file(path: &Path, workspace_path: &Path) -> Option<RawDocument> {
        let content = fs::read_to_string(path).ok()?;
        let mut turns = Vec::new();

        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content) {
            let items = if let Some(arr) = json_val.as_array() {
                arr.clone()
            } else if let Some(arr) = json_val.get("history").or_else(|| json_val.get("entries")).and_then(|e| e.as_array()) {
                arr.clone()
            } else {
                vec![json_val]
            };

            for item in items {
                let cmd = item.get("command").and_then(|c| c.as_str()).unwrap_or("");
                let prompt = item.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
                let output = item.get("output").and_then(|o| o.as_str()).unwrap_or("");
                let response = item.get("response").and_then(|r| r.as_str()).unwrap_or("");

                let user_text = if !prompt.is_empty() {
                    prompt.to_string()
                } else if !cmd.is_empty() {
                    format!("$ {cmd}")
                } else {
                    String::new()
                };

                if !user_text.is_empty() {
                    turns.push(SessionTurn::user(user_text));
                }

                if !response.is_empty() {
                    turns.push(SessionTurn::assistant(response, None));
                } else if !output.is_empty() {
                    turns.push(SessionTurn::tool(output, Vec::new(), Vec::new()));
                }
            }
        } else {
            // JSONL fallback
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                    let cmd = val.get("command").and_then(|c| c.as_str()).unwrap_or("");
                    let prompt = val.get("prompt").and_then(|p| p.as_str()).unwrap_or("");
                    let response = val.get("response").or_else(|| val.get("output")).and_then(|r| r.as_str()).unwrap_or("");

                    let user_text = if !prompt.is_empty() { prompt } else { cmd };
                    if !user_text.is_empty() {
                        turns.push(SessionTurn::user(user_text));
                    }
                    if !response.is_empty() {
                        turns.push(SessionTurn::assistant(response, None));
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
            .unwrap_or("agy_cli_session")
            .to_string();

        let metadata = serde_json::json!({
            "agent_name": "agy_cli",
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
impl RecallPlugin for AgyCliRecallPlugin {
    fn name(&self) -> &'static str {
        "agy_cli"
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
    async fn test_agy_cli_parser() {
        let temp = tempdir().unwrap();
        let transcript_dir = temp.path().join("transcripts");
        fs::create_dir_all(&transcript_dir).unwrap();
        let transcript_file = transcript_dir.join("cli_session.json");

        let json = serde_json::json!([
            {
                "command": "agy run-agent test",
                "prompt": "Run benchmark suite across all 9 IDE parsers",
                "response": "Benchmark suite started with 9 fixtures."
            }
        ]);

        fs::write(&transcript_file, json.to_string()).unwrap();

        let plugin = AgyCliRecallPlugin::with_custom_root(&transcript_dir);
        assert!(plugin.detect());

        let docs = plugin.ingest_sessions(Path::new("/workspace/orbit")).await;
        assert_eq!(docs.len(), 1);
        let doc = &docs[0];
        assert_eq!(doc.metadata["agent_name"], "agy_cli");
        assert_eq!(doc.metadata["session_id"], "cli_session");
        assert!(doc.content.contains("Run benchmark suite across all 9 IDE parsers"));
        assert!(doc.content.contains("Benchmark suite started with 9 fixtures."));
    }
}
