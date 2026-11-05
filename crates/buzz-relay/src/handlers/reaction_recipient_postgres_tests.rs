//! Postgres-backed tests for relay-derived reaction recipients: a kind 7 is
//! indexed under its target's effective author, so `#p` reads find it
//! whether or not the reactor tagged that author.

use std::sync::Arc;

use buzz_core::TenantContext;
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};

use super::group_state_postgres_tests::{
    community, create_channel, http_auth, req_event_ids, state, test_conn,
};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

/// Ingest an event that `keys` signs and return it.
async fn publish(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    kind: u16,
    content: &str,
    tags: &[&[&str]],
) -> Event {
    let tags: Vec<Tag> = tags
        .iter()
        .map(|tag| Tag::parse(tag.iter().copied()).unwrap())
        .collect();
    let event = EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign");
    let result = ingest_event(state, tenant, event.clone(), http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("ingest kind:{kind}: {error:?}"));
    assert!(result.accepted, "kind:{kind} refused: {}", result.message);
    event
}

/// Run one WebSocket COUNT as `reader` and return the count.
async fn count(state: &Arc<AppState>, tenant: &TenantContext, reader: &Keys, filter: Value) -> u64 {
    let (conn, mut rx) = test_conn(tenant, reader);
    let filter: nostr::Filter = serde_json::from_value(filter).expect("filter");
    crate::handlers::count::handle_count("c".into(), vec![filter], conn, state.clone()).await;
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("COUNT") => return message[2]["count"].as_u64().expect("count"),
            other => panic!("unexpected frame {other:?}: {text}"),
        }
    }
    panic!("no COUNT reply");
}

/// The `event_mentions` pubkeys indexed for `event`.
async fn mention_pubkeys(state: &AppState, tenant: &TenantContext, event: &Event) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT pubkey_hex FROM event_mentions \
         WHERE community_id = $1 AND event_id = $2 ORDER BY pubkey_hex",
    )
    .bind(tenant.community().as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .fetch_all(state.db.pool())
    .await
    .expect("read mentions")
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

/// The `outcome` label of every derived-recipient counter increment, sorted.
fn derived_outcomes(recorder: &DebuggingRecorder) -> Vec<String> {
    let mut outcomes = Vec::new();
    for (key, _, _, value) in recorder.snapshotter().snapshot().into_vec() {
        let key = key.key();
        if key.name() != "buzz_relay_reaction_recipient_derived_total" {
            continue;
        }
        let outcome = key
            .labels()
            .find(|label| label.key() == "outcome")
            .map_or("-", |label| label.value())
            .to_owned();
        let DebugValue::Counter(count) = value else {
            panic!("not a counter: {key:?}");
        };
        outcomes.extend(std::iter::repeat_n(outcome, count as usize));
    }
    outcomes.sort();
    outcomes
}

/// Store a kind 9 in `channel_id` signed by the relay key, as REST and
/// workflow messages are, with `tags` after the `h` tag.
async fn store_relay_signed(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: uuid::Uuid,
    tags: &[&[&str]],
) -> Event {
    let channel = channel_id.to_string();
    let mut all_tags = vec![Tag::parse(["h", channel.as_str()]).unwrap()];
    all_tags.extend(
        tags.iter()
            .map(|tag| Tag::parse(tag.iter().copied()).unwrap()),
    );
    let event = EventBuilder::new(Kind::Custom(9), "via the relay")
        .tags(all_tags)
        .sign_with_keys(&state.relay_keypair)
        .expect("sign relay message");
    state
        .db
        .insert_event(tenant.community(), &event, Some(channel_id))
        .await
        .expect("store relay-signed message");
    event
}

/// A reaction is indexed under its target's effective author, so the author
/// reads it by `#p` and the reactor does not, with or without a `p` tag, and
/// COUNT agrees with REQ. Mutation: drop the derived mention insert, or the
/// `#p` match on `derived_recipients` → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_reaction_is_indexed_under_its_targets_effective_author() {
    let recorder = DebuggingRecorder::new();
    let _recorder = metrics::set_default_local_recorder(&recorder);
    let state = state().await;
    let tenant = community(&state, "reaction-recipients").await;
    let author = Keys::generate();
    let reactor = Keys::generate();
    let author_hex = author.public_key().to_hex();
    let reactor_hex = reactor.public_key().to_hex();
    let channel_id = create_channel(&state, &tenant, &author, &[]).await;
    let channel = channel_id.to_string();
    publish(
        &state,
        &tenant,
        &author,
        9000,
        "",
        &[&["h", &channel], &["p", &reactor_hex]],
    )
    .await;
    let message = publish(&state, &tenant, &author, 9, "hello", &[&["h", &channel]]).await;
    let message_id = message.id.to_hex();

    let untagged = publish(&state, &tenant, &reactor, 7, "+", &[&["e", &message_id]]).await;
    let by_author = json!({"kinds": [7], "#p": [author_hex]});
    assert_eq!(
        req_event_ids(&state, &tenant, &author, by_author.clone()).await,
        vec![untagged.id.to_hex()],
        "the author reads an untagged reaction to their message"
    );
    assert_eq!(
        req_event_ids(
            &state,
            &tenant,
            &reactor,
            json!({"kinds": [7], "#p": [reactor_hex]})
        )
        .await,
        Vec::<String>::new(),
        "the reactor is not a recipient of their own reaction"
    );
    assert_eq!(
        mention_pubkeys(&state, &tenant, &untagged).await,
        vec![author_hex.clone()]
    );

    // A reaction that also tags the author yields one mention row, not two.
    let tagged = publish(
        &state,
        &tenant,
        &reactor,
        7,
        "🎉",
        &[&["e", &message_id], &["p", &author_hex]],
    )
    .await;
    assert_eq!(
        mention_pubkeys(&state, &tenant, &tagged).await,
        vec![author_hex.clone()]
    );

    // A relay-signed message names its author in `actor`; the reaction goes
    // to that author, not to the relay key.
    let relay_signed =
        store_relay_signed(&state, &tenant, channel_id, &[&["actor", &author_hex]]).await;
    let to_relay_signed = publish(
        &state,
        &tenant,
        &reactor,
        7,
        "+",
        &[&["e", &relay_signed.id.to_hex()]],
    )
    .await;
    assert_eq!(
        mention_pubkeys(&state, &tenant, &to_relay_signed).await,
        vec![author_hex.clone()]
    );

    let expected = sorted(vec![
        untagged.id.to_hex(),
        tagged.id.to_hex(),
        to_relay_signed.id.to_hex(),
    ]);
    assert_eq!(
        sorted(req_event_ids(&state, &tenant, &author, by_author.clone()).await),
        expected
    );
    assert_eq!(
        count(&state, &tenant, &author, by_author).await,
        expected.len() as u64,
        "COUNT agrees with REQ"
    );

    // A relay-signed message that names no author falls back to the relay key.
    let anonymous = store_relay_signed(&state, &tenant, channel_id, &[]).await;
    let to_anonymous = publish(
        &state,
        &tenant,
        &reactor,
        7,
        "+",
        &[&["e", &anonymous.id.to_hex()]],
    )
    .await;
    assert_eq!(
        mention_pubkeys(&state, &tenant, &to_anonymous).await,
        vec![state.relay_keypair.public_key().to_hex()]
    );

    assert_eq!(
        derived_outcomes(&recorder),
        ["duplicate", "fallback_pubkey", "inserted", "inserted"]
    );
}

/// Only reactions take a recipient from the mention index. For other kinds a
/// mention row that no literal `p` tag matches, such as a workflow owner's
/// attribution, still fails the `#p` re-check. Mutation: derive recipients
/// for every kind → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_mention_row_without_a_p_tag_matches_only_for_reactions() {
    let state = state().await;
    let tenant = community(&state, "reaction-recipients-other-kinds").await;
    let owner = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let workflow_message = store_relay_signed(
        &state,
        &tenant,
        channel_id,
        &[&["p", &owner_hex, "", "owner"], &["buzz:workflow", "true"]],
    )
    .await;
    sqlx::query(
        "INSERT INTO event_mentions \
         (community_id, pubkey_hex, event_id, event_created_at, channel_id, event_kind) \
         VALUES ($1, $2, $3, to_timestamp($4), $5, 9)",
    )
    .bind(tenant.community().as_uuid())
    .bind(&owner_hex)
    .bind(workflow_message.id.as_bytes().as_slice())
    .bind(workflow_message.created_at.as_secs() as f64)
    .bind(channel_id)
    .execute(state.db.pool())
    .await
    .expect("index the attribution as a mention");

    assert_eq!(
        req_event_ids(
            &state,
            &tenant,
            &owner,
            json!({"kinds": [9], "#p": [owner_hex]})
        )
        .await,
        Vec::<String>::new()
    );
}
