//! Metadata-only recovery after a reply deletion whose live recount was missed.

use axum::{http::StatusCode, Json};
use buzz_core::{kind::KIND_THREAD_SUMMARY, TenantContext};
use nostr::{EventBuilder, Filter, Kind, Tag};
use serde_json::Value;
use uuid::Uuid;

use super::{api_error, extract_channel_from_filter, internal_error};
use crate::state::AppState;

type QueryError = (StatusCode, Json<Value>);

fn request_channel(filter: &Filter) -> Result<Uuid, QueryError> {
    let channel = extract_channel_from_filter(filter).ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "resolve_thread_roots requires exactly one #h channel",
        )
    })?;
    if !filter.kinds.as_ref().is_some_and(|kinds| {
        kinds.len() == 1 && kinds.contains(&Kind::Custom(KIND_THREAD_SUMMARY as u16))
    }) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "resolve_thread_roots requires kinds [39005]",
        ));
    }
    if !filter
        .ids
        .as_ref()
        .is_some_and(|ids| !ids.is_empty() && ids.len() <= 100)
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "resolve_thread_roots requires 1 to 100 target ids",
        ));
    }
    Ok(channel)
}

/// Return signed root summaries from retained, channel-scoped reply metadata.
pub(super) async fn query(
    state: &AppState,
    tenant: &TenantContext,
    filter: &Filter,
    accessible_channels: &[Uuid],
) -> Result<Vec<Value>, QueryError> {
    let channel = request_channel(filter)?;
    if !accessible_channels.contains(&channel) {
        return Ok(Vec::new());
    }
    let ids = filter
        .ids
        .iter()
        .flatten()
        .map(|id| id.as_bytes().to_vec())
        .collect::<Vec<_>>();
    let summaries = state
        .db
        .resolve_thread_root_summaries(tenant.community(), channel, &ids)
        .await
        .map_err(|e| internal_error(&format!("thread ownership summaries: {e}")))?;
    let mut events = Vec::with_capacity(summaries.len());
    for (root, summary) in summaries {
        let root_hex = hex::encode(root);
        let channel_hex = channel.to_string();
        let tags = [
            ["e", root_hex.as_str()],
            ["d", root_hex.as_str()],
            ["h", channel_hex.as_str()],
        ]
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| internal_error(&format!("thread summary tags: {e}")))?;
        let content = serde_json::json!({
            "reply_count": summary.reply_count,
            "descendant_count": summary.descendant_count,
            "last_reply_at": summary.last_reply_at.map(|t| t.timestamp()),
            "participants": summary.participants.iter().map(hex::encode).collect::<Vec<_>>(),
        });
        let event = EventBuilder::new(
            Kind::Custom(KIND_THREAD_SUMMARY as u16),
            content.to_string(),
        )
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| internal_error(&format!("thread summary signing: {e}")))?;
        events.push(
            serde_json::to_value(event)
                .map_err(|e| internal_error(&format!("thread summary serialization: {e}")))?,
        );
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(ids: usize) -> Filter {
        serde_json::from_value(serde_json::json!({
            "kinds": [39005], "#h": ["11111111-1111-4111-8111-111111111111"],
            "ids": (0..ids).map(|id| format!("{id:064x}")).collect::<Vec<_>>()
        }))
        .expect("valid filter")
    }

    #[test]
    fn ownership_request_requires_bounded_ids_and_explicit_scope() {
        assert!(request_channel(&filter(1)).is_ok());
        assert!(request_channel(&filter(100)).is_ok());
        assert!(request_channel(&filter(0)).is_err());
        assert!(request_channel(&filter(101)).is_err());
        let mut wrong_kind = filter(1);
        wrong_kind.kinds = Some([Kind::TextNote].into_iter().collect());
        assert!(request_channel(&wrong_kind).is_err());
        let mut unscoped = filter(1);
        unscoped.generic_tags.clear();
        assert!(request_channel(&unscoped).is_err());
    }
}
