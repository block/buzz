use std::collections::HashSet;

use tauri::State;

use crate::{
    app_state::AppState, managed_agents::RelayAgentInfo, nostr_convert, relay::query_relay,
};

#[tauri::command]
pub async fn list_relay_agents(state: State<'_, AppState>) -> Result<Vec<RelayAgentInfo>, String> {
    let owner_pubkey = state
        .keys
        .lock()
        .map_err(|error| error.to_string())?
        .public_key()
        .to_hex();
    let events = query_relay(
        &state,
        &[
            serde_json::json!({ "kinds": [10100] }),
            serde_json::json!({ "kinds": [0], "#auth": [&owner_pubkey] }),
        ],
    )
    .await?;

    let parse_agents = |value: serde_json::Value| {
        serde_json::from_value::<Vec<RelayAgentInfo>>(
            value
                .get("agents")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|error| format!("agent parse failed: {error}"))
    };
    let mut agents = parse_agents(nostr_convert::agents_from_events(&events))?;
    let owned_profile_agents = parse_agents(nostr_convert::owned_agent_profiles_from_events(
        &events,
        &owner_pubkey,
    ))?;
    let mut known_pubkeys: HashSet<String> = agents
        .iter()
        .map(|agent| agent.pubkey.to_lowercase())
        .collect();
    agents.extend(
        owned_profile_agents
            .into_iter()
            .filter(|agent| known_pubkeys.insert(agent.pubkey.to_lowercase())),
    );
    Ok(agents)
}
