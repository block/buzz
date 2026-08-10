//! Filesystem watching for the Documents vault.
//!
//! Two deliberate differences from the Onyx watcher this is based on:
//!
//!  * The `vault-file-modified` payload carries each path's mtime, not just the
//!    path. Our own `write_vault_file` trips the watcher, and without an mtime
//!    the frontend cannot distinguish that echo from a genuine external edit —
//!    it would cancel the pending autosave and discard whatever the user typed
//!    during the poll window.
//!
//!  * Dotted paths are filtered before emitting. Onyx skips hidden entries when
//!    building its tree but watches them anyway, so a vault containing `.git`
//!    or `.obsidian` thrashes the event stream during ordinary editor activity.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::vault_path::VaultState;

/// Emitted when a watched file's contents changed.
pub const VAULT_FILE_MODIFIED_EVENT: &str = "vault-file-modified";
/// Emitted when the set of files changed (create / delete / rename).
pub const VAULT_FILES_CHANGED_EVENT: &str = "vault-files-changed";

#[derive(Clone, Serialize)]
pub struct VaultModifiedEntry {
    path: String,
    /// Milliseconds since the epoch, or 0 when unavailable.
    modified_ms: u64,
}

#[derive(Default)]
pub struct VaultWatcherState {
    inner: Mutex<Option<RecommendedWatcher>>,
}

impl VaultWatcherState {
    fn replace(&self, watcher: Option<RecommendedWatcher>) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|e| e.to_string())?;
        // Dropping the previous watcher unregisters it.
        *guard = watcher;
        Ok(())
    }
}

/// Whether any component of `path` starts with a dot.
///
/// Matches the tree walker's hidden-entry rule, so the watcher never reports
/// churn the user cannot see.
fn is_hidden(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| name.starts_with('.'))
    })
}

fn modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0)
}

/// Splits an event's paths into the ones worth telling the frontend about.
fn visible_paths(event: &Event, vault_root: &Path) -> Vec<PathBuf> {
    event
        .paths
        .iter()
        .filter(|path| {
            // Compare relative to the vault so a dotted directory *above* the
            // vault (a vault inside ~/.config, say) does not filter everything.
            let relative = path.strip_prefix(vault_root).unwrap_or(path);
            !is_hidden(relative)
        })
        .cloned()
        .collect()
}

fn handle_event(app: &AppHandle, vault_root: &Path, event: Event) {
    let paths = visible_paths(&event, vault_root);
    if paths.is_empty() {
        return;
    }

    match event.kind {
        EventKind::Modify(_) => {
            let entries: Vec<VaultModifiedEntry> = paths
                .iter()
                .map(|path| VaultModifiedEntry {
                    modified_ms: modified_ms(path),
                    path: path.to_string_lossy().to_string(),
                })
                .collect();
            let _ = app.emit(VAULT_FILE_MODIFIED_EVENT, entries);
        }
        EventKind::Create(_) | EventKind::Remove(_) => {
            let _ = app.emit(VAULT_FILES_CHANGED_EVENT, ());
        }
        _ => {}
    }
}

/// Starts watching the active vault. Replaces any existing watcher.
#[tauri::command]
pub async fn start_vault_watch(
    app: AppHandle,
    state: State<'_, VaultState>,
    watcher_state: State<'_, VaultWatcherState>,
) -> Result<(), String> {
    let root = state.require_root()?;
    let event_root = root.clone();

    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<Event>| {
            if let Ok(event) = result {
                handle_event(&app, &event_root, event);
            }
        },
        Config::default().with_poll_interval(Duration::from_secs(1)),
    )
    .map_err(|e| format!("Could not watch the vault folder: {e}"))?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("Could not watch the vault folder: {e}"))?;

    watcher_state.replace(Some(watcher))
}

#[tauri::command]
pub async fn stop_vault_watch(watcher_state: State<'_, VaultWatcherState>) -> Result<(), String> {
    watcher_state.replace(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_detection_covers_any_component() {
        assert!(is_hidden(Path::new(".git")));
        assert!(is_hidden(Path::new(".git/objects/ab/cdef")));
        assert!(is_hidden(Path::new("Notes/.obsidian/workspace.json")));
        assert!(is_hidden(Path::new("Notes/.hidden.md")));

        assert!(!is_hidden(Path::new("Notes/plain.md")));
        assert!(!is_hidden(Path::new("Notes/Sub Folder/note.md")));
        // A dot inside a name, rather than leading it, is ordinary.
        assert!(!is_hidden(Path::new("Notes/v1.2.notes.md")));
    }

    #[test]
    fn a_dotted_directory_above_the_vault_does_not_filter_everything() {
        // A vault living under ~/.config must still report its own files.
        let root = Path::new("/home/user/.config/vault");
        let event = Event {
            attrs: Default::default(),
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![root.join("Notes/plain.md")],
        };
        assert_eq!(visible_paths(&event, root).len(), 1);
    }

    #[test]
    fn hidden_paths_inside_the_vault_are_filtered_out() {
        let root = Path::new("/vault");
        let event = Event {
            attrs: Default::default(),
            kind: EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![
                root.join(".git/index"),
                root.join("Notes/plain.md"),
                root.join(".obsidian/workspace.json"),
            ],
        };
        let visible = visible_paths(&event, root);
        assert_eq!(visible, vec![root.join("Notes/plain.md")]);
    }
}
