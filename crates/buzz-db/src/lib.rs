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
//! This crate intentionally keeps database runtime and Buzz domain persistence
//! together while maintaining an internal boundary between them:
//!
//! - Runtime concerns own pool construction, writer/replica routing, transaction
//!   creation, session invariants, metrics, and health support.
//! - Store concerns own domain-specific SQL, row mapping, locking, mutation
//!   rules, indexes, and focused persistence tests.
//!
//! Transaction-required store operations accept [`sqlx::Transaction`] so their
//! composition requirement is visible in the type. Private connection helpers
//! are reserved for SQL primitives that are valid on any same-session
//! connection. New domains should prove this boundary incrementally instead of
//! exposing raw pools or introducing broad store traits.

/// Explicit deployment-global admin report reads.
pub mod admin_moderation;
/// Community-scoped authentication allowlist persistence.
pub mod allowlist;
/// API token storage and lookup.
pub mod api_token;
/// Relay-scoped archived identity persistence (NIP-IA).
pub mod archived_identities;
/// Channel lifecycle and metadata persistence.
pub mod channel;
/// Channel membership and roster persistence.
pub mod channel_members;
/// Community lifecycle and host-map persistence.
pub mod community;
/// Durable whole-community deletion lifecycle and PostgreSQL adapter.
pub mod deletion;
/// Direct message channel persistence.
pub mod dm;
/// Database error types.
pub mod error;
/// Event storage and retrieval.
pub mod event;
/// Home feed queries.
pub mod feed;
/// Git repository name registry (NIP-34 kind:30617).
pub mod git_repo;
/// Embedded database migrations.
pub mod migration;
/// Community moderation: reports, bans/timeouts, audit actions.
pub mod moderation;
mod observability;
/// Monthly table partition management.
pub mod partition;
/// Buzz product-feedback sidecar persistence.
pub mod product_feedback;
/// Community-scoped push lease and durable wake-outbox persistence.
pub mod push;
/// Reaction persistence.
pub mod reaction;
pub mod relay_admin_actions;
/// Use-limited relay invite persistence (v2 opaque tokens).
pub mod relay_invite;
/// Relay-level membership persistence (NIP-43).
pub mod relay_members;
/// Deployment-global operator/moderator roster persistence.
pub mod relay_operators;
/// Event-reminder delivery query, claim, and release persistence.
pub mod reminder;
/// Replaceable-event persistence and coordinate locking.
pub mod replaceable;
/// Replica freshness fence for keyset-cursor read routing.
pub mod replica_fence;
/// Thread metadata persistence.
pub mod thread;
/// Per-community usage rollup queries for Prometheus gauges.
pub mod usage;
/// User profile persistence.
pub mod user;
/// Workflow, run, and approval persistence.
pub mod workflow;

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

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::postgres::{PgConnection, PgPoolOptions};
use sqlx::{Connection, PgPool, QueryBuilder};
use std::time::Duration;
use uuid::Uuid;

use buzz_core::{CommunityId, StoredEvent};

/// Extract p-tag mentions from an event and insert into the `event_mentions` table.
///
/// This pool-owning wrapper propagates failures to its caller. Replacement writes
/// use the transaction-bound helper below so event storage and mention indexing
/// commit or roll back together. Duplicate inserts are silently skipped with
/// `INSERT ... ON CONFLICT DO NOTHING`.
pub async fn insert_mentions(
    pool: &PgPool,
    community_id: CommunityId,
    event: &nostr::Event,
    channel_id: Option<Uuid>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    insert_mentions_in_transaction(&mut tx, community_id, event, channel_id).await?;
    tx.commit().await?;
    Ok(())
}

/// Insert mention rows on the caller's transaction. Replacement writes use
/// this so the authoritative event and its discovery index commit or roll back
/// as one unit.
async fn insert_mentions_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community_id: CommunityId,
    event: &nostr::Event,
    channel_id: Option<Uuid>,
) -> Result<()> {
    let p_tags: Vec<&str> = event
        .tags
        .iter()
        .filter_map(|tag| {
            let tag_vec = tag.as_slice();
            if tag_vec.len() >= 2 && tag_vec[0] == "p" {
                Some(tag_vec[1].as_str())
            } else {
                None
            }
        })
        .collect();

    if p_tags.is_empty() {
        return Ok(());
    }

    let event_id_bytes = event.id.as_bytes();
    let created_at_secs = event.created_at.as_secs() as i64;
    let created_at = DateTime::from_timestamp(created_at_secs, 0)
        .ok_or(crate::error::DbError::InvalidTimestamp(created_at_secs))?;
    let kind = event.kind.as_u16() as u32;

    // Validate and normalize pubkeys, logging any malformed ones.
    let valid_pubkeys: Vec<String> = p_tags
        .into_iter()
        .filter(|pk| {
            if pk.len() != 64 || !pk.chars().all(|c| c.is_ascii_hexdigit()) {
                tracing::debug!(
                    event_id = %event.id,
                    invalid_ptag = pk,
                    "skipping malformed p-tag in insert_mentions"
                );
                false
            } else {
                true
            }
        })
        .map(|pk| pk.to_ascii_lowercase())
        .collect();

    if valid_pubkeys.is_empty() {
        return Ok(());
    }

    // Multi-row INSERT ... ON CONFLICT DO NOTHING, chunked to stay under
    // Postgres's 65,535 bind-parameter statement cap (6 binds per row caps a
    // single statement at ~10.9k rows). Relay-signed kind 39002 rosters carry
    // one p-tag per channel member and can exceed that. The caller owns the
    // transaction so all chunks share its commit boundary.
    const MENTION_INSERT_CHUNK_ROWS: usize = 5_000;
    for chunk in valid_pubkeys.chunks(MENTION_INSERT_CHUNK_ROWS) {
        let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
            "INSERT INTO event_mentions \
             (community_id, pubkey_hex, event_id, event_created_at, channel_id, event_kind) ",
        );

        qb.push_values(chunk, |mut b, pubkey| {
            b.push_bind(community_id.as_uuid())
                .push_bind(pubkey.as_str())
                .push_bind(event_id_bytes.as_slice())
                .push_bind(created_at)
                .push_bind(channel_id)
                .push_bind(kind as i32);
        });

        qb.push(" ON CONFLICT DO NOTHING");

        qb.build().execute(&mut **tx).await?;
    }
    Ok(())
}

/// Database handle. Clone is cheap (Arc-backed pool).
#[derive(Clone, Debug)]
pub struct Db {
    pub(crate) pool: PgPool,
    /// Maximum connections configured for this pool (from [`DbConfig::max_connections`]).
    pub(crate) max_connections: u32,
    /// Optional read-replica pool (from [`DbConfig::read_database_url`]).
    ///
    /// `None` means no replica is configured and every read routes to the
    /// writer pool — the pre-replica behavior. Only lag-tolerant reads may
    /// route here (see [`Db::read`]); locks, transactions, and anything
    /// consistency-critical stays on `pool`.
    pub(crate) read_pool: Option<PgPool>,
    /// Maximum connections configured for the read-replica pool (from
    /// [`DbConfig::read_max_connections`], defaulting to the writer's
    /// sizing). Kept separately from `max_connections` so
    /// [`Db::read_pool_stats`] reports the reader's own ceiling — a
    /// utilisation gauge derived from the writer's max would understate
    /// reader saturation by exactly the ratio of the two pool sizes.
    pub(crate) read_max_connections: u32,
    /// Freshness fence gating cursor-page routing to the replica.
    ///
    /// Starts closed; a background probe ([`replica_fence::run_probe`])
    /// commits heartbeat tokens and retains proof entries. Routing proves
    /// coverage per request on the serving reader session; when the ring is
    /// empty or stale, every routed read stays on the writer.
    pub(crate) fence: std::sync::Arc<replica_fence::ReplicaFence>,
    /// Bounded-staleness routing budget `B`: a read routed under
    /// [`RoutePredicate::Bounded`] may be served from a proved replica
    /// session only when the proved heartbeat entry is at most this old.
    /// `None` disables the bounded arm entirely (the rollout default) —
    /// bounded-stale read semantics are a product decision, not an
    /// invariant, so the gate ships off.
    pub(crate) replica_read_max_age: Option<Duration>,
    /// Whether the reader endpoint supports the Aurora PostgreSQL identity
    /// function ([`replica_fence::AURORA_IDENTITY_FN`]) — probed
    /// once per process on the first routed read (on a plain autocommit
    /// checkout, outside any request transaction) and cached. Unset means
    /// not yet probed (or the probe hit a transient error and will retry).
    /// Shared across `Db` clones.
    pub(crate) reader_aurora_identity: std::sync::Arc<std::sync::OnceLock<bool>>,
}

/// The session that served (or will serve) a routed read, so follow-up
/// queries in the same request (the channel-window aux closure) run on the
/// **same proved snapshot** — a different pooled reader session may sit at a
/// different replay position, and even the same connection advances its
/// snapshot between autocommit statements.
///
/// `Replica` holds the request's `REPEATABLE READ, READ ONLY` transaction:
/// the heartbeat observation was its first statement, so the snapshot the
/// proof was taken against is exactly the snapshot every follow-up sees.
/// Dropping the session rolls the read-only transaction back and returns
/// the connection to the pool.
///
/// `Writer` carries the writer pool: follow-ups there are authoritative by
/// construction and need no session pinning.
pub struct ReadSession {
    inner: ReadSessionInner,
}

enum ReadSessionInner {
    /// The proved replica request transaction (snapshot-anchored), plus the
    /// writer pool so a mid-request replica failure (e.g. a hot-standby
    /// recovery conflict cancelling the held snapshot) degrades the session
    /// to the writer instead of surfacing an error: degraded capacity,
    /// never holes — and never a 500 the writer could have served.
    Replica {
        tx: sqlx::Transaction<'static, sqlx::Postgres>,
        writer: PgPool,
    },
    /// The writer pool (cheap clone; Arc-backed).
    Writer(PgPool),
}

impl ReadSession {
    /// Query events on this session (see [`Db::query_events`]).
    ///
    /// If the proved replica transaction fails mid-request, the session
    /// permanently degrades to the writer and the query is re-run there.
    /// The writer is always at or ahead of any replica replay position, so
    /// the degraded follow-up can only observe *more* than the proof-time
    /// snapshot, never less — fresher aux rows, the same failure semantics
    /// as a request that routed to the writer to begin with.
    #[datastore_span(name = "read_session_query_events", system = "postgresql")]
    pub async fn query_events(&mut self, q: &EventQuery) -> Result<Vec<StoredEvent>> {
        let degraded = match &mut self.inner {
            ReadSessionInner::Replica { tx, writer } => {
                match event::query_events_on(tx, q).await {
                    Ok(rows) => return Ok(rows),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "replica session query failed mid-request; degrading to writer"
                        );
                        // Deliberately not a `buzz_db_route_decision` event:
                        // the page's route was already recorded, and the
                        // offload metric must stay one-event-per-request.
                        metrics::counter!("buzz_db_read_session_degraded").increment(1);
                        writer.clone()
                    }
                }
            }
            ReadSessionInner::Writer(pool) => return event::query_events(pool, q).await,
        };
        // Replacing the inner drops the replica transaction (rolling it
        // back and returning the reader connection to its pool).
        self.inner = ReadSessionInner::Writer(degraded.clone());
        event::query_events(&degraded, q).await
    }

    /// Whether this session is a proved replica connection (observability).
    pub fn is_replica(&self) -> bool {
        matches!(self.inner, ReadSessionInner::Replica { .. })
    }
}

/// Where one routed read is served (see [`Db::route_read`]).
enum RouteDecision {
    /// A reader request transaction whose first-statement heartbeat
    /// observation proved this fence entry — the page runs inside it. The
    /// `&'static str` is the metric reason (`covered`/`fresh`); the caller
    /// records the route only once the page is actually served from the
    /// replica, so a post-verification writer re-run or a mid-query replica
    /// failure emits exactly one `buzz_db_route_decision` event per request
    /// (the offload percentage is read straight off `decision="replica"`).
    Replica(
        sqlx::Transaction<'static, sqlx::Postgres>,
        replica_fence::TokenEntry,
        &'static str,
    ),
    /// Fail closed: serve from the writer pool (already recorded).
    Writer,
}

/// The ONLY place [`route_proof::ChannelScoped`] can be constructed. A
/// crate-root tuple struct would be mintable via `ChannelScoped(())` from
/// every descendant module — tuple-struct field privacy is module-scoped —
/// so the token lives in its own module and E0423 enforces the invariant.
mod route_proof {
    use uuid::Uuid;

    /// Proof that a query/page can only return rows with
    /// `channel_id IS NOT NULL` — the domain of the commit-time floor guard
    /// (migration 0021). `channel_ids` (retains channel-NULL rows) and
    /// `global_only = false` are explicitly NOT proofs.
    ///
    /// Each constructor keys off *how* its path proves channel-bearing-ness:
    /// a pinned query filter, a bare `Uuid` argument, or a `NOT NULL` column
    /// reached through an inner join. Do not add a universal constructor
    /// callers reshape their inputs to fit, and never fabricate a throwaway
    /// `EventQuery` purely to mint a token — the proof must be the SQL's
    /// shape, not "someone assembled a struct".
    #[derive(Clone, Copy)]
    pub(crate) struct ChannelScoped(());

    impl ChannelScoped {
        /// Constructor 1: the query pins a single channel
        /// (`EventQuery.channel_id = Some(_)`, compiled to a
        /// `channel_id = $n` predicate). This proof covers BOTH query
        /// builders — the SELECT builder (`event::query_events_on`) and the
        /// COUNT builder (`event::count_events`) pin identically; if the
        /// two ever drift, this comment is a lie and the routed COUNT seam
        /// is unsound.
        /// Sound under conjunction: any additional clause (e.g.
        /// `channel_ids`, which alone retains channel-NULL rows) is ANDed,
        /// and `channel_id = <uuid>` never matches NULL — the pin strictly
        /// narrows and cannot be widened back out to global rows.
        pub(crate) fn from_pinned_channel(q: &crate::event::EventQuery) -> Option<Self> {
            q.channel_id.map(|_| ChannelScoped(()))
        }

        /// Constructor 2 (thread pages): the page is an inner JOIN from
        /// `thread_metadata` to `events`, and `thread_metadata.channel_id`
        /// is `UUID NOT NULL` — every writer that creates a row passes a
        /// concrete channel (`ThreadMetadataParams.channel_id: Uuid`,
        /// non-Option). Channel-bearing by construction of the join, not by
        /// query predicate.
        pub(crate) fn from_thread_metadata_join() -> Self {
            ChannelScoped(())
        }

        /// Constructor 3 (channel windows): the channel arrives as a bare
        /// `Uuid` argument and the SQL binds it unconditionally
        /// (`e.channel_id = $2` in `get_channel_window_on`); every served
        /// row is channel-bearing. No `EventQuery` exists on this path.
        pub(crate) fn from_channel_id(_channel_id: Uuid) -> Self {
            ChannelScoped(())
        }
    }
}
use route_proof::ChannelScoped;

/// The predicate one routed read must satisfy (see [`Db::route_read`]).
///
/// Discipline: no `Default`, no `Deserialize`, stays non-`pub` — any of
/// those re-opens the [`ChannelScoped`] mint.
enum RoutePredicate {
    /// Bounded staleness: the proved entry must be within the configured
    /// read budget `B` (default off). Bounds TIME — the page misses at most
    /// the freshest `B` of writes. Sound for ANY query shape, including
    /// global (channel-NULL) rows: it relies only on heartbeat commit order,
    /// not the floor guard.
    Bounded,
    /// Completeness: the proved wall must cover the page's upper bound.
    /// Bounds CONTENT — every row at/below `upper` is present, meaningful
    /// even when the cursor is hours old, where `B`-freshness says nothing.
    /// Sound ONLY on the floor guard's domain (channel-bearing rows), hence
    /// the proof token. `upper` is non-optional: the no-upper-bound
    /// post-verifying case is [`RoutePredicate::CoveredPostVerified`].
    ///
    /// Bounds INSERT-completeness only — "no missing rows", not "no extra
    /// rows". Soft deletes are `UPDATE .. SET deleted_at` commits outside
    /// the floor guard and never touch `created_at`, so a covered page can
    /// briefly serve a row the writer already excludes; deletion visibility
    /// is bounded by replication lag under `FENCE_STALENESS` (30s), not by
    /// `upper` or `B`. Do not extend the covered arm to a surface that
    /// cannot absorb extra rows (this is why the routed COUNT seam is
    /// bounded-only).
    Covered {
        upper: DateTime<Utc>,
        /// Never read — the field exists so constructing this variant
        /// requires minting the token through `route_proof`.
        #[allow(dead_code)]
        proof: ChannelScoped,
    },
    /// Forward-walking thread pages: no upper bound is derivable from the
    /// cursor; the caller post-verifies the served rows against the proved
    /// wall (full page + tail at/below the wall, else re-run on the writer).
    /// Only the thread path constructs this — a general routed caller does
    /// no post-verification and must never self-certify.
    CoveredPostVerified {
        #[allow(dead_code)]
        proof: ChannelScoped,
    },
    /// Either arm admits, covered tried first (it has no budget dependence).
    /// For general routed reads that are channel-pinned AND carry an
    /// `until` upper bound.
    BoundedOrCovered {
        upper: DateTime<Utc>,
        /// Never read — see [`RoutePredicate::Covered::proof`].
        #[allow(dead_code)]
        proof: ChannelScoped,
    },
}

impl RoutePredicate {
    /// A channel-window request: cursor pages are covered-only — for deep
    /// keyset pages only coverage answers "have all rows below the cursor
    /// replayed?" — and a head fetch is bounded. The channel id is the
    /// bare-`Uuid` proof that the window SQL pins a channel.
    fn from_channel_cursor(channel_id: Uuid, cursor: &Option<(DateTime<Utc>, Vec<u8>)>) -> Self {
        match cursor {
            Some((ts, _)) => RoutePredicate::Covered {
                upper: *ts,
                proof: ChannelScoped::from_channel_id(channel_id),
            },
            None => RoutePredicate::Bounded,
        }
    }

    /// General entry point for the routed query seams: derives the strongest
    /// sound predicate from the query shape. Never produces a covered arm
    /// without both a channel-scope proof AND a real upper bound.
    ///
    /// `routing_enabled` is whether `BUZZ_REPLICA_READ_MAX_AGE_MS` is set
    /// (non-zero). When it is NOT, this returns `Bounded` — which the zero
    /// budget then fails closed — so the new seams are genuinely dark at
    /// the deploy default even for channel-pinned queries carrying `until`.
    /// Without this gate, `BoundedOrCovered` would take the covered arm
    /// (which has no budget dependence) and route on day one with no env
    /// var set and no kill switch short of removing the replica URL
    /// (Dawn's covered-at-zero-budget catch). The pre-existing cursor
    /// paths (`Covered`/`CoveredPostVerified` from channel windows and
    /// thread pages) intentionally still route at B=0 — status quo,
    /// unchanged.
    fn for_query(q: &event::EventQuery, routing_enabled: bool) -> Self {
        if !routing_enabled {
            return RoutePredicate::Bounded;
        }
        match (ChannelScoped::from_pinned_channel(q), q.until) {
            (Some(proof), Some(upper)) => RoutePredicate::BoundedOrCovered { upper, proof },
            _ => RoutePredicate::Bounded,
        }
    }
}

/// Map the configured read budget (`BUZZ_REPLICA_READ_MAX_AGE_MS`) to the
/// runtime gate: `0` disables bounded-staleness routing; anything above the
/// fence staleness gate is clamped to it (an entry older than the staleness
/// gate never routes anyway, so a larger budget would only misrepresent the
/// config).
fn read_budget_from_ms(ms: u64) -> Option<Duration> {
    match ms {
        0 => None,
        ms => Some(Duration::from_millis(ms).min(replica_fence::FENCE_STALENESS)),
    }
}

/// Snapshot of Postgres connection pool utilisation.
#[derive(Debug, Clone, Copy)]
pub struct DbPoolStats {
    /// Total connections currently in the pool (idle + active).
    pub size: u32,
    /// Connections available for immediate reuse.
    pub idle: u32,
    /// Pool ceiling — the `max_connections` value set at construction.
    pub max: u32,
}

/// Owns the detached Postgres session holding the relay usage-metrics advisory lock.
///
/// The connection deliberately does not return to the main pool: session advisory
/// locks must remain bound to this exact physical connection, and the poller
/// pings it before each leader-only collection tick.
pub struct UsageMetricsLeader {
    connection: PgConnection,
}

impl UsageMetricsLeader {
    /// Returns whether the lock-owning session is still reachable.
    ///
    /// Bounded to 5 seconds — a blackholed connection (no RST) would otherwise
    /// stall the entire poller tick until the OS TCP timeout.
    pub async fn is_live(&mut self) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.connection.ping())
            .await
            .is_ok_and(|r| r.is_ok())
    }
}

/// Configuration for the Postgres connection pool.
#[derive(Debug, Clone)]
pub struct DbConfig {
    /// Postgres connection URL (usually sourced from `DATABASE_URL`).
    pub database_url: String,
    /// Optional read-replica connection URL (usually sourced from
    /// `READ_DATABASE_URL`, e.g. an Aurora `cluster-ro-` endpoint). `None`
    /// disables replica routing: [`Db::read`] falls back to the writer pool.
    pub read_database_url: Option<String>,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Maximum connections in the read-replica pool (env
    /// `BUZZ_DB_READ_POOL_SIZE`). `None` inherits [`Self::max_connections`].
    pub read_max_connections: Option<u32>,
    /// Minimum number of idle connections to maintain.
    pub min_connections: u32,
    /// Seconds to wait when acquiring a connection before timing out.
    pub acquire_timeout_secs: u64,
    /// Maximum connection lifetime in seconds before recycling.
    pub max_lifetime_secs: u64,
    /// Seconds a connection may sit idle before being closed.
    pub idle_timeout_secs: u64,
    /// Replica read budget `B` in milliseconds (bounded arm, env
    /// `BUZZ_REPLICA_READ_MAX_AGE_MS`). `0` disables bounded-staleness
    /// routing — the rollout default. Values above
    /// [`replica_fence::FENCE_STALENESS`] are clamped to it: an entry older
    /// than the staleness gate never routes anyway, so a larger budget
    /// would only misrepresent the config.
    pub replica_read_max_age_ms: u64,
}

impl Default for DbConfig {
    /// Sized for a single relay pod against PG max_connections=100.
    /// Staging measured 51 idle + 1 active out of 50 — most connections sat unused.
    /// At 20 main + 5 audit = 25/pod, four relay pods fit within the PG limit.
    fn default() -> Self {
        Self {
            database_url: "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string(), // sadscan:disable np.postgres.1
            read_database_url: None,
            max_connections: 20,
            read_max_connections: None,
            min_connections: 2,
            acquire_timeout_secs: 3,
            max_lifetime_secs: 1800,
            idle_timeout_secs: 600,
            replica_read_max_age_ms: 0,
        }
    }
}

impl Db {
    /// Creates a new `Db` by connecting a Postgres pool with the given config.
    ///
    /// When `config.read_database_url` is set, a second pool with the same
    /// sizing is connected to it for lag-tolerant reads (see [`Db::read`]).
    ///
    /// The writer pool arms the commit-time `created_at` floor guard
    /// (migration 0021) on every connection by setting the
    /// `buzz.created_at_floor` GUC — this is what makes the replica fence
    /// proof hold for every insert path that goes through this pool.
    pub async fn new(config: &DbConfig) -> Result<Self> {
        let pool = Self::connect_pool(config, &config.database_url).await?;
        let read_max_connections = config
            .read_max_connections
            .unwrap_or(config.max_connections);
        let read_pool = match &config.read_database_url {
            Some(url) => Some(Self::connect_read_pool(config, url, read_max_connections)?),
            None => None,
        };
        let replica_read_max_age = read_budget_from_ms(config.replica_read_max_age_ms);
        Ok(Self {
            pool,
            max_connections: config.max_connections,
            read_pool,
            read_max_connections,
            fence: std::sync::Arc::new(replica_fence::ReplicaFence::new()),
            replica_read_max_age,
            reader_aurora_identity: std::sync::Arc::new(std::sync::OnceLock::new()),
        })
    }

    /// Connect the writer pool with all session-level safety premises.
    ///
    /// SQLx stores one `after_connect` hook, so the floor guard and transaction
    /// isolation assertion must remain in this single closure. Registering a
    /// second hook replaces the first and silently disarms the floor trigger.
    async fn connect_pool(config: &DbConfig, url: &str) -> Result<PgPool> {
        let options = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs))
            .max_lifetime(Duration::from_secs(config.max_lifetime_secs))
            .idle_timeout(Duration::from_secs(config.idle_timeout_secs))
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    // `SET` cannot take bind parameters; `set_config` can.
                    sqlx::query("SELECT set_config('buzz.created_at_floor', $1, false)")
                        .bind(replica_fence::CREATED_AT_FLOOR_SECS.to_string())
                        .execute(&mut *conn)
                        .await?;
                    let isolation: String = sqlx::query_scalar("SHOW transaction_isolation")
                        .fetch_one(&mut *conn)
                        .await?;
                    if isolation != "read committed" {
                        return Err(sqlx::Error::Configuration(
                            format!(
                                "writer pool requires READ COMMITTED transaction isolation, got {isolation}"
                            )
                            .into(),
                        ));
                    }
                    Ok(())
                })
            });
        Ok(options.connect(url).await?)
    }

    /// Reader acquire timeout — deliberately far below the writer's
    /// (seconds-denominated) timeout. Failing closed to the writer must be
    /// fast: a saturated reader pool that made routed reads wait the full
    /// writer-style timeout would add dead latency during exactly the load
    /// spike the offload exists for. A miss here surfaces as
    /// `writer/reader_acquire_timeout` (see [`Db::proved_reader`] for why
    /// the reason names the mechanism rather than a diagnosis).
    const READER_ACQUIRE_TIMEOUT: Duration = Duration::from_millis(150);

    /// Connect the read-replica pool **lazily** — no connection is
    /// attempted at construction, so a reader that is down at boot cannot
    /// crash the relay (it starts all-writer with the fence closed and
    /// recovers when the replica returns).
    ///
    /// `min_connections` is pinned to 0 explicitly: sqlx's lazy pool still
    /// spawns an eager background connect task to satisfy a nonzero
    /// minimum, which would reintroduce boot-time reader dial attempts (and
    /// their log noise) that "lazy" is meant to avoid. With 0, connections
    /// are dialed only on first acquire; the ~10-minute reaper never tops
    /// the pool back up, which is fine — routed reads re-fill it on demand.
    ///
    /// No floor guard or writer-isolation assertion: replica sessions are
    /// read-only, so the commit-time trigger from migration 0021 never fires
    /// here and the write fence that depends on READ COMMITTED is never reached.
    fn connect_read_pool(config: &DbConfig, url: &str, max_connections: u32) -> Result<PgPool> {
        Ok(PgPoolOptions::new()
            .max_connections(max_connections)
            .min_connections(0)
            .acquire_timeout(Self::READER_ACQUIRE_TIMEOUT)
            .max_lifetime(Duration::from_secs(config.max_lifetime_secs))
            .idle_timeout(Duration::from_secs(config.idle_timeout_secs))
            .connect_lazy(url)?)
    }

    /// Spawn a one-shot reader reachability probe that only WARNs.
    ///
    /// With a lazy pool and `min_connections(0)`, nothing dials the replica
    /// until the first routed read — so a misconfigured `READ_DATABASE_URL`
    /// would otherwise be invisible until traffic arrives and quietly falls
    /// back to the writer. This ping is the only boot-time reader-down
    /// visibility; it must never gate startup or [`Db::spawn_fence_probe`].
    ///
    /// On success it also primes the Aurora identity capability cache
    /// ([`Db::reader_aurora_identity`]) on the connection it already holds,
    /// so the first routed read doesn't spend a second acquire (up to
    /// another [`Db::READER_ACQUIRE_TIMEOUT`]) inside
    /// [`Db::reader_aurora_capability_on`]. Prime failure is fine: the routed
    /// path re-probes on the connection it already holds, so a failed prime
    /// costs a round trip rather than a second acquire budget.
    pub fn spawn_read_pool_boot_ping(&self) {
        let Some(read_pool) = self.read_pool.clone() else {
            return;
        };
        let aurora_identity = self.reader_aurora_identity.clone();
        tokio::spawn(async move {
            match observability::acquire(&read_pool, observability::PoolRole::Reader).await {
                Ok(mut conn) => {
                    tracing::info!("read replica reachable at boot");
                    match replica_fence::reader_supports_aurora_identity(&mut conn).await {
                        Ok(supported) => {
                            let _ = aurora_identity.set(supported);
                        }
                        Err(e) => tracing::debug!(
                            error = %e,
                            "aurora identity boot prime failed; first routed read will probe"
                        ),
                    }
                }
                Err(e) => tracing::warn!(
                    "read replica unreachable at boot; serving all-writer until it recovers: {e}"
                ),
            }
        });
    }

    /// Creates a `Db` from an existing `PgPool` (useful in tests).
    pub fn from_pool(pool: PgPool) -> Self {
        Self {
            max_connections: pool.options().get_max_connections(),
            read_max_connections: pool.options().get_max_connections(),
            pool,
            read_pool: None,
            fence: std::sync::Arc::new(replica_fence::ReplicaFence::new()),
            replica_read_max_age: None,
            reader_aurora_identity: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Creates a `Db` from distinct writer and read pools (useful in tests,
    /// where a second database stands in for a lagged replica).
    ///
    /// The fence starts closed; tests that want cursor pages served by the
    /// fake replica must open it via
    /// [`replica_fence::ReplicaFence::force_open_for_tests`] (see
    /// [`Db::fence`]).
    pub fn from_pools(pool: PgPool, read_pool: PgPool) -> Self {
        Self {
            max_connections: pool.options().get_max_connections(),
            read_max_connections: read_pool.options().get_max_connections(),
            pool,
            read_pool: Some(read_pool),
            fence: std::sync::Arc::new(replica_fence::ReplicaFence::new()),
            replica_read_max_age: None,
            reader_aurora_identity: std::sync::Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Test hook: set the head-fetch routing budget (Predicate A), which
    /// [`Db::from_pools`] leaves disabled.
    pub fn set_replica_read_max_age_for_tests(&mut self, budget: Option<Duration>) {
        self.replica_read_max_age = budget;
    }

    /// The freshness fence gating replica routing (see [`replica_fence`]).
    pub fn fence(&self) -> &std::sync::Arc<replica_fence::ReplicaFence> {
        &self.fence
    }

    /// Verify the floor guard end-to-end, then spawn the background fence
    /// probe. Returns `Ok(false)` when no replica is configured.
    ///
    /// Ordering matters (Perci, PR #2084 review): this must run **after**
    /// the migration decision. On a relay with `BUZZ_AUTO_MIGRATE` off, the
    /// writer pool arms the GUC regardless, but if migration 0021 has not
    /// been applied there is no trigger enforcing it — and a heartbeat probe
    /// would open the fence over an unenforced floor. So the probe is gated
    /// on an unconditional two-part verification against the live schema:
    /// catalog shape ([`replica_fence::verify_floor_guard_catalog`]) and
    /// observed semantics through this exact pool
    /// ([`replica_fence::verify_floor_guard_behavior`]).
    ///
    /// On any verification failure the probe is never spawned and the fence
    /// stays closed: every cursor page routes to the writer. The relay keeps
    /// serving — degraded capacity, never holes.
    pub async fn spawn_fence_probe(&self) -> Result<bool> {
        if self.read_pool.is_none() {
            return Ok(false);
        }
        replica_fence::verify_floor_guard_catalog(&self.pool).await?;
        replica_fence::verify_floor_guard_behavior(&self.pool).await?;
        tokio::spawn(replica_fence::run_probe(
            self.pool.clone(),
            std::sync::Arc::clone(&self.fence),
        ));
        Ok(true)
    }

    /// The pool for lag-tolerant reads: the read replica when configured,
    /// otherwise the writer pool.
    ///
    /// Removed as a public escape hatch (Dawn, review of 1b0aa0dfa): the
    /// raw replica pool carries **no fence proof**, which is exactly the
    /// bug class the routed-read machinery exists to eliminate. All replica
    /// reads must go through [`Db::route_read`]-backed entry points; this
    /// remains only for the fence's own plumbing tests.
    #[cfg(test)]
    fn read(&self) -> &PgPool {
        self.read_pool.as_ref().unwrap_or(&self.pool)
    }

    /// Whether a distinct read-replica pool is configured.
    pub fn has_read_pool(&self) -> bool {
        self.read_pool.is_some()
    }

    /// Open a reader request transaction and complete the connection-local
    /// half of the fence proof: `BEGIN ISOLATION LEVEL REPEATABLE READ, READ
    /// ONLY`, then observe the heartbeat token/epoch as the transaction's
    /// **first statement** — anchoring the snapshot every follow-up
    /// statement (page, participants, aux closure) sees to exactly the
    /// snapshot the proof was taken against — and resolve it against the
    /// retained ring. Returns the open transaction together with the
    /// strongest [`replica_fence::TokenEntry`] its observation supports, or
    /// the fail-closed reason for route metrics.
    ///
    /// `REPEATABLE READ` is the strongest isolation a hot standby supports
    /// (`SERIALIZABLE` is writer-only); `READ ONLY` documents intent and
    /// rejects accidental writes. Everything but `Ok` fails closed — begin
    /// failure, missing heartbeat row (migration not yet replayed there),
    /// observation error, epoch mismatch, or a token below every retained
    /// entry all route the request to the writer.
    async fn proved_reader(
        &self,
        read_pool: &PgPool,
    ) -> std::result::Result<
        (
            sqlx::Transaction<'static, sqlx::Postgres>,
            replica_fence::TokenEntry,
        ),
        &'static str,
    > {
        // One checkout per routed read. The Aurora capability probe and the
        // read-only transaction share a single `acquire()` so the request path
        // spends exactly one READER_ACQUIRE_TIMEOUT budget. Probing through
        // `read_pool` separately would spend a second budget whenever the
        // capability is uncached — i.e. after a failed boot ping, which is
        // precisely the reader-unavailable case the bound must hold for.
        let conn = match observability::acquire(read_pool, observability::PoolRole::Reader).await {
            Ok(conn) => conn,
            Err(sqlx::Error::PoolTimedOut) => {
                tracing::warn!("reader pool acquire timed out; routing to writer");
                return Err("reader_acquire_timeout");
            }
            Err(e) => {
                tracing::warn!(error = %e, "reader connection acquire failed; routing to writer");
                return Err("reader_validation_error");
            }
        };
        let mut conn = conn;
        let aurora = self.reader_aurora_capability_on(&mut conn).await;
        let mut tx = match sqlx::Transaction::begin(
            conn,
            Some(sqlx::SqlStr::from_static(
                "BEGIN ISOLATION LEVEL REPEATABLE READ, READ ONLY",
            )),
        )
        .await
        {
            Ok(tx) => tx,
            // The acquire miss gets its own reason code: the reader pool's
            // short acquire timeout (READER_ACQUIRE_TIMEOUT) makes this the
            // fast fail-closed path under load, and
            // `buzz_db_route_decision{decision="writer",reason="reader_acquire_timeout"}`
            // is the operator's alert signal for a struggling reader pool.
            //
            // The reason deliberately names the mechanism, not a diagnosis:
            // `PoolTimedOut` proves only that no connection was handed out
            // within the 150ms budget. That budget includes cold connect
            // (TCP+TLS+auth), and sqlx's `size` counts in-flight dials, so
            // this fires for slow connection establishment as well as for
            // established-connection contention — and neither `size == 0`
            // nor `size >= max` recovers the missing causal bit (in-flight
            // dials hold a size slot, and a cold burst can push
            // `active = size - idle` toward max with zero busy connections).
            // Runbook: correlate with `buzz_db_read_pool_active` / `_max`
            // and reader connection health/latency; high active suggests
            // contention, but this metric alone does not distinguish
            // contention from slow connects. Note the gauge is a coarse
            // sample (BUZZ_POOL_METRICS_INTERVAL_SECS, default 10s) while
            // the event it explains lasts ~150ms — a short burst may fall
            // between samples entirely, so absence of elevated active is
            // NOT evidence of a cold connect.
            Err(sqlx::Error::PoolTimedOut) => {
                tracing::warn!("reader pool acquire timed out; routing to writer");
                return Err("reader_acquire_timeout");
            }
            Err(e) => {
                tracing::warn!(error = %e, "reader transaction begin failed; routing to writer");
                return Err("reader_validation_error");
            }
        };
        let obs = match replica_fence::observe_heartbeat(&mut tx, aurora).await {
            Ok(Some(observation)) => observation,
            Ok(None) => return Err("reader_validation_error"),
            Err(e) => {
                tracing::warn!(error = %e, "heartbeat observation failed; routing to writer");
                return Err("reader_validation_error");
            }
        };
        match self.fence.resolve(obs.token, obs.epoch) {
            replica_fence::ResolveOutcome::Proved(entry) => {
                tracing::debug!(
                    token = obs.token,
                    proved_token = entry.token,
                    backend = %obs.backend,
                    "reader snapshot proved fence coverage"
                );
                Ok((tx, entry))
            }
            replica_fence::ResolveOutcome::EpochMismatch => Err("reader_validation_error"),
            replica_fence::ResolveOutcome::TokenBehind => Err("reader_token_behind"),
        }
    }

    /// Whether the reader endpoint supports the Aurora PostgreSQL identity
    /// function ([`replica_fence::AURORA_IDENTITY_FN`]), probed
    /// once per process and cached (see [`Db::reader_aurora_identity`]).
    /// The probe runs on a plain autocommit checkout — never inside the
    /// request transaction, where an undefined-function error would abort
    /// it. Probe failure (acquire or transient) degrades to the plain
    /// identity tuple for THIS request without caching, so a later request
    /// retries; identity is evidence, never a routing gate.
    /// Aurora capability on a connection the caller already holds, so the
    /// routed path never spends a second acquire budget.
    async fn reader_aurora_capability_on(
        &self,
        conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    ) -> bool {
        if let Some(cached) = self.reader_aurora_identity.get() {
            return *cached;
        }
        match replica_fence::reader_supports_aurora_identity(conn).await {
            Ok(supported) => *self.reader_aurora_identity.get_or_init(|| supported),
            Err(e) => {
                tracing::debug!(error = %e, "aurora identity probe failed; will retry");
                false
            }
        }
    }

    /// Record one route decision (Rev 2 observability): which path, where it
    /// went, and why.
    fn record_route(path: &'static str, decision: &'static str, reason: &'static str) {
        metrics::counter!(
            "buzz_db_route_decision",
            "path" => path,
            "decision" => decision,
            "reason" => reason,
        )
        .increment(1);
    }

    /// Run pending database migrations.
    #[datastore_span(name = "migrate", system = "postgresql")]
    pub async fn migrate(&self) -> Result<()> {
        migration::run_migrations(&self.pool).await
    }

    /// Returns `true` if the database is reachable (used by readiness probes).
    pub async fn ping(&self) -> bool {
        sqlx::query("SELECT 1").execute(&self.pool).await.is_ok()
    }

    /// Validate the minimum deletion fence catalog required by serving paths.
    pub async fn validate_deletion_serving_catalog(&self) -> Result<()> {
        self.deletion_store().validate_serving_catalog().await
    }

    /// Validate the exact live community-deletion tenant catalog for destruction.
    pub async fn validate_deletion_catalog(&self) -> Result<()> {
        self.deletion_store().validate_catalog().await
    }

    /// Returns pool utilisation stats for metrics emission.
    ///
    /// `size`  — total connections (idle + active)
    /// `idle`  — connections available for immediate reuse
    /// `max`   — pool ceiling set at construction
    pub fn pool_stats(&self) -> DbPoolStats {
        DbPoolStats {
            size: self.pool.size(),
            idle: self.pool.num_idle() as u32,
            max: self.max_connections,
        }
    }

    /// Pool utilisation stats for the read-replica pool, when configured.
    ///
    /// `max` is the **reader's** ceiling ([`Db::read_max_connections`]), not
    /// the writer's: `buzz_db_read_pool_active / buzz_db_read_pool_max` is
    /// the operator's utilisation signal for tuning `BUZZ_DB_READ_POOL_SIZE`,
    /// and deriving it from the writer's max would misreport saturation by
    /// exactly the ratio of the two pool sizes — in the direction that hides
    /// the problem.
    pub fn read_pool_stats(&self) -> Option<DbPoolStats> {
        self.read_pool.as_ref().map(|p| DbPoolStats {
            size: p.size(),
            idle: p.num_idle() as u32,
            max: self.read_max_connections,
        })
    }

    /// Try to acquire the detached session advisory lock for relay usage metrics.
    ///
    /// The returned guard owns the exact connection that acquired the lock. It is
    /// detached from the shared pool so a stable leader neither returns a locked
    /// session to other callers nor permanently consumes a pool slot. Dropping the
    /// guard closes the connection and releases the session-scoped lock.
    #[datastore_span(name = "try_lock_usage_metrics", system = "postgresql")]
    pub async fn try_lock_usage_metrics(
        &self,
        lock_key: i64,
    ) -> Result<Option<UsageMetricsLeader>> {
        let mut connection =
            observability::acquire(&self.pool, observability::PoolRole::Writer).await?;
        let acquired = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
            .bind(lock_key)
            .fetch_one(&mut *connection)
            .await?;
        if acquired {
            Ok(Some(UsageMetricsLeader {
                connection: connection.detach(),
            }))
        } else {
            Ok(None)
        }
    }

    /// List reports for the deployment-global read-only admin plane.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "admin_list_reports", system = "postgresql")]
    pub async fn admin_list_reports(
        &self,
        community_id: Option<Uuid>,
        status: Option<&str>,
        report_type: Option<&str>,
        target_kind: Option<&str>,
        after: Option<DateTime<Utc>>,
        before: Option<DateTime<Utc>>,
        cursor: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> Result<Vec<admin_moderation::AdminReport>> {
        admin_moderation::list_reports(
            &self.pool,
            community_id,
            status,
            report_type,
            target_kind,
            after,
            before,
            cursor,
            limit,
        )
        .await
    }

    /// Fetch one report for the deployment-global read-only admin plane.
    #[datastore_span(name = "admin_get_report", system = "postgresql")]
    pub async fn admin_get_report(
        &self,
        id: Uuid,
    ) -> Result<Option<admin_moderation::AdminReportDetail>> {
        admin_moderation::get_report(&self.pool, id).await
    }

    /// List feedback for the deployment-global read-only admin plane.
    #[datastore_span(name = "admin_list_feedback", system = "postgresql")]
    pub async fn admin_list_feedback(
        &self,
        limit: i64,
    ) -> Result<Vec<admin_moderation::AdminFeedback>> {
        admin_moderation::list_feedback(&self.pool, limit).await
    }

    /// Fetch one feedback submission for the deployment-global admin plane.
    #[datastore_span(name = "admin_get_feedback", system = "postgresql")]
    pub async fn admin_get_feedback(
        &self,
        id: Uuid,
    ) -> Result<Option<admin_moderation::AdminFeedback>> {
        admin_moderation::get_feedback(&self.pool, id).await
    }

    /// Return total number of communities on this relay.
    #[datastore_span(name = "usage_community_count", system = "postgresql")]
    pub async fn usage_community_count(&self) -> Result<i64> {
        usage::community_count(&self.pool).await
    }

    /// Return per-community user counts split by human/agent.
    #[datastore_span(name = "usage_user_counts", system = "postgresql")]
    pub async fn usage_user_counts(&self) -> Result<Vec<usage::CommunityUserCounts>> {
        usage::user_counts(&self.pool).await
    }

    /// Return per-community channel counts by type.
    #[datastore_span(name = "usage_channel_counts", system = "postgresql")]
    pub async fn usage_channel_counts(&self) -> Result<Vec<usage::CommunityChannelCount>> {
        usage::channel_counts(&self.pool).await
    }

    /// Return per-community kind=9 message counts.
    #[datastore_span(name = "usage_message_counts", system = "postgresql")]
    pub async fn usage_message_counts(&self) -> Result<Vec<usage::CommunityMessageCount>> {
        usage::message_counts(&self.pool).await
    }

    /// Return per-community relay-member counts by role.
    #[datastore_span(name = "usage_relay_member_counts", system = "postgresql")]
    pub async fn usage_relay_member_counts(&self) -> Result<Vec<usage::CommunityMemberCount>> {
        usage::relay_member_counts(&self.pool).await
    }

    /// Return per-community workflow counts by status.
    #[datastore_span(name = "usage_workflow_counts", system = "postgresql")]
    pub async fn usage_workflow_counts(&self) -> Result<Vec<usage::CommunityWorkflowCount>> {
        usage::workflow_counts(&self.pool).await
    }

    /// Return per-community git-repo counts.
    #[datastore_span(name = "usage_git_repo_counts", system = "postgresql")]
    pub async fn usage_git_repo_counts(&self) -> Result<Vec<usage::CommunityGitRepoCount>> {
        usage::git_repo_counts(&self.pool).await
    }

    /// Return per-community distinct active-user counts for a given SQL interval.
    ///
    /// `interval_sql` must be a trusted literal such as `"1 day"` or `"7 days"`.
    #[datastore_span(name = "usage_active_user_counts", system = "postgresql")]
    pub async fn usage_active_user_counts(
        &self,
        interval_sql: &'static str,
    ) -> Result<Vec<usage::CommunityActiveUsers>> {
        usage::active_user_counts(&self.pool, interval_sql).await
    }

    /// Return per-community active-channel counts for a given SQL interval.
    #[datastore_span(name = "usage_active_channel_counts", system = "postgresql")]
    pub async fn usage_active_channel_counts(
        &self,
        interval_sql: &'static str,
    ) -> Result<Vec<usage::CommunityActiveChannels>> {
        usage::active_channel_counts(&self.pool, interval_sql).await
    }

    /// Return all community id → host mappings.
    #[datastore_span(name = "usage_community_hosts", system = "postgresql")]
    pub async fn usage_community_hosts(&self) -> Result<Vec<usage::CommunityHost>> {
        usage::community_hosts(&self.pool).await
    }

    /// Return the shared durable whole-community deletion adapter.
    pub fn deletion_store(&self) -> deletion::DeletionStore {
        deletion::DeletionStore::new(self.pool.clone())
    }

    /// Begin a database transaction for atomic multi-statement operations.
    ///
    /// Returns a `'static` transaction because `PgPool` is `Arc`-backed internally.
    /// The transaction holds an owned pool handle, not a borrow.
    pub async fn begin_transaction(&self) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        let connection =
            observability::acquire(&self.pool, observability::PoolRole::Writer).await?;
        sqlx::Transaction::begin(connection, None)
            .await
            .map_err(Into::into)
    }

    /// Insert an event while holding and validating an admitted serving-write
    /// lease under the community ordering lock through commit.
    ///
    /// External side effects use a durable lease rather than one long-lived DB
    /// transaction. Their final database mutation presents that exact lease so
    /// it may finish during quiescing without admitting any new serving work.
    pub async fn insert_event_with_serving_write_guard(
        &self,
        lease: &deletion::ServingWriteLease,
        event: &nostr::Event,
        channel_id: Option<Uuid>,
    ) -> Result<(StoredEvent, bool)> {
        let community_id = lease.community_id;
        let kind_u16 = event.kind.as_u16();
        let kind_u32 = u32::from(kind_u16);
        if kind_u32 == buzz_core::kind::KIND_AUTH {
            return Err(DbError::AuthEventRejected);
        }
        if buzz_core::kind::is_ephemeral(kind_u32) {
            return Err(DbError::EphemeralEventRejected(kind_u16));
        }

        let mut tx = self.pool.begin().await?;
        self.deletion_store()
            .guard_transaction_with_serving_lease(&mut tx, lease)
            .await?;
        let result = event::insert_event_with_thread_metadata_tx(
            &mut tx,
            community_id,
            event,
            channel_id,
            None,
        )
        .await?;
        tx.commit().await?;
        if result.1 {
            if let Err(e) = insert_mentions(&self.pool, community_id, event, channel_id).await {
                tracing::warn!(event_id = %event.id, "Failed to insert mentions: {e}");
            }
        }
        Ok(result)
    }

    /// Shared route decision for one read: evaluate the predicate against a
    /// proved reader session and record the decision. Fail closed to the
    /// writer everywhere.
    async fn route_read(&self, path: &'static str, predicate: RoutePredicate) -> RouteDecision {
        let Some(read_pool) = &self.read_pool else {
            Self::record_route(path, "writer", "disabled");
            return RouteDecision::Writer;
        };
        // Cheap prechecks on the shared ring before spending a reader
        // checkout; the connection-local observation still has to prove it.
        let Some(newest) = self.fence.newest() else {
            Self::record_route(path, "writer", "uninitialized");
            return RouteDecision::Writer;
        };
        // Precheck helpers against the newest shared entry: if the newest
        // cannot satisfy an arm, no proved (older-or-equal) entry can.
        let bounded_precheck =
            |budget: &Option<Duration>| -> std::result::Result<(), &'static str> {
                match budget {
                    Some(budget) if newest.committed_at.elapsed() <= *budget => Ok(()),
                    Some(_) => Err("stale"),
                    None => Err("disabled"),
                }
            };
        let covered_precheck = |upper: &DateTime<Utc>| -> std::result::Result<(), &'static str> {
            if *upper <= newest.fence_wall {
                Ok(())
            } else {
                Err("stale")
            }
        };
        let precheck = match &predicate {
            RoutePredicate::Bounded => bounded_precheck(&self.replica_read_max_age),
            RoutePredicate::Covered { upper, .. } => covered_precheck(upper),
            // No upper bound: the caller post-verifies served rows.
            RoutePredicate::CoveredPostVerified { .. } => Ok(()),
            // Covered first (no budget dependence), else bounded.
            RoutePredicate::BoundedOrCovered { upper, .. } => {
                covered_precheck(upper).or_else(|_| bounded_precheck(&self.replica_read_max_age))
            }
        };
        if let Err(reason) = precheck {
            Self::record_route(path, "writer", reason);
            return RouteDecision::Writer;
        }
        match self.proved_reader(read_pool).await {
            Ok((tx, entry)) => {
                // Re-evaluate against the entry the session actually proved
                // (it may be older than the shared newest).
                let bounded_holds = || {
                    self.replica_read_max_age
                        .is_some_and(|budget| entry.committed_at.elapsed() <= budget)
                };
                let verdict: Option<&'static str> = match &predicate {
                    RoutePredicate::Bounded => bounded_holds().then_some("fresh"),
                    RoutePredicate::Covered { upper, .. } => {
                        (*upper <= entry.fence_wall).then_some("covered")
                    }
                    // No upper bound: the caller post-verifies the served
                    // rows against the proved wall.
                    RoutePredicate::CoveredPostVerified { .. } => Some("covered"),
                    RoutePredicate::BoundedOrCovered { upper, .. } => {
                        if *upper <= entry.fence_wall {
                            Some("covered")
                        } else {
                            bounded_holds().then_some("fresh")
                        }
                    }
                };
                match verdict {
                    Some(reason) => RouteDecision::Replica(tx, entry, reason),
                    None => {
                        // The session proves an older entry than the
                        // predicate needs (replication lag) — fail closed.
                        Self::record_route(path, "writer", "stale");
                        RouteDecision::Writer
                    }
                }
            }
            Err(reason) => {
                Self::record_route(path, "writer", reason);
                RouteDecision::Writer
            }
        }
    }

    /// Ensures monthly partitions exist for the next N months.
    #[datastore_span(name = "ensure_future_partitions", system = "postgresql")]
    pub async fn ensure_future_partitions(&self, months_ahead: u32) -> Result<()> {
        partition::ensure_future_partitions(&self.pool, months_ahead).await
    }

    /// Backfill `d_tag` for existing NIP-33 events (kind 30000–39999) that have `d_tag IS NULL`.
    ///
    /// Idempotent — safe to call on every startup. No-ops when all rows are already populated.
    /// Runs a single UPDATE touching only NIP-33 rows with NULL d_tag.
    #[datastore_span(name = "backfill_d_tags", system = "postgresql")]
    pub async fn backfill_d_tags(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE events \
             SET d_tag = COALESCE( \
                 (SELECT elem->>1 FROM jsonb_array_elements(tags) AS elem \
                  WHERE elem->>0 = 'd' LIMIT 1), \
                 '' \
             ) \
             WHERE kind BETWEEN 30000 AND 39999 AND d_tag IS NULL \
               AND community_write_allowed(community_id)",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    // ── relay_operators (deployment-global principal roster) ──────────────────

    /// Fetch one relay operator/moderator row by pubkey (32-byte binary).
    pub async fn get_relay_operator(
        &self,
        pubkey: &[u8],
    ) -> Result<Option<relay_operators::RelayOperatorRecord>> {
        relay_operators::get(&self.pool, pubkey).await
    }

    /// List all relay operator/moderator rows ordered by creation time.
    pub async fn list_relay_operators(&self) -> Result<Vec<relay_operators::RelayOperatorRecord>> {
        relay_operators::list(&self.pool).await
    }

    /// Insert or update a relay operator/moderator row (upsert by pubkey).
    ///
    /// `config_operator_exists` is the caller's request-time snapshot of
    /// whether a config-backed operator is effective; a demotion that would
    /// leave no effective operator is rejected with [`DbError::LastOperator`].
    pub async fn upsert_relay_operator(
        &self,
        pubkey: &[u8],
        role: &str,
        added_by: &[u8],
        config_operator_exists: bool,
    ) -> Result<()> {
        relay_operators::upsert(&self.pool, pubkey, role, added_by, config_operator_exists).await
    }

    /// Remove a relay operator/moderator row. Returns `true` if deleted.
    /// Records the revocation in the append-only audit trail; `actor` is the
    /// authenticated operator performing the removal. `config_operator_exists`
    /// is the caller's request-time snapshot of whether a config-backed
    /// operator is effective; deleting the sole effective operator is rejected
    /// with [`DbError::LastOperator`].
    pub async fn remove_relay_operator(
        &self,
        pubkey: &[u8],
        actor: &[u8],
        config_operator_exists: bool,
    ) -> Result<bool> {
        relay_operators::remove(&self.pool, pubkey, actor, config_operator_exists).await
    }

    // ── relay_admin_actions (HTTP enforcement state machine) ──────────────────

    /// Atomic decision-only report closure: CAS open→terminal + audit row in one transaction.
    #[allow(clippy::too_many_arguments)]
    pub async fn resolve_report_decision_atomic(
        &self,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        terminal_status: &str,
        audit_action: &str,
        actor_pubkey: &[u8],
        actor_authority: &str,
        target_pubkey: Option<&[u8]>,
        target_event_id: Option<&[u8]>,
        channel_id: Option<uuid::Uuid>,
        reason: Option<&str>,
    ) -> Result<bool> {
        relay_admin_actions::resolve_report_decision_atomic(
            &self.pool,
            community_id,
            report_id,
            terminal_status,
            audit_action,
            actor_pubkey,
            actor_authority,
            target_pubkey,
            target_event_id,
            channel_id,
            reason,
        )
        .await
    }

    /// Attempt to claim a report for HTTP enforcement (CAS open → processing).
    #[allow(clippy::too_many_arguments)]
    pub async fn claim_report_for_enforcement(
        &self,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        request_id: uuid::Uuid,
        actor_pubkey: &[u8],
        actor_role: &str,
        action: &str,
        reason: Option<&str>,
        timeout_until: Option<chrono::DateTime<chrono::Utc>>,
        audit_action: &str,
        actor_authority: &str,
        target_pubkey: Option<&[u8]>,
        target_event_id: Option<&[u8]>,
        channel_id: Option<uuid::Uuid>,
    ) -> Result<relay_admin_actions::ClaimResult> {
        relay_admin_actions::claim_report(
            &self.pool,
            community_id,
            report_id,
            request_id,
            actor_pubkey,
            actor_role,
            action,
            reason,
            timeout_until,
            audit_action,
            actor_authority,
            target_pubkey,
            target_event_id,
            channel_id,
        )
        .await
    }

    /// Advance an action from 'pending' to 'enforcing'.
    pub async fn begin_enforcing_action(&self, action_id: uuid::Uuid) -> Result<bool> {
        relay_admin_actions::begin_enforcing(&self.pool, action_id).await
    }

    /// Commit the core mutation step (advance step_marker to 'mutation_committed').
    pub async fn commit_action_mutation_step(&self, action_id: uuid::Uuid) -> Result<bool> {
        relay_admin_actions::commit_mutation_step(&self.pool, action_id).await
    }

    /// Finalize enforcement: action → succeeded, report → terminal status,
    /// and enqueue outbox delivery rows atomically.
    #[allow(clippy::too_many_arguments)]
    pub async fn finalize_action_success(
        &self,
        action_id: uuid::Uuid,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        terminal_status: &str,
        actor_pubkey: &[u8],
        action_name: &str,
        target_pubkey: Option<&[u8]>,
        target_event_id: Option<&[u8]>,
        channel_id: Option<uuid::Uuid>,
        reason: Option<&str>,
        timeout_until: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<bool> {
        relay_admin_actions::finalize_success(
            &self.pool,
            action_id,
            community_id,
            report_id,
            terminal_status,
            actor_pubkey,
            action_name,
            target_pubkey,
            target_event_id,
            channel_id,
            reason,
            timeout_until,
        )
        .await
    }

    /// Atomically execute a ban mutation and commit the step marker.
    /// Returns `true` if this driver committed the marker, `false` if already marked or lease lost.
    pub async fn execute_ban_with_marker(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        community_id: CommunityId,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
        reason: Option<&str>,
    ) -> Result<bool> {
        relay_admin_actions::execute_ban_with_marker(
            &self.pool,
            action_id,
            lease_token,
            community_id,
            target_pubkey,
            actor_pubkey,
            reason,
        )
        .await
    }

    /// Atomically execute a timeout mutation and commit the step marker.
    /// Returns `true` if this driver committed the marker, `false` if already marked or lease lost.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_timeout_with_marker(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        community_id: CommunityId,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
        until: chrono::DateTime<chrono::Utc>,
        reason: Option<&str>,
    ) -> Result<bool> {
        relay_admin_actions::execute_timeout_with_marker(
            &self.pool,
            action_id,
            lease_token,
            community_id,
            target_pubkey,
            actor_pubkey,
            until,
            reason,
        )
        .await
    }

    /// Atomically execute a kick mutation and commit the step marker.
    /// Returns `Removed` (member was present), `AlreadyGone` (absent before this action),
    /// or `AlreadyMarked` (marker already committed by another driver or lease lost).
    pub async fn execute_kick_with_marker(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        community_id: CommunityId,
        channel_id: uuid::Uuid,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<relay_admin_actions::KickWithMarkerResult> {
        relay_admin_actions::execute_kick_with_marker(
            &self.pool,
            action_id,
            lease_token,
            community_id,
            channel_id,
            target_pubkey,
            actor_pubkey,
        )
        .await
    }

    /// Atomically execute a soft-delete mutation and commit the step marker.
    /// Returns `true` if this driver committed the marker, `false` if already marked or lease lost.
    pub async fn execute_delete_with_marker(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        community_id: CommunityId,
        target_event_id: &[u8],
        parent_event_id: Option<&[u8]>,
        root_event_id: Option<&[u8]>,
    ) -> Result<bool> {
        relay_admin_actions::execute_delete_with_marker(
            &self.pool,
            action_id,
            lease_token,
            community_id,
            target_event_id,
            parent_event_id,
            root_event_id,
        )
        .await
    }

    /// Acquire the action mutation lease (prevents concurrent double-mutation).
    pub async fn acquire_admin_action_lease(
        &self,
        action_id: uuid::Uuid,
        lease_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<relay_admin_actions::LeaseResult> {
        relay_admin_actions::acquire_action_lease(&self.pool, action_id, lease_until).await
    }

    /// Release the action mutation lease. No-op if caller no longer holds the token.
    pub async fn release_admin_action_lease(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
    ) -> Result<()> {
        relay_admin_actions::release_action_lease(&self.pool, action_id, lease_token).await
    }

    /// Claim a batch of stranded `relay_admin_actions` for the action recovery worker.
    pub async fn claim_stranded_admin_action_batch(
        &self,
        worker_id: &str,
        lease_until: chrono::DateTime<chrono::Utc>,
        batch_size: i64,
    ) -> Result<Vec<relay_admin_actions::StrandedActionClaim>> {
        relay_admin_actions::claim_stranded_action_batch(
            &self.pool,
            worker_id,
            lease_until,
            batch_size,
        )
        .await
    }

    /// Record a pre-mutation enforcement failure (keeps report in 'processing').
    pub async fn record_action_failure(
        &self,
        action_id: uuid::Uuid,
        lease_token: uuid::Uuid,
        error: &str,
    ) -> Result<bool> {
        relay_admin_actions::record_failure(&self.pool, action_id, lease_token, error).await
    }

    /// Cancel a pre-mutation failed action (returns report to 'open'),
    /// attributing the cancel to `cancelled_by`.
    pub async fn cancel_admin_action(
        &self,
        action_id: uuid::Uuid,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        cancelled_by: &[u8],
    ) -> Result<bool> {
        relay_admin_actions::cancel_action(
            &self.pool,
            action_id,
            community_id,
            report_id,
            cancelled_by,
        )
        .await
    }

    /// Reopen a terminal report (resolved|dismissed|escalated → open) with a
    /// durable `reopen` audit row, keyed idempotent on `request_id`.
    pub async fn reopen_report(
        &self,
        community_id: CommunityId,
        report_id: uuid::Uuid,
        request_id: uuid::Uuid,
        actor_pubkey: &[u8],
        actor_role: &str,
        reason: Option<&str>,
    ) -> Result<relay_admin_actions::ReopenResult> {
        relay_admin_actions::reopen_report(
            &self.pool,
            community_id,
            report_id,
            request_id,
            actor_pubkey,
            actor_role,
            reason,
        )
        .await
    }

    /// Fetch an action record by ID.
    pub async fn get_admin_action(
        &self,
        action_id: uuid::Uuid,
    ) -> Result<Option<relay_admin_actions::AdminActionRecord>> {
        relay_admin_actions::get_action(&self.pool, action_id).await
    }

    /// Enqueue an outbox artifact/notice delivery command.
    pub async fn enqueue_admin_outbox(
        &self,
        action_id: uuid::Uuid,
        task_type: &str,
        payload: serde_json::Value,
        dedup_key: &str,
    ) -> Result<()> {
        relay_admin_actions::enqueue_outbox(&self.pool, action_id, task_type, payload, dedup_key)
            .await
    }

    /// Mark an outbox record as delivered, fenced by the claim token.
    /// Returns `true` if updated, `false` if ownership was already lost.
    pub async fn mark_admin_outbox_delivered(
        &self,
        outbox_id: uuid::Uuid,
        claim_token: uuid::Uuid,
    ) -> Result<bool> {
        relay_admin_actions::mark_outbox_delivered(&self.pool, outbox_id, claim_token).await
    }

    /// Mark an outbox record as failed, fenced by the claim token.
    /// Returns `true` if updated, `false` if ownership was already lost.
    pub async fn fail_admin_outbox_row(
        &self,
        outbox_id: uuid::Uuid,
        claim_token: uuid::Uuid,
        error: &str,
    ) -> Result<bool> {
        relay_admin_actions::fail_outbox_row(&self.pool, outbox_id, claim_token, error).await
    }

    /// Claim a batch of pending outbox rows for the given worker pod.
    pub async fn claim_pending_admin_outbox_batch(
        &self,
        worker_id: &str,
        lease_until: chrono::DateTime<chrono::Utc>,
        batch_size: i64,
    ) -> Result<Vec<relay_admin_actions::OutboxRecord>> {
        relay_admin_actions::claim_pending_outbox_batch(
            &self.pool,
            worker_id,
            lease_until,
            batch_size,
        )
        .await
    }

    /// List pending outbox records for an action.
    pub async fn list_pending_admin_outbox(
        &self,
        action_id: uuid::Uuid,
    ) -> Result<Vec<relay_admin_actions::OutboxRecord>> {
        relay_admin_actions::list_pending_outbox(&self.pool, action_id).await
    }

    /// Deployment-authority kick: remove a member without requiring tenant owner/admin actor.
    pub async fn deploy_kick_member(
        &self,
        community_id: CommunityId,
        channel_id: uuid::Uuid,
        target_pubkey: &[u8],
        actor_pubkey: &[u8],
    ) -> Result<relay_admin_actions::KickResult> {
        relay_admin_actions::deploy_kick_member(
            &self.pool,
            community_id,
            channel_id,
            target_pubkey,
            actor_pubkey,
        )
        .await
    }

    /// Update product_feedback status (operator-managed lifecycle).
    pub async fn update_feedback_status(&self, id: uuid::Uuid, status: &str) -> Result<bool> {
        relay_admin_actions::update_feedback_status(&self.pool, id, status).await
    }

    /// Sidecar an accepted product-feedback event, idempotent by event id.
    #[datastore_span(name = "insert_product_feedback", system = "postgresql")]
    pub async fn insert_product_feedback(
        &self,
        community: CommunityId,
        feedback: product_feedback::NewProductFeedback<'_>,
    ) -> Result<Uuid> {
        product_feedback::insert(&self.pool, community, feedback).await
    }

    /// List product feedback across the deployment, newest first.
    #[datastore_span(name = "list_product_feedback", system = "postgresql")]
    pub async fn list_product_feedback(
        &self,
        limit: i64,
    ) -> Result<Vec<product_feedback::ProductFeedbackRecord>> {
        product_feedback::list(&self.pool, limit).await
    }

    /// Insert a tenant-scoped NIP-56 report row, idempotent by report event id.
    #[datastore_span(name = "insert_moderation_report", system = "postgresql")]
    pub async fn insert_moderation_report(
        &self,
        community: CommunityId,
        report: moderation::NewReport<'_>,
    ) -> Result<Uuid> {
        moderation::insert_report(&self.pool, community, report).await
    }

    /// List moderation reports for a community, newest first.
    #[datastore_span(name = "list_moderation_reports", system = "postgresql")]
    pub async fn list_moderation_reports(
        &self,
        community: CommunityId,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<moderation::ReportRecord>> {
        moderation::list_reports(&self.pool, community, status, limit).await
    }

    /// Fetch one moderation report by row id.
    #[datastore_span(name = "get_moderation_report", system = "postgresql")]
    pub async fn get_moderation_report(
        &self,
        community: CommunityId,
        report_id: Uuid,
    ) -> Result<Option<moderation::ReportRecord>> {
        moderation::get_report(&self.pool, community, report_id).await
    }

    /// Fetch one moderation report by signed NIP-56 report event id.
    #[datastore_span(name = "get_moderation_report_by_event", system = "postgresql")]
    pub async fn get_moderation_report_by_event(
        &self,
        community: CommunityId,
        report_event_id: &[u8],
    ) -> Result<Option<moderation::ReportRecord>> {
        moderation::get_report_by_event(&self.pool, community, report_event_id).await
    }

    /// Resolve, dismiss, or escalate an open moderation report.
    #[datastore_span(name = "resolve_moderation_report", system = "postgresql")]
    pub async fn resolve_moderation_report(
        &self,
        community: CommunityId,
        report_id: Uuid,
        status: &str,
        resolved_by: &[u8],
        action_id: Option<Uuid>,
    ) -> Result<bool> {
        moderation::resolve_report(
            &self.pool,
            community,
            report_id,
            status,
            resolved_by,
            action_id,
        )
        .await
    }

    /// Upsert a community ban for a member pubkey.
    #[datastore_span(name = "ban_community_member", system = "postgresql")]
    pub async fn ban_community_member(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        actor: &[u8],
        reason: Option<&str>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        moderation::ban_member(&self.pool, community, pubkey, actor, reason, expires_at).await
    }

    /// Lift a community ban for a member pubkey.
    #[datastore_span(name = "unban_community_member", system = "postgresql")]
    pub async fn unban_community_member(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        actor: &[u8],
    ) -> Result<bool> {
        moderation::unban_member(&self.pool, community, pubkey, actor).await
    }

    /// Upsert a community timeout/write-block for a member pubkey.
    #[datastore_span(name = "timeout_community_member", system = "postgresql")]
    pub async fn timeout_community_member(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        actor: &[u8],
        muted_until: DateTime<Utc>,
        reason: Option<&str>,
    ) -> Result<()> {
        moderation::timeout_member(&self.pool, community, pubkey, actor, muted_until, reason).await
    }

    /// Clear a community timeout/write-block for a member pubkey.
    #[datastore_span(name = "untimeout_community_member", system = "postgresql")]
    pub async fn untimeout_community_member(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        actor: &[u8],
    ) -> Result<bool> {
        moderation::untimeout_member(&self.pool, community, pubkey, actor).await
    }

    /// Fetch the active ban/timeout restriction state for enforcement hot paths.
    #[datastore_span(name = "moderation_restriction_state", system = "postgresql")]
    pub async fn moderation_restriction_state(
        &self,
        community: CommunityId,
        pubkey: &[u8],
    ) -> Result<moderation::RestrictionState> {
        moderation::restriction_state(&self.pool, community, pubkey).await
    }

    /// Fetch the full ban/timeout row for a member pubkey.
    #[datastore_span(name = "get_community_ban", system = "postgresql")]
    pub async fn get_community_ban(
        &self,
        community: CommunityId,
        pubkey: &[u8],
    ) -> Result<Option<moderation::BanRecord>> {
        moderation::get_ban(&self.pool, community, pubkey).await
    }

    /// List currently restricted members in a community.
    #[datastore_span(name = "list_community_restrictions", system = "postgresql")]
    pub async fn list_community_restrictions(
        &self,
        community: CommunityId,
    ) -> Result<Vec<moderation::BanRecord>> {
        moderation::list_restricted(&self.pool, community).await
    }

    /// Insert a moderation audit action row.
    #[datastore_span(name = "insert_moderation_action", system = "postgresql")]
    pub async fn insert_moderation_action(
        &self,
        community: CommunityId,
        action: moderation::NewAction<'_>,
    ) -> Result<Uuid> {
        moderation::insert_action(&self.pool, community, action).await
    }

    /// List moderation audit action rows, newest first.
    #[datastore_span(name = "list_moderation_actions", system = "postgresql")]
    pub async fn list_moderation_actions(
        &self,
        community: CommunityId,
        limit: i64,
    ) -> Result<Vec<moderation::ActionRecord>> {
        moderation::list_actions(&self.pool, community, limit).await
    }

    /// Return the current owner of git repo name `repo_id` in `community`, or
    /// `None` if unreserved. See [`git_repo::repo_name_owner`].
    #[datastore_span(name = "repo_name_owner", system = "postgresql")]
    pub async fn repo_name_owner(
        &self,
        community: CommunityId,
        repo_id: &str,
    ) -> Result<Option<String>> {
        git_repo::repo_name_owner(&self.pool, community, repo_id).await
    }

    /// Reserve a git repo name for `owner_pubkey` in `community` (NIP-34).
    ///
    /// See [`git_repo::reserve_repo_name`] for the outcome semantics. The
    /// per-pubkey quota is enforced by the caller against `count_repos_for_owner`.
    #[datastore_span(name = "reserve_repo_name", system = "postgresql")]
    pub async fn reserve_repo_name(
        &self,
        community: CommunityId,
        repo_id: &str,
        owner_pubkey: &str,
    ) -> Result<git_repo::ReserveOutcome> {
        git_repo::reserve_repo_name(&self.pool, community, repo_id, owner_pubkey).await
    }

    /// Count git repos reserved by `owner_pubkey` in `community` (quota check).
    #[datastore_span(name = "count_repos_for_owner", system = "postgresql")]
    pub async fn count_repos_for_owner(
        &self,
        community: CommunityId,
        owner_pubkey: &str,
    ) -> Result<i64> {
        git_repo::count_repos_for_owner(&self.pool, community, owner_pubkey).await
    }

    /// Release a git repo name reservation held by `owner_pubkey` (rollback).
    ///
    /// Returns the number of rows removed (0 or 1). See [`git_repo::release_repo_name`].
    #[datastore_span(name = "release_repo_name", system = "postgresql")]
    pub async fn release_repo_name(
        &self,
        community: CommunityId,
        repo_id: &str,
        owner_pubkey: &str,
    ) -> Result<u64> {
        git_repo::release_repo_name(&self.pool, community, repo_id, owner_pubkey).await
    }

    /// Returns `true` if `pubkey` (64-char hex) is archived in `community_id`.
    #[datastore_span(name = "is_archived", system = "postgresql")]
    pub async fn is_archived(&self, community_id: CommunityId, pubkey: &str) -> Result<bool> {
        archived_identities::is_archived(&self.pool, community_id, pubkey).await
    }

    /// Archives an identity in `community_id`. Returns `true` if inserted, `false` if already archived.
    #[allow(clippy::too_many_arguments)]
    #[datastore_span(name = "archive", system = "postgresql")]
    pub async fn archive(
        &self,
        community_id: CommunityId,
        pubkey: &str,
        consent_path: &str,
        actor: &str,
        reason: Option<&str>,
        replaced_by: Option<&str>,
        request_event_id: &str,
    ) -> Result<bool> {
        archived_identities::archive(
            &self.pool,
            community_id,
            pubkey,
            consent_path,
            actor,
            reason,
            replaced_by,
            request_event_id,
        )
        .await
    }

    /// Unarchives an identity from `community_id`. Returns `true` if deleted, `false` if absent.
    #[datastore_span(name = "unarchive", system = "postgresql")]
    pub async fn unarchive(&self, community_id: CommunityId, pubkey: &str) -> Result<bool> {
        archived_identities::unarchive(&self.pool, community_id, pubkey).await
    }

    /// Returns all identities archived in `community_id`, ordered by archive time ascending.
    #[datastore_span(name = "list_archived", system = "postgresql")]
    pub async fn list_archived(
        &self,
        community_id: CommunityId,
    ) -> Result<Vec<archived_identities::ArchivedIdentity>> {
        archived_identities::list_archived(&self.pool, community_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::CommunityId;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_db() -> Db {
        let database_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
        let pool = PgPool::connect(&database_url)
            .await
            .expect("connect to test DB");
        Db::from_pool(pool)
    }

    async fn make_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("communities-of-channels-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert community");
        id
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn database_guard_covers_legacy_writer_and_nip09_deletion() {
        use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

        let db = setup_db().await;
        let community = CommunityId::from_uuid(make_community(&db.pool).await);
        let keys = Keys::generate();
        let d_tag = format!("read-state:{}", "b".repeat(32));
        let tags = vec![
            Tag::parse(["d", d_tag.as_str()]).expect("d tag"),
            Tag::parse(["t", "read-state"]).expect("t tag"),
        ];
        let base = Timestamp::now().as_secs();
        let a = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "A")
            .tags(tags.clone())
            .custom_created_at(Timestamp::from(base))
            .sign_with_keys(&keys)
            .expect("sign A");
        let x = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "X")
            .tags(tags.clone())
            .custom_created_at(Timestamp::from(base + 1))
            .sign_with_keys(&keys)
            .expect("sign X");
        let b = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "B")
            .tags(tags.clone())
            .custom_created_at(Timestamp::from(base + 2))
            .sign_with_keys(&keys)
            .expect("sign B");
        let c = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_READ_STATE as u16), "C")
            .tags(tags)
            .custom_created_at(Timestamp::from(base + 3))
            .sign_with_keys(&keys)
            .expect("sign C");

        async fn legacy_insert(
            pool: &PgPool,
            community: CommunityId,
            event: &nostr::Event,
            d_tag: &str,
        ) -> std::result::Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
            sqlx::query(
                "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, d_tag) \
                 VALUES ($1, $2, $3, to_timestamp($4), $5, $6, $7, $8, NOW(), $9) ON CONFLICT DO NOTHING",
            )
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .bind(event.pubkey.to_bytes())
            .bind(event.created_at.as_secs() as f64)
            .bind(buzz_core::kind::KIND_READ_STATE as i32)
            .bind(serde_json::to_value(&event.tags).expect("serialize tags"))
            .bind(&event.content)
            .bind(event.sig.serialize().as_slice())
            .bind(d_tag)
            .execute(pool)
            .await
        }

        legacy_insert(&db.pool, community, &a, &d_tag)
            .await
            .expect("legacy insert A");
        let duplicate = legacy_insert(&db.pool, community, &a, &d_tag)
            .await
            .expect("legacy duplicate A remains idempotent");
        assert_eq!(duplicate.rows_affected(), 0);

        sqlx::query(
            "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
        )
        .bind(community.as_uuid())
        .bind("c".repeat(64))
        .bind(a.id.as_bytes().as_slice())
        .bind(a.created_at.as_secs() as f64)
        .execute(&db.pool)
        .await
        .expect("insert live mention");

        // Emulate the pre-PR replacement path after migration 0007: soft-delete
        // the live row, then insert B without any application watermark write.
        sqlx::query(
            "UPDATE events SET deleted_at=NOW() \
             WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .execute(&db.pool)
        .await
        .expect("legacy soft-delete A");
        let mentions_after_delete: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM event_mentions WHERE community_id=$1 AND event_id=$2",
        )
        .bind(community.as_uuid())
        .bind(a.id.as_bytes().as_slice())
        .fetch_one(&db.pool)
        .await
        .expect("count mentions after delete");
        assert_eq!(mentions_after_delete, 0);

        let stale_mention = sqlx::query(
            "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
        )
        .bind(community.as_uuid())
        .bind("d".repeat(64))
        .bind(a.id.as_bytes().as_slice())
        .bind(a.created_at.as_secs() as f64)
        .execute(&db.pool)
        .await
        .expect("stale post-commit mention is skipped");
        assert_eq!(stale_mention.rows_affected(), 0);

        legacy_insert(&db.pool, community, &b, &d_tag)
            .await
            .expect("legacy insert B");
        let duplicate_b = legacy_insert(&db.pool, community, &b, &d_tag)
            .await
            .expect("live duplicate B is skipped");
        assert_eq!(duplicate_b.rows_affected(), 0);

        sqlx::query(
            "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
        )
        .bind(community.as_uuid())
        .bind("e".repeat(64))
        .bind(b.id.as_bytes().as_slice())
        .bind(b.created_at.as_secs() as f64)
        .execute(&db.pool)
        .await
        .expect("insert B mention");

        // Exercise the new Rust hard-delete path independently. An in-flight
        // mention holds KEY SHARE on B, so replacement by C must block, then
        // complete after the mention commits and remove both B and its mention.
        let mut rust_mention_tx = db
            .pool
            .begin()
            .await
            .expect("begin Rust mention transaction");
        sqlx::query(
            "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind("e".repeat(64))
        .bind(b.id.as_bytes().as_slice())
        .bind(b.created_at.as_secs() as f64)
        .execute(&mut *rust_mention_tx)
        .await
        .expect("hold B live-event key-share lock");

        let replace_db = db.clone();
        let replace_d_tag = d_tag.clone();
        let replace_c = c.clone();
        let replace_task = tokio::spawn(async move {
            replace_db
                .replace_parameterized_event(community, &replace_c, &replace_d_tag, None)
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !replace_task.is_finished(),
            "Rust hard delete should wait for mention lock"
        );
        rust_mention_tx
            .commit()
            .await
            .expect("release Rust mention lock");
        let replaced = tokio::time::timeout(std::time::Duration::from_secs(2), replace_task)
            .await
            .expect("Rust hard delete deadlocked with mention insert")
            .expect("replacement task panicked")
            .expect("replace B with C");
        assert!(replaced.1, "C must replace B");
        let b_mentions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM event_mentions WHERE community_id=$1 AND event_id=$2",
        )
        .bind(community.as_uuid())
        .bind(b.id.as_bytes().as_slice())
        .fetch_one(&db.pool)
        .await
        .expect("count B mentions after Rust replacement");
        assert_eq!(b_mentions, 0);

        sqlx::query(
            "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078)",
        )
        .bind(community.as_uuid())
        .bind("f".repeat(64))
        .bind(c.id.as_bytes().as_slice())
        .bind(c.created_at.as_secs() as f64)
        .execute(&db.pool)
        .await
        .expect("insert C mention");

        // Exercise legacy UPDATE-trigger deletion with the same barrier. While
        // deletion waits on C's KEY SHARE lock, an exact replay must already be
        // a zero-row trigger no-op; it must not wait for deletion or resurrect C.
        let mut legacy_mention_tx = db
            .pool
            .begin()
            .await
            .expect("begin legacy mention transaction");
        sqlx::query(
            "INSERT INTO event_mentions \
                 (community_id, pubkey_hex, event_id, event_created_at, event_kind) \
             VALUES ($1, $2, $3, to_timestamp($4), 30078) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind("f".repeat(64))
        .bind(c.id.as_bytes().as_slice())
        .bind(c.created_at.as_secs() as f64)
        .execute(&mut *legacy_mention_tx)
        .await
        .expect("hold C live-event key-share lock");

        let delete_pool = db.pool.clone();
        let delete_pubkey = keys.public_key().to_bytes();
        let delete_d_tag = d_tag.clone();
        let delete_task = tokio::spawn(async move {
            sqlx::query(
                "UPDATE events SET deleted_at=NOW() \
                 WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3 AND deleted_at IS NULL",
            )
            .bind(community.as_uuid())
            .bind(delete_pubkey)
            .bind(delete_d_tag)
            .execute(&delete_pool)
            .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !delete_task.is_finished(),
            "legacy delete should wait for mention lock"
        );

        let replay_while_delete_waits = legacy_insert(&db.pool, community, &c, &d_tag)
            .await
            .expect("concurrent exact C replay is skipped");
        assert_eq!(replay_while_delete_waits.rows_affected(), 0);

        legacy_mention_tx
            .commit()
            .await
            .expect("release legacy mention lock");
        tokio::time::timeout(std::time::Duration::from_secs(2), delete_task)
            .await
            .expect("legacy delete deadlocked with mention insert")
            .expect("delete task panicked")
            .expect("legacy NIP-09 delete C");

        let payloads: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("count retained payloads");
        assert_eq!(
            payloads, 0,
            "legacy soft deletes must not retain NIP-RS payloads"
        );

        // Opposite commit order: deletion has committed before exact replay.
        // Equality remains an observable zero-row no-op, never a resurrection.
        let replay_c = legacy_insert(&db.pool, community, &c, &d_tag)
            .await
            .expect("post-delete exact C replay is skipped");
        assert_eq!(replay_c.rows_affected(), 0);
        let payloads_after_exact_replay: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("count payloads after exact replay");
        assert_eq!(payloads_after_exact_replay, 0);

        let replay = legacy_insert(&db.pool, community, &x, &d_tag).await;
        assert!(
            replay.is_err(),
            "database guard must reject A < X < C replay"
        );

        let watermark: (chrono::DateTime<chrono::Utc>, Vec<u8>) = sqlx::query_as(
            "SELECT created_at, event_id FROM parameterized_event_watermarks \
             WHERE community_id=$1 AND kind=30078 AND pubkey=$2 AND d_tag=$3",
        )
        .bind(community.as_uuid())
        .bind(keys.public_key().to_bytes())
        .bind(&d_tag)
        .fetch_one(&db.pool)
        .await
        .expect("read C watermark");
        assert_eq!(watermark.0.timestamp(), base as i64 + 3);
        assert_eq!(watermark.1, c.id.as_bytes().as_slice());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn test_usage_metrics_lock_has_single_owner_and_releases_on_drop() {
        // Use a private scratch database — not the shared TEST_DATABASE_URL.
        // Postgres advisory locks are per-database; hardcoding the production
        // USAGE_METRICS_LOCK_KEY (0x4255_5A5A_4D45_5452) on the shared test DB
        // races any live buzz-relay on the same database (see #3619).
        let admin_url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&admin_url)
            .await
            .expect("connect admin to create scratch db");
        let (pool, scratch_name) = create_scratch_db(&admin, "usage_metrics_lock").await;
        let first = Db::from_pool(pool.clone());
        let second = Db::from_pool(pool.clone());
        // Same key as production (`buzz-relay` USAGE_METRICS_LOCK_KEY) — safe here
        // because the scratch DB is empty of other holders.
        let key = 0x4255_5A5A_4D45_5452;

        let mut leader = first
            .try_lock_usage_metrics(key)
            .await
            .expect("first lock attempt")
            .expect("first database handle becomes leader");
        assert!(leader.is_live().await, "lock owner remains reachable");
        assert!(
            second
                .try_lock_usage_metrics(key)
                .await
                .expect("second lock attempt")
                .is_none(),
            "another session cannot become leader while the guard exists"
        );

        drop(leader);
        assert!(
            second
                .try_lock_usage_metrics(key)
                .await
                .expect("lock attempt after leader drop")
                .is_some(),
            "dropping the detached session releases its advisory lock"
        );

        // Release any remaining session state before DROP DATABASE.
        drop(first);
        drop(second);
        drop_scratch_db(&admin, pool, &scratch_name).await;
    }

    // ---- Read-replica routing ------------------------------------------------
    //
    // These tests pin the routing contract of `Db::read()` and the two routed
    // methods. A second scratch database stands in for the replica; the
    // fixtures are deliberately DIVERGENT (rows that exist in only one of the
    // two databases) so every assertion observes which pool actually served
    // the query instead of trusting the routing code's word for it.

    async fn admin_url() -> String {
        std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into())
    }

    /// Create a fresh scratch database on the same server and optionally run migrations.
    async fn create_scratch_db_through(
        admin: &PgPool,
        prefix: &str,
        target: Option<i64>,
    ) -> (PgPool, String) {
        let name = format!("{}_{}", prefix, Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(admin)
            .await
            .expect("create scratch db");
        let base = admin_url().await;
        // Swap the database path segment of the admin URL for the scratch name.
        let scratch_url = {
            let idx = base.rfind('/').expect("db url has a path segment");
            format!("{}/{}", &base[..idx], name)
        };
        let pool = PgPool::connect(&scratch_url)
            .await
            .expect("connect scratch db");
        match target {
            Some(target) => migration::run_migrations_through(&pool, target)
                .await
                .expect("migrate scratch db through target"),
            None => migration::run_migrations(&pool)
                .await
                .expect("migrate scratch db"),
        }
        (pool, name)
    }

    /// Create a fresh scratch database on the same server and run all migrations.
    /// Returns (pool, db_name); callers should `drop_scratch_db` when done.
    async fn create_scratch_db(admin: &PgPool, prefix: &str) -> (PgPool, String) {
        create_scratch_db_through(admin, prefix, None).await
    }

    async fn drop_scratch_db(admin: &PgPool, pool: PgPool, name: &str) {
        pool.close().await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(admin)
        .await;
    }

    /// Insert identical community + channel rows into a database so the same
    /// (community, channel) ids resolve in both writer and replica.
    async fn seed_community_channel(
        pool: &PgPool,
        community: Uuid,
        channel: Uuid,
        author: &nostr::Keys,
    ) {
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("replica-routing-{}.example", community.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        crate::channel::create_channel_with_id(
            pool,
            CommunityId::from_uuid(community),
            channel,
            &format!("replica-routing-{channel}"),
            crate::channel::ChannelType::Stream,
            crate::channel::ChannelVisibility::Open,
            None,
            author.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create channel");
    }

    fn signed_event_at(keys: &nostr::Keys, content: &str, secs: u64) -> nostr::Event {
        nostr::EventBuilder::new(nostr::Kind::Custom(9), content)
            .custom_created_at(nostr::Timestamp::from(secs))
            .sign_with_keys(keys)
            .expect("sign event")
    }

    async fn insert_top_level(pool: &PgPool, community: Uuid, channel: Uuid, ev: &nostr::Event) {
        let ts =
            chrono::DateTime::from_timestamp(ev.created_at.as_secs() as i64, 0).expect("valid ts");
        event::insert_event_with_thread_metadata(
            pool,
            CommunityId::from_uuid(community),
            ev,
            Some(channel),
            Some(event::ThreadMetadataParams {
                event_id: ev.id.as_bytes(),
                event_created_at: ts,
                channel_id: channel,
                parent_event_id: None,
                parent_event_created_at: None,
                root_event_id: None,
                root_event_created_at: None,
                depth: 0,
                broadcast: true,
            }),
        )
        .await
        .expect("insert top-level event");
    }

    async fn insert_thread_reply(
        pool: &PgPool,
        community: Uuid,
        channel: Uuid,
        root: &nostr::Event,
        reply: &nostr::Event,
    ) {
        let reply_ts = chrono::DateTime::from_timestamp(reply.created_at.as_secs() as i64, 0)
            .expect("valid ts");
        let root_ts = chrono::DateTime::from_timestamp(root.created_at.as_secs() as i64, 0)
            .expect("valid ts");
        event::insert_event_with_thread_metadata(
            pool,
            CommunityId::from_uuid(community),
            reply,
            Some(channel),
            Some(event::ThreadMetadataParams {
                event_id: reply.id.as_bytes(),
                event_created_at: reply_ts,
                channel_id: channel,
                parent_event_id: Some(root.id.as_bytes()),
                parent_event_created_at: Some(root_ts),
                root_event_id: Some(root.id.as_bytes()),
                root_event_created_at: Some(root_ts),
                depth: 1,
                broadcast: false,
            }),
        )
        .await
        .expect("insert reply");
    }

    /// Composite thread cursor: 8-byte BE seconds + raw event id.
    fn thread_cursor(reply: &crate::thread::ThreadReply) -> Vec<u8> {
        let mut cur = reply.created_at.timestamp().to_be_bytes().to_vec();
        cur.extend_from_slice(&reply.event_id);
        cur
    }

    #[tokio::test]
    async fn read_falls_back_to_writer_when_no_replica_configured() {
        // Pure wiring test — connect_lazy never touches the network.
        let pool = sqlx::PgPool::connect_lazy(TEST_DB_URL).expect("lazy pool");
        let db = Db::from_pool(pool);
        assert!(!db.has_read_pool());
        assert!(
            std::ptr::eq(db.read(), &db.pool),
            "read() must be the writer pool when no replica is configured"
        );
        assert!(db.read_pool_stats().is_none());
    }

    #[test]
    fn read_budget_zero_disables_and_large_values_clamp_to_staleness() {
        assert_eq!(read_budget_from_ms(0), None, "0 = bounded routing off");
        assert_eq!(
            read_budget_from_ms(1000),
            Some(std::time::Duration::from_millis(1000))
        );
        assert_eq!(
            read_budget_from_ms(10_000_000),
            Some(replica_fence::FENCE_STALENESS),
            "budgets above the staleness gate clamp to it"
        );
    }

    /// Truth table for [`RoutePredicate::for_query`]: the strongest sound
    /// predicate per query shape, and — the deploy-day default row — that
    /// `routing_enabled = false` (BUZZ_REPLICA_READ_MAX_AGE_MS unset)
    /// forces `Bounded` even for covered-eligible shapes, so the zero
    /// budget fails the new seams closed (Dawn's covered-at-zero-budget
    /// catch, design doc rev 5).
    #[test]
    fn for_query_predicate_truth_table() {
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let channel = Uuid::new_v4();
        let until = chrono::Utc::now();

        let pinned_with_until = {
            let mut q = event::EventQuery::for_community(community);
            q.channel_id = Some(channel);
            q.until = Some(until);
            q
        };
        let pinned_no_until = {
            let mut q = event::EventQuery::for_community(community);
            q.channel_id = Some(channel);
            q
        };
        let unpinned_with_until = {
            let mut q = event::EventQuery::for_community(community);
            q.until = Some(until);
            q
        };
        let global_only = {
            let mut q = event::EventQuery::for_community(community);
            q.global_only = true;
            q.until = Some(until);
            q
        };

        // Deploy-day default: budget unset ⇒ Bounded regardless of shape.
        // The zero budget then fails Bounded closed, so the new seams
        // record writer/disabled — merging with no env var set is a no-op.
        assert!(
            matches!(
                RoutePredicate::for_query(&pinned_with_until, false),
                RoutePredicate::Bounded
            ),
            "budget unset must not reach the covered arm even when eligible"
        );

        // Budget set + channel pin + until ⇒ the strongest predicate.
        assert!(matches!(
            RoutePredicate::for_query(&pinned_with_until, true),
            RoutePredicate::BoundedOrCovered { .. }
        ));

        // Missing either covered precondition ⇒ Bounded.
        assert!(matches!(
            RoutePredicate::for_query(&pinned_no_until, true),
            RoutePredicate::Bounded
        ));
        assert!(matches!(
            RoutePredicate::for_query(&unpinned_with_until, true),
            RoutePredicate::Bounded
        ));
        // global_only implies `channel_id = None`, so the channel-pin
        // precondition fails and no covered arm is possible — `for_query`
        // never inspects `global_only` itself; the row holds because
        // constructor 1 (channel pin) returns None for an unpinned query.
        assert!(matches!(
            RoutePredicate::for_query(&global_only, true),
            RoutePredicate::Bounded
        ));
    }

    /// The pre-existing cursor paths are NOT budget-gated: a channel-window
    /// cursor page still derives `Covered` with no `routing_enabled` input
    /// at all — at B=0 today it routes covered, and that status quo is
    /// intentionally unchanged by the `for_query` gate (Max's matrix row:
    /// old paths route at budget-unset; only the new seams go dark).
    #[test]
    fn channel_cursor_predicate_is_not_budget_gated() {
        let channel = Uuid::new_v4();
        let cursor = Some((chrono::Utc::now(), vec![1u8; 32]));
        assert!(matches!(
            RoutePredicate::from_channel_cursor(channel, &cursor),
            RoutePredicate::Covered { .. }
        ));
        // Head fetch (no cursor) is bounded — gated by the budget.
        assert!(matches!(
            RoutePredicate::from_channel_cursor(channel, &None),
            RoutePredicate::Bounded
        ));
    }

    /// D5 wiring: `read_pool_stats().max` must be the READER pool's own
    /// ceiling, not the writer's — `buzz_db_read_pool_active / _max` is the
    /// operator's utilisation signal and inheriting the writer's max hides
    /// reader saturation by exactly the sizing ratio. Pure wiring test:
    /// `connect_lazy` never touches the network, but it does spawn the
    /// pool reaper task, which needs a Tokio runtime — hence
    /// `#[tokio::test]` despite the test body itself never awaiting.
    #[tokio::test]
    async fn read_pool_stats_reports_reader_ceiling_not_writer() {
        let writer = sqlx::postgres::PgPoolOptions::new()
            .max_connections(20)
            .connect_lazy(TEST_DB_URL)
            .expect("lazy writer pool");
        let reader = sqlx::postgres::PgPoolOptions::new()
            .max_connections(40)
            .connect_lazy(TEST_DB_URL)
            .expect("lazy reader pool");
        let db = Db::from_pools(writer, reader);
        assert_eq!(db.pool_stats().max, 20);
        assert_eq!(
            db.read_pool_stats().expect("read pool configured").max,
            40,
            "reader gauge must report the reader's own ceiling"
        );
    }

    /// D4 wiring: the reader pool is built lazily with `min_connections(0)`
    /// and the short reader acquire timeout — construction must succeed
    /// with no replica listening (reader-down at boot must not crash the
    /// relay), and `read_max_connections` must honour
    /// `DbConfig::read_max_connections` over the writer sizing.
    /// `#[tokio::test]` because `connect_lazy` spawns the pool reaper task,
    /// which needs a Tokio runtime even though nothing is dialed.
    #[tokio::test]
    async fn connect_read_pool_is_lazy_and_independently_sized() {
        let config = DbConfig {
            max_connections: 20,
            read_max_connections: Some(7),
            ..DbConfig::default()
        };
        // Unroutable per RFC 5737 TEST-NET-1: proves nothing is dialed at
        // construction time.
        let pool = Db::connect_read_pool(&config, "postgres://user:pw@192.0.2.1:5432/none", 7)
            .expect("lazy construction must not dial the replica");
        assert_eq!(pool.options().get_max_connections(), 7);
        assert_eq!(pool.options().get_min_connections(), 0);
        assert_eq!(
            pool.options().get_acquire_timeout(),
            Db::READER_ACQUIRE_TIMEOUT
        );
    }

    /// Channel window: head fetch (no cursor) reads the WRITER; cursor pages
    /// read the REPLICA. Divergent fixtures prove which pool served each.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn channel_window_routes_head_to_writer_and_cursor_pages_to_replica() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "routing_w").await;
        let (replica, rname) = create_scratch_db(&admin, "routing_r").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        // Shared history (both databases): m1 < m2 < m3.
        let base = 1_700_000_000u64;
        let m1 = signed_event_at(&author, "m1", base);
        let m2 = signed_event_at(&author, "m2", base + 10);
        let m3 = signed_event_at(&author, "m3", base + 20);
        for pool in [&writer, &replica] {
            for ev in [&m1, &m2, &m3] {
                insert_top_level(pool, community, channel, ev).await;
            }
        }
        // Lag: the newest event exists only on the writer.
        let fresh = signed_event_at(&author, "fresh-writer-only", base + 30);
        insert_top_level(&writer, community, channel, &fresh).await;
        // Marker: exists only on the "replica" (unphysical for a real replica,
        // but it makes replica-served pages unambiguous).
        let marker = signed_event_at(&author, "replica-only-marker", base + 5);
        insert_top_level(&replica, community, channel, &marker).await;

        let db = Db::from_pools(writer.clone(), replica.clone());
        // Open the fence through "now": the fixture's history is far in the
        // past, so every cursor falls below the fence and routing is
        // eligible. Fence-gating itself is pinned by the fence tests below.
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);

        // Head fetch (cursor: None) → writer: sees `fresh`, never `marker`.
        let head = db
            .get_channel_window(cid, channel, 2, None, None)
            .await
            .expect("head window");
        let head_contents: Vec<String> = head
            .rows
            .iter()
            .map(|r| r.stored_event.event.content.clone())
            .collect();
        assert_eq!(
            head_contents,
            vec!["fresh-writer-only".to_string(), "m3".to_string()],
            "head fetch must be served by the writer"
        );

        // Cursor page → replica: sees `marker`, never `fresh`.
        let cursor = head.next_cursor.expect("has_more implies next_cursor");
        let page2 = db
            .get_channel_window(cid, channel, 10, Some(cursor), None)
            .await
            .expect("cursor window");
        let page2_contents: Vec<String> = page2
            .rows
            .iter()
            .map(|r| r.stored_event.event.content.clone())
            .collect();
        assert_eq!(
            page2_contents,
            vec![
                "m2".to_string(),
                "replica-only-marker".to_string(),
                "m1".to_string()
            ],
            "cursor page must be served by the replica"
        );

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Fail-closed on a mid-request replica failure (Dawn, review of
    /// 1b0aa0dfa): a replica-routed page whose query errors *after* the
    /// proof (the live shape is a hot-standby recovery conflict — 40001 /
    /// 25P02 — cancelling the held snapshot under `max_standby_streaming_delay`)
    /// must be re-run on the writer and served, never surfaced as an error
    /// the writer could have answered. Degraded capacity, never holes.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replica_window_failure_falls_back_to_writer() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "fb_w").await;
        let (replica, rname) = create_scratch_db(&admin, "fb_r").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let m1 = signed_event_at(&author, "m1", base);
        let m2 = signed_event_at(&author, "m2", base + 10);
        let m3 = signed_event_at(&author, "m3", base + 20);
        for pool in [&writer, &replica] {
            for ev in [&m1, &m2, &m3] {
                insert_top_level(pool, community, channel, ev).await;
            }
        }
        let marker = signed_event_at(&author, "replica-only-marker", base + 5);
        insert_top_level(&replica, community, channel, &marker).await;

        let db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);

        let head = db
            .get_channel_window(cid, channel, 1, None, None)
            .await
            .expect("head window");
        let cursor = head.next_cursor.expect("has_more implies next_cursor");

        // Guard against a vacuous pass: the cursor page must actually be
        // replica-eligible before we break the replica.
        let healthy = db
            .get_channel_window(cid, channel, 10, Some(cursor.clone()), None)
            .await
            .expect("healthy cursor window");
        assert!(
            healthy
                .rows
                .iter()
                .any(|r| r.stored_event.event.content == "replica-only-marker"),
            "fixture must route the cursor page to the replica while healthy"
        );

        // Break the replica AFTER the proof point: the heartbeat table stays
        // intact (the observation succeeds), the page query then fails.
        sqlx::query("DROP TABLE events CASCADE")
            .execute(&replica)
            .await
            .expect("drop replica events");

        let page = db
            .get_channel_window(cid, channel, 10, Some(cursor), None)
            .await
            .expect("replica failure must fall back to the writer, not error");
        let contents: Vec<&str> = page
            .rows
            .iter()
            .map(|r| r.stored_event.event.content.as_str())
            .collect();
        assert_eq!(
            contents,
            vec!["m2", "m1"],
            "fallback page must be the writer's answer (no replica marker)"
        );

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// [`replica_window_failure_falls_back_to_writer`] for the thread-replies
    /// path: a replica-routed thread page whose query errors after the proof
    /// re-runs on the writer instead of surfacing an error.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn replica_thread_failure_falls_back_to_writer() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "fbt_w").await;
        let (replica, rname) = create_scratch_db(&admin, "fbt_r").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let root = signed_event_at(&author, "root", base);
        for pool in [&writer, &replica] {
            insert_top_level(pool, community, channel, &root).await;
        }
        let replies: Vec<nostr::Event> = (1..=3)
            .map(|i| signed_event_at(&author, &format!("r{i}"), base + 10 * i as u64))
            .collect();
        for pool in [&writer, &replica] {
            for reply in &replies {
                insert_thread_reply(pool, community, channel, &root, reply).await;
            }
        }
        // Replica-only divergent reply between r2 and r3 marks replica serves.
        let ghost = signed_event_at(&author, "replica-only-ghost", base + 25);
        insert_thread_reply(&replica, community, channel, &root, &ghost).await;

        let db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);

        let page1 = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 2, None)
            .await
            .expect("head page");
        let cur = thread_cursor(page1.last().expect("page 1 non-empty"));

        // Healthy: the full page after r2 is the replica's [ghost].
        let healthy = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur))
            .await
            .expect("healthy replica page");
        assert_eq!(
            healthy[0].stored_event.event.content, "replica-only-ghost",
            "fixture must route the cursor page to the replica while healthy"
        );

        sqlx::query("DROP TABLE events CASCADE")
            .execute(&replica)
            .await
            .expect("drop replica events");

        let page = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur))
            .await
            .expect("replica failure must fall back to the writer, not error");
        assert_eq!(
            page[0].stored_event.event.content, "r3",
            "fallback page must be the writer's answer"
        );

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Mid-request degradation of the held session (Dawn, review of
    /// 1b0aa0dfa): when the proved replica transaction dies between the page
    /// and an aux follow-up (stand-in: `pg_terminate_backend` on the reader
    /// connection, the same tx-fatal shape as a recovery-conflict cancel),
    /// [`ReadSession::query_events`] must re-run the query on the writer and
    /// permanently degrade the session instead of surfacing the error.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn read_session_degrades_to_writer_when_replica_connection_dies() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "deg_w").await;
        let (replica, rname) = create_scratch_db(&admin, "deg_r").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let m1 = signed_event_at(&author, "m1", base);
        let m2 = signed_event_at(&author, "m2", base + 10);
        for pool in [&writer, &replica] {
            for ev in [&m1, &m2] {
                insert_top_level(pool, community, channel, ev).await;
            }
        }
        // Writer-only row proves the degraded aux ran on the writer.
        let fresh = signed_event_at(&author, "fresh-writer-only", base + 20);
        insert_top_level(&writer, community, channel, &fresh).await;

        let db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);

        let head = db
            .get_channel_window(cid, channel, 1, None, None)
            .await
            .expect("head window");
        let cursor = head.next_cursor.expect("has_more implies next_cursor");
        let (_window, mut session) = db
            .get_channel_window_with_session(cid, channel, 10, Some(cursor), None)
            .await
            .expect("routed cursor window");
        assert!(
            session.is_replica(),
            "fixture must route this page to the replica"
        );

        // Kill the reader's backend out from under the held transaction.
        sqlx::query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE datname = $1 AND pid <> pg_backend_pid()",
        )
        .bind(&rname)
        .execute(&admin)
        .await
        .expect("terminate replica backends");

        let mut aux = EventQuery::for_community(cid);
        aux.channel_id = Some(channel);
        let rows = session
            .query_events(&aux)
            .await
            .expect("session must degrade to the writer, not error");
        assert!(
            rows.iter()
                .any(|se| se.event.content == "fresh-writer-only"),
            "degraded aux must be served by the writer"
        );
        assert!(
            !session.is_replica(),
            "the session must be permanently degraded to the writer"
        );

        drop(session);
        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Snapshot continuity (Wren, review of 17ea2ff6a): the routed request
    /// runs inside ONE `REPEATABLE READ, READ ONLY` transaction whose first
    /// statement was the heartbeat observation — so a row committed on the
    /// replica *after* the proof must be invisible to every follow-up
    /// statement in the same request (page, participants, aux). This
    /// distinguishes the transaction contract from mere connection reuse:
    /// autocommit statements on the same backend advance their snapshot
    /// per statement and WOULD see the mid-request row.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn routed_request_holds_one_snapshot_across_page_and_aux() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "snap_w").await;
        let (replica, rname) = create_scratch_db(&admin, "snap_r").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let m1 = signed_event_at(&author, "m1", base);
        let m2 = signed_event_at(&author, "m2", base + 10);
        for pool in [&writer, &replica] {
            for ev in [&m1, &m2] {
                insert_top_level(pool, community, channel, ev).await;
            }
        }

        let db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);

        // Head page on the writer yields the cursor for a replica-routed page.
        let head = db
            .get_channel_window(cid, channel, 1, None, None)
            .await
            .expect("head window");
        let cursor = head.next_cursor.expect("has_more implies next_cursor");

        // Route the cursor page to the replica and HOLD the session.
        let (window, mut session) = db
            .get_channel_window_with_session(cid, channel, 10, Some(cursor), None)
            .await
            .expect("routed cursor window");
        assert!(
            session.is_replica(),
            "fixture must route this page to the replica"
        );
        assert_eq!(window.rows.len(), 1, "page after m2 is [m1]");

        // Mid-request: a new event commits on the replica (stands in for
        // replay advancing between the page and the aux closure).
        let mid = signed_event_at(&author, "mid-request-commit", base + 5);
        insert_top_level(&replica, community, channel, &mid).await;

        // A fresh autocommit statement on ANOTHER session sees it — the row
        // is really there (control for the assertion below).
        let mut control = EventQuery::for_community(cid);
        control.channel_id = Some(channel);
        let visible_elsewhere = event::query_events(&replica, &control)
            .await
            .expect("control query");
        assert!(
            visible_elsewhere
                .iter()
                .any(|se| se.event.content == "mid-request-commit"),
            "control: the mid-request row must be committed and visible to a new snapshot"
        );

        // The held request session must NOT see it: its snapshot was
        // anchored by the heartbeat observation, before the commit.
        let mut aux = EventQuery::for_community(cid);
        aux.channel_id = Some(channel);
        let in_request = session.query_events(&aux).await.expect("aux query");
        assert!(
            !in_request
                .iter()
                .any(|se| se.event.content == "mid-request-commit"),
            "request transaction must hold the proof-time snapshot; a \
             mid-request commit leaking in means the aux ran outside the \
             request transaction (autocommit connection reuse)"
        );
        // Rows from the proof-time snapshot are still served.
        assert!(
            in_request.iter().any(|se| se.event.content == "m1"),
            "proof-time rows must remain visible in the request snapshot"
        );

        drop(session);
        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Head gate (Predicate A): with the budget unset, a head fetch reads
    /// the writer even over an open fence; with a budget set and a fresh
    /// proved entry, the head page is served by the replica session
    /// (bounded staleness accepted); with a budget the fence entry exceeds,
    /// the head page falls back to the writer.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn head_fetch_routes_by_configured_budget() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "head_w").await;
        let (replica, rname) = create_scratch_db(&admin, "head_r").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let shared = signed_event_at(&author, "shared", base);
        for pool in [&writer, &replica] {
            insert_top_level(pool, community, channel, &shared).await;
        }
        // Divergent heads prove which pool served the fetch.
        let fresh = signed_event_at(&author, "fresh-writer-only", base + 30);
        insert_top_level(&writer, community, channel, &fresh).await;
        let marker = signed_event_at(&author, "replica-only-marker", base + 20);
        insert_top_level(&replica, community, channel, &marker).await;

        let mut db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);
        let head_contents = |w: &thread::ChannelWindow| -> Vec<String> {
            w.rows
                .iter()
                .map(|r| r.stored_event.event.content.clone())
                .collect()
        };

        // Budget unset (rollout default): head → writer, fence open or not.
        let head = db
            .get_channel_window(cid, channel, 2, None, None)
            .await
            .expect("head, gate off");
        assert_eq!(
            head_contents(&head),
            vec!["fresh-writer-only".to_string(), "shared".to_string()],
            "head routing must default off"
        );

        // Budget set, entry fresh (just recorded): head → replica.
        db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
        let head = db
            .get_channel_window(cid, channel, 2, None, None)
            .await
            .expect("head, gate on");
        assert_eq!(
            head_contents(&head),
            vec!["replica-only-marker".to_string(), "shared".to_string()],
            "a fresh proved entry within budget must serve the head from the replica"
        );

        // Entry older than the budget: head falls back to the writer.
        db.fence().close();
        db.fence().force_open_for_tests_at(
            chrono::Utc::now(),
            std::time::Instant::now() - std::time::Duration::from_secs(10),
        );
        let head = db
            .get_channel_window(cid, channel, 2, None, None)
            .await
            .expect("head, entry too old");
        assert_eq!(
            head_contents(&head),
            vec!["fresh-writer-only".to_string(), "shared".to_string()],
            "an over-budget entry must fail the head gate closed"
        );

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// End-to-end deploy-default proof for the NEW routed seams: with the
    /// budget unset, a covered-eligible query (channel-pinned + `until`)
    /// through [`Db::query_events_routed`] is served by the WRITER — the
    /// `for_query` gate keeps the covered arm dark (rev 5). With the budget
    /// set and a fresh proved entry, the same query routes to the replica.
    /// Divergent fixtures prove which pool served each read.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn query_events_routed_defaults_dark_and_routes_covered_when_enabled() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "qer_w").await;
        let (replica, rname) = create_scratch_db(&admin, "qer_r").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let shared = signed_event_at(&author, "shared", base);
        for pool in [&writer, &replica] {
            insert_top_level(pool, community, channel, &shared).await;
        }
        let writer_only = signed_event_at(&author, "writer-only", base + 10);
        insert_top_level(&writer, community, channel, &writer_only).await;
        let replica_only = signed_event_at(&author, "replica-only", base + 20);
        insert_top_level(&replica, community, channel, &replica_only).await;

        let mut db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);

        // Covered-eligible shape: channel-pinned with an `until` upper
        // bound below the (now) fence wall.
        let q = {
            let mut q = EventQuery::for_community(cid);
            q.channel_id = Some(channel);
            q.until = chrono::DateTime::from_timestamp((base + 60) as i64, 0);
            q
        };
        let contents = |evs: &[StoredEvent]| -> std::collections::BTreeSet<String> {
            evs.iter().map(|e| e.event.content.clone()).collect()
        };

        // Deploy default: budget unset ⇒ writer, even though the shape is
        // covered-eligible and the fence is open.
        let rows = db
            .query_events_routed("test_routed", &q)
            .await
            .expect("routed query, gate off");
        assert!(
            contents(&rows).contains("writer-only"),
            "budget unset must serve the writer"
        );
        assert!(
            !contents(&rows).contains("replica-only"),
            "budget unset must not reach the replica via the covered arm"
        );

        // Budget set ⇒ the covered arm serves it from the replica.
        db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
        let rows = db
            .query_events_routed("test_routed", &q)
            .await
            .expect("routed query, gate on");
        assert!(
            contents(&rows).contains("replica-only"),
            "budget set + covered-eligible must route to the replica"
        );
        assert!(!contents(&rows).contains("writer-only"));

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// COUNT is bounded-only (rev 5 deletion-visibility rule): a
    /// covered-eligible shape must NOT let a count take the covered arm.
    /// With the budget unset the count reads the WRITER even with an open
    /// fence; with the budget set and a fresh entry it reads the replica
    /// under the bounded arm. Divergent row counts prove the serving pool.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn count_events_routed_is_bounded_only() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "cnt_w").await;
        let (replica, rname) = create_scratch_db(&admin, "cnt_r").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        // Writer: 2 rows. Replica: 1 row.
        for (i, content) in ["a", "b"].iter().enumerate() {
            let ev = signed_event_at(&author, content, base + i as u64);
            insert_top_level(&writer, community, channel, &ev).await;
        }
        let ev = signed_event_at(&author, "c", base);
        insert_top_level(&replica, community, channel, &ev).await;

        let mut db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);

        // Covered-eligible shape on purpose: pinned + until. A count must
        // ignore that eligibility.
        let q = {
            let mut q = EventQuery::for_community(cid);
            q.channel_id = Some(channel);
            q.until = chrono::DateTime::from_timestamp((base + 60) as i64, 0);
            q
        };

        // Budget unset ⇒ bounded arm disabled ⇒ writer.
        let n = db
            .count_events_routed("test_count", &q)
            .await
            .expect("count, gate off");
        assert_eq!(n, 2, "budget unset must count on the writer");

        // Budget set + fresh entry ⇒ bounded arm ⇒ replica.
        db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
        let n = db
            .count_events_routed("test_count", &q)
            .await
            .expect("count, gate on");
        assert_eq!(n, 1, "budget set must count on the replica (bounded)");

        // Entry older than the budget ⇒ bounded fails ⇒ writer. Covered
        // would still hold here (upper <= wall) — proving count never
        // consults it.
        db.fence().close();
        db.fence().force_open_for_tests_at(
            chrono::Utc::now(),
            std::time::Instant::now() - std::time::Duration::from_secs(10),
        );
        let n = db
            .count_events_routed("test_count", &q)
            .await
            .expect("count, entry too old");
        assert_eq!(
            n, 2,
            "an over-budget entry must fail the count closed to the writer, \
             even when the covered arm would admit the shape"
        );

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Routed relay-membership check: budget unset ⇒ writer; budget set +
    /// fresh proved entry ⇒ replica (bounded arm); over-budget entry ⇒
    /// writer. Divergent membership rows prove which pool answered.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn is_relay_member_is_bounded_routed_and_fails_closed() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "mem_w").await;
        let (replica, rname) = create_scratch_db(&admin, "mem_r").await;

        let community = Uuid::new_v4();
        for pool in [&writer, &replica] {
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(community)
                .bind(format!("member-routing-{}.example", community.simple()))
                .execute(pool)
                .await
                .expect("insert community");
        }
        let cid = CommunityId::from_uuid(community);
        let writer_only = "aa".repeat(32);
        let replica_only = "bb".repeat(32);
        relay_members::add_relay_member(&writer, cid, &writer_only, "member", None)
            .await
            .expect("seed writer member");
        relay_members::add_relay_member(&replica, cid, &replica_only, "member", None)
            .await
            .expect("seed replica member");

        let mut db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());

        // Budget unset ⇒ bounded arm disabled ⇒ writer.
        assert!(
            db.is_relay_member(cid, &writer_only)
                .await
                .expect("gate off"),
            "budget unset must answer from the writer"
        );
        assert!(!db.is_relay_member(cid, &replica_only).await.unwrap());

        // Budget set + fresh entry ⇒ replica.
        db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
        assert!(
            db.is_relay_member(cid, &replica_only)
                .await
                .expect("gate on"),
            "budget set must answer from the replica"
        );
        assert!(!db.is_relay_member(cid, &writer_only).await.unwrap());

        // Entry older than the budget ⇒ fail closed to the writer. Close
        // first so no prior fresh entry can be the one proved (matches the
        // count test; today `force_open_for_tests_at` also clears the ring).
        db.fence().close();
        db.fence().force_open_for_tests_at(
            chrono::Utc::now(),
            std::time::Instant::now() - std::time::Duration::from_secs(10),
        );
        assert!(
            db.is_relay_member(cid, &writer_only)
                .await
                .expect("entry too old"),
            "an over-budget entry must fail closed to the writer"
        );

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Community separation across every routed seam, verified on
    /// REPLICA-SERVED reads.
    ///
    /// The pre-existing feed/event scoping tests prove the shared SQL
    /// builders confine rows to one community, but they exercise those
    /// builders through the WRITER wrapper. `_on` variants are
    /// executor-only refactors, so scoping *should* be identical — this
    /// test refuses to take that on faith and re-proves it through the
    /// routed executor, on a snapshot the replica actually served.
    ///
    /// Construction: two communities A and B exist in BOTH databases with
    /// the same ids. The replica additionally holds a `replica-only` row in
    /// each — divergent fixtures, so any row bearing that content proves
    /// the replica (not the writer) served the read. Every assertion
    /// requests A and demands B's rows never appear, including B's
    /// `replica-only` row, which is the one a leaky predicate would surface.
    /// The routed fallback must cost ONE reader acquire budget, even when the
    /// Aurora capability cache is cold.
    ///
    /// Regression test for a stacked-budget bug found at `9fa3c9c0b`: the
    /// capability probe used to `acquire()` from the pool itself and return
    /// `false` *uncached* on `PoolTimedOut`, so the routed read then spent a
    /// SECOND `READER_ACQUIRE_TIMEOUT` inside `begin`. Measured 302ms against
    /// a ~150ms documented bound. Boot priming
    /// ([`Db::spawn_read_pool_boot_ping`]) hid it only when the boot ping
    /// SUCCEEDED — and a reader that is unavailable at boot is exactly the
    /// case the bound is specified for, so the two failures are correlated.
    ///
    /// The fixture reproduces that state deliberately: a size-1 reader whose
    /// sole connection is established and then HELD (so every further acquire
    /// must time out), with `reader_aurora_identity` asserted cold. It routes
    /// through `count_events_routed` rather than calling `proved_reader`
    /// directly, because `buzz_db_route_decision` is emitted by `route_read`
    /// — a direct call would prove the timing but never emit the label.
    ///
    /// Timing uses an upper bound of 2x the budget minus a margin: it must
    /// fail for two stacked budgets (~300ms) while tolerating scheduler
    /// jitter on one (~150ms). Asserting a lower bound too would pin the
    /// budget's own value, which `reader_acquire_timeout_is_the_documented_budget`
    /// already covers.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires Postgres"]
    async fn routed_fallback_spends_one_acquire_budget_when_aurora_cache_is_cold() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (seed, wname) = create_scratch_db(&admin, "one_budget").await;
        seed.close().await;
        let base = admin_url().await;
        let scratch_url = {
            let idx = base.rfind('/').expect("db url has a path segment");
            format!("{}/{}", &base[..idx], wname)
        };

        // `Db::new` so the writer arms the floor guard and the reader is the
        // real lazy `connect_read_pool` pool (min_connections=0, 150ms
        // acquire timeout). Reader is sized 1 so holding one connection
        // saturates it.
        let mut db = Db::new(&DbConfig {
            database_url: scratch_url.clone(),
            read_database_url: Some(scratch_url),
            max_connections: 4,
            read_max_connections: Some(1),
            ..DbConfig::default()
        })
        .await
        .expect("connect armed Db with size-1 lazy reader");
        db.fence().force_open_for_tests(chrono::Utc::now());
        db.set_replica_read_max_age_for_tests(Some(Duration::from_secs(5)));

        let read_pool = db.read_pool.clone().expect("reader pool configured");
        // Establish and hold the reader's only connection: saturated.
        let held = read_pool
            .acquire()
            .await
            .expect("establish the reader's sole connection");
        assert_eq!(
            db.read_max_connections, 1,
            "reader max must report 1 for this fixture to test saturation"
        );
        assert_eq!(
            read_pool.size(),
            1,
            "the sole reader connection is established and held"
        );
        // The bug is only observable with the capability cache cold; if a
        // future change primes it here, this fixture would silently stop
        // discriminating.
        assert!(
            db.reader_aurora_identity.get().is_none(),
            "Aurora capability must be UNPRIMED (post-boot-ping-failure state)"
        );

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let query = EventQuery::for_community(CommunityId::from_uuid(Uuid::new_v4()));

        // The recorder is installed thread-locally, so it must stay installed
        // across the `.await` — hence the guard form rather than
        // `with_local_recorder`, whose closure cannot host an await. The
        // `current_thread` flavor keeps the route decision on this thread; on
        // a multi-thread runtime the emit could land on a worker where no
        // local recorder is installed and the label assertions would vacuously
        // see an empty snapshot.
        let start = std::time::Instant::now();
        let count = {
            let _guard = metrics::set_default_local_recorder(&recorder);
            db.count_events_routed("one_budget_probe", &query).await
        }
        .expect("writer fallback still answers the read");
        let elapsed = start.elapsed();

        assert_eq!(count, 0, "writer answered on an empty scratch database");
        assert!(
            elapsed < Duration::from_millis(250),
            "routed fallback must spend ONE {}ms acquire budget, not two; took {}ms",
            Db::READER_ACQUIRE_TIMEOUT.as_millis(),
            elapsed.as_millis()
        );

        let reasons: std::collections::HashMap<(String, String), u64> = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, ..)| key.key().name() == "buzz_db_route_decision")
            .map(|(key, _, _, value)| {
                let metrics_util::debugging::DebugValue::Counter(n) = value else {
                    panic!("buzz_db_route_decision must be a counter");
                };
                let labels: Vec<_> = key.key().labels().collect();
                let get = |name: &str| {
                    labels
                        .iter()
                        .find(|l| l.key() == name)
                        .map(|l| l.value().to_owned())
                        .unwrap_or_default()
                };
                ((get("decision"), get("reason")), n)
            })
            .collect();

        assert_eq!(
            reasons.get(&("writer".to_owned(), "reader_acquire_timeout".to_owned())),
            Some(&1),
            "saturated reader must fall back as writer/reader_acquire_timeout; got {reasons:?}"
        );
        // `reader_validation_error` would mean we misclassified a timeout as a
        // broken reader, and `pool_busy` is the retired name — neither may
        // appear in ANY emitted label.
        assert!(
            !reasons
                .keys()
                .any(|(_, reason)| reason == "reader_validation_error" || reason == "pool_busy"),
            "no reader_validation_error or retired pool_busy label may be emitted; got {reasons:?}"
        );

        drop(held);
        drop_scratch_db(&admin, db.pool.clone(), &wname).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn routed_reads_are_confined_to_the_requested_community() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "sep_w").await;
        let (replica, rname) = create_scratch_db(&admin, "sep_r").await;

        let author = nostr::Keys::generate();
        let (comm_a, chan_a) = (Uuid::new_v4(), Uuid::new_v4());
        let (comm_b, chan_b) = (Uuid::new_v4(), Uuid::new_v4());
        for pool in [&writer, &replica] {
            seed_community_channel(pool, comm_a, chan_a, &author).await;
            seed_community_channel(pool, comm_b, chan_b, &author).await;
        }

        // A p-tag mention is what makes a row eligible for the mentions and
        // needs-action feeds. Kind 9 satisfies mentions + activity;
        // needs-action admits only approval/reminder kinds, so each
        // community also gets a kind-46010 row.
        let mentioned = nostr::Keys::generate();
        let mentioned_hex = mentioned.public_key().to_hex();
        let mentioned_bytes = mentioned.public_key().to_bytes();
        let tagged_kind = |kind: u16, content: &str, secs: u64| {
            nostr::EventBuilder::new(nostr::Kind::Custom(kind), content)
                .tags([nostr::Tag::parse(["p", mentioned_hex.as_str()]).expect("p tag")])
                .custom_created_at(nostr::Timestamp::from(secs))
                .sign_with_keys(&author)
                .expect("sign event")
        };
        let tagged = |content: &str, secs: u64| tagged_kind(9, content, secs);

        let base = 1_700_000_000u64;
        // Shared rows (both DBs) + replica-only rows (divergence) per community.
        let a_shared = tagged("a-shared", base);
        let b_shared = tagged("b-shared", base + 1);
        for pool in [&writer, &replica] {
            insert_top_level(pool, comm_a, chan_a, &a_shared).await;
            insert_mentions(
                pool,
                CommunityId::from_uuid(comm_a),
                &a_shared,
                Some(chan_a),
            )
            .await
            .expect("mentions a-shared");
            insert_top_level(pool, comm_b, chan_b, &b_shared).await;
            insert_mentions(
                pool,
                CommunityId::from_uuid(comm_b),
                &b_shared,
                Some(chan_b),
            )
            .await
            .expect("mentions b-shared");
        }
        let a_replica_only = tagged("a-replica-only", base + 10);
        let b_replica_only = tagged("b-replica-only", base + 11);
        insert_top_level(&replica, comm_a, chan_a, &a_replica_only).await;
        insert_mentions(
            &replica,
            CommunityId::from_uuid(comm_a),
            &a_replica_only,
            Some(chan_a),
        )
        .await
        .expect("mentions a-replica-only");
        insert_top_level(&replica, comm_b, chan_b, &b_replica_only).await;
        insert_mentions(
            &replica,
            CommunityId::from_uuid(comm_b),
            &b_replica_only,
            Some(chan_b),
        )
        .await
        .expect("mentions b-replica-only");

        // Needs-action fixtures: approval kind, replica-only in BOTH
        // communities, so the assertion below is replica-served on A and
        // must still not see B's.
        let a_approval = tagged_kind(46010, "a-approval-replica-only", base + 20);
        let b_approval = tagged_kind(46010, "b-approval-replica-only", base + 21);
        insert_top_level(&replica, comm_a, chan_a, &a_approval).await;
        insert_mentions(
            &replica,
            CommunityId::from_uuid(comm_a),
            &a_approval,
            Some(chan_a),
        )
        .await
        .expect("mentions a-approval");
        insert_top_level(&replica, comm_b, chan_b, &b_approval).await;
        insert_mentions(
            &replica,
            CommunityId::from_uuid(comm_b),
            &b_approval,
            Some(chan_b),
        )
        .await
        .expect("mentions b-approval");

        let mut db = Db::from_pools(writer.clone(), replica.clone());
        db.fence().force_open_for_tests(chrono::Utc::now());
        db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(5)));
        let cid_a = CommunityId::from_uuid(comm_a);

        let contents = |evs: &[StoredEvent]| -> std::collections::BTreeSet<String> {
            evs.iter().map(|e| e.event.content.clone()).collect()
        };
        // Every routed seam must (a) have been served by the replica —
        // proven by a divergent row absent from the writer — and (b) contain
        // no row belonging to community B. All B fixtures are named `b-*`,
        // so the leak check is a single prefix scan.
        let assert_a_only = |rows: &[StoredEvent], marker: &str, seam: &str| {
            let got = contents(rows);
            assert!(
                got.contains(marker),
                "{seam}: must be replica-served (divergent row `{marker}` absent from writer); got {got:?}"
            );
            assert!(
                !got.iter().any(|c| c.starts_with("b-")),
                "{seam}: community B rows leaked into a community A read; got {got:?}"
            );
        };

        // 1. Generic query — covered arm (channel-pinned + `until`).
        let mut q = EventQuery::for_community(cid_a);
        q.channel_id = Some(chan_a);
        q.until = chrono::DateTime::from_timestamp((base + 60) as i64, 0);
        let rows = db
            .query_events_routed("sep_query", &q)
            .await
            .expect("routed query");
        assert_a_only(&rows, "a-replica-only", "query_events_routed");

        // 2. Generic query — bounded arm (no channel pin at all, so a
        //    missing community predicate could not be masked by the pin).
        let unpinned = EventQuery::for_community(cid_a);
        let rows = db
            .query_events_routed_bounded("sep_query_bounded", &unpinned)
            .await
            .expect("routed bounded query");
        assert_a_only(&rows, "a-replica-only", "query_events_routed_bounded");

        // 3. COUNT — bounded-only. Community A holds 3 rows on the replica
        //    (shared + replica-only + approval) but only 1 on the writer,
        //    and 3 more exist in community B. Exactly 3 proves the read was
        //    both replica-served and community-confined.
        let count = db
            .count_events_routed("sep_count", &unpinned)
            .await
            .expect("routed count");
        assert_eq!(
            count, 3,
            "count must see A's three replica rows only — not B's, not the writer's one"
        );

        // 4. By-ID hydration — ids carry no channel pin, and B's ids are
        //    requested alongside A's. Only A's may hydrate.
        let ids: Vec<&[u8]> = vec![
            a_shared.id.as_bytes(),
            a_replica_only.id.as_bytes(),
            b_shared.id.as_bytes(),
            b_replica_only.id.as_bytes(),
        ];
        let rows = db
            .get_events_by_ids_routed("sep_by_ids", cid_a, &ids)
            .await
            .expect("routed by-ids");
        assert_a_only(&rows, "a-replica-only", "get_events_by_ids_routed");

        // 5-7. All three feed builders, each given BOTH channels as
        //      accessible — so only the community predicate can exclude B.
        let both = [chan_a, chan_b];
        let rows = db
            .query_feed_mentions_routed("sep_feed", cid_a, &mentioned_bytes, &both, None, 50)
            .await
            .expect("routed mentions");
        assert_a_only(&rows, "a-replica-only", "query_feed_mentions_routed");

        let rows = db
            .query_feed_needs_action_routed("sep_feed", cid_a, &mentioned_bytes, &both, None, 50)
            .await
            .expect("routed needs action");
        assert_a_only(
            &rows,
            "a-approval-replica-only",
            "query_feed_needs_action_routed",
        );

        let rows = db
            .query_feed_activity_routed("sep_feed", cid_a, &both, None, 50)
            .await
            .expect("routed activity");
        assert_a_only(&rows, "a-replica-only", "query_feed_activity_routed");

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// D4: a LAZY reader pool (connect_lazy, min_connections=0, never yet
    /// used) must still let [`Db::spawn_fence_probe`] verify the writer's
    /// floor guard and spawn — reader-down or reader-idle at boot must not
    /// disable fence probing.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn lazy_reader_pool_still_spawns_fence_probe() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (seed, wname) = create_scratch_db(&admin, "lazy_w").await;
        seed.close().await;

        let writer_url = {
            let base = admin_url().await;
            let idx = base.rfind('/').expect("db url has a path segment");
            format!("{}/{}", &base[..idx], wname)
        };
        // `Db::new` (not `from_pools`) so the WRITER pool arms the
        // `buzz.created_at_floor` GUC — `spawn_fence_probe` verifies the
        // floor guard on a writer connection, and `create_scratch_db`'s
        // plain `PgPool::connect` never arms it. The reader is still the
        // lazy `connect_read_pool` pool this test is about.
        let db = Db::new(&DbConfig {
            database_url: writer_url.clone(),
            read_database_url: Some(writer_url),
            max_connections: 2,
            ..DbConfig::default()
        })
        .await
        .expect("connect armed Db with lazy reader");

        let spawned = db
            .spawn_fence_probe()
            .await
            .expect("floor-guard verification must pass on the migrated writer");
        assert!(spawned, "a configured (lazy) reader must spawn the probe");

        drop_scratch_db(&admin, db.pool.clone(), &wname).await;
    }

    /// Thread replies: head fetch reads the writer; a FULL cursor page is
    /// served by the replica; an UNDER-limit cursor page (candidate terminal
    /// page) is re-run on the writer so a lagged replica can never truncate
    /// the tail into a false EOF.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn thread_replies_cursor_pages_route_to_replica_with_writer_terminal_verification() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "routing_tw").await;
        let (replica, rname) = create_scratch_db(&admin, "routing_tr").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let root = signed_event_at(&author, "root", base);
        for pool in [&writer, &replica] {
            insert_top_level(pool, community, channel, &root).await;
        }

        // Writer holds replies r1..r5; the lagged replica only has r1..r3.
        let replies: Vec<nostr::Event> = (1..=5)
            .map(|i| signed_event_at(&author, &format!("r{i}"), base + 10 * i as u64))
            .collect();
        for reply in &replies {
            insert_thread_reply(&writer, community, channel, &root, reply).await;
        }
        for reply in &replies[..3] {
            insert_thread_reply(&replica, community, channel, &root, reply).await;
        }

        let db = Db::from_pools(writer.clone(), replica.clone());
        // Open the fence through "now" — fixture history is far in the past.
        db.fence().force_open_for_tests(chrono::Utc::now());
        let cid = CommunityId::from_uuid(community);

        // Page 1 (no cursor) → writer.
        let page1 = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 2, None)
            .await
            .expect("page 1");
        let contents: Vec<&str> = page1
            .iter()
            .map(|r| r.stored_event.event.content.as_str())
            .collect();
        assert_eq!(contents, vec!["r1", "r2"], "head page from writer");

        // Page 2: replica serves a FULL page (r3 exists there) — but wait:
        // replica has r1..r3, page after r2 with limit 2 returns only [r3]
        // (under limit) → terminal-verification re-runs on the writer, which
        // returns [r3, r4]. A lag-truncated EOF must never surface.
        let cur2 = thread_cursor(page1.last().expect("page 1 non-empty"));
        let page2 = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 2, Some(&cur2))
            .await
            .expect("page 2");
        let contents: Vec<&str> = page2
            .iter()
            .map(|r| r.stored_event.event.content.as_str())
            .collect();
        assert_eq!(
            contents,
            vec!["r3", "r4"],
            "under-limit replica page must be re-verified on the writer"
        );

        // Full-page replica serve: with limit 1, the page after r2 is [r3] —
        // exactly `limit` rows, so the replica result stands. Prove it came
        // from the replica with a replica-only divergent reply.
        let ghost = signed_event_at(&author, "replica-only-ghost", base + 25);
        insert_thread_reply(&replica, community, channel, &root, &ghost).await;
        let page_replica = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur2))
            .await
            .expect("full replica page");
        let contents: Vec<&str> = page_replica
            .iter()
            .map(|r| r.stored_event.event.content.as_str())
            .collect();
        assert_eq!(
            contents,
            vec!["replica-only-ghost"],
            "a full cursor page must be served by the replica"
        );

        // Same query with no replica configured reads the writer and cannot
        // see the ghost.
        let db_writer_only = Db::from_pool(writer.clone());
        let page_writer = db_writer_only
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur2))
            .await
            .expect("writer-only page");
        let contents: Vec<&str> = page_writer
            .iter()
            .map(|r| r.stored_event.event.content.as_str())
            .collect();
        assert_eq!(contents, vec!["r3"], "unset replica falls back to writer");

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Channel DESC scrollback, out-of-order commit adversary: the replica is
    /// missing a MIDDLE row (`m2`) because a transaction with an older
    /// client-signed `created_at` committed late and has not replayed yet.
    /// The replica's cursor page would be `[m1]` — silently skipping `m2`
    /// forever, since the next cursor advances past it. The fence must route
    /// any cursor above it to the writer.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn channel_cursor_above_fence_stays_on_writer_preventing_middle_hole() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "fence_cw").await;
        let (replica, rname) = create_scratch_db(&admin, "fence_cr").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let m1 = signed_event_at(&author, "m1", base);
        let m2 = signed_event_at(&author, "m2-late-commit", base + 10);
        let m3 = signed_event_at(&author, "m3", base + 20);
        let m4 = signed_event_at(&author, "m4", base + 30);
        for ev in [&m1, &m2, &m3, &m4] {
            insert_top_level(&writer, community, channel, ev).await;
        }
        // Replica replayed everything EXCEPT the late-committed m2.
        for ev in [&m1, &m3, &m4] {
            insert_top_level(&replica, community, channel, ev).await;
        }

        let db = Db::from_pools(writer.clone(), replica.clone());
        let cid = CommunityId::from_uuid(community);

        // Head page (writer): [m4, m3]; cursor lands on m3 (base+20).
        let head = db
            .get_channel_window(cid, channel, 2, None, None)
            .await
            .expect("head window");
        let cursor = head.next_cursor.expect("has_more implies next_cursor");

        // Fence closed → cursor page must come from the writer: m2 present.
        let contents = |w: &thread::ChannelWindow| -> Vec<String> {
            w.rows
                .iter()
                .map(|r| r.stored_event.event.content.clone())
                .collect()
        };
        let page_closed = db
            .get_channel_window(cid, channel, 10, Some(cursor.clone()), None)
            .await
            .expect("cursor page, fence closed");
        assert_eq!(
            contents(&page_closed),
            vec!["m2-late-commit".to_string(), "m1".to_string()],
            "fence closed: cursor pages route to the writer"
        );

        // Fence open but BELOW the cursor timestamp (covers base+5 only):
        // the cursor (base+20) is not covered → writer again.
        db.fence().force_open_for_tests(
            chrono::DateTime::from_timestamp(base as i64 + 5, 0).expect("ts"),
        );
        let page_below = db
            .get_channel_window(cid, channel, 10, Some(cursor.clone()), None)
            .await
            .expect("cursor page, fence below cursor");
        assert_eq!(
            contents(&page_below),
            vec!["m2-late-commit".to_string(), "m1".to_string()],
            "cursor above the fence must stay on the writer"
        );

        // Counterfactual pinning the hazard: were the fence (wrongly) open
        // through now, the replica would serve the page WITHOUT m2 — the
        // permanent-skip hole this fence exists to prevent.
        db.fence().force_open_for_tests(chrono::Utc::now());
        let page_hazard = db
            .get_channel_window(cid, channel, 10, Some(cursor), None)
            .await
            .expect("cursor page, fence wrongly open");
        assert_eq!(
            contents(&page_hazard),
            vec!["m1".to_string()],
            "fixture models the inversion: an over-open fence would skip m2"
        );

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Thread ASC pagination, out-of-order commit adversary: the replica
    /// holds a FULL page whose newest row (`r4`) has a later key than a
    /// not-yet-replayed row (`r3`). The old under-limit check alone would
    /// serve `[r4]` and the client cursor would advance past `r3` forever.
    /// The fence rule (full AND tail ≤ fence) must send that page to the
    /// writer instead.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn thread_full_replica_page_above_fence_is_reverified_on_writer() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (writer, wname) = create_scratch_db(&admin, "fence_tw").await;
        let (replica, rname) = create_scratch_db(&admin, "fence_tr").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&writer, community, channel, &author).await;
        seed_community_channel(&replica, community, channel, &author).await;

        let base = 1_700_000_000u64;
        let root = signed_event_at(&author, "root", base);
        for pool in [&writer, &replica] {
            insert_top_level(pool, community, channel, &root).await;
        }
        let replies: Vec<nostr::Event> = (1..=4)
            .map(|i| signed_event_at(&author, &format!("r{i}"), base + 10 * i as u64))
            .collect();
        for reply in &replies {
            insert_thread_reply(&writer, community, channel, &root, reply).await;
        }
        // Replica replayed r1, r2, r4 — the late-committed r3 is missing.
        for reply in [&replies[0], &replies[1], &replies[3]] {
            insert_thread_reply(&replica, community, channel, &root, reply).await;
        }

        let db = Db::from_pools(writer.clone(), replica.clone());
        let cid = CommunityId::from_uuid(community);

        // Fence covers r2 (base+20) but not r3/r4.
        db.fence().force_open_for_tests(
            chrono::DateTime::from_timestamp(base as i64 + 20, 0).expect("ts"),
        );

        // Page after r2 with limit 1: the replica would return the FULL page
        // [r4] — but its tail is above the fence, so the writer re-runs it
        // and returns [r3]. No skip.
        let page1 = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 2, None)
            .await
            .expect("head page");
        let cur = thread_cursor(page1.last().expect("head page non-empty"));
        let page = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur))
            .await
            .expect("cursor page");
        let contents: Vec<&str> = page
            .iter()
            .map(|r| r.stored_event.event.content.as_str())
            .collect();
        assert_eq!(
            contents,
            vec!["r3"],
            "a full replica page above the fence must be re-run on the writer"
        );

        // Counterfactual: an over-open fence would serve the replica's [r4],
        // skipping r3 permanently.
        db.fence().force_open_for_tests(chrono::Utc::now());
        let hazard = db
            .get_thread_replies(cid, root.id.as_bytes(), Some(10), 1, Some(&cur))
            .await
            .expect("hazard page");
        let contents: Vec<&str> = hazard
            .iter()
            .map(|r| r.stored_event.event.content.as_str())
            .collect();
        assert_eq!(
            contents,
            vec!["r4"],
            "fixture models the inversion: an over-open fence would skip r3"
        );

        drop_scratch_db(&admin, replica, &rname).await;
        drop_scratch_db(&admin, writer, &wname).await;
    }

    /// Commit-time floor guard (migration 0021), exact held-transaction
    /// adversary: a channel-bearing row whose `created_at` is older than the
    /// floor at COMMIT time must abort the transaction — the guard runs
    /// inside commit processing with `clock_timestamp()`, so holding the
    /// transaction open cannot outrun it. channel_id-NULL rows are
    /// structurally exempt, and sessions without the GUC are unaffected.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn created_at_floor_guard_aborts_old_channel_rows_at_commit() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (pool, name) = create_scratch_db(&admin, "floor_guard").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&pool, community, channel, &author).await;

        let insert_raw = |ev: nostr::Event, channel_id: Option<Uuid>| {
            let pool = pool.clone();
            async move {
                let mut tx = pool.begin().await.expect("begin");
                // Arm the guard for this transaction only (the relay's
                // writer pool arms it per connection; tests are explicit).
                sqlx::query("SELECT set_config('buzz.created_at_floor', $1, true)")
                    .bind(crate::replica_fence::CREATED_AT_FLOOR_SECS.to_string())
                    .execute(&mut *tx)
                    .await
                    .expect("arm guard");
                sqlx::query(
                    "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, \
                     content, sig, received_at, channel_id) \
                     VALUES ($1, $2, $3, to_timestamp($4), 9, '[]', $5, $6, NOW(), $7)",
                )
                .bind(community)
                .bind(ev.id.as_bytes().as_slice())
                .bind(ev.pubkey.to_bytes().as_slice())
                .bind(ev.created_at.as_secs() as f64)
                .bind(&ev.content)
                .bind(ev.sig.serialize().as_slice())
                .bind(channel_id)
                .execute(&mut *tx)
                .await
                .expect("insert inside tx (guard is deferred to commit)");
                // Hold the transaction "open" past the insert, then commit —
                // the deferred guard must still see the stale created_at.
                sqlx::query("SELECT pg_sleep(0.05)")
                    .execute(&mut *tx)
                    .await
                    .expect("hold tx");
                tx.commit().await
            }
        };

        let now_secs = chrono::Utc::now().timestamp() as u64;
        let floor = crate::replica_fence::CREATED_AT_FLOOR_SECS as u64;

        // Old channel-bearing row → COMMIT aborts with check_violation.
        let old = signed_event_at(&author, "old-held-tx", now_secs - floor - 60);
        let err = insert_raw(old, Some(channel))
            .await
            .expect_err("below-floor channel row must abort at COMMIT");
        let code = match &err {
            sqlx::Error::Database(db_err) => db_err.code().map(|c| c.to_string()),
            other => panic!("expected database error, got {other:?}"),
        };
        assert_eq!(
            code.as_deref(),
            Some("23514"),
            "guard raises check_violation"
        );

        // Fresh channel-bearing row → commits.
        let fresh = signed_event_at(&author, "fresh", now_secs);
        insert_raw(fresh, Some(channel))
            .await
            .expect("fresh row commits under the armed guard");

        // Old row WITHOUT a channel (push lease / profile shapes) →
        // structurally exempt, commits.
        let old_global = signed_event_at(&author, "old-global", now_secs - floor - 60);
        insert_raw(old_global, None)
            .await
            .expect("channel_id-NULL rows are exempt from the floor");

        // Unarmed session (no GUC) → guard inert; backfills stay possible
        // (and must hold the fence closed, per the migration header).
        let old_backfill = signed_event_at(&author, "old-backfill", now_secs - floor - 60);
        insert_top_level(&pool, community, channel, &old_backfill).await;

        drop_scratch_db(&admin, pool, &name).await;
    }

    #[test]
    fn writer_pool_safety_hook_is_single_and_composed() {
        let source = include_str!("lib.rs");
        let connect_pool = source
            .split("async fn connect_pool")
            .nth(1)
            .and_then(|tail| tail.split("const READER_ACQUIRE_TIMEOUT").next())
            .expect("connect_pool source block");
        assert_eq!(
            connect_pool.matches(".after_connect(").count(),
            1,
            "SQLx replaces after_connect hooks; writer safety must use exactly one"
        );
        assert!(connect_pool.contains("buzz.created_at_floor"));
        assert!(connect_pool.contains("SHOW transaction_isolation"));
        assert!(!connect_pool.contains("arm_floor_guard"));
        assert!(!connect_pool.contains("_arm_floor_guard"));
        assert!(!connect_pool.contains("allow(unused_variables)"));

        let reader_doc = source
            .split("fn connect_read_pool")
            .next()
            .and_then(|prefix| prefix.rsplit("/// Connect the read-replica").next())
            .expect("reader pool documentation");
        assert!(reader_doc.contains("replica sessions are"));
        assert!(reader_doc.contains("read-only"));
        assert!(!reader_doc.contains("Db::connect_pool"));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn writer_pool_rejects_non_read_committed_database_default() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (seed_pool, name) = create_scratch_db(&admin, "writer_isolation").await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER DATABASE {name} SET default_transaction_isolation = 'repeatable read'"
        )))
        .execute(&admin)
        .await
        .expect("set unsafe database default");
        seed_pool.close().await;

        let base = admin_url().await;
        let idx = base.rfind('/').expect("db url has a path segment");
        let scratch_url = format!("{}/{}", &base[..idx], name);
        let error = Db::new(&DbConfig {
            database_url: scratch_url,
            max_connections: 1,
            min_connections: 1,
            acquire_timeout_secs: 1,
            ..DbConfig::default()
        })
        .await
        .expect_err("writer pool must reject pinned-snapshot database defaults");
        assert!(
            error.to_string().contains("requires READ COMMITTED")
                || error.to_string().contains("pool timed out"),
            "unexpected isolation rejection: {error}"
        );

        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop isolation test database");
    }

    /// The armed writer pool (`Db::new`) must enforce the floor end-to-end
    /// through the public insert APIs, and the session GUC must be verifiably
    /// set on pooled connections.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn armed_pool_rejects_old_channel_inserts_through_public_api() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (seed_pool, name) = create_scratch_db(&admin, "floor_pool").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&seed_pool, community, channel, &author).await;

        // Connect a Db the production way: after_connect arms the guard.
        let base = admin_url().await;
        let idx = base.rfind('/').expect("db url has a path segment");
        let scratch_url = format!("{}/{}", &base[..idx], name);
        let db = Db::new(&DbConfig {
            database_url: scratch_url,
            max_connections: 2,
            ..DbConfig::default()
        })
        .await
        .expect("connect armed Db");
        let cid = CommunityId::from_uuid(community);

        // Perci nit: assert the effective session value, not the intent.
        let effective: String = sqlx::query_scalar("SHOW buzz.created_at_floor")
            .fetch_one(&db.pool)
            .await
            .expect("SHOW guard GUC");
        assert_eq!(
            effective,
            crate::replica_fence::CREATED_AT_FLOOR_SECS.to_string(),
            "writer pool must arm the floor guard on every connection"
        );
        let isolation: String = sqlx::query_scalar("SHOW transaction_isolation")
            .fetch_one(&db.pool)
            .await
            .expect("SHOW writer isolation");
        assert_eq!(
            isolation, "read committed",
            "the same writer after_connect hook must enforce the isolation premise"
        );

        let now_secs = chrono::Utc::now().timestamp() as u64;
        let floor = crate::replica_fence::CREATED_AT_FLOOR_SECS as u64;

        // insert_event (single INSERT, autocommit): old channel row rejected.
        let old = signed_event_at(&author, "old-direct", now_secs - floor - 60);
        let err = event::insert_event(&db.pool, cid, &old, Some(channel))
            .await
            .expect_err("armed pool must reject below-floor channel inserts");
        assert!(
            err.to_string().contains("below the replica-fence floor"),
            "unexpected error: {err}"
        );

        // insert_event_with_thread_metadata (multi-statement tx): same.
        let old2 = signed_event_at(&author, "old-thread-meta", now_secs - floor - 90);
        let ts = chrono::DateTime::from_timestamp(old2.created_at.as_secs() as i64, 0)
            .expect("valid ts");
        let err = event::insert_event_with_thread_metadata(
            &db.pool,
            cid,
            &old2,
            Some(channel),
            Some(event::ThreadMetadataParams {
                event_id: old2.id.as_bytes(),
                event_created_at: ts,
                channel_id: channel,
                parent_event_id: None,
                parent_event_created_at: None,
                root_event_id: None,
                root_event_created_at: None,
                depth: 0,
                broadcast: true,
            }),
        )
        .await
        .expect_err("armed pool must reject below-floor thread-metadata inserts");
        assert!(
            err.to_string().contains("below the replica-fence floor"),
            "unexpected error: {err}"
        );

        // Fresh events pass through both APIs.
        let fresh = signed_event_at(&author, "fresh-direct", now_secs);
        event::insert_event(&db.pool, cid, &fresh, Some(channel))
            .await
            .expect("fresh insert passes the armed guard");

        drop_scratch_db(&admin, seed_pool, &name).await;
        // db pool still holds connections to the dropped DB; close it.
        db.pool.close().await;
    }

    /// `spawn_fence_probe` must verify the floor guard before letting the
    /// probe run — catalog shape AND observed behavior — and refuse on
    /// sabotage. This is the production gate for a relay running with
    /// `BUZZ_AUTO_MIGRATE` off: an armed GUC with no enforcing trigger must
    /// never yield an open fence.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn fence_probe_refuses_to_start_without_verified_floor_guard() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (seed_pool, wname) = create_scratch_db(&admin, "fence_gate_w").await;
        let (replica_pool, rname) = create_scratch_db(&admin, "fence_gate_r").await;
        seed_pool.close().await;
        replica_pool.close().await;

        let base = admin_url().await;
        let idx = base.rfind('/').expect("db url has a path segment");
        let writer_url = format!("{}/{}", &base[..idx], wname);
        let replica_url = format!("{}/{}", &base[..idx], rname);

        // Healthy schema: verification passes, probe starts. A SEPARATE Db
        // instance, because its background probe legitimately opens its own
        // fence (the heartbeat probe is writer-side only) — the refusal
        // assertions below must run against a fence whose spawns were all
        // refused.
        let db_healthy = Db::new(&DbConfig {
            database_url: writer_url.clone(),
            read_database_url: Some(replica_url.clone()),
            max_connections: 2,
            ..DbConfig::default()
        })
        .await
        .expect("connect armed Db with replica");
        assert!(
            db_healthy
                .spawn_fence_probe()
                .await
                .expect("verification passes"),
            "probe must start on a verified schema"
        );

        let db = Db::new(&DbConfig {
            database_url: writer_url,
            read_database_url: Some(replica_url),
            max_connections: 2,
            ..DbConfig::default()
        })
        .await
        .expect("connect armed Db with replica");

        // Sabotage A: catalog-shaped no-op — same trigger, gutted function
        // body. Catalog check alone would pass; behavior check must refuse.
        sqlx::query(
            "CREATE OR REPLACE FUNCTION events_created_at_floor_guard() RETURNS trigger \
             LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END $$",
        )
        .execute(&db.pool)
        .await
        .expect("gut the guard function");
        let err = db
            .spawn_fence_probe()
            .await
            .expect_err("inert guard body must refuse the probe");
        assert!(
            err.to_string().contains("floor guard is inert"),
            "unexpected error: {err}"
        );

        // Sabotage B: trigger dropped entirely (the BUZZ_AUTO_MIGRATE=off /
        // 0021-unapplied shape). Catalog check must refuse.
        sqlx::query("DROP TRIGGER events_created_at_floor ON events")
            .execute(&db.pool)
            .await
            .expect("drop the guard trigger");
        let err = db
            .spawn_fence_probe()
            .await
            .expect_err("missing trigger must refuse the probe");
        assert!(
            err.to_string().contains("missing or mis-shaped"),
            "unexpected error: {err}"
        );

        // In both refusal states the fence never opened.
        assert!(
            db.fence().verified_through().is_none(),
            "fence must remain closed when verification refuses the probe"
        );

        db_healthy.pool.close().await;
        if let Some(rp) = &db_healthy.read_pool {
            rp.close().await;
        }
        db.pool.close().await;
        if let Some(rp) = &db.read_pool {
            rp.close().await;
        }
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {wname} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {rname} WITH (FORCE)"
        )))
        .execute(&admin)
        .await;
    }

    /// The `UPDATE OF` arm of the floor guard (Perci's second structural
    /// hole): an old row legitimately admitted with `channel_id` NULL must
    /// not be movable into keyset windows, and a channel row's `created_at`
    /// must not be movable below the fence — through raw SQL, at COMMIT.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn floor_guard_blocks_updates_that_move_rows_below_the_fence() {
        let admin = PgPool::connect(&admin_url().await)
            .await
            .expect("connect admin");
        let (pool, name) = create_scratch_db(&admin, "floor_upd").await;

        let author = nostr::Keys::generate();
        let community = Uuid::new_v4();
        let channel = Uuid::new_v4();
        seed_community_channel(&pool, community, channel, &author).await;

        let now_secs = chrono::Utc::now().timestamp() as u64;
        let floor = crate::replica_fence::CREATED_AT_FLOOR_SECS as u64;

        // Seed via unarmed session: one old channel-NULL row, one fresh
        // channel row.
        let old_null = signed_event_at(&author, "old-null", now_secs - floor - 120);
        insert_top_level(&pool, community, channel, &old_null).await;
        sqlx::query("UPDATE events SET channel_id = NULL WHERE community_id = $1 AND id = $2")
            .bind(community)
            .bind(old_null.id.as_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("detach channel (unarmed seed)");
        let fresh = signed_event_at(&author, "fresh-row", now_secs);
        insert_top_level(&pool, community, channel, &fresh).await;

        // Armed transaction, deferred to COMMIT (the production shape).
        let run_armed_update = |sql: &'static str, id: Vec<u8>, age: Option<u64>| {
            let pool = pool.clone();
            async move {
                let mut tx = pool.begin().await.expect("begin");
                sqlx::query("SELECT set_config('buzz.created_at_floor', $1, true)")
                    .bind(crate::replica_fence::CREATED_AT_FLOOR_SECS.to_string())
                    .execute(&mut *tx)
                    .await
                    .expect("arm guard");
                let q = sqlx::query(sql).bind(community).bind(id);
                let q = match age {
                    Some(a) => q.bind(a as f64),
                    None => q,
                };
                q.execute(&mut *tx)
                    .await
                    .expect("update inside tx (deferred)");
                tx.commit().await
            }
        };

        // channel-NULL → channel-bearing on an old row: COMMIT must abort.
        let err = run_armed_update(
            "UPDATE events SET channel_id = community_id WHERE community_id = $1 AND id = $2",
            old_null.id.as_bytes().to_vec(),
            None,
        )
        .await
        .expect_err("moving an old channel-NULL row into a channel must abort at COMMIT");
        assert!(
            matches!(&err, sqlx::Error::Database(e) if e.code().as_deref() == Some("23514")),
            "unexpected error: {err}"
        );

        // created_at rewrite below the floor on a channel row: COMMIT must abort.
        let err = run_armed_update(
            "UPDATE events SET created_at = clock_timestamp() - make_interval(secs => $3::double precision) \
             WHERE community_id = $1 AND id = $2",
            fresh.id.as_bytes().to_vec(),
            Some(floor + 120),
        )
        .await
        .expect_err("rewriting created_at below the floor must abort at COMMIT");
        assert!(
            matches!(&err, sqlx::Error::Database(e) if e.code().as_deref() == Some("23514")),
            "unexpected error: {err}"
        );

        drop_scratch_db(&admin, pool, &name).await;
    }
}
