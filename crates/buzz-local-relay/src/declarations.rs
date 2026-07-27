//! Journal-derived operator configuration.
//!
//! Evaluates owner-signed sync declaration heads (kind 30700) into the
//! adapter's operating configuration, per the runtime evaluation policy in
//! `specs/architecture/sovereign-sync-agreement-v0.1-draft.md`: only heads
//! authored by the node's owner AND `n`-tagged with this node's label
//! govern; a domain with any such head is governed wholesale by the journal
//! (file config ignored); only `status: "active"` heads confer trust;
//! malformed heads are a startup error, never silently dropped trust.

use buzz_core::kind::KIND_SYNC_DECLARATION;
use buzz_core::replication::ReplicationSourceId;
use nostr::{Alphabet, Event, Filter, Kind, PublicKey, SingleLetterTag, TagKind};
use thiserror::Error;

use crate::identity::RelayPeerTrust;
use crate::{EventStore, QueryError};

/// A declaration head could not be evaluated into configuration.
#[derive(Debug, Error)]
pub enum DeclarationError {
    /// The journal could not be queried.
    #[error("declaration query failed: {0}")]
    Query(#[from] QueryError),
    /// An owner-signed head exists but cannot be interpreted; trust is
    /// ambiguous, so the adapter must refuse to start rather than guess.
    #[error("malformed declaration {d_tag:?} (event {event_id}): {reason}")]
    Malformed {
        /// The head's `d` tag.
        d_tag: String,
        /// The offending event ID.
        event_id: String,
        /// Why evaluation failed.
        reason: String,
    },
}

/// The admit domain as evaluated from the journal, scoped to one node.
///
/// Only owner-signed heads whose `n` tag equals `node_label` govern: journals
/// replicate whole, so one owner's declarations for different nodes coexist
/// in every copy, and each node must evaluate only its own. `None` means the
/// owner has published no `admit/*` head for this node — the domain is
/// unclaimed and bootstrap file config may govern. `Some(entries)` means the
/// journal governs the domain wholesale; `entries` holds only active heads
/// and may be empty (all heads revoked — trust is empty, not a fallback).
pub async fn admit_domain_from_journal(
    store: &EventStore,
    owner: &PublicKey,
    node_label: &str,
) -> Result<Option<Vec<(ReplicationSourceId, RelayPeerTrust)>>, DeclarationError> {
    let filter = Filter::new()
        .authors([*owner])
        .kind(Kind::Custom(KIND_SYNC_DECLARATION as u16));
    let heads = store.query(&[filter]).await?;

    let mut claimed = false;
    let mut entries = Vec::new();
    for event in &heads {
        let Some(d_tag) = d_tag(event) else { continue };
        let Some(source) = d_tag.strip_prefix("admit/") else {
            continue;
        };
        if n_tag(event) != Some(node_label) {
            continue;
        }
        claimed = true;
        let content: serde_json::Value =
            serde_json::from_str(&event.content).map_err(|error| malformed(event, &error))?;
        let status = content["status"].as_str().unwrap_or("active");
        if status != "active" {
            continue;
        }
        let principal = content["principal"]
            .as_str()
            .ok_or_else(|| malformed(event, &"active admit head requires a string principal"))?;
        let keys = p_tags(event)
            .map(|hex| {
                let pubkey = PublicKey::from_hex(hex)
                    .map_err(|_| malformed(event, &format!("invalid p-tag pubkey {hex:?}")))?;
                Ok((pubkey, format!("{principal}#nostr-key")))
            })
            .collect::<Result<Vec<_>, DeclarationError>>()?;
        if keys.is_empty() {
            return Err(malformed(
                event,
                &"active admit head requires at least one p-tag verification key",
            ));
        }
        let source_id = ReplicationSourceId::new(source.to_string());
        entries.push((
            source_id.clone(),
            RelayPeerTrust::new(source_id, principal, keys),
        ));
    }

    Ok(claimed.then_some(entries))
}

fn d_tag(event: &Event) -> Option<&str> {
    named_tag(event, "d")
}

fn n_tag(event: &Event) -> Option<&str> {
    named_tag(event, "n")
}

fn named_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some(name))
            .then(|| values.get(1).map(String::as_str))
            .flatten()
    })
}

fn p_tags(event: &Event) -> impl Iterator<Item = &str> {
    let single_p = TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::P));
    event.tags.iter().filter_map(move |tag| {
        let values = tag.as_slice();
        (tag.kind() == single_p)
            .then(|| values.get(1).map(String::as_str))
            .flatten()
    })
}

fn malformed(event: &Event, reason: &dyn std::fmt::Display) -> DeclarationError {
    DeclarationError::Malformed {
        d_tag: d_tag(event).unwrap_or("<missing>").to_string(),
        event_id: event.id.to_hex(),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StorageMode;
    use nostr::{EventBuilder, Keys, Tag};

    async fn store_with(events: Vec<Event>) -> EventStore {
        let store = EventStore::open(StorageMode::Ephemeral).await.unwrap();
        for event in events {
            let result = store.accept(event).await.unwrap();
            assert!(result.accepted, "test event rejected: {:?}", result.message);
        }
        store
    }

    const NODE: &str = "ted-laptop";

    fn admit_event(keys: &Keys, source: &str, content: serde_json::Value, p: &[&Keys]) -> Event {
        admit_event_for_node(keys, NODE, source, content, p)
    }

    fn admit_event_for_node(
        keys: &Keys,
        node: &str,
        source: &str,
        content: serde_json::Value,
        p: &[&Keys],
    ) -> Event {
        let mut tags = vec![
            Tag::identifier(format!("admit/{source}")),
            Tag::custom(TagKind::custom("n"), [node]),
        ];
        for peer in p {
            tags.push(Tag::public_key(peer.public_key()));
        }
        EventBuilder::new(
            Kind::Custom(KIND_SYNC_DECLARATION as u16),
            content.to_string(),
        )
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
    }

    #[tokio::test]
    async fn unclaimed_domain_returns_none() {
        let owner = Keys::generate();
        let store = store_with(vec![]).await;
        let result = admit_domain_from_journal(&store, &owner.public_key(), NODE)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn active_head_confers_trust() {
        let owner = Keys::generate();
        let peer = Keys::generate();
        let event = admit_event(
            &owner,
            "node-c/work",
            serde_json::json!({"status": "active", "principal": "did:buzz:node-c"}),
            &[&peer],
        );
        let store = store_with(vec![event]).await;
        let entries = admit_domain_from_journal(&store, &owner.public_key(), NODE)
            .await
            .unwrap()
            .expect("domain claimed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, ReplicationSourceId::new("node-c/work"));
    }

    #[tokio::test]
    async fn revoked_head_claims_domain_but_confers_nothing() {
        let owner = Keys::generate();
        let peer = Keys::generate();
        let event = admit_event(
            &owner,
            "node-b/demo",
            serde_json::json!({"status": "revoked", "principal": "did:buzz:node-b"}),
            &[&peer],
        );
        let store = store_with(vec![event]).await;
        let entries = admit_domain_from_journal(&store, &owner.public_key(), NODE)
            .await
            .unwrap()
            .expect("revoked head still claims the domain");
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn foreign_heads_do_not_govern() {
        let owner = Keys::generate();
        let stranger = Keys::generate();
        let peer = Keys::generate();
        let event = admit_event(
            &stranger,
            "intruder/stream",
            serde_json::json!({"status": "active", "principal": "did:buzz:intruder"}),
            &[&peer],
        );
        let store = store_with(vec![event]).await;
        let result = admit_domain_from_journal(&store, &owner.public_key(), NODE)
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "foreign declarations must not claim the domain"
        );
    }

    #[tokio::test]
    async fn active_head_without_keys_is_malformed() {
        let owner = Keys::generate();
        let event = admit_event(
            &owner,
            "node-c/work",
            serde_json::json!({"status": "active", "principal": "did:buzz:node-c"}),
            &[],
        );
        let store = store_with(vec![event]).await;
        let error = admit_domain_from_journal(&store, &owner.public_key(), NODE)
            .await
            .unwrap_err();
        assert!(matches!(error, DeclarationError::Malformed { .. }));
    }

    #[tokio::test]
    async fn replacement_supersedes_earlier_head() {
        let owner = Keys::generate();
        let peer_old = Keys::generate();
        let peer_new = Keys::generate();
        let old = admit_event(
            &owner,
            "node-c/work",
            serde_json::json!({"status": "active", "principal": "did:buzz:node-c"}),
            &[&peer_old],
        );
        // Addressable replacement is by (author, kind, d); a strictly newer
        // created_at supersedes.
        let new = EventBuilder::new(
            Kind::Custom(KIND_SYNC_DECLARATION as u16),
            serde_json::json!({"status": "active", "principal": "did:buzz:node-c"}).to_string(),
        )
        .tags([
            Tag::identifier("admit/node-c/work"),
            Tag::custom(TagKind::custom("n"), [NODE]),
            Tag::public_key(peer_new.public_key()),
        ])
        .custom_created_at((old.created_at.as_secs() + 10).into())
        .sign_with_keys(&owner)
        .unwrap();
        let store = store_with(vec![old, new.clone()]).await;
        let entries = admit_domain_from_journal(&store, &owner.public_key(), NODE)
            .await
            .unwrap()
            .expect("domain claimed");
        assert_eq!(entries.len(), 1, "only the head should govern");
    }

    #[tokio::test]
    async fn heads_for_other_nodes_do_not_govern() {
        let owner = Keys::generate();
        let peer = Keys::generate();
        let event = admit_event_for_node(
            &owner,
            "cf-rendezvous",
            "ted-laptop/sovereign",
            serde_json::json!({"status": "active", "principal": "did:buzz:ted-laptop"}),
            &[&peer],
        );
        let store = store_with(vec![event]).await;
        let result = admit_domain_from_journal(&store, &owner.public_key(), NODE)
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "a replicated journal carries other nodes' declarations; they must not claim this node's domain"
        );
    }
}
