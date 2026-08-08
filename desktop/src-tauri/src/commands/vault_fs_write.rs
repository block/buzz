//! Mutating vault filesystem commands.
//!
//! Every path here is validated against the active vault before it is touched,
//! and the returned [`ValidatedVaultPath`] is the only thing the `fs::` calls
//! see — see `vault_path.rs` for why that distinction is load-bearing.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::Serialize;
use tauri::State;

use crate::commands::vault_path::{reject_move_into_self, ValidatedVaultPath, VaultState};

#[derive(Serialize)]
pub struct VaultWriteResult {
    /// Modification time of the file we just wrote, in milliseconds since the
    /// epoch.
    ///
    /// The frontend records this so the filesystem watcher can tell our own
    /// write apart from a genuine external edit. Without it, saving fires a
    /// change event that looks exactly like someone else editing the file, and
    /// the reconciler would discard keystrokes typed during the poll window.
    modified_ms: u64,
}

/// Milliseconds since the epoch for `path`'s mtime, or 0 when unavailable.
///
/// A missing mtime is not worth failing a successful write over; it only costs
/// the echo-suppression optimisation for that one save.
fn modified_ms(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|delta| delta.as_millis() as u64)
        .unwrap_or(0)
}

/// Writes `content`, replacing the file atomically.
///
/// Atomic because this runs on autosave over the user's real notes: a partial
/// write from a crash or a full disk would otherwise truncate a file the user
/// still has open. `atomic-write-file` writes to a sibling temp file and
/// renames over the target.
#[tauri::command]
pub async fn write_vault_file(
    state: State<'_, VaultState>,
    path: String,
    content: String,
) -> Result<VaultWriteResult, String> {
    let validated = state.validate(&path)?;
    tokio::task::spawn_blocking(move || {
        use atomic_write_file::AtomicWriteFile;

        let target = validated.as_path();
        let mut file = AtomicWriteFile::open(target)
            .map_err(|e| format!("Could not open the note for writing: {e}"))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("Could not write the note: {e}"))?;
        file.commit()
            .map_err(|e| format!("Could not save the note: {e}"))?;

        Ok(VaultWriteResult {
            modified_ms: modified_ms(target),
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Creates an empty note. Fails if something already exists at `path`.
#[tauri::command]
pub async fn create_vault_file(state: State<'_, VaultState>, path: String) -> Result<(), String> {
    let validated = state.validate(&path)?;
    tokio::task::spawn_blocking(move || {
        let target = validated.as_path();
        if target.exists() {
            return Err("A file with that name already exists.".to_string());
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create the containing folder: {e}"))?;
        }
        fs::write(target, "").map_err(|e| format!("Could not create the note: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn create_vault_folder(state: State<'_, VaultState>, path: String) -> Result<(), String> {
    let validated = state.validate(&path)?;
    tokio::task::spawn_blocking(move || {
        let target = validated.as_path();
        if target.exists() {
            return Err("A folder with that name already exists.".to_string());
        }
        fs::create_dir_all(target).map_err(|e| format!("Could not create the folder: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Renames or moves an entry. Both endpoints must be inside the vault.
#[tauri::command]
pub async fn rename_vault_entry(
    state: State<'_, VaultState>,
    old_path: String,
    new_path: String,
) -> Result<(), String> {
    let source = state.validate(&old_path)?;
    let destination = state.validate(&new_path)?;
    reject_move_into_self(&source, &destination)?;

    tokio::task::spawn_blocking(move || rename_blocking(&source, &destination))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

fn rename_blocking(
    source: &ValidatedVaultPath,
    destination: &ValidatedVaultPath,
) -> Result<(), String> {
    let from = source.as_path();
    let to = destination.as_path();

    if !from.exists() {
        return Err("That file no longer exists.".to_string());
    }
    if to.exists() {
        return Err("Something with that name already exists.".to_string());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create the destination folder: {e}"))?;
    }
    fs::rename(from, to).map_err(|e| format!("Could not move that item: {e}"))
}

/// Deletes a note, or a folder and everything under it.
#[tauri::command]
pub async fn delete_vault_entry(state: State<'_, VaultState>, path: String) -> Result<(), String> {
    let validated = state.validate(&path)?;
    tokio::task::spawn_blocking(move || {
        let target = validated.as_path();
        if target.is_dir() {
            fs::remove_dir_all(target).map_err(|e| format!("Could not delete the folder: {e}"))
        } else {
            fs::remove_file(target).map_err(|e| format!("Could not delete the note: {e}"))
        }
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(tag: &str) -> (PathBuf, VaultState) {
        let root =
            std::env::temp_dir().join(format!("buzz-vault-write-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Notes")).unwrap();
        fs::write(root.join("Notes/plain.md"), "# plain").unwrap();

        let state = VaultState::default();
        state.set(root.clone()).unwrap();
        (root, state)
    }

    fn validated(state: &VaultState, path: &Path) -> ValidatedVaultPath {
        state.validate(&path.to_string_lossy()).unwrap()
    }

    #[test]
    fn rename_moves_a_note_into_another_folder() {
        let (root, state) = fixture("move");
        fs::create_dir_all(root.join("Archive")).unwrap();

        let source = validated(&state, &root.join("Notes/plain.md"));
        let destination = validated(&state, &root.join("Archive/plain.md"));
        rename_blocking(&source, &destination).unwrap();

        assert!(!root.join("Notes/plain.md").exists());
        assert_eq!(
            fs::read_to_string(root.join("Archive/plain.md")).unwrap(),
            "# plain"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_refuses_to_clobber_an_existing_entry() {
        let (root, state) = fixture("clobber");
        fs::write(root.join("Notes/other.md"), "# other").unwrap();

        let source = validated(&state, &root.join("Notes/plain.md"));
        let destination = validated(&state, &root.join("Notes/other.md"));
        assert!(rename_blocking(&source, &destination).is_err());

        // The would-be victim is untouched.
        assert_eq!(
            fs::read_to_string(root.join("Notes/other.md")).unwrap(),
            "# other"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_reports_a_missing_source() {
        let (root, state) = fixture("missing");
        let source = validated(&state, &root.join("Notes/gone.md"));
        let destination = validated(&state, &root.join("Notes/new.md"));
        assert!(rename_blocking(&source, &destination).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_rejects_moving_a_folder_into_itself() {
        let (root, state) = fixture("intoself");
        let source = validated(&state, &root.join("Notes"));
        let nested = validated(&state, &root.join("Notes/Inner"));
        let sibling = validated(&state, &root.join("Elsewhere"));

        assert!(reject_move_into_self(&source, &nested).is_err());
        assert!(reject_move_into_self(&source, &sibling).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn modified_ms_reflects_a_written_file() {
        let (root, _state) = fixture("mtime");
        let stamp = modified_ms(&root.join("Notes/plain.md"));
        assert!(stamp > 0, "a freshly written file must report an mtime");

        // A path that does not exist degrades to 0 rather than failing.
        assert_eq!(modified_ms(&root.join("Notes/absent.md")), 0);
        let _ = fs::remove_dir_all(&root);
    }

    /// Deleting a linked folder must unlink it, never empty out its target.
    ///
    /// Containment here is lexical, so a folder the user symlinked into their
    /// vault is reachable on purpose — which means "delete folder" can be aimed
    /// at a link pointing anywhere on disk. `delete_vault_entry` branches on
    /// `is_dir()`, which *follows* the link, so it calls `remove_dir_all` on a
    /// symlink. That is only safe because `remove_dir_all` refuses to descend
    /// through one (the fix for CVE-2022-21658); if that ever stopped holding,
    /// or the branch were rewritten to canonicalize first, deleting a linked
    /// folder would silently destroy the real directory behind it.
    #[cfg(unix)]
    #[test]
    fn deleting_a_linked_folder_removes_the_link_not_its_target() {
        let (root, state) = fixture("unlink");
        let outside = root.parent().unwrap().join(format!(
            "buzz-vault-write-outside-{}-unlink",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("precious.md"), "IRREPLACEABLE").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Linked")).unwrap();

        let target = validated(&state, &root.join("Linked"));
        let path = target.as_path();
        assert!(
            path.is_dir(),
            "is_dir() follows the link, as the command does"
        );
        assert!(fs::remove_dir_all(path).is_ok());

        assert!(
            root.join("Linked").symlink_metadata().is_err(),
            "the link itself must be gone"
        );
        assert_eq!(
            fs::read_to_string(outside.join("precious.md")).unwrap(),
            "IRREPLACEABLE",
            "the link's target must be untouched"
        );

        let _ = fs::remove_dir_all(&outside);
        let _ = fs::remove_dir_all(&root);
    }
}
