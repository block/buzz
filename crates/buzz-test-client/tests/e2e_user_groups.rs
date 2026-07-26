//! End-to-end integration tests for relay-managed user groups.
//!
//! These tests require a running relay, Postgres, and Redis. They are ignored
//! by default so infrastructure-free test runs can still compile the suite.
//!
//! # Running
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_user_groups -- --ignored
//! ```

use std::time::Duration;

use buzz_sdk::{
    build_group_add_members, build_group_create, build_group_delete, build_group_edit,
    build_group_remove_members,
};
use buzz_test_client::BuzzTestClient;
use nostr::{Alphabet, Event, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use uuid::Uuid;

const KIND_GROUP_CREATE: u16 = 47_000;
const KIND_GROUP_STATE: u16 = 39_100;
const KIND_CHANNEL_MEMBERS: u16 = 39_002;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

fn relay_host() -> String {
    let relay = url::Url::parse(&relay_url()).expect("parse RELAY_URL");
    let host = relay.host_str().expect("RELAY_URL has a host");
    relay
        .port()
        .map_or_else(|| host.to_string(), |port| format!("{host}:{port}"))
}

fn sub_id(name: &str) -> String {
    format!("e2e-user-groups-{name}-{}", Uuid::new_v4())
}

/// Unique group handle that fits the 32-char handle limit.
fn unique_handle(prefix: &str) -> String {
    let hex = Uuid::new_v4().simple().to_string();
    format!("{prefix}-{}", &hex[..12])
}

async fn e2e_db_pool() -> sqlx::Pool<sqlx::Postgres> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string()); // sadscan:disable np.postgres.1
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect to e2e Postgres")
}

async fn ensure_test_community() -> Uuid {
    let pool = e2e_db_pool().await;
    let host = relay_host();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO communities (id, host) \
         VALUES ($1, $2) \
         ON CONFLICT (lower(host)) DO NOTHING",
    )
    .bind(id)
    .bind(&host)
    .execute(&pool)
    .await
    .unwrap_or_else(|error| panic!("seed community {host}: {error}"));

    sqlx::query_scalar("SELECT id FROM communities WHERE lower(host) = lower($1)")
        .bind(&host)
        .fetch_one(&pool)
        .await
        .unwrap_or_else(|error| panic!("lookup community {host}: {error}"))
}

async fn seed_community_member(keys: &Keys, role: &str) {
    let pool = e2e_db_pool().await;
    let community_id = ensure_test_community().await;
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, $3, NULL) \
         ON CONFLICT (community_id, pubkey) DO UPDATE \
         SET role = $3, updated_at = now()",
    )
    .bind(community_id)
    .bind(keys.public_key().to_hex())
    .bind(role)
    .execute(&pool)
    .await
    .unwrap_or_else(|error| panic!("seed relay member {role}: {error}"));

    // Community admission on open relays (require_relay_membership=false, the
    // dev default) checks the users table, so seed that row as well.
    sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(community_id)
        .bind(keys.public_key().to_bytes().to_vec())
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("seed user row {role}: {error}"));
}

async fn create_channel(client: &mut BuzzTestClient, keys: &Keys, visibility: &str) -> Uuid {
    let channel_id = Uuid::new_v4();
    let channel_id_text = channel_id.to_string();
    let channel_name = format!("user-groups-e2e-{}", channel_id.simple());
    let event = EventBuilder::new(Kind::Custom(9_007), "")
        .tags([
            Tag::parse(["h", &channel_id_text]).expect("h tag"),
            Tag::parse(["name", &channel_name]).expect("name tag"),
            Tag::parse(["channel_type", "stream"]).expect("channel type tag"),
            Tag::parse(["visibility", visibility]).expect("visibility tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign create-channel event");

    let ok = client
        .send_event(event)
        .await
        .expect("submit create-channel event");
    assert!(
        ok.accepted,
        "channel creation should be accepted: {}",
        ok.message
    );
    channel_id
}

fn tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == name)
        .filter_map(|tag| tag.content().map(ToOwned::to_owned))
        .collect()
}

fn has_tag(event: &Event, name: &str) -> bool {
    event.tags.iter().any(|tag| tag.kind().to_string() == name)
}

fn group_id_from_create(event: &Event) -> Uuid {
    let group_id = tag_values(event, "g")
        .into_iter()
        .next()
        .expect("group create has a g tag");
    Uuid::parse_str(&group_id).expect("g tag contains a UUID")
}

async fn query_snapshot(client: &mut BuzzTestClient, kind: u16, d_tag: &str) -> Event {
    let sid = sub_id("snapshot");
    let filter = Filter::new()
        .kind(Kind::Custom(kind))
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [d_tag]);
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe to snapshot");
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect snapshot");
    assert_eq!(
        events.len(),
        1,
        "expected exactly one kind:{kind} snapshot for d={d_tag}"
    );
    events.into_iter().next().expect("one snapshot")
}

async fn channel_has_member(channel_id: Uuid, keys: &Keys) -> bool {
    let pool = e2e_db_pool().await;
    let community_id = ensure_test_community().await;
    sqlx::query_scalar(
        "SELECT EXISTS (\
            SELECT 1 FROM channel_members \
            WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3 \
              AND removed_at IS NULL\
         )",
    )
    .bind(community_id)
    .bind(channel_id)
    .bind(keys.public_key().to_bytes().to_vec())
    .fetch_one(&pool)
    .await
    .expect("query active channel membership")
}

#[tokio::test]
#[ignore]
async fn test_group_create_publishes_complete_snapshot() {
    let creator = Keys::generate();
    let member = Keys::generate();
    seed_community_member(&creator, "member").await;
    seed_community_member(&member, "member").await;

    let mut client = BuzzTestClient::connect(&relay_url(), &creator)
        .await
        .expect("connect creator");
    let channel_id = create_channel(&mut client, &creator, "open").await;
    let member_hex = member.public_key().to_hex();
    let create = build_group_create(
        &unique_handle("ios"),
        "iOS Team",
        Some("People who build the iOS app"),
        &[&member_hex],
        &[channel_id],
    )
    .expect("build group create")
    .sign_with_keys(&creator)
    .expect("sign group create");
    let group_id = group_id_from_create(&create);
    let handle = tag_values(&create, "handle")
        .into_iter()
        .next()
        .expect("handle tag");

    let ok = client.send_event(create).await.expect("send group create");
    assert!(
        ok.accepted,
        "group creation should be accepted: {}",
        ok.message
    );

    let snapshot = query_snapshot(&mut client, KIND_GROUP_STATE, &group_id.to_string()).await;
    assert_eq!(snapshot.kind, Kind::Custom(KIND_GROUP_STATE));
    assert_eq!(tag_values(&snapshot, "d"), vec![group_id.to_string()]);
    assert_eq!(tag_values(&snapshot, "handle"), vec![handle]);
    assert_eq!(tag_values(&snapshot, "name"), vec!["iOS Team"]);
    assert_eq!(
        tag_values(&snapshot, "creator"),
        vec![creator.public_key().to_hex()]
    );
    assert_eq!(tag_values(&snapshot, "p"), vec![member_hex]);
    assert_eq!(
        tag_values(&snapshot, "channel"),
        vec![channel_id.to_string()]
    );

    client.disconnect().await.expect("disconnect creator");
}

#[tokio::test]
#[ignore]
async fn test_duplicate_group_handle_is_rejected() {
    let creator = Keys::generate();
    seed_community_member(&creator, "member").await;
    let mut client = BuzzTestClient::connect(&relay_url(), &creator)
        .await
        .expect("connect creator");
    let handle = unique_handle("duplicate");

    for expected_accepted in [true, false] {
        let event = build_group_create(&handle, "Duplicate Handle", None, &[], &[])
            .expect("build group create")
            .sign_with_keys(&creator)
            .expect("sign group create");
        let ok = client.send_event(event).await.expect("send group create");
        assert_eq!(
            ok.accepted, expected_accepted,
            "unexpected duplicate-handle result: {}",
            ok.message
        );
        if !expected_accepted {
            assert!(
                ok.message.contains("duplicate"),
                "duplicate rejection should explain the conflict: {}",
                ok.message
            );
        }
    }

    client.disconnect().await.expect("disconnect creator");
}

#[tokio::test]
#[ignore]
async fn test_invalid_group_handle_is_rejected() {
    let creator = Keys::generate();
    seed_community_member(&creator, "member").await;
    let mut client = BuzzTestClient::connect(&relay_url(), &creator)
        .await
        .expect("connect creator");
    let group_id = Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_GROUP_CREATE), "")
        .tags([
            Tag::parse(["g", &group_id]).expect("g tag"),
            Tag::parse(["handle", "Invalid Handle!"]).expect("handle tag"),
            Tag::parse(["name", "Invalid"]).expect("name tag"),
        ])
        .sign_with_keys(&creator)
        .expect("sign invalid group create");

    let ok = client
        .send_event(event)
        .await
        .expect("send invalid group create");
    assert!(!ok.accepted, "invalid handle must be rejected");
    assert!(
        ok.message.contains("group handle"),
        "invalid-handle rejection should be specific: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect creator");
}

#[tokio::test]
#[ignore]
async fn test_group_edit_requires_creator_or_community_admin() {
    let creator = Keys::generate();
    let unrelated_member = Keys::generate();
    let admin = Keys::generate();
    seed_community_member(&creator, "member").await;
    seed_community_member(&unrelated_member, "member").await;
    seed_community_member(&admin, "admin").await;

    let create = build_group_create(&unique_handle("edit"), "Original Name", None, &[], &[])
        .expect("build group create")
        .sign_with_keys(&creator)
        .expect("sign group create");
    let group_id = group_id_from_create(&create);
    let mut creator_client = BuzzTestClient::connect(&relay_url(), &creator)
        .await
        .expect("connect creator");
    let ok = creator_client
        .send_event(create)
        .await
        .expect("send group create");
    assert!(ok.accepted, "group create rejected: {}", ok.message);
    creator_client
        .disconnect()
        .await
        .expect("disconnect creator");

    let mut member_client = BuzzTestClient::connect(&relay_url(), &unrelated_member)
        .await
        .expect("connect unrelated member");
    let unrelated_edit = build_group_edit(group_id, None, Some("Unauthorized Edit"), None, None)
        .expect("build unrelated edit")
        .sign_with_keys(&unrelated_member)
        .expect("sign unrelated edit");
    let ok = member_client
        .send_event(unrelated_edit)
        .await
        .expect("send unrelated edit");
    assert!(!ok.accepted, "unrelated member must not edit the group");
    assert!(
        ok.message.contains("creator") && ok.message.contains("admin"),
        "authorization rejection should explain who can edit: {}",
        ok.message
    );
    member_client
        .disconnect()
        .await
        .expect("disconnect unrelated member");

    let mut admin_client = BuzzTestClient::connect(&relay_url(), &admin)
        .await
        .expect("connect admin");
    let admin_edit = build_group_edit(group_id, None, Some("Admin Edited"), None, None)
        .expect("build admin edit")
        .sign_with_keys(&admin)
        .expect("sign admin edit");
    let ok = admin_client
        .send_event(admin_edit)
        .await
        .expect("send admin edit");
    assert!(
        ok.accepted,
        "community admin should be allowed to edit: {}",
        ok.message
    );
    let snapshot = query_snapshot(&mut admin_client, KIND_GROUP_STATE, &group_id.to_string()).await;
    assert_eq!(tag_values(&snapshot, "name"), vec!["Admin Edited"]);

    admin_client.disconnect().await.expect("disconnect admin");
}

#[tokio::test]
#[ignore]
async fn test_group_membership_updates_snapshot_and_default_channel_membership() {
    let creator = Keys::generate();
    let member = Keys::generate();
    seed_community_member(&creator, "member").await;
    seed_community_member(&member, "member").await;

    let mut client = BuzzTestClient::connect(&relay_url(), &creator)
        .await
        .expect("connect creator");
    let channel_id = create_channel(&mut client, &creator, "open").await;
    let channel_id_text = channel_id.to_string();
    let before_members = query_snapshot(&mut client, KIND_CHANNEL_MEMBERS, &channel_id_text).await;

    let create = build_group_create(
        &unique_handle("autojoin"),
        "Auto Join",
        None,
        &[],
        &[channel_id],
    )
    .expect("build group create")
    .sign_with_keys(&creator)
    .expect("sign group create");
    let group_id = group_id_from_create(&create);
    let ok = client.send_event(create).await.expect("send group create");
    assert!(ok.accepted, "group create rejected: {}", ok.message);

    let member_hex = member.public_key().to_hex();
    let add = build_group_add_members(group_id, &[&member_hex])
        .expect("build add-member")
        .sign_with_keys(&creator)
        .expect("sign add-member");
    let ok = client.send_event(add).await.expect("send add-member");
    assert!(ok.accepted, "add-member rejected: {}", ok.message);

    let group_after_add =
        query_snapshot(&mut client, KIND_GROUP_STATE, &group_id.to_string()).await;
    assert_eq!(tag_values(&group_after_add, "p"), vec![member_hex.clone()]);
    assert!(
        channel_has_member(channel_id, &member).await,
        "added group member should be an active default-channel member"
    );
    let after_members = query_snapshot(&mut client, KIND_CHANNEL_MEMBERS, &channel_id_text).await;
    assert_ne!(
        after_members.id, before_members.id,
        "kind:39002 should be refreshed after the auto-join"
    );
    assert!(
        tag_values(&after_members, "p").contains(&member_hex),
        "refreshed kind:39002 should contain the auto-joined member"
    );

    let remove = build_group_remove_members(group_id, &[&member.public_key().to_hex()])
        .expect("build remove-member")
        .sign_with_keys(&creator)
        .expect("sign remove-member");
    let ok = client.send_event(remove).await.expect("send remove-member");
    assert!(ok.accepted, "remove-member rejected: {}", ok.message);
    let group_after_remove =
        query_snapshot(&mut client, KIND_GROUP_STATE, &group_id.to_string()).await;
    assert_ne!(
        group_after_remove.id, group_after_add.id,
        "remove-member should refresh the group snapshot"
    );
    assert!(
        tag_values(&group_after_remove, "p").is_empty(),
        "removed member should be absent from the group snapshot"
    );

    client.disconnect().await.expect("disconnect creator");
}

#[tokio::test]
#[ignore]
async fn test_group_delete_publishes_tombstone_snapshot() {
    let creator = Keys::generate();
    seed_community_member(&creator, "member").await;
    let mut client = BuzzTestClient::connect(&relay_url(), &creator)
        .await
        .expect("connect creator");
    let create = build_group_create(&unique_handle("delete"), "Delete Me", None, &[], &[])
        .expect("build group create")
        .sign_with_keys(&creator)
        .expect("sign group create");
    let group_id = group_id_from_create(&create);
    let ok = client.send_event(create).await.expect("send group create");
    assert!(ok.accepted, "group create rejected: {}", ok.message);

    let delete = build_group_delete(group_id)
        .expect("build group delete")
        .sign_with_keys(&creator)
        .expect("sign group delete");
    let ok = client.send_event(delete).await.expect("send group delete");
    assert!(ok.accepted, "group delete rejected: {}", ok.message);

    let tombstone = query_snapshot(&mut client, KIND_GROUP_STATE, &group_id.to_string()).await;
    assert_eq!(tag_values(&tombstone, "d"), vec![group_id.to_string()]);
    assert!(
        has_tag(&tombstone, "deleted"),
        "snapshot should be tombstoned"
    );
    assert!(
        tag_values(&tombstone, "handle").is_empty()
            && tag_values(&tombstone, "name").is_empty()
            && tag_values(&tombstone, "creator").is_empty()
            && tag_values(&tombstone, "p").is_empty()
            && tag_values(&tombstone, "channel").is_empty(),
        "tombstone should not retain live group metadata"
    );

    client.disconnect().await.expect("disconnect creator");
}

#[tokio::test]
#[ignore]
async fn test_private_channel_is_rejected_as_group_default() {
    let creator = Keys::generate();
    seed_community_member(&creator, "member").await;
    let mut client = BuzzTestClient::connect(&relay_url(), &creator)
        .await
        .expect("connect creator");
    let private_channel = create_channel(&mut client, &creator, "private").await;
    let create = build_group_create(
        &unique_handle("private-default"),
        "Private Default",
        None,
        &[],
        &[private_channel],
    )
    .expect("build group create")
    .sign_with_keys(&creator)
    .expect("sign group create");

    let ok = client.send_event(create).await.expect("send group create");
    assert!(
        !ok.accepted,
        "private channel must not be accepted as a group default"
    );
    assert!(
        ok.message.contains("public") && ok.message.contains("active"),
        "private-channel rejection should explain the constraint: {}",
        ok.message
    );

    client.disconnect().await.expect("disconnect creator");
}

#[tokio::test]
#[ignore]
async fn test_non_community_member_group_command_is_rejected() {
    ensure_test_community().await;
    let outsider = Keys::generate();
    let event = build_group_create(&unique_handle("outsider"), "Outsider Group", None, &[], &[])
        .expect("build outsider group create")
        .sign_with_keys(&outsider)
        .expect("sign outsider group create");

    let response = reqwest::Client::new()
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", outsider.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).expect("serialize outsider command"))
        .send()
        .await
        .expect("submit outsider command");

    // The HTTP bridge maps relay-level rejections to 400 with an error body.
    assert_eq!(
        response.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "non-community-member group command must be rejected"
    );
    let body: serde_json::Value = response.json().await.expect("parse rejection body");
    let error = body["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("only community members"),
        "rejection should cite community membership: {body}"
    );
}
