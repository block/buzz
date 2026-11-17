use super::scrub_postgres_tests::{legacy_reference, message, options, row, setup, PauseAfterPlan};
use super::*;
use crate::user_deletion::ExecutionMode;
use nostr::{EventBuilder, Keys, Kind, Tag};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn missing_metadata_lookup_can_use_existing_tags_index() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &alice, "root", None).await;
    legacy_reference(&db, c, &bob, &root, "lower").await;
    let mut tx = db.pool().begin().await.unwrap();
    // A tiny fixture normally favors a sequential scan. Verify the production
    // predicate has an index path without asserting a planner cost estimate.
    sqlx::query("SET LOCAL enable_seqscan=off")
        .execute(&mut *tx)
        .await
        .unwrap();
    let explain: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (FORMAT JSON) {}",
        legacy_replies::CANDIDATES_SQL
    )))
    .bind(c.as_uuid())
    .bind(json!([
        [["e", root.id.to_hex()]],
        [["e", root.id.to_hex().to_ascii_uppercase()]]
    ]))
    .bind(10001_i64)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let plan = explain.to_string();
    assert!(
        plan.contains("Index Cond") && plan.contains("tags @> probe.tags"),
        "{plan}"
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn legacy_candidate_limit_refuses_incomplete_deletion() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &alice, "keep on overflow", None).await;
    sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig)
        SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),$2,now(),1,$3,'reply',decode(repeat('00',64),'hex')
        FROM generate_series(1,10001) n")
        .bind(c.as_uuid()).bind(bob.public_key().as_bytes().as_slice())
        .bind(json!([["e",root.id.to_hex(),"","reply"]])).execute(db.pool()).await.unwrap();
    for mode in [ExecutionMode::ReadOnly, ExecutionMode::Delete] {
        let error = db
            .user_deletion_store()
            .run_message_batches(&options(c, &alice), mode, None, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("too many reference matches without thread metadata"),
            "{error}"
        );
        assert_eq!(row(&db, c, &root).await["content"], "keep on overflow");
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn unrelated_legacy_replies_do_not_exhaust_selected_id_lookup() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &alice, "retain for reply", None).await;
    let leaf = message(&db, c, ch, &alice, "delete", None).await;
    let unrelated = message(&db, c, ch, &bob, "unrelated root", None).await;
    let survivor = legacy_reference(&db, c, &bob, &root, "lower").await;
    let before = row(&db, c, &survivor).await;
    // More unrelated replies than the old global candidate cap. Both preview
    // and deletion must still find the one relevant reply and delete the leaf.
    sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig)
        SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),$2,now(),1,$3,'unrelated',decode(repeat('00',64),'hex')
        FROM generate_series(1,10001) n")
        .bind(c.as_uuid()).bind(bob.public_key().as_bytes().as_slice())
        .bind(json!([["e",unrelated.id.to_hex(),"","reply"]]))
        .execute(db.pool()).await.unwrap();
    let other = db
        .ensure_configured_community(&format!("other-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    db.ensure_user(other, bob.public_key().as_bytes())
        .await
        .unwrap();
    // An exact match in another community must not preserve the leaf here.
    let foreign = legacy_reference(&db, other, &bob, &leaf, "upper").await;
    let foreign_before = row(&db, other, &foreign).await;
    for mode in [ExecutionMode::ReadOnly, ExecutionMode::Delete] {
        let summary = db
            .user_deletion_store()
            .run_message_batches(&options(c, &alice), mode, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(summary.scrubbed_events, 1);
        assert_eq!(row(&db, c, &survivor).await, before);
        assert_eq!(row(&db, other, &foreign).await, foreign_before);
    }
    assert_eq!(row(&db, c, &root).await["content"], "");
    let remains: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(c.as_uuid())
            .bind(leaf.id.as_bytes().as_slice())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(!remains);
    let unrelated_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND content='unrelated'",
    )
    .bind(c.as_uuid())
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(unrelated_count, 10001);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mixed_case_reply_with_metadata_preserves_parent() {
    use super::scrub_postgres_tests::timestamp;
    use crate::event::ThreadMetadataParams;

    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &alice, "retain", None).await;
    let mut reference = root.id.to_hex();
    // Change one letter only; the saved metadata still uses the binary ID.
    if let Some(index) = reference.find(|ch: char| ch.is_ascii_alphabetic()) {
        reference[index..index + 1].make_ascii_uppercase();
    }
    assert_ne!(reference, reference.to_ascii_lowercase());
    assert_ne!(reference, reference.to_ascii_uppercase());
    let survivor = EventBuilder::new(Kind::Custom(9), "keep")
        .tags([
            Tag::parse(["h", &ch.to_string()]).unwrap(),
            Tag::parse(["e", &reference, "", "reply"]).unwrap(),
        ])
        .sign_with_keys(&bob)
        .unwrap();
    db.insert_event_with_thread_metadata(
        c,
        &survivor,
        Some(ch),
        Some(ThreadMetadataParams {
            event_id: survivor.id.as_bytes(),
            event_created_at: timestamp(&survivor),
            channel_id: ch,
            parent_event_id: Some(root.id.as_bytes()),
            parent_event_created_at: Some(timestamp(&root)),
            root_event_id: Some(root.id.as_bytes()),
            root_event_created_at: Some(timestamp(&root)),
            depth: 1,
            broadcast: false,
        }),
    )
    .await
    .unwrap();
    let before = row(&db, c, &survivor).await;
    let summary = db
        .user_deletion_store()
        .run_message_batches(
            &options(c, &alice),
            ExecutionMode::Delete,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(summary.scrubbed_events, 1);
    assert_eq!(row(&db, c, &root).await["content"], "");
    assert_eq!(row(&db, c, &survivor).await, before);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reconnected_batch_still_bounds_lock_wait_and_loses_no_rows() {
    let (db, c, ch, alice, _) = setup().await;
    let root = message(&db, c, ch, &alice, "keep on reconnect", None).await;
    let opts = options(c, &alice);
    let session = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with((*db.pool().connect_options()).clone())
        .await
        .unwrap();
    let mut tx = session.begin().await.unwrap();
    let plan = plan(&mut tx, &opts, &mut std::io::sink(), true)
        .await
        .unwrap();
    grouped::prepare(&mut tx, *c.as_uuid(), &plan)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    // Close the physical session, losing temporary tables and session settings.
    // The next batch must install its own limits on its replacement connection.
    session.acquire().await.unwrap().close().await.unwrap();
    let mut blocker = db.pool().begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(crate::deletion::community_deletion_lock_key(c))
        .execute(&mut *blocker)
        .await
        .unwrap();
    let mut output = Vec::new();
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        execute::execute_owned_batch(
            &session,
            &opts,
            &plan.start,
            &plan.batches[0],
            &mut output,
            None,
        ),
    )
    .await;
    blocker.rollback().await.unwrap();
    let error = outcome
        .expect("batch lock wait must be bounded")
        .unwrap_err();
    assert!(error.to_string().contains("timeout"), "{error}");
    // After the lock is available, the missing worklist must also fail closed.
    assert!(execute::execute_owned_batch(
        &session,
        &opts,
        &plan.start,
        &plan.batches[0],
        &mut Vec::new(),
        None
    )
    .await
    .is_err());
    assert_eq!(row(&db, c, &root).await["content"], "keep on reconnect");
    session.close().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn plain_links_and_root_only_tags_do_not_retain_deleted_messages() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &alice, "erase", None).await;
    for tags in [
        vec![vec!["e".to_owned(), root.id.to_hex()]],
        vec![vec![
            "e".to_owned(),
            root.id.to_hex(),
            "".into(),
            "mention".into(),
        ]],
        vec![vec![
            "e".to_owned(),
            root.id.to_hex(),
            "".into(),
            "root".into(),
        ]],
        vec![vec!["e".to_owned(), root.id.to_hex(), "reply".into()]],
    ] {
        let survivor = EventBuilder::new(Kind::TextNote, "link")
            .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
            .sign_with_keys(&bob)
            .unwrap();
        db.insert_event(c, &survivor, None).await.unwrap();
    }
    let before: Vec<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM events e WHERE community_id=$1 AND pubkey=$2 ORDER BY id",
    )
    .bind(c.as_uuid())
    .bind(bob.public_key().as_bytes().as_slice())
    .fetch_all(db.pool())
    .await
    .unwrap();
    let summary = db
        .user_deletion_store()
        .run_message_batches(
            &options(c, &alice),
            ExecutionMode::Delete,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(summary.scrubbed_events, 0);
    let after: Vec<Value> =
        sqlx::query_scalar("SELECT to_jsonb(e) FROM events e WHERE community_id=$1 ORDER BY id")
            .bind(c.as_uuid())
            .fetch_all(db.pool())
            .await
            .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn missing_metadata_preserves_nested_ancestry_and_last_valid_marker() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &alice, "root", None).await;
    let parent = legacy_reference(&db, c, &alice, &root, "mixed").await;
    let unused = message(&db, c, ch, &alice, "unused", None).await;
    let survivor = EventBuilder::new(Kind::TextNote, "keep")
        .tags([
            Tag::parse(["e", &unused.id.to_hex(), "", "reply"]).unwrap(),
            Tag::parse(["e", &parent.id.to_hex().to_uppercase(), "", "reply"]).unwrap(),
            Tag::parse(["e", "invalid", "", "reply"]).unwrap(),
        ])
        .sign_with_keys(&bob)
        .unwrap();
    db.insert_event(c, &survivor, None).await.unwrap();
    let before = row(&db, c, &survivor).await;
    let summary = db
        .user_deletion_store()
        .run_message_batches(
            &options(c, &alice),
            ExecutionMode::Delete,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(summary.scrubbed_events, 2);
    assert_eq!(row(&db, c, &root).await["content"], "");
    assert_eq!(row(&db, c, &parent).await["content"], "");
    assert_eq!(row(&db, c, &survivor).await, before);
    let remains: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(c.as_uuid())
            .bind(unused.id.as_bytes().as_slice())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(!remains);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn blocked_batch_output_does_not_hold_community_writers() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &alice, "root", None).await;
    let (planned, ready) = tokio::sync::oneshot::channel();
    let (resume, receive) = std::sync::mpsc::channel();
    let opts = options(c, &alice);
    let store = db.user_deletion_store();
    let task = tokio::spawn(async move {
        let mut writer = PauseAfterPlan {
            record: "batch",
            line: Vec::new(),
            planned: Some(planned),
            resume: receive,
        };
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
            .await
    });
    ready.await.unwrap();
    // This production writer needs the shared community lock. It must finish
    // while the deletion command is still blocked writing its batch line.
    let reply = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        message(&db, c, ch, &bob, "concurrent", Some((&root, &root, 1))),
    )
    .await;
    resume.send(()).unwrap();
    assert!(reply.is_ok(), "output held the community lock");
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("new surviving references"));
    assert_eq!(row(&db, c, &root).await["content"], "root");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn ownership_change_after_planning_preserves_agent_content() {
    let (db, c, ch, alice, bob) = setup().await;
    let agent = Keys::generate();
    db.ensure_user(c, agent.public_key().as_bytes())
        .await
        .unwrap();
    sqlx::query("UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3")
        .bind(c.as_uuid())
        .bind(alice.public_key().as_bytes().as_slice())
        .bind(agent.public_key().as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    let event = message(&db, c, ch, &agent, "must survive transfer", None).await;
    let before = row(&db, c, &event).await;
    let (planned, ready) = tokio::sync::oneshot::channel();
    let (resume, receive) = std::sync::mpsc::channel();
    let opts = options(c, &alice);
    let store = db.user_deletion_store();
    let task = tokio::spawn(async move {
        let mut writer = PauseAfterPlan {
            record: "delete_start",
            line: Vec::new(),
            planned: Some(planned),
            resume: receive,
        };
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
            .await
    });
    ready.await.unwrap();
    sqlx::query("UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3")
        .bind(c.as_uuid())
        .bind(bob.public_key().as_bytes().as_slice())
        .bind(agent.public_key().as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    resume.send(()).unwrap();
    assert!(task
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("ownership changed"));
    assert_eq!(row(&db, c, &event).await, before);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn conflicting_community_ownership_is_reported_and_preserved() {
    let (db, c, ch, alice, bob) = setup().await;
    let other = db
        .ensure_configured_community(&format!("other-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let agent = Keys::generate();
    for community in [c, other] {
        for key in [&alice, &bob, &agent] {
            db.ensure_user(community, key.public_key().as_bytes())
                .await
                .unwrap();
        }
        let owner = if community == c { &bob } else { &alice };
        sqlx::query("UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3")
            .bind(community.as_uuid())
            .bind(owner.public_key().as_bytes().as_slice())
            .bind(agent.public_key().as_bytes().as_slice())
            .execute(db.pool())
            .await
            .unwrap();
    }
    let event = message(&db, c, ch, &agent, "other owner", None).await;
    let before = row(&db, c, &event).await;
    let mut output = Vec::new();
    let summary = db
        .user_deletion_store()
        .run_message_batches(
            &options(c, &alice),
            ExecutionMode::Delete,
            None,
            &mut output,
        )
        .await
        .unwrap();
    assert_eq!(summary.items, 0);
    assert_eq!(row(&db, c, &event).await, before);
    let first: Value =
        serde_json::from_slice(output.split(|b| *b == b'\n').next().unwrap()).unwrap();
    assert_eq!(
        first["excluded_agents"],
        json!([{"pubkey":agent.public_key().to_hex(),"reason":"different owner in target community"}])
    );
}
