use std::collections::{HashMap, HashSet};

use nostr::Event;
use serde_json::Value;

use crate::models::{SearchUsersResponse, UserSearchResultInfo};

use super::profile_valid_oa_owner_pubkey;

/// Convert a single kind:0 event to a [`UserSearchResultInfo`].
pub fn user_search_result_from_event(ev: &Event) -> UserSearchResultInfo {
    let v: Value = serde_json::from_str(&ev.content).unwrap_or(Value::Null);
    let owner_pubkey = profile_valid_oa_owner_pubkey(ev);
    UserSearchResultInfo {
        pubkey: ev.pubkey.to_hex(),
        display_name: v
            .get("display_name")
            .and_then(Value::as_str)
            .or_else(|| v.get("name").and_then(Value::as_str))
            .map(str::to_string),
        avatar_url: v.get("picture").and_then(Value::as_str).map(str::to_string),
        nip05_handle: v.get("nip05").and_then(Value::as_str).map(str::to_string),
        is_agent: owner_pubkey.is_some(),
        owner_pubkey,
    }
}

/// Convert kind:0 events (e.g. from a NIP-50 search) to [`SearchUsersResponse`].
pub fn search_users_from_events(events: &[Event]) -> SearchUsersResponse {
    let users = events.iter().map(user_search_result_from_event).collect();
    SearchUsersResponse { users }
}

/// Convert a default kind:0 page to user-search results for empty-query pickers.
pub fn list_user_search_results(events: &[Event], limit: usize) -> SearchUsersResponse {
    if limit == 0 {
        return SearchUsersResponse { users: Vec::new() };
    }

    let mut latest_by_pubkey: HashMap<String, (nostr::Timestamp, UserSearchResultInfo)> =
        HashMap::new();

    for ev in events {
        if ev.kind.as_u16() != 0 {
            continue;
        }

        let info = user_search_result_from_event(ev);
        let key = info.pubkey.to_lowercase();
        let should_replace = match latest_by_pubkey.get(&key) {
            Some((created_at, _)) => ev.created_at > *created_at,
            None => true,
        };
        if should_replace {
            latest_by_pubkey.insert(key, (ev.created_at, info));
        }
    }

    let mut users: Vec<UserSearchResultInfo> = latest_by_pubkey
        .into_values()
        .map(|(_, info)| info)
        .collect();
    users.sort_by_cached_key(|info| {
        let label = info
            .display_name
            .as_deref()
            .or(info.nip05_handle.as_deref())
            .unwrap_or(&info.pubkey)
            .to_lowercase();
        (label, info.pubkey.clone())
    });
    users.truncate(limit);

    SearchUsersResponse { users }
}

/// Rank and truncate kind:0 events from a NIP-50 search response for the
/// member-picker / DM-recipient autocomplete.
///
/// The relay returns results scored by Postgres FTS rank against the whole kind:0
/// JSON content blob. That ranking is fine as a recall mechanism but not as
/// final ordering — a user whose `display_name` *is* the query should always
/// rank above someone whose `about` happens to mention it. We re-rank with a
/// small deterministic scoring function:
///
/// - exact match (case-insensitive)   > prefix match > substring match
/// - field priority: display_name (or name) > nip05 > pubkey hex
///
/// `limit` clamps the output. Pubkey de-duplication keeps only the
/// highest-scoring result per pubkey (the relay should already return one doc
/// per event id, and kind:0 is a NIP-16 replaceable event so stale rows are
/// soft-deleted in the DB and filtered out before reaching us — this is
/// defense in depth in case both somehow slip through).
pub fn rank_user_search_results(
    events: &[Event],
    query: &str,
    limit: usize,
) -> SearchUsersResponse {
    let q = query.trim().to_lowercase();
    if q.is_empty() || limit == 0 {
        return SearchUsersResponse { users: Vec::new() };
    }

    // (score, input_index) — input index is a stable tiebreaker preserving
    // the relay's relevance order for ties.
    let mut scored: Vec<(u32, usize, UserSearchResultInfo)> = Vec::with_capacity(events.len());

    for (idx, ev) in events.iter().enumerate() {
        // Defensive: NIP-50 may return kinds we didn't expect if the relay
        // doesn't honor the `kinds` filter under search. Skip non-kind:0.
        if ev.kind.as_u16() != 0 {
            continue;
        }

        let info = user_search_result_from_event(ev);
        let display = info.display_name.as_deref().unwrap_or("").to_lowercase();
        let nip05 = info.nip05_handle.as_deref().unwrap_or("").to_lowercase();
        let pubkey = info.pubkey.to_lowercase();

        let score = match_score(&q, &display, &nip05, &pubkey);
        if score == 0 {
            continue;
        }
        scored.push((score, idx, info));
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut seen: HashSet<String> = HashSet::new();
    let mut users: Vec<UserSearchResultInfo> = Vec::with_capacity(limit.min(scored.len()));
    for (_, _, info) in scored {
        if !seen.insert(info.pubkey.clone()) {
            continue;
        }
        users.push(info);
        if users.len() >= limit {
            break;
        }
    }

    SearchUsersResponse { users }
}

fn match_score(q: &str, display_name: &str, nip05: &str, pubkey_hex: &str) -> u32 {
    const DISPLAY_EXACT: u32 = 1000;
    const DISPLAY_PREFIX: u32 = 900;
    const DISPLAY_CONTAINS: u32 = 800;
    const NIP05_EXACT: u32 = 700;
    const NIP05_PREFIX: u32 = 600;
    const NIP05_CONTAINS: u32 = 500;
    const PUBKEY_PREFIX: u32 = 400;

    let score_field = |field: &str, exact: u32, prefix: u32, contains: u32| -> u32 {
        if field.is_empty() {
            0
        } else if field == q {
            exact
        } else if field.starts_with(q) {
            prefix
        } else if field.contains(q) {
            contains
        } else {
            0
        }
    };

    let display_score = score_field(
        display_name,
        DISPLAY_EXACT,
        DISPLAY_PREFIX,
        DISPLAY_CONTAINS,
    );
    let nip05_score = score_field(nip05, NIP05_EXACT, NIP05_PREFIX, NIP05_CONTAINS);
    let pubkey_score = if !pubkey_hex.is_empty() && pubkey_hex.starts_with(q) {
        PUBKEY_PREFIX
    } else {
        0
    };

    display_score.max(nip05_score).max(pubkey_score)
}

#[cfg(test)]
mod tests {
    use nostr::{Event, EventBuilder, Kind, Tag};

    use super::*;

    fn ev(kind: u16, content: &str, tags: Vec<Vec<&str>>) -> Event {
        let keys = nostr::Keys::generate();
        let tags: Vec<Tag> = tags
            .into_iter()
            .map(|t| Tag::parse(t.into_iter().map(str::to_string).collect::<Vec<_>>()).unwrap())
            .collect();
        EventBuilder::new(Kind::from_u16(kind), content)
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign")
    }

    fn oa_profile_event(content: &str) -> Event {
        let agent_keys = nostr::Keys::generate();
        let owner_keys = nostr::Keys::generate();
        let agent_pubkey = agent_keys.public_key();
        let tag_json = buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_pubkey, "")
            .expect("compute auth tag");
        let tag_values: Vec<String> = serde_json::from_str(&tag_json).expect("parse auth tag json");
        let auth_tag = Tag::parse(tag_values).expect("parse auth tag");

        EventBuilder::new(Kind::Metadata, content)
            .tags(vec![auth_tag])
            .sign_with_keys(&agent_keys)
            .expect("sign")
    }

    #[test]
    fn search_users_maps_each_event() {
        let e1 = ev(0, r#"{"name":"a"}"#, vec![]);
        let e2 = ev(0, r#"{"display_name":"B"}"#, vec![]);
        let r = search_users_from_events(&[e1, e2]);
        assert_eq!(r.users.len(), 2);
        assert_eq!(r.users[0].display_name.as_deref(), Some("a"));
        assert_eq!(r.users[1].display_name.as_deref(), Some("B"));
    }

    #[test]
    fn user_search_result_marks_valid_nip_oa_profile_as_agent() {
        let event = oa_profile_event(r#"{"display_name":"Mira"}"#);
        let owner_pubkey = event
            .tags
            .iter()
            .find_map(|tag| tag.as_slice().get(1).cloned())
            .expect("owner pubkey");
        let result = user_search_result_from_event(&event);

        assert_eq!(result.display_name.as_deref(), Some("Mira"));
        assert_eq!(result.owner_pubkey.as_deref(), Some(owner_pubkey.as_str()));
        assert!(result.is_agent);
    }

    #[test]
    fn user_search_result_ignores_invalid_nip_oa_auth_tag() {
        let owner = "a".repeat(64);
        let signature = "b".repeat(128);
        let event = ev(
            0,
            r#"{"display_name":"Not An Agent"}"#,
            vec![vec!["auth", &owner, "", &signature]],
        );
        let result = user_search_result_from_event(&event);

        assert!(!result.is_agent);
    }

    #[test]
    fn list_user_search_results_sorts_dedupes_and_limits() {
        let keys = nostr::Keys::generate();
        let old = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Zed"}"#)
            .custom_created_at(nostr::Timestamp::from(1000))
            .sign_with_keys(&keys)
            .unwrap();
        let latest = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Aaron"}"#)
            .custom_created_at(nostr::Timestamp::from(2000))
            .sign_with_keys(&keys)
            .unwrap();
        let bob = ev(0, r#"{"display_name":"Bob"}"#, vec![]);
        let note = ev(1, "Not a profile", vec![]);

        let r = list_user_search_results(&[old, bob, note, latest], 2);

        assert_eq!(r.users.len(), 2);
        assert_eq!(r.users[0].display_name.as_deref(), Some("Aaron"));
        assert_eq!(r.users[1].display_name.as_deref(), Some("Bob"));
    }

    #[test]
    fn rank_empty_query_returns_empty() {
        let e = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let r = rank_user_search_results(&[e], "  ", 10);
        assert!(r.users.is_empty());
    }

    #[test]
    fn rank_zero_limit_returns_empty() {
        let e = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let r = rank_user_search_results(&[e], "alice", 0);
        assert!(r.users.is_empty());
    }

    #[test]
    fn rank_skips_non_kind_zero_events() {
        let e = ev(1, "alice", vec![]);
        let r = rank_user_search_results(&[e], "alice", 10);
        assert!(r.users.is_empty());
    }

    #[test]
    fn rank_skips_results_with_no_name_or_nip05_match() {
        let e = ev(
            0,
            r#"{"display_name":"Bob","nip05":"bob@example.com","about":"I work with alice"}"#,
            vec![],
        );
        let r = rank_user_search_results(&[e], "alice", 10);
        assert!(r.users.is_empty());
    }

    #[test]
    fn rank_exact_display_name_beats_substring_match() {
        let exact = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let sub = ev(0, r#"{"display_name":"alice-the-second"}"#, vec![]);
        let r = rank_user_search_results(&[sub, exact], "alice", 10);
        assert_eq!(r.users.len(), 2);
        assert_eq!(r.users[0].display_name.as_deref(), Some("alice"));
        assert_eq!(r.users[1].display_name.as_deref(), Some("alice-the-second"));
    }

    #[test]
    fn rank_display_name_beats_nip05_match_of_same_tier() {
        let by_name = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let by_nip05 = ev(0, r#"{"display_name":"X","nip05":"alice@x.com"}"#, vec![]);
        let r = rank_user_search_results(&[by_nip05, by_name], "alice", 10);
        assert_eq!(r.users[0].display_name.as_deref(), Some("alice"));
    }

    #[test]
    fn rank_display_substring_still_beats_nip05_exact() {
        let by_name_sub = ev(0, r#"{"display_name":"my-alice-account"}"#, vec![]);
        let by_nip05_exact = ev(0, r#"{"display_name":"Bob","nip05":"alice"}"#, vec![]);
        let r = rank_user_search_results(&[by_nip05_exact, by_name_sub], "alice", 10);
        assert_eq!(r.users[0].display_name.as_deref(), Some("my-alice-account"));
    }

    #[test]
    fn rank_pubkey_prefix_match() {
        let e = ev(0, r#"{"display_name":"unrelated"}"#, vec![]);
        let prefix: String = e.pubkey.to_hex().chars().take(8).collect();
        let r = rank_user_search_results(std::slice::from_ref(&e), &prefix, 10);
        assert_eq!(r.users.len(), 1);
        assert_eq!(r.users[0].pubkey, e.pubkey.to_hex());
    }

    #[test]
    fn rank_dedupes_by_pubkey() {
        let keys = nostr::Keys::generate();
        let mk = |content: &str| {
            EventBuilder::new(Kind::from_u16(0), content)
                .sign_with_keys(&keys)
                .expect("sign")
        };
        let weak = mk(r#"{"display_name":"alice-old"}"#);
        let strong = mk(r#"{"display_name":"alice"}"#);
        let r = rank_user_search_results(&[weak, strong], "alice", 10);
        assert_eq!(r.users.len(), 1);
        assert_eq!(r.users[0].display_name.as_deref(), Some("alice"));
    }

    #[test]
    fn rank_respects_limit() {
        let events: Vec<Event> = (0..5)
            .map(|i| ev(0, &format!(r#"{{"display_name":"alice{}"}}"#, i), vec![]))
            .collect();
        let r = rank_user_search_results(&events, "alice", 3);
        assert_eq!(r.users.len(), 3);
    }

    #[test]
    fn rank_case_insensitive() {
        let e = ev(0, r#"{"display_name":"AlIcE"}"#, vec![]);
        let r = rank_user_search_results(&[e], "alice", 10);
        assert_eq!(r.users.len(), 1);
        let r =
            rank_user_search_results(&[ev(0, r#"{"display_name":"alice"}"#, vec![])], "ALICE", 10);
        assert_eq!(r.users.len(), 1);
    }

    #[test]
    fn rank_tiebreak_preserves_relay_relevance_order() {
        let first = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let second = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let first_pk = first.pubkey.to_hex();
        let r = rank_user_search_results(&[first, second], "alice", 10);
        assert_eq!(r.users.len(), 2);
        assert_eq!(r.users[0].pubkey, first_pk);
    }

    #[test]
    fn rank_falls_back_to_name_when_display_name_absent() {
        let e = ev(0, r#"{"name":"alice"}"#, vec![]);
        let r = rank_user_search_results(&[e], "alice", 10);
        assert_eq!(r.users.len(), 1);
        assert_eq!(r.users[0].display_name.as_deref(), Some("alice"));
    }
}
