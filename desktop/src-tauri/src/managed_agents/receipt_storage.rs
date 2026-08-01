use std::fs;
use std::path::{Path, PathBuf};

use tauri::AppHandle;

use super::{managed_agents_base_dir, ManagedAgentRuntimeKey};

pub fn remove_agent_runtime_receipt(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
) -> Result<(), String> {
    let path = managed_agents_base_dir(app)?
        .join("agent-pids")
        .join(format!("{}.json", key.runtime_id()));
    remove_agent_runtime_receipt_path(&path)
}

pub fn receipt_deletion_tombstone(path: &Path) -> PathBuf {
    path.with_extension("json.delete-pending")
}

pub fn finish_pending_agent_runtime_receipt_deletion(path: &Path) -> Result<(), String> {
    let tombstone = receipt_deletion_tombstone(path);
    if !tombstone.exists() {
        return Ok(());
    }
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&tombstone)
        .and_then(|file| file.sync_all())
        .map_err(|error| {
            format!(
                "failed to flush managed-agent receipt tombstone {}: {error}",
                tombstone.display()
            )
        })?;
    fs::remove_file(&tombstone).map_err(|error| {
        format!(
            "failed to remove managed-agent receipt tombstone {}: {error}",
            tombstone.display()
        )
    })?;
    verify_absent(&tombstone)
}

pub fn remove_agent_runtime_receipt_path(path: &Path) -> Result<(), String> {
    let tombstone = receipt_deletion_tombstone(path);
    if !path.exists() {
        return finish_pending_agent_runtime_receipt_deletion(path);
    }
    if tombstone.exists() {
        return Err(format!(
            "both runtime receipt and deletion tombstone exist for {}",
            path.display()
        ));
    }
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| {
            format!(
                "failed to flush managed-agent runtime receipt {}: {error}",
                path.display()
            )
        })?;
    fs::rename(path, &tombstone).map_err(|error| {
        format!(
            "failed to retire managed-agent runtime receipt {}: {error}",
            path.display()
        )
    })?;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&tombstone)
        .and_then(|file| file.sync_all())
        .map_err(|error| {
            format!(
                "failed to durably commit runtime receipt retirement {}: {error}",
                tombstone.display()
            )
        })?;
    finish_pending_agent_runtime_receipt_deletion(path)?;
    verify_absent(path)
}

fn verify_absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "managed-agent runtime receipt artifact still exists: {}",
            path.display()
        )),
        Err(error) => Err(format!(
            "failed to verify managed-agent runtime receipt artifact {}: {error}",
            path.display()
        )),
    }
}
