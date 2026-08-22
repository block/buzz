//! Task lifecycle enums shared across crates.
//!
//! These live in `buzz-core` (zero I/O deps) so the DB layer, the relay HTTP
//! surface, and future clients agree on one spelling of a task's status and of
//! the lifecycle events that status changes record.
//!
//! Tasks are durable work items owned by a human or a harness agent. They are
//! deliberately unrelated to `buzz-workflow`, which models the scheduled
//! execution engine.

use std::fmt;
use std::str::FromStr;

/// Where a task sits in its lifecycle.
///
/// The spelling of each variant is the value stored in `tasks.status` and
/// pinned by that column's `CHECK` constraint, so adding a variant here
/// requires a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// Accepted but not started.
    Todo,
    /// Actively being worked.
    InProgress,
    /// Cannot proceed until something else resolves.
    Blocked,
    /// Finished successfully.
    Done,
    /// Abandoned without completion.
    Cancelled,
}

impl TaskStatus {
    /// Canonical string representation (matches the `tasks.status` CHECK).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether the task has left the working set (done or cancelled).
    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }

    /// Whether `tasks.done_at` must carry a timestamp in this status.
    ///
    /// `done_at` is the completion timestamp, so it is set for `Done` and only
    /// for `Done` — cancelling a task closes it without completing it. The
    /// database enforces the same equivalence via
    /// `chk_tasks_done_at_matches_status`; this keeps the write path from
    /// having to learn that constraint by failing it.
    pub fn requires_done_at(&self) -> bool {
        matches!(self, Self::Done)
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "todo" => Ok(Self::Todo),
            "in_progress" => Ok(Self::InProgress),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("unknown task status: {other:?}")),
        }
    }
}

/// A row in the append-only `task_events` log.
///
/// Stored as free `TEXT` rather than a database enum so a new action can ship
/// across a rolling upgrade without a migration; this enum is the set the
/// relay itself writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAction {
    /// The task was created.
    Created,
    /// `status` moved from one value to another.
    StatusChanged,
    /// `assignee_pubkey` changed.
    Assigned,
    /// A human or agent left a comment.
    Commented,
    /// `title` changed.
    TitleChanged,
    /// An agent persisted its summary of the task. At most one per task.
    SummaryPersisted,
}

impl TaskAction {
    /// Canonical string representation (matches `task_events.action`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::StatusChanged => "status_changed",
            Self::Assigned => "assigned",
            Self::Commented => "commented",
            Self::TitleChanged => "title_changed",
            Self::SummaryPersisted => "summary_persisted",
        }
    }

    /// Whether at most one event with this action may exist per task.
    ///
    /// Mirrors the partial unique index `idx_task_events_one_summary_per_task`.
    pub fn is_singleton_per_task(&self) -> bool {
        matches!(self, Self::SummaryPersisted)
    }
}

impl fmt::Display for TaskAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "created" => Ok(Self::Created),
            "status_changed" => Ok(Self::StatusChanged),
            "assigned" => Ok(Self::Assigned),
            "commented" => Ok(Self::Commented),
            "title_changed" => Ok(Self::TitleChanged),
            "summary_persisted" => Ok(Self::SummaryPersisted),
            other => Err(format!("unknown task action: {other:?}")),
        }
    }
}

/// The lifecycle event a status change records, or `None` when the requested
/// status is the one the task already has.
///
/// A `PATCH` that restates the current status is idempotent: it must not append
/// a `status_changed` row claiming a transition that did not happen, otherwise
/// a client retry inflates the task's history.
pub fn status_change_action(from: TaskStatus, to: TaskStatus) -> Option<TaskAction> {
    (from != to).then_some(TaskAction::StatusChanged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_through_its_canonical_spelling() {
        for status in [
            TaskStatus::Todo,
            TaskStatus::InProgress,
            TaskStatus::Blocked,
            TaskStatus::Done,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(
                status.as_str().parse::<TaskStatus>(),
                Ok(status),
                "{status} must survive a string round trip"
            );
        }
        assert!("in-progress".parse::<TaskStatus>().is_err());
        assert!("DONE".parse::<TaskStatus>().is_err());
    }

    #[test]
    fn action_round_trips_through_its_canonical_spelling() {
        for action in [
            TaskAction::Created,
            TaskAction::StatusChanged,
            TaskAction::Assigned,
            TaskAction::Commented,
            TaskAction::TitleChanged,
            TaskAction::SummaryPersisted,
        ] {
            assert_eq!(action.as_str().parse::<TaskAction>(), Ok(action));
        }
        assert!("summary".parse::<TaskAction>().is_err());
    }

    #[test]
    fn a_real_status_change_records_status_changed() {
        assert_eq!(
            status_change_action(TaskStatus::Todo, TaskStatus::InProgress),
            Some(TaskAction::StatusChanged)
        );
        assert_eq!(
            status_change_action(TaskStatus::Blocked, TaskStatus::Done),
            Some(TaskAction::StatusChanged)
        );
        // Reopening is a transition like any other — the log is append-only,
        // so it records the move rather than rewriting the earlier one.
        assert_eq!(
            status_change_action(TaskStatus::Done, TaskStatus::Todo),
            Some(TaskAction::StatusChanged)
        );
    }

    #[test]
    fn restating_the_current_status_records_nothing() {
        for status in [
            TaskStatus::Todo,
            TaskStatus::InProgress,
            TaskStatus::Blocked,
            TaskStatus::Done,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(
                status_change_action(status, status),
                None,
                "restating {status} must not append a status_changed row"
            );
        }
    }

    #[test]
    fn done_at_is_required_exactly_for_done() {
        // Pins the Rust side of `chk_tasks_done_at_matches_status`: cancelled
        // closes a task without completing it, so it carries no done_at.
        assert!(TaskStatus::Done.requires_done_at());
        for status in [
            TaskStatus::Todo,
            TaskStatus::InProgress,
            TaskStatus::Blocked,
            TaskStatus::Cancelled,
        ] {
            assert!(
                !status.requires_done_at(),
                "{status} must not carry a completion timestamp"
            );
        }
    }

    #[test]
    fn closed_covers_both_terminal_statuses() {
        assert!(TaskStatus::Done.is_closed());
        assert!(TaskStatus::Cancelled.is_closed());
        assert!(!TaskStatus::Todo.is_closed());
        assert!(!TaskStatus::InProgress.is_closed());
        assert!(!TaskStatus::Blocked.is_closed());
    }

    #[test]
    fn only_the_summary_action_is_capped_at_one_per_task() {
        assert!(TaskAction::SummaryPersisted.is_singleton_per_task());
        for action in [
            TaskAction::Created,
            TaskAction::StatusChanged,
            TaskAction::Assigned,
            TaskAction::Commented,
            TaskAction::TitleChanged,
        ] {
            assert!(!action.is_singleton_per_task());
        }
    }
}
