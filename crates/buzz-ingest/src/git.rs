#![deny(unsafe_code)]
//! Git repository introspection and commit metadata extraction for Orbit.
//!
//! Enriches ingested documents with commit SHA, branch name, author, and message.

use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};

/// Extracted Git metadata for an ingested file or workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitMetadata {
    /// Active branch name (e.g. `main`, `feature/memory`).
    pub branch: Option<String>,
    /// Full or abbreviated commit SHA of current HEAD.
    pub commit_sha: Option<String>,
    /// Commit author name and email.
    pub author: Option<String>,
    /// Commit summary / message.
    pub commit_message: Option<String>,
    /// True if working tree has uncommitted modifications.
    pub is_dirty: bool,
}

impl Default for GitMetadata {
    fn default() -> Self {
        Self {
            branch: None,
            commit_sha: None,
            author: None,
            commit_message: None,
            is_dirty: false,
        }
    }
}

/// Git metadata extractor.
pub struct GitMetadataExtractor;

impl GitMetadataExtractor {
    /// Locates the enclosing `.git` root by ascending from `start`.
    pub fn find_git_root(start: &Path) -> Option<PathBuf> {
        let mut cur = if start.is_absolute() {
            start.to_path_buf()
        } else if let Ok(abs) = std::env::current_dir() {
            abs.join(start)
        } else {
            start.to_path_buf()
        };

        loop {
            if cur.join(".git").exists() {
                return Some(cur);
            }
            if !cur.pop() {
                break;
            }
        }
        None
    }

    /// Inspects the workspace root to locate `.git` and extract context.
    pub fn extract(workspace_path: impl AsRef<Path>) -> GitMetadata {
        let start = workspace_path.as_ref();
        let Some(root) = Self::find_git_root(start) else {
            return GitMetadata::default();
        };
        let git_dir = root.join(".git");

        // ponytail: execute native git command via std::process::Command, fallback to .git file inspection
        let branch = Self::run_git_command(&root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .or_else(|| Self::read_branch_from_git_dir(&git_dir));

        let commit_sha = Self::run_git_command(&root, &["rev-parse", "HEAD"])
            .or_else(|| Self::read_sha_from_git_dir(&git_dir));

        let author = Self::run_git_command(&root, &["log", "-1", "--format=%an <%ae>"]);
        let commit_message = Self::run_git_command(&root, &["log", "-1", "--format=%s"]);
        let is_dirty = Self::run_git_command(&root, &["status", "--porcelain"])
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        GitMetadata {
            branch,
            commit_sha,
            author,
            commit_message,
            is_dirty,
        }
    }

    fn run_git_command(dir: &Path, args: &[&str]) -> Option<String> {
        let output = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .ok()?;

        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        None
    }

    fn read_branch_from_git_dir(git_dir: &Path) -> Option<String> {
        let head_file = git_dir.join("HEAD");
        if let Ok(content) = std::fs::read_to_string(head_file) {
            let line = content.trim();
            if let Some(ref_path) = line.strip_prefix("ref: refs/heads/") {
                return Some(ref_path.to_string());
            }
        }
        None
    }

    fn read_sha_from_git_dir(git_dir: &Path) -> Option<String> {
        let head_file = git_dir.join("HEAD");
        if let Ok(content) = std::fs::read_to_string(&head_file) {
            let line = content.trim();
            if let Some(ref_path) = line.strip_prefix("ref: ") {
                let target = git_dir.join(ref_path);
                if let Ok(sha) = std::fs::read_to_string(target) {
                    return Some(sha.trim().to_string());
                }
            } else if line.len() == 40 {
                return Some(line.to_string());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_git_metadata_on_current_repo() {
        let meta = GitMetadataExtractor::extract(".");
        // Since we are running inside the git workspace, commit_sha or branch should be resolved
        println!("Extracted git metadata: {:?}", meta);
        assert!(meta.branch.is_some() || meta.commit_sha.is_some());
    }
}
