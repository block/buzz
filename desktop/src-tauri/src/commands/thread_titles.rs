//! Shared thread titles (NIP-AR artifacts). The convention lives in
//! `buzz_sdk::thread_title` so the desktop and the `buzz` CLI write identical events.
//!
//! Artifact queries are HTTP-only on the relay, so these run in Rust rather
//! than through the TS WebSocket client.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use buzz_sdk_pkg::thread_title::{
    build_thread_title_revision, is_thread_title_head_conflict, normalize_thread_title,
    parse_thread_title, plan_thread_title, select_thread_title_head, thread_title_current_filter,
    thread_title_history_filter, thread_title_id, thread_title_list_filter,
    thread_titles_for_roots_filter, ThreadTitle, ThreadTitleHead, MAX_THREAD_TITLE_ROOTS_PER_QUERY,
};
use tauri::State;

use crate::{
    app_state::AppState,
    relay::{query_relay, submit_event},
};

/// One relay page; the Threads view is a "recents" list, not an archive.
/// Header and timeline titles don't read this list (see [`get_thread_titles`]).
const LIST_LIMIT: u32 = 1000;
/// Roots per activity query.
const ACTIVITY_ROOT_BATCH: usize = 100;
/// Newest replies read per activity query. Roots absent from this window last
/// saw a reply before every returned one, so the top of the list stays exact.
const ACTIVITY_REPLY_WINDOW: u32 = 500;
/// Stream and forum reply kinds (edits and diffs are not new activity).
const REPLY_KINDS: [u16; 3] = [9, 40002, 45003];
const CONFLICT_MESSAGE: &str =
    "conflict: someone else changed this thread's title while you were editing";

#[derive(Debug, serde::Serialize)]
pub struct ThreadTitleEntry {
    #[serde(flatten)]
    title: ThreadTitle,
    /// Unix seconds of the newest of: last reply, title revision.
    last_activity_at: u64,
}

#[derive(Debug, serde::Serialize)]
pub struct SetThreadTitleResult {
    title: Option<String>,
    /// The live revision after the write; `None` when the title is cleared.
    revision: Option<String>,
}

fn validate_root(root_id: &str) -> Result<String, String> {
    let root = root_id.to_ascii_lowercase();
    if root.len() != 64 || !root.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid thread root id: {root_id}"));
    }
    Ok(root)
}

/// `root -> newest reply time` for the given roots, from replies whose `e`
/// tags name them (NIP-10 replies tag their thread root).
fn last_reply_times(replies: &[nostr::Event], roots: &HashSet<&str>) -> HashMap<String, u64> {
    let mut latest: HashMap<String, u64> = HashMap::new();
    for reply in replies {
        let created_at = reply.created_at.as_secs();
        for tag in reply.tags.iter() {
            if let [name, value, ..] = tag.as_slice() {
                if name == "e" && roots.contains(value.as_str()) {
                    let entry = latest.entry(value.clone()).or_default();
                    *entry = (*entry).max(created_at);
                }
            }
        }
    }
    latest
}

/// Newest activity first; ties keep the relay's newest-revision-first order.
fn sort_by_activity(
    titles: Vec<ThreadTitle>,
    last_replies: &HashMap<String, u64>,
) -> Vec<ThreadTitleEntry> {
    let mut entries: Vec<_> = titles
        .into_iter()
        .map(|title| {
            let last_reply = last_replies.get(&title.root).copied().unwrap_or(0);
            ThreadTitleEntry {
                last_activity_at: last_reply.max(title.updated_at),
                title,
            }
        })
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.last_activity_at));
    entries
}

/// Every live thread title the user can read, most recently active first.
#[tauri::command]
pub async fn list_thread_titles(
    state: State<'_, AppState>,
) -> Result<Vec<ThreadTitleEntry>, String> {
    let events = query_relay(&state, &[thread_title_list_filter(None, LIST_LIMIT)]).await?;
    let titles: Vec<ThreadTitle> = events.iter().filter_map(parse_thread_title).collect();

    let mut roots_by_channel: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for title in &titles {
        roots_by_channel
            .entry(title.channel.as_str())
            .or_default()
            .push(title.root.as_str());
    }
    let all_roots: HashSet<&str> = titles.iter().map(|t| t.root.as_str()).collect();
    let mut replies = Vec::new();
    for (channel, roots) in &roots_by_channel {
        for batch in roots.chunks(ACTIVITY_ROOT_BATCH) {
            let filter = serde_json::json!({
                "kinds": REPLY_KINDS,
                "#e": batch,
                "#h": [channel],
                "limit": ACTIVITY_REPLY_WINDOW,
            });
            // Activity only refines the order; a failed lookup falls back to
            // the title's own timestamp instead of hiding the list.
            match query_relay(&state, &[filter]).await {
                Ok(events) => replies.extend(events),
                Err(e) => eprintln!("buzz-desktop: thread activity lookup failed: {e}"),
            }
        }
    }
    let last_replies = last_reply_times(&replies, &all_roots);
    Ok(sort_by_activity(titles, &last_replies))
}

/// Live titles among `events` for the requested roots. A title only counts
/// when its `d` is the one derived from its root, so a title naming a root it
/// wasn't derived from can't stand in for that thread's real one.
fn titles_for_roots(events: &[nostr::Event], roots: &HashSet<String>) -> Vec<ThreadTitle> {
    events
        .iter()
        .filter_map(parse_thread_title)
        .filter(|title| {
            let root = title.root.to_ascii_lowercase();
            roots.contains(&root) && title.artifact == thread_title_id(&root).to_string()
        })
        .collect()
}

/// Live titles for specific thread roots (untitled roots are omitted). Looks
/// each one up by its derived `d`, so it stays exact however many titles the
/// user can read.
#[tauri::command]
pub async fn get_thread_titles(
    root_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ThreadTitle>, String> {
    let roots: Vec<String> = root_ids
        .iter()
        .map(|id| validate_root(id))
        .collect::<Result<BTreeSet<_>, _>>()?
        .into_iter()
        .collect();
    let mut titles = Vec::new();
    for batch in roots.chunks(MAX_THREAD_TITLE_ROOTS_PER_QUERY) {
        let ids: Vec<&str> = batch.iter().map(String::as_str).collect();
        let events = query_relay(&state, &[thread_titles_for_roots_filter(&ids)]).await?;
        titles.extend(titles_for_roots(&events, &batch.iter().cloned().collect()));
    }
    Ok(titles)
}

async fn fetch_head(state: &AppState, root: &str) -> Result<Option<ThreadTitleHead>, String> {
    let current = query_relay(state, &[thread_title_current_filter(root)]).await?;
    if let Some(event) = current.first() {
        return Ok(Some(ThreadTitleHead::from_event(event)));
    }
    let history = query_relay(state, &[thread_title_history_filter(root)]).await?;
    Ok(select_thread_title_head(&history))
}

/// An edit is only valid against the title the user started from. `None`
/// means they started from an untitled thread (never titled, or cleared).
fn check_expected(head: Option<&ThreadTitleHead>, expected: Option<&str>) -> Result<(), String> {
    let live = head.filter(|h| !h.deleted).map(|h| h.event_id.as_str());
    if live == expected {
        Ok(())
    } else {
        Err(CONFLICT_MESSAGE.into())
    }
}

/// Set (`title = Some`) or clear (`title = None`) a thread's shared title,
/// provided it is still at `expected_revision`. Never overwrites a concurrent
/// edit: a lost race is reported as a conflict rather than retried.
#[tauri::command]
pub async fn set_thread_title(
    channel_id: String,
    root_id: String,
    title: Option<String>,
    expected_revision: Option<String>,
    state: State<'_, AppState>,
) -> Result<SetThreadTitleResult, String> {
    let channel = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let root = validate_root(&root_id)?;
    let desired = title
        .as_deref()
        .map(normalize_thread_title)
        .transpose()
        .map_err(|e| e.to_string())?;
    let head = fetch_head(&state, &root).await?;
    check_expected(head.as_ref(), expected_revision.as_deref())?;
    let plan = plan_thread_title(head.as_ref(), desired.as_deref());
    let Some(builder) = build_thread_title_revision(channel, &root, &plan, desired.as_deref())
        .map_err(|e| e.to_string())?
    else {
        return Ok(SetThreadTitleResult {
            revision: head.filter(|h| !h.deleted).map(|h| h.event_id),
            title: desired,
        });
    };
    match submit_event(builder, &state).await {
        Ok(response) => Ok(SetThreadTitleResult {
            revision: desired.as_ref().map(|_| response.event_id),
            title: desired,
        }),
        // The relay's `prev` check caught an edit that landed after our read.
        Err(e) if is_thread_title_head_conflict(&e) => Err(CONFLICT_MESSAGE.into()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn title(root: &str, updated_at: u64) -> ThreadTitle {
        ThreadTitle {
            title: root.into(),
            channel: "c".into(),
            root: root.into(),
            artifact: "a".into(),
            revision: "r".into(),
            author: "p".into(),
            updated_at,
        }
    }

    fn reply(created_at: u64, tags: &[[&str; 2]]) -> nostr::Event {
        EventBuilder::new(Kind::Custom(9), "reply")
            .tags(tags.iter().map(|t| Tag::parse(*t).unwrap()))
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn recent_replies_outrank_recent_titles() {
        let roots: HashSet<&str> = ["old-title-busy", "new-title-quiet", "no-replies"].into();
        let replies = last_reply_times(
            &[
                reply(400, &[["e", "old-title-busy"]]),
                // Nested reply: root plus a parent that isn't a titled root.
                reply(500, &[["e", "old-title-busy"], ["e", "some-parent"]]),
                reply(900, &[["e", "untitled-root"]]),
            ],
            &roots,
        );
        assert_eq!(replies.len(), 1);
        let sorted = sort_by_activity(
            vec![
                title("new-title-quiet", 300),
                title("old-title-busy", 100),
                title("no-replies", 200),
            ],
            &replies,
        );
        let order: Vec<_> = sorted.iter().map(|e| e.title.root.as_str()).collect();
        assert_eq!(order, ["old-title-busy", "new-title-quiet", "no-replies"]);
        assert_eq!(sorted[0].last_activity_at, 500);
    }

    #[test]
    fn edits_must_start_from_the_live_title() {
        let live = ThreadTitleHead {
            event_id: "rev-a".into(),
            deleted: false,
            title: Some("A".into()),
        };
        let cleared = ThreadTitleHead {
            event_id: "rev-d".into(),
            deleted: true,
            title: None,
        };
        assert!(check_expected(Some(&live), Some("rev-a")).is_ok());
        assert!(check_expected(None, None).is_ok());
        assert!(check_expected(Some(&cleared), None).is_ok());
        // Someone renamed, titled, or cleared it after the editor opened.
        assert!(check_expected(Some(&live), Some("rev-old")).is_err());
        assert!(check_expected(Some(&live), None).is_err());
        assert!(check_expected(Some(&cleared), Some("rev-a")).is_err());
    }

    fn title_event(root: &str, d: &str) -> nostr::Event {
        EventBuilder::new(Kind::Custom(45010), "t")
            .tags(
                [
                    ["type", "buzz.thread-title"],
                    ["d", d],
                    ["h", "c"],
                    ["root", root],
                    ["title", "t"],
                ]
                .iter()
                .map(|t| Tag::parse(*t).unwrap()),
            )
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn root_lookup_keeps_only_titles_derived_from_their_root() {
        let asked = "a".repeat(64);
        let other = "b".repeat(64);
        let roots: HashSet<String> = [asked.clone()].into();
        let id = |root: &str| thread_title_id(root).to_string();
        let found = titles_for_roots(
            &[
                title_event(&asked, &id(&asked)),
                // Names the asked root under another thread's id.
                title_event(&asked, &id(&other)),
                // A root nobody asked for.
                title_event(&other, &id(&other)),
            ],
            &roots,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].artifact, id(&asked));
    }

    #[test]
    fn root_ids_are_validated_and_lowercased() {
        assert_eq!(validate_root(&"A".repeat(64)).unwrap(), "a".repeat(64));
        assert!(validate_root("abc").is_err());
        assert!(validate_root(&"g".repeat(64)).is_err());
    }
}
