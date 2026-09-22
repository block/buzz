//! End-to-end tests for kind:30178 team-catalog events (NIP-AP).
//!
//! Kind 30178 is the shareable projection of a team. It joins kind:30175 in
//! `SHARED_GATED_KINDS`, so these tests assert the wire behaviour of that gate
//! at every read chokepoint (REQ, `ids` lookup, COUNT, live fan-out) plus the
//! ingest envelope rules that make the gate sound:
//! - Exactly one non-empty, bounded `d` tag — the team's stable local id, which
//!   may contain a colon (`builtin-team:welcome`) unlike a persona slug.
//! - `shared`, if present, is exactly `["shared", "true"]`.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test --test e2e_team_catalog -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};

const TEAM_CATALOG_KIND: u16 = 30178;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-team-catalog-{name}-{}", uuid::Uuid::new_v4())
}

fn catalog_content(name: &str) -> String {
    serde_json::json!({ "v": 1, "name": name, "members": [] }).to_string()
}

/// Build a kind:30178 event, optionally carrying the `["shared","true"]` opt-in.
fn catalog_event(keys: &Keys, d_tag: &str, shared: bool) -> nostr::Event {
    catalog_event_at(keys, d_tag, shared, Timestamp::now().as_secs())
}

/// Same as [`catalog_event`] with an explicit `created_at`, so NIP-33 head
/// ordering is deterministic instead of resolved by event-id tie-break.
fn catalog_event_at(keys: &Keys, d_tag: &str, shared: bool, created_at: u64) -> nostr::Event {
    let mut tags = vec![Tag::parse(["d", d_tag]).unwrap()];
    if shared {
        tags.push(Tag::parse(["shared", "true"]).unwrap());
    }
    EventBuilder::new(
        Kind::Custom(TEAM_CATALOG_KIND),
        catalog_content("Test Team"),
    )
    .tags(tags)
    .custom_created_at(Timestamp::from(created_at))
    .sign_with_keys(keys)
    .unwrap()
}

fn author_filter(author: &Keys) -> Filter {
    Filter::new()
        .kind(Kind::Custom(TEAM_CATALOG_KIND))
        .author(author.public_key())
}

fn coordinate_filter(author: &Keys, d_tag: &str) -> Filter {
    author_filter(author).custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag])
}

fn d_tag_of(event: &nostr::Event) -> Option<&str> {
    event.tags.iter().find_map(|t| {
        let parts = t.as_slice();
        if parts.first().map(|p| p.as_str()) != Some("d") {
            return None;
        }
        Some(parts.get(1)?.as_str())
    })
}

/// The author's own unshared projection round-trips at its NIP-33 coordinate.
///
/// The `d` tag is a UUID, matching the desktop team id — proof the envelope does
/// NOT apply the persona slug grammar.
#[tokio::test]
#[ignore]
async fn test_team_catalog_publish_and_query_own_unshared() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let event = catalog_event(&keys, &d_tag, false);
    let event_id = event.id;
    let ok = client.send_event(event).await.expect("send catalog");
    assert!(ok.accepted, "relay rejected catalog event: {}", ok.message);

    let sid = sub_id("own-unshared");
    client
        .subscribe(&sid, vec![coordinate_filter(&keys, &d_tag)])
        .await
        .expect("subscribe");
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert_eq!(events.len(), 1, "author must see own unshared projection");
    assert_eq!(events[0].id, event_id);

    client.disconnect().await.expect("disconnect");
}

/// A built-in team id (`builtin-team:welcome`) is accepted as the `d` tag.
///
/// The colon is illegal in a persona slug; rewriting the id to fit would break
/// NIP-33 addressing against the team's own kind:30176 head.
#[tokio::test]
#[ignore]
async fn test_team_catalog_accepts_builtin_colon_d_tag() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = format!("builtin-team:{}", &uuid::Uuid::new_v4().to_string()[..8]);

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let ok = client
        .send_event(catalog_event(&keys, &d_tag, true))
        .await
        .expect("send catalog");
    assert!(
        ok.accepted,
        "relay rejected colon-bearing team id: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// Ingest refuses an empty `d` tag: generic NIP-33 storage maps it to the empty
/// coordinate, collapsing every team into one `(pubkey, 30178, "")` slot.
#[tokio::test]
#[ignore]
async fn test_team_catalog_rejects_empty_d_tag() {
    let url = relay_url();
    let keys = Keys::generate();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let ok = client
        .send_event(catalog_event(&keys, "", false))
        .await
        .expect("send catalog");
    assert!(!ok.accepted, "empty d-tag must be rejected");
    assert!(
        ok.message.contains("invalid:"),
        "expected an `invalid:` refusal, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// Ingest refuses a valueless `["d"]` tag alongside a valued one. Counting only
/// tags that carry a value would see exactly one `d` here and accept the event;
/// a NIP-33 consumer that reads `["d"]` as an empty-valued first `d` tag would
/// then address the event at `""` where this relay addresses it at the team id.
#[tokio::test]
#[ignore]
async fn test_team_catalog_rejects_valueless_plus_valued_d_tags() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = EventBuilder::new(
        Kind::Custom(TEAM_CATALOG_KIND),
        catalog_content("Two d tags"),
    )
    .tags(vec![
        Tag::parse(["d"]).unwrap(),
        Tag::parse(["d", d_tag.as_str()]).unwrap(),
    ])
    .sign_with_keys(&keys)
    .unwrap();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let ok = client.send_event(event).await.expect("send catalog");
    assert!(
        !ok.accepted,
        "a valueless `d` tag must count toward the exactly-one rule"
    );
    assert!(
        ok.message.contains("invalid:"),
        "expected an `invalid:` refusal, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// Ingest refuses a malformed `shared` tag. A three-element tag would satisfy
/// the SQL containment clause `tags @> '[["shared","true"]]'` as a superset
/// while the in-process gate reads it as unshared — the two layers must agree,
/// so such an event can never be stored.
#[tokio::test]
#[ignore]
async fn test_team_catalog_rejects_three_element_shared_tag() {
    let url = relay_url();
    let keys = Keys::generate();
    let d_tag = uuid::Uuid::new_v4().to_string();

    let event = EventBuilder::new(
        Kind::Custom(TEAM_CATALOG_KIND),
        catalog_content("Malformed"),
    )
    .tags(vec![
        Tag::parse(["d", d_tag.as_str()]).unwrap(),
        Tag::parse(["shared", "true", "extra"]).unwrap(),
    ])
    .sign_with_keys(&keys)
    .unwrap();

    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");
    let ok = client.send_event(event).await.expect("send catalog");
    assert!(!ok.accepted, "three-element shared tag must be rejected");
    assert!(
        ok.message.contains("invalid:"),
        "expected an `invalid:` refusal, got: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect");
}

/// REQ historical delivery: a foreign reader receives only shared projections,
/// while the author receives both of their own.
#[tokio::test]
#[ignore]
async fn test_team_catalog_foreign_sees_only_shared() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_unshared = format!("priv-{}", uuid::Uuid::new_v4());
    let d_shared = format!("pub-{}", uuid::Uuid::new_v4());

    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let shared_event = catalog_event(&author_keys, &d_shared, true);
    let shared_id = shared_event.id;
    let ok = author
        .send_event(catalog_event(&author_keys, &d_unshared, false))
        .await
        .expect("send unshared");
    assert!(ok.accepted, "unshared ingest rejected: {}", ok.message);
    let ok = author.send_event(shared_event).await.expect("send shared");
    assert!(ok.accepted, "shared ingest rejected: {}", ok.message);

    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("fg-all");
    foreign
        .subscribe(&sid, vec![author_filter(&author_keys)])
        .await
        .expect("subscribe");
    let events = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        !events
            .iter()
            .any(|e| d_tag_of(e) == Some(d_unshared.as_str())),
        "foreign reader must NOT see the unshared projection"
    );
    assert!(
        events.iter().any(|e| e.id == shared_id),
        "foreign reader must see the shared projection"
    );

    let sid_author = sub_id("auth-all");
    author
        .subscribe(&sid_author, vec![author_filter(&author_keys)])
        .await
        .expect("subscribe author");
    let author_events = author
        .collect_until_eose(&sid_author, Duration::from_secs(5))
        .await
        .expect("collect author");
    assert!(
        author_events.len() >= 2,
        "author must see both own projections, got {}",
        author_events.len()
    );

    author.disconnect().await.expect("disconnect author");
    foreign.disconnect().await.expect("disconnect foreign");
}

/// Knowing an event id does NOT grant access: `{ids:[unshared]}` returns nothing
/// to a foreign reader.
#[tokio::test]
#[ignore]
async fn test_team_catalog_ids_lookup_unshared_returns_nothing_to_foreign() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let event = catalog_event(&author_keys, &uuid::Uuid::new_v4().to_string(), false);
    let event_id = event.id;

    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ok = author.send_event(event).await.expect("send");
    assert!(ok.accepted, "ingest rejected: {}", ok.message);
    author.disconnect().await.expect("disconnect author");

    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("ids-unshared");
    foreign
        .subscribe(&sid, vec![Filter::new().id(event_id)])
        .await
        .expect("subscribe");
    let events = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    assert!(
        events.is_empty(),
        "ids-lookup of an unshared projection must return nothing, got {:?}",
        events.iter().map(|e| e.id).collect::<Vec<_>>()
    );

    foreign.disconnect().await.expect("disconnect foreign");
}

/// COUNT must take the per-event fallback for kind:30178 so the aggregate does
/// not leak the existence of unshared projections.
#[tokio::test]
#[ignore]
async fn test_team_catalog_count_excludes_foreign_unshared() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");
    let ok = author
        .send_event(catalog_event(
            &author_keys,
            &uuid::Uuid::new_v4().to_string(),
            false,
        ))
        .await
        .expect("send unshared");
    assert!(ok.accepted, "unshared rejected: {}", ok.message);
    let ok = author
        .send_event(catalog_event(
            &author_keys,
            &uuid::Uuid::new_v4().to_string(),
            true,
        ))
        .await
        .expect("send shared");
    assert!(ok.accepted, "shared rejected: {}", ok.message);
    author.disconnect().await.expect("disconnect author");

    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("count");
    let count_msg = serde_json::json!(["COUNT", sid, author_filter(&author_keys)]);
    foreign.send_raw(&count_msg).await.expect("send COUNT");

    let count = match foreign.recv_event(Duration::from_secs(5)).await {
        Ok(RelayMessage::Count { count, .. }) => count,
        Ok(RelayMessage::Closed { message, .. }) => panic!("COUNT closed unexpectedly: {message}"),
        Ok(other) => panic!("unexpected relay message for COUNT: {other:?}"),
        Err(e) => panic!("unexpected error for COUNT: {e}"),
    };
    assert_eq!(
        count, 1,
        "foreign COUNT must see only the shared projection, got {count}"
    );

    foreign.disconnect().await.expect("disconnect foreign");
}

/// Live fan-out honours the gate, and unsharing (a NIP-33 replacement that drops
/// the `shared` tag) retracts the projection from foreign readers.
#[tokio::test]
#[ignore]
async fn test_team_catalog_live_fanout_and_unshare_retracts() {
    let url = relay_url();
    let author_keys = Keys::generate();
    let foreign_keys = Keys::generate();

    let d_tag = uuid::Uuid::new_v4().to_string();
    let now = Timestamp::now().as_secs();
    let (t0, t1, t2) = (now.saturating_sub(2), now.saturating_sub(1), now);

    // Subscribe BEFORE publishing, scoped to this author so parallel tests
    // publishing their own 30178s cannot trip the leak assertion.
    let mut foreign = BuzzTestClient::connect(&url, &foreign_keys)
        .await
        .expect("connect foreign");
    let sid = sub_id("fanout");
    foreign
        .subscribe(&sid, vec![author_filter(&author_keys)])
        .await
        .expect("subscribe");
    let _ = foreign
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("drain eose");

    let mut author = BuzzTestClient::connect(&url, &author_keys)
        .await
        .expect("connect author");

    // Unshared publish must NOT reach the foreign connection.
    let ok = author
        .send_event(catalog_event_at(&author_keys, &d_tag, false, t0))
        .await
        .expect("send unshared");
    assert!(ok.accepted, "unshared rejected: {}", ok.message);
    match foreign.recv_event(Duration::from_millis(750)).await {
        Err(buzz_test_client::TestClientError::Timeout) => {}
        Ok(RelayMessage::Event { event, .. }) if event.kind == Kind::Custom(TEAM_CATALOG_KIND) => {
            panic!("unshared projection leaked to foreign live subscription");
        }
        Ok(_) => {}
        Err(e) => panic!("unexpected error awaiting fan-out: {e}"),
    }

    // Shared replacement MUST reach it.
    let shared_event = catalog_event_at(&author_keys, &d_tag, true, t1);
    let shared_id = shared_event.id;
    let ok = author.send_event(shared_event).await.expect("send shared");
    assert!(ok.accepted, "shared rejected: {}", ok.message);
    let delivered = loop {
        match foreign.recv_event(Duration::from_secs(5)).await {
            Ok(RelayMessage::Event { event, .. }) if event.id == shared_id => break true,
            Ok(_) => continue,
            Err(buzz_test_client::TestClientError::Timeout) => break false,
            Err(e) => panic!("unexpected error awaiting shared fan-out: {e}"),
        }
    };
    assert!(
        delivered,
        "shared projection must fan out to foreign readers"
    );

    // Unshare: replace at the same coordinate without the tag. Subsequent
    // foreign REQs must return nothing.
    let ok = author
        .send_event(catalog_event_at(&author_keys, &d_tag, false, t2))
        .await
        .expect("send unshare");
    assert!(ok.accepted, "unshare rejected: {}", ok.message);

    let sid_post = sub_id("post-unshare");
    foreign
        .subscribe(&sid_post, vec![coordinate_filter(&author_keys, &d_tag)])
        .await
        .expect("subscribe post");
    let after = foreign
        .collect_until_eose(&sid_post, Duration::from_secs(5))
        .await
        .expect("collect post");
    assert!(
        after.is_empty(),
        "unsharing must retract the projection from foreign readers, got {} event(s)",
        after.len()
    );

    author.disconnect().await.expect("disconnect author");
    foreign.disconnect().await.expect("disconnect foreign");
}
