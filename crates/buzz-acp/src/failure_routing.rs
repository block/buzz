//! Opt-in recovery notifications. A route selects one sibling, not a model or
//! execution lease. It must never turn an external request into sibling authority.

use std::{collections::HashSet, time::Duration};

use nostr::{Event, PublicKey};

use crate::{queue::FlushBatch, relay::RestClient, OwnerCache};

pub(crate) const VISITED_TAG: &str = "buzz:agent-failure-visited";
// At most eight failing agents, including the final recipient. Do not send
// again once eight agents have already been visited.
const MAX_HOPS: usize = 8;
const MAX_AUTHORS: usize = 50;
const MAX_SOURCE_EVENTS: usize = 256;

/// Explicit operator choices; neither is enabled by default.
#[derive(Clone, Debug, Default)]
pub(crate) struct FailureHandlers {
    pub ordinary: Option<PublicKey>,
    pub recovery: Option<PublicKey>,
}

impl FailureHandlers {
    pub(crate) fn parse(
        ordinary: Option<&str>,
        recovery: Option<&str>,
        own: PublicKey,
    ) -> Result<Self, String> {
        let parse = |value: Option<&str>| -> Result<Option<PublicKey>, String> {
            value
                .map(|value| {
                    let key = PublicKey::from_hex(value.trim())
                        .map_err(|_| "failure handler must be a 64-character hex pubkey")?;
                    if key == own {
                        return Err("failure handler cannot be this agent".into());
                    }
                    Ok(key)
                })
                .transpose()
        };
        Ok(Self {
            ordinary: parse(ordinary)?,
            recovery: parse(recovery)?,
        })
    }

    pub(crate) fn enabled(&self) -> bool {
        self.ordinary.is_some() || self.recovery.is_some()
    }
}

/// Created only after source, sibling and membership checks succeed.
pub(crate) struct VerifiedRoute {
    recipient: PublicKey,
    visited: Vec<PublicKey>,
}

impl VerifiedRoute {
    pub(crate) fn recipient(&self) -> String {
        self.recipient.to_hex()
    }

    pub(crate) fn visited(&self) -> impl Iterator<Item = String> + '_ {
        self.visited.iter().map(PublicKey::to_hex)
    }
}

pub(crate) fn is_failure(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.first().map(String::as_str) == Some("buzz:agent-failure")
            && parts.get(1).map(String::as_str) == Some("1")
    })
}

fn candidate(
    own: PublicKey,
    batch: &FlushBatch,
    handlers: &FailureHandlers,
) -> Option<VerifiedRoute> {
    if batch
        .events
        .len()
        .saturating_add(batch.cancelled_events.len())
        > MAX_SOURCE_EVENTS
        || batch.scope.channel_id() != batch.channel_id
    {
        return None;
    }
    let events: Vec<_> = batch
        .events
        .iter()
        .chain(batch.cancelled_events.iter())
        .map(|item| &item.event)
        .collect();
    if events.is_empty() {
        return None;
    }
    // A fresh request does not erase a recovery branch merged into its batch.
    // Preserve the union of visited agents even when selecting the ordinary
    // handler. A separate fresh request can still start a new recovery chain.
    let recipient = if events.iter().any(|event| !is_failure(event)) {
        handlers.ordinary?
    } else {
        handlers.recovery?
    };
    let mut visited = Vec::new();
    for event in events.iter().filter(|event| is_failure(event)) {
        for tag in event
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(VISITED_TAG))
        {
            let parts = tag.as_slice();
            if parts.len() != 2 {
                return None;
            }
            let key = PublicKey::from_hex(&parts[1]).ok()?;
            if !visited.contains(&key) {
                visited.push(key);
            }
            if visited.len() >= MAX_HOPS {
                return None;
            }
        }
        if !visited.contains(&event.pubkey) {
            visited.push(event.pubkey);
        }
    }
    if !visited.contains(&own) {
        visited.push(own);
    }
    if visited.len() >= MAX_HOPS || visited.contains(&recipient) {
        return None;
    }
    Some(VerifiedRoute { recipient, visited })
}

/// One total preflight deadline. Failure leaves the existing author notification
/// behavior in place; it never tries another identity or weakens an author gate.
pub(crate) async fn resolve(
    rest: &RestClient,
    batch: &FlushBatch,
    handlers: &FailureHandlers,
    owner: Option<String>,
) -> Option<VerifiedRoute> {
    let route = candidate(rest.keys.public_key(), batch, handlers)?;
    let owner = owner?;
    let checks = async {
        let cache = OwnerCache::new(Some(owner));
        let mut authors = HashSet::new();
        for item in batch.events.iter().chain(batch.cancelled_events.iter()) {
            if item.event.verify().is_err()
                || buzz_sdk::extract_channel_id(&item.event) != Some(batch.channel_id)
            {
                return None;
            }
            authors.insert(item.event.pubkey.to_hex());
        }
        if authors.len() > MAX_AUTHORS {
            return None;
        }
        for author in authors {
            if !crate::is_owner_or_sibling(&author, &cache, rest).await {
                return None;
            }
        }
        // A handler must be an attested sibling, not the human owner.
        if !crate::check_sibling_via_profile(&route.recipient(), cache.get()?, rest).await {
            return None;
        }
        let relay_key = PublicKey::from_hex(&rest.relay_self().await.ok()??).ok()?;
        let membership = rest
            .query_raw(&[serde_json::json!({
                "kinds": [39002], "#d": [batch.channel_id.to_string()], "limit": 1
            })])
            .await
            .ok()?;
        let event: Event = serde_json::from_value(membership.as_array()?.first()?.clone()).ok()?;
        // NIP-29 snapshots must be signed by the active relay's NIP-11 self key.
        // A valid signature by an arbitrary channel member is insufficient.
        let tags = &event.tags;
        if event.kind.as_u16() != 39002
            || event.pubkey != relay_key
            || event.verify().is_err()
            || !tags.iter().any(|tag| {
                tag.as_slice().first().map(String::as_str) == Some("d")
                    && tag.as_slice().get(1).map(String::as_str)
                        == Some(batch.channel_id.to_string().as_str())
            })
        {
            return None;
        }
        let recipient = route.recipient();
        let member = tags.iter().any(|tag| {
            tag.as_slice().first().map(String::as_str) == Some("p")
                && tag.as_slice().get(1).map(String::as_str) == Some(recipient.as_str())
        });
        member.then_some(route)
    };
    match tokio::time::timeout(Duration::from_secs(5), checks).await {
        Ok(Some(route)) => Some(route),
        _ => {
            tracing::warn!(channel = %batch.channel_id, "failure handler preflight unavailable or denied; retaining default notice");
            None
        }
    }
}
