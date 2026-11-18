//! Relay-backed regression coverage for workflow deletion.
//!
//! # Running
//!
//! Start an isolated relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 DATABASE_URL=postgres://… cargo test \
//!   -p buzz-test-client --test e2e_workflow_deletion -- --ignored
//! ```

use std::panic::AssertUnwindSafe;
use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage, TestClientError};
use futures_util::FutureExt;
use nostr::{Alphabet, Event, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp};
use sqlx::PgPool;
use uuid::Uuid;

const KIND_WORKFLOW_DEFINITION: u16 = 30_620;
const KIND_WORKFLOW_TRIGGER: u16 = 46_020;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn database_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL required for failure injection")
}

fn sub_id(phase: &str) -> String {
    format!("e2e-workflow-deletion-{phase}-{}", Uuid::new_v4())
}

fn definition_filter(keys: &Keys, workflow_id: &str) -> Filter {
    Filter::new()
        .kind(Kind::Custom(KIND_WORKFLOW_DEFINITION))
        .author(keys.public_key())
        .custom_tags(SingleLetterTag::lowercase(Alphabet::D), [workflow_id])
}

async fn query_definition(
    client: &mut BuzzTestClient,
    keys: &Keys,
    workflow_id: &str,
) -> Vec<Event> {
    let subscription = sub_id("definition-query");
    client
        .subscribe(&subscription, vec![definition_filter(keys, workflow_id)])
        .await
        .expect("subscribe for fresh workflow definition query");
    let events = client
        .collect_until_eose(&subscription, Duration::from_secs(5))
        .await
        .expect("query workflow definition");
    client
        .close_subscription(&subscription)
        .await
        .expect("close definition query");
    events
}

async fn query_deletion(client: &mut BuzzTestClient, deletion: &Event) -> Vec<Event> {
    let subscription = sub_id("deletion-query");
    client
        .subscribe(
            &subscription,
            vec![Filter::new().kind(Kind::EventDeletion).id(deletion.id)],
        )
        .await
        .expect("fresh exact-ID deletion query");
    let events = client
        .collect_until_eose(&subscription, Duration::from_secs(5))
        .await
        .expect("query deletion history");
    client
        .close_subscription(&subscription)
        .await
        .expect("close deletion query");
    events
}

async fn database_representation_counts(
    pool: &PgPool,
    keys: &Keys,
    workflow_id: Uuid,
) -> (i64, i64) {
    let owner = keys.public_key().to_bytes().to_vec();
    let d_tag = workflow_id.to_string();
    let workflow_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM workflows WHERE id = $1 AND owner_pubkey = $2")
            .bind(workflow_id)
            .bind(&owner)
            .fetch_one(pool)
            .await
            .expect("count executable workflow representation");
    let definition_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE kind = $1 AND pubkey = $2 AND d_tag = $3 \
         AND deleted_at IS NULL",
    )
    .bind(i32::from(KIND_WORKFLOW_DEFINITION))
    .bind(&owner)
    .bind(d_tag)
    .fetch_one(pool)
    .await
    .expect("count visible workflow definition representation");
    (workflow_rows, definition_rows)
}

struct FailureInjector {
    pool: PgPool,
}

impl FailureInjector {
    async fn install(pool: PgPool, keys: &Keys, workflow_id: Uuid) -> Self {
        // This fixture runs only against a dedicated disposable database. The
        // shared trigger is inert unless its private table contains this test's
        // exact owner + d-tag coordinate.
        sqlx::raw_sql(
            "CREATE TABLE workflow_delete_failure_injection (\
                 owner_pubkey BYTEA NOT NULL, d_tag TEXT NOT NULL, \
                 PRIMARY KEY (owner_pubkey, d_tag)\
             ); \
             CREATE FUNCTION reject_injected_workflow_delete() RETURNS trigger \
             LANGUAGE plpgsql AS $$ \
             BEGIN \
               IF OLD.kind = 30620 AND OLD.deleted_at IS NULL \
                  AND NEW.deleted_at IS NOT NULL \
                  AND EXISTS (SELECT 1 FROM workflow_delete_failure_injection \
                              WHERE owner_pubkey = OLD.pubkey AND d_tag = OLD.d_tag) \
               THEN RAISE EXCEPTION 'injected workflow deletion failure'; \
               END IF; \
               RETURN NEW; \
             END $$; \
             CREATE TRIGGER reject_injected_workflow_delete \
             BEFORE UPDATE ON events FOR EACH ROW \
             EXECUTE FUNCTION reject_injected_workflow_delete();",
        )
        .execute(&pool)
        .await
        .expect("install coordinate-scoped deletion failure injector");
        sqlx::query(
            "INSERT INTO workflow_delete_failure_injection (owner_pubkey, d_tag) VALUES ($1, $2)",
        )
        .bind(keys.public_key().to_bytes().to_vec())
        .bind(workflow_id.to_string())
        .execute(&pool)
        .await
        .expect("arm deletion failure injector for test coordinate");
        Self { pool }
    }

    async fn remove(&mut self) {
        sqlx::raw_sql(
            "DROP TRIGGER IF EXISTS reject_injected_workflow_delete ON events; \
             DROP FUNCTION IF EXISTS reject_injected_workflow_delete(); \
             DROP TABLE IF EXISTS workflow_delete_failure_injection;",
        )
        .execute(&self.pool)
        .await
        .expect("remove deletion failure injector");
    }
}

async fn create_channel(client: &mut BuzzTestClient, keys: &Keys, channel_id: &str) {
    let create_channel = EventBuilder::new(Kind::Custom(9007), "")
        .tags([
            Tag::parse(["h", channel_id]).expect("channel h tag"),
            Tag::parse(["name", "workflow-deletion-e2e"]).expect("channel name tag"),
            Tag::parse(["channel_type", "stream"]).expect("channel type tag"),
            Tag::parse(["visibility", "open"]).expect("channel visibility tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign channel creation");
    let created = client
        .send_event(create_channel)
        .await
        .expect("create channel");
    assert!(
        created.accepted,
        "channel creation rejected: {}",
        created.message
    );
}

fn workflow_definition(channel_id: &str, workflow_id: &str) -> EventBuilder {
    EventBuilder::new(
        Kind::Custom(KIND_WORKFLOW_DEFINITION),
        "name: deletion-regression\ntrigger:\n  on: message_posted\nsteps:\n  - id: pause\n    action: delay\n    duration: 1s\n",
    )
    .tags([
        Tag::parse(["d", workflow_id]).expect("workflow d tag"),
        Tag::parse(["h", channel_id]).expect("workflow h tag"),
    ])
}

async fn create_workflow(
    client: &mut BuzzTestClient,
    keys: &Keys,
    channel_id: &str,
    workflow_id: &str,
) -> nostr::EventId {
    let definition = workflow_definition(channel_id, workflow_id)
        .sign_with_keys(keys)
        .expect("sign workflow definition");
    let definition_id = definition.id;
    let created = client
        .send_event(definition)
        .await
        .expect("create workflow");
    assert!(
        created.accepted,
        "workflow creation rejected: {}",
        created.message
    );
    definition_id
}

fn workflow_deletion(keys: &Keys, workflow_id: &str) -> Event {
    let coordinate = format!(
        "{KIND_WORKFLOW_DEFINITION}:{}:{workflow_id}",
        keys.public_key().to_hex()
    );
    EventBuilder::new(Kind::EventDeletion, "")
        .tags([Tag::parse(["a", coordinate.as_str()]).expect("workflow coordinate")])
        .sign_with_keys(keys)
        .expect("sign workflow deletion")
}

async fn assert_trigger_rejected(client: &mut BuzzTestClient, keys: &Keys, workflow_id: &str) {
    let trigger = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_TRIGGER), "{}")
        .tags([Tag::parse(["d", workflow_id]).expect("trigger d tag")])
        .sign_with_keys(keys)
        .expect("sign workflow trigger");
    let triggered = client
        .send_event(trigger)
        .await
        .expect("submit post-deletion trigger");
    assert!(!triggered.accepted, "deleted workflow remained triggerable");
    assert!(
        triggered.message.contains("workflow not found"),
        "unexpected post-deletion trigger rejection: {}",
        triggered.message
    );
}

async fn assert_no_live_deletion(listener: &mut BuzzTestClient) {
    assert!(
        matches!(
            listener.recv_event(Duration::from_millis(300)).await,
            Err(TestClientError::Timeout)
        ),
        "rejected/completed duplicate deletion must not be delivered"
    );
}

async fn assert_live_event(listener: &mut BuzzTestClient, subscription: &str, expected: &Event) {
    match listener
        .recv_event(Duration::from_secs(5))
        .await
        .expect("receive live event")
    {
        RelayMessage::Event {
            subscription_id,
            event,
        } => {
            assert_eq!(subscription_id, subscription);
            assert_eq!(*event, *expected, "dispatch the exact signed event");
        }
        other => panic!("expected live event, got {other:?}"),
    }
}

async fn audit_count(pool: &PgPool, event: &Event) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE object_id = $1 AND action = 'event_created' AND actor_pubkey = $2 AND detail->>'event_kind' = $3")
        .bind(event.id.to_hex())
        .bind(event.pubkey.to_bytes().to_vec())
        .bind(event.kind.as_u16().to_string())
        .fetch_one(pool).await.expect("count event audit entries")
}

async fn flush_audit(client: &mut BuzzTestClient, keys: &Keys, pool: &PgPool) -> Event {
    // Audit enqueue is awaited before OK. A later marker in the same relay's
    // FIFO audit queue gives a causal flush fence instead of an arbitrary sleep.
    let marker = EventBuilder::new(Kind::TextNote, Uuid::new_v4().to_string())
        .sign_with_keys(keys)
        .expect("sign audit fence");
    assert!(
        client
            .send_event(marker.clone())
            .await
            .expect("publish audit fence")
            .accepted
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while audit_count(pool, &marker).await == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("audit worker must persist fence");
    marker
}

#[tokio::test]
#[ignore]
async fn deleting_workflow_removes_definition_and_rejects_manual_trigger() {
    let keys = Keys::generate();
    let channel_id = Uuid::new_v4().to_string();
    let workflow_id = Uuid::new_v4().to_string();
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");

    create_channel(&mut client, &keys, &channel_id).await;
    let definition_id = create_workflow(&mut client, &keys, &channel_id, &workflow_id).await;

    let before = query_definition(&mut client, &keys, &workflow_id).await;
    assert_eq!(
        before.len(),
        1,
        "definition must be queryable before deletion"
    );
    assert_eq!(before[0].id, definition_id);

    let deletion = workflow_deletion(&keys, &workflow_id);
    let deleted = client.send_event(deletion).await.expect("delete workflow");
    assert!(
        deleted.accepted,
        "workflow deletion rejected: {}",
        deleted.message
    );

    let after = query_definition(&mut client, &keys, &workflow_id).await;
    assert!(
        after.is_empty(),
        "deleted workflow definition remained queryable: {after:?}"
    );

    assert_trigger_rejected(&mut client, &keys, &workflow_id).await;

    client.disconnect().await.expect("disconnect");
}

async fn run_failed_deletion_replay_scenario(pool: &PgPool) {
    let keys = Keys::generate();
    let channel_id = Uuid::new_v4().to_string();
    let workflow_id = Uuid::new_v4();
    let workflow_id_text = workflow_id.to_string();
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");

    create_channel(&mut client, &keys, &channel_id).await;
    create_workflow(&mut client, &keys, &channel_id, &workflow_id_text).await;
    assert_eq!(
        database_representation_counts(pool, &keys, workflow_id).await,
        (1, 1),
        "workflow setup must create both representations"
    );

    let deletion = workflow_deletion(&keys, &workflow_id_text);
    let deletion_id = deletion.id;
    let mut listener = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect live listener");
    let subscription = sub_id("live-deletion");
    listener
        .subscribe(
            &subscription,
            vec![Filter::new()
                .kinds([Kind::EventDeletion, Kind::TextNote])
                .author(keys.public_key())],
        )
        .await
        .expect("subscribe before failure");
    assert!(listener
        .collect_until_eose(&subscription, Duration::from_secs(5))
        .await
        .expect("establish live subscription")
        .is_empty());
    let mut injector = FailureInjector::install(pool.clone(), &keys, workflow_id).await;
    let rejected = client
        .send_event(deletion.clone())
        .await
        .expect("submit injected-failure deletion");
    assert!(
        !rejected.accepted,
        "injected deletion failure was acknowledged"
    );
    // WebSocket ingestion deliberately redacts internal database errors.
    assert_eq!(rejected.message, "error: internal server error");
    let marker = flush_audit(&mut client, &keys, pool).await;
    assert_live_event(&mut listener, &subscription, &marker).await;
    assert_no_live_deletion(&mut listener).await;
    assert_eq!(
        audit_count(pool, &deletion).await,
        0,
        "failed deletion must not be audited as accepted"
    );

    assert_eq!(
        database_representation_counts(pool, &keys, workflow_id).await,
        (1, 1),
        "failed deletion must retain both workflow representations"
    );
    let after_failure = query_definition(&mut client, &keys, &workflow_id_text).await;
    assert_eq!(
        after_failure.len(),
        1,
        "definition disappeared after rejected deletion"
    );

    let stored_requests: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE id = $1")
        .bind(deletion_id.to_bytes().to_vec())
        .fetch_one(pool)
        .await
        .expect("count stored deletion");
    assert_eq!(
        stored_requests, 0,
        "rejected deletion must roll back its public event"
    );
    assert!(
        query_deletion(&mut client, &deletion).await.is_empty(),
        "rejected deletion must not appear in fresh REQ history"
    );
    injector.remove().await;
    let replayed = client
        .send_event(deletion.clone())
        .await
        .expect("replay identical signed deletion");
    assert!(
        replayed.accepted,
        "identical deletion replay rejected: {}",
        replayed.message
    );
    assert_eq!(replayed.event_id, deletion_id.to_hex());
    assert_eq!(
        query_deletion(&mut client, &deletion).await,
        vec![deletion.clone()]
    );
    assert_live_event(&mut listener, &subscription, &deletion).await;
    let marker = flush_audit(&mut client, &keys, pool).await;
    assert_live_event(&mut listener, &subscription, &marker).await;
    assert_eq!(
        audit_count(pool, &deletion).await,
        1,
        "repair must enqueue normal audit exactly once"
    );
    assert_eq!(
        database_representation_counts(pool, &keys, workflow_id).await,
        (0, 0),
        "successful replay must remove both workflow representations"
    );
    assert!(
        query_definition(&mut client, &keys, &workflow_id_text)
            .await
            .is_empty(),
        "definition remained queryable after successful replay"
    );
    assert_trigger_rejected(&mut client, &keys, &workflow_id_text).await;
    let duplicate = client
        .send_event(deletion.clone())
        .await
        .expect("repeat completed deletion");
    assert!(duplicate.accepted && duplicate.message.starts_with("duplicate:"));
    let marker = flush_audit(&mut client, &keys, pool).await;
    assert_live_event(&mut listener, &subscription, &marker).await;
    assert_no_live_deletion(&mut listener).await;
    assert_eq!(
        audit_count(pool, &deletion).await,
        1,
        "completed duplicate must not re-audit"
    );
    listener.disconnect().await.expect("disconnect listener");
    client.disconnect().await.expect("disconnect");
}

#[tokio::test]
#[ignore]
async fn failed_workflow_deletion_rolls_back_and_identical_event_can_be_replayed() {
    let pool = PgPool::connect(&database_url())
        .await
        .expect("connect to isolated workflow-deletion database");
    let result = AssertUnwindSafe(run_failed_deletion_replay_scenario(&pool))
        .catch_unwind()
        .await;

    // Clean up even when the scenario panics so a failed run cannot poison the
    // disposable relay for subsequent validation.
    sqlx::raw_sql(
        "DROP TRIGGER IF EXISTS reject_injected_workflow_delete ON events; \
         DROP FUNCTION IF EXISTS reject_injected_workflow_delete(); \
         DROP TABLE IF EXISTS workflow_delete_failure_injection;",
    )
    .execute(&pool)
    .await
    .expect("final deletion failure injector cleanup");

    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[ignore]
async fn workflow_can_be_intentionally_recreated_with_a_backdated_definition() {
    let keys = Keys::generate();
    let channel_id = Uuid::new_v4().to_string();
    let workflow_id = Uuid::new_v4();
    let workflow_id_text = workflow_id.to_string();
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");
    create_channel(&mut client, &keys, &channel_id).await;
    create_workflow(&mut client, &keys, &channel_id, &workflow_id_text).await;
    let deletion = workflow_deletion(&keys, &workflow_id_text);
    let backdated = workflow_definition(&channel_id, &workflow_id_text)
        .custom_created_at(Timestamp::from(deletion.created_at.as_secs() - 60))
        .sign_with_keys(&keys)
        .expect("sign intentional backdated recreation");
    assert!(client.send_event(deletion).await.expect("delete").accepted);
    assert!(query_definition(&mut client, &keys, &workflow_id_text)
        .await
        .is_empty());
    let recreated = client
        .send_event(backdated.clone())
        .await
        .expect("recreate backdated workflow");
    assert!(
        recreated.accepted,
        "backdating is intentional: {}",
        recreated.message
    );
    let definitions = query_definition(&mut client, &keys, &workflow_id_text).await;
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].id, backdated.id);
    let pool = PgPool::connect(&database_url())
        .await
        .expect("isolated database");
    assert_eq!(
        database_representation_counts(&pool, &keys, workflow_id).await,
        (1, 1)
    );
    let trigger = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_TRIGGER), "{}")
        .tags([Tag::parse(["d", workflow_id_text.as_str()]).expect("trigger coordinate")])
        .sign_with_keys(&keys)
        .expect("sign recreated workflow trigger");
    assert!(
        client
            .send_event(trigger)
            .await
            .expect("trigger recreated workflow")
            .accepted
    );
    client.disconnect().await.expect("disconnect");
}

async fn concurrent_deletion_scenario(
    pool: &PgPool,
    barrier: &mut Option<sqlx::Transaction<'_, sqlx::Postgres>>,
    tasks: &mut tokio::task::JoinSet<Result<buzz_test_client::OkResponse, TestClientError>>,
) {
    let keys = Keys::generate();
    let channel_id = Uuid::new_v4().to_string();
    let workflow_id = Uuid::new_v4();
    let workflow_id_text = workflow_id.to_string();
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("connect");
    create_channel(&mut client, &keys, &channel_id).await;
    create_workflow(&mut client, &keys, &channel_id, &workflow_id_text).await;
    let deletion = EventBuilder::new(Kind::EventDeletion, "")
        .tags([
            Tag::parse([
                "a",
                &format!("30620:{}:{workflow_id}", keys.public_key().to_hex()),
            ])
            .expect("coordinate"),
            Tag::public_key(Keys::generate().public_key()),
        ])
        .sign_with_keys(&keys)
        .expect("sign deletion");
    let mut listener = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("listener");
    let subscription = sub_id("concurrent-listener");
    listener
        .subscribe(
            &subscription,
            vec![Filter::new()
                .kinds([Kind::EventDeletion, Kind::TextNote])
                .author(keys.public_key())],
        )
        .await
        .expect("subscribe before concurrent requests");
    assert!(listener
        .collect_until_eose(&subscription, Duration::from_secs(5))
        .await
        .expect("listener EOSE")
        .is_empty());

    // Block A's first-insert-only mention indexing. In the old split path its
    // public event is already committed and B can repair while A is held. In
    // the atomic path B must wait on A's uncommitted unique event insertion.
    // Both orderings are causally observed in Postgres, never guessed by sleep.
    let lock_key = 7_735_306_205_i64;
    *barrier = Some(pool.begin().await.expect("barrier transaction"));
    let held = barrier.as_mut().expect("barrier exists");
    let controller: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut **held)
        .await
        .expect("controller pid");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut **held)
        .await
        .expect("hold indexing barrier");
    // Interpolated values are a fixed integer and an EventId's hex encoding,
    // neither of which can contain SQL syntax.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION block_workflow_deletion_index() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN IF NEW.event_id = decode('{}', 'hex') THEN
           PERFORM pg_advisory_xact_lock({lock_key});
         END IF; RETURN NEW; END $$;
         CREATE TRIGGER block_workflow_deletion_index BEFORE INSERT ON event_mentions
         FOR EACH ROW EXECUTE FUNCTION block_workflow_deletion_index();",
        deletion.id.to_hex()
    )))
    .execute(pool)
    .await
    .expect("install event-scoped indexing barrier");

    let mut second_client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("second publisher");
    let first_event = deletion.clone();
    tasks.spawn(async move { client.send_event(first_event).await });
    let first_pid: i32 =
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let pid = sqlx::query_scalar::<_, i32>(
                "SELECT pid FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) LIMIT 1")
                .bind(controller).fetch_optional(pool).await.expect("find held first ingest");
                if let Some(pid) = pid {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("first ingest must reach indexing barrier");
    let second_event = deletion.clone();
    let second = tasks.spawn(async move { second_client.send_event(second_event).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))")
                .bind(first_pid).fetch_one(pool).await.expect("observe second ingest wait");
            if blocked || second.is_finished() { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("second ingest must either block on A or finish its repair");
    barrier
        .take()
        .expect("held barrier")
        .commit()
        .await
        .expect("release first ingest");
    let first = tasks
        .join_next()
        .await
        .expect("first task")
        .expect("join first")
        .expect("first response");
    let second = tasks
        .join_next()
        .await
        .expect("second task")
        .expect("join second")
        .expect("second response");
    assert!(
        first.accepted && second.accepted,
        "both requests accepted: {first:?}, {second:?}"
    );
    let mut client = BuzzTestClient::connect(&relay_url(), &keys)
        .await
        .expect("verification client");
    let marker = flush_audit(&mut client, &keys, pool).await;
    assert_eq!(
        audit_count(pool, &deletion).await,
        1,
        "concurrent ingests must have one dispatch/audit owner"
    );
    assert_eq!(
        [&first, &second]
            .iter()
            .filter(|response| response.message.is_empty())
            .count(),
        1,
        "exactly one ingest owns normal acceptance"
    );
    assert_eq!(
        [&first, &second]
            .iter()
            .filter(|response| response.message.starts_with("duplicate:"))
            .count(),
        1,
        "the other ingest must be a completed duplicate"
    );
    // Fan-out tasks can complete out of order; require each exact event once.
    let mut expected = vec![deletion.clone(), marker];
    for _ in 0..2 {
        match listener
            .recv_event(Duration::from_secs(5))
            .await
            .expect("receive deletion/fence")
        {
            RelayMessage::Event {
                subscription_id,
                event,
            } => {
                assert_eq!(subscription_id, subscription);
                let index = expected
                    .iter()
                    .position(|candidate| candidate == event.as_ref())
                    .expect("one copy of each exact deletion/fence event");
                expected.remove(index);
            }
            other => panic!("expected deletion/fence, got {other:?}"),
        }
    }
    assert_no_live_deletion(&mut listener).await;
    assert_eq!(query_deletion(&mut client, &deletion).await, vec![deletion]);
    assert_eq!(
        database_representation_counts(pool, &keys, workflow_id).await,
        (0, 0)
    );
    assert!(query_definition(&mut client, &keys, &workflow_id_text)
        .await
        .is_empty());
    assert_trigger_rejected(&mut client, &keys, &workflow_id_text).await;
}

#[tokio::test]
#[ignore]
async fn concurrent_identical_deletions_dispatch_and_audit_once() {
    let pool = PgPool::connect(&database_url())
        .await
        .expect("isolated database");
    let mut barrier = None;
    let mut tasks = tokio::task::JoinSet::new();
    let result = AssertUnwindSafe(concurrent_deletion_scenario(
        &pool,
        &mut barrier,
        &mut tasks,
    ))
    .catch_unwind()
    .await;
    // Release blocked server transactions before dropping a trigger they use.
    if let Some(barrier) = barrier {
        barrier
            .rollback()
            .await
            .expect("release failed scenario barrier");
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    sqlx::raw_sql(
        "DROP TRIGGER IF EXISTS block_workflow_deletion_index ON event_mentions;
        DROP FUNCTION IF EXISTS block_workflow_deletion_index();",
    )
    .execute(&pool)
    .await
    .expect("remove indexing barrier");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
