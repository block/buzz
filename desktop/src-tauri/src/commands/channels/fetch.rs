//! Relay-backed channel list computation for the channels commands.
//!
//! Split out of `channels.rs` to keep that file under the per-file line cap.
//! Owns the two-phase relay fetch (`fetch_channels`), its `DirectoryScope`
//! (member-only poll vs. the discovery superset), the paged directory cursor,
//! the not-modified hash, and the member-count collection. The Tauri commands
//! and channel writes stay in `channels.rs`.

use crate::{app_state::AppState, models::ChannelInfo, nostr_convert, relay::query_relay};

pub(super) const DIRECTORY_PAGE_SIZE: usize = 500;
// Keep this aligned with the relay's aggregate explicit-`#h` request bound.
// Each filter carries one channel so the relay can use its channel_id index.
const LAST_MESSAGE_QUERY_CHANNEL_BATCH_SIZE: usize = 128;
// Human-visible channel activity that drives sidebar Recent ordering. Keep this
// aligned with desktop/src/shared/constants/kinds.ts::CHANNEL_MESSAGE_EVENT_KINDS.
const CHANNEL_RECENCY_EVENT_KINDS: [u16; 4] = [9, 40002, 45001, 45003];

pub(super) fn advance_directory_cursor(filter: &mut serde_json::Value, page: &[nostr::Event]) {
    let last = page
        .last()
        .expect("a full relay page always has a last event");
    filter["until"] = serde_json::json!(last.created_at.as_secs());
    filter["before_id"] = serde_json::json!(last.id.to_hex());
}

/// Fetch every page for a historical relay filter using the relay's composite
/// `(until, before_id)` cursor. A timestamp-only cursor can skip rows when more
/// than one page of events shares the same second.
async fn query_relay_all(
    state: &AppState,
    mut filter: serde_json::Value,
) -> Result<Vec<nostr::Event>, String> {
    filter["limit"] = serde_json::json!(DIRECTORY_PAGE_SIZE);
    let mut all = Vec::new();

    loop {
        let page = query_relay(state, &[filter.clone()]).await?;
        let done = page.len() < DIRECTORY_PAGE_SIZE;

        if !done {
            advance_directory_cursor(&mut filter, &page);
        }

        all.extend(page);
        if done {
            return Ok(all);
        }
    }
}

/// Whether an open channel not yet in the real member set should still be
/// classified `is_member=true` via the pending-owner overlay. Pulled out of
/// `get_channels`'s open-channel branch so the exact `(d_tag, my_pubkey,
/// overlay) -> is_member` decision — including the identity binding that
/// keeps one identity's pending entry from covering another's — is directly
/// unit-testable without going through the async relay-backed command.
pub(super) fn classify_pending_owner(
    state: &AppState,
    my_pubkey: &str,
    d_tag: Option<&str>,
) -> bool {
    d_tag.is_some_and(|d| state.is_pending_owned_channel(my_pubkey, d))
}

// ── FNV-1a hash for the not-modified short-circuit ───────────────────────────

/// FNV-1a 64-bit hash over arbitrary bytes. Used in preference to
/// `std::collections::hash_map::DefaultHasher` because the standard library
/// does not guarantee cross-invocation stability.
fn fnv1a_64(data: &[u8]) -> u64 {
    const OFFSET: u64 = 14695981039346656037;
    const PRIME: u64 = 1099511628211;
    let mut hash = OFFSET;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Stable projection of `ChannelInfo` for hashing. Excludes `last_message_at`
/// so routine message traffic does not invalidate the not-modified short-circuit
/// for the channel list.
#[derive(serde::Serialize)]
struct ChannelInfoForHash<'a> {
    id: &'a str,
    name: &'a str,
    channel_type: &'a str,
    visibility: &'a str,
    description: &'a str,
    topic: &'a Option<String>,
    purpose: &'a Option<String>,
    member_count: i64,
    member_pubkeys: &'a Vec<String>,
    archived_at: &'a Option<String>,
    participants: &'a Vec<String>,
    participant_pubkeys: &'a Vec<String>,
    is_member: bool,
    ttl_seconds: &'a Option<i32>,
    ttl_deadline: &'a Option<String>,
}

/// Compute a stable 64-bit FNV-1a hash over the channel list, canonicalized
/// by sorting on channel id and excluding `last_message_at`. Returns a
/// 16-character lowercase hex string.
pub(super) fn compute_channels_hash(channels: &[ChannelInfo]) -> String {
    let mut sorted: Vec<&ChannelInfo> = channels.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));

    let projections: Vec<ChannelInfoForHash<'_>> = sorted
        .iter()
        .map(|c| ChannelInfoForHash {
            id: &c.id,
            name: &c.name,
            channel_type: &c.channel_type,
            visibility: &c.visibility,
            description: &c.description,
            topic: &c.topic,
            purpose: &c.purpose,
            member_count: c.member_count,
            member_pubkeys: &c.member_pubkeys,
            archived_at: &c.archived_at,
            participants: &c.participants,
            participant_pubkeys: &c.participant_pubkeys,
            is_member: c.is_member,
            ttl_seconds: &c.ttl_seconds,
            ttl_deadline: &c.ttl_deadline,
        })
        .collect();

    let canonical = serde_json::to_string(&projections).unwrap_or_default();
    format!("{:016x}", fnv1a_64(canonical.as_bytes()))
}

// ── Core fetch implementation ─────────────────────────────────────────────────

pub(super) fn last_message_filter(channel_id: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": CHANNEL_RECENCY_EVENT_KINDS,
        "#h": [channel_id],
        "limit": 1
    })
}

pub(super) fn last_message_filter_batches(
    filters: &[serde_json::Value],
) -> Vec<&[serde_json::Value]> {
    filters
        .chunks(LAST_MESSAGE_QUERY_CHANNEL_BATCH_SIZE)
        .collect()
}

async fn query_last_messages(
    state: &AppState,
    filters: &[serde_json::Value],
) -> Result<Vec<nostr::Event>, String> {
    let mut messages = Vec::with_capacity(filters.len());
    for batch in last_message_filter_batches(filters) {
        messages.extend(query_relay(state, batch).await?);
    }
    Ok(messages)
}

/// Whether `fetch_channels` includes the unbounded all-open directory scan.
///
/// The 60s channel poll uses [`DirectoryScope::MemberOnly`]: it resolves only
/// the channels the identity belongs to (plus its own not-yet-propagated
/// creations), so phase 2's fan-out is bounded by membership instead of the
/// entire relay. [`DirectoryScope::IncludeOpenDirectory`] additionally scans
/// every open channel — the discovery surfaces (channel browser, global
/// search) and onboarding need that superset, but the poll must not pay for it
/// on every tick.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectoryScope {
    MemberOnly,
    IncludeOpenDirectory,
}

/// Fetch the channel list from the relay at the requested [`DirectoryScope`].
/// Called by `get_channels` (member-only poll, wrapped with hash-based
/// short-circuit logic), `get_open_channel_directory` (discovery superset), and
/// `ensure_starter_channels` (which needs the raw open-inclusive list).
///
/// Relay round-trips run in two concurrent phases:
/// - Phase 1 (parallel): member-chain (kind:39002→kind:39000), the non-member
///   metadata source (pending-owned ids when member-only, else the all-open
///   kind:39000 scan), and the hidden-DM snapshot (kind:30622).
/// - Phase 2 (parallel): member counts (kind:39002 batch) and last-message
///   timestamps (bounded per-channel human-visible activity batches), fanned
///   out over the merged set. Member-count failures degrade to zero; timestamp
///   failures abort so cached recency is never replaced by a false
///   authoritative empty result.
pub(super) async fn fetch_channels(
    state: &AppState,
    scope: DirectoryScope,
) -> Result<Vec<ChannelInfo>, String> {
    #[cfg(debug_assertions)]
    let _profile_start = std::time::Instant::now();

    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    // Channels this identity created whose kind:39002 membership hasn't yet
    // propagated. Under member-only scope they are the only non-member
    // metadata we resolve, so a just-created channel stays visible without the
    // all-open scan (#1761). Read before the member chain runs; any that have
    // since become real members are harmlessly skipped during the merge.
    let pending_owned_ids = state.pending_owned_channel_ids(&my_pubkey);

    // Phase 1 — concurrent: member-chain (steps 1→2), the non-member metadata
    // source (step 3), and hidden-DM snapshot (step 6). No mutual dependencies.
    let (member_chain_result, open_meta_result, hidden_dms) = tokio::join!(
        // Steps 1+2: find the channels this identity belongs to, then fetch
        // their metadata events.
        async {
            // Step 1: kind:39002 events listing my pubkey as a member.
            let member_events = query_relay_all(
                state,
                serde_json::json!({"kinds": [39002], "#p": [&my_pubkey]}),
            )
            .await?;

            let mut member_channel_ids: Vec<String> = member_events
                .iter()
                .filter_map(|ev| {
                    ev.tags.iter().find_map(|t| {
                        let s = t.as_slice();
                        if s.len() >= 2 && s[0] == "d" {
                            Some(s[1].clone())
                        } else {
                            None
                        }
                    })
                })
                .collect();
            member_channel_ids.sort();
            member_channel_ids.dedup();

            // Real kind:39002 membership has landed — clear the pending-owner
            // overlay so a subsequent leave correctly flips `is_member` back
            // to false. See `AppState::pending_owned_channels`.
            for id in &member_channel_ids {
                state.clear_pending_owned_channel(&my_pubkey, id);
            }

            // Step 2: fetch channel metadata events (kind:39000) for member channels.
            // kind:39000 is addressable: exactly one event per `d` tag, so a limit
            // equal to the number of ids is both necessary and sufficient.
            let meta_events = if !member_channel_ids.is_empty() {
                query_relay(
                    state,
                    &[serde_json::json!({
                        "kinds": [39000],
                        "#d": &member_channel_ids,
                        "limit": member_channel_ids.len(),
                    })],
                )
                .await?
            } else {
                Vec::new()
            };

            Ok::<_, String>(meta_events)
        },
        // Step 3: non-member channel metadata (kind:39000).
        // - IncludeOpenDirectory: scan ALL open channels so the discovery
        //   surfaces can show joinable channels the user hasn't joined yet.
        // - MemberOnly: resolve only the pending-owned ids, keeping a
        //   just-created channel visible without the unbounded all-open scan.
        async {
            match scope {
                DirectoryScope::IncludeOpenDirectory => {
                    query_relay_all(state, serde_json::json!({"kinds": [39000]})).await
                }
                DirectoryScope::MemberOnly if !pending_owned_ids.is_empty() => {
                    query_relay(
                        state,
                        &[serde_json::json!({
                            "kinds": [39000],
                            "#d": &pending_owned_ids,
                            "limit": pending_owned_ids.len(),
                        })],
                    )
                    .await
                }
                DirectoryScope::MemberOnly => Ok(Vec::new()),
            }
        },
        // Step 6: NIP-DV hidden-DM snapshot. Tolerant — a failure means no DMs
        // are hidden rather than aborting the whole fetch.
        async {
            let events = query_relay(
                state,
                &[serde_json::json!({
                    "kinds": [buzz_core_pkg::kind::KIND_DM_VISIBILITY],
                    "#p": [&my_pubkey],
                    "limit": 1,
                })],
            )
            .await
            .unwrap_or_default();
            events
                .iter()
                .max_by_key(|e| e.created_at.as_secs())
                .map(|e| {
                    e.tags
                        .iter()
                        .filter_map(|t| {
                            let s = t.as_slice();
                            (s.len() >= 2 && s[0] == "h").then(|| s[1].clone())
                        })
                        .collect::<std::collections::HashSet<String>>()
                })
                .unwrap_or_default()
        },
    );

    #[cfg(debug_assertions)]
    let t_phase1 = _profile_start.elapsed();

    let meta_events = member_chain_result?;
    let open_meta_events = open_meta_result?;
    // hidden_dms is already a resolved HashSet (tolerant path above)

    // Merge: member channels (marked as member) + non-member channels (open
    // directory when included, else pending-owned) not already in the member set.
    let member_d_tags: std::collections::HashSet<String> = meta_events
        .iter()
        .filter_map(|ev| {
            ev.tags.iter().find_map(|t| {
                let s = t.as_slice();
                if s.len() >= 2 && s[0] == "d" {
                    Some(s[1].clone())
                } else {
                    None
                }
            })
        })
        .collect();

    let mut channels = Vec::with_capacity(meta_events.len() + open_meta_events.len());
    for ev in &meta_events {
        if let Ok(info) = nostr_convert::channel_info_from_event(ev, None, Some(true)) {
            channels.push(info);
        }
    }
    for ev in &open_meta_events {
        // Skip channels already included from the member set.
        let d_tag = ev.tags.iter().find_map(|t| {
            let s = t.as_slice();
            if s.len() >= 2 && s[0] == "d" {
                Some(s[1].clone())
            } else {
                None
            }
        });
        if let Some(ref d) = d_tag {
            if member_d_tags.contains(d) {
                continue;
            }
        }
        // The overlay (`AppState::pending_owned_channels`) marks channels this
        // identity just created via `create_channel` whose kind:39002 owner
        // membership hasn't propagated yet (#1761).
        let is_pending_owner = classify_pending_owner(state, &my_pubkey, d_tag.as_deref());
        if let Ok(info) = nostr_convert::channel_info_from_event(ev, None, Some(is_pending_owner)) {
            channels.push(info);
        }
    }

    // Phase 2 — concurrent: member counts (step 4) and last-message timestamps
    // (step 5). Member-count failures degrade to zero. Timestamp failures
    // abort this refresh so the frontend keeps its previous Recent ordering.
    let all_channel_ids: Vec<String> = channels.iter().map(|c| c.id.clone()).collect();
    if !all_channel_ids.is_empty() {
        let last_msg_filters: Vec<serde_json::Value> = all_channel_ids
            .iter()
            .map(|id| last_message_filter(id))
            .collect();

        // Bind both filter arrays before the join so their lifetimes cover
        // both branches of the concurrent pair.
        let member_count_filters = [serde_json::json!({
            "kinds": [39002],
            "#d": &all_channel_ids,
            "limit": all_channel_ids.len(),
        })];
        let (members_result, message_result) = tokio::join!(
            // Step 4: batch-fetch kind:39002 for member counts.
            query_relay(state, &member_count_filters),
            // Step 5: preserve one indexed filter per channel while keeping
            // every relay request within its aggregate explicit-channel cap.
            query_last_messages(state, &last_msg_filters),
        );
        // Message timestamps drive the user-selected Recent ordering. Unlike
        // member counts, a failed query must not masquerade as an authoritative
        // empty result and clear every cached timestamp in the frontend.
        let messages = message_result?;

        let membership = collect_members_by_channel(&members_result.unwrap_or_default());
        for channel in &mut channels {
            if let Some(info) = membership.get(&channel.id) {
                channel.member_count = info.count;
                channel.member_pubkeys = info.pubkeys.clone();
            }
        }

        let mut last_message_by_channel: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();
        for ev in &messages {
            if let Some(ch_id) = ev.tags.iter().find_map(|t| {
                let s = t.as_slice();
                (s.len() >= 2 && s[0] == "h").then(|| s[1].clone())
            }) {
                let ts = ev.created_at.as_secs();
                last_message_by_channel
                    .entry(ch_id)
                    .and_modify(|existing| {
                        if ts > *existing {
                            *existing = ts;
                        }
                    })
                    .or_insert(ts);
            }
        }
        for channel in &mut channels {
            if let Some(&ts) = last_message_by_channel.get(&channel.id) {
                channel.last_message_at = Some(nostr_convert::timestamp_to_iso(ts));
            }
        }
    }

    #[cfg(debug_assertions)]
    {
        let total = _profile_start.elapsed();
        eprintln!(
            "buzz-desktop: get_channels profile channels={} phase1(member_chain+open_meta+hidden_dm)={:?} phase2(member_counts+last_msg)={:?} total={:?}",
            channels.len(),
            t_phase1,
            total - t_phase1,
            total,
        );
    }

    // NIP-DV: drop DMs the viewer has hidden.
    if !hidden_dms.is_empty() {
        channels.retain(|c| c.channel_type != "dm" || !hidden_dms.contains(&c.id));
    }

    Ok(channels)
}

pub(super) struct ChannelMembership {
    pub(super) count: i64,
    pub(super) pubkeys: Vec<String>,
}

/// Build a `channel_id → membership` map from a batch of kind:39002 events.
/// Events without a `d` tag are skipped; member dedupe is delegated to
/// [`nostr_convert::channel_members_from_event`] so the parsing rules match the
/// per-channel `get_channel_members` path.
pub(super) fn collect_members_by_channel(
    events: &[nostr::Event],
) -> std::collections::HashMap<String, ChannelMembership> {
    let mut map: std::collections::HashMap<String, ChannelMembership> =
        std::collections::HashMap::with_capacity(events.len());
    for ev in events {
        let Some(d) = ev.tags.iter().find_map(|t| {
            let s = t.as_slice();
            (s.len() >= 2 && s[0] == "d").then(|| s[1].clone())
        }) else {
            continue;
        };
        let Ok(resp) = nostr_convert::channel_members_from_event(ev) else {
            continue;
        };
        let pubkeys: Vec<String> = resp.members.iter().map(|m| m.pubkey.clone()).collect();
        map.insert(
            d,
            ChannelMembership {
                count: pubkeys.len() as i64,
                pubkeys,
            },
        );
    }
    map
}
