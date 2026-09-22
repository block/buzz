use super::*;
use buzz_core::channel::{ChannelType, ChannelVisibility};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

async fn fixture() -> (Db, CommunityId, Uuid, Keys, nostr::Event) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() == Ok("migration") {
        crate::migration::run_migrations(&pool).await.unwrap();
    }
    let db = Db::from_pool(pool);
    let community = db
        .ensure_configured_community(&format!("tw-{}.local", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let keys = Keys::generate();
    let channel = Uuid::new_v4();
    db.create_channel_with_id(
        community,
        channel,
        "window",
        ChannelType::Stream,
        ChannelVisibility::Private,
        None,
        &keys.public_key().to_bytes(),
        None,
    )
    .await
    .unwrap();
    let root = make_event(&keys, channel, 9, "root", None, Timestamp::now().as_secs());
    db.insert_event(community, &root, Some(channel))
        .await
        .unwrap();
    (db, community, channel, keys, root)
}

fn make_event(
    keys: &Keys,
    channel: Uuid,
    kind: u16,
    content: &str,
    target: Option<&nostr::Event>,
    ts: u64,
) -> nostr::Event {
    let mut tags = vec![Tag::parse(["h", &channel.to_string()]).unwrap()];
    if let Some(target) = target {
        tags.push(Tag::parse(["e", &target.id.to_hex(), "", "reply"]).unwrap());
    }
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .custom_created_at(Timestamp::from(ts))
        .sign_with_keys(keys)
        .unwrap()
}

fn request(channel: Uuid, root: &nostr::Event, limit: u32) -> Request {
    Request::parse(&serde_json::json!({"thread_window":true,"#h":[channel],
        "#e":[root.id.to_hex()], "kinds":[9], "limit":limit}))
    .unwrap()
}

async fn replies(
    db: &Db,
    cid: CommunityId,
    channel: Uuid,
    keys: &Keys,
    root: &nostr::Event,
    count: usize,
) -> Vec<nostr::Event> {
    let mut events = Vec::new();
    for n in 0..count {
        // Large same-second groups as well as timestamp boundaries.
        events.push(make_event(
            keys,
            channel,
            9,
            &format!("reply {n}"),
            Some(root),
            root.created_at.as_secs() + (n / 120) as u64,
        ));
    }
    // Batch fixture writes, preserving real signed events and production table
    // constraints. Production selection is exercised through the Db seam.
    for batch in events.chunks(500) {
        let mut q = QueryBuilder::new("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id) ");
        q.push_values(batch, |mut b, event| {
            b.push_bind(cid.as_uuid())
                .push_bind(event.id.to_bytes().to_vec())
                .push_bind(event.pubkey.to_bytes().to_vec())
                .push_bind(DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).unwrap())
                .push_bind(9_i32)
                .push_bind(serde_json::to_value(&event.tags).unwrap())
                .push_bind(&event.content)
                .push_bind(event.sig.serialize().to_vec())
                .push_bind(Utc::now())
                .push_bind(channel);
        });
        q.build().execute(&db.pool).await.unwrap();
        let mut q = QueryBuilder::new("INSERT INTO thread_metadata (community_id,event_created_at,event_id,channel_id,parent_event_id,parent_event_created_at,root_event_id,root_event_created_at,depth,broadcast) ");
        q.push_values(batch, |mut b, event| {
            let root_ts = DateTime::from_timestamp(root.created_at.as_secs() as i64, 0).unwrap();
            b.push_bind(cid.as_uuid())
                .push_bind(DateTime::from_timestamp(event.created_at.as_secs() as i64, 0).unwrap())
                .push_bind(event.id.to_bytes().to_vec())
                .push_bind(channel)
                .push_bind(root.id.to_bytes().to_vec())
                .push_bind(root_ts)
                .push_bind(root.id.to_bytes().to_vec())
                .push_bind(root_ts)
                .push_bind(1_i32)
                .push_bind(false);
        });
        q.build().execute(&db.pool).await.unwrap();
    }
    events.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    events
}

async fn assert_pages(db: &Db, cid: CommunityId, mut req: Request, expected: &[nostr::Event]) {
    let mut delivered = Vec::new();
    loop {
        let (window, _) = db.get_thread_window_with_session(cid, &req).await.unwrap();
        assert_eq!(window.has_more, window.next_cursor.is_some());
        assert!(window.rows.len() <= req.limit as usize);
        delivered.extend(window.rows.iter().map(|row| row.event.id));
        if let Some(next) = window.next_cursor {
            assert_ne!(req.cursor.as_ref(), Some(&next));
            req.cursor = Some(next);
        } else {
            break;
        }
        assert!(delivered.len() <= expected.len());
    }
    assert_eq!(delivered, expected.iter().map(|e| e.id).collect::<Vec<_>>());
}

async fn cardinalities() {
    let (db, cid, ch, keys, root) = fixture().await;
    // Keep bulk-ingest statistics deliberately stale in both schema paths.
    // Otherwise autoanalyze can hide a root-wide sort + repeated broad event
    // scans that exceeded the production four-second deadline at 10k replies.
    sqlx::raw_sql(
        "ALTER TABLE thread_metadata SET (autovacuum_enabled=false); \
         DO $$ DECLARE part regclass; BEGIN \
             FOR part IN SELECT inhrelid::regclass FROM pg_inherits \
                 WHERE inhparent='events'::regclass LOOP \
                 EXECUTE format('ALTER TABLE %s SET (autovacuum_enabled=false)', part); \
             END LOOP; \
         END $$;",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    for count in [0, 1, 50, 51, 501, 10_000] {
        let root = make_event(
            &keys,
            ch,
            9,
            &format!("root {count}"),
            None,
            root.created_at.as_secs(),
        );
        db.insert_event(cid, &root, Some(ch)).await.unwrap();
        let expected = replies(&db, cid, ch, &keys, &root, count).await;
        assert_pages(&db, cid, request(ch, &root, 50), &expected).await;
        let (head, _) = db
            .get_thread_window_with_session(cid, &request(ch, &root, 50))
            .await
            .unwrap();
        assert_eq!(head.has_more, count > 50);
        assert_eq!(head.rows.len(), count.min(50));
    }
    let index: (bool,bool,String) = sqlx::query_as("SELECT indisvalid,indisready,pg_get_indexdef(indexrelid) FROM pg_index WHERE indexrelid='idx_thread_metadata_window'::regclass")
        .fetch_one(&db.pool).await.unwrap();
    assert!(index.0 && index.1);
    assert!(index
        .2
        .ends_with("(community_id, root_event_id, event_created_at DESC, event_id)"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_cardinalities_and_index() {
    cardinalities().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_thread_window_cardinalities_and_index() {
    cardinalities().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_predicates_corruption_and_legacy() {
    let (db, cid, ch, keys, root) = fixture().await;
    let mut expected = replies(&db, cid, ch, &keys, &root, 51).await;
    // The 50th raw row is damaged. It must STILL be the page's cursor, not
    // the 49th delivered row; the sentinel cannot leak into the page.
    let broken = expected[49].id;
    sqlx::query("UPDATE events SET sig='\\x00' WHERE community_id=$1 AND id=$2")
        .bind(cid.as_uuid())
        .bind(broken.to_bytes().to_vec())
        .execute(&db.pool)
        .await
        .unwrap();
    let req = request(ch, &root, 50);
    let (page, _) = db.get_thread_window_with_session(cid, &req).await.unwrap();
    assert_eq!(page.rows.len(), 49);
    assert!(page.has_more);
    assert_eq!(page.next_cursor.as_ref().unwrap().id, broken.to_hex());
    assert!(!page.rows.iter().any(|r| r.event.id == expected[50].id));
    expected.remove(49);
    assert_pages(&db, cid, req, &expected).await;
    // Selection predicates must run BEFORE the probe. Exclude a deleted row,
    // a too-deep row and a different conversation kind from a 50-row set.
    let deleted = expected.remove(0);
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(cid.as_uuid())
        .bind(deleted.id.to_bytes().to_vec())
        .execute(&db.pool)
        .await
        .unwrap();
    let deep = expected.remove(0);
    sqlx::query("UPDATE thread_metadata SET depth=2 WHERE community_id=$1 AND event_id=$2")
        .bind(cid.as_uuid())
        .bind(deep.id.to_bytes().to_vec())
        .execute(&db.pool)
        .await
        .unwrap();
    let other = expected.remove(0);
    sqlx::query("UPDATE events SET kind=40002 WHERE community_id=$1 AND id=$2")
        .bind(cid.as_uuid())
        .bind(other.id.to_bytes().to_vec())
        .execute(&db.pool)
        .await
        .unwrap();
    let mut req = request(ch, &root, 50);
    req.depth = 1;
    let (page, _) = db.get_thread_window_with_session(cid, &req).await.unwrap();
    assert!(!page.has_more);
    assert_eq!(page.rows.len(), expected.len());
    assert_pages(&db, cid, req.clone(), &expected).await;
    // Deleting the root does not hide the descendants.
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(cid.as_uuid())
        .bind(root.id.to_bytes().to_vec())
        .execute(&db.pool)
        .await
        .unwrap();
    assert_pages(&db, cid, req.clone(), &expected).await;
    req.channel = Uuid::new_v4();
    let (page, _) = db.get_thread_window_with_session(cid, &req).await.unwrap();
    assert!(page.rows.is_empty() && !page.has_more && !page.root_in_channel);
    let legacy = db
        .get_thread_replies(cid, &root.id.to_bytes(), None, 100, None)
        .await
        .unwrap();
    assert!(legacy
        .windows(2)
        .all(|p| (p[0].created_at, &p[0].event_id) < (p[1].created_at, &p[1].event_id)));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_aux_1001_with_damaged_row_fails_instead_of_false_eof() {
    let (db, cid, ch, keys, root) = fixture().await;
    let rows = replies(&db, cid, ch, &keys, &root, 1001).await;
    // Change kinds only in the isolated corruption fixture. Reconstruction is
    // structural in the existing store; signatures are not reverified on read.
    sqlx::query("UPDATE events SET kind=40003 WHERE community_id=$1 AND id<>$2")
        .bind(cid.as_uuid())
        .bind(root.id.to_bytes().to_vec())
        .execute(&db.pool)
        .await
        .unwrap();
    let targets = vec![root.id.to_hex()];
    let accessible = [ch];
    let mut query = AuxQuery {
        community: cid,
        targets: &targets,
        kinds: &[40003],
        accessible: &accessible,
        cursor: None,
    };
    let mut session = ReadSession {
        inner: ReadSessionInner::Writer(db.pool.clone()),
    };
    let first = session
        .thread_window_aux(&query, &mut AuxBudget::default())
        .await
        .unwrap();
    assert_eq!(first.events.len(), 1000);
    assert!(first.next_cursor.is_some());
    query.cursor = first.next_cursor;
    let tail = session
        .thread_window_aux(&query, &mut AuxBudget::default())
        .await
        .unwrap();
    assert_eq!(tail.events.len(), 1);
    assert!(tail.next_cursor.is_none());
    query.cursor = None;
    sqlx::query("UPDATE events SET sig='\\x00' WHERE community_id=$1 AND id=$2")
        .bind(cid.as_uuid())
        .bind(rows[500].id.to_bytes().to_vec())
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(matches!(
        session
            .thread_window_aux(&query, &mut AuxBudget::default())
            .await,
        Err(DbError::InvalidData(_))
    ));
    // A deleted damaged payload still supplies its ID for delete-of-aux.
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(cid.as_uuid())
        .bind(rows[500].id.to_bytes().to_vec())
        .execute(&db.pool)
        .await
        .unwrap();
    let first = session
        .thread_window_aux(&query, &mut AuxBudget::default())
        .await
        .unwrap();
    assert_eq!(first.events.len(), 999);
    assert_eq!(first.target_ids.len(), 1000);
    assert!(first.target_ids.contains(&rows[500].id.to_hex()));
    assert!(first.next_cursor.is_some());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_aux_query_budget_fails_on_65th_real_scan() {
    let (db, community, channel, _, root) = fixture().await;
    let targets = [root.id.to_hex()];
    let query = AuxQuery {
        community,
        targets: &targets,
        kinds: &[7],
        accessible: &[channel],
        cursor: None,
    };
    let mut session = ReadSession {
        inner: ReadSessionInner::Writer(db.pool.clone()),
    };
    let mut budget = AuxBudget::default();
    for _ in 0..64 {
        assert!(session
            .thread_window_aux(&query, &mut budget)
            .await
            .unwrap()
            .events
            .is_empty());
    }
    let result = session.thread_window_aux(&query, &mut budget).await;
    assert!(
        matches!(result, Err(DbError::InvalidData(ref message)) if message.contains("query budget"))
    );
    assert_eq!(budget.queries, 65);
    assert_eq!(budget.rows, 0);
}
