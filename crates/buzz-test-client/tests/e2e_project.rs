//! End-to-end tests for kind:30621 multi-repo project events (NIP-MP).
//!
//! The ingest unit tests in `buzz-relay` pin the envelope contract in isolation.
//! These tests cover the three behaviors that only exist once an event reaches
//! storage, plus proof that the envelope validator is actually wired into the
//! live write path:
//! - a valid cross-owner project round-trips through its NIP-33 coordinate;
//! - replacement is keyed by `(pubkey, 30621, d)` — newer wins for one author,
//!   and two authors sharing a `d` hold two independent projects (this is what
//!   makes owner-only editing free rather than a relay permission check);
//! - a NIP-09 `a`-tag tombstone removes the project coordinate and leaves every
//!   referenced kind:30617 announcement untouched, because membership is an
//!   assertion about repositories and never authority over them;
//! - malformed envelopes are refused by the relay, not merely by the validator.
//!
//! See `docs/nips/NIP-MP.md` for the normative contract.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_project -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};

const PROJECT_KIND: u16 = 30621;
const REPO_ANNOUNCEMENT_KIND: u16 = 30617;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-project-{name}-{}", uuid::Uuid::new_v4())
}

/// A short unique suffix so concurrent runs never collide on a `d` tag.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().to_string()[..8])
}

fn member_coord(owner: &Keys, repo_d: &str) -> String {
    format!(
        "{REPO_ANNOUNCEMENT_KIND}:{}:{repo_d}",
        owner.public_key().to_hex()
    )
}

/// Build a project event. `members` are canonical `30617:<owner>:<d>`
/// coordinates; `created_at` defaults to now when `None`.
fn project_event(
    keys: &Keys,
    d_tag: &str,
    name: &str,
    members: &[String],
    created_at: Option<u64>,
) -> nostr::Event {
    let mut tags = vec![
        Tag::parse(["d", d_tag]).unwrap(),
        Tag::parse(["name", name]).unwrap(),
    ];
    tags.extend(
        members
            .iter()
            .map(|m| Tag::parse(["a", m.as_str()]).unwrap()),
    );
    let builder = EventBuilder::new(Kind::Custom(PROJECT_KIND), "").tags(tags);
    match created_at {
        Some(ts) => builder.custom_created_at(Timestamp::from(ts)),
        None => builder,
    }
    .sign_with_keys(keys)
    .unwrap()
}

/// Announce a repository so a project has a real coordinate to reference.
fn repo_announcement(keys: &Keys, repo_d: &str) -> nostr::Event {
    EventBuilder::new(Kind::Custom(REPO_ANNOUNCEMENT_KIND), "")
        .tags(vec![
            Tag::parse(["d", repo_d]).unwrap(),
            Tag::parse(["name", repo_d]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap()
}

/// A NIP-09 `a`-tag-only deletion at a NIP-33 coordinate. No `e` tag, so the
/// relay takes the coordinate-delete path rather than the event-id path.
/// `created_at` defaults to now when `None`.
fn coordinate_delete(keys: &Keys, kind: u16, d_tag: &str, created_at: Option<u64>) -> nostr::Event {
    let coord = format!("{kind}:{}:{d_tag}", keys.public_key().to_hex());
    let builder =
        EventBuilder::new(Kind::Custom(5), "")
            .tags(vec![Tag::parse(["a", coord.as_str()]).unwrap()]);
    match created_at {
        Some(ts) => builder.custom_created_at(Timestamp::from(ts)),
        None => builder,
    }
    .sign_with_keys(keys)
    .unwrap()
}

fn addressable_filter(kind: u16, author: &Keys, d_tag: &str) -> Filter {
    Filter::new()
        .kind(Kind::Custom(kind))
        .author(author.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag])
}

/// Subscribe with `filter` and drain to EOSE.
async fn query(client: &mut BuzzTestClient, name: &str, filter: Filter) -> Vec<nostr::Event> {
    let sid = sub_id(name);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect events")
}

#[tokio::test]
#[ignore]
async fn test_project_publish_and_query_returns_cross_owner_members() {
    let url = relay_url();
    let owner = Keys::generate();
    let other = Keys::generate();
    let d_tag = unique("project");

    let members = vec![
        member_coord(&owner, "buzz"),
        member_coord(&other, "buzz-infra"),
    ];

    let mut client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect");

    let event = project_event(&owner, &d_tag, "Platform", &members, None);
    let ok = client.send_event(event).await.expect("send project");
    assert!(ok.accepted, "relay rejected project event: {}", ok.message);

    let events = query(
        &mut client,
        "query",
        addressable_filter(PROJECT_KIND, &owner, &d_tag),
    )
    .await;

    assert_eq!(events.len(), 1, "expected exactly one project event");
    let stored: Vec<&str> = events[0]
        .tags
        .iter()
        .filter_map(|t| {
            let parts = t.as_slice();
            (parts.first().map(|s| s.as_str()) == Some("a")).then(|| parts[1].as_str())
        })
        .collect();
    assert_eq!(
        stored, members,
        "both members must survive the round trip, including the one owned by another pubkey"
    );

    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn test_project_replacement_keeps_only_newest_for_same_author_and_d() {
    let url = relay_url();
    let owner = Keys::generate();
    let d_tag = unique("project-replace");
    let now = Timestamp::now().as_secs();

    let mut client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect");

    let first = project_event(&owner, &d_tag, "Old", &[], Some(now - 100));
    let ok = client.send_event(first).await.expect("send old");
    assert!(ok.accepted, "relay rejected old project: {}", ok.message);

    let members = vec![member_coord(&owner, "buzz")];
    let second = project_event(&owner, &d_tag, "New", &members, Some(now));
    let ok = client.send_event(second).await.expect("send new");
    assert!(ok.accepted, "relay rejected new project: {}", ok.message);

    let events = query(
        &mut client,
        "replace",
        addressable_filter(PROJECT_KIND, &owner, &d_tag),
    )
    .await;

    assert_eq!(
        events.len(),
        1,
        "NIP-33: only the newest head should remain"
    );
    let name = events[0]
        .tags
        .iter()
        .find_map(|t| {
            let parts = t.as_slice();
            (parts.first().map(|s| s.as_str()) == Some("name")).then(|| parts[1].as_str())
        })
        .expect("name tag");
    assert_eq!(name, "New", "the newer head must win");

    client.disconnect().await.expect("disconnect");
}

/// Owner-only editing is a property of the addressable model, not a relay
/// permission check: two authors publishing the same `d` occupy two coordinates,
/// so neither can overwrite the other. This is the test that would fail if the
/// kind were ever classified as plain-replaceable or keyed on `d` alone.
#[tokio::test]
#[ignore]
async fn test_project_same_d_under_two_authors_are_independent() {
    let url = relay_url();
    let alice = Keys::generate();
    let bob = Keys::generate();
    let d_tag = unique("project-shared-d");

    let mut alice_client = BuzzTestClient::connect(&url, &alice)
        .await
        .expect("connect");
    let ok = alice_client
        .send_event(project_event(&alice, &d_tag, "Alice", &[], None))
        .await
        .expect("send alice");
    assert!(
        ok.accepted,
        "relay rejected alice's project: {}",
        ok.message
    );

    let mut bob_client = BuzzTestClient::connect(&url, &bob).await.expect("connect");
    let ok = bob_client
        .send_event(project_event(&bob, &d_tag, "Bob", &[], None))
        .await
        .expect("send bob");
    assert!(ok.accepted, "relay rejected bob's project: {}", ok.message);

    for (label, keys, expected_name) in [("alice", &alice, "Alice"), ("bob", &bob, "Bob")] {
        let events = query(
            &mut alice_client,
            label,
            addressable_filter(PROJECT_KIND, keys, &d_tag),
        )
        .await;
        assert_eq!(
            events.len(),
            1,
            "{label} should still hold their own project at the shared `d`"
        );
        let name = events[0]
            .tags
            .iter()
            .find_map(|t| {
                let parts = t.as_slice();
                (parts.first().map(|s| s.as_str()) == Some("name")).then(|| parts[1].as_str())
            })
            .expect("name tag");
        assert_eq!(name, expected_name, "{label}'s project was overwritten");
    }

    alice_client.disconnect().await.expect("disconnect");
    bob_client.disconnect().await.expect("disconnect");
}

/// Deleting a project must delete only the grouping. A project is metadata about
/// repositories; if a tombstone at the project coordinate cascaded to the
/// referenced kind:30617s, adding a repo to someone's project would become a way
/// to destroy it.
#[tokio::test]
#[ignore]
async fn test_project_tombstone_deletes_coordinate_and_spares_members() {
    let url = relay_url();
    let owner = Keys::generate();
    let repo_d = unique("repo");
    let project_d = unique("project-tombstone");

    let mut client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect");

    let ok = client
        .send_event(repo_announcement(&owner, &repo_d))
        .await
        .expect("send announcement");
    assert!(ok.accepted, "relay rejected announcement: {}", ok.message);

    let members = vec![member_coord(&owner, &repo_d)];
    let ok = client
        .send_event(project_event(&owner, &project_d, "Doomed", &members, None))
        .await
        .expect("send project");
    assert!(ok.accepted, "relay rejected project: {}", ok.message);

    let before = query(
        &mut client,
        "tombstone-pre",
        addressable_filter(PROJECT_KIND, &owner, &project_d),
    )
    .await;
    assert_eq!(before.len(), 1, "project should be live before deletion");

    let ok = client
        .send_event(coordinate_delete(&owner, PROJECT_KIND, &project_d, None))
        .await
        .expect("send tombstone");
    assert!(ok.accepted, "relay rejected tombstone: {}", ok.message);

    let after = query(
        &mut client,
        "tombstone-post",
        addressable_filter(PROJECT_KIND, &owner, &project_d),
    )
    .await;
    assert!(
        after.is_empty(),
        "tombstone should remove the project coordinate, got {} event(s)",
        after.len()
    );

    let repo = query(
        &mut client,
        "member-after",
        addressable_filter(REPO_ANNOUNCEMENT_KIND, &owner, &repo_d),
    )
    .await;
    assert_eq!(
        repo.len(),
        1,
        "deleting a project must not touch the repositories it referenced"
    );

    client.disconnect().await.expect("disconnect");
}

/// NIP-09 scopes an `a`-tag deletion to versions at or before the deletion's own
/// `created_at`. A tombstone signed between V1 and V2 — delayed in transit or
/// replayed by a third party — must therefore retire V1 only and leave the newer
/// V2 head live. Before the timestamp predicate landed in
/// `soft_delete_by_coordinate`, the coordinate delete was timestamp-blind and
/// this sequence silently destroyed V2.
#[tokio::test]
#[ignore]
async fn test_stale_tombstone_between_versions_leaves_newer_project_live() {
    let url = relay_url();
    let owner = Keys::generate();
    let project_d = unique("project-stale-tombstone");
    let now = Timestamp::now().as_secs();

    let mut client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect");

    let ok = client
        .send_event(project_event(
            &owner,
            &project_d,
            "V1",
            &[],
            Some(now - 100),
        ))
        .await
        .expect("send v1");
    assert!(ok.accepted, "relay rejected V1: {}", ok.message);

    let ok = client
        .send_event(project_event(&owner, &project_d, "V2", &[], Some(now)))
        .await
        .expect("send v2");
    assert!(ok.accepted, "relay rejected V2: {}", ok.message);

    // Timestamped strictly between V1 and V2: valid for V1, stale for V2.
    let ok = client
        .send_event(coordinate_delete(
            &owner,
            PROJECT_KIND,
            &project_d,
            Some(now - 50),
        ))
        .await
        .expect("send stale tombstone");
    assert!(
        ok.accepted,
        "a well-formed tombstone is still an acceptable event: {}",
        ok.message
    );

    let after = query(
        &mut client,
        "stale-tombstone",
        addressable_filter(PROJECT_KIND, &owner, &project_d),
    )
    .await;

    assert_eq!(
        after.len(),
        1,
        "a tombstone older than the live head must not delete it, got {} event(s)",
        after.len()
    );
    let name = after[0]
        .tags
        .iter()
        .find_map(|t| {
            let parts = t.as_slice();
            (parts.first().map(|s| s.as_str()) == Some("name")).then(|| parts[1].as_str())
        })
        .expect("surviving head must carry its name tag");
    assert_eq!(name, "V2", "the surviving head must be the newer version");

    client.disconnect().await.expect("disconnect");
}

/// Proves the envelope validator is reachable from the live write path — a unit
/// test of `validate_project_envelope` cannot show that ingest calls it.
#[tokio::test]
#[ignore]
async fn test_project_malformed_envelope_rejected_by_relay() {
    let url = relay_url();
    let owner = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &owner)
        .await
        .expect("connect");

    let duplicate = member_coord(&owner, "buzz");
    // Each case pairs a malformed event with the substring its rejection must
    // carry, so a refusal for an unrelated reason cannot satisfy the assertion.
    let cases: Vec<(&str, nostr::Event, &str)> = vec![
        (
            "duplicate member coordinate",
            project_event(
                &owner,
                &unique("project-dup"),
                "Dup",
                &[duplicate.clone(), duplicate],
                None,
            ),
            "duplicate member coordinate",
        ),
        (
            "member coordinate naming the wrong kind",
            project_event(
                &owner,
                &unique("project-badkind"),
                "Bad kind",
                &[format!("30618:{}:buzz", owner.public_key().to_hex())],
                None,
            ),
            "member `a` tag must be",
        ),
        (
            "member coordinate with an uppercase-hex owner",
            project_event(
                &owner,
                &unique("project-upper"),
                "Uppercase",
                &[format!("{REPO_ANNOUNCEMENT_KIND}:{}:buzz", "A".repeat(64))],
                None,
            ),
            "member `a` tag must be",
        ),
    ];

    for (label, event, expected) in cases {
        let ok = client.send_event(event).await.expect("send");
        assert!(
            !ok.accepted,
            "relay must reject a project with a {label}, got OK: {}",
            ok.message
        );
        assert!(
            ok.message.contains(expected),
            "rejection for {label} must name the rule that fired, got: {}",
            ok.message
        );
    }

    client.disconnect().await.expect("disconnect");
}
