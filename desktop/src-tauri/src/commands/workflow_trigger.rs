//! A manual run is prepared once, then submitted/retried as the same signed event.
//! The renderer owns the operation (including ambiguous failures); native code
//! never silently rebuilds a submitted trigger. No restart recovery is promised.
use nostr::Event;
use serde::Deserialize;
use tauri::State;

use super::workflows::{trigger_wire_from_message, WorkflowTriggerWire};
use crate::{app_state::AppState, events, relay};

fn trigger_scope(
    state: &AppState,
    expected_relay_url: &str,
    expected_signer_pubkey: &str,
) -> Result<(String, nostr::Keys), String> {
    if expected_relay_url.trim().is_empty() || expected_signer_pubkey.trim().is_empty() {
        return Err("workflow trigger requires a community and identity".to_string());
    }
    let base = relay::relay_api_base_url_with_override(state);
    let keys = state.signing_keys()?;
    relay::assert_expected_relay_scope(Some(expected_relay_url), &base)?;
    relay::assert_expected_signer(Some(expected_signer_pubkey), &keys.public_key().to_hex())?;
    Ok((base, keys))
}

/// Resolve and sign without publishing. Losing this response cannot create a run.
#[tauri::command]
pub async fn prepare_workflow_trigger(
    workflow_id: String,
    expected_relay_url: String,
    expected_signer_pubkey: String,
    state: State<'_, AppState>,
) -> Result<Event, String> {
    prepare_trigger(
        &workflow_id,
        &expected_relay_url,
        &expected_signer_pubkey,
        &state,
    )
    .await
}

async fn prepare_trigger(
    workflow_id: &str,
    expected_relay_url: &str,
    expected_signer_pubkey: &str,
    state: &AppState,
) -> Result<Event, String> {
    #[derive(Deserialize)]
    struct Revision {
        id: String,
    }
    let id = uuid::Uuid::parse_str(workflow_id).map_err(|_| "invalid workflow id".to_string())?;
    let (base, keys) = trigger_scope(state, expected_relay_url, expected_signer_pubkey)?;
    // Neither the GET authentication nor the event signer re-reads active scope
    // after the await. A concurrent switch cannot combine tenant A with key B.
    let revision: Revision = relay::get_relay_json_at_with_keys(
        state,
        &format!("/workflows/{id}/revision"),
        &base,
        &keys,
    )
    .await?;
    events::build_workflow_trigger(&id.to_string(), &revision.id)?
        .sign_with_keys(&keys)
        .map_err(|error| format!("failed to sign trigger: {error}"))
}

/// Submit the caller-retained event verbatim, including its original signature.
#[tauri::command]
pub async fn trigger_workflow(
    workflow_id: String,
    event: Event,
    expected_relay_url: String,
    expected_signer_pubkey: String,
    state: State<'_, AppState>,
) -> Result<WorkflowTriggerWire, String> {
    submit_trigger(
        workflow_id,
        &event,
        &expected_relay_url,
        &expected_signer_pubkey,
        &state,
    )
    .await
}

async fn submit_trigger(
    workflow_id: String,
    event: &Event,
    expected_relay_url: &str,
    expected_signer_pubkey: &str,
    state: &AppState,
) -> Result<WorkflowTriggerWire, String> {
    let (base, keys) = trigger_scope(state, expected_relay_url, expected_signer_pubkey)?;
    if event.kind != nostr::Kind::Custom(46020)
        || !event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["d", workflow_id.as_str()])
    {
        return Err("signed trigger does not match the workflow".to_string());
    }
    event
        .verify()
        .map_err(|error| format!("invalid signed trigger: {error}"))?;
    let result = relay::submit_signed_event_at_with_keys(event, state, &base, &keys).await?;
    if result.event_id != event.id.to_hex() {
        return Err("relay acknowledged a different trigger event".to_string());
    }
    trigger_wire_from_message(workflow_id, &result.message)
}

#[cfg(test)]
#[path = "workflow_trigger_tests.rs"]
mod tests;
