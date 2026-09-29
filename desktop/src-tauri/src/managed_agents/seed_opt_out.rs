//! Persistent opt-out for built-in seed personas/teams the user deleted.
//!
//! Without this, `merge_personas` / `merge_teams` re-insert missing built-ins on
//! every load ("keeps coming back"). Prefer: only seed when missing AND not opted out.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use tauri::AppHandle;

use super::storage::managed_agents_base_dir;

fn opt_out_path_for_base(base: &Path) -> PathBuf {
    base.join("deleted-seed-ids.json")
}

pub(crate) fn opt_out_path<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(opt_out_path_for_base(&managed_agents_base_dir(app)?))
}

pub(crate) fn load_deleted_seed_ids_from_path(path: &Path) -> HashSet<String> {
    if !path.exists() {
        return HashSet::new();
    }
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str::<Vec<String>>(&content)
            .unwrap_or_default()
            .into_iter()
            .collect(),
        Err(_) => HashSet::new(),
    }
}

pub(crate) fn load_deleted_seed_ids<R: tauri::Runtime>(app: &AppHandle<R>) -> HashSet<String> {
    match opt_out_path(app) {
        Ok(path) => load_deleted_seed_ids_from_path(&path),
        Err(_) => HashSet::new(),
    }
}

pub(crate) fn remember_deleted_seed_id<R: tauri::Runtime>(
    app: &AppHandle<R>,
    id: &str,
) -> Result<(), String> {
    let path = opt_out_path(app)?;
    let mut ids = load_deleted_seed_ids_from_path(&path);
    if !ids.insert(id.to_string()) {
        return Ok(());
    }
    let mut list: Vec<String> = ids.into_iter().collect();
    list.sort();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create managed-agents dir: {error}"))?;
    }
    let json = serde_json::to_string_pretty(&list)
        .map_err(|error| format!("failed to serialize deleted seed ids: {error}"))?;
    fs::write(&path, json).map_err(|error| format!("failed to write deleted seed ids: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn remember_and_load_round_trip() {
        let dir = tempdir().unwrap();
        let path = opt_out_path_for_base(dir.path());
        assert!(load_deleted_seed_ids_from_path(&path).is_empty());

        let mut ids = HashSet::new();
        ids.insert("builtin:fizz".to_string());
        let list: Vec<String> = ids.iter().cloned().collect();
        fs::write(&path, serde_json::to_string(&list).unwrap()).unwrap();

        let loaded = load_deleted_seed_ids_from_path(&path);
        assert!(loaded.contains("builtin:fizz"));
    }

    #[test]
    fn missing_file_is_empty_set() {
        let dir = tempdir().unwrap();
        let path = opt_out_path_for_base(dir.path());
        assert!(load_deleted_seed_ids_from_path(&path).is_empty());
    }
}
