#![deny(unsafe_code)]
#![warn(missing_docs)]
//! buzz-db — Postgres event store for Buzz.
//!
//! ## Design invariants
//! - AUTH events (kind 22242) are never stored — they carry bearer tokens.
//! - Ephemeral events (20000–29999) are never stored — Redis pub/sub only.
//! - Events table is partitioned by month on `created_at`.
//! - No FK references to partitioned tables.
//! - Uses `sqlx::query()` (runtime) not `sqlx::query!()` (compile-time).
//!
//! ## Runtime and store ownership
//! Database runtime infrastructure and domain persistence are physically
//! separated behind this crate-root compatibility facade:
//!
//! - Runtime concerns own pool construction, writer/replica routing,
//!   transactions, sessions, metrics, health support, and migrations.
//! - Store concerns own domain-specific SQL, row mapping, locking, mutation
//!   rules, indexes, and focused persistence tests.
//!
//! Existing crate-root modules, records, and [`Db`] methods remain the public
//! API. The internal `runtime` and `store` namespaces are not public APIs.

mod runtime;
mod store;

/// Database error types.
pub mod error;

#[cfg(test)]
mod test_support;

pub use runtime::{
    insert_mentions, migration, replica_fence, Db, DbConfig, DbPoolStats, DbReadinessOutcome,
    ReadSession,
};

/// Valid low-cardinality `(pool_role, operation)` pairs for pool-acquisition telemetry.
pub const DB_POOL_ACQUIRE_VALID_PAIRS: [(&str, &str); 11] =
    runtime::observability::POOL_ACQUIRE_VALID_PAIRS;

/// Raw Prometheus series ceiling per relay pod for the operation-aware contract.
pub const DB_POOL_ACQUIRE_RAW_SERIES_PER_POD: usize =
    runtime::observability::POOL_ACQUIRE_RAW_SERIES_PER_POD;
pub(crate) use runtime::{
    insert_mentions_in_transaction, observability, route_proof, ReadSessionInner, RouteDecision,
    RoutePredicate,
};
pub use store::{
    admin_moderation, allowlist, api_token, archived_identities, channel, channel_members,
    community, deletion, dm, event, feed, git_repo, moderation, partition, product_feedback, push,
    reaction, relay_admin_actions, relay_invite, relay_members, relay_operators, reminder,
    replaceable, thread, usage, user, workflow,
};

pub use allowlist::AllowlistEntry;
pub use api_token::{ApiTokenRecord, TokenSummary};
pub use community::{
    ArchivedCommunityRecord, CommunityRecord, CreateCommunityWithOwnerResult,
    CreatedCommunityRecord, EnsuredCommunityRecord, OwnedCommunityRecord,
    UnarchivedCommunityRecord,
};
pub use error::{DbError, Result};
pub use event::{EventQuery, DEFAULT_MAX_PAGE_LIMIT};
pub use reaction::ReactionEventInsertOutcome;
pub use reminder::DueReminder;
pub use usage::UsageMetricsLeader;

use buzz_core::CommunityId;
