use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::managed_agents::storage::atomic_write_json_restricted;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqAuthorizationConfig {
    #[serde(default)]
    pub trust_store: Option<String>,
    pub receipt_root: String,
}

fn config_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("nxtlinq"))
        .map_err(|error| format!("could not locate Nxtlinq configuration storage: {error}"))
}

fn default_config(app: &AppHandle) -> Result<NxtlinqAuthorizationConfig, String> {
    Ok(NxtlinqAuthorizationConfig {
        trust_store: None,
        receipt_root: config_root(app)?.join("receipts").display().to_string(),
    })
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_root(app)?.join("authorization.json"))
}

fn validate_absolute_path(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path"));
    }
    if path
        .components()
        .any(|component| component.as_os_str().is_empty())
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

pub(crate) fn prepare_receipt_directory(path: &Path) -> Result<(), String> {
    validate_absolute_path(path, "receipt root")?;
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("inspect receipt root: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("receipt root must be a real directory, not a symlink".to_string());
        }
    } else {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("create receipt root {}: {error}", path.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("restrict receipt root {}: {error}", path.display()))?;
    }
    Ok(())
}

fn validate_config(config: &NxtlinqAuthorizationConfig) -> Result<(), String> {
    let trust_store = config
        .trust_store
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or("select a trusted-signers.json file")?;
    let trust_path = Path::new(trust_store);
    validate_absolute_path(trust_path, "trust store")?;
    if !trust_path.is_file() {
        return Err(format!(
            "trust store does not exist or is not a file: {}",
            trust_path.display()
        ));
    }
    let bytes = std::fs::read(trust_path)
        .map_err(|error| format!("read trust store {}: {error}", trust_path.display()))?;
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| format!("trust store is not valid JSON: {error}"))?;
    prepare_receipt_directory(Path::new(config.receipt_root.trim()))
}

#[tauri::command]
pub fn get_nxtlinq_authorization_config(
    app: AppHandle,
) -> Result<NxtlinqAuthorizationConfig, String> {
    let path = config_path(&app)?;
    if !path.exists() {
        return default_config(&app);
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("read Nxtlinq configuration {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse Nxtlinq configuration {}: {error}", path.display()))
}

#[tauri::command]
pub fn set_nxtlinq_authorization_config(
    app: AppHandle,
    mut config: NxtlinqAuthorizationConfig,
) -> Result<NxtlinqAuthorizationConfig, String> {
    config.trust_store = config
        .trust_store
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());
    config.receipt_root = config.receipt_root.trim().to_string();
    validate_config(&config)?;
    let root = config_root(&app)?;
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("create Nxtlinq configuration directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("restrict Nxtlinq configuration directory: {error}"))?;
    }
    let payload = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("serialize Nxtlinq configuration: {error}"))?;
    atomic_write_json_restricted(&config_path(&app)?, &payload)?;
    Ok(config)
}

#[tauri::command]
pub async fn pick_nxtlinq_trust_store(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Nxtlinq trust store", &["json"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = receiver
        .await
        .map_err(|_| "trust-store picker closed unexpectedly".to_string())?
    else {
        return Ok(None);
    };
    path.as_path()
        .map(|path| Some(path.display().to_string()))
        .ok_or("selected trust-store path is invalid".to_string())
}

#[tauri::command]
pub async fn pick_nxtlinq_directory(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = sender.send(path);
    });
    let Some(path) = receiver
        .await
        .map_err(|_| "directory picker closed unexpectedly".to_string())?
    else {
        return Ok(None);
    };
    path.as_path()
        .map(|path| Some(path.display().to_string()))
        .ok_or("selected directory path is invalid".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_requires_an_existing_json_trust_store() {
        let root = tempfile::tempdir().unwrap();
        let trust = root.path().join("trusted-signers.json");
        std::fs::write(&trust, "not json").unwrap();
        let config = NxtlinqAuthorizationConfig {
            trust_store: Some(trust.display().to_string()),
            receipt_root: root.path().join("receipts").display().to_string(),
        };
        assert!(validate_config(&config).unwrap_err().contains("valid JSON"));
        std::fs::write(&trust, r#"{"signers":[]}"#).unwrap();
        validate_config(&config).unwrap();
        assert!(root.path().join("receipts").is_dir());
    }
}
