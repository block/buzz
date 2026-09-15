//! Preserve the asker's authority when an interaction is projected as text.

use buzz_core::{interaction::single_tag, kind::KIND_STREAM_MESSAGE};
use nostr::{Event, EventId, PublicKey};

/// Resolve the principal before applying the existing owner/allowlist policy.
pub(super) fn effective_prompt_author(
    event: &Event,
    relay_self: Option<&str>,
    agent_pubkey_hex: &str,
) -> String {
    super::verified_workflow_owner(event, relay_self, agent_pubkey_hex)
        .or_else(|| verified_interaction_author(event, relay_self))
        .unwrap_or_else(|| event.pubkey.to_hex())
}

fn verified_interaction_author(event: &Event, relay_self: Option<&str>) -> Option<String> {
    if event.kind.as_u16() as u32 != KIND_STREAM_MESSAGE
        || event.pubkey != PublicKey::from_hex(relay_self?).ok()?
        || event.verify().is_err()
    {
        return None;
    }
    // Only the relay's exact projection contract conveys authority. Ordinary
    // client-signed messages cannot borrow the actor tag's identity.
    let prompt = single_tag(event, "interaction").ok()??;
    if EventId::from_hex(prompt).ok()?.to_hex() != prompt {
        return None;
    }
    let actor = single_tag(event, "actor").ok()??;
    let canonical = PublicKey::from_hex(actor).ok()?.to_hex();
    (canonical == actor).then_some(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::RespondTo, pool, relay, InboundAuthorGate, OwnerCache};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::collections::{HashMap, HashSet};

    fn projection(keys: &Keys, actor: &str, extra: Vec<Tag>) -> Event {
        let mut tags = vec![
            Tag::parse(["interaction", &"ab".repeat(32)]).unwrap(),
            Tag::parse(["actor", actor]).unwrap(),
        ];
        tags.extend(extra);
        EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "Approve?")
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn interaction_attribution_requires_verified_canonical_relay_provenance() {
        let relay = Keys::generate();
        let attacker = Keys::generate();
        let actor = Keys::generate().public_key().to_hex();
        let relay_hex = relay.public_key().to_hex();
        let event = projection(&relay, &actor, vec![]);
        assert_eq!(
            verified_interaction_author(&event, Some(&relay_hex)),
            Some(actor.clone())
        );
        assert_eq!(verified_interaction_author(&event, None), None);
        assert_eq!(
            verified_interaction_author(&projection(&attacker, &actor, vec![]), Some(&relay_hex)),
            None
        );
        for extra in [
            Tag::parse(["actor", &actor]).unwrap(),
            Tag::parse(["interaction", &"ab".repeat(32)]).unwrap(),
        ] {
            assert_eq!(
                verified_interaction_author(
                    &projection(&relay, &actor, vec![extra]),
                    Some(&relay_hex)
                ),
                None
            );
        }
        let mut tampered = event.clone();
        tampered.content.push('!');
        assert_eq!(
            verified_interaction_author(&tampered, Some(&relay_hex)),
            None
        );
        let wrong_kind = EventBuilder::new(Kind::Custom(40010), event.content.clone())
            .tags(event.tags.clone())
            .sign_with_keys(&relay)
            .unwrap();
        assert_eq!(
            verified_interaction_author(&wrong_kind, Some(&relay_hex)),
            None
        );
        for (tag_name, replacement) in [
            ("actor", "not-a-key"),
            ("interaction", "bad-id"),
            ("interaction", &"AB".repeat(32)),
        ] {
            let tags = event
                .tags
                .iter()
                .map(|tag| {
                    if tag.as_slice()[0] == tag_name {
                        Tag::parse([tag_name, replacement]).unwrap()
                    } else {
                        tag.clone()
                    }
                })
                .collect::<Vec<_>>();
            let malformed = EventBuilder::new(event.kind, "Approve?")
                .tags(tags)
                .sign_with_keys(&relay)
                .unwrap();
            assert_eq!(
                verified_interaction_author(&malformed, Some(&relay_hex)),
                None
            );
        }
    }

    #[tokio::test]
    async fn connected_interaction_gate_applies_policy_to_the_asker_not_the_relay() {
        let relay_keys = Keys::generate();
        let relay_hex = relay_keys.public_key().to_hex();
        let owner = Keys::generate().public_key().to_hex();
        let outsider = Keys::generate().public_key().to_hex();
        let agent = Keys::generate().public_key().to_hex();
        let (rest, server) =
            crate::author_gate_tests::nip11_server(serde_json::json!({"self":relay_hex})).await;
        let mut gate = InboundAuthorGate::connect(&rest, &agent, "interaction-test").await;
        let cache = OwnerCache::new(Some(owner.clone()));
        cache.cache_sibling(outsider.clone(), false);
        cache.cache_sibling(relay_hex.clone(), false);
        let channel_id = uuid::Uuid::new_v4();
        let channels = pool::ChannelInfoResolver::new(
            HashMap::from([(
                channel_id,
                relay::ChannelInfo {
                    name: "interaction".into(),
                    channel_type: "stream".into(),
                    description: None,
                },
            )]),
            rest.clone(),
        );
        for (actor, policy, allowlist, expected) in [
            (
                &outsider,
                RespondTo::Allowlist,
                HashSet::from([relay_hex.clone()]),
                false,
            ),
            (
                &outsider,
                RespondTo::Allowlist,
                HashSet::from([outsider.clone()]),
                true,
            ),
            (&owner, RespondTo::OwnerOnly, HashSet::new(), true),
            (&outsider, RespondTo::OwnerOnly, HashSet::new(), false),
        ] {
            let event = relay::BuzzEvent {
                connection_generation: 0,
                channel_id,
                event: projection(&relay_keys, actor, vec![Tag::parse(["p", &agent]).unwrap()]),
            };
            let decision = gate
                .evaluate_listener_event(&event, &policy, &allowlist, &cache, &channels, &rest)
                .await;
            assert_eq!(decision.effective_author, *actor);
            assert_eq!(
                decision.allowed, expected,
                "policy must follow the original asker"
            );
        }
        server.abort();
    }
}
