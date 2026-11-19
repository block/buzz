//! `buzz sidebar stars|mutes|projects` — your own synced per-id flags
//! (kind:30078, d=`channel-stars` / `channel-mutes` /
//! `project-sidebar-membership`, NIP-44 encrypted to yourself).
//!
//! All three documents are `{version: 1, <collection>: {<id>: {<flag>,
//! updatedAt}}}` with a per-entry last-writer-wins merge, wire-compatible with
//! desktop `channelStarsStorage.ts`, `channelMutesStorage.ts` and
//! `projectSidebarMembership.ts`. Every write is one head read and one publish.

use std::collections::{BTreeMap, BTreeSet};

use buzz_core::kind::KIND_PROJECT;
use serde_json::{json, Map, Value};

use super::sidebar_sync::{self, channel_ids, now, Lane};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::{SidebarChannelFlagCmd, SidebarProjectsCmd};

/// Entry cap shared with desktop's star and mute stores.
const MAX_ENTRIES: usize = 500;

#[derive(Clone, Copy)]
enum Ids {
    Channels,
    Projects,
}

/// How one document differs from the others.
pub(crate) struct Spec {
    lane: Lane,
    collection: &'static str,
    flag: &'static str,
    ids: Ids,
    /// Oldest flags are evicted past this many, with clears first at equal age.
    max_entries: Option<usize>,
}

pub(crate) const STARS: Spec = Spec {
    lane: Lane {
        d_tag: "channel-stars",
        label: "channel stars",
    },
    collection: "channels",
    flag: "starred",
    ids: Ids::Channels,
    max_entries: Some(MAX_ENTRIES),
};

pub(crate) const MUTES: Spec = Spec {
    lane: Lane {
        d_tag: "channel-mutes",
        label: "channel mutes",
    },
    collection: "channels",
    flag: "muted",
    ids: Ids::Channels,
    max_entries: Some(MAX_ENTRIES),
};

pub(crate) const PROJECTS: Spec = Spec {
    lane: Lane {
        d_tag: "project-sidebar-membership",
        label: "project sidebar membership",
    },
    collection: "projects",
    flag: "selected",
    ids: Ids::Projects,
    max_entries: None,
};

impl From<SidebarChannelFlagCmd> for Action {
    fn from(cmd: SidebarChannelFlagCmd) -> Self {
        match cmd {
            SidebarChannelFlagCmd::List => Action::List,
            SidebarChannelFlagCmd::Add { channels } => Action::Set {
                ids: channels,
                on: true,
            },
            SidebarChannelFlagCmd::Remove { channels } => Action::Set {
                ids: channels,
                on: false,
            },
        }
    }
}

impl From<SidebarProjectsCmd> for Action {
    fn from(cmd: SidebarProjectsCmd) -> Self {
        match cmd {
            SidebarProjectsCmd::List => Action::List,
            SidebarProjectsCmd::Add { projects } => Action::Set {
                ids: projects,
                on: true,
            },
            SidebarProjectsCmd::Remove { projects } => Action::Set {
                ids: projects,
                on: false,
            },
        }
    }
}

/// What a flag command asks for; ids are validated and canonicalised by `run`.
pub(crate) enum Action {
    List,
    Set { ids: Vec<String>, on: bool },
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    on: bool,
    updated_at: f64,
}

type Store = BTreeMap<String, Entry>;

fn valid_entry(spec: &Spec, key: &str, value: &Value) -> Option<Entry> {
    if matches!(spec.ids, Ids::Projects) && key.is_empty() {
        return None;
    }
    let object = value.as_object()?;
    let updated_at = object.get("updatedAt")?.as_f64().filter(|n| *n >= 0.0)?;
    Some(Entry {
        on: object.get(spec.flag)?.as_bool()?,
        updated_at,
    })
}

// Preserve wire timestamps for untouched entries and compare in one unit.
fn timestamp_millis(spec: &Spec, updated_at: f64) -> f64 {
    if matches!(spec.ids, Ids::Channels) && (1e9..1e10).contains(&updated_at) {
        updated_at * 1000.0
    } else {
        updated_at
    }
}

/// Legacy desktop unions its cache with the new record and keeps the newest
/// entries by timestamp and ID. A protected add must survive that union.
fn legacy_desktop_would_trim_add(spec: &Spec, store: &Store, changed: &[String]) -> bool {
    let Some(max) = spec.max_entries else {
        return false;
    };
    if store.len() <= max {
        return false;
    }
    let mut by_age: Vec<_> = store.iter().collect();
    by_age.sort_by(|(left_id, left), (right_id, right)| {
        timestamp_millis(spec, left.updated_at)
            .total_cmp(&timestamp_millis(spec, right.updated_at))
            .then_with(|| left_id.cmp(right_id))
    });
    by_age[..store.len() - max]
        .iter()
        .any(|(id, _)| changed.contains(id))
}

/// Keep the current edit and preserve clears newer than active flags.
fn bound(spec: &Spec, store: &mut Store, keep: &[String]) {
    let Some(max) = spec.max_entries else { return };
    if store.len() <= max {
        return;
    }
    let keep: BTreeSet<_> = keep.iter().collect();
    let mut entries: Vec<_> = std::mem::take(store).into_iter().collect();
    entries.sort_by(|(left_id, left), (right_id, right)| {
        keep.contains(left_id)
            .cmp(&keep.contains(right_id))
            .then_with(|| {
                timestamp_millis(spec, left.updated_at)
                    .total_cmp(&timestamp_millis(spec, right.updated_at))
            })
            .then_with(|| left.on.cmp(&right.on))
            .then_with(|| left_id.cmp(right_id))
    });
    store.extend(entries.split_off(entries.len() - max));
}

/// Validate the complete star/mute head before using it as a write base.
fn decode(spec: &Spec, json: &Value) -> Option<Store> {
    if let (Ids::Projects, Some(addresses)) = (spec.ids, json.as_array()) {
        return Some(
            addresses
                .iter()
                .filter_map(Value::as_str)
                .filter(|address| !address.is_empty())
                .map(|address| {
                    (
                        address.to_string(),
                        Entry {
                            on: true,
                            updated_at: 0.0,
                        },
                    )
                })
                .collect(),
        );
    }
    let object = json.as_object()?;
    if object.get("version")?.as_f64()? != 1.0 {
        return None;
    }
    let entries = object.get(spec.collection)?.as_object()?;
    let mut store = Store::new();
    for (key, value) in entries {
        match valid_entry(spec, key, value) {
            Some(entry) => {
                store.insert(key.clone(), entry);
            }
            None if matches!(spec.ids, Ids::Channels) => return None,
            None => (),
        }
    }
    Some(store)
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n < 9_007_199_254_740_992.0 {
        json!(n as u64)
    } else {
        json!(n)
    }
}

fn encode(spec: &Spec, store: &Store) -> Value {
    let entries: Map<String, Value> = store
        .iter()
        .map(|(id, entry)| {
            (
                id.clone(),
                json!({spec.flag: entry.on, "updatedAt": number(entry.updated_at)}),
            )
        })
        .collect();
    json!({"version": 1, spec.collection: entries})
}

/// `30621:<owner>:<d>` with a lowercase owner, as desktop builds project
/// addresses.
fn project_addresses(addresses: &[String]) -> Result<Vec<String>, CliError> {
    addresses
        .iter()
        .map(|address| {
            let invalid = || CliError::Usage(format!("invalid project address: {address}"));
            let mut parts = address.trim().splitn(3, ':');
            let (Some(kind), Some(owner), Some(d)) = (parts.next(), parts.next(), parts.next())
            else {
                return Err(invalid());
            };
            let owner_ok = owner.len() == 64 && owner.bytes().all(|b| b.is_ascii_hexdigit());
            if kind != KIND_PROJECT.to_string() || !owner_ok || d.is_empty() {
                return Err(invalid());
            }
            Ok(format!("{kind}:{}:{d}", owner.to_ascii_lowercase()))
        })
        .collect()
}

fn canonical_ids(spec: &Spec, ids: &[String]) -> Result<Vec<String>, CliError> {
    match spec.ids {
        Ids::Channels => channel_ids(ids),
        Ids::Projects => project_addresses(ids),
    }
}

/// Sets `ids` to `on`, stamping each changed entry strictly newer than the one
/// it replaces so the edit wins every client's merge. Returns the changed ids;
/// an id already in the requested state, or absent when switching off, is left
/// alone.
fn set_flags(spec: &Spec, store: &mut Store, ids: &[String], on: bool) -> Vec<String> {
    let clock = now().as_millis() as f64;
    let mut changed = Vec::new();
    for id in ids {
        let current = store.get(id);
        if current.map_or(!on, |entry| entry.on == on) {
            continue;
        }
        let updated_at = current.map_or(clock, |entry| {
            clock.max(timestamp_millis(spec, entry.updated_at) + 1.0)
        });
        store.insert(id.clone(), Entry { on, updated_at });
        changed.push(id.clone());
    }
    changed
}

async fn fetch_store(spec: &Spec, client: &BuzzClient) -> Result<(u64, Store), CliError> {
    let head = sidebar_sync::fetch_head(client, spec.lane, |json, _| decode(spec, json)).await?;
    Ok((head.created_at, head.doc.unwrap_or_default()))
}

pub(crate) async fn run(
    spec: &Spec,
    action: Action,
    client: &BuzzClient,
) -> Result<Value, CliError> {
    match action {
        Action::List => {
            let (_, store) = fetch_store(spec, client).await?;
            let on: Vec<&String> = store
                .iter()
                .filter(|(_, entry)| entry.on)
                .map(|(id, _)| id)
                .collect();
            Ok(json!({spec.collection: on}))
        }
        Action::Set { ids, on } => {
            let ids = canonical_ids(spec, &ids)?;
            let (head_created_at, mut store) = fetch_store(spec, client).await?;
            let changed = set_flags(spec, &mut store, &ids, on);
            if changed.is_empty() {
                return Ok(sidebar_sync::unchanged());
            }
            if on && legacy_desktop_would_trim_add(spec, &store, &changed) {
                return Err(CliError::Usage(format!(
                    "{} add would be dropped by legacy desktop at the {MAX_ENTRIES}-entry limit",
                    spec.lane.label
                )));
            }
            let required = if on { &ids } else { &changed };
            bound(spec, &mut store, required);
            if required.iter().any(|id| !store.contains_key(id)) {
                return Err(CliError::Usage(format!(
                    "{} keeps at most {MAX_ENTRIES} entries; pass fewer ids",
                    spec.lane.label
                )));
            }
            let plaintext = serde_json::to_string(&encode(spec, &store)).map_err(|e| {
                CliError::Other(format!("failed to encode {}: {e}", spec.lane.label))
            })?;
            sidebar_sync::publish(client, spec.lane, plaintext, head_created_at).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::sidebar_sync::test_support::{
        published, published_count, seed_at, setup, Shared,
    };
    use super::*;
    use crate::credentials::Credentials;
    use nostr::Keys;

    const C1: &str = "11111111-1111-4111-8111-111111111111";
    const C2: &str = "22222222-2222-4222-8222-222222222222";
    const C3: &str = "abcdef01-2345-4678-89ab-cdef01234567";
    const OWNER: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

    fn set(ids: &[&str], on: bool) -> Action {
        Action::Set {
            ids: ids.iter().map(|id| id.to_string()).collect(),
            on,
        }
    }

    fn seed(
        client: &BuzzClient,
        relay: &Shared,
        spec: &Spec,
        doc: Value,
        created_at: u64,
    ) -> Value {
        seed_at(client, relay, spec.lane, &doc.to_string(), created_at)
    }

    fn entries(count: usize, flag: &str, first_updated_at: u64) -> Value {
        let map: Map<String, Value> = (0..count)
            .map(|i| {
                let id = format!("{i:036}");
                (
                    id,
                    json!({flag: true, "updatedAt": first_updated_at + i as u64}),
                )
            })
            .collect();
        Value::Object(map)
    }

    #[tokio::test]
    async fn stars_and_mutes_sync_desktop_format() {
        for spec in [&STARS, &MUTES] {
            let (client, relay) = setup().await;
            let list = |client| run(spec, Action::List, client);
            assert_eq!(list(&client).await.unwrap(), json!({"channels": []}));

            // Two channels land in one publish; an uppercase id is canonicalised.
            let before = now().as_millis() as u64;
            run(spec, set(&[C1, &C3.to_uppercase()], true), &client)
                .await
                .unwrap();
            assert_eq!(published_count(&relay), 1);
            let (event, doc) = published(&client, &relay);
            assert_eq!(event.kind.as_u16(), 30078);
            assert!(event
                .tags
                .iter()
                .any(|t| t.as_slice() == ["d", spec.lane.d_tag]));
            assert!(event
                .tags
                .iter()
                .any(|t| t.as_slice() == ["t", spec.lane.d_tag]));
            assert_eq!(doc["version"], json!(1));
            assert_eq!(doc["channels"][C3][spec.flag], json!(true));
            let stamp = doc["channels"][C1]["updatedAt"].as_u64().unwrap();
            assert!(
                stamp >= before && stamp < before + 60_000,
                "milliseconds: {stamp}"
            );
            assert_eq!(list(&client).await.unwrap(), json!({"channels": [C1, C3]}));

            // Repeats and clearing an unknown channel publish nothing.
            for action in [set(&[C1], true), set(&[C2], false)] {
                let again = run(spec, action, &client).await.unwrap();
                assert_eq!(again["message"], json!("unchanged"));
            }
            assert_eq!(published_count(&relay), 1);

            // Clearing keeps a `false` entry, as desktop does, so the clear syncs.
            run(spec, set(&[C1], false), &client).await.unwrap();
            let (_, doc) = published(&client, &relay);
            assert_eq!(doc["channels"][C1][spec.flag], json!(false));
            assert_eq!(list(&client).await.unwrap(), json!({"channels": [C3]}));

            // Invalid input fails before any publish.
            let err = run(spec, set(&[C2, "nope"], true), &client)
                .await
                .unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{err:?}");
            assert_eq!(published_count(&relay), 2);
        }
    }

    #[tokio::test]
    async fn an_edit_beats_the_entry_it_replaces_and_follows_the_head() {
        let (client, relay) = setup().await;
        let future = now().as_secs() + 1_000;
        // Legacy seconds ahead of our clock, alongside a millisecond entry.
        seed(
            &client,
            &relay,
            &STARS,
            json!({"version": 1.0, "channels": {
                C1: {"starred": false, "updatedAt": future},
                C2: {"starred": true, "updatedAt": (future - 1) * 1000},
            }}),
            future,
        );
        run(&STARS, set(&[C1], true), &client).await.unwrap();
        let (event, doc) = published(&client, &relay);
        assert_eq!(event.created_at.as_secs(), future + 1);
        assert_eq!(doc["channels"][C1]["updatedAt"], json!(future * 1000 + 1));
        assert_eq!(doc["channels"][C2]["updatedAt"], json!((future - 1) * 1000));
        assert_eq!(doc["channels"].as_object().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn refuses_an_unreadable_head_and_a_dominated_write() {
        for doc in [
            json!({"version": 2, "channels": {}}),
            json!({"version": 1}),
            json!({"version": 1, "channels": []}),
            json!({"version": 1, "channels": {C1: {"starred": "yes", "updatedAt": 5}}}),
            json!(["not", "a", "store"]),
            json!("text"),
        ] {
            let (client, relay) = setup().await;
            seed(&client, &relay, &STARS, doc, 10);
            assert!(run(&STARS, Action::List, &client).await.is_err());
            assert!(run(&STARS, set(&[C1], true), &client).await.is_err());
            assert_eq!(published_count(&relay), 1);
        }

        let (client, relay) = setup().await;
        let mut head = seed(&client, &relay, &MUTES, json!({"version": 1}), 10);
        head["content"] = json!("tampered");
        relay.lock().unwrap().events = vec![head];
        let err = run(&MUTES, set(&[C1], true), &client).await.unwrap_err();
        assert!(matches!(err, CliError::Other(_)), "{err:?}");

        // A head with a forged signature, or from another author, is refused
        // even when the relay returns it.
        let (client, relay) = setup().await;
        let mut forged = seed(&client, &relay, &MUTES, json!({"version": 1}), 10);
        forged["sig"] = json!("0".repeat(128));
        relay.lock().unwrap().events = vec![forged];
        relay.lock().unwrap().leaky = true;
        let refused = run(&MUTES, set(&[C1], true), &client).await.unwrap_err();
        assert!(
            refused.to_string().contains("fails verification"),
            "{refused}"
        );
        let (client, relay) = setup().await;
        let other =
            BuzzClient::new(String::new(), Credentials::from_keys(Keys::generate())).unwrap();
        seed(&other, &relay, &MUTES, json!({"version": 1}), 10);
        relay.lock().unwrap().leaky = true;
        let refused = run(&MUTES, set(&[C1], true), &client).await.unwrap_err();
        assert!(
            refused.to_string().contains("fails verification"),
            "{refused}"
        );
        assert_eq!(published_count(&relay), 1);

        // Another author's head is never read as ours.
        let (client, relay) = setup().await;
        let other =
            BuzzClient::new(String::new(), Credentials::from_keys(Keys::generate())).unwrap();
        seed(
            &other,
            &relay,
            &MUTES,
            json!({"version": 1, "channels": entries(1, "muted", 1)}),
            10,
        );
        let list = run(&MUTES, Action::List, &client).await.unwrap();
        assert_eq!(list, json!({"channels": []}));

        relay.lock().unwrap().duplicate = true;
        let err = run(&MUTES, set(&[C1], true), &client).await.unwrap_err();
        assert!(matches!(err, CliError::Conflict(_)), "{err:?}");
    }

    #[tokio::test]
    async fn an_unrelated_publish_keeps_a_seconds_writer_compatible() {
        for spec in [&STARS, &MUTES] {
            let (client, relay) = setup().await;
            let seconds = now().as_secs();
            seed(
                &client,
                &relay,
                spec,
                json!({"version": 1, "channels": {C1: {spec.flag: true, "updatedAt": seconds - 1}}}),
                seconds,
            );
            run(spec, set(&[C2], true), &client).await.unwrap();
            let (published_event, mut doc) = published(&client, &relay);
            let original = doc["channels"][C1]["updatedAt"].as_u64().unwrap();
            assert_eq!(original, seconds - 1);
            assert!(seconds > original, "a seconds writer must be able to win");
            doc["channels"][C1] = json!({spec.flag: false, "updatedAt": seconds});
            seed(
                &client,
                &relay,
                spec,
                doc,
                published_event.created_at.as_secs() + 1,
            );
            run(spec, set(&[C3], true), &client).await.unwrap();
            let (_, doc) = published(&client, &relay);
            assert_eq!(
                doc["channels"][C1],
                json!({spec.flag: false, "updatedAt": seconds})
            );
        }
    }

    #[tokio::test]
    async fn keeps_the_newest_500_entries() {
        let (client, relay) = setup().await;
        seed(
            &client,
            &relay,
            &STARS,
            json!({"version": 1, "channels": entries(500, "starred", 100)}),
            10,
        );
        run(&STARS, set(&[C1], true), &client).await.unwrap();
        let (_, doc) = published(&client, &relay);
        let channels = doc["channels"].as_object().unwrap();
        assert_eq!(channels.len(), 500);
        assert!(channels.contains_key(C1));
        assert!(!channels.contains_key(&format!("{:036}", 0)));
        assert!(channels.contains_key(&format!("{:036}", 1)));

        // An edit that would evict its own entries is refused.
        let before = published_count(&relay);
        let ids: Vec<String> = (0..501)
            .map(|i| format!("ffffffff-0000-4000-8000-{i:012}"))
            .collect();
        let err = run(&STARS, Action::Set { ids, on: true }, &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        assert_eq!(published_count(&relay), before);
    }

    #[tokio::test]
    async fn refuses_an_add_that_legacy_desktop_would_trim_after_merging() {
        for spec in [&STARS, &MUTES] {
            let (client, relay) = setup().await;
            let time = now().as_secs();
            let mut channels = entries(500, spec.flag, time + 600);
            channels[format!("{:036}", 0)] =
                json!({spec.flag: true, "updatedAt": (time + 599) * 1000});
            seed(
                &client,
                &relay,
                spec,
                json!({"version": 1, "channels": channels}),
                time,
            );

            let err = run(spec, set(&[C1], true), &client).await.unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{err:?}");
            assert_eq!(published_count(&relay), 1);
        }
    }

    #[tokio::test]
    async fn lists_all_active_flags_in_an_oversized_head() {
        for spec in [&STARS, &MUTES] {
            let (client, relay) = setup().await;
            let mut channels = entries(500, spec.flag, 100);
            channels[C2] = json!({spec.flag: true, "updatedAt": 99});
            seed(
                &client,
                &relay,
                spec,
                json!({"version": 1, "channels": channels}),
                10,
            );

            let list = run(spec, Action::List, &client).await.unwrap();
            let ids = list["channels"].as_array().unwrap();
            assert_eq!(ids.len(), 501);
            assert!(ids.contains(&json!(C2)));
            assert_eq!(
                run(spec, set(&[C2], true), &client).await.unwrap(),
                sidebar_sync::unchanged()
            );
            assert_eq!(published_count(&relay), 1);
        }
    }

    #[tokio::test]
    async fn repairs_oversized_flags_with_mixed_timestamps_and_keeps_the_edit() {
        for spec in [&STARS, &MUTES] {
            let (client, relay) = setup().await;
            let time = now().as_secs() - 100;
            let older_clear = "old-clear";
            let newer_clear = "new-clear";
            let mut channels = entries(498, spec.flag, (time + 10) * 1000)
                .as_object()
                .unwrap()
                .clone();
            channels.insert(
                older_clear.into(),
                json!({spec.flag: false, "updatedAt": time * 1000}),
            );
            channels.insert(
                newer_clear.into(),
                json!({spec.flag: false, "updatedAt": time + 1}),
            );
            let active = format!("{:036}", 0);
            channels.insert(C1.into(), json!({spec.flag: true, "updatedAt": time + 2}));
            seed(
                &client,
                &relay,
                spec,
                json!({"version": 1, "channels": channels}),
                time,
            );

            // A changed flag repairs the head and prunes the oldest entries.
            run(spec, set(&[C3], true), &client).await.unwrap();
            let (_, doc) = published(&client, &relay);
            let channels = doc["channels"].as_object().unwrap();
            assert_eq!(channels.len(), 500);
            assert!(!channels.contains_key(older_clear));
            assert!(!channels.contains_key(newer_clear));
            assert!(channels.contains_key(&active));
            assert_eq!(channels[C1]["updatedAt"], json!(time + 2));
            assert_eq!(channels[C3][spec.flag], true);

            // Reintroduce two clears with mixed units, with one excess entry.
            let mut channels = channels.clone();
            channels.remove(&active);
            channels.insert(
                older_clear.into(),
                json!({spec.flag: false, "updatedAt": time * 1000}),
            );
            channels.insert(
                newer_clear.into(),
                json!({spec.flag: false, "updatedAt": time + 1}),
            );
            seed(
                &client,
                &relay,
                spec,
                json!({"version": 1, "channels": channels}),
                now().as_secs() + 3,
            );
            run(spec, set(&[C1], false), &client).await.unwrap();
            let (_, doc) = published(&client, &relay);
            let channels = doc["channels"].as_object().unwrap();
            assert_eq!(channels.len(), 500);
            assert!(!channels.contains_key(older_clear));
            assert_eq!(channels[newer_clear]["updatedAt"], json!(time + 1));
            assert_eq!(channels[C1][spec.flag], json!(false));
        }
    }

    #[tokio::test]
    async fn oversized_no_ops_publish_nothing_and_a_new_clear_survives() {
        for spec in [&STARS, &MUTES] {
            let (client, relay) = setup().await;
            let time = now().as_secs();
            let mut channels = entries(499, spec.flag, (time + 60) * 1000)
                .as_object()
                .unwrap()
                .clone();
            channels.insert(
                C1.into(),
                json!({spec.flag: false, "updatedAt": (time + 120) * 1000}),
            );
            channels.insert(
                C2.into(),
                json!({spec.flag: true, "updatedAt": (time + 60) * 1000}),
            );
            seed(
                &client,
                &relay,
                spec,
                json!({"version": 1, "channels": channels}),
                time,
            );
            for action in [set(&[C1], false), set(&[C2], true)] {
                assert_eq!(
                    run(spec, action, &client).await.unwrap(),
                    sidebar_sync::unchanged()
                );
            }
            assert_eq!(published_count(&relay), 1);

            // The new clear is older than every competing flag in this
            // skewed snapshot. The current edit must survive anyway.
            channels.insert(C1.into(), json!({spec.flag: true, "updatedAt": time - 1}));
            seed(
                &client,
                &relay,
                spec,
                json!({"version": 1, "channels": channels}),
                time + 1,
            );
            run(spec, set(&[C1], false), &client).await.unwrap();
            let (_, doc) = published(&client, &relay);
            let channels = doc["channels"].as_object().unwrap();
            assert_eq!(channels.len(), 500);
            assert_eq!(channels[C1][spec.flag], false);
        }
    }

    #[tokio::test]
    async fn projects_use_milliseconds_and_keep_every_entry() {
        let (client, relay) = setup().await;
        let address = format!("30621:{}:my-project", OWNER.to_uppercase());
        let canonical = format!("30621:{OWNER}:my-project");
        let before = now().as_millis() as u64;
        run(&PROJECTS, set(&[&address], true), &client)
            .await
            .unwrap();
        let (event, doc) = published(&client, &relay);
        assert!(event
            .tags
            .iter()
            .any(|t| t.as_slice() == ["d", "project-sidebar-membership"]));
        assert!(event
            .tags
            .iter()
            .any(|t| t.as_slice() == ["t", "project-sidebar-membership"]));
        let entry = &doc["projects"][&canonical];
        assert_eq!(entry["selected"], json!(true));
        assert!(entry["updatedAt"].as_u64().unwrap() >= before);
        assert_eq!(
            run(&PROJECTS, Action::List, &client).await.unwrap(),
            json!({"projects": [canonical]})
        );
        let again = run(&PROJECTS, set(&[&canonical], true), &client)
            .await
            .unwrap();
        assert_eq!(again["message"], json!("unchanged"));
        run(&PROJECTS, set(&[&canonical], false), &client)
            .await
            .unwrap();
        let (_, doc) = published(&client, &relay);
        assert_eq!(doc["projects"][&canonical]["selected"], json!(false));

        for bad in [
            "my-project",
            &format!("30617:{OWNER}:x"),
            &format!("30621:{}:x", "zz".repeat(32)),
            &format!("30621:{OWNER}:"),
        ] {
            let err = run(&PROJECTS, set(&[bad], true), &client)
                .await
                .unwrap_err();
            assert!(matches!(err, CliError::Usage(_)), "{bad}: {err:?}");
        }

        // The legacy address array reads as selected; a document without a
        // collection is unreadable rather than empty.
        let (client, relay) = setup().await;
        seed(&client, &relay, &PROJECTS, json!(["a:b:c", ""]), 10);
        let list = run(&PROJECTS, Action::List, &client).await.unwrap();
        assert_eq!(list, json!({"projects": ["a:b:c"]}));
        seed(&client, &relay, &PROJECTS, json!({"version": 1}), 20);
        assert!(run(&PROJECTS, Action::List, &client).await.is_err());

        // No eviction: 520 entries survive a write, and the size cap is an error.
        let (client, relay) = setup().await;
        seed(
            &client,
            &relay,
            &PROJECTS,
            json!({"version": 1, "projects": entries(520, "selected", 1)}),
            10,
        );
        run(&PROJECTS, set(&[&canonical], true), &client)
            .await
            .unwrap();
        let (_, doc) = published(&client, &relay);
        assert_eq!(doc["projects"].as_object().unwrap().len(), 521);
        let (client, relay) = setup().await;
        seed(
            &client,
            &relay,
            &PROJECTS,
            json!({"version": 1, "projects": entries(800, "selected", 1)}),
            10,
        );
        let err = run(&PROJECTS, set(&[&canonical], true), &client)
            .await
            .unwrap_err();
        assert!(matches!(err, CliError::Usage(_)), "{err:?}");
        assert_eq!(published_count(&relay), 1);
    }
}
