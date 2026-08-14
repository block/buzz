use crate::{
    app_state::AppState, managed_agents::RelayAgentInfo, nostr_convert, relay::query_relay,
};

pub(super) async fn list_relay_agents(state: &AppState) -> Result<Vec<RelayAgentInfo>, String> {
    // Self-authored directory profiles and owner-authored managed-agent
    // definitions are both public discovery sources.
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [10100, 30177],
        })],
    )
    .await?;

    // A 30177 event is trusted only after the target agent's kind:0 profile
    // proves that the event author is its NIP-OA owner.
    let target_pubkeys = nostr_convert::relay_agents::managed_agent_target_pubkeys(&events);
    let identity_profiles = if target_pubkeys.is_empty() {
        Vec::new()
    } else {
        query_relay(
            state,
            &[serde_json::json!({
                "kinds": [0],
                "authors": target_pubkeys,
            })],
        )
        .await?
    };

    Ok(nostr_convert::relay_agents::relay_agents_from_events(
        &events,
        &identity_profiles,
    ))
}
