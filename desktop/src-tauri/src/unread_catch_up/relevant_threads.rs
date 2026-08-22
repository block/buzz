use std::collections::{HashMap, HashSet};

use futures_util::{stream, StreamExt};
use nostr::{Event, Keys};

use super::{
    catch_up_kinds, fetch_filter_pages, AppState, CatchUpChannel, CHANNEL_FETCH_CONCURRENCY,
    ROOT_FILTER_CHUNK,
};

// Mirrors buzz-relay's aggregate explicit `#h` ceiling. Keep every HTTP
// bridge request independently admissible when the channel set is large.
const CHANNEL_FILTER_CHUNK: usize = 128;

pub(super) struct RelevantThreadEvents {
    pub(super) by_channel: HashMap<String, Vec<Event>>,
    pub(super) errors_by_channel: HashMap<String, String>,
}

struct RelevantThreadQuery {
    channels: Vec<CatchUpChannel>,
    filter: serde_json::Value,
}

fn relevant_thread_filter(channels: &[CatchUpChannel], roots: &[String]) -> serde_json::Value {
    let since = channels
        .iter()
        .map(relevant_thread_since)
        .min()
        .unwrap_or(0);
    let channel_type = if channels.iter().any(|channel| channel.channel_type == "dm") {
        "dm"
    } else {
        "stream"
    };
    let channel_ids = channels
        .iter()
        .map(|channel| channel.id.as_str())
        .collect::<Vec<_>>();
    serde_json::json!({
        "kinds": catch_up_kinds(channel_type),
        "#h": channel_ids,
        "#e": roots,
        "since": since,
        "limit": super::CATCH_UP_LIMIT,
    })
}

fn relevant_thread_since(channel: &CatchUpChannel) -> u64 {
    channel.read_at.map_or(0, |value| value.saturating_add(1))
}

fn relevant_thread_queries(
    channels: &[CatchUpChannel],
    roots: &[String],
) -> Vec<RelevantThreadQuery> {
    let mut queries = Vec::new();
    for channel_chunk in channels.chunks(CHANNEL_FILTER_CHUNK) {
        for root_chunk in roots.chunks(ROOT_FILTER_CHUNK) {
            queries.push(RelevantThreadQuery {
                channels: channel_chunk.to_vec(),
                filter: relevant_thread_filter(channel_chunk, root_chunk),
            });
        }
    }
    queries
}

fn event_channel_id(event: &Event) -> Option<&str> {
    event
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().is_some_and(|part| part == "h"))
        .and_then(|tag| tag.as_slice().get(1))
        .map(String::as_str)
}

fn bucket_requested_events(
    events: Vec<Event>,
    channels: &[CatchUpChannel],
) -> HashMap<String, Vec<Event>> {
    let requested = channels
        .iter()
        .map(|channel| (channel.id.to_ascii_lowercase(), channel))
        .collect::<HashMap<_, _>>();
    let mut by_channel = HashMap::<String, Vec<Event>>::new();
    for event in events {
        let Some(channel_id) = event_channel_id(&event).map(str::to_ascii_lowercase) else {
            continue;
        };
        let Some(channel) = requested.get(&channel_id) else {
            continue;
        };
        if !catch_up_kinds(&channel.channel_type).contains(&(event.kind.as_u16() as u32)) {
            continue;
        }
        by_channel.entry(channel_id).or_default().push(event);
    }
    by_channel
}

pub(super) async fn fetch_relevant_thread_events(
    state: &AppState,
    api_base: &str,
    keys: &Keys,
    channels: &[CatchUpChannel],
    roots: &[String],
) -> RelevantThreadEvents {
    if channels.is_empty() || roots.is_empty() {
        return RelevantThreadEvents {
            by_channel: HashMap::new(),
            errors_by_channel: HashMap::new(),
        };
    }
    let pages = stream::iter(relevant_thread_queries(channels, roots))
        .map(|query| async move {
            let result = fetch_filter_pages(state, api_base, keys, &query.filter).await;
            (query.channels, result)
        })
        .buffered(CHANNEL_FETCH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let mut events = Vec::new();
    let mut errors_by_channel = HashMap::new();
    for (query_channels, page) in pages {
        match page {
            Ok(page) => events.extend(page),
            Err(error) => {
                for channel in query_channels {
                    errors_by_channel
                        .entry(channel.id.to_ascii_lowercase())
                        .or_insert_with(|| error.clone());
                }
            }
        }
    }
    let mut seen = HashSet::new();
    events.retain(|event| seen.insert(event.id));
    let mut by_channel = bucket_requested_events(events, channels);
    for channel_id in errors_by_channel.keys() {
        by_channel.remove(channel_id);
    }
    RelevantThreadEvents {
        by_channel,
        errors_by_channel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: &str, channel_type: &str, read_at: Option<u64>) -> CatchUpChannel {
        CatchUpChannel {
            id: id.into(),
            channel_type: channel_type.into(),
            name: id.into(),
            read_at,
            timeline_read_at: read_at,
            discovery_at: None,
        }
    }

    #[test]
    fn one_root_filter_covers_channels_with_the_same_frontier() {
        let channels = vec![
            channel("a", "stream", Some(10)),
            channel("b", "dm", Some(10)),
        ];
        let filter = relevant_thread_filter(&channels, &["root".into()]);

        assert_eq!(filter["since"], 11);
        assert_eq!(filter["#e"], serde_json::json!(["root"]));
        assert_eq!(filter["#h"], serde_json::json!(["a", "b"]));
        assert!(filter["kinds"].as_array().is_some_and(
            |kinds| kinds.contains(&serde_json::json!(super::super::KIND_HUDDLE_STARTED))
        ));
    }

    #[test]
    fn channel_chunk_uses_its_oldest_frontier_and_filters_later_per_channel() {
        let channels = vec![
            channel("old", "stream", None),
            channel("current", "stream", Some(900)),
            channel("same-current", "dm", Some(900)),
        ];

        let queries = relevant_thread_queries(&channels, &["root".into()]);
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].filter["since"], 0);
        assert_eq!(
            queries[0].filter["#h"],
            serde_json::json!(["old", "current", "same-current"])
        );
    }

    #[test]
    fn same_frontier_channels_respect_the_relay_explicit_channel_limit() {
        let channels = (0..129)
            .map(|index| channel(&format!("channel-{index}"), "stream", None))
            .collect::<Vec<_>>();

        let queries = relevant_thread_queries(&channels, &["root".into()]);
        let sizes = queries
            .iter()
            .map(|query| query.filter["#h"].as_array().map_or(0, Vec::len))
            .collect::<Vec<_>>();

        assert_eq!(sizes, [128, 1]);
        assert!(sizes.iter().all(|size| *size <= CHANNEL_FILTER_CHUNK));
    }

    #[test]
    fn distinct_frontiers_do_not_multiply_the_root_chunk_fanout() {
        let channels = (0..100)
            .map(|index| channel(&format!("channel-{index}"), "stream", Some(index)))
            .collect::<Vec<_>>();
        let roots = (0..1_000)
            .map(|index| format!("root-{index}"))
            .collect::<Vec<_>>();

        assert_eq!(relevant_thread_queries(&channels, &roots).len(), 5);
    }
}
