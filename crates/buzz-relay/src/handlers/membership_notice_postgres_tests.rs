//! Postgres-backed tests for the membership notices sent by a channel delete,
//! a moderation kick and a moderation DM: each notice reaches the member's
//! live subscription, and a channel-list read made on receipt already shows
//! the change. "On receipt" is enforced by `notice_barrier`, which parks the
//! producer at the notice while the test reads. A new DM message resurfaces
//! the DM for members who hid it, in the message's own commit.

use std::sync::Arc;

use buzz_core::kind::{
    KIND_DM_HIDE, KIND_DM_VISIBILITY, KIND_MEMBER_ADDED_NOTIFICATION,
    KIND_MEMBER_REMOVED_NOTIFICATION,
};
use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio::sync::mpsc::Receiver;
use uuid::Uuid;

use super::group_state_postgres_tests::{create_channel, http_auth, test_conn};
use super::side_effects::notice_barrier;
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

async fn fixture() -> (Arc<AppState>, TenantContext) {
    let state = crate::api::bridge::postgres_tests::bridge_handler_test_state()
        .await
        .expect("local Postgres and Redis");
    let host = format!("membership-notices-{}.local", Uuid::new_v4().simple());
    let community = state
        .db
        .ensure_configured_community(&host)
        .await
        .expect("ensure community")
        .id;
    (state, TenantContext::resolved(community, &host))
}

async fn publish(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    kind: u16,
    tags: &[&[&str]],
) {
    let tags: Vec<Tag> = tags
        .iter()
        .map(|tag| Tag::parse(tag.iter().copied()).unwrap())
        .collect();
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign");
    let result = ingest_event(state, tenant, event, http_auth(keys))
        .await
        .unwrap_or_else(|error| panic!("ingest kind:{kind}: {error:?}"));
    assert!(result.accepted, "kind:{kind} refused: {}", result.message);
}

/// Register `reader` as a connected client holding the global notice
/// subscription clients send, and return its outbound frames.
fn subscribe_notices(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
) -> Receiver<axum::extract::ws::Message> {
    let filter = json!({
        "kinds": [KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION],
        "#p": [reader.public_key().to_hex()],
    });
    subscribe(state, tenant, reader, filter, None)
}

/// Register `reader` as a connected client holding one live subscription to
/// `filter`, scoped to `channel` when its events are a channel's, and return
/// its outbound frames.
fn subscribe(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    filter: Value,
    channel: Option<Uuid>,
) -> Receiver<axum::extract::ws::Message> {
    let conn_id = Uuid::new_v4();
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    state.conn_manager.register(
        conn_id,
        tx,
        tokio::sync::mpsc::channel(4).0,
        tokio::sync::mpsc::channel(1).0,
        None,
        tokio_util::sync::CancellationToken::new(),
        tenant.community(),
        Arc::new(std::sync::atomic::AtomicU8::new(0)),
        Arc::new(tokio::sync::Mutex::new(Default::default())),
        3,
        crate::state::CommunityConnectionControl::new(tokio_util::sync::CancellationToken::new()),
    );
    state
        .conn_manager
        .set_authenticated_pubkey(conn_id, reader.public_key().to_bytes().to_vec());
    state.sub_registry.register_scoped(
        tenant.community(),
        conn_id,
        "live".into(),
        vec![serde_json::from_value(filter).expect("filter")],
        channel,
    );
    rx
}

/// The `(kind, h, id)` of every live notice delivered so far.
fn live_notices(rx: &mut Receiver<axum::extract::ws::Message>) -> Vec<(u64, String, String)> {
    let mut notices = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        if message[0] == "EVENT" {
            let event = &message[2];
            notices.push((
                event["kind"].as_u64().unwrap(),
                tag_values(event, "h").concat(),
                event["id"].as_str().unwrap().to_owned(),
            ));
        }
    }
    notices
}

fn notices(rx: &mut Receiver<axum::extract::ws::Message>) -> Vec<(u64, String)> {
    live_notices(rx)
        .into_iter()
        .map(|(kind, h, _)| (kind, h))
        .collect()
}

/// The notices delivered once at least one arrives, for notices sent off
/// the request path.
async fn next_notices(rx: &mut Receiver<axum::extract::ws::Message>) -> Vec<(u64, String)> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let delivered = notices(rx);
            if !delivered.is_empty() {
                return delivered;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("no notice delivered")
}

fn tag_values(event: &Value, name: &str) -> Vec<String> {
    event["tags"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|tag| tag[0] == name)
        .map(|tag| tag[1].as_str().unwrap().to_owned())
        .collect()
}

/// The `name` tag values of the events a one-shot read as `reader` returns.
async fn read_tags(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
    filter: Value,
    name: &str,
) -> Vec<String> {
    let (conn, mut rx) = test_conn(tenant, reader);
    let filters: Vec<nostr::Filter> = serde_json::from_value(json!([filter])).unwrap();
    crate::handlers::req::handle_req("read".into(), filters, vec![None], conn, state.clone()).await;
    let mut values = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let axum::extract::ws::Message::Text(text) = frame else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("relay frame");
        match message[0].as_str() {
            Some("EVENT") => values.extend(tag_values(&message[2], name)),
            Some("EOSE") => return values,
            Some("CLOSED") => panic!("read refused: {message}"),
            _ => {}
        }
    }
    panic!("no EOSE");
}

/// A channel-list read as `reader`: the channels its 39002 discovery names,
/// as a client builds its sidebar.
async fn listed_channels(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    reader: &Keys,
) -> Vec<String> {
    let filter = json!({"kinds": [39002], "#p": [reader.public_key().to_hex()]});
    read_tags(state, tenant, reader, filter, "d").await
}

/// The DMs `reader`'s current NIP-DV snapshot hides from its sidebar, read
/// as a client does after its channel list refreshes.
async fn hidden_dms(state: &Arc<AppState>, tenant: &TenantContext, reader: &Keys) -> Vec<String> {
    let filter = json!({"kinds": [KIND_DM_VISIBILITY], "#p": [reader.public_key().to_hex()]});
    read_tags(state, tenant, reader, filter, "h").await
}

/// Wait for the producer to reach its next notice to the parked reader, run
/// `on_receipt` while it is held there, then let it continue. Returns the
/// notice's channel and what `on_receipt` saw.
pub(crate) async fn at_receipt<T>(
    barrier: &mut notice_barrier::Barrier,
    on_receipt: impl AsyncFnOnce(String) -> T,
) -> (String, T) {
    let (channel, release) =
        tokio::time::timeout(std::time::Duration::from_secs(10), barrier.next())
            .await
            .expect("producer reached no notice")
            .expect("barrier open");
    let seen = on_receipt(channel.to_string()).await;
    release.send(()).expect("producer waiting");
    (channel.to_string(), seen)
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn channel_delete_notifies_every_member_after_cleanup() {
    let (state, tenant) = fixture().await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let h = channel.to_string();
    publish(
        &state,
        &tenant,
        &owner,
        9000,
        &[&["h", &h], &["p", &member.public_key().to_hex()]],
    )
    .await;
    assert!(listed_channels(&state, &tenant, &member).await.contains(&h));
    let mut owner_rx = subscribe_notices(&state, &tenant, &owner);
    let mut member_rx = subscribe_notices(&state, &tenant, &member);
    let mut barrier = notice_barrier::install(&member.public_key().to_bytes());

    // The 9008 is accepted while the member's notice is still held, so the
    // notices are off the request path.
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        publish(&state, &tenant, &owner, 9008, &[&["h", &h]]),
    )
    .await
    .expect("9008 OK waited on the held notice");
    assert!(
        notices(&mut member_rx).is_empty(),
        "notice delivered before the OK"
    );
    let (noticed, listed) = at_receipt(&mut barrier, async |_| {
        listed_channels(&state, &tenant, &member).await
    })
    .await;

    assert_eq!(noticed, h);
    assert!(!listed.contains(&h), "notice sent before discovery cleanup");
    for rx in [&mut owner_rx, &mut member_rx] {
        assert_eq!(
            next_notices(rx).await,
            vec![(KIND_MEMBER_REMOVED_NOTIFICATION as u64, h.clone())]
        );
    }
}

#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn moderation_kick_notifies_the_kicked_member_after_cleanup() {
    let (state, tenant) = fixture().await;
    let owner = Keys::generate();
    let member = Keys::generate();
    let channel = create_channel(&state, &tenant, &owner, &[]).await;
    let h = channel.to_string();
    let member_bytes = member.public_key().to_bytes();
    publish(
        &state,
        &tenant,
        &owner,
        9000,
        &[&["h", &h], &["p", &member.public_key().to_hex()]],
    )
    .await;
    assert!(listed_channels(&state, &tenant, &member).await.contains(&h));
    let mut rx = subscribe_notices(&state, &tenant, &member);
    let mut barrier = notice_barrier::install(&member_bytes);

    // The kick mutation commits the removal; live effects follow.
    state
        .db
        .remove_member(
            tenant.community(),
            channel,
            &member_bytes,
            &owner.public_key().to_bytes(),
        )
        .await
        .expect("remove member");
    let effects = tokio::spawn({
        let (state, tenant) = (state.clone(), tenant.clone());
        async move {
            crate::handlers::side_effects::apply_kick_live_side_effects(
                &tenant,
                &state,
                channel,
                &member_bytes,
            )
            .await
        }
    });
    let (noticed, listed) = at_receipt(&mut barrier, async |_| {
        listed_channels(&state, &tenant, &member).await
    })
    .await;
    effects.await.expect("task").expect("kick live effects");

    assert_eq!(noticed, h);
    assert!(
        !listed.contains(&h),
        "notice sent before the roster refresh"
    );
    assert_eq!(
        notices(&mut rx),
        vec![(KIND_MEMBER_REMOVED_NOTIFICATION as u64, h.clone())]
    );
}

/// A recipient whose channel set is cached when its moderation DM is
/// created still finds the DM in the read its notice prompts.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn moderation_dm_notice_follows_a_cached_channel_list() {
    let (state, tenant) = fixture().await;
    let recipient = Keys::generate();
    let recipient_bytes = recipient.public_key().to_bytes();
    // Warm the recipient's cached channel set, as a recent REQ does.
    assert_eq!(
        listed_channels(&state, &tenant, &recipient).await,
        Vec::<String>::new()
    );
    let mut barrier = notice_barrier::install(&recipient_bytes);

    let deliver = tokio::spawn({
        let (state, tenant) = (state.clone(), tenant.clone());
        async move {
            crate::handlers::moderation_notices::send_moderation_notice(
                &tenant,
                &state,
                &recipient_bytes,
                crate::handlers::moderation_notices::ModerationNotice::ContentActioned {
                    action_id: Uuid::new_v4(),
                    public_reason: "spam".into(),
                },
                chrono::Utc::now(),
            )
            .await
        }
    });
    let (dm, listed) = at_receipt(&mut barrier, async |_| {
        listed_channels(&state, &tenant, &recipient).await
    })
    .await;
    deliver.await.expect("task").expect("notice");

    assert!(listed.contains(&dm), "notice sent before the cache drop");
}

/// A moderation DM resurfaces the DM, and on receipt of its notice the
/// channel list shows it and the NIP-DV snapshot no longer hides it. The
/// second delivery follows a real hide in the same (pinned) second and still
/// sends its own notice, so a client that only hears notices sees it return.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn moderation_dm_notifies_the_recipient_once_visible() {
    let (state, tenant) = fixture().await;
    let recipient = Keys::generate();
    let recipient_bytes = recipient.public_key().to_bytes();
    let mut rx = subscribe_notices(&state, &tenant, &recipient);
    let mut barrier = notice_barrier::install(&recipient_bytes);
    barrier.pin_created_at(nostr::Timestamp::now().as_secs());
    let deliver = || {
        let (state, tenant) = (state.clone(), tenant.clone());
        tokio::spawn(async move {
            crate::handlers::moderation_notices::send_moderation_notice(
                &tenant,
                &state,
                &recipient_bytes,
                crate::handlers::moderation_notices::ModerationNotice::ContentActioned {
                    action_id: Uuid::new_v4(),
                    public_reason: "spam".into(),
                },
                chrono::Utc::now(),
            )
            .await
        })
    };
    let on_receipt = async |dm: String| {
        (
            listed_channels(&state, &tenant, &recipient)
                .await
                .contains(&dm),
            hidden_dms(&state, &tenant, &recipient).await.contains(&dm),
        )
    };

    let first = deliver();
    let (dm, seen) = at_receipt(&mut barrier, on_receipt).await;
    first.await.expect("task").expect("first notice");
    assert_eq!(seen, (true, false), "listed and visible on receipt");
    let first_notice = live_notices(&mut rx);
    assert_eq!(first_notice.len(), 1, "{first_notice:?}");
    assert_eq!(
        (first_notice[0].0, &first_notice[0].1),
        (KIND_MEMBER_ADDED_NOTIFICATION as u64, &dm)
    );

    publish(
        &state,
        &tenant,
        &recipient,
        KIND_DM_HIDE as u16,
        &[&["h", &dm]],
    )
    .await;
    assert!(hidden_dms(&state, &tenant, &recipient).await.contains(&dm));

    let second = deliver();
    let (again, seen) = at_receipt(&mut barrier, on_receipt).await;
    second.await.expect("task").expect("second notice");
    assert_eq!(again, dm);
    assert_eq!(seen, (true, false), "listed and visible on receipt");
    let second_notice = live_notices(&mut rx);
    assert_eq!(
        second_notice.len(),
        1,
        "the second delivery's notice coalesced"
    );
    assert_eq!(
        (second_notice[0].0, &second_notice[0].1),
        (KIND_MEMBER_ADDED_NOTIFICATION as u64, &dm)
    );
    assert_ne!(second_notice[0].2, first_notice[0].2);
}

/// A new message from someone else resurfaces a DM its reader hid, in the
/// reader's NIP-DV snapshot. The reader's own message, and one addressed only
/// to agents, leave the hide alone.
/// The message commits with the snapshot: when the snapshot does not land,
/// neither is stored or delivered, and the resend is not taken as a
/// duplicate. A read made when the message fans out sees the DM.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn dm_message_from_someone_else_unhides_it() {
    let (state, tenant) = fixture().await;
    let reader = Keys::generate();
    let sender = Keys::generate();
    let (dm, _) = state
        .db
        .open_dm(
            tenant.community(),
            &[
                &reader.public_key().to_bytes(),
                &sender.public_key().to_bytes(),
            ],
            &reader.public_key().to_bytes(),
            false,
            &|_: &str, _: &[u8]| Ok(()),
        )
        .await
        .expect("open dm");
    let dm = dm.id.to_string();
    // The `h` tags of the reader's snapshot, which must exist.
    let hidden = async || {
        let filter = json!({"kinds": [KIND_DM_VISIBILITY], "#p": [reader.public_key().to_hex()]});
        let snapshot = read_tags(&state, &tenant, &reader, filter.clone(), "d").await;
        assert_eq!(snapshot, [reader.public_key().to_hex()], "one snapshot");
        read_tags(&state, &tenant, &reader, filter, "h").await
    };
    publish(
        &state,
        &tenant,
        &reader,
        KIND_DM_HIDE as u16,
        &[&["h", &dm]],
    )
    .await;
    assert_eq!(hidden().await, std::slice::from_ref(&dm));

    publish(&state, &tenant, &reader, 9, &[&["h", &dm]]).await;
    assert_eq!(
        hidden().await,
        std::slice::from_ref(&dm),
        "the reader's own message keeps the DM hidden"
    );
    publish(
        &state,
        &tenant,
        &sender,
        9,
        &[&["h", &dm], &["audience", "agents"]],
    )
    .await;
    assert_eq!(
        hidden().await,
        std::slice::from_ref(&dm),
        "a message only for agents keeps the DM hidden"
    );

    let filter = json!({"kinds": [9], "#h": [dm]});
    let mut live = subscribe(&state, &tenant, &reader, filter, Some(dm.parse().unwrap()));
    let message = EventBuilder::new(Kind::Custom(9), "")
        .tags([Tag::parse(["h", dm.as_str()]).unwrap()])
        .sign_with_keys(&sender)
        .expect("sign");
    let stored = async || {
        let filter = json!({"kinds": [9], "#h": [dm], "ids": [message.id.to_hex()]});
        read_tags(&state, &tenant, &reader, filter, "h").await
    };
    // The trigger skips the snapshot's insert, so the replacement reports it
    // did not land, as a lost race would.
    let refuse = format!("refuse_dm_snapshot_{}", Uuid::new_v4().simple());
    let pool = state.db.pool();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {refuse}() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RETURN NULL; END $$; \
         CREATE TRIGGER {refuse} BEFORE INSERT ON events FOR EACH ROW \
         WHEN (NEW.kind = {KIND_DM_VISIBILITY} AND NEW.d_tag = '{}') \
         EXECUTE FUNCTION {refuse}();",
        reader.public_key().to_hex()
    )))
    .execute(pool)
    .await
    .expect("install snapshot refusal");
    let refused = ingest_event(&state, &tenant, message.clone(), http_auth(&sender)).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER {refuse} ON events; DROP FUNCTION {refuse}();"
    )))
    .execute(pool)
    .await
    .expect("remove snapshot refusal");
    assert!(refused.is_err(), "message accepted without its snapshot");
    assert!(
        stored().await.is_empty(),
        "message stored without its snapshot"
    );
    assert_eq!(hidden().await, std::slice::from_ref(&dm));
    assert!(
        live_notices(&mut live).is_empty(),
        "refused message delivered"
    );

    // Park the resend at its fan-out and read as a client does on receipt.
    let mut barrier = notice_barrier::install(&sender.public_key().to_bytes());
    let resend = tokio::spawn({
        let (state, tenant, message) = (state.clone(), tenant.clone(), message.clone());
        let auth = http_auth(&sender);
        async move { ingest_event(&state, &tenant, message, auth).await }
    });
    let (_, hidden_on_receipt) = at_receipt(&mut barrier, async |_| hidden().await).await;
    let resent = resend.await.expect("task").expect("resend");
    assert!(
        resent.accepted && !resent.message.starts_with("duplicate"),
        "resend refused: {}",
        resent.message
    );
    assert!(
        hidden_on_receipt.is_empty(),
        "someone else's message unhides the DM before it is delivered"
    );
    let delivered = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let delivered = live_notices(&mut live);
            if !delivered.is_empty() {
                return delivered;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("message not delivered");
    assert_eq!(
        delivered,
        [(9, dm.clone(), message.id.to_hex())],
        "the resend is delivered once"
    );
    assert_eq!(stored().await, std::slice::from_ref(&dm));
}

/// A hide racing a message from someone else lands wholly before or after it.
/// The message is parked after it read the DM as visible, holding the DM's
/// membership key shared; the hide waits on that key exclusively rather than
/// committing underneath it, so the hide is the later write and the reader's
/// snapshot ends up hiding the DM.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn dm_hide_racing_a_message_lands_after_it() {
    let (state, tenant) = fixture().await;
    let reader = Keys::generate();
    let sender = Keys::generate();
    let (dm, _) = state
        .db
        .open_dm(
            tenant.community(),
            &[
                &reader.public_key().to_bytes(),
                &sender.public_key().to_bytes(),
            ],
            &reader.public_key().to_bytes(),
            false,
            &|_: &str, _: &[u8]| Ok(()),
        )
        .await
        .expect("open dm");
    let dm = dm.id.to_string();
    let message = EventBuilder::new(Kind::Custom(9), "")
        .tags([Tag::parse(["h", dm.as_str()]).unwrap()])
        .sign_with_keys(&sender)
        .expect("sign");
    let mut barrier = notice_barrier::install(message.id.as_bytes());
    let send = tokio::spawn({
        let (state, tenant, message) = (state.clone(), tenant.clone(), message.clone());
        let auth = http_auth(&sender);
        async move { ingest_event(&state, &tenant, message, auth).await }
    });
    let (_, hide) = at_receipt(&mut barrier, async |_| {
        let hide = tokio::spawn({
            let (state, tenant, reader, dm) =
                (state.clone(), tenant.clone(), reader.clone(), dm.clone());
            async move {
                publish(
                    &state,
                    &tenant,
                    &reader,
                    KIND_DM_HIDE as u16,
                    &[&["h", &dm]],
                )
                .await
            }
        });
        // The hide must wait for the message's shared hold, exclusively, on
        // this DM's membership key, not commit underneath it.
        let key = format!("buzz_channel_membership:{}:{dm}", tenant.community().as_uuid());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' \
                     AND NOT granted AND mode = 'ExclusiveLock' \
                     AND database = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                     AND ((classid::bigint << 32) | objid::bigint) = hashtextextended($1, 0))",
                )
                .bind(&key)
                .fetch_one(state.db.pool())
                .await
                .expect("observe the hide waiting");
                if waiting {
                    break;
                }
                assert!(!hide.is_finished(), "hide committed under the message");
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("hide never waited on the message");
        // Released when this returns; the hide then follows the message.
        Some(hide)
    })
    .await;
    let sent = send.await.expect("task").expect("send");
    assert!(sent.accepted, "{}", sent.message);
    hide.expect("hide task").await.expect("hide");
    let filter = json!({"kinds": [KIND_DM_VISIBILITY], "#p": [reader.public_key().to_hex()]});
    let hidden = read_tags(&state, &tenant, &reader, filter, "h").await;
    assert_eq!(hidden, [dm], "the later hide is kept");
}
