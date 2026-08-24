//! Native companion-window lifecycle for an agent activity feed.

use sha2::{Digest, Sha256};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use url::form_urlencoded;
use uuid::Uuid;

const PUBKEY_HEX_LENGTH: usize = 64;

fn normalized_pubkey(pubkey: &str) -> Result<String, String> {
    let normalized = pubkey.trim().to_ascii_lowercase();
    if normalized.len() != PUBKEY_HEX_LENGTH
        || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("agent pubkey must be 64 hexadecimal characters".to_string());
    }
    Ok(normalized)
}

fn normalized_community_id(community_id: &str) -> Result<String, String> {
    let normalized = community_id.trim();
    if normalized.is_empty() {
        return Err("community id must not be empty".to_string());
    }
    Ok(normalized.to_string())
}

fn window_label(community_id: &str, channel_id: &Uuid, pubkey: &str) -> String {
    let community_scope = hex::encode(Sha256::digest(community_id.as_bytes()));
    format!("agent-activity-{community_scope}-{pubkey}-{channel_id}")
}

fn activity_route(community_id: &str, channel_id: &Uuid, pubkey: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("community", community_id)
        .append_pair("agentSession", pubkey)
        .append_pair("agentSessionChannel", &channel_id.to_string())
        .finish();
    format!("index.html#/channels/{channel_id}?{query}")
}

/// Open an agent's channel-scoped activity feed without replacing the main
/// window's thread panel. Each agent/channel pair owns one reusable window.
#[tauri::command]
pub fn open_agent_activity_window(
    app: tauri::AppHandle,
    community_id: String,
    channel_id: String,
    pubkey: String,
) -> Result<bool, String> {
    let community_id = normalized_community_id(&community_id)?;
    let channel_id =
        Uuid::parse_str(channel_id.trim()).map_err(|_| "channel id must be a UUID".to_string())?;
    let pubkey = normalized_pubkey(&pubkey)?;
    let label = window_label(&community_id, &channel_id, &pubkey);

    if let Some(window) = app.get_webview_window(&label) {
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(true);
    }

    let route = activity_route(&community_id, &channel_id, &pubkey);
    WebviewWindowBuilder::new(&app, label, WebviewUrl::App(route.into()))
        .title("Agent activity")
        .inner_size(560.0, 760.0)
        .min_inner_size(420.0, 520.0)
        .build()
        .map_err(|error| error.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{activity_route, normalized_community_id, normalized_pubkey, window_label};
    use uuid::Uuid;

    #[test]
    fn normalizes_valid_pubkeys() {
        let uppercase = "AB".repeat(32);
        assert_eq!(normalized_pubkey(&uppercase), Ok("ab".repeat(32)));
    }

    #[test]
    fn labels_distinguish_pubkeys_with_the_same_prefix() {
        let channel_id = Uuid::nil();
        let first = format!("{}{}", "ab".repeat(6), "cd".repeat(26));
        let second = format!("{}{}", "ab".repeat(6), "ef".repeat(26));

        assert_ne!(
            window_label("community-a", &channel_id, &first),
            window_label("community-a", &channel_id, &second)
        );
    }

    #[test]
    fn labels_distinguish_communities() {
        let channel_id = Uuid::nil();
        let pubkey = "ab".repeat(32);

        assert_ne!(
            window_label("community-a", &channel_id, &pubkey),
            window_label("community-b", &channel_id, &pubkey)
        );
    }

    #[test]
    fn route_carries_immutable_community_scope() {
        let channel_id = Uuid::nil();
        let pubkey = "ab".repeat(32);

        assert_eq!(
            activity_route("community & one", &channel_id, &pubkey),
            format!(
                "index.html#/channels/{channel_id}?community=community+%26+one&agentSession={pubkey}&agentSessionChannel={channel_id}"
            )
        );
    }

    #[test]
    fn rejects_empty_community_ids() {
        assert!(normalized_community_id("  ").is_err());
    }

    #[test]
    fn rejects_invalid_pubkeys() {
        assert!(normalized_pubkey("abc").is_err());
        assert!(normalized_pubkey(&"zz".repeat(32)).is_err());
    }
}
