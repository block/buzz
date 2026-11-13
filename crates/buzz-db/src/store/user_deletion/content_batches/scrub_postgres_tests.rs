use super::*;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    event::ThreadMetadataParams,
    user_deletion::ExecutionMode,
    CommunityId, Db,
};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

async fn setup() -> (Db, CommunityId, Uuid, Keys, Keys) {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool);
    let community = db
        .ensure_configured_community(&format!("content-scrub-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let alice = Keys::generate();
    let bob = Keys::generate();
    for k in [&alice, &bob] {
        db.ensure_user(community, k.public_key().as_bytes())
            .await
            .unwrap();
    }
    let channel = db
        .create_channel(
            community,
            "scrubbing",
            ChannelType::Stream,
            ChannelVisibility::Open,
            None,
            alice.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    (db, community, channel, alice, bob)
}

fn timestamp(event: &nostr::Event) -> DateTime<Utc> {
    DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).unwrap()
}

async fn message(
    db: &Db,
    community: CommunityId,
    channel: Uuid,
    author: &Keys,
    name: &str,
    parent: Option<(&nostr::Event, &nostr::Event, i32)>,
) -> nostr::Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    if let Some((p, r, _)) = parent {
        tags.push(Tag::parse(["e", &r.id.to_hex(), "", "root"]).unwrap());
        tags.push(Tag::parse(["e", &p.id.to_hex(), "", "reply"]).unwrap());
        tags.push(Tag::parse(["p", &p.pubkey.to_hex()]).unwrap());
        tags.push(Tag::parse(["secret", "private metadata"]).unwrap());
    }
    let e = EventBuilder::new(Kind::Custom(9), name)
        .tags(tags)
        .custom_created_at(Timestamp::now())
        .sign_with_keys(author)
        .unwrap();
    let meta = parent.map(|(p, r, depth)| ThreadMetadataParams {
        event_id: e.id.as_bytes(),
        event_created_at: timestamp(&e),
        channel_id: channel,
        parent_event_id: Some(p.id.as_bytes()),
        parent_event_created_at: Some(timestamp(p)),
        root_event_id: Some(r.id.as_bytes()),
        root_event_created_at: Some(timestamp(r)),
        depth,
        broadcast: false,
    });
    db.insert_event_with_thread_metadata(community, &e, Some(channel), meta)
        .await
        .unwrap();
    e
}

fn options(community: CommunityId, alice: &Keys) -> ContentBatchOptions {
    ContentBatchOptions {
        community: *community.as_uuid(),
        pubkey: alice.public_key().to_hex(),
        relay_pubkey: None,
        batch_size: 1,
        max_items: 1000,
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn batch_crossing_selection_pages_removes_side_rows_before_events() {
    let (db, c, ch, alice, bob) = setup().await;
    let survivor = message(&db, c, ch, &bob, "preserve", None).await;
    let before = row(&db, c, &survivor).await;
    // This item offsets the event batches by one. Batch two starts with the
    // last event of page one, followed by page two's side row and event.
    sqlx::query("INSERT INTO subscriptions (community_id,id,owner_pubkey) VALUES ($1,'offset',$2)")
        .bind(c.as_uuid())
        .bind(alice.public_key().as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    for n in 0..101 {
        let e = EventBuilder::new(Kind::Custom(9), format!("erase {n}"))
            .tags([Tag::parse(["h", &ch.to_string()]).unwrap()])
            .custom_created_at(Timestamp::from(Timestamp::now().as_secs() - 101 + n))
            .sign_with_keys(&alice)
            .unwrap();
        db.insert_event(c, &e, Some(ch)).await.unwrap();
        if n == 100 {
            sqlx::query("INSERT INTO event_mentions (community_id,event_id,pubkey_hex,event_created_at) VALUES ($1,$2,$3,$4)")
                .bind(c.as_uuid()).bind(e.id.as_bytes().as_slice())
                .bind(bob.public_key().to_hex()).bind(timestamp(&e))
                .execute(db.pool()).await.unwrap();
        }
    }
    let mut opts = options(c, &alice);
    opts.batch_size = 100;
    let result = db
        .user_deletion_store()
        .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(result.items, 103);
    assert_eq!(row(&db, c, &survivor).await, before);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM event_mentions WHERE community_id=$1")
            .bind(c.as_uuid())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}

async fn row(db: &Db, community: CommunityId, e: &nostr::Event) -> Value {
    sqlx::query_scalar("SELECT to_jsonb(e) FROM events e WHERE community_id=$1 AND id=$2")
        .bind(community.as_uuid())
        .bind(e.id.as_bytes().as_slice())
        .fetch_one(db.pool())
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn references_without_thread_metadata_are_removed_children_first() {
    let (db, c, ch, alice, _) = setup().await;
    let root = message(&db, c, ch, &alice, "root", None).await;
    // A signed edit can refer to an event while having an earlier timestamp.
    let edit = EventBuilder::new(Kind::Custom(40003), "edited")
        .tags([Tag::parse(["e", &root.id.to_hex()]).unwrap()])
        .custom_created_at(Timestamp::from(root.created_at.as_secs() - 1))
        .sign_with_keys(&alice)
        .unwrap();
    db.insert_event(c, &edit, Some(ch)).await.unwrap();
    let deletion = EventBuilder::new(Kind::Custom(5), "")
        .tags([Tag::parse(["e", &edit.id.to_hex()]).unwrap()])
        .custom_created_at(Timestamp::from(root.created_at.as_secs() + 1))
        .sign_with_keys(&alice)
        .unwrap();
    db.insert_event(c, &deletion, Some(ch)).await.unwrap();
    let result = db
        .user_deletion_store()
        .run_message_batches(
            &options(c, &alice),
            ExecutionMode::Delete,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.items, 3);
    assert_eq!(result.scrubbed_events, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn scrub_keeps_multiple_ancestors_and_matches_manual_delete_counters() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &bob, "root", None).await;
    let a = message(&db, c, ch, &alice, "erase a", Some((&root, &root, 1))).await;
    let b = message(&db, c, ch, &alice, "erase b", Some((&a, &root, 2))).await;
    let survivor = message(&db, c, ch, &bob, "preserve", Some((&b, &root, 3))).await;
    let leaf = message(&db, c, ch, &alice, "unreferenced", Some((&root, &root, 1))).await;
    let before = row(&db, c, &survivor).await;
    let store = db.user_deletion_store();
    let opts = options(c, &alice);
    let mut out = Vec::new();
    let preview = store
        .run_message_batches(&opts, ExecutionMode::ReadOnly, None, &mut out)
        .await
        .unwrap();
    assert_eq!(preview.scrubbed_events, 2);
    assert_eq!(row(&db, c, &survivor).await, before);
    let deleted = store
        .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(deleted.scrubbed_events, 2);
    for e in [&a, &b] {
        let r = row(&db, c, e).await;
        assert_eq!(r["pubkey"], "\\x");
        assert_eq!(r["sig"], "\\x");
        assert_eq!(r["content"], "");
        assert!(!r["deleted_at"].is_null());
        assert!(r["tags"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t[0] == "e" && t.as_array().unwrap().len() <= 4));
        assert!(db
            .get_event_by_id_including_deleted(c, e.id.as_bytes())
            .await
            .unwrap()
            .is_none());
        assert!(db
            .get_thread_metadata_by_event(c, e.id.as_bytes())
            .await
            .unwrap()
            .is_some());
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(c.as_uuid())
            .bind(leaf.id.as_bytes().as_slice())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(!exists);
    assert_eq!(row(&db, c, &survivor).await, before);
    let meta = db
        .get_thread_metadata_by_event(c, root.id.as_bytes())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meta.reply_count, 0);
    assert_eq!(meta.descendant_count, 1);
    let reply_meta = db
        .get_thread_metadata_by_event(c, b.id.as_bytes())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply_meta.reply_count, 1);
    assert_eq!(
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );

    // Use the actual manual-deletion storage path for the same shape.
    let root2 = message(&db, c, ch, &bob, "manual root", None).await;
    let a2 = message(&db, c, ch, &alice, "manual a", Some((&root2, &root2, 1))).await;
    let b2 = message(&db, c, ch, &alice, "manual b", Some((&a2, &root2, 2))).await;
    let _survivor2 = message(&db, c, ch, &bob, "manual survivor", Some((&b2, &root2, 3))).await;
    let leaf2 = message(&db, c, ch, &alice, "manual leaf", Some((&root2, &root2, 1))).await;
    for (e, parent) in [(&a2, &root2), (&b2, &a2), (&leaf2, &root2)] {
        db.soft_delete_event_and_update_thread(
            c,
            e.id.as_bytes(),
            Some(parent.id.as_bytes()),
            Some(root2.id.as_bytes()),
        )
        .await
        .unwrap();
    }
    let manual = db
        .get_thread_metadata_by_event(c, root2.id.as_bytes())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (manual.reply_count, manual.descendant_count),
        (meta.reply_count, meta.descendant_count)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn stale_resolved_reply_cannot_commit_after_parent_erasure() {
    let (db, c, ch, alice, bob) = setup().await;
    let parent = message(&db, c, ch, &alice, "parent", None).await;
    let reply = EventBuilder::new(Kind::Custom(9), "racing reply")
        .sign_with_keys(&bob)
        .unwrap();
    let meta = ThreadMetadataParams {
        event_id: reply.id.as_bytes(),
        event_created_at: timestamp(&reply),
        channel_id: ch,
        parent_event_id: Some(parent.id.as_bytes()),
        parent_event_created_at: Some(timestamp(&parent)),
        root_event_id: Some(parent.id.as_bytes()),
        root_event_created_at: Some(timestamp(&parent)),
        depth: 1,
        broadcast: false,
    };
    db.user_deletion_store()
        .run_message_batches(
            &options(c, &alice),
            ExecutionMode::Delete,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    let result = db
        .insert_event_with_thread_metadata(c, &reply, Some(ch), Some(meta))
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("reply parent disappeared"));
    assert!(db
        .get_event_by_id(c, reply.id.as_bytes())
        .await
        .unwrap()
        .is_none());
}

struct InterruptAfterEventBatch {
    line: Vec<u8>,
    event_batch: bool,
}
impl Write for InterruptAfterEventBatch {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for b in bytes {
            if *b == b'\n' {
                let value: Value = serde_json::from_slice(&self.line).unwrap();
                self.line.clear();
                if value["record"] == "batch" {
                    self.event_batch = value["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|i| i["table"] == "events");
                }
                if value["record"] == "batch_committed" && self.event_batch {
                    return Err(std::io::Error::other(
                        "simulated output interruption after commit",
                    ));
                }
            } else {
                self.line.push(*b);
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn interruption_retries_without_losing_ancestry_or_decrementing_twice() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &bob, "root", None).await;
    let a = message(&db, c, ch, &alice, "a", Some((&root, &root, 1))).await;
    let b = message(&db, c, ch, &alice, "b", Some((&a, &root, 2))).await;
    let survivor = message(&db, c, ch, &bob, "survivor", Some((&b, &root, 3))).await;
    let before = row(&db, c, &survivor).await;
    let opts = options(c, &alice);
    let mut writer = InterruptAfterEventBatch {
        line: Vec::new(),
        event_batch: false,
    };
    let store = db.user_deletion_store();
    let error = store
        .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("simulated output interruption"));
    store
        .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    for e in [&a, &b] {
        assert!(!row(&db, c, e).await["deleted_at"].is_null());
    }
    let meta = db
        .get_thread_metadata_by_event(c, root.id.as_bytes())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((meta.reply_count, meta.descendant_count), (0, 1));
    assert_eq!(row(&db, c, &survivor).await, before);
}

struct PauseAfterPlan {
    line: Vec<u8>,
    planned: Option<tokio::sync::oneshot::Sender<()>>,
    resume: std::sync::mpsc::Receiver<()>,
}
impl Write for PauseAfterPlan {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for b in bytes {
            if *b == b'\n' {
                let value: Value = serde_json::from_slice(&self.line).unwrap();
                self.line.clear();
                if value["record"] == "delete_start" {
                    self.planned.take().unwrap().send(()).unwrap();
                    tokio::task::block_in_place(|| {
                        self.resume
                            .recv_timeout(std::time::Duration::from_secs(15))
                            .unwrap();
                    });
                }
            } else {
                self.line.push(*b);
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn reply_arriving_after_planning_requires_scrub_on_retry() {
    let (db, c, ch, alice, bob) = setup().await;
    let root = message(&db, c, ch, &alice, "root", None).await;
    let (planned_tx, planned_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let opts = options(c, &alice);
    let store = db.user_deletion_store();
    let task = tokio::spawn(async move {
        let mut writer = PauseAfterPlan {
            line: Vec::new(),
            planned: Some(planned_tx),
            resume: resume_rx,
        };
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
            .await
    });
    planned_rx.await.unwrap();
    let reply = message(&db, c, ch, &bob, "new reply", Some((&root, &root, 1))).await;
    let before = row(&db, c, &reply).await;
    resume_tx.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("new surviving references"));
    assert_eq!(row(&db, c, &root).await["content"], "root");
    let result = db
        .user_deletion_store()
        .run_message_batches(
            &options(c, &alice),
            ExecutionMode::Delete,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(result.scrubbed_events, 1);
    assert_eq!(row(&db, c, &reply).await, before);
}

fn reference_id(event: &nostr::Event, case: &str) -> String {
    let id = event.id.to_hex();
    match case {
        "upper" => id.to_ascii_uppercase(),
        "mixed" => id
            .chars()
            .enumerate()
            .map(|(i, ch)| {
                if i % 2 == 0 {
                    ch.to_ascii_uppercase()
                } else {
                    ch
                }
            })
            .collect(),
        _ => id,
    }
}

async fn legacy_reference(
    db: &Db,
    c: CommunityId,
    author: &Keys,
    root: &nostr::Event,
    case: &str,
) -> nostr::Event {
    // Retain the original spelling in a signed event, without thread metadata.
    // Invalid e tags must not reach decode(), regardless of SQL predicate order.
    let event = EventBuilder::new(Kind::TextNote, "surviving reference")
        .tags([
            Tag::parse(["e", "not-hex"]).unwrap(),
            Tag::parse(["e", &"g".repeat(64)]).unwrap(),
            Tag::parse(["e", &root.id.to_hex()[..63]]).unwrap(),
            Tag::parse(["e", &format!("{}00", root.id.to_hex())]).unwrap(),
            Tag::parse(["e", &reference_id(root, case), "", "reply"]).unwrap(),
        ])
        .sign_with_keys(author)
        .unwrap();
    event.verify().unwrap();
    db.insert_event(c, &event, None).await.unwrap();
    assert!(db
        .get_thread_metadata_by_event(c, event.id.as_bytes())
        .await
        .unwrap()
        .is_none());
    event
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn legacy_references_preserve_parents_for_every_hex_case() {
    for case in ["lower", "upper", "mixed"] {
        let (db, c, ch, alice, bob) = setup().await;
        let root = message(&db, c, ch, &alice, "root", None).await;
        let leaf = message(&db, c, ch, &alice, "unreferenced", None).await;
        let survivor = legacy_reference(&db, c, &bob, &root, case).await;
        // A different tag type or a malformed ID must not retain a target.
        let unrelated = EventBuilder::new(Kind::TextNote, "unrelated")
            .tags([
                Tag::parse(["p", &leaf.id.to_hex()]).unwrap(),
                Tag::parse(["e", &format!("{}00", leaf.id.to_hex())]).unwrap(),
            ])
            .sign_with_keys(&bob)
            .unwrap();
        db.insert_event(c, &unrelated, None).await.unwrap();
        let before = row(&db, c, &survivor).await;
        let unrelated_before = row(&db, c, &unrelated).await;
        let root_before = row(&db, c, &root).await;
        let store = db.user_deletion_store();
        let opts = options(c, &alice);
        let preview = store
            .run_message_batches(&opts, ExecutionMode::ReadOnly, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(preview.scrubbed_events, 1, "{case}");
        assert_eq!(row(&db, c, &root).await, root_before);
        let deleted = store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(deleted.scrubbed_events, 1, "{case}");
        let retained = row(&db, c, &root).await;
        assert_eq!(retained["content"], "");
        assert_eq!(retained["pubkey"], "\\x");
        assert_eq!(retained["sig"], "\\x");
        assert!(!retained["deleted_at"].is_null());
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)",
        )
        .bind(c.as_uuid())
        .bind(leaf.id.as_bytes().as_slice())
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert!(!exists);
        assert_eq!(row(&db, c, &survivor).await, before);
        assert_eq!(row(&db, c, &unrelated).await, unrelated_before);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn late_legacy_references_block_deletion_for_every_hex_case() {
    for case in ["lower", "upper", "mixed"] {
        let (db, c, ch, alice, bob) = setup().await;
        let root = message(&db, c, ch, &alice, "root", None).await;
        let root_before = row(&db, c, &root).await;
        let (planned_tx, planned_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let opts = options(c, &alice);
        let store = db.user_deletion_store();
        let task = tokio::spawn(async move {
            let mut writer = PauseAfterPlan {
                line: Vec::new(),
                planned: Some(planned_tx),
                resume: resume_rx,
            };
            store
                .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
                .await
        });
        planned_rx.await.unwrap();
        let survivor = legacy_reference(&db, c, &bob, &root, case).await;
        let before = row(&db, c, &survivor).await;
        resume_tx.send(()).unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert!(
            error.to_string().contains("new surviving references"),
            "{case}: {error}"
        );
        assert_eq!(row(&db, c, &root).await, root_before);
        assert_eq!(row(&db, c, &survivor).await, before);
        let result = db
            .user_deletion_store()
            .run_message_batches(
                &options(c, &alice),
                ExecutionMode::Delete,
                None,
                &mut Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(result.scrubbed_events, 1, "{case}");
        assert_eq!(row(&db, c, &root).await["content"], "");
        assert_eq!(row(&db, c, &survivor).await, before);
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reference_index_required_before_preview_or_deletion() {
    let (db, c, ch, alice, _) = setup().await;
    let root = message(&db, c, ch, &alice, "preserve until index ready", None).await;
    let before = row(&db, c, &root).await;
    for replacement in [
        None,
        Some("CREATE INDEX idx_events_reference_ids ON events(community_id)"),
        Some("CREATE INDEX idx_events_reference_ids ON ONLY events USING GIN ((lower(jsonb_path_query_array(tags, 'strict $[*] ? (@[0] == \"e\" && @[1] like_regex \"^[0-9A-Fa-f]{64}$\")[1]', '{}'::jsonb, true)::text)::jsonb) jsonb_path_ops)"),
    ] {
        sqlx::query("DROP INDEX IF EXISTS idx_events_reference_ids")
            .execute(db.pool()).await.unwrap();
        if let Some(sql) = replacement {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
                .execute(db.pool()).await.unwrap();
        }
        for mode in [ExecutionMode::ReadOnly, ExecutionMode::Delete] {
            let error = db.user_deletion_store()
                .run_message_batches(&options(c, &alice), mode, None, &mut Vec::new())
                .await.unwrap_err();
            assert!(error.to_string().contains("event reference index"), "{error}");
            assert_eq!(row(&db, c, &root).await, before);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn reference_index_removed_after_planning_refuses_batch() {
    let (db, c, ch, alice, _) = setup().await;
    let root = message(&db, c, ch, &alice, "preserve after plan", None).await;
    let before = row(&db, c, &root).await;
    let (planned_tx, planned_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let opts = options(c, &alice);
    let store = db.user_deletion_store();
    let task = tokio::spawn(async move {
        let mut writer = PauseAfterPlan {
            line: Vec::new(),
            planned: Some(planned_tx),
            resume: resume_rx,
        };
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
            .await
    });
    planned_rx.await.unwrap();
    sqlx::query("DROP INDEX idx_events_reference_ids")
        .execute(db.pool())
        .await
        .unwrap();
    resume_tx.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert!(
        error.to_string().contains("event reference index"),
        "{error}"
    );
    assert_eq!(row(&db, c, &root).await, before);
}
