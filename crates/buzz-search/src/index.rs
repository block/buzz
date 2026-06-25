//! Event indexing — Nostr events → Typesense documents. Upsert semantics.

use serde_json::{json, Value};
use tracing::{debug, warn};

use buzz_core::event::StoredEvent;
use buzz_core::kind::event_kind_i32;

use crate::error::SearchError;

/// Converts a [`StoredEvent`] into a Typesense document JSON value.
pub fn event_to_document(event: &StoredEvent) -> Result<Value, SearchError> {
    let nostr_event = &event.event;

    // Use ASCII unit separator (U+001F) as delimiter to avoid ambiguity with
    // tag values that contain colons (e.g. URLs in "r" tags).
    let tags_flat: Vec<String> = nostr_event
        .tags
        .iter()
        .flat_map(|tag| {
            let tag_vec = tag.as_slice();
            if tag_vec.len() >= 2 {
                vec![format!("{}\x1f{}", tag_vec[0], tag_vec[1])]
            } else if tag_vec.len() == 1 {
                vec![tag_vec[0].to_string()]
            } else {
                vec![]
            }
        })
        .collect();

    // Global events use a sentinel value instead of NULL/absent. Typesense 27.1's
    // `__missing__` filter does not reliably match absent optional fields, so we use
    // an explicit `__global__` sentinel that can be matched with `channel_id:=__global__`.
    // NOTE: Historical docs indexed before this change have channel_id absent/null and
    // won't match the sentinel filter. A full reindex (`just reindex-search`) is needed
    // after deploy to backfill. Pre-existing global events (kind:0 only) were already
    // excluded from search results by the old `.filter(|h| h.channel_id.is_some())`, so
    // this is not a regression — those docs were never returned.
    let channel_id_val = event
        .channel_id
        .as_ref()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "__global__".to_string());

    // For kind:0 (user metadata) we append the parsed JSON values to `content`
    // so Typesense's word-tokenizer can index them cleanly. Without this, a
    // raw blob like `{"display_name":"alice","about":"loves cats"}` does not
    // produce a clean `alice` token — the leading `"` glues onto the next
    // word, so the doc is unreachable for the obvious `q=alice` search.
    //
    // We only flatten kind:0 (the structured-metadata kind defined by NIP-01)
    // and only the small set of fields the member-picker uses. Bio / about /
    // website are intentionally left out so they don't pollute name-prefix
    // searches with false positives. Stays consistent with `display_name >
    // nip05 > pubkey` ranking applied on the desktop side.
    //
    // The Typesense `content` field is write-only as far as the relay's read
    // paths go (the bridge fetches the canonical event from Postgres by id
    // after Typesense returns hits), so appending derived tokens here doesn't
    // affect any consumer's view of the event's actual content.
    //
    // NOTE: existing kind:0 docs indexed before this change won't have the
    // appended tokens. Running `just reindex-search` (or the
    // `buzz-relay reindex-search` admin path) repopulates them. New /
    // updated profiles get the tokens automatically.
    let content_indexed = if event_kind_i32(nostr_event) == 0 {
        flatten_kind0_for_indexing(nostr_event.content.as_str())
    } else {
        nostr_event.content.as_str().to_string()
    };

    let doc = json!({
        "id":         nostr_event.id.to_string(),
        "content":    content_indexed,
        // Cast to i32 for Typesense schema (int32 field). nostr Kind is u16; all Buzz kinds fit in i32.
        "kind":       event_kind_i32(nostr_event),
        "pubkey":     nostr_event.pubkey.to_string(),
        "channel_id": channel_id_val,
        "created_at": nostr_event.created_at.as_secs() as i64,
        "tags_flat":  tags_flat,
    });

    Ok(doc)
}

/// For kind:0 events, return the original content with the searchable fields
/// (`display_name`, `name`, `nip05`) appended as space-separated plain words.
///
/// Tolerant of malformed input: anything that fails JSON parsing returns the
/// original content unchanged, never an error.
fn flatten_kind0_for_indexing(raw_content: &str) -> String {
    let Ok(parsed) = serde_json::from_str::<Value>(raw_content) else {
        return raw_content.to_string();
    };
    let Some(obj) = parsed.as_object() else {
        return raw_content.to_string();
    };

    let mut extracted: Vec<&str> = Vec::with_capacity(3);
    for key in ["display_name", "name", "nip05"] {
        if let Some(val) = obj.get(key).and_then(Value::as_str) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                extracted.push(trimmed);
            }
        }
    }

    if extracted.is_empty() {
        raw_content.to_string()
    } else {
        // Single leading space ensures we don't smash the closing `}` of the
        // original JSON into the first appended token.
        format!("{} {}", raw_content, extracted.join(" "))
    }
}

/// Indexes a single event via Typesense upsert.
pub async fn index_event(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    collection_name: &str,
    event: &StoredEvent,
) -> Result<(), SearchError> {
    let doc = event_to_document(event)?;
    let url = format!(
        "{}/collections/{}/documents?action=upsert",
        base_url, collection_name
    );

    debug!(event_id = %event.event.id, collection = collection_name, "indexing event");

    let resp = client
        .post(&url)
        .header("X-TYPESENSE-API-KEY", api_key)
        .header("Content-Type", "application/json")
        .json(&doc)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status == 200 || status == 201 {
        Ok(())
    } else {
        let body = resp.text().await.unwrap_or_default();
        Err(SearchError::Api { status, body })
    }
}

/// Indexes a batch of events via Typesense JSONL import.
pub async fn index_batch(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    collection_name: &str,
    events: &[StoredEvent],
) -> Result<usize, SearchError> {
    if events.is_empty() {
        return Ok(0);
    }

    let mut jsonl = String::new();
    for event in events {
        let doc = event_to_document(event)?;
        jsonl.push_str(&serde_json::to_string(&doc)?);
        jsonl.push('\n');
    }

    let url = format!(
        "{}/collections/{}/documents/import?action=upsert",
        base_url, collection_name
    );

    debug!(
        count = events.len(),
        collection = collection_name,
        "batch indexing events"
    );

    let resp = client
        .post(&url)
        .header("X-TYPESENSE-API-KEY", api_key)
        .header("Content-Type", "text/plain")
        .body(jsonl)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(SearchError::Api { status, body });
    }

    let body = resp.text().await.unwrap_or_default();
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(result) => {
                if result["success"].as_bool().unwrap_or(false) {
                    succeeded += 1;
                } else {
                    failed += 1;
                    warn!(
                        error = result["error"].as_str().unwrap_or("unknown"),
                        "batch import document failure"
                    );
                }
            }
            Err(e) => {
                warn!(line = line, error = %e, "could not parse batch import result line");
                failed += 1;
            }
        }
    }

    if failed > 0 {
        Err(SearchError::BatchPartial { succeeded, failed })
    } else {
        Ok(succeeded)
    }
}

/// Validate that `event_id` is a 64-character lowercase hex string, as
/// required by the Nostr protocol (SHA-256 of the serialised event).
///
/// Rejects the input early to avoid sending a malformed path segment to
/// Typesense, which could otherwise produce confusing 404 or 400 responses,
/// or — if the value contains `/` or `?` — accidentally hit a different API
/// endpoint.
fn validate_event_id(event_id: &str) -> Result<(), SearchError> {
    if event_id.len() != 64 {
        return Err(SearchError::InvalidEventId(format!(
            "event_id must be 64 hex characters, got {} characters",
            event_id.len()
        )));
    }
    if !event_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SearchError::InvalidEventId(
            "event_id must contain only hex characters (0-9, a-f)".into(),
        ));
    }
    Ok(())
}

/// Deletes an event from the index by event ID hex string.
pub async fn delete_event(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    collection_name: &str,
    event_id: &str,
) -> Result<(), SearchError> {
    validate_event_id(event_id)?;

    let url = format!(
        "{}/collections/{}/documents/{}",
        base_url, collection_name, event_id
    );

    debug!(
        event_id = event_id,
        collection = collection_name,
        "deleting event from index"
    );

    let resp = client
        .delete(&url)
        .header("X-TYPESENSE-API-KEY", api_key)
        .send()
        .await?;

    match resp.status().as_u16() {
        200 => Ok(()),
        404 => {
            debug!(
                event_id = event_id,
                "event not found in index (already deleted)"
            );
            Ok(())
        }
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(SearchError::Api { status, body })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::event::StoredEvent;
    use nostr::{EventBuilder, Keys, Kind};
    use uuid::Uuid;

    fn make_stored_event(content: &str, kind: Kind, channel_id: Option<Uuid>) -> StoredEvent {
        let keys = Keys::generate();
        let event = EventBuilder::new(kind, content)
            .tags([])
            .sign_with_keys(&keys)
            .expect("signing failed");
        StoredEvent::new(event, channel_id)
    }

    #[test]
    fn document_fields_correct() {
        let channel_id = Uuid::new_v4();
        let stored = make_stored_event("hello world", Kind::TextNote, Some(channel_id));
        let doc = event_to_document(&stored).unwrap();

        assert_eq!(doc["id"].as_str().unwrap(), stored.event.id.to_string());
        assert_eq!(doc["content"].as_str().unwrap(), "hello world");
        assert_eq!(doc["kind"].as_i64().unwrap(), 1i64);
        assert_eq!(doc["channel_id"].as_str().unwrap(), channel_id.to_string());
        assert!(doc["created_at"].as_i64().is_some());
        assert!(doc["channel_id"].is_string());
    }

    #[test]
    fn document_no_channel_id_uses_global_sentinel() {
        let stored = make_stored_event("no channel", Kind::TextNote, None);
        let doc = event_to_document(&stored).unwrap();
        assert_eq!(doc["channel_id"].as_str().unwrap(), "__global__");
    }

    #[test]
    fn kind0_appends_display_name_for_tokenization() {
        let stored = make_stored_event(
            r#"{"display_name":"alice","about":"loves cats"}"#,
            Kind::Metadata,
            None,
        );
        let doc = event_to_document(&stored).unwrap();
        let content = doc["content"].as_str().unwrap();
        // Original JSON is preserved (read paths don't depend on this but it
        // costs nothing and makes debugging the index cheaper).
        assert!(content.contains(r#""display_name":"alice""#));
        // The display name is also present as a free-standing token so the
        // default Typesense tokenizer can index it without the leading-quote
        // gluing onto the next character.
        assert!(content.ends_with(" alice"), "got: {content:?}");
    }

    #[test]
    fn kind0_appends_name_when_display_name_absent() {
        // NIP-01 allows `name` as the canonical display field too.
        let stored = make_stored_event(r#"{"name":"bob","about":"x"}"#, Kind::Metadata, None);
        let doc = event_to_document(&stored).unwrap();
        let content = doc["content"].as_str().unwrap();
        assert!(content.ends_with(" bob"), "got: {content:?}");
    }

    #[test]
    fn kind0_includes_both_display_name_and_name_when_present() {
        let stored = make_stored_event(
            r#"{"display_name":"Alice","name":"alice"}"#,
            Kind::Metadata,
            None,
        );
        let doc = event_to_document(&stored).unwrap();
        let content = doc["content"].as_str().unwrap();
        assert!(content.ends_with(" Alice alice"), "got: {content:?}");
    }

    #[test]
    fn kind0_includes_nip05_in_appended_tokens() {
        let stored = make_stored_event(
            r#"{"display_name":"alice","nip05":"alice@example.com"}"#,
            Kind::Metadata,
            None,
        );
        let doc = event_to_document(&stored).unwrap();
        let content = doc["content"].as_str().unwrap();
        assert!(
            content.ends_with(" alice alice@example.com"),
            "got: {content:?}"
        );
    }

    #[test]
    fn kind0_excludes_about_and_website_from_appended_tokens() {
        // `about` and `website` deliberately do not get appended — including
        // them would cause name-prefix searches to return false positives from
        // bios. The user's own display_name still appears.
        let stored = make_stored_event(
            r#"{"display_name":"alice","about":"I work with bob on x","website":"https://carol.example"}"#,
            Kind::Metadata,
            None,
        );
        let doc = event_to_document(&stored).unwrap();
        let content = doc["content"].as_str().unwrap();
        assert!(content.ends_with(" alice"), "got: {content:?}");
        // Sanity: the about/website are still in the doc because we preserve
        // the original JSON — they just don't appear in the trailing tokens.
        assert!(content.contains("bob"));
        assert!(content.contains("carol"));
    }

    #[test]
    fn kind0_malformed_json_is_passed_through_unchanged() {
        let stored = make_stored_event("not json at all", Kind::Metadata, None);
        let doc = event_to_document(&stored).unwrap();
        assert_eq!(doc["content"].as_str().unwrap(), "not json at all");
    }

    #[test]
    fn kind0_non_object_json_is_passed_through_unchanged() {
        // Defensive: NIP-01 says content is a JSON object, but a malformed
        // client could publish e.g. a JSON array. We don't crash, we just
        // skip the flattening for that doc.
        let stored = make_stored_event(r#"["nope"]"#, Kind::Metadata, None);
        let doc = event_to_document(&stored).unwrap();
        assert_eq!(doc["content"].as_str().unwrap(), r#"["nope"]"#);
    }

    #[test]
    fn kind0_empty_string_values_skipped() {
        let stored = make_stored_event(
            r#"{"display_name":"","name":"alice","nip05":"   "}"#,
            Kind::Metadata,
            None,
        );
        let doc = event_to_document(&stored).unwrap();
        let content = doc["content"].as_str().unwrap();
        // Only `name` is non-empty; whitespace-only `nip05` is also skipped.
        assert!(content.ends_with(" alice"), "got: {content:?}");
    }

    #[test]
    fn kind0_no_searchable_fields_is_passed_through() {
        // Profile with only fields we don't extract.
        let stored = make_stored_event(
            r#"{"about":"just a bio","picture":"https://x"}"#,
            Kind::Metadata,
            None,
        );
        let doc = event_to_document(&stored).unwrap();
        let content = doc["content"].as_str().unwrap();
        // No trailing space-separated tokens added; original content unchanged.
        assert_eq!(content, r#"{"about":"just a bio","picture":"https://x"}"#);
    }

    #[test]
    fn non_kind0_events_not_flattened() {
        // kind:1 (note) with a JSON-looking body must be left strictly alone.
        let json_looking = r#"{"display_name":"alice"}"#;
        let stored = make_stored_event(json_looking, Kind::TextNote, None);
        let doc = event_to_document(&stored).unwrap();
        assert_eq!(doc["content"].as_str().unwrap(), json_looking);
    }

    #[test]
    fn tag_flattening_uses_unit_separator() {
        let keys = Keys::generate();
        let tag = nostr::Tag::parse(["e", "abc123def456"]).expect("tag parse");
        let event = EventBuilder::new(Kind::TextNote, "tagged")
            .tags([tag])
            .sign_with_keys(&keys)
            .expect("sign");
        let stored = StoredEvent::new(event, None);
        let doc = event_to_document(&stored).unwrap();

        let tags_flat = doc["tags_flat"].as_array().unwrap();
        assert!(!tags_flat.is_empty());
        // Must use \x1f, not colon, to avoid ambiguity with values containing colons.
        let entry = tags_flat[0].as_str().unwrap();
        assert!(
            entry.contains('\x1f'),
            "expected unit separator in tag entry: {entry:?}"
        );
        assert!(
            !entry.contains(':') || entry.starts_with("http"),
            "colon used as delimiter"
        );
        assert!(entry.contains("abc123def456"));
    }

    #[test]
    fn delete_event_rejects_invalid_id() {
        // Too short
        assert!(matches!(
            validate_event_id("abc123"),
            Err(SearchError::InvalidEventId(_))
        ));
        // Right length but non-hex character
        let bad = "g".repeat(64);
        assert!(matches!(
            validate_event_id(&bad),
            Err(SearchError::InvalidEventId(_))
        ));
        // Valid 64-char hex
        let good = "a".repeat(64);
        assert!(validate_event_id(&good).is_ok());
        // Uppercase hex should also be accepted
        let upper = "A".repeat(64);
        assert!(validate_event_id(&upper).is_ok());
        // Path injection attempt
        assert!(matches!(
            validate_event_id("../admin"),
            Err(SearchError::InvalidEventId(_))
        ));
    }

    #[test]
    fn tag_with_colon_value_not_ambiguous() {
        let keys = Keys::generate();
        // "r" tag with a URL value containing colons
        let tag = nostr::Tag::parse(["r", "wss://relay.example.com"]).expect("tag parse");
        let event = EventBuilder::new(Kind::TextNote, "relay ref")
            .tags([tag])
            .sign_with_keys(&keys)
            .expect("sign");
        let stored = StoredEvent::new(event, None);
        let doc = event_to_document(&stored).unwrap();

        let tags_flat = doc["tags_flat"].as_array().unwrap();
        let entry = tags_flat[0].as_str().unwrap();
        // With \x1f delimiter, splitting on \x1f gives exactly ["r", "wss://relay.example.com"]
        let parts: Vec<&str> = entry.splitn(2, '\x1f').collect();
        assert_eq!(parts[0], "r");
        assert_eq!(parts[1], "wss://relay.example.com");
    }
}
