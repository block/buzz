use nostr::Keys;
use tauri::{AppHandle, State};

use crate::{
    agent_operations::{
        calendar, channel_member_pubkeys, operations_lock, storage,
        types::{
            OperationsConfig, OperationsStatus, SaveOperationsConfig, ScopeDeliveryState,
            ScopedOperations, SCHEDULE_COPY,
        },
    },
    app_state::AppState,
    managed_agents::{load_managed_agents, BackendKind},
    relay::{relay_http_base_url, relay_ws_url_with_override},
};

fn active_scope(state: &AppState) -> Result<(String, String), String> {
    let owner = state.signing_keys()?.public_key().to_hex();
    let relay = buzz_core_pkg::relay::normalize_relay_url(&relay_ws_url_with_override(state))
        .map_err(|error| format!("invalid active relay: {error}"))?;
    Ok((owner, relay))
}

#[tauri::command]
pub async fn get_agent_operations_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<OperationsStatus, String> {
    let _workspace_guard = state.workspace_apply_lock.clone().lock_owned().await;
    let _guard = operations_lock().lock().await;
    let (owner, relay) = active_scope(&state)?;
    let store = storage::load(&app)?;
    let scope = storage::current_scope(&store, &owner, &relay);
    Ok(OperationsStatus {
        config: scope.map(|scope| scope.config.clone()).unwrap_or_default(),
        schedule: SCHEDULE_COPY,
        next_manila_boundary_utc: calendar::next_boundary(chrono::Utc::now()).to_rfc3339(),
        metric_coverage_since: scope.and_then(|scope| scope.delivery.metric_coverage_since),
        last_confirmed_digest: scope.and_then(|scope| scope.delivery.confirmed_digest.clone()),
    })
}

#[tauri::command]
pub async fn save_agent_operations_config(
    input: SaveOperationsConfig,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<OperationsStatus, String> {
    let _workspace_guard = state.workspace_apply_lock.clone().lock_owned().await;
    let (owner, relay) = active_scope(&state)?;
    let channel_id = input
        .channel_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let assistant_pubkey = input
        .assistant_pubkey
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    if input.enabled {
        let channel = channel_id
            .as_deref()
            .ok_or("Choose a destination channel before enabling operations automation")?;
        uuid::Uuid::parse_str(channel).map_err(|_| "Destination must be a valid channel UUID")?;
        let assistant = assistant_pubkey
            .as_deref()
            .ok_or("Choose a local managed assistant before enabling operations automation")?;
        nostr::PublicKey::from_hex(assistant)
            .map_err(|_| "Assistant must have a valid exact pubkey")?;
        {
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            let record = load_managed_agents(&app)?
                .into_iter()
                .find(|record| record.pubkey.eq_ignore_ascii_case(assistant))
                .ok_or("The selected managed assistant no longer exists")?;
            if record.backend != BackendKind::Local {
                return Err("The selected assistant must be a local managed agent".to_string());
            }
            let keys = Keys::parse(record.private_key_nsec.trim())
                .map_err(|_| "The selected assistant signing record is unavailable")?;
            if !keys.public_key().to_hex().eq_ignore_ascii_case(assistant) {
                return Err(
                    "The selected assistant signing record does not match its pubkey".into(),
                );
            }
        }
        let owner_keys = state.signing_keys()?;
        if !owner_keys
            .public_key()
            .to_hex()
            .eq_ignore_ascii_case(&owner)
        {
            return Err("Active owner changed while saving operations automation".to_string());
        }
        let members =
            channel_member_pubkeys(&state, &relay_http_base_url(&relay), &owner_keys, channel)
                .await
                .map_err(|error| format!("Could not verify channel membership: {error}"))?;
        if !members
            .iter()
            .any(|pubkey| pubkey.eq_ignore_ascii_case(&owner))
        {
            return Err("The active owner is not a member of the selected channel".to_string());
        }
        if !members
            .iter()
            .any(|pubkey| pubkey.eq_ignore_ascii_case(assistant))
        {
            return Err(
                "The selected assistant is not a member of the selected channel".to_string(),
            );
        }
    }

    let _guard = operations_lock().lock().await;
    let mut store = storage::load(&app)?;
    let config = OperationsConfig {
        enabled: input.enabled,
        channel_id,
        assistant_pubkey,
    };
    if let Some(scope) = storage::current_scope_mut(&mut store, &owner, &relay) {
        scope.config = config.clone();
        if !input.enabled {
            scope.delivery.metric_coverage_since = None;
        }
    } else {
        store.scopes.push(ScopedOperations {
            owner_pubkey: owner.clone(),
            relay_url: relay.clone(),
            config: config.clone(),
            delivery: ScopeDeliveryState::default(),
        });
    }
    storage::save(&app, &mut store)?;
    let scope = storage::current_scope(&store, &owner, &relay).expect("scope was inserted");
    Ok(OperationsStatus {
        config,
        schedule: SCHEDULE_COPY,
        next_manila_boundary_utc: calendar::next_boundary(chrono::Utc::now()).to_rfc3339(),
        metric_coverage_since: scope.delivery.metric_coverage_since,
        last_confirmed_digest: scope.delivery.confirmed_digest.clone(),
    })
}
