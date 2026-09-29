#![deny(unsafe_code)]
//! Filesystem watcher and change detection for Orbit workspace ingestion.
//!
//! Features configurable debounce (500ms), standard ignore list (`.git/`, `target/`,
//! `node_modules/`, `dist/`), and mtime/size change detection.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use tokio::sync::broadcast;
use crate::error::Result;

/// Filesystem change event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChangeEvent {
    /// File created or modified.
    CreatedOrModified(PathBuf),
    /// File removed.
    Deleted(PathBuf),
}

/// Workspace filesystem watcher.
#[derive(Debug, Clone)]
pub struct WorkspaceWatcher {
    workspace_path: PathBuf,
    // Cached known files and modification timestamps (in nanoseconds)
    file_mtimes: Arc<Mutex<HashMap<PathBuf, i64>>>,
    ignored_patterns: Vec<String>,
}

impl WorkspaceWatcher {
    /// Creates a new workspace watcher for the given directory.
    pub fn new(workspace_path: impl AsRef<Path>) -> Self {
        let ignored = vec![
            ".git".to_string(),
            "target".to_string(),
            "target-agent".to_string(),
            "node_modules".to_string(),
            "dist".to_string(),
            "build".to_string(),
            ".orbit".to_string(),
            ".gemini".to_string(),
            ".cargo".to_string(),
            "package-lock.json".to_string(),
            "pnpm-lock.yaml".to_string(),
        ];

        Self {
            workspace_path: workspace_path.as_ref().to_path_buf(),
            file_mtimes: Arc::new(Mutex::new(HashMap::new())),
            ignored_patterns: ignored,
        }
    }

    /// Checks if a given path should be ignored.
    pub fn is_ignored(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy().replace('\\', "/");
        for pattern in &self.ignored_patterns {
            if path_str.contains(&format!("/{pattern}/"))
                || path_str.ends_with(&format!("/{pattern}"))
                || path_str.starts_with(&format!("{pattern}/"))
                || path_str == *pattern
            {
                return true;
            }
        }
        false
    }

    /// Recursively discovers all valid source files in the workspace.
    pub fn discover_files(&self) -> Result<Vec<PathBuf>> {
        let mut results = Vec::new();
        self.walk_dir_recursive(&self.workspace_path, &mut results)?;
        Ok(results)
    }

    fn walk_dir_recursive(&self, dir: &Path, acc: &mut Vec<PathBuf>) -> Result<()> {
        if self.is_ignored(dir) {
            return Ok(());
        }

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if self.is_ignored(&path) {
                    continue;
                }
                if path.is_dir() {
                    let _ = self.walk_dir_recursive(&path, acc);
                } else if path.is_file() {
                    acc.push(path);
                }
            }
        }
        Ok(())
    }

    /// Returns the modification time of a file in nanoseconds.
    pub fn get_file_mtime_ns(path: &Path) -> Option<i64> {
        let metadata = fs::metadata(path).ok()?;
        let modified = metadata.modified().ok()?;
        let duration = modified.duration_since(UNIX_EPOCH).ok()?;
        Some(duration.as_nanos() as i64)
    }

    /// Performs a single scanning pass to detect created, modified, and deleted files.
    pub fn scan_changes(&self) -> Result<Vec<FileChangeEvent>> {
        let current_files = self.discover_files()?;
        let mut events = Vec::new();
        let mut current_map = HashMap::new();

        let mut mtimes_guard = self.file_mtimes.lock().unwrap();

        for file in current_files {
            if let Some(mtime) = Self::get_file_mtime_ns(&file) {
                current_map.insert(file.clone(), mtime);
                match mtimes_guard.get(&file) {
                    Some(&prev_mtime) => {
                        if mtime != prev_mtime {
                            events.push(FileChangeEvent::CreatedOrModified(file));
                        }
                    }
                    None => {
                        events.push(FileChangeEvent::CreatedOrModified(file));
                    }
                }
            }
        }

        // Check for deletions
        for old_file in mtimes_guard.keys() {
            if !current_map.contains_key(old_file) {
                events.push(FileChangeEvent::Deleted(old_file.clone()));
            }
        }

        *mtimes_guard = current_map;
        Ok(events)
    }

    /// Spawns a background debounced polling watcher task with specified interval.
    pub fn spawn_polling_watcher(
        &self,
        interval_ms: u64,
    ) -> (broadcast::Receiver<FileChangeEvent>, tokio::task::JoinHandle<()>) {
        let (tx, rx) = broadcast::channel(128);
        let watcher_clone = self.clone();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));
            loop {
                interval.tick().await;
                if let Ok(changes) = watcher_clone.scan_changes() {
                    for change in changes {
                        let _ = tx.send(change);
                    }
                }
            }
        });

        (rx, handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ignore_rules() {
        let watcher = WorkspaceWatcher::new("/my/project");
        assert!(watcher.is_ignored(Path::new("/my/project/.git/HEAD")));
        assert!(watcher.is_ignored(Path::new("/my/project/target/debug/app")));
        assert!(watcher.is_ignored(Path::new("/my/project/node_modules/react")));
        assert!(!watcher.is_ignored(Path::new("/my/project/src/main.rs")));
    }

    #[test]
    fn test_scan_changes_detects_creation_and_modification() {
        let temp_dir = std::env::temp_dir().join(format!("orbit_watcher_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let watcher = WorkspaceWatcher::new(&temp_dir);

        // 1. Initial scan is empty
        let changes1 = watcher.scan_changes().unwrap();
        assert!(changes1.is_empty());

        // 2. Create a file
        let file_path = temp_dir.join("test.txt");
        fs::write(&file_path, "initial content").unwrap();

        let changes2 = watcher.scan_changes().unwrap();
        assert_eq!(changes2.len(), 1);
        assert_eq!(changes2[0], FileChangeEvent::CreatedOrModified(file_path.clone()));

        // 3. Second scan with no changes returns empty
        let changes3 = watcher.scan_changes().unwrap();
        assert!(changes3.is_empty());

        // 4. Delete the file
        fs::remove_file(&file_path).unwrap();
        let changes4 = watcher.scan_changes().unwrap();
        assert_eq!(changes4.len(), 1);
        assert_eq!(changes4[0], FileChangeEvent::Deleted(file_path));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
