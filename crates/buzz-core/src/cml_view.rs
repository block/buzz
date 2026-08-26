//! Observation-time projection of reduced CML state into a UI-ready card.
//!
//! [`crate::cml::CmlTask::validate`] requires `runtime.presence` to be derived
//! from heartbeat age **at `updated_at`**, so the stored value is only correct
//! at the instant the transition was signed. A board renders later, so echoing
//! that field would report a long-dead worker as live. This module recomputes
//! liveness against an explicit observation timestamp and never mutates the
//! signed snapshot.

use serde::Serialize;

use crate::cml::{CmlStatus, CmlTask, Presence, Priority};

/// Number of leading hex characters used when displaying a commit SHA.
const SHORT_SHA_LEN: usize = 7;

/// A privacy-safe, UI-ready projection of one CML task at a point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkstreamCard {
    /// Human-readable task title from the snapshot.
    pub title: String,
    /// One testable outcome from the snapshot.
    pub objective: String,
    /// Reduced lifecycle status from the snapshot.
    pub status: CmlStatus,
    /// Task priority from the snapshot.
    pub priority: Priority,
    /// Liveness recomputed at the observation timestamp.
    pub liveness: Presence,
    /// True only when liveness is [`Presence::Online`] and a lease is unexpired.
    pub live_claim: bool,
    /// Repository in `owner/name` form.
    pub repo: String,
    /// Feature branch name.
    pub branch: String,
    /// Shortened upstream base commit.
    pub base_short: String,
    /// Shortened implementation head, absent when no head exists.
    pub head_short: Option<String>,
    /// Privacy-safe worktree alias; never an absolute path.
    pub worktree_alias: String,
    /// Pseudonymous host identifier, when assigned.
    pub host_id: Option<String>,
    /// Count of currently recorded blockers.
    pub blocker_count: usize,
    /// Current reviewer/fixer round.
    pub review_round: u8,
}

/// Recompute liveness from heartbeat age at `observed_at`.
///
/// Uses the same thresholds as snapshot validation: within one TTL is
/// [`Presence::Online`], within two is [`Presence::Stale`], beyond that (or
/// with no heartbeat at all) is [`Presence::Offline`]. A heartbeat dated after
/// `observed_at` is treated as current rather than as an error, because a card
/// must render regardless of small clock skew between hosts.
pub fn liveness_at(task: &CmlTask, observed_at: u64) -> Presence {
    let Some(heartbeat) = task.runtime.last_heartbeat_at else {
        return Presence::Offline;
    };
    let ttl = task.runtime.ttl_seconds;
    let age = observed_at.saturating_sub(heartbeat);
    if age <= ttl {
        Presence::Online
    } else if age <= ttl.saturating_mul(2) {
        Presence::Stale
    } else {
        Presence::Offline
    }
}

/// True when a lease exists and has not expired as of `observed_at`.
fn lease_held_at(task: &CmlTask, observed_at: u64) -> bool {
    task.lease
        .as_ref()
        .is_some_and(|lease| lease.expires_at > observed_at)
}

/// Shorten a commit SHA for display without inventing absent data.
fn short_sha(sha: &str) -> String {
    sha.chars().take(SHORT_SHA_LEN).collect()
}

/// Project a reduced CML task into a card as observed at `observed_at`.
///
/// Liveness is derived, never read from `runtime.presence`; git metadata is
/// surfaced as-signed with SHAs shortened and no fabricated head.
pub fn project_workstream_card(task: &CmlTask, observed_at: u64) -> WorkstreamCard {
    let liveness = liveness_at(task, observed_at);
    WorkstreamCard {
        title: task.title.clone(),
        objective: task.objective.clone(),
        status: task.status,
        priority: task.priority,
        live_claim: liveness == Presence::Online && lease_held_at(task, observed_at),
        liveness,
        repo: task.git.repo.clone(),
        branch: task.git.branch.clone(),
        base_short: short_sha(&task.git.base_sha),
        head_short: task.git.head_sha.as_deref().map(short_sha),
        worktree_alias: task.git.worktree_alias.clone(),
        host_id: task.runtime.host_id.clone(),
        blocker_count: task.blockers.len(),
        review_round: task.review.round,
    }
}
