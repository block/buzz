//! Production startup and relay-readiness recovery for queued NIP-CB events.

use std::sync::Arc;

use chrono::Utc;
use tauri::{AppHandle, Manager};

use super::audit::{EncryptedBriefAudit, RelayBriefAuditPublisher};
use crate::app_state::AppState;

fn audit_store_path() -> Result<std::path::PathBuf, String> {
    crate::managed_agents::nest_dir()
        .map(|nest| nest.join("command-brief").join("audit.db"))
        .ok_or_else(|| "command brief recovery unavailable".to_string())
}

/// Rearm and republish one bounded batch for the current owner.
///
/// This command is safe to call on app startup, every disconnected-to-connected
/// transition, and manual recovery. The spool owns retry bounding and exact-ID
/// idempotency.
#[tauri::command]
pub async fn recover_command_brief_publications(app: AppHandle) -> Result<u32, String> {
    let state = app.state::<AppState>();
    if state
        .identity_lost
        .load(std::sync::atomic::Ordering::Acquire)
        || state
            .keyring_locked
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Ok(0);
    }
    let owner_keys = state
        .keys
        .lock()
        .map_err(|_| "command brief recovery unavailable".to_string())?
        .clone();
    let audit = EncryptedBriefAudit::new(
        audit_store_path()?,
        owner_keys,
        Arc::new(RelayBriefAuditPublisher::new(app)),
    );
    let recovered = audit
        .recover_on_relay_ready(Utc::now().timestamp())
        .await
        .map_err(|_| "command brief recovery unavailable".to_string())?;
    u32::try_from(recovered).map_err(|_| "command brief recovery unavailable".to_string())
}
