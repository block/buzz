//! Postgres-backed tests for NIP-PM channel pins through ingest and REQ: who
//! may pin, what the relay answers, who can read the list, and that deleting a
//! pinned message rewrites it.

use std::sync::Arc;

use buzz_core::kind::{
    KIND_CHANNEL_PINS, KIND_DM_OPEN, KIND_PIN_MESSAGE, KIND_SYSTEM_MESSAGE, KIND_UNPIN_MESSAGE,
};
use buzz_core::TenantContext;
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use uuid::Uuid;

use super::group_state_postgres_tests::{community, create_channel, http_auth, test_conn};
use crate::handlers::ingest::{ingest_event, IngestError, IngestResult};
use crate::state::AppState;

struct Fixture {
    state: Arc<AppState>,
    tenant: TenantContext,
}

async fn fixture() -> Fixture {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let tenant = community(&state, "pins").await;
    Fixture { state, tenant }
}

fn signed(keys: &Keys, kind: u32, content: &str, tags: &[&[&str]]) -> Event {
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags(
            tags.iter()
                .map(|tag| Tag::parse(tag.iter().copied()).expect("tag")),
        )
        .sign_with_keys(keys)
        .expect("sign")
}

/// A pin or unpin of `target` in `channel`. The `client-id` tag makes two
/// commands for one target different events.
fn command(keys: &Keys, kind: u32, channel: Uuid, target: &str) -> Event {
    signed(
        keys,
        kind,
        "",
        &[
            &["h", &channel.to_string()],
            &["e", target],
            &["client-id", &Uuid::new_v4().to_string()],
        ],
    )
}

impl Fixture {
    async fn send(&self, keys: &Keys, event: Event) -> Result<IngestResult, IngestError> {
        ingest_event(&self.state, &self.tenant, event, http_auth(keys)).await
    }

    /// Ingest `event` and return its `OK` message.
    async fn accept(&self, keys: &Keys, event: Event) -> String {
        let kind = event.kind;
        let result = self
            .send(keys, event)
            .await
            .unwrap_or_else(|error| panic!("kind:{kind}: {error:?}"));
        assert!(result.accepted, "kind:{kind}: {}", result.message);
        result.message
    }

    async fn message(&self, keys: &Keys, channel: Uuid) -> String {
        // Unique content: two messages of one author are different events.
        let content = Uuid::new_v4().to_string();
        let message = signed(keys, 9, &content, &[&["h", &channel.to_string()]]);
        let id = message.id.to_hex();
        self.accept(keys, message).await;
        id
    }

    async fn pin(&self, keys: &Keys, channel: Uuid, target: &str) -> String {
        self.accept(keys, command(keys, KIND_PIN_MESSAGE, channel, target))
            .await
    }

    async fn unpin(&self, keys: &Keys, channel: Uuid, target: &str) -> String {
        self.accept(keys, command(keys, KIND_UNPIN_MESSAGE, channel, target))
            .await
    }

    /// The events a historical REQ returns to `reader`.
    async fn read(&self, reader: &Keys, filter: Value) -> Vec<Event> {
        read(&self.state, &self.tenant, reader, filter).await
    }

    /// The pinned IDs `reader` gets when it reads `channel`'s list by `#h`.
    /// `None` when the relay returns no list.
    async fn pins(&self, reader: &Keys, channel: Uuid) -> Option<Vec<String>> {
        let mut lists = self
            .read(
                reader,
                json!({"kinds": [KIND_CHANNEL_PINS], "#h": [channel.to_string()]}),
            )
            .await;
        assert!(lists.len() <= 1, "a channel has one pin list");
        lists.pop().map(|list| pinned(&list))
    }

    /// The content of each pin notice stored in `channel`, ordered by target.
    async fn notices(&self, reader: &Keys, channel: Uuid) -> Vec<Value> {
        let notices = self
            .read(
                reader,
                json!({"kinds": [KIND_SYSTEM_MESSAGE], "#h": [channel.to_string()]}),
            )
            .await;
        let mut notices: Vec<Value> = notices
            .iter()
            .map(|notice| serde_json::from_str(&notice.content).expect("notice content"))
            .filter(|content: &Value| content["type"] == "message_pinned")
            .collect();
        notices.sort_by_key(|notice| notice["target_event_id"].to_string());
        notices
    }
}

async fn read(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    filter: Value,
) -> Vec<Event> {
    let (conn, mut rx) = test_conn(tenant, reader);
    let filter: nostr::Filter = serde_json::from_value(filter).expect("filter");
    crate::handlers::req::handle_req("p".into(), vec![filter], vec![None], conn, state.clone())
        .await;
    let mut events = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("EVENT") => {
                events.push(serde_json::from_value(message[2].clone()).expect("event"))
            }
            Some("EOSE") | Some("CLOSED") => return events,
            other => panic!("unexpected frame {other:?}: {text}"),
        }
    }
    panic!("no EOSE");
}

fn pinned(list: &Event) -> Vec<String> {
    list.tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag[0] == "e")
        .map(|tag| tag[1].clone())
        .collect()
}

/// A live subscription of `reader` to `channel`'s pin lists by `#h`, on a
/// connection fan-out can reach.
struct LiveLists(tokio::sync::mpsc::Receiver<axum::extract::ws::Message>);

impl LiveLists {
    async fn open(f: &Fixture, reader: &Keys, channel: Uuid) -> Self {
        let (conn, mut rx) = test_conn(&f.tenant, reader);
        f.state.conn_manager.register(
            conn.conn_id,
            conn.send_tx.clone(),
            conn.ctrl_tx.clone(),
            conn.terminal_ctrl_tx.clone(),
            None,
            conn.cancel.clone(),
            f.tenant.community(),
            conn.backpressure_count.clone(),
            conn.subscriptions.clone(),
            conn.grace_limit,
            crate::state::CommunityConnectionControl::new(conn.cancel.clone()),
        );
        f.state
            .conn_manager
            .set_authenticated_pubkey(conn.conn_id, reader.public_key().to_bytes().to_vec());
        let filter = json!({"kinds": [KIND_CHANNEL_PINS], "#h": [channel.to_string()]});
        crate::handlers::req::handle_req(
            "live".into(),
            vec![serde_json::from_value(filter).expect("filter")],
            vec![None],
            conn,
            f.state.clone(),
        )
        .await;
        // Drain the stored history; what follows is live.
        while rx.try_recv().is_ok() {}
        Self(rx)
    }

    /// The pinned IDs of the next `count` lists, oldest list first. Fan-out
    /// runs in tasks of its own, so this waits for each list and orders them
    /// by `created_at`, not by arrival. No further list has arrived.
    async fn delivered(&mut self, count: usize) -> Vec<Vec<String>> {
        let mut lists = Vec::new();
        while lists.len() < count {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), self.0.recv())
                .await
                .expect("a live pin list")
                .expect("open connection");
            let axum::extract::ws::Message::Text(text) = frame else {
                continue;
            };
            let message: Value = serde_json::from_str(&text).expect("relay frame");
            assert_eq!(message[0], "EVENT", "{text}");
            let list: Event = serde_json::from_value(message[2].clone()).expect("event");
            assert_eq!(list.kind.as_u16() as u32, KIND_CHANNEL_PINS);
            lists.push(list);
        }
        assert!(self.0.try_recv().is_err(), "an unexpected live frame");
        lists.sort_by_key(|list| list.created_at);
        lists.iter().map(pinned).collect()
    }
}

fn refusal(result: Result<IngestResult, IngestError>) -> String {
    match result {
        Err(IngestError::Rejected(reason)) => reason,
        Err(error) => panic!("unexpected error: {error:?}"),
        Ok(result) => panic!("accepted: {:?}", result.message),
    }
}

/// The journey: members pin and unpin, the relay answers each command, keeps
/// one relay-signed list with the newest pin first, announces each pin that
/// changes it, and sends every new list to `#h` subscribers.
/// Mutation: drop `dispatch_list` from `pins::persist` → RED (no live list);
/// answer a stored command like a new one → RED (no `duplicate:`); drop
/// `command_event_id` from the notice → RED (re-pins share one notice); let
/// `target_readable_in_channel` pass a command → RED (the reply is stored).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn members_pin_and_unpin_through_one_relay_signed_list() {
    let f = fixture().await;
    let (owner, other) = (Keys::generate(), Keys::generate());
    let channel = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let first = f.message(&owner, channel).await;
    let second = f.message(&other, channel).await;
    assert_eq!(f.pins(&owner, channel).await, None, "no list before a pin");
    let mut live = LiveLists::open(&f, &other, channel).await;

    // An open channel lets anyone who may post pin, not only its creator.
    // Elements after a tag's value, such as a relay hint, carry no meaning.
    let hinted = signed(
        &other,
        KIND_PIN_MESSAGE,
        "",
        &[
            &["h", &channel.to_string(), "wss://relay.example.org"],
            &["e", &first, "wss://relay.example.org"],
        ],
    );
    assert_eq!(f.accept(&other, hinted.clone()).await, "");
    let again = command(&owner, KIND_PIN_MESSAGE, channel, &second);
    assert_eq!(f.accept(&owner, again.clone()).await, "");
    assert_eq!(
        f.pins(&owner, channel).await,
        Some(vec![second.clone(), first.clone()])
    );

    let lists = f
        .read(
            &owner,
            json!({"kinds": [KIND_CHANNEL_PINS], "#h": [channel.to_string()]}),
        )
        .await;
    let [list] = lists.as_slice() else {
        panic!("one list by #h, got {}", lists.len());
    };
    assert_eq!(list.pubkey, f.state.relay_keypair.public_key());
    assert_eq!(list.content, "");
    assert_eq!(
        list.tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>(),
        [
            vec!["d".to_owned(), channel.to_string()],
            vec!["h".to_owned(), channel.to_string()],
            vec!["e".to_owned(), second.clone()],
            vec!["e".to_owned(), first.clone()],
        ]
    );
    assert!(list.verify().is_ok());

    // A resend is a duplicate; a new pin of a pinned message changes nothing.
    assert_eq!(f.accept(&owner, again.clone()).await, "duplicate:");
    assert_eq!(f.pin(&owner, channel, &first).await, "");
    assert_eq!(
        f.unpin(&owner, channel, &f.message(&owner, channel).await)
            .await,
        ""
    );
    let mut notices = [
        json!({"type": "message_pinned", "actor": other.public_key().to_hex(),
               "target_event_id": first, "command_event_id": hinted.id.to_hex()}),
        json!({"type": "message_pinned", "actor": owner.public_key().to_hex(),
               "target_event_id": second, "command_event_id": again.id.to_hex()}),
    ];
    notices.sort_by_key(|notice| notice["target_event_id"].to_string());
    assert_eq!(
        f.notices(&owner, channel).await,
        notices,
        "one notice for each pin that changed the list"
    );

    assert_eq!(f.unpin(&other, channel, &second).await, "");
    assert_eq!(f.unpin(&owner, channel, &first).await, "");
    assert_eq!(f.pins(&owner, channel).await, Some(Vec::new()));
    assert_eq!(f.notices(&owner, channel).await.len(), 2, "no unpin notice");

    // Each pin has its own notice, however quickly the same member pins the
    // same message again.
    for _ in 0..3 {
        assert_eq!(f.pin(&owner, channel, &first).await, "");
        assert_eq!(f.unpin(&owner, channel, &first).await, "");
    }
    assert_eq!(f.notices(&owner, channel).await.len(), 5);
    let mut delivered = vec![
        vec![first.clone()],
        vec![second.clone(), first.clone()],
        vec![first.clone()],
        Vec::new(),
    ];
    for _ in 0..3 {
        delivered.extend([vec![first.clone()], Vec::new()]);
    }
    assert_eq!(live.delivered(delivered.len()).await, delivered);

    // A command is not a message: nothing replies to it.
    let reply = signed(
        &owner,
        9,
        "reply",
        &[
            &["h", &channel.to_string()],
            &["e", &hinted.id.to_hex(), "", "reply"],
        ],
    );
    let reason = refusal(f.send(&owner, reply).await);
    assert!(reason.contains("reply parent not found"), "{reason}");
}

/// Only those who can read a channel get its pin list: stored by `#h` or by
/// `#d`, live by `#h`. A DM is a channel like any other. `#h` names the
/// channel, so it also reads an unlisted channel the reader has not joined.
/// Mutation: store the list without its channel → RED (an outsider reads a
/// private channel's pins).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn the_list_is_read_like_the_channel() {
    let f = fixture().await;
    let (owner, peer, outsider) = (Keys::generate(), Keys::generate(), Keys::generate());
    let private = create_channel(&f.state, &f.tenant, &owner, &[&["visibility", "private"]]).await;
    let opened = f
        .accept(
            &owner,
            signed(
                &owner,
                KIND_DM_OPEN,
                "",
                &[&["p", &peer.public_key().to_hex()]],
            ),
        )
        .await;
    let opened: Value =
        serde_json::from_str(opened.strip_prefix("response:").expect("DM response")).expect("json");
    let dm: Uuid = opened["channel_id"]
        .as_str()
        .expect("channel_id")
        .parse()
        .expect("uuid");

    for (channel, readers) in [(private, vec![&owner]), (dm, vec![&owner, &peer])] {
        let mut live = LiveLists::open(&f, &outsider, channel).await;
        let mut member = LiveLists::open(&f, &owner, channel).await;
        let message = f.message(&owner, channel).await;
        assert_eq!(f.pin(&owner, channel, &message).await, "");
        let by_d = json!({"kinds": [KIND_CHANNEL_PINS], "#d": [channel.to_string()]});
        for reader in readers {
            assert_eq!(f.pins(reader, channel).await, Some(vec![message.clone()]));
            assert_eq!(f.read(reader, by_d.clone()).await.len(), 1);
        }
        assert_eq!(f.pins(&outsider, channel).await, None);
        assert!(f.read(&outsider, by_d).await.is_empty());
        // The member's list has arrived, so the outsider's would have too.
        assert_eq!(member.delivered(1).await, [vec![message.clone()]]);
        assert!(live.delivered(0).await.is_empty());

        let refused = command(&outsider, KIND_PIN_MESSAGE, channel, &message);
        refusal(f.send(&outsider, refused).await);
    }

    let unlisted = create_channel(&f.state, &f.tenant, &owner, &[&["unlisted", "true"]]).await;
    let message = f.message(&owner, unlisted).await;
    assert_eq!(f.pin(&owner, unlisted, &message).await, "");
    assert_eq!(f.pins(&outsider, unlisted).await, Some(vec![message]));
    let by_d = json!({"kinds": [KIND_CHANNEL_PINS], "#d": [unlisted.to_string()]});
    assert!(
        f.read(&outsider, by_d).await.is_empty(),
        "`#d` does not name a channel for this kind"
    );
}

/// Each refusal stores nothing and leaves the list alone.
/// Mutation: drop any one check in `pins::persist` → RED for its row.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn refused_commands_change_nothing() {
    let f = fixture().await;
    let (owner, guest) = (Keys::generate(), Keys::generate());
    let channel = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let elsewhere = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let message = f.message(&owner, channel).await;
    let foreign = f.message(&owner, elsewhere).await;
    let h = channel.to_string();
    f.accept(
        &owner,
        signed(
            &owner,
            9000,
            "",
            &[&["h", &h], &["p", &guest.public_key().to_hex(), "guest"]],
        ),
    )
    .await;

    let malformed: [(&str, Event, &str); 6] = [
        (
            "two e tags",
            signed(
                &owner,
                KIND_PIN_MESSAGE,
                "",
                &[&["h", &h], &["e", &message], &["e", &foreign]],
            ),
            "invalid: pin command requires exactly one e tag with a value",
        ),
        (
            "no e tag",
            signed(&owner, KIND_UNPIN_MESSAGE, "", &[&["h", &h]]),
            "invalid: pin command requires exactly one e tag with a value",
        ),
        (
            "an uppercase ID",
            signed(
                &owner,
                KIND_PIN_MESSAGE,
                "",
                &[&["h", &h], &["e", &message.to_uppercase()]],
            ),
            "invalid: e tag must be a lowercase hex event ID",
        ),
        (
            "a short ID",
            signed(
                &owner,
                KIND_PIN_MESSAGE,
                "",
                &[&["h", &h], &["e", &message[..62]]],
            ),
            "invalid: e tag must be a lowercase hex event ID",
        ),
        (
            "content",
            signed(
                &owner,
                KIND_PIN_MESSAGE,
                "why",
                &[&["h", &h], &["e", &message]],
            ),
            "invalid: pin command content must be empty",
        ),
        (
            "a message of another channel",
            command(&owner, KIND_PIN_MESSAGE, channel, &foreign),
            "invalid: target is not a message in this channel",
        ),
    ];
    let mut refused = Vec::new();
    for (case, event, reason) in malformed {
        assert_eq!(
            refusal(f.send(&owner, event.clone()).await),
            reason,
            "{case}"
        );
        refused.push(event);
    }

    let by_guest = command(&guest, KIND_PIN_MESSAGE, channel, &message);
    refusal(f.send(&guest, by_guest.clone()).await);
    refused.push(by_guest);

    // Clients never write the list.
    let forged = signed(
        &owner,
        KIND_CHANNEL_PINS,
        "",
        &[&["d", &h], &["e", &message]],
    );
    assert_eq!(
        refusal(f.send(&owner, forged.clone()).await),
        "restricted: relay-only kind"
    );
    refused.push(forged);

    assert_eq!(f.pin(&owner, channel, &message).await, "");
    sqlx::query("UPDATE channels SET archived_at = NOW() WHERE id = $1")
        .bind(channel)
        .execute(f.state.db.pool())
        .await
        .expect("archive");
    let archived = command(&owner, KIND_UNPIN_MESSAGE, channel, &message);
    assert_eq!(
        refusal(f.send(&owner, archived.clone()).await),
        "invalid: channel is archived"
    );
    refused.push(archived);

    let ids: Vec<Vec<u8>> = refused
        .iter()
        .map(|event| event.id.to_bytes().to_vec())
        .collect();
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id = $1 AND id = ANY($2)")
            .bind(f.tenant.community().as_uuid())
            .bind(&ids)
            .fetch_one(f.state.db.pool())
            .await
            .expect("count refused events");
    assert_eq!(stored, 0, "a refused command is not stored");
    assert_eq!(f.pins(&owner, channel).await, Some(vec![message]));
    assert_eq!(f.notices(&owner, channel).await.len(), 1);
}

/// A pinned message deleted by its author (kind:5) or by a channel admin
/// (kind:9005) leaves the list with the delete, and subscribers get the new
/// list.
/// Mutation: stop dispatching `pin_lists` in `persist_deletion` → RED (no
/// live list); drop `unpin_deleted_messages` → RED (the list keeps the ID).
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn deleting_a_pinned_message_rewrites_the_list() {
    let f = fixture().await;
    let (owner, author) = (Keys::generate(), Keys::generate());
    let channel = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    let h = channel.to_string();
    let by_author = f.message(&author, channel).await;
    let by_admin = f.message(&author, channel).await;
    let kept = f.message(&owner, channel).await;
    for message in [&by_author, &by_admin, &kept] {
        f.pin(&owner, channel, message).await;
    }
    let mut live = LiveLists::open(&f, &owner, channel).await;

    f.accept(&author, signed(&author, 5, "", &[&["e", &by_author]]))
        .await;
    assert_eq!(
        f.pins(&owner, channel).await,
        Some(vec![kept.clone(), by_admin.clone()])
    );

    f.accept(
        &owner,
        signed(&owner, 9005, "", &[&["h", &h], &["e", &by_admin]]),
    )
    .await;
    let by_d = json!({"kinds": [KIND_CHANNEL_PINS], "#d": [h]});
    let lists = f.read(&owner, by_d).await;
    assert_eq!(
        lists.iter().map(pinned).collect::<Vec<_>>(),
        [vec![kept.clone()]]
    );
    assert_eq!(
        live.delivered(2).await,
        [vec![kept.clone(), by_admin], vec![kept.clone()]]
    );

    // The list itself is not a channel admin's to delete.
    let list = signed(
        &owner,
        9005,
        "",
        &[&["h", &h], &["e", &lists[0].id.to_hex()]],
    );
    let reason = refusal(f.send(&owner, list).await);
    assert!(
        reason.contains("the pin list cannot be deleted"),
        "{reason}"
    );
    assert_eq!(f.pins(&owner, channel).await, Some(vec![kept]));
}
