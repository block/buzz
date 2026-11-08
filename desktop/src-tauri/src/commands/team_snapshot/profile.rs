use crate::app_state::AppState;

/// Publish an imported member only after its own signer holds the selected
/// target-local avatar bytes. The caller captures owner and relay before I/O;
/// failures remain in the import result's profile_sync_error for the user.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn publish_imported_team_member_profile(
    state: &AppState,
    relay: &str,
    owner: &nostr::Keys,
    agent: &nostr::Keys,
    name: &str,
    avatar: Option<&str>,
    about: Option<&str>,
    auth_tag: Option<&str>,
) -> Option<String> {
    crate::publish_created_agent_profile(state, relay, owner, agent, name, avatar, about, auth_tag)
        .await
}
