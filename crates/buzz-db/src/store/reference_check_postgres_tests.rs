//! Writers that name a channel or a user reject a missing parent in
//! application code. Each test drops the foreign keys it covers from the
//! per-test database first, so a pass proves the in-transaction check rather
//! than the constraint.
//!
//! The relay-member references (join-policy acceptances) are covered next to
//! the removal path they race, in `relay_members.rs`.

use sqlx::PgPool;
use uuid::Uuid;

use crate::channel_members::{
    lock_channel_archived_at_in_transaction, lock_channel_memberships_in_transaction,
};
use crate::error::{DbError, Result};
use crate::relay_admin_actions::{
    claim_direct_action, claim_report, resolve_report_decision_atomic, ClaimResult,
    DirectActionInput, DirectClaim,
};
use crate::user::{ensure_user, set_agent_owner};
use crate::{CommunityId, Db};

async fn setup() -> (Db, CommunityId) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect test database");
    let db = Db::from_pool(pool);
    let community = new_community(&db).await;
    (db, community)
}

async fn new_community(db: &Db) -> CommunityId {
    db.ensure_configured_community(&format!("reference-check-{}.example", Uuid::new_v4()))
        .await
        .expect("create community")
        .id
}

async fn new_channel(db: &Db, community: CommunityId) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, created_by) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(community.as_uuid())
    .bind(format!("ch-{}", id.simple()))
    .bind([9u8; 32].as_slice())
    .execute(&db.pool)
    .await
    .expect("insert channel");
    id
}

async fn count(db: &Db, table: &str, community: CommunityId) -> i64 {
    // `relay_admin_actions` is keyed by `report_community_id`.
    let column = if table == "relay_admin_actions" {
        "report_community_id"
    } else {
        "community_id"
    };
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE {column} = $1"
    )))
    .bind(community.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("count rows")
}

fn assert_not_found<T: std::fmt::Debug>(result: Result<T>, what: &str) {
    match result {
        Err(DbError::NotFound(message)) => {
            assert!(message.contains(what), "unexpected message: {message}")
        }
        other => panic!("expected NotFound({what}), got {other:?}"),
    }
}

// -- Row 35: channel_members -> channels ---------------------------------------

#[tokio::test]
#[ignore = "requires Postgres"]
async fn insert_auto_membership_rejects_missing_channel_without_foreign_key() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("channel_members", "channels")])
        .await;
    let member = [0x41; 32];
    let creator = [0x42; 32];
    let other = new_community(&db).await;
    let elsewhere = new_channel(&db, other).await;

    for missing in [Uuid::new_v4(), elsewhere] {
        let mut tx = db
            .begin_event_write_transaction(community)
            .await
            .expect("begin join");
        let mut locks = lock_channel_memberships_in_transaction(&mut tx, &[missing])
            .await
            .expect("membership lock");
        assert_not_found(
            locks
                .insert_auto_membership(missing, &member, &creator)
                .await,
            "channel",
        );
        drop(locks);
        tx.rollback().await.expect("rollback join");
    }
    assert_eq!(count(&db, "channel_members", community).await, 0);
    assert_eq!(count(&db, "channel_members", other).await, 0);

    // Control: an existing channel is joined, and a re-add stays idempotent.
    let channel = new_channel(&db, community).await;
    for _ in 0..2 {
        let mut tx = db
            .begin_event_write_transaction(community)
            .await
            .expect("begin join");
        let mut locks = lock_channel_memberships_in_transaction(&mut tx, &[channel])
            .await
            .expect("membership lock");
        locks
            .insert_auto_membership(channel, &member, &creator)
            .await
            .expect("existing channel");
        drop(locks);
        tx.commit().await.expect("commit join");
    }
    assert_eq!(count(&db, "channel_members", community).await, 1);
}

/// The huddle join and `put_members` read the channel row through this lock.
/// A missing channel must not read as "not archived"; a soft-deleted channel
/// keeps its row, so it still does.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn lock_channel_archived_at_distinguishes_missing_from_not_archived() {
    let (db, community) = setup().await;
    let live = new_channel(&db, community).await;
    let archived = new_channel(&db, community).await;
    let deleted = new_channel(&db, community).await;
    let deleted_and_archived = new_channel(&db, community).await;
    sqlx::query("UPDATE channels SET archived_at = now() WHERE id = ANY($1)")
        .bind([archived, deleted_and_archived].as_slice())
        .execute(&db.pool)
        .await
        .expect("archive");
    sqlx::query("UPDATE channels SET deleted_at = now() WHERE id = ANY($1)")
        .bind([deleted, deleted_and_archived].as_slice())
        .execute(&db.pool)
        .await
        .expect("soft delete");
    let other = new_community(&db).await;
    let elsewhere = new_channel(&db, other).await;

    let mut tx = db
        .begin_event_write_transaction(community)
        .await
        .expect("begin");
    assert_eq!(
        lock_channel_archived_at_in_transaction(&mut tx, live)
            .await
            .expect("live channel"),
        None
    );
    assert!(lock_channel_archived_at_in_transaction(&mut tx, archived)
        .await
        .expect("archived channel")
        .is_some());
    for soft_deleted in [deleted, deleted_and_archived] {
        assert_eq!(
            lock_channel_archived_at_in_transaction(&mut tx, soft_deleted)
                .await
                .expect("soft-deleted channel keeps its row"),
            None
        );
    }
    for missing in [Uuid::new_v4(), elsewhere] {
        assert!(
            matches!(
                lock_channel_archived_at_in_transaction(&mut tx, missing).await,
                Err(DbError::ChannelNotFound(id)) if id == missing
            ),
            "a channel with no row in the community is missing"
        );
    }
    tx.rollback().await.expect("rollback");
}

// -- Row 53: users.agent_owner_pubkey -> users -----------------------------------

#[tokio::test]
#[ignore = "requires Postgres"]
async fn set_agent_owner_rejects_missing_owner_with_and_without_foreign_key() {
    let (db, community) = setup().await;
    let agent = [0x51; 32];
    let owner = [0x52; 32];
    ensure_user(&db.pool, community, &agent)
        .await
        .expect("agent");
    let stored_owner = |pool: PgPool, who: [u8; 32]| async move {
        sqlx::query_scalar::<_, Option<Vec<u8>>>(
            "SELECT agent_owner_pubkey FROM users WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community.as_uuid())
        .bind(who.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read owner")
    };

    // With the owner foreign key intact the contract is the same: the check
    // runs before the UPDATE, so a missing owner is `NotFound` rather than a
    // foreign key violation, and the agent keeps no owner.
    assert_not_found(
        set_agent_owner(&db.pool, community, &agent, &owner).await,
        "owner",
    );
    assert_eq!(stored_owner(db.pool.clone(), agent).await, None);
    assert_not_found(
        set_agent_owner(&db.pool, community, &[0x53; 32], &owner).await,
        "agent pubkey",
    );

    // Now the foreign key is gone and the application check stands alone.
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("users", "users")]).await;

    // Unknown owner: refused, and the agent keeps no owner.
    assert_not_found(
        set_agent_owner(&db.pool, community, &agent, &owner).await,
        "owner",
    );
    assert_eq!(stored_owner(db.pool.clone(), agent).await, None);

    // The owner exists only in another community.
    let other = new_community(&db).await;
    ensure_user(&db.pool, other, &owner)
        .await
        .expect("other owner");
    assert_not_found(
        set_agent_owner(&db.pool, community, &agent, &owner).await,
        "owner",
    );
    assert_eq!(stored_owner(db.pool.clone(), agent).await, None);

    // An unknown agent is refused before the owner is considered.
    assert_not_found(
        set_agent_owner(&db.pool, community, &[0x53; 32], &owner).await,
        "agent pubkey",
    );

    // Control: a known owner links the agent.
    ensure_user(&db.pool, community, &owner)
        .await
        .expect("owner");
    assert!(set_agent_owner(&db.pool, community, &agent, &owner)
        .await
        .expect("link"));
    assert_eq!(
        stored_owner(db.pool.clone(), agent).await,
        Some(owner.to_vec())
    );

    // Already owned: first write wins and the answer is `false`, even for a
    // proposed owner that does not exist. The stored owner is unchanged.
    assert!(!set_agent_owner(&db.pool, community, &agent, &[0x54; 32])
        .await
        .expect("already owned"));
    assert_eq!(
        stored_owner(db.pool.clone(), agent).await,
        Some(owner.to_vec())
    );
}

// -- Row 60: thread_metadata -> channels ----------------------------------------

fn reply_event(keys: &nostr::Keys) -> nostr::Event {
    nostr::EventBuilder::new(nostr::Kind::Custom(9), Uuid::new_v4().to_string())
        .sign_with_keys(keys)
        .expect("sign")
}

fn thread_meta(event: &nostr::Event, channel: Uuid) -> crate::event::ThreadMetadataParams<'_> {
    crate::event::ThreadMetadataParams {
        event_id: event.id.as_bytes(),
        event_created_at: chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
            .expect("created_at"),
        channel_id: channel,
        parent_event_id: None,
        parent_event_created_at: None,
        root_event_id: None,
        root_event_created_at: None,
        depth: 0,
        broadcast: false,
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_metadata_writers_reject_missing_channel_without_foreign_key() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("thread_metadata", "channels")])
        .await;
    let keys = nostr::Keys::generate();
    let other = new_community(&db).await;
    let elsewhere = new_channel(&db, other).await;

    for missing in [Uuid::new_v4(), elsewhere] {
        // The event writer: the whole transaction fails, event row included.
        let event = reply_event(&keys);
        assert_not_found(
            crate::event::insert_event_with_thread_metadata(
                &db.pool,
                community,
                &event,
                Some(missing),
                Some(thread_meta(&event, missing)),
            )
            .await,
            "channel",
        );
        assert_eq!(count(&db, "events", community).await, 0);
        assert_eq!(count(&db, "thread_metadata", community).await, 0);

        // The standalone writer.
        let event = reply_event(&keys);
        let meta = thread_meta(&event, missing);
        assert_not_found(
            crate::thread::insert_thread_metadata(
                &db.pool,
                community,
                meta.event_id,
                meta.event_created_at,
                missing,
                None,
                None,
                None,
                None,
                0,
                false,
            )
            .await,
            "channel",
        );
        assert_eq!(count(&db, "thread_metadata", community).await, 0);
    }

    // Control: an existing channel is accepted, through both writers.
    let channel = new_channel(&db, community).await;
    let event = reply_event(&keys);
    let (_, inserted) = crate::event::insert_event_with_thread_metadata(
        &db.pool,
        community,
        &event,
        Some(channel),
        Some(thread_meta(&event, channel)),
    )
    .await
    .expect("existing channel");
    assert!(inserted);
    assert_eq!(count(&db, "thread_metadata", community).await, 1);
    let second = reply_event(&keys);
    let meta = thread_meta(&second, channel);
    crate::thread::insert_thread_metadata(
        &db.pool,
        community,
        meta.event_id,
        meta.event_created_at,
        channel,
        None,
        None,
        None,
        None,
        0,
        false,
    )
    .await
    .expect("existing channel");
    assert_eq!(count(&db, "thread_metadata", community).await, 2);

    // Replaying a stored event is a no-op that never reaches the metadata
    // write, so a missing channel on the replay changes nothing.
    let (_, replayed) = crate::event::insert_event_with_thread_metadata(
        &db.pool,
        community,
        &event,
        Some(channel),
        Some(thread_meta(&event, Uuid::new_v4())),
    )
    .await
    .expect("replay of a stored event");
    assert!(!replayed);
    assert_eq!(count(&db, "thread_metadata", community).await, 2);
}

// -- Rows 42/43: moderation_actions and moderation_reports -> channels -----------

fn new_report<'a>(
    event_id: &'a [u8],
    channel_id: Option<Uuid>,
    note: Option<&'a str>,
) -> crate::moderation::NewReport<'a> {
    crate::moderation::NewReport {
        report_event_id: event_id,
        reporter_pubkey: &[0x61; 32],
        target: crate::moderation::ReportTarget::Pubkey(vec![0x62; 32]),
        channel_id,
        report_type: "spam",
        note,
    }
}

fn new_action(channel_id: Option<Uuid>) -> crate::moderation::NewAction<'static> {
    crate::moderation::NewAction {
        actor_pubkey: &[0x63; 32],
        action: "delete_message",
        target_pubkey: None,
        target_event_id: None,
        channel_id,
        reason_code: None,
        public_reason: None,
        private_reason: None,
        matched_principal: None,
        actor_authority: None,
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn moderation_report_rejects_missing_channel_without_foreign_key() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("moderation_reports", "channels")])
        .await;
    let other = new_community(&db).await;
    let elsewhere = new_channel(&db, other).await;

    for missing in [Uuid::new_v4(), elsewhere] {
        assert_not_found(
            crate::moderation::insert_report(
                &db.pool,
                community,
                new_report(&[1; 32], Some(missing), None),
            )
            .await,
            "channel",
        );
    }
    assert_eq!(count(&db, "moderation_reports", community).await, 0);

    // A report with no channel references nothing.
    crate::moderation::insert_report(&db.pool, community, new_report(&[2; 32], None, None))
        .await
        .expect("no channel");

    // Existing report: re-ingest with the same event id is idempotent, but a
    // missing channel on the retry is refused first and leaves the row alone.
    let channel = new_channel(&db, community).await;
    let id = crate::moderation::insert_report(
        &db.pool,
        community,
        new_report(&[3; 32], Some(channel), Some("original")),
    )
    .await
    .expect("existing channel");
    let again = crate::moderation::insert_report(
        &db.pool,
        community,
        new_report(&[3; 32], Some(channel), Some("retry")),
    )
    .await
    .expect("idempotent retry");
    assert_eq!(again, id);
    assert_not_found(
        crate::moderation::insert_report(
            &db.pool,
            community,
            new_report(&[3; 32], Some(Uuid::new_v4()), Some("hijack")),
        )
        .await,
        "channel",
    );
    let stored = crate::moderation::get_report(&db.pool, community, id)
        .await
        .expect("read report")
        .expect("stored report");
    assert_eq!(stored.channel_id, Some(channel));
    assert_eq!(stored.note.as_deref(), Some("original"));
    assert_eq!(count(&db, "moderation_reports", community).await, 2);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn moderation_action_rejects_missing_channel_without_foreign_key() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("moderation_actions", "channels")])
        .await;
    let other = new_community(&db).await;
    let elsewhere = new_channel(&db, other).await;

    for missing in [Uuid::new_v4(), elsewhere] {
        assert_not_found(
            crate::moderation::insert_action(&db.pool, community, new_action(Some(missing))).await,
            "channel",
        );
    }
    assert_eq!(count(&db, "moderation_actions", community).await, 0);

    crate::moderation::insert_action(&db.pool, community, new_action(None))
        .await
        .expect("no channel");
    let channel = new_channel(&db, community).await;
    crate::moderation::insert_action(&db.pool, community, new_action(Some(channel)))
        .await
        .expect("existing channel");
    assert_eq!(count(&db, "moderation_actions", community).await, 2);
}

async fn open_report(db: &Db, community: CommunityId) -> Uuid {
    let event_id = [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    crate::moderation::insert_report(
        &db.pool,
        community,
        new_report(&event_id, None, Some("queued")),
    )
    .await
    .expect("open report")
}

async fn report_state(db: &Db, community: CommunityId, id: Uuid) -> (String, Option<Vec<u8>>) {
    sqlx::query_as(
        "SELECT status, resolved_by FROM moderation_reports WHERE community_id = $1 AND id = $2",
    )
    .bind(community.as_uuid())
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("read report state")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn report_resolution_writers_reject_missing_channel_without_foreign_key() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("moderation_actions", "channels")])
        .await;
    let actor = [0x71; 32];
    let target = [0x72; 32];
    let missing = Uuid::new_v4();
    let channel = new_channel(&db, community).await;

    // resolve_report_decision_atomic: a missing channel fails the audit insert
    // and rolls the status change back, so the report stays open.
    let report = open_report(&db, community).await;
    let resolve = |report: Uuid, channel: Option<Uuid>| {
        let pool = db.pool.clone();
        async move {
            resolve_report_decision_atomic(
                &pool,
                community,
                report,
                "dismissed",
                "dismiss_report",
                &actor,
                "relay_operator",
                Some(&target),
                None,
                channel,
                Some("reason"),
            )
            .await
        }
    };
    assert_not_found(resolve(report, Some(missing)).await, "channel");
    assert_eq!(
        report_state(&db, community, report).await,
        ("open".to_owned(), None)
    );
    assert_eq!(count(&db, "moderation_actions", community).await, 0);
    assert!(resolve(report, Some(channel)).await.expect("resolve"));
    assert_eq!(count(&db, "moderation_actions", community).await, 1);
    // Already resolved: the not-open outcome wins over a missing channel.
    assert!(!resolve(report, Some(missing)).await.expect("not open"));
    assert_eq!(
        report_state(&db, community, report).await,
        ("dismissed".to_owned(), Some(actor.to_vec()))
    );
    assert_eq!(count(&db, "moderation_actions", community).await, 1);

    // claim_report.
    let report = open_report(&db, community).await;
    let request_id = Uuid::new_v4();
    let claim = |report: Uuid, request_id: Uuid, channel: Option<Uuid>| {
        let pool = db.pool.clone();
        async move {
            claim_report(
                &pool,
                community,
                report,
                request_id,
                &actor,
                "operator",
                "ban",
                Some("reason"),
                None,
                "resolve:ban",
                "relay_operator",
                Some(&target),
                None,
                channel,
            )
            .await
        }
    };
    let actions_before = count(&db, "moderation_actions", community).await;
    assert_not_found(claim(report, request_id, Some(missing)).await, "channel");
    assert_eq!(
        report_state(&db, community, report).await,
        ("open".to_owned(), None)
    );
    assert_eq!(count(&db, "relay_admin_actions", community).await, 0);
    assert_eq!(
        count(&db, "moderation_actions", community).await,
        actions_before
    );
    assert!(matches!(
        claim(report, request_id, Some(channel)).await,
        Ok(ClaimResult::Claimed(_))
    ));
    // Existing rejections win over a missing channel, and change nothing.
    assert!(matches!(
        claim(report, Uuid::new_v4(), Some(missing)).await,
        Ok(ClaimResult::NotOpen(status)) if status == "processing"
    ));
    assert!(matches!(
        claim(report, request_id, Some(missing)).await,
        Ok(ClaimResult::AlreadyClaimed(_))
    ));
    assert!(matches!(
        claim(Uuid::new_v4(), Uuid::new_v4(), Some(missing)).await,
        Ok(ClaimResult::NotFound)
    ));
    assert_eq!(count(&db, "relay_admin_actions", community).await, 1);
    assert_eq!(
        count(&db, "moderation_actions", community).await,
        actions_before + 1
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn direct_action_claim_rejects_missing_channel_without_foreign_key() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(&db.pool, &[("moderation_actions", "channels")])
        .await;
    let actor = [0x81; 32];
    let author = [0x82; 32];
    let event_id = [0x83; 32];
    let channel = new_channel(&db, community).await;
    let request_id = Uuid::new_v4();
    let input = |channel_id: Option<Uuid>| DirectActionInput {
        community_id: community,
        request_id,
        actor_pubkey: &actor,
        actor_role: "operator",
        actor_authority: "relay_operator",
        action: "delete",
        reason: Some("reason"),
        timeout_secs: None,
        timeout_until: None,
        target_pubkey: Some(&author),
        target_event_id: Some(&event_id),
        channel_id,
    };

    // A missing channel fails the audit insert and rolls the action row back.
    assert_not_found(
        claim_direct_action(&db.pool, &input(Some(Uuid::new_v4()))).await,
        "channel",
    );
    assert_eq!(count(&db, "relay_admin_actions", community).await, 0);
    assert_eq!(count(&db, "moderation_actions", community).await, 0);

    assert!(matches!(
        claim_direct_action(&db.pool, &input(Some(channel))).await,
        Ok(DirectClaim::Claimed(_))
    ));
    // Replay of the accepted request is classified against the stored row, even
    // if the replay names a channel that is not there; nothing is written.
    assert!(matches!(
        claim_direct_action(&db.pool, &input(Some(Uuid::new_v4()))).await,
        Ok(DirectClaim::Existing(_))
    ));
    assert_eq!(count(&db, "relay_admin_actions", community).await, 1);
    assert_eq!(count(&db, "moderation_actions", community).await, 1);
}

// -- Lock tests ---------------------------------------------------------------

/// The channel and owner checks hold `FOR KEY SHARE` on the parent rows, so a
/// concurrent delete or key update of a parent waits for the writer. The
/// foreign keys are dropped, so only the explicit lock can block the writer
/// behind a parent held `FOR UPDATE`.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn reference_checks_key_share_lock_parents_without_foreign_keys() {
    let (db, community) = setup().await;
    crate::test_support::drop_foreign_keys_between(
        &db.pool,
        &[("moderation_actions", "channels"), ("users", "users")],
    )
    .await;
    let channel = new_channel(&db, community).await;
    let agent = [0x91; 32];
    let owner = [0x92; 32];
    ensure_user(&db.pool, community, &agent)
        .await
        .expect("agent");
    ensure_user(&db.pool, community, &owner)
        .await
        .expect("owner");

    for lock_owner in [false, true] {
        let mut holder = db.pool.begin().await.expect("begin parent lock holder");
        if lock_owner {
            sqlx::query("SELECT 1 FROM users WHERE community_id = $1 AND pubkey = $2 FOR UPDATE")
                .bind(community.as_uuid())
                .bind(owner.as_slice())
                .execute(&mut *holder)
                .await
                .expect("lock owner");
        } else {
            sqlx::query("SELECT 1 FROM channels WHERE community_id = $1 AND id = $2 FOR UPDATE")
                .bind(community.as_uuid())
                .bind(channel)
                .execute(&mut *holder)
                .await
                .expect("lock channel");
        }
        let name = format!("reference-writer-{}", Uuid::new_v4().simple());
        let pool = crate::test_support::named_pool(&name).await;
        let write = tokio::spawn(async move {
            if lock_owner {
                set_agent_owner(&pool, community, &agent, &owner)
                    .await
                    .map(|_| ())
            } else {
                crate::moderation::insert_action(&pool, community, new_action(Some(channel)))
                    .await
                    .map(|_| ())
            }
        });
        crate::test_support::wait_for_lock_wait(&db.pool, &name).await;
        holder.commit().await.expect("release parent lock");
        write
            .await
            .expect("join writer")
            .expect("writer proceeds once the parent lock is released");
    }
    assert_eq!(count(&db, "moderation_actions", community).await, 1);
}
