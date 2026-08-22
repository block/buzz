//! Publish the agent's relay-agent directory entry (kind 10100).
//!
//! Buzz Desktop decides whether to offer an agent in `@` autocomplete from the
//! union of two sets (`desktop/src/features/agents/knownAgentPubkeys.ts`):
//!
//! 1. **managed agents** — agents the desktop itself holds a key for and runs.
//!    An agent created in the app is in this set immediately and needs no relay
//!    event at all.
//! 2. **relay agents** — built from kind 10100 events, filtered by
//!    `relayAgentCanRespondInChannel`: the entry's `channel_ids` must contain
//!    the channel being typed in, and its `respond_to`/`respond_to_allowlist`
//!    must admit the person typing.
//!
//! A harness-run agent can never be in set (1) — the desktop neither holds its
//! key nor runs it — so set (2) is its only route to being mentionable. Until
//! now nothing in the harness published that entry, which left every
//! self-hosted agent invisible in autocomplete however healthy it was, with no
//! error anywhere to explain it.
//!
//! The entry is derived from state the harness already has: the channels it
//! discovered and subscribed to, and its own author gate. It carries no
//! secrets. Kind 10100 is replaceable, so republishing simply supersedes.

use std::collections::{HashMap, HashSet};

use nostr::{EventBuilder, Kind};
use uuid::Uuid;

use crate::config::RespondTo;
use crate::relay::{self, ChannelInfo, RelayError, RelayEventPublisher};

/// Relay-agent directory entry. Not currently covered by a NIP; the shape is
/// what `desktop/src-tauri/src/nostr_convert.rs::agents_from_events` parses
/// into `RelayAgentInfo`.
pub const KIND_AGENT_DIRECTORY: u16 = 10100;

/// Build the entry body.
///
/// `channels`/`channel_ids` are positionally aligned, matching the desktop's
/// `RelayAgentInfo`. Absent fields there default rather than error, but
/// `channel_ids` defaulting to empty means "eligible in no channel", so it is
/// always emitted explicitly.
pub fn build_entry(
    display_name: Option<&str>,
    channels: &HashMap<Uuid, ChannelInfo>,
    subscribed: &HashSet<Uuid>,
    respond_to: &RespondTo,
    allowlist: &HashSet<String>,
) -> serde_json::Value {
    // Only channels the harness is actually subscribed to. Advertising one it
    // does not listen on would offer a mention that silently goes nowhere.
    let mut ids: Vec<&Uuid> = channels
        .keys()
        .filter(|id| subscribed.contains(id))
        .collect();
    // Stable order so the published content only changes when the set does.
    ids.sort();

    let names: Vec<&str> = ids
        .iter()
        .map(|id| {
            channels
                .get(id)
                .map(|c| c.name.as_str())
                .unwrap_or("channel")
        })
        .collect();
    let id_strings: Vec<String> = ids.iter().map(|id| id.to_string()).collect();

    let mut entry = serde_json::json!({
        "agent_type": "agent",
        "channels": names,
        "channel_ids": id_strings,
        "capabilities": [],
        "status": "online",
        "respond_to": respond_to.to_string(),
    });

    // Mirrors the inbound author gate, so the people who may prompt the agent
    // are exactly the people whose client offers it.
    if *respond_to == RespondTo::Allowlist {
        let mut allow: Vec<&str> = allowlist.iter().map(String::as_str).collect();
        allow.sort_unstable();
        entry["respond_to_allowlist"] = serde_json::json!(allow);
    }

    // Omitted rather than guessed when unset: the desktop falls back to the
    // agent's kind 0 display name, and failing that its npub.
    if let Some(name) = display_name.filter(|n| !n.trim().is_empty()) {
        entry["name"] = serde_json::json!(name.trim());
    }

    entry
}

/// Sign and publish the entry.
///
/// Best-effort by design: a failure here costs discoverability, never the
/// agent's ability to do its job, so callers log and carry on.
pub async fn publish(
    publisher: &RelayEventPublisher,
    keys: &nostr::Keys,
    auth_tag: Option<&nostr::Tag>,
    entry: &serde_json::Value,
) -> Result<(), RelayError> {
    let mut builder = EventBuilder::new(Kind::Custom(KIND_AGENT_DIRECTORY), entry.to_string());
    if let Some(tag) = auth_tag {
        // Carry the NIP-OA attestation, as the agent's other events do, so
        // clients can show the entry as owned rather than unattributed.
        builder = builder.tags([tag.clone()]);
    }
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| relay::RelayError::Http(format!("directory sign error: {e}")))?;
    publisher.publish_event(event).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str) -> ChannelInfo {
        ChannelInfo {
            name: name.to_string(),
            channel_type: "stream".to_string(),
            description: None,
        }
    }

    fn fixture() -> (HashMap<Uuid, ChannelInfo>, Uuid, Uuid) {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let mut m = HashMap::new();
        m.insert(a, info("general"));
        m.insert(b, info("ops"));
        (m, a, b)
    }

    #[test]
    fn advertises_only_subscribed_channels() {
        let (channels, a, b) = fixture();
        let subscribed: HashSet<Uuid> = [a].into_iter().collect();
        let e = build_entry(
            None,
            &channels,
            &subscribed,
            &RespondTo::Anyone,
            &HashSet::new(),
        );
        assert_eq!(e["channel_ids"], serde_json::json!([a.to_string()]));
        assert_eq!(e["channels"], serde_json::json!(["general"]));
        assert!(!e["channel_ids"].to_string().contains(&b.to_string()));
    }

    #[test]
    fn names_and_ids_stay_aligned() {
        let (channels, a, b) = fixture();
        let subscribed: HashSet<Uuid> = [a, b].into_iter().collect();
        let e = build_entry(
            None,
            &channels,
            &subscribed,
            &RespondTo::Anyone,
            &HashSet::new(),
        );
        let ids = e["channel_ids"].as_array().unwrap();
        let names = e["channels"].as_array().unwrap();
        assert_eq!(ids.len(), names.len());
        for (i, id) in ids.iter().enumerate() {
            let uuid: Uuid = id.as_str().unwrap().parse().unwrap();
            assert_eq!(names[i].as_str().unwrap(), channels[&uuid].name);
        }
    }

    #[test]
    fn allowlist_is_published_only_in_allowlist_mode() {
        let (channels, a, _) = fixture();
        let subscribed: HashSet<Uuid> = [a].into_iter().collect();
        let allow: HashSet<String> = ["bb".repeat(32)].into_iter().collect();

        let gated = build_entry(None, &channels, &subscribed, &RespondTo::Allowlist, &allow);
        assert_eq!(gated["respond_to"], "allowlist");
        assert_eq!(
            gated["respond_to_allowlist"],
            serde_json::json!(["bb".repeat(32)])
        );

        let open = build_entry(None, &channels, &subscribed, &RespondTo::Anyone, &allow);
        assert_eq!(open["respond_to"], "anyone");
        assert!(open.get("respond_to_allowlist").is_none());
    }

    #[test]
    fn name_is_omitted_when_unset_or_blank() {
        let (channels, a, _) = fixture();
        let subscribed: HashSet<Uuid> = [a].into_iter().collect();
        for name in [None, Some(""), Some("   ")] {
            let e = build_entry(
                name,
                &channels,
                &subscribed,
                &RespondTo::Anyone,
                &HashSet::new(),
            );
            assert!(
                e.get("name").is_none(),
                "blank name must be omitted, not published"
            );
        }
        let e = build_entry(
            Some("  scout  "),
            &channels,
            &subscribed,
            &RespondTo::Anyone,
            &HashSet::new(),
        );
        assert_eq!(e["name"], "scout");
    }

    #[test]
    fn output_is_stable_across_builds() {
        let (channels, a, b) = fixture();
        let subscribed: HashSet<Uuid> = [b, a].into_iter().collect();
        let one = build_entry(
            Some("x"),
            &channels,
            &subscribed,
            &RespondTo::Anyone,
            &HashSet::new(),
        );
        let two = build_entry(
            Some("x"),
            &channels,
            &subscribed,
            &RespondTo::Anyone,
            &HashSet::new(),
        );
        assert_eq!(one.to_string(), two.to_string());
    }
}
