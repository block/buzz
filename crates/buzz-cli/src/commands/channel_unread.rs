//! `buzz channels unread` — per-channel unread counts from the synced NIP-RS
//! read state. Read-only: it never publishes.
//!
//! Counting follows desktop's `communityUnreadObserver.ts`
//! (`isUnreadExternalEvent`): an event is unread when it is not yours and is
//! newer than the greatest of the channel frontier, its `msg:<id>` marker and,
//! for a non-broadcast reply, its `thread:<root>` marker. Thread replies count.
//! Desktop's notification gate (participated/followed threads, muted channels,
//! hidden DMs) lives in desktop-local storage and is not applied here.

use std::collections::HashMap;

use buzz_core::kind::{
    KIND_FORUM_COMMENT, KIND_FORUM_POST, KIND_HUDDLE_STARTED, KIND_STREAM_MESSAGE,
    KIND_STREAM_MESSAGE_V2,
};
use serde_json::{json, Value};

use super::read_state::{self, ReadState};
use crate::client::{extract_d_tag, extract_tag_value, BuzzClient};
use crate::error::CliError;
use crate::validate::validate_uuid;

const KIND_GROUP_METADATA: u32 = 39000;
const KIND_GROUP_MEMBERS: u32 = 39002;
const MEMBER_CHANNEL_LIMIT: u32 = 1000;

/// Resource bounds of one run.
pub(crate) struct Bounds {
    /// Most channels counted in one run.
    pub max_channels: usize,
    /// Most events read per channel; a channel that reaches it reports `capped`.
    pub events_per_channel: u32,
}

pub(crate) const BOUNDS: Bounds = Bounds {
    max_channels: 200,
    events_per_channel: 1000,
};

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ChannelUnread {
    pub channel_id: String,
    pub name: String,
    pub last_read_at: Option<u64>,
    pub unread_count: u32,
    /// `unread_count` is a lower bound: the per-channel event cap was reached.
    pub capped: bool,
}

#[derive(Debug)]
pub(crate) struct Outcome {
    pub channels: Vec<ChannelUnread>,
    /// Channels found but not counted because of `--limit`.
    pub dropped: usize,
    pub skipped_slots: Vec<read_state::SkippedSlot>,
}

pub(super) struct Channel {
    pub id: String,
    name: String,
    pub is_dm: bool,
}

pub(super) fn message_kinds(is_dm: bool) -> Vec<u32> {
    let mut kinds = vec![
        KIND_STREAM_MESSAGE,
        KIND_STREAM_MESSAGE_V2,
        KIND_FORUM_POST,
        KIND_FORUM_COMMENT,
    ];
    if is_dm {
        kinds.push(KIND_HUDDLE_STARTED);
    }
    kinds
}

fn tags(event: &Value) -> impl Iterator<Item = Vec<&str>> {
    event
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tag| tag.as_array())
        .map(|tag| tag.iter().map(|v| v.as_str().unwrap_or("")).collect())
}

/// The thread root a reply counts against, as desktop's `getThreadReference`
/// and `isBroadcastReply` resolve it. A broadcast reply has none.
pub(super) fn thread_root(event: &Value) -> Option<String> {
    let mut broadcast = false;
    let mut root = None;
    let mut reply = None;
    for tag in tags(event) {
        match tag.as_slice() {
            ["broadcast", "1", ..] => broadcast = true,
            ["e", id, _, "root", ..] => {
                root.get_or_insert(*id);
            }
            ["e", id, _, "reply", ..] => reply = Some(*id),
            _ => {}
        }
    }
    if broadcast {
        return None;
    }
    let parent = reply?;
    Some(root.unwrap_or(parent).to_string())
}

fn is_unread(event: &Value, me: &str, channel_read_at: Option<u64>, state: &ReadState) -> bool {
    if event
        .get("pubkey")
        .and_then(Value::as_str)
        .is_some_and(|p| p.eq_ignore_ascii_case(me))
    {
        return false;
    }
    let id = event.get("id").and_then(Value::as_str).unwrap_or("");
    let created_at = event.get("created_at").and_then(Value::as_u64).unwrap_or(0);
    let read_at = [
        channel_read_at,
        state.frontier(&format!("msg:{id}")),
        thread_root(event).and_then(|root| state.frontier(&format!("thread:{root}"))),
    ]
    .into_iter()
    .flatten()
    .max();
    read_at.is_none_or(|read_at| created_at > read_at)
}

fn newest_by_d(events: Vec<Value>) -> HashMap<String, Value> {
    let mut newest: HashMap<String, Value> = HashMap::new();
    for event in events {
        let d = extract_d_tag(&event);
        if d.is_empty() {
            continue;
        }
        let at = |e: &Value| e.get("created_at").and_then(Value::as_u64).unwrap_or(0);
        if newest.get(&d).is_none_or(|head| at(&event) > at(head)) {
            newest.insert(d, event);
        }
    }
    newest
}

async fn member_channel_ids(client: &BuzzClient) -> Result<Vec<String>, CliError> {
    let filter = json!({
        "kinds": [KIND_GROUP_MEMBERS],
        "#p": [client.keys().public_key().to_hex()],
    });
    let events = client
        .query_all_bounded(filter, MEMBER_CHANNEL_LIMIT)
        .await
        .map_err(|e| match e {
            CliError::Other(msg) => {
                CliError::Other(format!("{msg}; name the channels with --channel instead"))
            }
            other => other,
        })?;
    let mut ids: Vec<String> = events
        .iter()
        .map(extract_d_tag)
        .filter(|id| !id.is_empty())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

async fn channel_metadata(
    client: &BuzzClient,
    ids: &[String],
) -> Result<HashMap<String, Value>, CliError> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let filter = json!({"kinds": [KIND_GROUP_METADATA], "#d": ids});
    let events = client
        .query_all_bounded(filter, MEMBER_CHANNEL_LIMIT)
        .await?;
    Ok(newest_by_d(events))
}

fn is_archived(metadata: &Value) -> bool {
    tags(metadata).any(|t| t.as_slice() == ["archived", "true"])
}

pub(super) async fn resolve_channels(
    client: &BuzzClient,
    requested: &[String],
    limit: usize,
) -> Result<(Vec<Channel>, usize), CliError> {
    let explicit = !requested.is_empty();
    let ids = if explicit {
        requested.to_vec()
    } else {
        member_channel_ids(client).await?
    };
    let metadata = channel_metadata(client, &ids).await?;
    let mut channels = Vec::new();
    for id in ids {
        match metadata.get(&id) {
            Some(meta) if !explicit && is_archived(meta) => {}
            Some(meta) => channels.push(Channel {
                name: extract_tag_value(meta, "name"),
                is_dm: extract_tag_value(meta, "t") == "dm",
                id,
            }),
            None if explicit => {
                return Err(CliError::NotFound(format!("channel {id} not found")));
            }
            None => channels.push(Channel {
                name: String::new(),
                is_dm: false,
                id,
            }),
        }
    }
    channels.sort_by(|a, b| a.id.cmp(&b.id));
    let dropped = channels.len().saturating_sub(limit);
    channels.truncate(limit);
    Ok((channels, dropped))
}

async fn count_channel(
    client: &BuzzClient,
    channel: &Channel,
    state: &ReadState,
    bounds: &Bounds,
) -> Result<ChannelUnread, CliError> {
    let last_read_at = state.frontier(&channel.id);
    let mut filter = json!({
        "kinds": message_kinds(channel.is_dm),
        "#h": [channel.id],
    });
    if let Some(read_at) = last_read_at {
        filter["since"] = json!(read_at + 1);
    }
    let events = client
        .query_paginated(filter, bounds.events_per_channel)
        .await?;
    let capped = events.len() >= bounds.events_per_channel as usize;
    let me = client.keys().public_key().to_hex();
    let unread = events
        .iter()
        .filter(|e| is_unread(e, &me, last_read_at, state))
        .count();
    Ok(ChannelUnread {
        channel_id: channel.id.clone(),
        name: channel.name.clone(),
        last_read_at,
        unread_count: u32::try_from(unread).unwrap_or(u32::MAX),
        capped,
    })
}

/// Compute unread counts for the user's channels, or for `requested` ones.
pub(crate) async fn unread(
    client: &BuzzClient,
    requested: &[String],
    limit: Option<u32>,
    bounds: &Bounds,
) -> Result<Outcome, CliError> {
    let limit = match limit {
        None => bounds.max_channels,
        Some(0) => return Err(CliError::Usage("--limit must be at least 1".into())),
        Some(n) if n as usize > bounds.max_channels => {
            return Err(CliError::Usage(format!(
                "--limit must be at most {}",
                bounds.max_channels
            )));
        }
        Some(n) => n as usize,
    };
    let mut requested: Vec<String> = requested
        .iter()
        .map(|c| validate_uuid(c).map(|()| c.to_lowercase()))
        .collect::<Result<_, _>>()?;
    requested.sort();
    requested.dedup();
    if requested.len() > limit {
        return Err(CliError::Usage(format!(
            "{} channels given but at most {limit} can be counted per run",
            requested.len()
        )));
    }

    let state = read_state::fetch(client).await?;
    let (channels, dropped) = resolve_channels(client, &requested, limit).await?;
    let mut counted = Vec::with_capacity(channels.len());
    for channel in &channels {
        counted.push(count_channel(client, channel, &state, bounds).await?);
    }
    counted.sort_by(|a, b| {
        b.unread_count
            .cmp(&a.unread_count)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.channel_id.cmp(&b.channel_id))
    });
    Ok(Outcome {
        channels: counted,
        dropped,
        skipped_slots: state.skipped,
    })
}

pub async fn cmd_unread(
    client: &BuzzClient,
    requested: &[String],
    limit: Option<u32>,
) -> Result<(), CliError> {
    let outcome = unread(client, requested, limit, &BOUNDS).await?;
    if outcome.dropped > 0 {
        eprintln!(
            "{}",
            json!({
                "warning": "channels_truncated",
                "counted": outcome.channels.len(),
                "dropped": outcome.dropped,
                "message": "more channels than --limit; rerun with --channel to count the rest",
            })
        );
    }
    if !outcome.skipped_slots.is_empty() {
        let slots: Vec<Value> = outcome
            .skipped_slots
            .iter()
            .map(|s| json!({"d": s.d_tag, "v": s.version}))
            .collect();
        eprintln!(
            "{}",
            json!({
                "warning": "read_state_slots_skipped",
                "slots": slots,
                "message": "written by a newer schema; counts may overstate unread",
            })
        );
    }
    let out = serde_json::to_string(&outcome.channels)
        .map_err(|e| CliError::Other(format!("failed to encode unread counts: {e}")))?;
    println!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::read_state::tests::slot_event;
    use crate::commands::test_relay::{fake_relay, Shared};
    use nostr::Keys;

    struct World {
        client: BuzzClient,
        relay: Shared,
        other: Keys,
        counter: u64,
    }

    const GEN: &str = "11111111-1111-4111-8111-111111111111";
    const DEV: &str = "22222222-2222-4222-8222-222222222222";
    const OLD: &str = "33333333-3333-4333-8333-333333333333";
    const NEW: &str = "44444444-4444-4444-8444-444444444444";

    async fn world() -> World {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
        World {
            client,
            relay,
            other: Keys::generate(),
            counter: 0,
        }
    }

    impl World {
        fn push(&self, event: Value) {
            self.relay.lock().unwrap().events.push(event);
        }

        fn signed(&self, keys: &Keys, kind: u16, tags: Vec<nostr::Tag>, at: u64) -> Value {
            let event = nostr::EventBuilder::new(nostr::Kind::Custom(kind), "x")
                .tags(tags)
                .custom_created_at(nostr::Timestamp::from(at))
                .sign_with_keys(keys)
                .unwrap();
            serde_json::to_value(event).unwrap()
        }

        fn member(&self, channel: &str) {
            let me = self.client.keys().public_key();
            let tags = vec![nostr::Tag::identifier(channel), nostr::Tag::public_key(me)];
            self.push(self.signed(&Keys::generate(), 39002, tags, 1));
        }

        fn channel(&self, id: &str, name: &str, extra: &[(&str, &str)]) {
            let mut tags = vec![
                nostr::Tag::identifier(id),
                nostr::Tag::parse(["name", name]).unwrap(),
            ];
            tags.extend(
                extra
                    .iter()
                    .map(|(k, v)| nostr::Tag::parse([*k, *v]).unwrap()),
            );
            self.push(self.signed(&Keys::generate(), 39000, tags, 1));
        }

        fn say(&mut self, channel: &str, author: &Keys, at: u64, extra: &[Vec<&str>]) -> String {
            self.counter += 1;
            let mut tags = vec![nostr::Tag::parse(["h", channel]).unwrap()];
            tags.extend(extra.iter().map(|t| nostr::Tag::parse(t.clone()).unwrap()));
            tags.push(nostr::Tag::parse(["nonce", &self.counter.to_string()]).unwrap());
            let event = self.signed(author, 9, tags, at);
            let id = event["id"].as_str().unwrap().to_string();
            self.push(event);
            id
        }

        fn read_state(&self, slot: &str, contexts: Value, at: u64) {
            let plaintext = json!({"v": 1, "client_id": slot, "contexts": contexts}).to_string();
            self.push(slot_event(self.client.keys(), slot, &plaintext, at));
        }

        fn queries_for(&self, kind: u64) -> Vec<Value> {
            self.relay
                .lock()
                .unwrap()
                .filters
                .iter()
                .filter(|f| {
                    f["kinds"]
                        .as_array()
                        .is_some_and(|k| k.contains(&json!(kind)))
                })
                .cloned()
                .collect()
        }
    }

    fn by_id<'a>(out: &'a Outcome, id: &str) -> &'a ChannelUnread {
        out.channels.iter().find(|c| c.channel_id == id).unwrap()
    }

    #[tokio::test]
    async fn counts_unread_with_desktop_rules() {
        let mut w = world().await;
        let me = w.client.keys().clone();
        let other = w.other.clone();
        for (id, name) in [(GEN, "general"), (DEV, "dev"), (OLD, "old")] {
            w.member(id);
            w.channel(id, name, &[]);
        }
        w.member(NEW);
        w.channel(NEW, "new", &[("archived", "true")]);

        // general: frontier 100 from slot a, raised to 150 by slot b (max-merge).
        w.say(GEN, &other, 90, &[]);
        w.say(GEN, &other, 120, &[]);
        w.say(GEN, &other, 160, &[]);
        w.say(GEN, &me, 170, &[]);
        let marked = w.say(GEN, &other, 180, &[]);
        let root = w.say(GEN, &other, 80, &[]);
        let reply = w.say(
            GEN,
            &other,
            200,
            &[vec!["e", &root, "", "root"], vec!["e", &root, "", "reply"]],
        );
        let broadcast = w.say(
            GEN,
            &other,
            203,
            &[
                vec!["e", &root, "", "root"],
                vec!["e", &root, "", "reply"],
                vec!["broadcast", "1"],
            ],
        );
        w.read_state("a", json!({GEN: 100, DEV: 5}), 1);
        w.read_state(
            "b",
            json!({GEN: 150, format!("msg:{marked}"): 180, format!("thread:{root}"): 205}),
            2,
        );
        // dev: never read, two from others, one of mine.
        w.say(DEV, &other, 10, &[]);
        w.say(DEV, &other, 11, &[]);
        w.say(DEV, &me, 12, &[]);
        // old: fully read.
        w.say(OLD, &other, 50, &[]);
        w.read_state("c", json!({OLD: 50}), 3);
        let _ = (reply, broadcast);

        let out = unread(&w.client, &[], None, &BOUNDS).await.unwrap();

        // 160 (> 150); 180 is covered by its msg marker; `reply` 200 is covered by
        // thread:root 205; the broadcast reply 203 ignores the thread marker; 90/120
        // predate the frontier; mine is excluded.
        assert_eq!(by_id(&out, GEN).unread_count, 2);
        assert_eq!(by_id(&out, GEN).last_read_at, Some(150));
        assert_eq!(by_id(&out, DEV).unread_count, 2);
        assert_eq!(by_id(&out, DEV).last_read_at, Some(5));
        assert_eq!(by_id(&out, OLD).unread_count, 0);
        assert!(
            out.channels.iter().all(|c| c.channel_id != NEW),
            "archived is skipped"
        );
        assert!(out.channels.iter().all(|c| !c.capped));
        assert_eq!(out.dropped, 0);
        let order: Vec<_> = out.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            order,
            ["dev", "general", "old"],
            "sorted by unread desc, then name"
        );

        // The production queries are bounded and scoped, not a firehose.
        let reads = w.queries_for(9);
        assert_eq!(reads.len(), 3);
        for q in &reads {
            assert!(q["limit"].as_u64().unwrap() <= u64::from(BOUNDS.events_per_channel));
            assert!(q["#h"].as_array().is_some_and(|h| h.len() == 1));
        }
        let general_q = reads.iter().find(|q| q["#h"] == json!([GEN])).unwrap();
        assert_eq!(
            general_q["since"],
            json!(151),
            "reads start after the frontier"
        );
        let dev_q = reads.iter().find(|q| q["#h"] == json!([DEV])).unwrap();
        assert_eq!(dev_q["since"], json!(6));
        let state_qs = w.queries_for(30078);
        assert_eq!(state_qs.len(), 1, "read state is one atomic snapshot");
        assert_eq!(state_qs[0]["read_state_snapshot"], json!(1));
        for paging in ["#t", "since", "until", "limit"] {
            assert!(state_qs[0].get(paging).is_none(), "{paging}");
        }
    }

    #[tokio::test]
    async fn bounds_channels_and_events_and_says_so() {
        let mut w = world().await;
        let other = w.other.clone();
        for id in [GEN, DEV, OLD] {
            w.member(id);
            w.channel(id, id, &[]);
            for at in 1..=4 {
                w.say(id, &other, at, &[]);
            }
        }
        let bounds = Bounds {
            max_channels: 2,
            events_per_channel: 3,
        };

        let out = unread(&w.client, &[], Some(2), &bounds).await.unwrap();
        assert_eq!((out.channels.len(), out.dropped), (2, 1));
        assert!(out.channels.iter().all(|c| c.capped && c.unread_count == 3));
        assert_eq!(
            w.queries_for(9).len(),
            2,
            "the dropped channel is never queried"
        );

        let err = unread(&w.client, &[], Some(3), &bounds).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        let err = unread(&w.client, &[], Some(0), &bounds).await.unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        let all = [GEN, DEV, OLD].map(String::from);
        let err = unread(&w.client, &all, None, &bounds).await.unwrap_err();
        assert!(
            matches!(err, CliError::Usage(_)),
            "explicit channels are never silently dropped"
        );
    }

    #[tokio::test]
    async fn explicit_channels_are_validated_and_must_exist() {
        let mut w = world().await;
        let other = w.other.clone();
        w.channel(GEN, "general", &[("archived", "true"), ("t", "dm")]);
        w.say(GEN, &other, 5, &[]);

        let out = unread(&w.client, &[GEN.to_uppercase(), GEN.into()], None, &BOUNDS)
            .await
            .unwrap();
        assert_eq!(
            out.channels.len(),
            1,
            "case-insensitive dedupe; archived is allowed when named"
        );
        assert_eq!(out.channels[0].unread_count, 1);
        let queried = w.queries_for(9);
        assert!(
            queried[0]["kinds"]
                .as_array()
                .unwrap()
                .contains(&json!(48100)),
            "dm counts huddle starts"
        );

        let err = unread(&w.client, &["nope".into()], None, &BOUNDS)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        let err = unread(&w.client, &[DEV.into()], None, &BOUNDS)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)));
    }

    #[tokio::test]
    async fn an_unreadable_slot_fails_the_command_before_any_count() {
        let w = world().await;
        w.member(GEN);
        w.channel(GEN, "general", &[]);
        w.read_state("a", json!({GEN: 1}), 1);
        w.push(slot_event(w.client.keys(), "b", "garbage", 2));

        let err = unread(&w.client, &[], None, &BOUNDS).await.unwrap_err();
        assert!(err.to_string().contains("unreadable"), "{err}");
        assert!(w.queries_for(9).is_empty());
        assert!(w.queries_for(39002).is_empty());
    }
}
