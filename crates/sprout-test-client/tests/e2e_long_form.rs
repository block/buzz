//! End-to-end tests for NIP-23 long-form content (kind:30023).
//!
//! These tests require a running relay instance. By default they are marked
//! `#[ignore]` so that `cargo test` does not fail in CI when the relay is not
//! available.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test --test e2e_long_form -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable:
//!
//! ```text
//! RELAY_URL=ws://relay.example.com cargo test --test e2e_long_form -- --ignored
//! ```

use std::time::Duration;

use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};
use sprout_test_client::SproutTestClient;

const KIND_LONG_FORM: u16 = 30023;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-{name}-{}", uuid::Uuid::new_v4())
}

/// Build a kind:30023 event with standard NIP-23 tags.
fn build_long_form_event(
    keys: &Keys,
    d_tag: &str,
    title: &str,
    content: &str,
    extra_tags: Vec<Tag>,
) -> nostr::Event {
    let mut tags = vec![
        Tag::parse(&["d", d_tag]).unwrap(),
        Tag::parse(&["title", title]).unwrap(),
    ];
    tags.extend(extra_tags);
    EventBuilder::new(Kind::Custom(KIND_LONG_FORM), content, tags)
        .sign_with_keys(keys)
        .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// kind:30023 events are accepted by the relay.
#[tokio::test]
#[ignore]
async fn test_long_form_accepted() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let event = build_long_form_event(
        &keys,
        "test-article-accept",
        "Test Article",
        "# Hello\n\nThis is a test article.",
        vec![],
    );

    let ok = client.send_event(event).await.expect("send event");
    assert!(
        ok.accepted,
        "relay should accept kind:30023: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// kind:30023 events are retrievable via REQ with kinds filter.
#[tokio::test]
#[ignore]
async fn test_long_form_retrievable() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let d_tag = format!("retrieve-{}", uuid::Uuid::new_v4().simple());
    let event = build_long_form_event(
        &keys,
        &d_tag,
        "Retrievable Article",
        "# Retrievable\n\nBody text.",
        vec![],
    );
    let event_id = event.id;

    let ok = client.send_event(event).await.expect("send event");
    assert!(ok.accepted, "relay should accept: {}", ok.message);

    // Query back by kind + author
    let sid = sub_id("retrieve");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_LONG_FORM))
        .author(keys.public_key());
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        events.iter().any(|e| e.id == event_id),
        "should find the published article in query results"
    );

    client.disconnect().await.expect("disconnect");
}

/// kind:30023 is stored globally (channel_id = NULL) — stray h-tags are ignored.
/// An event with a stray h-tag should still be retrievable via a global query
/// (no h-tag filter), proving it was stored as global.
#[tokio::test]
#[ignore]
async fn test_long_form_stray_h_tag_ignored() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    // Publish with a stray h-tag (a UUID that doesn't correspond to any channel).
    let fake_channel = uuid::Uuid::new_v4().to_string();
    let d_tag = format!("stray-h-{}", uuid::Uuid::new_v4().simple());
    let event = build_long_form_event(
        &keys,
        &d_tag,
        "Stray H-Tag Article",
        "Should be stored globally despite h-tag.",
        vec![Tag::parse(&["h", &fake_channel]).unwrap()],
    );
    let event_id = event.id;

    let ok = client.send_event(event).await.expect("send event");
    assert!(ok.accepted, "relay should accept: {}", ok.message);

    // Query globally (no h-tag filter) — should find the article.
    let sid = sub_id("stray-h");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_LONG_FORM))
        .author(keys.public_key());
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        events.iter().any(|e| e.id == event_id),
        "article with stray h-tag should be retrievable via global query"
    );

    // NOTE: Ideally, querying with #h=<fake_channel> should NOT return the
    // article since it's global. However, the raw h-tag remains on the stored
    // event (Nostr events are signed — tags can't be stripped without breaking
    // the signature), and the read-path filter matching in filter.rs treats
    // explicit h-tags as authoritative. This is a pre-existing limitation
    // affecting all global-only kinds (0, 1, 3, 30023) and should be fixed
    // in the filter layer as a follow-up.

    client.disconnect().await.expect("disconnect");
}

/// NIP-33 replacement: publishing a newer kind:30023 with the same d-tag replaces the old one.
#[tokio::test]
#[ignore]
async fn test_long_form_nip33_replacement() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let d_tag = format!("replace-{}", uuid::Uuid::new_v4().simple());

    // Publish v1
    let v1 = build_long_form_event(&keys, &d_tag, "Article v1", "Version 1 content.", vec![]);
    let ok1 = client.send_event(v1).await.expect("send v1");
    assert!(ok1.accepted, "v1 should be accepted: {}", ok1.message);

    // Small delay to ensure different created_at timestamps
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Publish v2 with the same d-tag
    let v2 = build_long_form_event(
        &keys,
        &d_tag,
        "Article v2",
        "Version 2 content — updated.",
        vec![],
    );
    let v2_id = v2.id;
    let ok2 = client.send_event(v2).await.expect("send v2");
    assert!(ok2.accepted, "v2 should be accepted: {}", ok2.message);

    // Query — should only get v2 (v1 replaced)
    let sid = sub_id("replace");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_LONG_FORM))
        .author(keys.public_key())
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert_eq!(
        events.len(),
        1,
        "should have exactly one event after replacement"
    );
    assert_eq!(events[0].id, v2_id, "surviving event should be v2");
    assert!(
        events[0].content.contains("Version 2"),
        "content should be v2"
    );

    client.disconnect().await.expect("disconnect");
}

/// NIP-33 stale-write protection: an older event cannot replace a newer one.
#[tokio::test]
#[ignore]
async fn test_long_form_stale_write_rejected() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let d_tag = format!("stale-{}", uuid::Uuid::new_v4().simple());

    // Publish the "newer" event first (with a future-ish timestamp)
    let newer = {
        let tags = vec![
            Tag::parse(&["d", &d_tag]).unwrap(),
            Tag::parse(&["title", "Newer Article"]).unwrap(),
        ];
        EventBuilder::new(Kind::Custom(KIND_LONG_FORM), "Newer content.", tags)
            .custom_created_at(Timestamp::from(nostr::Timestamp::now().as_u64() + 100))
            .sign_with_keys(&keys)
            .unwrap()
    };
    let newer_id = newer.id;
    let ok1 = client.send_event(newer).await.expect("send newer");
    assert!(ok1.accepted, "newer should be accepted: {}", ok1.message);

    // Now try to publish an "older" event with the same d-tag but earlier timestamp
    let older = {
        let tags = vec![
            Tag::parse(&["d", &d_tag]).unwrap(),
            Tag::parse(&["title", "Older Article"]).unwrap(),
        ];
        EventBuilder::new(Kind::Custom(KIND_LONG_FORM), "Older content.", tags)
            .custom_created_at(Timestamp::from(nostr::Timestamp::now().as_u64() - 100))
            .sign_with_keys(&keys)
            .unwrap()
    };
    let _ok2 = client.send_event(older).await.expect("send older");
    // Stale write may be rejected or accepted-as-duplicate — either way,
    // the older event must NOT replace the newer one.

    // Query — should still have the newer event
    let sid = sub_id("stale");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_LONG_FORM))
        .author(keys.public_key())
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert_eq!(events.len(), 1, "should have exactly one event");
    assert_eq!(
        events[0].id, newer_id,
        "surviving event should be the newer one"
    );
    assert!(
        events[0].content.contains("Newer"),
        "content should be from the newer event"
    );

    client.disconnect().await.expect("disconnect");
}

/// NIP-09 a-tag deletion: a kind:5 deletion targeting the addressable
/// coordinate `30023:<pubkey>:<d-tag>` causes the live event row for that
/// coordinate to be soft-deleted, so subsequent REQs no longer return it.
///
/// Regression test for issue #714 — before the fix,
/// `handle_a_tag_deletion` only handled the workflow kind and silently
/// no-op'd for kind:30023.
#[tokio::test]
#[ignore]
async fn test_long_form_a_tag_deletion() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    // Publish a note.
    let d_tag = format!("a-del-{}", uuid::Uuid::new_v4().simple());
    let note = build_long_form_event(&keys, &d_tag, "Doomed Article", "Body.", vec![]);
    let note_id = note.id;
    let ok = client.send_event(note).await.expect("send note");
    assert!(ok.accepted, "note should be accepted: {}", ok.message);

    // Sanity check it's queryable before deletion.
    let sid_pre = sub_id("a-del-pre");
    let filter_pre = Filter::new()
        .kind(Kind::Custom(KIND_LONG_FORM))
        .author(keys.public_key())
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);
    client
        .subscribe(&sid_pre, vec![filter_pre])
        .await
        .expect("subscribe pre");
    let pre = client
        .collect_until_eose(&sid_pre, Duration::from_secs(5))
        .await
        .expect("collect pre");
    assert!(
        pre.iter().any(|e| e.id == note_id),
        "note should be queryable before deletion"
    );

    // Build the addressable coordinate and emit a kind:5 deletion targeting it.
    let a_coord = format!(
        "{}:{}:{}",
        KIND_LONG_FORM,
        keys.public_key().to_hex(),
        d_tag
    );
    let del = EventBuilder::new(
        Kind::EventDeletion,
        "",
        vec![Tag::parse(&["a", &a_coord]).unwrap()],
    )
    .sign_with_keys(&keys)
    .unwrap();
    let ok_del = client.send_event(del).await.expect("send deletion");
    assert!(
        ok_del.accepted,
        "a-tag deletion should be accepted: {}",
        ok_del.message
    );

    // Query — should now be empty.
    let sid_post = sub_id("a-del-post");
    let filter_post = Filter::new()
        .kind(Kind::Custom(KIND_LONG_FORM))
        .author(keys.public_key())
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);
    client
        .subscribe(&sid_post, vec![filter_post])
        .await
        .expect("subscribe post");
    let post = client
        .collect_until_eose(&sid_post, Duration::from_secs(5))
        .await
        .expect("collect post");
    assert!(
        post.is_empty(),
        "a-tag deletion should remove the note from REQ results (got {} events)",
        post.len()
    );

    client.disconnect().await.expect("disconnect");
}

/// A kind:5 carrying a malformed `e` tag alongside a valid `a` coordinate must
/// NOT be routed as an addressable deletion — a malformed `e` makes the
/// deletion ambiguous, not addressable-only. Regression guard for relay
/// routing keyed on "no e tags present" rather than "no valid e-ids decoded":
/// the note must survive.
#[tokio::test]
#[ignore]
async fn test_long_form_malformed_e_plus_a_does_not_delete() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let d_tag = format!("mixed-del-{}", uuid::Uuid::new_v4().simple());
    let note = build_long_form_event(&keys, &d_tag, "Survivor", "Body.", vec![]);
    let note_id = note.id;
    let ok = client.send_event(note).await.expect("send note");
    assert!(ok.accepted, "note should be accepted: {}", ok.message);

    // kind:5 with a *malformed* e tag (not 64 hex chars) plus a valid a coord.
    let a_coord = format!(
        "{}:{}:{}",
        KIND_LONG_FORM,
        keys.public_key().to_hex(),
        d_tag
    );
    let del = EventBuilder::new(
        Kind::EventDeletion,
        "",
        vec![
            Tag::parse(&["e", "not-a-valid-event-id"]).unwrap(),
            Tag::parse(&["a", &a_coord]).unwrap(),
        ],
    )
    .sign_with_keys(&keys)
    .unwrap();
    // Relay may accept-and-noop or reject; either is fine. The contract under
    // test is that the coordinate is NOT soft-deleted.
    let _ = client.send_event(del).await.expect("send mixed deletion");

    let sid = sub_id("mixed-del-post");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_LONG_FORM))
        .author(keys.public_key())
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let post = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");
    assert!(
        post.iter().any(|e| e.id == note_id),
        "malformed-e + a must NOT soft-delete the coordinate; note should survive"
    );

    client.disconnect().await.expect("disconnect");
}

/// `notes set` re-publish preserves the original `published_at` while letting
/// `created_at` advance. This is the contract that NIP-23 readers rely on to
/// tell "when the author first wrote this" from "when they last updated it",
/// and the carry-forward logic in `sprout-cli`'s `build_set_event` (unit-tested
/// there) only works if the relay round-trips the tag faithfully.
///
/// The carry rule is duplicated inline here (rather than reaching into
/// `sprout-cli`) so this e2e crate stays free of CLI deps; the rule's
/// correctness is unit-tested in `commands::notes::tests`.
#[tokio::test]
#[ignore]
async fn test_long_form_set_twice_preserves_published_at() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = SproutTestClient::connect(&url, &keys)
        .await
        .expect("connect");

    let d_tag = format!("preserve-pat-{}", uuid::Uuid::new_v4().simple());
    let original_published_at: u64 = 1_700_000_000;

    // First publish: stamp `published_at` = original_published_at.
    let v1 = build_long_form_event(
        &keys,
        &d_tag,
        "First",
        "v1 body",
        vec![Tag::parse(&["published_at", &original_published_at.to_string()]).unwrap()],
    );
    let ok1 = client.send_event(v1).await.expect("send v1");
    assert!(ok1.accepted, "v1 should be accepted: {}", ok1.message);

    // Ensure created_at advances between writes.
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Re-publish carrying the original `published_at` forward — what
    // `notes set` does on update when `--title` (or nothing) changes.
    let v2 = EventBuilder::new(
        Kind::Custom(KIND_LONG_FORM),
        "v2 body",
        vec![
            Tag::parse(&["d", &d_tag]).unwrap(),
            Tag::parse(&["title", "First"]).unwrap(),
            Tag::parse(&["published_at", &original_published_at.to_string()]).unwrap(),
        ],
    )
    .custom_created_at(Timestamp::now())
    .sign_with_keys(&keys)
    .unwrap();
    let v2_id = v2.id;
    let v2_created_at = v2.created_at.as_u64();
    let ok2 = client.send_event(v2).await.expect("send v2");
    assert!(ok2.accepted, "v2 should be accepted: {}", ok2.message);

    // Re-fetch: there should be exactly one live event for (kind, author, d-tag),
    // and its `published_at` should still be the original — even though
    // `created_at` advanced.
    let sid = sub_id("preserve-pat");
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_LONG_FORM))
        .author(keys.public_key())
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), [d_tag.as_str()]);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");

    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert_eq!(events.len(), 1, "exactly one live event after re-publish");
    let live = &events[0];
    assert_eq!(live.id, v2_id, "surviving event is v2");
    assert_eq!(
        live.created_at.as_u64(),
        v2_created_at,
        "created_at advanced to v2's timestamp"
    );
    let pa = live
        .tags
        .iter()
        .find(|t| t.as_slice().first().map(String::as_str) == Some("published_at"))
        .and_then(|t| t.as_slice().get(1).cloned())
        .and_then(|v| v.parse::<u64>().ok());
    assert_eq!(
        pa,
        Some(original_published_at),
        "published_at must be preserved across re-publish"
    );

    client.disconnect().await.expect("disconnect");
}
