//! Data projection only: every match decision belongs to the generated operation.
use super::input::{EventInput, EventTag, FilterInput, TagQuery};
use crate::event::StoredEvent;
use nostr::Filter;

/// Prepare immutable selectors once when a subscription is registered.
pub fn prepare_filters(filters: &[Filter]) -> Vec<FilterInput> {
    filters
        .iter()
        .map(|f| FilterInput {
            ids: f
                .ids
                .as_ref()
                .map(|ids| ids.iter().map(|id| id.to_bytes()).collect()),
            authors: f
                .authors
                .as_ref()
                .map(|authors| authors.iter().map(|author| author.to_bytes()).collect()),
            kinds: f
                .kinds
                .as_ref()
                .map(|kinds| kinds.iter().map(|kind| kind.as_u16()).collect()),
            since: f.since.map(|time| time.as_secs()),
            until: f.until.map(|time| time.as_secs()),
            tags: f
                .generic_tags
                .iter()
                .map(|(key, values)| TagQuery {
                    key: key.to_string().into_bytes(),
                    values: values
                        .iter()
                        .map(|value| value.as_bytes().to_vec())
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

/// Project the event once, after a fanout candidate passes its scope guards.
pub fn prepare_event(event: &StoredEvent) -> EventInput {
    EventInput {
        id: event.event.id.to_bytes(),
        author: event.event.pubkey.to_bytes(),
        kind: event.event.kind.as_u16(),
        created_at: event.event.created_at.as_secs(),
        tags: event
            .event
            .tags
            .iter()
            .map(|tag| EventTag {
                key: tag.kind().to_string().into_bytes(),
                value: tag.content().map(|value| value.as_bytes().to_vec()),
            })
            .collect(),
        channel: event.channel_id.map(|id| id.to_string().into_bytes()),
    }
}
