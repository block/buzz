//! Task and task-event persistence.
//!
//! Tasks are durable work items owned by a human or a harness agent (Claude
//! Code, Codex, the ACP mesh). They are relay-owned rows rather than Nostr
//! events, the same modeling choice already made for `workflow_runs` and
//! `workflow_approvals`, and they are unrelated to `buzz-workflow`'s scheduled
//! execution engine.
//!
//! Every statement here binds `community_id` first, matching the tenant
//! invariant that `(community_id, id)` — never a bare `id` — names a task. A
//! task id presented against the wrong tenant reads as absent, not as another
//! community's row.
//!
//! `task_events` is append-only: mutations record what changed instead of
//! overwriting history. `update_task` therefore runs the read, the write, and
//! the event append in one transaction with `SELECT … FOR UPDATE` on the task
//! row, so two concurrent PATCHes cannot interleave into a log that claims a
//! transition neither of them made.

use buzz_core::task::{status_change_action, TaskAction, TaskStatus};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder, Row as _, Transaction};
use uuid::Uuid;

use crate::error::{DbError, Result};
use crate::CommunityId;

/// Columns selected for every [`TaskRecord`]. Kept in one place so the row
/// parser and every query cannot drift apart.
///
/// A macro rather than a `const` so callers can splice it with `concat!` and
/// keep every statement a true string literal — sqlx only accepts `&'static
/// str` without an `AssertSqlSafe` escape hatch, and there is nothing dynamic
/// here worth asserting past.
macro_rules! task_columns {
    () => {
        "community_id, id, channel_id, created_by_pubkey, assignee_pubkey, \
         parent_task_id, title, body, status, priority, source, source_ref, \
         due_at, done_at, archived_at, created_at, updated_at"
    };
}

/// Columns selected for every [`TaskEventRecord`].
macro_rules! task_event_columns {
    () => {
        "id, task_id, actor_pubkey, action, from_status, to_status, body, created_at"
    };
}

/// A durable work item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    /// Task id, unique within its community.
    pub id: Uuid,
    /// Channel this task is bound to, if any.
    pub channel_id: Option<Uuid>,
    /// Creator's pubkey. Agents are users, so this covers both.
    pub created_by_pubkey: Option<Vec<u8>>,
    /// Current assignee's pubkey.
    pub assignee_pubkey: Option<Vec<u8>>,
    /// Parent task, for subtasks.
    pub parent_task_id: Option<Uuid>,
    /// Short title (1–200 characters).
    pub title: String,
    /// Long-form description.
    pub body: Option<String>,
    /// Lifecycle status.
    pub status: TaskStatus,
    /// Sort priority; higher sorts first.
    pub priority: i32,
    /// Harness origin (`manual`, `claude`, `codex`, `acp`, `mesh`, …).
    pub source: Option<String>,
    /// External reference owned by that harness.
    pub source_ref: Option<String>,
    /// Due date.
    pub due_at: Option<DateTime<Utc>>,
    /// Completion timestamp. Set exactly when `status` is `done`.
    pub done_at: Option<DateTime<Utc>>,
    /// Archive timestamp.
    pub archived_at: Option<DateTime<Utc>>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last-modification timestamp.
    pub updated_at: DateTime<Utc>,
}

/// One entry in a task's append-only history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEventRecord {
    /// Monotonic event id within the community.
    pub id: i64,
    /// The task this event belongs to.
    pub task_id: Uuid,
    /// Who performed the action.
    pub actor_pubkey: Option<Vec<u8>>,
    /// What happened.
    pub action: TaskAction,
    /// Status before a `status_changed` event.
    pub from_status: Option<TaskStatus>,
    /// Status after a `status_changed` event.
    pub to_status: Option<TaskStatus>,
    /// Comment or summary text.
    pub body: Option<String>,
    /// When it happened.
    pub created_at: DateTime<Utc>,
}

/// Fields accepted when creating a task.
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    /// Channel to bind the task to.
    pub channel_id: Option<Uuid>,
    /// Creator's pubkey (the authenticated caller).
    pub created_by_pubkey: Option<Vec<u8>>,
    /// Initial assignee.
    pub assignee_pubkey: Option<Vec<u8>>,
    /// Parent task, for subtasks.
    pub parent_task_id: Option<Uuid>,
    /// Short title (1–200 characters).
    pub title: String,
    /// Long-form description.
    pub body: Option<String>,
    /// Sort priority.
    pub priority: i32,
    /// Harness origin.
    pub source: Option<String>,
    /// External reference owned by that harness.
    pub source_ref: Option<String>,
    /// Due date.
    pub due_at: Option<DateTime<Utc>>,
}

/// Filters for [`list_tasks`]. `None` means "do not filter on this field".
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    /// Restrict to one status.
    pub status: Option<TaskStatus>,
    /// Restrict to one assignee.
    pub assignee_pubkey: Option<Vec<u8>>,
    /// Restrict to one channel.
    pub channel_id: Option<Uuid>,
    /// Restrict to tasks originating from one harness reference.
    ///
    /// Exact equality only. `source_ref` is opaque TEXT (see
    /// `migrations/0035_task_system.sql`), so no parsing or prefix matching is
    /// applied here.
    pub source_ref: Option<String>,
    /// Include archived tasks. Archived tasks are hidden by default.
    pub include_archived: bool,
    /// Maximum rows to return.
    pub limit: i64,
}

/// Fields a PATCH may change. `None` means "leave unchanged".
///
/// `assignee_pubkey` is a nested `Option` because unassigning is a real
/// operation: `Some(None)` clears the assignee, while `None` leaves it alone.
/// The same distinction applies to `due_at`.
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    /// New status.
    pub status: Option<TaskStatus>,
    /// New title.
    pub title: Option<String>,
    /// New priority.
    pub priority: Option<i32>,
    /// New due date, or `Some(None)` to clear it.
    pub due_at: Option<Option<DateTime<Utc>>>,
    /// New assignee, or `Some(None)` to unassign.
    pub assignee_pubkey: Option<Option<Vec<u8>>>,
}

impl TaskPatch {
    /// Whether the patch asks for any change at all.
    pub fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.title.is_none()
            && self.priority.is_none()
            && self.due_at.is_none()
            && self.assignee_pubkey.is_none()
    }
}

fn parse_status(raw: &str) -> Result<TaskStatus> {
    raw.parse::<TaskStatus>().map_err(DbError::InvalidData)
}

fn parse_task_row(row: &sqlx::postgres::PgRow) -> Result<TaskRecord> {
    let status: String = row.try_get("status")?;
    Ok(TaskRecord {
        id: row.try_get("id")?,
        channel_id: row.try_get("channel_id")?,
        created_by_pubkey: row.try_get("created_by_pubkey")?,
        assignee_pubkey: row.try_get("assignee_pubkey")?,
        parent_task_id: row.try_get("parent_task_id")?,
        title: row.try_get("title")?,
        body: row.try_get("body")?,
        status: parse_status(&status)?,
        priority: row.try_get("priority")?,
        source: row.try_get("source")?,
        source_ref: row.try_get("source_ref")?,
        due_at: row.try_get("due_at")?,
        done_at: row.try_get("done_at")?,
        archived_at: row.try_get("archived_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn parse_task_event_row(row: &sqlx::postgres::PgRow) -> Result<TaskEventRecord> {
    let action: String = row.try_get("action")?;
    let from_status: Option<String> = row.try_get("from_status")?;
    let to_status: Option<String> = row.try_get("to_status")?;
    Ok(TaskEventRecord {
        id: row.try_get("id")?,
        task_id: row.try_get("task_id")?,
        actor_pubkey: row.try_get("actor_pubkey")?,
        action: action.parse::<TaskAction>().map_err(DbError::InvalidData)?,
        from_status: from_status.as_deref().map(parse_status).transpose()?,
        to_status: to_status.as_deref().map(parse_status).transpose()?,
        body: row.try_get("body")?,
        created_at: row.try_get("created_at")?,
    })
}

/// Append one row to a task's history inside an open transaction.
///
/// `transition` carries the `(from, to)` pair for
/// [`TaskAction::StatusChanged`] and is `None` for every other action — the
/// two ends are only ever meaningful together, so they travel together.
async fn insert_task_event(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    task_id: Uuid,
    actor_pubkey: Option<&[u8]>,
    action: TaskAction,
    transition: Option<(TaskStatus, TaskStatus)>,
    body: Option<&str>,
) -> Result<TaskEventRecord> {
    let (from_status, to_status) = match transition {
        Some((from, to)) => (Some(from), Some(to)),
        None => (None, None),
    };
    let row = sqlx::query(concat!(
        "INSERT INTO task_events \
           (community_id, task_id, actor_pubkey, action, from_status, to_status, body) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         RETURNING ",
        task_event_columns!()
    ))
    .bind(community.as_uuid())
    .bind(task_id)
    .bind(actor_pubkey)
    .bind(action.as_str())
    .bind(from_status.map(|status| status.as_str()))
    .bind(to_status.map(|status| status.as_str()))
    .bind(body)
    .fetch_one(&mut **tx)
    .await?;
    parse_task_event_row(&row)
}

/// Create a task and its opening `created` history entry in one transaction.
///
/// The two must commit together: a task with no history would be invisible to
/// the task feed, which reads `task_events`.
pub async fn create_task(
    pool: &PgPool,
    community: CommunityId,
    new_task: NewTask,
) -> Result<TaskRecord> {
    let mut tx = pool.begin().await?;

    let row = sqlx::query(concat!(
        "INSERT INTO tasks \
           (community_id, channel_id, created_by_pubkey, assignee_pubkey, parent_task_id, \
            title, body, priority, source, source_ref, due_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         RETURNING ",
        task_columns!()
    ))
    .bind(community.as_uuid())
    .bind(new_task.channel_id)
    .bind(new_task.created_by_pubkey.as_deref())
    .bind(new_task.assignee_pubkey.as_deref())
    .bind(new_task.parent_task_id)
    .bind(&new_task.title)
    .bind(new_task.body.as_deref())
    .bind(new_task.priority)
    .bind(new_task.source.as_deref())
    .bind(new_task.source_ref.as_deref())
    .bind(new_task.due_at)
    .fetch_one(&mut *tx)
    .await?;
    let task = parse_task_row(&row)?;

    insert_task_event(
        &mut tx,
        community,
        task.id,
        new_task.created_by_pubkey.as_deref(),
        TaskAction::Created,
        None,
        None,
    )
    .await?;

    tx.commit().await?;
    Ok(task)
}

/// Read one task, or [`DbError::NotFound`] when no such task exists *in this
/// community*.
pub async fn get_task(pool: &PgPool, community: CommunityId, id: Uuid) -> Result<TaskRecord> {
    let row = sqlx::query(concat!(
        "SELECT ",
        task_columns!(),
        " FROM tasks WHERE community_id = $1 AND id = $2"
    ))
    .bind(community.as_uuid())
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("task {id}")))?;
    parse_task_row(&row)
}

/// List tasks newest-modified first, filtered by `filter`.
pub async fn list_tasks(
    pool: &PgPool,
    community: CommunityId,
    filter: &TaskFilter,
) -> Result<Vec<TaskRecord>> {
    let mut builder = QueryBuilder::<Postgres>::new("SELECT ");
    builder.push(task_columns!());
    builder.push(" FROM tasks WHERE community_id = ");
    builder.push_bind(community.as_uuid());

    if let Some(status) = filter.status {
        builder.push(" AND status = ");
        builder.push_bind(status.as_str());
    }
    if let Some(assignee) = filter.assignee_pubkey.as_deref() {
        builder.push(" AND assignee_pubkey = ");
        builder.push_bind(assignee);
    }
    if let Some(channel_id) = filter.channel_id {
        builder.push(" AND channel_id = ");
        builder.push_bind(channel_id);
    }
    if let Some(source_ref) = filter.source_ref.as_deref() {
        builder.push(" AND source_ref = ");
        builder.push_bind(source_ref.to_owned());
    }
    if !filter.include_archived {
        builder.push(" AND archived_at IS NULL");
    }
    builder.push(" ORDER BY updated_at DESC, id DESC LIMIT ");
    builder.push_bind(filter.limit);

    builder
        .build()
        .fetch_all(pool)
        .await?
        .iter()
        .map(parse_task_row)
        .collect()
}

/// Read one task's history oldest-first.
pub async fn list_task_events(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
) -> Result<Vec<TaskEventRecord>> {
    sqlx::query(concat!(
        "SELECT ",
        task_event_columns!(),
        " FROM task_events \
         WHERE community_id = $1 AND task_id = $2 \
         ORDER BY created_at ASC, id ASC"
    ))
    .bind(community.as_uuid())
    .bind(task_id)
    .fetch_all(pool)
    .await?
    .iter()
    .map(parse_task_event_row)
    .collect()
}

/// Apply a patch, appending one history row per field that actually changed.
///
/// Runs under `SELECT … FOR UPDATE` so the before-image the history records is
/// the one this transaction actually replaced. A patch whose every field
/// already holds the requested value commits no history at all, which keeps a
/// client retry from inflating the log.
///
/// `done_at` is derived from the new status rather than accepted from the
/// caller — the database's `chk_tasks_done_at_matches_status` requires the two
/// to agree, and deriving it is the only way a caller cannot violate that.
pub async fn update_task(
    pool: &PgPool,
    community: CommunityId,
    id: Uuid,
    patch: &TaskPatch,
    actor_pubkey: Option<&[u8]>,
) -> Result<TaskRecord> {
    let mut tx = pool.begin().await?;

    let current = sqlx::query(concat!(
        "SELECT ",
        task_columns!(),
        " FROM tasks WHERE community_id = $1 AND id = $2 FOR UPDATE"
    ))
    .bind(community.as_uuid())
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("task {id}")))?;
    let current = parse_task_row(&current)?;

    let new_status = patch.status.unwrap_or(current.status);
    let new_title = patch.title.clone().unwrap_or_else(|| current.title.clone());
    let new_priority = patch.priority.unwrap_or(current.priority);
    let new_due_at = patch.due_at.unwrap_or(current.due_at);
    let new_assignee = patch
        .assignee_pubkey
        .clone()
        .unwrap_or_else(|| current.assignee_pubkey.clone());
    // Derived, never caller-supplied: keeps `chk_tasks_done_at_matches_status`
    // satisfiable. Re-entering `done` preserves the original completion time.
    let new_done_at = if new_status.requires_done_at() {
        current.done_at.or_else(|| Some(Utc::now()))
    } else {
        None
    };

    let row = sqlx::query(concat!(
        "UPDATE tasks SET status = $3, title = $4, priority = $5, due_at = $6, \
                          assignee_pubkey = $7, done_at = $8, updated_at = NOW() \
         WHERE community_id = $1 AND id = $2 \
         RETURNING ",
        task_columns!()
    ))
    .bind(community.as_uuid())
    .bind(id)
    .bind(new_status.as_str())
    .bind(&new_title)
    .bind(new_priority)
    .bind(new_due_at)
    .bind(new_assignee.as_deref())
    .bind(new_done_at)
    .fetch_one(&mut *tx)
    .await?;
    let updated = parse_task_row(&row)?;

    if let Some(action) = status_change_action(current.status, new_status) {
        insert_task_event(
            &mut tx,
            community,
            id,
            actor_pubkey,
            action,
            Some((current.status, new_status)),
            None,
        )
        .await?;
    }
    if new_title != current.title {
        insert_task_event(
            &mut tx,
            community,
            id,
            actor_pubkey,
            TaskAction::TitleChanged,
            None,
            Some(&new_title),
        )
        .await?;
    }
    if new_assignee != current.assignee_pubkey {
        insert_task_event(
            &mut tx,
            community,
            id,
            actor_pubkey,
            TaskAction::Assigned,
            None,
            None,
        )
        .await?;
    }

    tx.commit().await?;
    Ok(updated)
}

/// Append a caller-supplied history entry (a comment, or an agent summary).
///
/// Returns [`DbError::NotFound`] when the task does not exist in this
/// community, so a comment can never create history for another tenant's task.
/// A second [`TaskAction::SummaryPersisted`] for the same task is rejected by
/// `idx_task_events_one_summary_per_task` and surfaces as
/// [`DbError::InvalidData`] rather than an opaque driver error.
pub async fn append_task_event(
    pool: &PgPool,
    community: CommunityId,
    task_id: Uuid,
    actor_pubkey: Option<&[u8]>,
    action: TaskAction,
    body: Option<&str>,
) -> Result<TaskEventRecord> {
    let mut tx = pool.begin().await?;

    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM tasks WHERE community_id = $1 AND id = $2 FOR UPDATE")
            .bind(community.as_uuid())
            .bind(task_id)
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Err(DbError::NotFound(format!("task {task_id}")));
    }

    let event = insert_task_event(
        &mut tx,
        community,
        task_id,
        actor_pubkey,
        action,
        None,
        body,
    )
    .await
    .map_err(|error| match &error {
        DbError::Sqlx(sqlx::Error::Database(db_error))
            if db_error.constraint() == Some("idx_task_events_one_summary_per_task") =>
        {
            DbError::InvalidData(format!("task {task_id} already has a persisted summary"))
        }
        _ => error,
    })?;

    tx.commit().await?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_patch_asks_for_nothing() {
        assert!(TaskPatch::default().is_empty());
    }

    #[test]
    fn clearing_a_field_is_not_an_empty_patch() {
        // `Some(None)` means "unassign", which is a real change. Treating it as
        // empty would silently drop the operation.
        let patch = TaskPatch {
            assignee_pubkey: Some(None),
            ..TaskPatch::default()
        };
        assert!(!patch.is_empty());

        let patch = TaskPatch {
            due_at: Some(None),
            ..TaskPatch::default()
        };
        assert!(!patch.is_empty());
    }

    #[test]
    fn every_selected_task_column_is_named_once() {
        // The row parser reads these by name; a duplicate or a stray comma
        // here would surface as a runtime decode error on every read.
        let columns: Vec<&str> = task_columns!().split(',').map(str::trim).collect();
        assert!(columns.contains(&"community_id"));
        assert!(columns.contains(&"status"));
        assert!(columns.contains(&"done_at"));
        let mut sorted = columns.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            columns.len(),
            "duplicate column in projection"
        );
    }

    // ── Live-Postgres integration coverage ──────────────────────────────────
    //
    // `#[ignore]`d, exactly like every other Postgres-backed test in this
    // crate: `just test-unit` runs `-p buzz-db --lib`, which skips them, and
    // `just test` (Docker Postgres + Redis) is what turns them on. Run one
    // directly with:
    //
    //     cargo test -p buzz-db --lib task::tests -- --ignored
    //
    // against a database that has migration 0033 applied.

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    fn test_database_url() -> String {
        std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned())
    }

    async fn setup_pool() -> PgPool {
        PgPool::connect(&test_database_url())
            .await
            .expect("connect to test DB")
    }

    async fn make_test_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("task-test-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    /// Tasks reference `users`, so a creator must exist before the insert.
    async fn make_test_user(pool: &PgPool, community: CommunityId, seed: u8) -> Vec<u8> {
        let pubkey = vec![seed; 32];
        crate::user::ensure_user(pool, community, &pubkey)
            .await
            .expect("ensure test user");
        pubkey
    }

    async fn delete_test_community(pool: &PgPool, community: CommunityId) {
        for table in ["task_events", "tasks", "users"] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DELETE FROM {table} WHERE community_id = $1"
            )))
            .bind(community.as_uuid())
            .execute(pool)
            .await
            .expect("delete test rows");
        }
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community.as_uuid())
            .execute(pool)
            .await
            .expect("delete test community");
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn create_then_list_then_get_round_trips_a_task_and_its_history() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = make_test_user(&pool, community, 0x11).await;

        let created = create_task(
            &pool,
            community,
            NewTask {
                created_by_pubkey: Some(creator.clone()),
                title: "ship the task system".to_owned(),
                body: Some("phase 1".to_owned()),
                priority: 5,
                source: Some("claude".to_owned()),
                ..NewTask::default()
            },
        )
        .await
        .expect("create task");

        assert_eq!(created.title, "ship the task system");
        assert_eq!(created.status, TaskStatus::Todo);
        assert_eq!(created.priority, 5);
        assert_eq!(created.done_at, None);
        assert_eq!(
            created.created_by_pubkey.as_deref(),
            Some(creator.as_slice())
        );

        let listed = list_tasks(
            &pool,
            community,
            &TaskFilter {
                limit: 10,
                ..TaskFilter::default()
            },
        )
        .await
        .expect("list tasks");
        assert_eq!(listed, vec![created.clone()]);

        let fetched = get_task(&pool, community, created.id)
            .await
            .expect("get task");
        assert_eq!(fetched, created);

        // create_task commits the task and its opening history entry together.
        let events = list_task_events(&pool, community, created.id)
            .await
            .expect("list events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, TaskAction::Created);

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn a_task_is_findable_by_its_source_ref() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = make_test_user(&pool, community, 0x21).await;

        let wanted = create_task(
            &pool,
            community,
            NewTask {
                created_by_pubkey: Some(creator.clone()),
                title: "from the thread we care about".to_owned(),
                source: Some("app".to_owned()),
                source_ref: Some("thread-head-aaa".to_owned()),
                ..NewTask::default()
            },
        )
        .await
        .expect("create linked task");

        let other = create_task(
            &pool,
            community,
            NewTask {
                created_by_pubkey: Some(creator.clone()),
                title: "from a different thread".to_owned(),
                source: Some("app".to_owned()),
                source_ref: Some("thread-head-bbb".to_owned()),
                ..NewTask::default()
            },
        )
        .await
        .expect("create unrelated task");

        // Exact equality: the reader queries the same key the writer wrote.
        let found = list_tasks(
            &pool,
            community,
            &TaskFilter {
                source_ref: Some("thread-head-aaa".to_owned()),
                limit: 10,
                ..TaskFilter::default()
            },
        )
        .await
        .expect("list by source_ref");
        assert_eq!(found, vec![wanted.clone()]);

        // An unknown reference is an empty page, never an error and never a
        // fallback to "everything".
        let missing = list_tasks(
            &pool,
            community,
            &TaskFilter {
                source_ref: Some("thread-head-does-not-exist".to_owned()),
                limit: 10,
                ..TaskFilter::default()
            },
        )
        .await
        .expect("list unknown source_ref");
        assert!(missing.is_empty());

        // Omitting the filter must keep today's behaviour: both tasks.
        let unfiltered = list_tasks(
            &pool,
            community,
            &TaskFilter {
                limit: 10,
                ..TaskFilter::default()
            },
        )
        .await
        .expect("list unfiltered");
        assert_eq!(unfiltered.len(), 2);
        assert!(unfiltered.contains(&wanted));
        assert!(unfiltered.contains(&other));

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn a_status_change_sets_done_at_and_appends_exactly_one_event() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let creator = make_test_user(&pool, community, 0x22).await;

        let task = create_task(
            &pool,
            community,
            NewTask {
                created_by_pubkey: Some(creator.clone()),
                title: "finish it".to_owned(),
                ..NewTask::default()
            },
        )
        .await
        .expect("create task");

        let done = update_task(
            &pool,
            community,
            task.id,
            &TaskPatch {
                status: Some(TaskStatus::Done),
                ..TaskPatch::default()
            },
            Some(&creator),
        )
        .await
        .expect("mark done");
        assert_eq!(done.status, TaskStatus::Done);
        assert!(
            done.done_at.is_some(),
            "done_at is derived from the status, not supplied by the caller"
        );

        let events = list_task_events(&pool, community, task.id)
            .await
            .expect("list events");
        assert_eq!(events.len(), 2, "created + status_changed");
        assert_eq!(events[1].action, TaskAction::StatusChanged);
        assert_eq!(events[1].from_status, Some(TaskStatus::Todo));
        assert_eq!(events[1].to_status, Some(TaskStatus::Done));

        // Restating the same status is idempotent: no second event.
        update_task(
            &pool,
            community,
            task.id,
            &TaskPatch {
                status: Some(TaskStatus::Done),
                ..TaskPatch::default()
            },
            Some(&creator),
        )
        .await
        .expect("restate done");
        let events = list_task_events(&pool, community, task.id)
            .await
            .expect("list events again");
        assert_eq!(events.len(), 2, "restating a status must append nothing");

        // Reopening clears done_at, keeping chk_tasks_done_at_matches_status
        // satisfiable.
        let reopened = update_task(
            &pool,
            community,
            task.id,
            &TaskPatch {
                status: Some(TaskStatus::Todo),
                ..TaskPatch::default()
            },
            Some(&creator),
        )
        .await
        .expect("reopen");
        assert_eq!(reopened.done_at, None);

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn a_task_id_is_invisible_to_another_community() {
        let pool = setup_pool().await;
        let owner = make_test_community(&pool).await;
        let stranger = make_test_community(&pool).await;
        let creator = make_test_user(&pool, owner, 0x33).await;

        let task = create_task(
            &pool,
            owner,
            NewTask {
                created_by_pubkey: Some(creator),
                title: "tenant-private".to_owned(),
                ..NewTask::default()
            },
        )
        .await
        .expect("create task");

        // The bare id is not a capability: presented against another tenant it
        // reads as absent, never as the owner's row.
        assert!(matches!(
            get_task(&pool, stranger, task.id).await,
            Err(DbError::NotFound(_))
        ));
        assert!(matches!(
            append_task_event(
                &pool,
                stranger,
                task.id,
                None,
                TaskAction::Commented,
                Some("leak?")
            )
            .await,
            Err(DbError::NotFound(_))
        ));

        delete_test_community(&pool, owner).await;
        delete_test_community(&pool, stranger).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn a_task_keeps_at_most_one_persisted_summary() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let actor = make_test_user(&pool, community, 0x44).await;

        let task = create_task(
            &pool,
            community,
            NewTask {
                created_by_pubkey: Some(actor.clone()),
                title: "summarize me".to_owned(),
                ..NewTask::default()
            },
        )
        .await
        .expect("create task");

        append_task_event(
            &pool,
            community,
            task.id,
            Some(&actor),
            TaskAction::SummaryPersisted,
            Some("first summary"),
        )
        .await
        .expect("first summary");

        let second = append_task_event(
            &pool,
            community,
            task.id,
            Some(&actor),
            TaskAction::SummaryPersisted,
            Some("second summary"),
        )
        .await;
        assert!(
            matches!(second, Err(DbError::InvalidData(_))),
            "the partial unique index must reject a second summary, got {second:?}"
        );

        // Ordinary comments stay unbounded.
        for _ in 0..2 {
            append_task_event(
                &pool,
                community,
                task.id,
                Some(&actor),
                TaskAction::Commented,
                Some("a comment"),
            )
            .await
            .expect("comment");
        }

        delete_test_community(&pool, community).await;
    }
}
