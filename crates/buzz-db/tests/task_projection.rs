use buzz_core::kind::{
    KIND_STREAM_MESSAGE, KIND_TASK_REQUESTED, KIND_TASK_RESOLVED, KIND_TASK_UPDATED,
};
use buzz_core::task::{TaskEventV1, TaskTarget};
use buzz_core::CommunityId;
use buzz_db::task::{
    get_task_for_owner, insert_task_event_with_projection, list_tasks_for_owner, TaskListQuery,
    TaskProjectionOutcome, TaskStatus,
};
use chrono::{DateTime, TimeZone, Utc};
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
        let content = if kind == KIND_TASK_RESOLVED {
            format!(
                r#"{{"resolution":"resolved","sourceVersion":{source_version},"sourceUpdatedAt":"2026-08-13T09:00:00Z"}}"#
            )
        } else {
            format!(
                r#"{{"taskType":"review","title":"{title}","context":null,"priority":"medium","dueAt":null,"agentName":"Review Agent","sourceVersion":{source_version},"sourceUpdatedAt":"2026-08-13T08:18:00Z"}}"#
            )
        };
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags([
                Tag::parse(["d", &task_id.to_string()]).unwrap(),
                Tag::parse(["p", &self.owner.public_key().to_hex()]).unwrap(),
                Tag::parse(["agent", &self.agent.public_key().to_hex()]).unwrap(),
                Tag::parse(["h", &self.channel_id.to_string()]).unwrap(),
                Tag::parse(["e", &self.source.id.to_hex(), "", "source"]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(&self.agent)
            .unwrap()
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
    let task = get_task_for_owner(
        &fixture.pool,
        fixture.community,
        fixture.owner.public_key().as_bytes(),
        task_id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(task.source_version, 1);
    assert_eq!(task.status, TaskStatus::Open);

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

    let task = get_task_for_owner(
        &fixture.pool,
        fixture.community,
        fixture.owner.public_key().as_bytes(),
        task_id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(task.title, "V3");
    assert_eq!(task.source_version, 3);

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

    let open = list_tasks_for_owner(
        &fixture.pool,
        fixture.community,
        fixture.owner.public_key().as_bytes(),
        &[fixture.channel_id],
        &TaskListQuery::open(100, 0, Utc::now()),
    )
    .await
    .unwrap();
    assert!(open.is_empty());

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
async fn owner_tenant_and_channel_filters_are_all_required_for_reads() {
    let fixture = Fixture::new().await;
    let task_id = Uuid::new_v4();
    let now = Utc::now().timestamp() as u64;
    fixture
        .apply(&fixture.task_event(KIND_TASK_REQUESTED, task_id, 1, "Private", now))
        .await
        .unwrap();

    let stranger = Keys::generate();
    assert!(get_task_for_owner(
        &fixture.pool,
        fixture.community,
        stranger.public_key().as_bytes(),
        task_id,
    )
    .await
    .unwrap()
    .is_none());
    assert!(get_task_for_owner(
        &fixture.pool,
        CommunityId::from_uuid(Uuid::new_v4()),
        fixture.owner.public_key().as_bytes(),
        task_id,
    )
    .await
    .unwrap()
    .is_none());
    let hidden = list_tasks_for_owner(
        &fixture.pool,
        fixture.community,
        fixture.owner.public_key().as_bytes(),
        &[],
        &TaskListQuery::open(100, 0, Utc::now()),
    )
    .await
    .unwrap();
    assert!(hidden.is_empty());

    let task = get_task_for_owner(
        &fixture.pool,
        fixture.community,
        fixture.owner.public_key().as_bytes(),
        task_id,
    )
    .await
    .unwrap()
    .unwrap();
    let target =
        TaskTarget::from_bytes(task.community_id, task.channel_id, &task.source_event_id).unwrap();
    assert_eq!(target.source_event_id(), fixture.source.id);

    sqlx::query(
        "UPDATE channel_members SET removed_at=now() \
         WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
    )
    .bind(fixture.community.as_uuid())
    .bind(fixture.channel_id)
    .bind(fixture.owner.public_key().as_bytes())
    .execute(&fixture.pool)
    .await
    .unwrap();
    assert!(get_task_for_owner(
        &fixture.pool,
        fixture.community,
        fixture.owner.public_key().as_bytes(),
        task_id,
    )
    .await
    .unwrap()
    .is_none());
    assert!(list_tasks_for_owner(
        &fixture.pool,
        fixture.community,
        fixture.owner.public_key().as_bytes(),
        &[fixture.channel_id],
        &TaskListQuery::open(100, 0, Utc::now()),
    )
    .await
    .unwrap()
    .is_empty());

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

#[test]
fn query_snapshot_time_is_preserved_for_cursor_sorting() {
    let as_of: DateTime<Utc> = "2026-08-13T09:30:00Z".parse().unwrap();
    let query = TaskListQuery::open(25, 50, as_of);
    assert_eq!(query.limit, 25);
    assert_eq!(query.offset, 50);
    assert_eq!(query.as_of, as_of);
}
