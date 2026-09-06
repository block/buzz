use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use super::types::{OperationsStore, ScopedOperations, STORE_VERSION};

const MAX_SCOPES: usize = 16;
const MAX_WAKES: usize = 40;
const MAX_EPISODES: usize = 256;
const MAX_BATCHES: usize = 128;

fn store_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("agent-operations.json"))
        .map_err(|error| format!("cannot resolve operations app-data path: {error}"))
}

pub(crate) fn load(app: &AppHandle) -> Result<OperationsStore, String> {
    let path = store_path(app)?;
    if !path.exists() {
        return Ok(OperationsStore::default());
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("cannot read operations settings: {error}"))?;
    let store: OperationsStore = serde_json::from_slice(&bytes)
        .map_err(|error| format!("operations settings need recovery: {error}"))?;
    if store.version != STORE_VERSION {
        return Err(format!(
            "operations settings version {} is unsupported; expected {STORE_VERSION}",
            store.version
        ));
    }
    if store.scopes.len() > MAX_SCOPES {
        return Err("operations settings contain too many scopes".to_string());
    }
    if store.scopes.iter().any(|scope| {
        scope.delivery.digest_wakes.len() > MAX_WAKES
            || scope.delivery.episodes.len() > MAX_EPISODES
            || scope.delivery.alert_batches.len() > MAX_BATCHES
    }) {
        return Err("operations delivery state exceeds its supported bounds".to_string());
    }
    Ok(store)
}

pub(crate) fn save(app: &AppHandle, store: &mut OperationsStore) -> Result<(), String> {
    for scope in &mut store.scopes {
        scope
            .delivery
            .digest_wakes
            .sort_by(|a, b| a.date.cmp(&b.date));
        if scope.delivery.digest_wakes.len() > MAX_WAKES {
            let drain = scope.delivery.digest_wakes.len() - MAX_WAKES;
            scope.delivery.digest_wakes.drain(..drain);
        }
        if scope.delivery.episodes.len() > MAX_EPISODES {
            let inactive = scope
                .delivery
                .episodes
                .iter()
                .filter(|(_, episode)| !episode.active)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in inactive {
                if scope.delivery.episodes.len() <= MAX_EPISODES {
                    break;
                }
                scope.delivery.episodes.remove(&id);
            }
        }
        if scope.delivery.alert_batches.len() > MAX_BATCHES {
            while scope.delivery.alert_batches.len() > MAX_BATCHES {
                let Some(index) = scope
                    .delivery
                    .alert_batches
                    .iter()
                    .position(|batch| batch.event_id.is_some())
                else {
                    break;
                };
                scope.delivery.alert_batches.remove(index);
            }
            if scope.delivery.alert_batches.len() > MAX_BATCHES {
                return Err("operations delivery state contains too many pending alerts".into());
            }
        }
    }
    if store.scopes.len() > MAX_SCOPES {
        return Err("operations settings contain too many scopes".to_string());
    }
    let path = store_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create operations settings directory: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("cannot serialize operations settings: {error}"))?;
    crate::managed_agents::storage::atomic_write_json_restricted(&path, &bytes)
}

pub(crate) fn current_scope_mut<'a>(
    store: &'a mut OperationsStore,
    owner: &str,
    relay: &str,
) -> Option<&'a mut ScopedOperations> {
    store
        .scopes
        .iter_mut()
        .find(|scope| scope.owner_pubkey == owner && scope.relay_url == relay)
}

pub(crate) fn current_scope<'a>(
    store: &'a OperationsStore,
    owner: &str,
    relay: &str,
) -> Option<&'a ScopedOperations> {
    store
        .scopes
        .iter()
        .find(|scope| scope.owner_pubkey == owner && scope.relay_url == relay)
}
