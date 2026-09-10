//! Addressed, thread-bound failure notices. These are recovery signals, not
//! authority to replay a task or evidence that its prior side effects stopped.

use std::collections::HashSet;

use nostr::{Event, EventId, Keys, Tag};

use crate::queue::{parse_thread_tags, FlushBatch};
use crate::scope::SessionScope;

const FAILURE_TAG: &str = "buzz:agent-failure";
const MAX_RECIPIENTS: usize = 50;

/// Build the signed notice sent by the production failure path.
pub(crate) fn build(keys: &Keys, batch: &FlushBatch, content: &str) -> anyhow::Result<Event> {
    anyhow::ensure!(
        batch.scope.channel_id() == batch.channel_id,
        "failure notice scope/channel mismatch"
    );
    let trigger = batch
        .events
        .last()
        .or_else(|| batch.cancelled_events.last())
        .ok_or_else(|| anyhow::anyhow!("failure notice has no triggering event"))?;
    let root_id = match &batch.scope {
        SessionScope::Thread { root_event_id, .. } => EventId::from_hex(root_event_id)?,
        SessionScope::Conversation { .. } => parse_thread_tags(&trigger.event)
            .root_event_id
            .as_deref()
            .map(EventId::from_hex)
            .transpose()?
            .unwrap_or(trigger.event.id),
    };
    let thread = buzz_sdk::ThreadRef {
        root_event_id: root_id,
        parent_event_id: trigger.event.id,
    };

    let own_key = keys.public_key();
    let mut seen = HashSet::new();
    let recipients: Vec<String> = batch
        .events
        .iter()
        .rev()
        .chain(batch.cancelled_events.iter().rev())
        .filter(|item| {
            // Never bounce a native failure signal back to its sender. A
            // failed recovery turn may still report a visible, unaddressed
            // notice; a new ordinary request in the batch remains eligible.
            !item.event.tags.iter().any(|tag| {
                let parts = tag.as_slice();
                parts.first().map(String::as_str) == Some(FAILURE_TAG)
                    && parts.get(1).map(String::as_str) == Some("1")
            })
        })
        .map(|item| item.event.pubkey)
        .filter(|key| *key != own_key && seen.insert(*key))
        .take(MAX_RECIPIENTS)
        .map(|key| key.to_hex())
        .collect();
    let mentions: Vec<&str> = recipients.iter().map(String::as_str).collect();
    let marker = Tag::parse([FAILURE_TAG, "1"])?;
    Ok(buzz_sdk::build_message(
        batch.channel_id,
        content,
        Some(&thread),
        &mentions,
        false,
        &[],
        &[],
    )?
    .tag(marker)
    .sign_with_keys(keys)?)
}
