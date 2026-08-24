use crate::{
    app_state::AppState, managed_agents::RelayAgentInfo, nostr_convert, relay::query_relay,
};

pub(super) async fn list_relay_agents(state: &AppState) -> Result<Vec<RelayAgentInfo>, String> {
    // Self-authored directory profiles and owner-authored managed-agent
    // definitions are both public discovery sources.
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [5, 10100, 30177, 13535],
        })],
    )
    .await?;

    // A 30177 event is trusted only after the target agent's kind:0 profile
    // proves that the event author is its NIP-OA owner.
    let mut target_pubkeys = nostr_convert::relay_agents::managed_agent_target_pubkeys(&events);
    for event in events.iter().filter(|event| event.kind.as_u16() == 5) {
        for tag in event.tags.iter() {
            let values = tag.as_slice();
            if values.first().map(String::as_str) != Some("a") { continue; }
            if let Some(agent) = values.get(1).and_then(|coordinate| coordinate.split(':').nth(2)) {
                if agent.len() == 64 { target_pubkeys.push(agent.to_ascii_lowercase()); }
            }
        }
    }
    for event in events
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == 13535)
    {
        for tag in event.tags.iter() {
            let values = tag.as_slice();
            if values.first().map(String::as_str) == Some("p") {
                if let Some(agent) = values.get(1).filter(|value| value.len() == 64) {
                    target_pubkeys.push(agent.to_ascii_lowercase());
                }
            }
        }
    }
    target_pubkeys.sort();
    target_pubkeys.dedup();
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
