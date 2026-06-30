use tauri::AppHandle;

use crate::{app_state::AppState, managed_agents::load_managed_agents};

use super::agents::ProfileReconcileData;

pub(super) fn refresh_auth_tag(
    app: &AppHandle,
    state: &AppState,
    pubkey: &str,
    data: &mut ProfileReconcileData,
) {
    let result = (|| -> Result<(), String> {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let records = load_managed_agents(app)?;
        let record = records
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?;
        data.auth_tag = record.auth_tag.clone();
        Ok(())
    })();

    if let Err(error) = result {
        eprintln!(
            "buzz-desktop: profile reconciliation using pre-start auth tag for agent {pubkey}: {error}"
        );
    }
}
