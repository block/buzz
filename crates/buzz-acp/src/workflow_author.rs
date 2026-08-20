//! Resolve the effective author of relay-signed workflow output for the inbound
//! author gate.
//!
//! Workflow `send_message` events are signed by the relay keypair, not the
//! workflow owner. Under default `RespondTo::OwnerOnly`, treating
//! `event.pubkey` as the author drops the event before mention matching runs.
//! When provenance checks pass, attribute the message to the workflow owner
//! carried in the `actor` tag (preferred) or the first `p` tag (legacy path).

use nostr::Event;

/// Returns true when `event` carries an exact `["buzz:workflow", "true"]` tag.
pub(crate) fn has_workflow_marker(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        tag.as_slice().first().map(|s| s.as_str()) == Some("buzz:workflow")
            && tag.as_slice().get(1).map(|s| s.as_str()) == Some("true")
    })
}

/// Extract the workflow owner's hex pubkey from attribution tags.
///
/// Prefers `actor`, then the first `p` tag — matching
/// `buzz-relay::handlers::ingest::effective_message_author`.
pub(crate) fn workflow_owner_pubkey_hex(event: &Event) -> Option<String> {
    if let Some(hex) = event.tags.iter().find_map(|tag| {
        if tag.kind().to_string() == "actor" {
            tag.content().map(str::to_string)
        } else {
            None
        }
    }) {
        if nostr::PublicKey::from_hex(&hex).is_ok() {
            return Some(hex);
        }
    }
    for tag in event.tags.iter() {
        if tag.kind().to_string() != "p" {
            continue;
        }
        if let Some(hex) = tag.content() {
            if nostr::PublicKey::from_hex(hex).is_ok() {
                return Some(hex.to_string());
            }
        }
    }
    None
}

/// When `event` is a relay-signed workflow message, return the attributed owner
/// hex pubkey. Fail closed when the marker is present but provenance is missing
/// or ambiguous.
pub(crate) fn workflow_owner_if_trusted(
    event: &Event,
    relay_self_hex: Option<&str>,
) -> Option<String> {
    if !has_workflow_marker(event) {
        return None;
    }
    let relay_self = relay_self_hex?;
    if event.pubkey.to_hex() != relay_self {
        tracing::debug!(
            event_id = %event.id.to_hex(),
            signer = %event.pubkey.to_hex(),
            "workflow marker present but signer is not the relay identity — fail closed"
        );
        return None;
    }
    let owner = workflow_owner_pubkey_hex(event)?;
    Some(owner)
}

/// Author hex used by `author_allowed` and prompt metadata.
pub(crate) fn inbound_author_hex(event: &Event, relay_self_hex: Option<&str>) -> String {
    workflow_owner_if_trusted(event, relay_self_hex).unwrap_or_else(|| event.pubkey.to_hex())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};

    fn owner_hex() -> String {
        nostr::Keys::generate().public_key().to_hex()
    }

    fn agent_hex() -> String {
        nostr::Keys::generate().public_key().to_hex()
    }

    fn relay_keys() -> nostr::Keys {
        nostr::Keys::generate()
    }

    fn relay_hex(keys: &nostr::Keys) -> String {
        keys.public_key().to_hex()
    }

    fn workflow_message(relay: &nostr::Keys, tags: Vec<Tag>, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(9), content)
            .tags(tags)
            .sign_with_keys(relay)
            .expect("sign workflow test event")
    }

    #[test]
    fn has_workflow_marker_requires_true_value() {
        let relay = relay_keys();
        let event = workflow_message(
            &relay,
            vec![Tag::parse(["buzz:workflow", "true"]).expect("tag")],
            "hi",
        );
        assert!(has_workflow_marker(&event));
        let other = workflow_message(
            &relay,
            vec![Tag::parse(["buzz:workflow", "false"]).expect("tag")],
            "hi",
        );
        assert!(!has_workflow_marker(&other));
    }

    #[test]
    fn workflow_owner_prefers_actor_over_p() {
        let relay = relay_keys();
        let owner = owner_hex();
        let agent = agent_hex();
        let event = workflow_message(
            &relay,
            vec![
                Tag::parse(["p", agent.as_str()]).expect("p"),
                Tag::parse(["actor", owner.as_str()]).expect("actor"),
                Tag::parse(["buzz:workflow", "true"]).expect("wf"),
            ],
            "@agent hi",
        );
        assert_eq!(workflow_owner_pubkey_hex(&event).as_deref(), Some(owner.as_str()));
    }

    #[test]
    fn workflow_owner_falls_back_to_first_p_tag() {
        let relay = relay_keys();
        let owner = owner_hex();
        let agent = agent_hex();
        let event = workflow_message(
            &relay,
            vec![
                Tag::parse(["p", owner.as_str()]).expect("owner"),
                Tag::parse(["p", agent.as_str()]).expect("mention"),
                Tag::parse(["buzz:workflow", "true"]).expect("wf"),
            ],
            "@agent hi",
        );
        assert_eq!(workflow_owner_pubkey_hex(&event).as_deref(), Some(owner.as_str()));
    }

    #[test]
    fn trusted_workflow_resolves_owner_for_author_gate() {
        let relay = relay_keys();
        let relay_hex = relay_hex(&relay);
        let owner = owner_hex();
        let agent = agent_hex();
        let event = workflow_message(
            &relay,
            vec![
                Tag::parse(["p", owner.as_str()]).expect("owner"),
                Tag::parse(["p", agent.as_str()]).expect("mention"),
                Tag::parse(["buzz:workflow", "true"]).expect("wf"),
            ],
            "@agent hi",
        );
        assert_eq!(inbound_author_hex(&event, Some(&relay_hex)), owner);
    }

    #[test]
    fn untrusted_signer_fails_closed() {
        let relay_hex = relay_hex(&relay_keys());
        let owner = owner_hex();
        let stranger = nostr::Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "forged")
            .tags([
                Tag::parse(["p", owner.as_str()]).expect("owner"),
                Tag::parse(["buzz:workflow", "true"]).expect("wf"),
            ])
            .sign_with_keys(&stranger)
            .expect("sign forged event");
        assert_eq!(
            inbound_author_hex(&event, Some(&relay_hex)),
            stranger.public_key().to_hex()
        );
    }

    #[test]
    fn missing_relay_identity_fails_closed() {
        let relay = relay_keys();
        let relay_hex = relay_hex(&relay);
        let owner = owner_hex();
        let event = workflow_message(
            &relay,
            vec![
                Tag::parse(["p", owner.as_str()]).expect("owner"),
                Tag::parse(["buzz:workflow", "true"]).expect("wf"),
            ],
            "hi",
        );
        assert_eq!(
            inbound_author_hex(&event, None),
            relay_hex,
            "without relay self, fall back to event signer"
        );
    }
}
