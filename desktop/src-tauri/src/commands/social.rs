use std::collections::{HashMap, HashSet};

use nostr::{Event, EventId, PublicKey, Tag};
use tauri::State;

use crate::{
    app_state::AppState,
    events,
    models::{
        ContactEntry, ContactListResponse, NoteReactionSummary, UserNoteInfo, UserNotesResponse,
    },
    nostr_convert,
    relay::{query_relay, submit_event, SubmitEventResponse},
};

const KIND_LONG_FORM: u32 = buzz_core_pkg::kind::KIND_LONG_FORM;
const LONG_FORM_IDENTIFIER_MAX_BYTES: usize = 4096;

fn e_tag_id(tag: &Tag) -> Option<&String> {
    let values = tag.as_slice();
    match (values.first().map(String::as_str), values.get(1)) {
        (Some("e"), Some(id)) => Some(id),
        _ => None,
    }
}

fn deleted_event_ids(events: &[Event]) -> HashSet<String> {
    events
        .iter()
        .flat_map(|event| event.tags.iter().filter_map(e_tag_id).cloned())
        .collect()
}

fn last_event_tag_id(event: &Event) -> Option<String> {
    event.tags.iter().rev().find_map(e_tag_id).cloned()
}

fn last_matching_event_tag_id(event: &Event, targets: &HashSet<String>) -> Option<String> {
    event
        .tags
        .iter()
        .rev()
        .filter_map(e_tag_id)
        .find(|id| targets.contains(*id))
        .cloned()
}

fn reaction_emoji(event: &Event) -> String {
    if event.content.is_empty() {
        "+".to_string()
    } else {
        event.content.clone()
    }
}

fn validate_pubkey(pubkey: &str) -> Result<String, String> {
    PublicKey::from_hex(pubkey)
        .map(|key| key.to_hex())
        .map_err(|e| format!("invalid pubkey: {e}"))
}

fn validate_long_form_identifier(identifier: &str) -> Result<(), String> {
    if identifier.is_empty() {
        return Err("invalid identifier: must not be empty".to_string());
    }
    if identifier.len() > LONG_FORM_IDENTIFIER_MAX_BYTES {
        return Err(format!(
            "invalid identifier: exceeds {LONG_FORM_IDENTIFIER_MAX_BYTES} bytes"
        ));
    }
    if identifier.chars().any(char::is_control) {
        return Err("invalid identifier: must not contain control characters".to_string());
    }

    Ok(())
}

fn long_form_note_filter(pubkey: &str, identifier: &str) -> serde_json::Value {
    serde_json::json!({
        "kinds": [KIND_LONG_FORM],
        "authors": [pubkey],
        "#d": [identifier],
        "limit": 1,
    })
}

fn long_form_note_from_event(event: &Event) -> Result<(String, UserNoteInfo), String> {
    if event.kind.as_u16() as u32 != KIND_LONG_FORM {
        return Err(format!(
            "expected kind:{KIND_LONG_FORM}, got {}",
            event.kind.as_u16()
        ));
    }

    let identifier = event
        .tags
        .iter()
        .find_map(|tag| {
            let values = tag.as_slice();
            if values.first().map(String::as_str) == Some("d") {
                values.get(1).cloned()
            } else {
                None
            }
        })
        .ok_or_else(|| "kind:30023 event is missing required d tag".to_string())?;
    validate_long_form_identifier(&identifier)?;

    let note = UserNoteInfo {
        id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs() as i64,
        content: event.content.clone(),
        tags: event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect(),
    };

    Ok((identifier, note))
}

/// Publish a global kind:1 text note (NIP-01).
#[tauri::command]
pub async fn publish_note(
    content: String,
    reply_to: Option<String>,
    mention_pubkeys: Option<Vec<String>>,
    media_tags: Option<Vec<Vec<String>>>,
    state: State<'_, AppState>,
) -> Result<SubmitEventResponse, String> {
    let reply_id = reply_to
        .map(|hex| EventId::from_hex(&hex).map_err(|e| format!("invalid reply_to event id: {e}")))
        .transpose()?;
    let mentions = mention_pubkeys.unwrap_or_default();
    let mention_refs: Vec<&str> = mentions.iter().map(|s| s.as_str()).collect();
    let media = media_tags.unwrap_or_default();
    let builder = events::build_note(&content, reply_id, &mention_refs, &media)?;
    submit_event(builder, &state).await
}

/// Fetch a user's NIP-02 contact list (kind:3).
#[tauri::command]
pub async fn get_contact_list(
    pubkey: String,
    state: State<'_, AppState>,
) -> Result<ContactListResponse, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [3],
            "authors": [pubkey],
            "limit": 1
        })],
    )
    .await?;

    if let Some(event) = events.first() {
        return nostr_convert::contact_list_from_event(event);
    }

    Ok(ContactListResponse {
        id: String::new(),
        pubkey,
        created_at: 0,
        tags: Vec::new(),
        content: String::new(),
    })
}

/// Replace the full contact list (kind:3, NIP-02). Read-before-write required
/// for delta updates — the caller must merge with the existing list.
#[tauri::command]
pub async fn set_contact_list(
    contacts: Vec<ContactEntry>,
    state: State<'_, AppState>,
) -> Result<SubmitEventResponse, String> {
    let tuples: Vec<(&str, Option<&str>, Option<&str>)> = contacts
        .iter()
        .map(|c| {
            (
                c.pubkey.as_str(),
                c.relay_url.as_deref(),
                c.petname.as_deref(),
            )
        })
        .collect();

    let builder = events::build_contact_list(&tuples)?;
    submit_event(builder, &state).await
}

/// Fetch global NIP-01 kind:1 notes without an author filter.
#[tauri::command]
pub async fn get_global_notes(
    limit: Option<u32>,
    before: Option<i64>,
    before_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<UserNotesResponse, String> {
    let _ = before_id;
    let mut filter = serde_json::Map::new();
    filter.insert("kinds".to_string(), serde_json::json!([1]));
    filter.insert(
        "limit".to_string(),
        serde_json::json!(limit.unwrap_or(50).min(200)),
    );
    if let Some(t) = before {
        filter.insert("until".to_string(), serde_json::json!(t));
    }

    let events = query_relay(&state, &[serde_json::Value::Object(filter)]).await?;
    Ok(nostr_convert::user_notes_from_events(&events))
}

fn validate_note_id(note_id: &str) -> Result<(), String> {
    if note_id.len() == 64 && note_id.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("invalid note id".to_string())
    }
}

/// Fetch a single NIP-01 kind:1 note by event id.
#[tauri::command]
pub async fn get_note(
    note_id: String,
    state: State<'_, AppState>,
) -> Result<Option<UserNoteInfo>, String> {
    validate_note_id(&note_id)?;
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [1],
            "ids": [note_id],
            "limit": 1,
        })],
    )
    .await?;

    Ok(nostr_convert::user_notes_from_events(&events)
        .notes
        .into_iter()
        .next())
}

/// Fetch a single NIP-23 long-form note by exact kind:30023 coordinate.
#[tauri::command]
pub async fn get_long_form_note(
    pubkey: String,
    identifier: String,
    state: State<'_, AppState>,
) -> Result<Option<UserNoteInfo>, String> {
    let pubkey = validate_pubkey(&pubkey)?;
    validate_long_form_identifier(&identifier)?;

    let events = query_relay(&state, &[long_form_note_filter(&pubkey, &identifier)]).await?;

    let Some(event) = events.first() else {
        return Ok(None);
    };
    if event.pubkey.to_hex() != pubkey {
        return Err("relay returned long-form note for a different author".to_string());
    }

    let (returned_identifier, note) = long_form_note_from_event(event)?;
    if returned_identifier != identifier {
        return Err("relay returned long-form note for a different identifier".to_string());
    }

    Ok(Some(note))
}

const MAX_NOTE_IDS: usize = 200;

/// Fetch and fold kind:7 reactions for visible Pulse notes.
#[tauri::command]
pub async fn get_note_reactions(
    note_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<NoteReactionSummary>, String> {
    if note_ids.is_empty() {
        return Ok(Vec::new());
    }
    if note_ids.len() > MAX_NOTE_IDS {
        return Err(format!(
            "too many note ids (max {MAX_NOTE_IDS}, got {})",
            note_ids.len()
        ));
    }
    for note_id in &note_ids {
        validate_note_id(note_id)?;
    }

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [7],
            "#e": note_ids,
            "limit": 500,
        })],
    )
    .await?;

    let reaction_ids: Vec<String> = events.iter().map(|event| event.id.to_hex()).collect();
    let deletion_events = if reaction_ids.is_empty() {
        Vec::new()
    } else {
        query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [5],
                "#e": reaction_ids,
                "limit": 500,
            })],
        )
        .await?
    };
    let deleted_reaction_ids = deleted_event_ids(&deletion_events);

    let targets: HashSet<String> = note_ids.into_iter().collect();
    let mut by_note_and_emoji = HashMap::<(String, String), HashSet<String>>::new();
    for event in events {
        if deleted_reaction_ids.contains(&event.id.to_hex()) {
            continue;
        }

        let Some(target_id) = last_matching_event_tag_id(&event, &targets) else {
            continue;
        };

        let emoji = reaction_emoji(&event);
        by_note_and_emoji
            .entry((target_id, emoji))
            .or_default()
            .insert(event.pubkey.to_hex());
    }

    let mut summaries: Vec<NoteReactionSummary> = by_note_and_emoji
        .into_iter()
        .map(|((note_id, emoji), pubkey_set)| {
            let mut pubkeys: Vec<String> = pubkey_set.into_iter().collect();
            pubkeys.sort();
            NoteReactionSummary {
                note_id,
                emoji,
                count: pubkeys.len(),
                pubkeys,
            }
        })
        .collect();
    summaries.sort_by(|left, right| {
        left.note_id
            .cmp(&right.note_id)
            .then_with(|| left.emoji.cmp(&right.emoji))
    });

    Ok(summaries)
}

/// Fetch notes liked by a user, excluding deleted reaction events.
#[tauri::command]
pub async fn get_liked_notes(
    author_pubkey: String,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> Result<UserNotesResponse, String> {
    let cap = limit.unwrap_or(50).min(MAX_NOTE_IDS as u32) as usize;
    let reaction_fetch_limit = (cap * 4).min(1000);
    let mut reactions = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [7],
            "authors": [author_pubkey],
            "limit": reaction_fetch_limit,
        })],
    )
    .await?;
    reactions.sort_by_key(|reaction| std::cmp::Reverse(reaction.created_at));

    let reaction_ids: Vec<String> = reactions.iter().map(|event| event.id.to_hex()).collect();
    let deletions = if reaction_ids.is_empty() {
        Vec::new()
    } else {
        query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [5],
                "authors": [author_pubkey],
                "#e": reaction_ids,
                "limit": 500,
            })],
        )
        .await?
    };
    let deleted_reaction_ids = deleted_event_ids(&deletions);

    let mut target_ids = Vec::<String>::new();
    let mut target_liked_at = HashMap::<String, i64>::new();
    let mut seen_targets = HashSet::<String>::new();
    for reaction in reactions {
        if target_ids.len() >= cap {
            break;
        }
        if deleted_reaction_ids.contains(&reaction.id.to_hex()) {
            continue;
        }
        let Some(target_id) = last_event_tag_id(&reaction) else {
            continue;
        };
        if seen_targets.insert(target_id.clone()) {
            target_liked_at.insert(target_id.clone(), reaction.created_at.as_secs() as i64);
            target_ids.push(target_id);
        }
    }

    if target_ids.is_empty() {
        return Ok(UserNotesResponse {
            notes: Vec::new(),
            next_cursor: None,
        });
    }

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [1],
            "ids": target_ids,
            "limit": cap,
        })],
    )
    .await?;
    let mut response = nostr_convert::user_notes_from_events(&events);
    response.notes.sort_by(|left, right| {
        target_liked_at
            .get(&right.id)
            .unwrap_or(&0)
            .cmp(target_liked_at.get(&left.id).unwrap_or(&0))
    });
    response.notes.truncate(cap);
    Ok(response)
}

/// Maximum number of pubkeys per timeline request to keep filter size bounded.
const MAX_TIMELINE_PUBKEYS: usize = 100;

/// Fetch notes for multiple pubkeys with a single multi-author query.
#[tauri::command]
pub async fn get_notes_timeline(
    pubkeys: Vec<String>,
    limit_per_user: Option<u32>,
    state: State<'_, AppState>,
) -> Result<UserNotesResponse, String> {
    if pubkeys.is_empty() {
        return Ok(UserNotesResponse {
            notes: Vec::new(),
            next_cursor: None,
        });
    }
    if pubkeys.len() > MAX_TIMELINE_PUBKEYS {
        return Err(format!(
            "too many pubkeys (max {MAX_TIMELINE_PUBKEYS}, got {})",
            pubkeys.len()
        ));
    }

    // One filter for all authors: `limit` here is the total cap. We use
    // `limit_per_user * pubkeys.len()` as a rough approximation, capped at 200
    // to match the prior implementation's behavior.
    let per_user = limit_per_user.unwrap_or(10).min(50) as usize;
    let cap: usize = (per_user * pubkeys.len()).min(200);

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [1],
            "authors": pubkeys,
            "limit": cap,
        })],
    )
    .await?;

    let mut notes: Vec<UserNoteInfo> = events
        .iter()
        .map(|ev| UserNoteInfo {
            id: ev.id.to_hex(),
            pubkey: ev.pubkey.to_hex(),
            created_at: ev.created_at.as_secs() as i64,
            content: ev.content.clone(),
            tags: ev.tags.iter().map(|tag| tag.as_slice().to_vec()).collect(),
        })
        .collect();

    // Sort newest-first.
    notes.sort_by_key(|note| std::cmp::Reverse(note.created_at));
    notes.truncate(200);

    Ok(UserNotesResponse {
        notes,
        next_cursor: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn tag(values: &[&str]) -> Tag {
        Tag::parse(values.iter().copied()).expect("parse tag")
    }

    fn event(tags: Vec<Tag>, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(7), content)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign event")
    }

    fn long_form_event(tags: Vec<Tag>, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(KIND_LONG_FORM as u16), content)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign event")
    }

    #[test]
    fn e_tag_id_returns_only_event_tag_values() {
        let e = tag(&["e", "a"]);
        let p = tag(&["p", "b"]);
        assert_eq!(e_tag_id(&e), Some(&"a".to_string()));
        assert_eq!(e_tag_id(&p), None);
    }

    #[test]
    fn validates_pubkey_as_canonical_hex() {
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();

        assert_eq!(validate_pubkey(&pubkey).unwrap(), pubkey);
        assert!(validate_pubkey("not-a-pubkey").is_err());
        assert!(validate_pubkey(&format!("{pubkey} ")).is_err());
    }

    #[test]
    fn validates_long_form_identifier() {
        assert!(validate_long_form_identifier("article-slug").is_ok());
        assert!(validate_long_form_identifier("").is_err());
        assert!(validate_long_form_identifier("article\nslug").is_err());
        assert!(validate_long_form_identifier(&"a".repeat(LONG_FORM_IDENTIFIER_MAX_BYTES)).is_ok());
        assert!(
            validate_long_form_identifier(&"a".repeat(LONG_FORM_IDENTIFIER_MAX_BYTES + 1)).is_err()
        );
    }

    #[test]
    fn long_form_note_filter_is_exact_coordinate_query() {
        let filter = long_form_note_filter("abc", "slug");

        assert_eq!(
            filter,
            serde_json::json!({
                "kinds": [30023],
                "authors": ["abc"],
                "#d": ["slug"],
                "limit": 1,
            })
        );
    }

    #[test]
    fn long_form_note_from_event_reuses_user_note_shape() {
        let event = long_form_event(
            vec![
                tag(&["d", "article-slug"]),
                tag(&["title", "Article title"]),
                tag(&["summary", "Short summary"]),
                tag(&["published_at", "1710000000"]),
                tag(&["t", "rust"]),
                tag(&["t", "nostr"]),
            ],
            "# Body",
        );

        let (identifier, note) = long_form_note_from_event(&event).unwrap();

        assert_eq!(note.id, event.id.to_hex());
        assert_eq!(note.pubkey, event.pubkey.to_hex());
        assert_eq!(identifier, "article-slug");
        assert_eq!(note.content, "# Body");
        assert!(note
            .tags
            .iter()
            .any(|tag| tag == &["title".to_string(), "Article title".to_string()]));
    }

    #[test]
    fn long_form_note_from_event_rejects_wrong_kind_or_missing_d_tag() {
        let wrong_kind = event(vec![tag(&["d", "article-slug"])], "body");
        assert!(long_form_note_from_event(&wrong_kind).is_err());

        let missing_d = long_form_event(Vec::new(), "body");
        assert!(long_form_note_from_event(&missing_d).is_err());
    }

    #[test]
    fn last_event_tag_id_uses_last_e_tag() {
        let ev = event(
            vec![tag(&["e", "a"]), tag(&["p", "x"]), tag(&["e", "b"])],
            "+",
        );
        assert_eq!(last_event_tag_id(&ev), Some("b".to_string()));
    }

    #[test]
    fn last_matching_event_tag_id_uses_last_visible_target() {
        let ev = event(
            vec![tag(&["e", "x"]), tag(&["e", "y"]), tag(&["e", "z"])],
            "+",
        );
        let targets = HashSet::from(["y".to_string(), "z".to_string()]);
        assert_eq!(
            last_matching_event_tag_id(&ev, &targets),
            Some("z".to_string())
        );
    }

    #[test]
    fn deleted_event_ids_collects_all_e_tags() {
        let first = event(vec![tag(&["e", "a"]), tag(&["e", "b"])], "");
        let second = event(vec![tag(&["p", "ignored"]), tag(&["e", "c"])], "");
        let deleted = deleted_event_ids(&[first, second]);
        assert!(deleted.contains("a"));
        assert!(deleted.contains("b"));
        assert!(deleted.contains("c"));
        assert!(!deleted.contains("ignored"));
    }

    #[test]
    fn reaction_emoji_defaults_empty_content_to_plus() {
        assert_eq!(reaction_emoji(&event(Vec::new(), "")), "+");
        assert_eq!(reaction_emoji(&event(Vec::new(), "🔥")), "🔥");
    }

    #[test]
    fn validate_note_id_requires_hex64() {
        assert!(validate_note_id(&"a".repeat(64)).is_ok());
        assert!(validate_note_id(&"g".repeat(64)).is_err());
        assert!(validate_note_id(&"a".repeat(63)).is_err());
    }
}
