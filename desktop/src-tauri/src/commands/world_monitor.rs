use buzz_command_sources_pkg::{
    mcp_http::{McpHttpClient, McpHttpError},
    usage::WorldMonitorUsageLedger,
    DEFAULT_WORLD_MONITOR_ENDPOINT, WORLD_MONITOR_KEYCHAIN_KEY,
};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use zeroize::Zeroize;

use crate::secret_store::SecretStore;

const DAILY_LIMIT: u8 = 25;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldMonitorConnectionStatus {
    NotConfigured,
    Configured,
    Connected,
    Unavailable,
    Unauthorised,
    QuotaLimited,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldMonitorConnectionView {
    pub endpoint: String,
    pub status: WorldMonitorConnectionStatus,
    pub brief_used: u8,
    pub brief_limit: u8,
    pub direct_used: u8,
    pub direct_limit: u8,
}

fn usage_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join("world-monitor-usage.json"))
        .map_err(|_| command_error())
}

fn command_error() -> String {
    "World Monitor configuration is unavailable.".to_string()
}

fn valid_api_key(value: &str) -> bool {
    value.starts_with("wm_live_")
        && value.len() >= 16
        && value.len() <= 512
        && value.is_ascii()
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
}

fn secret_store() -> &'static SecretStore {
    SecretStore::shared(crate::app_state::keyring_service())
}

fn usage_counts(app: &AppHandle) -> (u8, u8) {
    usage_path(app)
        .ok()
        .and_then(|path| {
            WorldMonitorUsageLedger::new(path)
                .snapshot(chrono::Local::now())
                .ok()
        })
        .map_or((0, 0), |snapshot| {
            (snapshot.brief_used, snapshot.direct_used)
        })
}

fn view(app: &AppHandle, status: WorldMonitorConnectionStatus) -> WorldMonitorConnectionView {
    let (brief_used, direct_used) = usage_counts(app);
    WorldMonitorConnectionView {
        endpoint: DEFAULT_WORLD_MONITOR_ENDPOINT.to_string(),
        status,
        brief_used,
        brief_limit: DAILY_LIMIT,
        direct_used,
        direct_limit: DAILY_LIMIT,
    }
}

#[tauri::command]
pub async fn get_world_monitor_connection(
    app: AppHandle,
) -> Result<WorldMonitorConnectionView, String> {
    let status = match secret_store().load(WORLD_MONITOR_KEYCHAIN_KEY) {
        Ok(Some(mut key)) => {
            key.zeroize();
            WorldMonitorConnectionStatus::Configured
        }
        Ok(None) => WorldMonitorConnectionStatus::NotConfigured,
        Err(_) => WorldMonitorConnectionStatus::Unavailable,
    };
    Ok(view(&app, status))
}

#[tauri::command]
pub async fn save_world_monitor_api_key(
    app: AppHandle,
    mut api_key: String,
) -> Result<WorldMonitorConnectionView, String> {
    if !valid_api_key(&api_key) {
        api_key.zeroize();
        return Err("Enter a valid World Monitor API key.".to_string());
    }
    let stored = secret_store().store(WORLD_MONITOR_KEYCHAIN_KEY, &api_key);
    api_key.zeroize();
    stored.map_err(|_| command_error())?;
    Ok(view(&app, WorldMonitorConnectionStatus::Configured))
}

#[tauri::command]
pub async fn remove_world_monitor_api_key(
    app: AppHandle,
) -> Result<WorldMonitorConnectionView, String> {
    secret_store()
        .delete(WORLD_MONITOR_KEYCHAIN_KEY)
        .map_err(|_| command_error())?;
    Ok(view(&app, WorldMonitorConnectionStatus::NotConfigured))
}

#[tauri::command]
pub async fn test_world_monitor_connection(
    app: AppHandle,
) -> Result<WorldMonitorConnectionView, String> {
    let Some(mut api_key) = secret_store()
        .load(WORLD_MONITOR_KEYCHAIN_KEY)
        .map_err(|_| command_error())?
    else {
        return Ok(view(&app, WorldMonitorConnectionStatus::NotConfigured));
    };
    let client = McpHttpClient::world_monitor(DEFAULT_WORLD_MONITOR_ENDPOINT, api_key.clone());
    api_key.zeroize();
    let status = match client {
        Ok(client) => match client.list_tools().await {
            Ok(_) => WorldMonitorConnectionStatus::Connected,
            Err(McpHttpError::Unauthorized) => WorldMonitorConnectionStatus::Unauthorised,
            Err(McpHttpError::RateLimited) => WorldMonitorConnectionStatus::QuotaLimited,
            Err(_) => WorldMonitorConnectionStatus::Unavailable,
        },
        Err(_) => WorldMonitorConnectionStatus::Unavailable,
    };
    Ok(view(&app, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_bounded_live_keys() {
        assert!(valid_api_key("wm_live_12345678"));
        for value in [
            "wrong_1234567890",
            "wm_live_short",
            "wm_live_has space",
            "wm_live_has\ncontrol",
        ] {
            assert!(!valid_api_key(value));
        }
        assert!(!valid_api_key(&format!("wm_live_{}", "x".repeat(600))));
    }

    #[test]
    fn connection_view_serialisation_never_has_a_secret_field() {
        let value = serde_json::to_value(WorldMonitorConnectionView {
            endpoint: DEFAULT_WORLD_MONITOR_ENDPOINT.to_string(),
            status: WorldMonitorConnectionStatus::Configured,
            brief_used: 3,
            brief_limit: 25,
            direct_used: 4,
            direct_limit: 25,
        })
        .expect("serialize");
        assert_eq!(
            value
                .as_object()
                .expect("object")
                .keys()
                .collect::<Vec<_>>(),
            [
                "briefLimit",
                "briefUsed",
                "directLimit",
                "directUsed",
                "endpoint",
                "status"
            ]
        );
        assert!(!value.to_string().contains("wm_live_"));
    }
}
