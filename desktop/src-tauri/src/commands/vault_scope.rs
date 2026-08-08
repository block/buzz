//! Choosing and activating the Documents vault folder.
//!
//! These are the only commands that accept a path the renderer chose. Every
//! other vault command reads the root from [`VaultState`] — see its docs.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::commands::vault_path::VaultState;

#[derive(Serialize)]
pub struct VaultInfo {
    /// Absolute path of the active vault, in the spelling the user chose.
    path: String,
    /// Basename, for display in the Documents header.
    name: String,
}

/// Show a folder picker and return the chosen path, or `None` when cancelled.
///
/// Selection only — the caller must still `set_active_vault` to grant access.
/// Mirrors the `pick_save_path` oneshot-channel bridge in `export_util.rs`.
#[tauri::command]
pub async fn pick_vault_folder(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Choose a vault folder")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });

    let selected = rx.await.map_err(|_| "dialog cancelled".to_string())?;
    let Some(folder) = selected else {
        return Ok(None);
    };

    let path = folder
        .as_path()
        .ok_or_else(|| "Folder dialog returned an invalid path".to_string())?;
    Ok(Some(path.to_string_lossy().to_string()))
}

/// Sanity-checks a candidate vault root without mutating anything.
///
/// Keeps Onyx's `set_vault_scope` guards (exists, is a directory, is not the
/// filesystem root, is not `$HOME` itself) and drops its `tauri-plugin-fs`
/// scope widening — Buzz has no such plugin and needs none, because every read
/// and write goes through a Rust command rather than the plugin's JS API.
fn validate_vault_root(candidate: &Path) -> Result<PathBuf, String> {
    if !candidate.is_absolute() {
        return Err("The vault folder must be an absolute path.".to_string());
    }
    if !candidate.exists() {
        return Err("That folder does not exist.".to_string());
    }
    if !candidate.is_dir() {
        return Err("The vault must be a folder, not a file.".to_string());
    }
    if candidate.parent().is_none() {
        return Err("The filesystem root cannot be used as a vault.".to_string());
    }
    if let Some(home) = dirs::home_dir() {
        // Comparing canonical spellings so `/home/x` and a symlinked `~` agree.
        let same_as_home = match (candidate.canonicalize(), home.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => candidate == home,
        };
        if same_as_home {
            return Err(
                "Your home folder is too broad to use as a vault. Choose a subfolder.".to_string(),
            );
        }
    }
    Ok(candidate.to_path_buf())
}

/// Activate `vault_path` as the vault every other vault command operates within.
#[tauri::command]
pub async fn set_active_vault(
    state: State<'_, VaultState>,
    vault_path: String,
) -> Result<VaultInfo, String> {
    let candidate = PathBuf::from(vault_path.trim());
    let root = tokio::task::spawn_blocking(move || validate_vault_root(&candidate))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    let stored = state.set(root)?;
    Ok(vault_info(&stored))
}

/// Forget the active vault. Every subsequent vault command fails until one is set.
#[tauri::command]
pub async fn clear_active_vault(state: State<'_, VaultState>) -> Result<(), String> {
    state.clear()
}

/// The active vault, or `None` when none is selected. Used on boot to reconcile
/// the frontend's stored path against the backend.
#[tauri::command]
pub async fn get_active_vault(state: State<'_, VaultState>) -> Result<Option<VaultInfo>, String> {
    Ok(state.root()?.as_deref().map(vault_info))
}

fn vault_info(root: &Path) -> VaultInfo {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());
    VaultInfo {
        path: root.to_string_lossy().to_string(),
        name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("buzz-vault-scope-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn accepts_an_ordinary_folder() {
        let root = temp_dir("ok");
        assert!(validate_vault_root(&root).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_a_missing_folder() {
        let root = temp_dir("missing");
        assert!(validate_vault_root(&root.join("nope")).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_a_file() {
        let root = temp_dir("file");
        let file = root.join("note.md");
        fs::write(&file, "x").unwrap();
        assert!(validate_vault_root(&file).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_a_relative_path() {
        assert!(validate_vault_root(Path::new("relative/vault")).is_err());
    }

    #[test]
    fn rejects_the_filesystem_root() {
        assert!(validate_vault_root(Path::new("/")).is_err());
    }

    #[test]
    fn rejects_the_home_directory_itself() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        assert!(
            validate_vault_root(&home).is_err(),
            "$HOME is too broad to grant wholesale"
        );
    }
}
