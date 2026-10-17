//! Original-message routing for kind:40003 message edits.
//!
//! An edit is an auxiliary event: it does not render as its own timeline row,
//! and its bare `["e", <target>]` tag names the edited message rather than a
//! thread. When an edit newly mentions an agent, every reply, reaction, typing
//! indicator, session scope, and failure notice must therefore follow the
//! original message. This module resolves that original once, at admission,
//! so the queue carries the routing with the event and no later stage needs a
//! network lookup.

use std::time::Duration;

use nostr::{Alphabet, Event, EventId, Filter, Kind, SingleLetterTag};
use uuid::Uuid;

use crate::queue::{edit_target_id, parse_thread_tags, ResolvedEdit};
use crate::relay::{RelayError, RestClient};

/// Bound for the single original-event lookup. Matches the other bounded
/// admission lookups on the listener loop (see `check_sibling_via_profile`).
const EDIT_ORIGINAL_FETCH_TIMEOUT: Duration = Duration::from_millis(2_000);

/// Resolve the original message targeted by `event` if it is an edit.
///
/// Returns `None` for ordinary events and whenever the original cannot be
/// fetched and verified; callers then route to the edit target id, never the
/// auxiliary edit event (see [`crate::queue::reaction_target_id`]).
pub(crate) async fn resolve_edit(
    event: &Event,
    channel_id: Uuid,
    rest: &RestClient,
) -> Option<ResolvedEdit> {
    resolve_edit_with(event, channel_id, |filters| async move {
        rest.query(&filters).await
    })
    .await
}

pub(crate) async fn resolve_edit_with<Query, QueryFut>(
    event: &Event,
    channel_id: Uuid,
    query: Query,
) -> Option<ResolvedEdit>
where
    Query: FnOnce(Vec<Filter>) -> QueryFut,
    QueryFut: std::future::Future<Output = Result<serde_json::Value, RelayError>>,
{
    let target_event_id = edit_target_id(event)?;
    let target_id = EventId::from_hex(&target_event_id).ok()?;
    let channel = channel_id.to_string();
    // Kinds and channel are explicit: the relay's read gate requires scoped
    // filters, and an edit may only target a message in its own channel.
    let filter = Filter::new()
        .id(target_id)
        .kinds([
            Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE as u16),
            Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE_V2 as u16),
        ])
        .custom_tags(SingleLetterTag::lowercase(Alphabet::H), [channel.as_str()])
        .limit(1);
    let response = match tokio::time::timeout(EDIT_ORIGINAL_FETCH_TIMEOUT, query(vec![filter]))
        .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            tracing::warn!(edit_event_id = %event.id, target_event_id, "edit routing: original event fetch failed: {error}");
            return None;
        }
        Err(_) => {
            tracing::warn!(edit_event_id = %event.id, target_event_id, "edit routing: original event fetch timed out");
            return None;
        }
    };
    let Some(raw) = response.as_array().and_then(|events| events.first()) else {
        tracing::warn!(edit_event_id = %event.id, target_event_id, "edit routing: original event was not returned");
        return None;
    };
    match serde_json::from_value::<Event>(raw.clone()) {
        Ok(original) if original.id == target_id && original.verify().is_ok() => {
            Some(ResolvedEdit {
                target_event_id,
                target_thread_tags: parse_thread_tags(&original),
            })
        }
        Ok(_) => {
            tracing::warn!(edit_event_id = %event.id, target_event_id, "edit routing: original event failed id/signature verification");
            None
        }
        Err(error) => {
            tracing::warn!(edit_event_id = %event.id, target_event_id, "edit routing: malformed original event: {error}");
            None
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use nostr::{Event, EventBuilder, Keys, Kind, Tag};

    /// Signed kind:40003 edit targeting `target` with optional extra tags.
    pub(crate) fn edit_event(target: &str, extra: &[[&str; 2]]) -> Event {
        let mut tags = vec![Tag::parse(["e", target]).expect("edit target")];
        tags.extend(extra.iter().map(|t| Tag::parse(*t).expect("tag")));
        EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE_EDIT as u16),
            "edited mention",
        )
        .tags(tags)
        .sign_with_keys(&Keys::generate())
        .expect("signed edit")
    }

    /// Signed kind:9 message, threaded under `root` when given.
    pub(crate) fn message(root: Option<&str>) -> Event {
        let tags: Vec<Tag> = root
            .map(|root| vec![Tag::parse(["e", root, "", "reply"]).expect("reply tag")])
            .unwrap_or_default();
        EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE as u16),
            "original",
        )
        .tags(tags)
        .sign_with_keys(&Keys::generate())
        .expect("signed message")
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{edit_event, message};
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn resolves_threaded_original_with_scoped_filter() {
        let root = "aa".repeat(32);
        let original = message(Some(&root));
        let edit = edit_event(&original.id.to_hex(), &[]);
        let channel = Uuid::new_v4();
        let returned = original.clone();
        let resolved = resolve_edit_with(&edit, channel, |filters| async move {
            let filter = serde_json::to_value(&filters[0]).unwrap();
            assert_eq!(filter["ids"], json!([returned.id.to_hex()]));
            assert_eq!(filter["kinds"], json!([9, 40002]));
            assert_eq!(filter["#h"], json!([channel.to_string()]));
            Ok(json!([returned]))
        })
        .await
        .expect("resolved");
        assert_eq!(resolved.target_event_id, original.id.to_hex());
        assert_eq!(
            resolved.target_thread_tags.root_event_id.as_deref(),
            Some(root.as_str())
        );
    }

    #[tokio::test]
    async fn rejects_a_returned_event_that_is_not_the_target() {
        let target = message(None);
        let impostor = message(None);
        let edit = edit_event(&target.id.to_hex(), &[]);
        let resolved =
            resolve_edit_with(
                &edit,
                Uuid::new_v4(),
                |_| async move { Ok(json!([impostor])) },
            )
            .await;
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn ordinary_events_are_not_resolved() {
        let resolved = resolve_edit_with(&message(None), Uuid::new_v4(), |_| async {
            panic!("ordinary events must not query the relay")
        })
        .await;
        assert!(resolved.is_none());
    }
}
