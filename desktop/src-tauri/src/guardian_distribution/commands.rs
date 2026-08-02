use super::{
    activate_receipt, builtin_manifest, deactivate, install_current, load_active_receipt, rollback,
    rollback_available, uninstall_active,
};
use crate::commands::numbat_findings::{
    reconcile_managed_numbat_hooks, uninstall_managed_numbat_hooks,
};
use crate::managed_agents::managed_agents_base_dir;
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tauri::AppHandle;

static LIFECYCLE_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GuardianNumbatStatus {
    state: String,
    provenance: String,
    version: Option<String>,
    digest_suffix: Option<String>,
    rollback_available: bool,
    target: String,
    detail: String,
}

#[tauri::command]
pub(crate) fn get_guardian_numbat_status(app: AppHandle) -> GuardianNumbatStatus {
    let target = current_target();
    let root = match component_root(&app) {
        Ok(root) => root,
        Err(detail) => return status("error", "none", None, None, false, target, detail),
    };
    status_from_root(&root, target)
}

#[tauri::command]
pub(crate) async fn activate_guardian_numbat(
    app: AppHandle,
) -> Result<GuardianNumbatStatus, String> {
    let _guard = lifecycle_lock().lock().await;
    let target = current_target();
    let root = component_root(&app)?;
    let previous = active_receipt_path(&root)?;
    let manifest = builtin_manifest()?;
    let artifact = manifest.artifact_for(std::env::consts::OS, std::env::consts::ARCH)?;
    let receipt_path = root
        .join("versions")
        .join(&manifest.version)
        .join(&target)
        .join("receipt.json");
    if !receipt_path.is_file() {
        return Err(format!(
            "Guardian Numbat {} for {}/{} is not staged",
            manifest.version, artifact.os, artifact.arch
        ));
    }
    activate_receipt(&root, &receipt_path)?;
    finish_activation(&app, &root, previous.as_deref())?;
    Ok(status_from_root(&root, target))
}

#[tauri::command]
pub(crate) async fn install_guardian_numbat(
    app: AppHandle,
) -> Result<GuardianNumbatStatus, String> {
    let _guard = lifecycle_lock().lock().await;
    let target = current_target();
    let root = component_root(&app)?;
    let previous = active_receipt_path(&root)?;
    install_current(&root, &target, env!("CARGO_PKG_VERSION")).await?;
    finish_activation(&app, &root, previous.as_deref())?;
    Ok(status_from_root(&root, target))
}

#[tauri::command]
pub(crate) async fn deactivate_guardian_numbat(
    app: AppHandle,
) -> Result<GuardianNumbatStatus, String> {
    let _guard = lifecycle_lock().lock().await;
    let target = current_target();
    let root = component_root(&app)?;
    if let Some((_, binary)) = load_active_receipt(&root)? {
        remove_hooks_or_restore(&app, &binary)?;
    }
    deactivate(&root)?;
    Ok(status_from_root(&root, target))
}

#[tauri::command]
pub(crate) async fn rollback_guardian_numbat(
    app: AppHandle,
) -> Result<GuardianNumbatStatus, String> {
    let _guard = lifecycle_lock().lock().await;
    let target = current_target();
    let root = component_root(&app)?;
    let (_, current_binary) = load_active_receipt(&root)?.ok_or("Guardian Numbat is not active")?;
    let current_receipt = current_binary
        .parent()
        .ok_or("Guardian binary has no version directory")?
        .join("receipt.json");
    rollback(&root)?;
    let (_, previous_binary) =
        load_active_receipt(&root)?.ok_or("Guardian rollback did not publish an active version")?;
    if let Err(error) = reconcile_managed_numbat_hooks(&app, &previous_binary) {
        activate_receipt(&root, &current_receipt)?;
        reconcile_managed_numbat_hooks(&app, &current_binary)?;
        return Err(format!("Guardian rollback hook reconciliation failed; restored the prior active version: {error}"));
    }
    Ok(status_from_root(&root, target))
}

#[tauri::command]
pub(crate) async fn uninstall_guardian_numbat(
    app: AppHandle,
) -> Result<GuardianNumbatStatus, String> {
    let _guard = lifecycle_lock().lock().await;
    let target = current_target();
    let root = component_root(&app)?;
    if let Some((_, binary)) = load_active_receipt(&root)? {
        remove_hooks_or_restore(&app, &binary)?;
    }
    uninstall_active(&root)?;
    Ok(status_from_root(&root, target))
}

fn lifecycle_lock() -> &'static tokio::sync::Mutex<()> {
    LIFECYCLE_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn active_receipt_path(root: &Path) -> Result<Option<PathBuf>, String> {
    let Some((_, binary)) = load_active_receipt(root)? else {
        return Ok(None);
    };
    let directory = binary
        .parent()
        .ok_or("Guardian binary has no receipt directory")?;
    Ok(Some(directory.join("receipt.json")))
}

fn finish_activation(
    app: &AppHandle,
    root: &Path,
    previous_receipt: Option<&Path>,
) -> Result<(), String> {
    let (_, active_binary) = load_active_receipt(root)?
        .ok_or("Guardian activation did not publish an active version")?;
    if let Err(error) = reconcile_managed_numbat_hooks(app, &active_binary) {
        if let Some(previous_receipt) = previous_receipt {
            activate_receipt(root, previous_receipt)?;
            let (_, previous_binary) = load_active_receipt(root)?
                .ok_or("Guardian recovery did not restore the previous active version")?;
            reconcile_managed_numbat_hooks(app, &previous_binary)?;
        } else {
            let _ = uninstall_managed_numbat_hooks(&active_binary);
            deactivate(root)?;
        }
        return Err(format!(
            "Guardian hook reconciliation failed; restored the prior lifecycle state: {error}"
        ));
    }
    Ok(())
}

fn remove_hooks_or_restore(app: &AppHandle, binary: &Path) -> Result<(), String> {
    if let Err(error) = uninstall_managed_numbat_hooks(binary) {
        return match reconcile_managed_numbat_hooks(app, binary) {
            Ok(()) => Err(format!(
                "Guardian hook removal failed; restored the active hook configuration: {error}"
            )),
            Err(restore_error) => Err(format!(
                "Guardian hook removal failed and the active hook configuration could not be fully restored: {error}; restoration: {restore_error}"
            )),
        };
    }
    Ok(())
}

pub(crate) fn recover_guardian_numbat(app: &AppHandle) -> Result<bool, String> {
    super::recover_activation(&component_root(app)?)
}

pub(crate) fn recover_guardian_numbat_on_boot(app: &AppHandle) {
    if let Err(error) = recover_guardian_numbat(app) {
        eprintln!("buzz-desktop: Guardian activation recovery failed: {error}");
    }
}

fn component_root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("numbat/components"))
}

fn status_from_root(root: &Path, target: String) -> GuardianNumbatStatus {
    match load_active_receipt(root) {
        Ok(Some((receipt, _))) => {
            let suffix = receipt
                .binary_sha256
                .get(receipt.binary_sha256.len().saturating_sub(12)..)
                .map(ToOwned::to_owned);
            status(
                "active",
                "buzz_managed",
                Some(receipt.version),
                suffix,
                rollback_available(root).unwrap_or(false),
                target,
                "Buzz-managed Numbat is active and its receipt and binary hashes match.".into(),
            )
        }
        Ok(None) => status(
            "not_active",
            "external_unmanaged",
            None,
            None,
            false,
            target,
            "No Buzz-managed Numbat version is active; PATH discovery remains external and unmanaged."
                .into(),
        ),
        Err(detail) => status("tampered", "buzz_managed", None, None, false, target, detail),
    }
}

fn status(
    state: &str,
    provenance: &str,
    version: Option<String>,
    digest_suffix: Option<String>,
    rollback_available: bool,
    target: String,
    detail: String,
) -> GuardianNumbatStatus {
    GuardianNumbatStatus {
        state: state.into(),
        provenance: provenance.into(),
        version,
        digest_suffix,
        rollback_available,
        target,
        detail,
    }
}

fn current_target() -> String {
    option_env!("TAURI_ENV_TARGET_TRIPLE")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardian_distribution::{load_verified_receipt, InstalledReceipt};
    use sha2::{Digest, Sha256};
    use std::fs;

    fn staged(root: &Path, target: &str) -> PathBuf {
        let dir = root.join(format!("versions/0.1.2/{target}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("numbat"), b"managed binary").unwrap();
        let receipt = InstalledReceipt {
            schema_version: 1,
            component: "numbat".into(),
            version: "0.1.2".into(),
            target: target.into(),
            manifest_sha256: "1".repeat(64),
            archive_sha256: "2".repeat(64),
            binary_sha256: hex::encode(Sha256::digest(b"managed binary")),
            binary_path: "numbat".into(),
            source_commit: "0e41ad66f5557f412eae330576271f2ee809d3de".into(),
            installed_at: "2026-08-02T00:00:00Z".into(),
            installer_buzz_version: "1.0.0".into(),
        };
        let path = dir.join("receipt.json");
        receipt.write_to(&path).unwrap();
        load_verified_receipt(root, &path).unwrap();
        path
    }

    #[test]
    fn status_distinguishes_absent_active_and_tampered() {
        let root = tempfile::tempdir().unwrap();
        let target = "test-target".to_string();
        assert_eq!(
            status_from_root(root.path(), target.clone()).state,
            "not_active"
        );

        let receipt = staged(root.path(), &target);
        activate_receipt(root.path(), &receipt).unwrap();
        let active = status_from_root(root.path(), target.clone());
        assert_eq!(active.state, "active");
        assert_eq!(active.provenance, "buzz_managed");
        assert_eq!(active.version.as_deref(), Some("0.1.2"));

        fs::write(receipt.parent().unwrap().join("numbat"), b"changed").unwrap();
        assert_eq!(status_from_root(root.path(), target).state, "tampered");
    }
}
