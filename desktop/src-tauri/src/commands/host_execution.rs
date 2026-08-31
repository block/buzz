//! Native executor entry point. Transport subscription/publication and the host
//! picker remain gated off; a relay ACK is never a launch result. No host-key-only
//! connection is created: every query uses the existing active owner authority.
use crate::{
    app_state::AppState,
    managed_agents::{self, AcpAvailabilityStatus, AuthStatus},
};
use buzz_core_pkg::host_execution::{self, Action, Receipt};
use nostr::{Event, JsonUtil, Timestamp};
use tauri::{AppHandle, State};

/// Inspect the destination's own provisioned configuration. Only an opaque
/// revision and Rust catalog ID cross IPC; no key, env, path or OS hostname.
#[tauri::command]
pub async fn inspect_local_execution_config(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
    agent: String,
) -> Result<serde_json::Value, String> {
    super::hosts::owner_keys(&state, &expected_owner)?;
    crate::relay::assert_expected_relay_scope(
        Some(&expected_relay),
        &crate::relay::relay_api_base_url_with_override(&state),
    )?;
    let captured_owner = expected_owner.clone();
    let result = tokio::task::spawn_blocking(move || {
        let records = managed_agents::load_managed_agents(&app)?;
        let record = records
            .iter()
            .find(|r| r.pubkey == agent)
            .ok_or("agent is not provisioned on this executor")?;
        managed_agents::execution_agent_owner(record, &captured_owner)?;
        serde_json::to_value(managed_agents::local_execution_config(&app, record)?)
            .map_err(|_| "cannot serialize execution config".into())
    })
    .await
    .map_err(|_| "execution config task failed")?;
    super::hosts::owner_keys(&state, &expected_owner)?;
    crate::relay::assert_expected_relay_scope(
        Some(&expected_relay),
        &crate::relay::relay_api_base_url_with_override(&state),
    )?;
    result
}

/// Execute an owner-signed, destination-encrypted request using only this
/// Desktop's own approved configuration. Fresh registration lookup is mandatory
/// even on retry. An unreachable/deleted registration blocks work, not merely UI.
#[tauri::command]
pub async fn execute_host_command(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
    expected_relay: String,
    event: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let owner = super::hosts::owner_keys(&state, &expected_owner)?;
    let relay = buzz_core_pkg::relay::normalize_relay_url(&expected_relay)
        .map_err(|_| "invalid execution relay")?;
    crate::relay::assert_expected_relay_scope(
        Some(&relay),
        &crate::relay::relay_api_base_url_with_override(&state),
    )?;
    let event = Event::from_json(event.to_string()).map_err(|_| "invalid execution event")?;
    buzz_core_pkg::verify_event(&event).map_err(|_| "invalid execution signature")?;
    if event.pubkey != owner.public_key()
        || event.kind.as_u16() as u32 != buzz_core_pkg::kind::KIND_HOST_COMMAND
    {
        return Err("foreign execution command".into());
    }
    let registrations: Vec<_> = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().is_some_and(|s| s == "e"))
        .collect();
    let [registration_tag] = registrations.as_slice() else {
        return Err("invalid execution registration reference".into());
    };
    let tag = registration_tag.as_slice();
    if tag.len() != 2 || !host_execution::hex_id(&tag[1], 64) {
        return Err("invalid execution registration reference".into());
    }
    let registration_id = tag[1].clone();
    // Discovery happens before the final authority read, not after a registration
    // cached during a potentially long-running CLI probe. Diagnostics stay native.
    let catalog = super::discover_acp_providers(app.clone(), Some(true))
        .await
        .unwrap_or_default();
    let registrations = crate::relay::query_private_host_at_with_keys(&state,
        &crate::relay::relay_http_base_url(&relay),
        &[serde_json::json!({"kinds": [50000], "ids": [registration_id], "#p": [expected_owner], "limit": 2})],
        &owner, None).await.map_err(|_| "cannot revalidate execution registration")?;
    let [registration] = registrations.as_slice() else {
        return Err("execution registration is absent or revoked".into());
    };
    if registration.id.to_hex() != registration_id {
        return Err("wrong execution registration returned".into());
    }
    let host = super::hosts::host_keys(&owner)?;
    let request = host_execution::decrypt_command(
        &host,
        registration,
        &event,
        &relay,
        // Authenticate historical bytes for read-only journal recovery. The
        // native transition checks the actual wall clock AFTER immutable replay
        // and BEFORE any new intent/side effect.
        event.created_at.as_secs(),
    )?;
    if event.created_at.as_secs() > Timestamp::now().as_secs().saturating_add(30) {
        return Err("execution command timestamp is in the future".into());
    }
    let compatible_runtime = match &request.action {
        Action::Start { runtime, .. } => catalog.iter().any(|entry| {
            entry.id == *runtime
                && entry.availability == AcpAvailabilityStatus::Available
                && matches!(
                    entry.auth_status,
                    AuthStatus::LoggedIn | AuthStatus::NotApplicable
                )
        }),
        Action::Stop { .. } => true,
    };
    // Bind the identity/relay again after the awaited query. The transition uses
    // these exact checked inputs and rechecks while holding the runtime locks.
    super::hosts::owner_keys(&state, &expected_owner)?;
    crate::relay::assert_expected_relay_scope(
        Some(&relay),
        &crate::relay::relay_api_base_url_with_override(&state),
    )?;
    let registration = registration.clone();
    let result = tokio::task::spawn_blocking(move || {
        let entry = managed_agents::execute_host_operation(
            &app,
            &expected_owner,
            &event.id.to_hex(),
            &request,
            compatible_runtime,
        )?;
        let result = Receipt {
            v: 1,
            command: entry.command_id,
            run: entry.request.run().into(),
            request: entry.request,
            outcome: entry.outcome,
            observed_at: entry.observed_at,
        };
        // Encrypted host-signed observation; no raw process error text escapes.
        let event =
            host_execution::receipt(&host, &registration, &result, Timestamp::now().as_secs())?;
        serde_json::to_value(event).map_err(|_| "cannot serialize execution receipt".into())
    })
    .await
    .map_err(|_| "execution task failed; outcome unknown")?;
    result
}
