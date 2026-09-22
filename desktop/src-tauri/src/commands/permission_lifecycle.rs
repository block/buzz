//! Desktop-owned recovery reader for managed ACP permission decisions.

use serde::Serialize;
use tauri::AppHandle;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionLifecycleSnapshot {
    records: Vec<serde_json::Value>,
}

#[tauri::command]
pub fn get_managed_agent_permission_lifecycle(
    app: AppHandle,
    pubkey: String,
    relay_url: String,
) -> Result<PermissionLifecycleSnapshot, String> {
    let key = crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let path = crate::managed_agents::managed_agent_permission_ledger_path(&app, &key)?;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PermissionLifecycleSnapshot {
                records: Vec::new(),
            });
        }
        Err(error) => return Err(format!("read permission lifecycle ledger: {error}")),
    };
    let file: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse permission lifecycle ledger: {error}"))?;
    let records = file
        .get("records")
        .and_then(serde_json::Value::as_object)
        .map(|records| records.values().cloned().collect())
        .unwrap_or_default();
    Ok(PermissionLifecycleSnapshot { records })
}
