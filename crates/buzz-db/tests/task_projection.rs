use buzz_core::kind::{
    KIND_STREAM_MESSAGE, KIND_TASK_REQUESTED, KIND_TASK_RESOLVED, KIND_TASK_UPDATED,
};
use buzz_core::task::TaskEventV1;
use buzz_core::CommunityId;
use buzz_db::task::{
    insert_task_event_with_projection, soft_delete_task_event_and_rebuild_projection,
    TaskProjectionOutcome,
};
use buzz_db::EventQuery;
use chrono::{TimeZone, Utc};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use uuid::Uuid;

const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

struct Fixture {
    pool: PgPool,
    schema: String,
    community: CommunityId,
    channel_id: Uuid,
    owner: Keys,
    agent: Keys,
    source: nostr::Event,
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
        let schema = format!("task_test_{}", Uuid::new_v4().simple());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
            .await
            .unwrap();
        admin.close().await;

        let scoped_url = format!("{url}?options=-c%20search_path%3D{schema}");
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .connect(&scoped_url)
            .await
            .unwrap();
        buzz_db::migration::run_migrations(&pool).await.unwrap();

        let community_uuid = Uuid::new_v4();
        let community = CommunityId::from_uuid(community_uuid);
        let channel_id = Uuid::new_v4();
        let owner = Keys::generate();
        let agent = Keys::generate();
        let owner_bytes = owner.public_key().to_bytes();
        let agent_bytes = agent.public_key().to_bytes();

        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_uuid)
            .bind(format!("tasks-{}.example", community_uuid.simple()))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2), ($1, $3)")
            .bind(community_uuid)
            .bind(owner_bytes.as_slice())
            .bind(agent_bytes.as_slice())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET agent_owner_pubkey=$1 WHERE community_id=$2 AND pubkey=$3")
            .bind(owner_bytes.as_slice())
            .bind(community_uuid)
            .bind(agent_bytes.as_slice())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels (community_id, id, name, visibility, created_by) \
             VALUES ($1, $2, 'private-task-channel', 'private', $3)",
        )
        .bind(community_uuid)
        .bind(channel_id)
        .bind(agent_bytes.as_slice())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey) \
             VALUES ($1, $2, $3), ($1, $2, $4)",
        )
        .bind(community_uuid)
        .bind(channel_id)
        .bind(owner_bytes.as_slice())
        .bind(agent_bytes.as_slice())
        .execute(&pool)
        .await
        .unwrap();

        let source = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "please review")
            .tags([Tag::parse(["h", &channel_id.to_string()]).unwrap()])
            .custom_created_at(Timestamp::from(
                Utc.with_ymd_and_hms(2026, 8, 13, 8, 0, 0)
                    .unwrap()
                    .timestamp() as u64,
            ))
            .sign_with_keys(&agent)
            .unwrap();
        buzz_db::event::insert_event_with_thread_metadata(
            &pool,
            community,
            &source,
            Some(channel_id),
            None,
        )
        .await
        .unwrap();

        Self {
            pool,
            schema,
            community,
            channel_id,
            owner,
            agent,
            source,
        }
    }

    fn task_event(
        &self,
        kind: u32,
        task_id: Uuid,
        source_version: i64,
        title: &str,
        created_at: u64,
    ) -> nostr::Event {
        self.task_event_for_source(
            kind,
            task_id,
            source_version,
            title,
            created_at,
            &self.source,
        )
    }

    fn task_event_for_source(
        &self,
        kind: u32,
        task_id: Uuid,
        source_version: i64,
        title: &str,
        created_at: u64,
        source: &nostr::Event,
    ) -> nostr::Event {
        let source_updated_at = chrono::DateTime::from_timestamp(created_at as i64, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let content = if kind == KIND_TASK_RESOLVED {
            format!(
                r#"{{"resolution":"resolved","sourceVersion":{source_version},"sourceUpdatedAt":"{source_updated_at}"}}"#
            )
        } else {
            format!(
                r#"{{"taskType":"review","title":"{title}","context":null,"priority":"medium","dueAt":null,"agentName":"Review Agent","sourceVersion":{source_version},"sourceUpdatedAt":"{source_updated_at}"}}"#
            )
        };
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags([
                Tag::parse(["d", &task_id.to_string()]).unwrap(),
                Tag::parse(["p", &self.owner.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &self.agent.public_key().to_hex()]).unwrap(),
                Tag::parse(["h", &self.channel_id.to_string()]).unwrap(),
                Tag::parse(["e", &source.id.to_hex(), "", "source"]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(&self.agent)
            .unwrap()
    }

    async fn insert_source(&self, label: &str, created_at: u64) -> nostr::Event {
        let source = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), label)
            .tags([Tag::parse(["h", &self.channel_id.to_string()]).unwrap()])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(&self.agent)
            .unwrap();
        buzz_db::event::insert_event_with_thread_metadata(
            &self.pool,
            self.community,
            &source,
            Some(self.channel_id),
            None,
        )
        .await
        .unwrap();
        source
    }

    async fn apply(&self, event: &nostr::Event) -> buzz_db::Result<TaskProjectionOutcome> {
        let parsed = TaskEventV1::parse(event).unwrap();
        insert_task_event_with_projection(&self.pool, self.community, event, &parsed).await
    }

    async fn teardown(self) {
        self.pool.close().await;
        let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .unwrap();
        admin
            .execute(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA \"{}\" CASCADE",
                self.schema
            )))
            .await
            .unwrap();
        admin.close().await;
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn duplicate_requested_event_creates_one_projection_row() {
    let fixture = Fixture::new().await;
    let task_id = Uuid::new_v4();
    let now = Utc::now().timestamp() as u64;
    let requested = fixture.task_event(KIND_TASK_REQUESTED, task_id, 1, "Review launch", now);

    assert_eq!(
        fixture.apply(&requested).await.unwrap(),
        TaskProjectionOutcome::Inserted
    );
    assert_eq!(
        fixture.apply(&requested).await.unwrap(),
        TaskProjectionOutcome::DuplicateEvent
    );
    let task: (i64, String) = sqlx::query_as(
        "SELECT source_version, status FROM buzz_tasks WHERE community_id=$1 AND id=$2",
    )
    .bind(fixture.community.as_uuid())
    .bind(task_id)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(task, (1, "open".into()));

    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn newer_update_wins_and_older_replay_is_ignored() {
    let fixture = Fixture::new().await;
    let task_id = Uuid::new_v4();
    let now = Utc::now().timestamp() as u64;
    fixture
        .apply(&fixture.task_event(KIND_TASK_REQUESTED, task_id, 1, "V1", now))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .apply(&fixture.task_event(KIND_TASK_UPDATED, task_id, 3, "V3", now + 2))
            .await
            .unwrap(),
        TaskProjectionOutcome::Updated
    );
    assert_eq!(
        fixture
            .apply(&fixture.task_event(KIND_TASK_UPDATED, task_id, 2, "V2", now + 1))
            .await
            .unwrap(),
        TaskProjectionOutcome::Stale
    );

    let task: (String, i64) = sqlx::query_as(
        "SELECT title, source_version FROM buzz_tasks WHERE community_id=$1 AND id=$2",
    )
    .bind(fixture.community.as_uuid())
    .bind(task_id)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(task, ("V3".into(), 3));

    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn terminal_transition_removes_open_task_and_rejects_reopen() {
    let fixture = Fixture::new().await;
    let task_id = Uuid::new_v4();
    let now = Utc::now().timestamp() as u64;
    fixture
        .apply(&fixture.task_event(KIND_TASK_REQUESTED, task_id, 1, "Open", now))
        .await
        .unwrap();
    assert_eq!(
        fixture
            .apply(&fixture.task_event(KIND_TASK_RESOLVED, task_id, 2, "", now + 1))
            .await
            .unwrap(),
        TaskProjectionOutcome::Resolved
    );

    let status: String =
        sqlx::query_scalar("SELECT status FROM buzz_tasks WHERE community_id=$1 AND id=$2")
            .bind(fixture.community.as_uuid())
            .bind(task_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(status, "resolved");

    let reopen = fixture
        .apply(&fixture.task_event(KIND_TASK_UPDATED, task_id, 3, "Reopen", now + 2))
        .await;
    assert!(reopen.is_err());
    let event_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(fixture.community.as_uuid())
            .bind(
                fixture
                    .task_event(KIND_TASK_UPDATED, task_id, 3, "Reopen", now + 2)
                    .id
                    .as_bytes()
                    .as_slice(),
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(
        event_count, 0,
        "invalid transition must roll back its event"
    );

    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn nostr_keyset_has_no_duplicates_or_holes_across_task_mutations() {
    let fixture = Fixture::new().await;
    let base = (Utc::now().timestamp() - 120) as u64;
    let mut original = Vec::new();
    for index in 0..4_u64 {
        let source = fixture
            .insert_source(&format!("source {index}"), base - 20 + index)
            .await;
        let task_id = Uuid::new_v4();
        let requested = fixture.task_event_for_source(
            KIND_TASK_REQUESTED,
            task_id,
            1,
            &format!("Task {index}"),
            base + 1,
            &source,
        );
        fixture.apply(&requested).await.unwrap();
        original.push((task_id, source, requested));
    }

    let mut first_query = EventQuery::for_community(fixture.community);
    first_query.channel_id = Some(fixture.channel_id);
    first_query.kinds = Some(vec![
        KIND_TASK_REQUESTED as i32,
        KIND_TASK_UPDATED as i32,
        KIND_TASK_RESOLVED as i32,
    ]);
    first_query.p_tag_hex = Some(fixture.owner.public_key().to_hex());
    first_query.limit = Some(2);
    let page_one = buzz_db::event::query_events(&fixture.pool, &first_query)
        .await
        .unwrap();
    assert_eq!(page_one.len(), 2);
    let cursor = page_one.last().unwrap();

    let new_source = fixture.insert_source("new between pages", base - 10).await;
    let new_task = fixture.task_event_for_source(
        KIND_TASK_REQUESTED,
        Uuid::new_v4(),
        1,
        "Created between pages",
        base + 10,
        &new_source,
    );
    fixture.apply(&new_task).await.unwrap();
    let updated_target = original
        .iter()
        .find(|(_, _, event)| event.id == page_one[0].event.id)
        .unwrap();
    let resolved_target = original
        .iter()
        .find(|(_, _, event)| event.id == page_one[1].event.id)
        .unwrap();
    let updated = fixture.task_event_for_source(
        KIND_TASK_UPDATED,
        updated_target.0,
        2,
        "Updated between pages",
        base + 9,
        &updated_target.1,
    );
    fixture.apply(&updated).await.unwrap();
    let resolved = fixture.task_event_for_source(
        KIND_TASK_RESOLVED,
        resolved_target.0,
        2,
        "",
        base + 8,
        &resolved_target.1,
    );
    fixture.apply(&resolved).await.unwrap();

    let mut second_query = first_query.clone();
    second_query.until = Some(
        chrono::DateTime::from_timestamp(cursor.event.created_at.as_secs() as i64, 0).unwrap(),
    );
    second_query.before_id = Some(cursor.event.id.as_bytes().to_vec());
    let page_two = buzz_db::event::query_events(&fixture.pool, &second_query)
        .await
        .unwrap();
    assert_eq!(page_two.len(), 2);

    let delivered: std::collections::HashSet<_> = page_one
        .iter()
        .chain(page_two.iter())
        .map(|stored| stored.event.id)
        .collect();
    let expected: std::collections::HashSet<_> =
        original.iter().map(|(_, _, event)| event.id).collect();
    assert_eq!(page_one.len() + page_two.len(), delivered.len());
    assert_eq!(
        delivered, expected,
        "snapshot events must appear exactly once"
    );
    for mutation in [&new_task, &updated, &resolved] {
        assert!(
            !page_two.iter().any(|stored| stored.event.id == mutation.id),
            "events accepted above the cursor belong to the live/head side"
        );
    }

    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn deleting_task_events_replays_projection_from_remaining_live_events() {
    let fixture = Fixture::new().await;
    let task_id = Uuid::new_v4();
    let base = Utc::now().timestamp() as u64;
    let requested = fixture.task_event(KIND_TASK_REQUESTED, task_id, 1, "Version one", base);
    let updated = fixture.task_event(KIND_TASK_UPDATED, task_id, 2, "Version two", base + 1);
    let resolved = fixture.task_event(KIND_TASK_RESOLVED, task_id, 3, "", base + 2);

    fixture.apply(&requested).await.unwrap();
    fixture.apply(&updated).await.unwrap();
    fixture.apply(&resolved).await.unwrap();

    assert!(soft_delete_task_event_and_rebuild_projection(
        &fixture.pool,
        fixture.community,
        resolved.id.as_bytes(),
    )
    .await
    .unwrap());
    let after_resolve_delete: (String, i64, String) = sqlx::query_as(
        "SELECT status, source_version, title FROM buzz_tasks WHERE community_id=$1 AND id=$2",
    )
    .bind(fixture.community.as_uuid())
    .bind(task_id)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(
        after_resolve_delete,
        ("open".into(), 2, "Version two".into())
    );

    assert!(soft_delete_task_event_and_rebuild_projection(
        &fixture.pool,
        fixture.community,
        updated.id.as_bytes(),
    )
    .await
    .unwrap());
    let after_update_delete: (String, i64, String) = sqlx::query_as(
        "SELECT status, source_version, title FROM buzz_tasks WHERE community_id=$1 AND id=$2",
    )
    .bind(fixture.community.as_uuid())
    .bind(task_id)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(
        after_update_delete,
        ("open".into(), 1, "Version one".into())
    );

    assert!(soft_delete_task_event_and_rebuild_projection(
        &fixture.pool,
        fixture.community,
        requested.id.as_bytes(),
    )
    .await
    .unwrap());
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM buzz_tasks WHERE community_id=$1 AND id=$2")
            .bind(fixture.community.as_uuid())
            .bind(task_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);

    let tombstones: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND id = ANY($2) AND deleted_at IS NOT NULL",
    )
    .bind(fixture.community.as_uuid())
    .bind(vec![
        requested.id.as_bytes().to_vec(),
        updated.id.as_bytes().to_vec(),
        resolved.id.as_bytes().to_vec(),
    ])
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(tombstones, 3);

    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn equal_version_transition_is_rejected_without_projection_change() {
    let fixture = Fixture::new().await;
    let task_id = Uuid::new_v4();
    let base = Utc::now().timestamp() as u64;
    let requested = fixture.task_event(KIND_TASK_REQUESTED, task_id, 1, "Version one", base);
    let resolved = fixture.task_event(KIND_TASK_RESOLVED, task_id, 2, "", base + 1);
    let competing_update =
        fixture.task_event(KIND_TASK_UPDATED, task_id, 2, "Equal loser", base + 2);

    fixture.apply(&requested).await.unwrap();
    fixture.apply(&resolved).await.unwrap();
    let conflict = fixture.apply(&competing_update).await;
    assert!(conflict.is_err());

    let winner: Vec<u8> =
        sqlx::query_scalar("SELECT task_event_id FROM buzz_tasks WHERE community_id=$1 AND id=$2")
            .bind(fixture.community.as_uuid())
            .bind(task_id)
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(winner, resolved.id.as_bytes());

    let state: (String, i64, String) = sqlx::query_as(
        "SELECT status, source_version, title FROM buzz_tasks WHERE community_id=$1 AND id=$2",
    )
    .bind(fixture.community.as_uuid())
    .bind(task_id)
    .fetch_one(&fixture.pool)
    .await
    .unwrap();
    assert_eq!(state, ("resolved".into(), 2, "Version one".into()));

    let conflict_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(fixture.community.as_uuid())
            .bind(competing_update.id.as_bytes().as_slice())
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(
        conflict_count, 0,
        "conflicting transition must roll back event"
    );

    fixture.teardown().await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn source_message_identity_is_checked_inside_the_atomic_write() {
    let fixture = Fixture::new().await;
    let now = Utc::now().timestamp() as u64;
    let task_id = Uuid::new_v4();
    let invalid = EventBuilder::new(
        Kind::Custom(KIND_TASK_REQUESTED as u16),
        r#"{"taskType":"review","title":"Wrong source","context":null,"priority":"medium","dueAt":null,"agentName":"Review Agent","sourceVersion":1,"sourceUpdatedAt":"2026-08-13T08:18:00Z"}"#,
    )
    .tags([
        Tag::parse(["d", &task_id.to_string()]).unwrap(),
        Tag::parse(["p", &fixture.owner.public_key().to_hex()]).unwrap(),
        Tag::parse(["agent", &fixture.agent.public_key().to_hex()]).unwrap(),
        Tag::parse(["h", &fixture.channel_id.to_string()]).unwrap(),
        Tag::parse([
            "e",
            &nostr::EventId::from_byte_array([42; 32]).to_hex(),
            "",
            "source",
        ])
        .unwrap(),
    ])
    .custom_created_at(Timestamp::from(now))
    .sign_with_keys(&fixture.agent)
    .unwrap();

    assert!(fixture.apply(&invalid).await.is_err());
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(fixture.community.as_uuid())
            .bind(invalid.id.as_bytes().as_slice())
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
    assert_eq!(stored, 0, "invalid source must roll back the task event");

    fixture.teardown().await;
}
