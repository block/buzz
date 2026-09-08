pub mod migration;
pub(crate) mod observability;
pub mod replica_fence;

use crate::{deletion, event, DbError, EventQuery, Result};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, QueryBuilder};
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
    let connection =
        observability::acquire_writer(pool, observability::WriterOperation::EventWrite).await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;
    insert_mentions_in_transaction(&mut tx, community_id, event, channel_id).await?;
    tx.commit().await?;
    Ok(())
}

/// Insert mention rows on the caller's transaction. Replacement writes use
/// this so the authoritative event and its discovery index commit or roll back
/// as one unit.
pub(crate) async fn insert_mentions_in_transaction(
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
    pub(crate) inner: ReadSessionInner,
}

pub(crate) enum ReadSessionInner {
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
pub(crate) enum RouteDecision {
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
pub(crate) mod route_proof {
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
pub(crate) enum RoutePredicate {
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
    pub(crate) fn from_channel_cursor(
        channel_id: Uuid,
        cursor: &Option<(DateTime<Utc>, Vec<u8>)>,
    ) -> Self {
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
    pub(crate) fn for_query(q: &event::EventQuery, routing_enabled: bool) -> Self {
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

/// Bounded outcome of the Postgres portion of a relay readiness check.
///
/// The variants deliberately separate waiting for a pooled connection from
/// executing the health query. Callers may safely use the variant names as
/// low-cardinality metric labels; detailed SQLx errors remain in logs rather
/// than becoming labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbReadinessOutcome {
    /// A writer-pool connection was acquired and `SELECT 1` succeeded.
    Success,
    /// No writer-pool connection became available before the readiness deadline.
    PoolTimeout,
    /// The writer pool returned a non-timeout acquisition error.
    PoolError,
    /// A connection was acquired, but `SELECT 1` exceeded the readiness deadline.
    QueryTimeout,
    /// A connection was acquired, but `SELECT 1` returned an error.
    QueryError,
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
    /// Session `lock_timeout` in milliseconds for writer connections (env
    /// `BUZZ_DB_LOCK_TIMEOUT_MS`). `0` disables the timeout.
    pub lock_timeout_ms: u64,
    /// Session `idle_in_transaction_session_timeout` in milliseconds for
    /// writer connections (env `BUZZ_DB_IDLE_TXN_TIMEOUT_MS`). `0` disables.
    pub idle_txn_timeout_ms: u64,
    /// Session `statement_timeout` in milliseconds for writer connections
    /// (env `BUZZ_DB_STATEMENT_TIMEOUT_MS`). `0` disables it and is the
    /// default because migrations and backfills may legitimately run long.
    pub statement_timeout_ms: u64,
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
            lock_timeout_ms: DEFAULT_LOCK_TIMEOUT_MS,
            idle_txn_timeout_ms: DEFAULT_IDLE_TXN_TIMEOUT_MS,
            statement_timeout_ms: 0,
        }
    }
}

/// Default writer `lock_timeout` in milliseconds.
pub const DEFAULT_LOCK_TIMEOUT_MS: u64 = 5_000;

/// Default writer `idle_in_transaction_session_timeout` in milliseconds.
pub const DEFAULT_IDLE_TXN_TIMEOUT_MS: u64 = 60_000;

impl DbConfig {
    /// Overlay writer session timeouts from the shared `BUZZ_DB_*_TIMEOUT_MS`
    /// environment variables. Missing or invalid values retain the existing
    /// configuration; explicit zeroes pass through to disable a timeout.
    ///
    /// This belongs in `buzz-db` so relay, admin, deletion, and audit writers
    /// share one policy. The separately deployed push gateway owns its own
    /// database and session policy.
    pub fn with_session_timeouts_from_env(mut self) -> Self {
        fn parse(key: &str) -> Option<u64> {
            std::env::var(key)
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
        }

        if let Some(value) = parse("BUZZ_DB_LOCK_TIMEOUT_MS") {
            self.lock_timeout_ms = value;
        }
        if let Some(value) = parse("BUZZ_DB_IDLE_TXN_TIMEOUT_MS") {
            self.idle_txn_timeout_ms = value;
        }
        if let Some(value) = parse("BUZZ_DB_STATEMENT_TIMEOUT_MS") {
            self.statement_timeout_ms = value;
        }
        self
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
        let pool = Self::connect_writer_pool(config).await?;
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
    /// Additional writer pools, including the relay audit pool, must use this
    /// constructor so they inherit the timeout, floor-guard, and isolation
    /// policy installed by [`Db::new`].
    pub async fn connect_writer_pool(config: &DbConfig) -> Result<PgPool> {
        let lock_timeout_ms = config.lock_timeout_ms;
        let idle_txn_timeout_ms = config.idle_txn_timeout_ms;
        let statement_timeout_ms = config.statement_timeout_ms;
        let options = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(Duration::from_secs(config.acquire_timeout_secs))
            .max_lifetime(Duration::from_secs(config.max_lifetime_secs))
            .idle_timeout(Duration::from_secs(config.idle_timeout_secs))
            .after_connect(move |conn, _meta| {
                Box::pin(async move {
                    // `SET` cannot take bind parameters; `set_config` can.
                    sqlx::query("SELECT set_config('buzz.created_at_floor', $1, false)")
                        .bind(replica_fence::CREATED_AT_FLOOR_SECS.to_string())
                        .execute(&mut *conn)
                        .await?;
                    // `lock_timeout` fails the waiting statement; it does not
                    // cancel the holder. `idle_in_transaction_session_timeout`
                    // reaps only holders idling inside an open transaction,
                    // while actively executing holders are bounded only by
                    // `statement_timeout` (off by default). Bare values are
                    // milliseconds. Migration/schema-destruction connections
                    // reset lock and statement timeouts before their intentional
                    // long wait (see `with_exclusive_schema_destruction_lock`).
                    sqlx::query(
                        "SELECT set_config('lock_timeout', $1, false), \
                                set_config('idle_in_transaction_session_timeout', $2, false), \
                                set_config('statement_timeout', $3, false)",
                    )
                    .bind(lock_timeout_ms.to_string())
                    .bind(idle_txn_timeout_ms.to_string())
                    .bind(statement_timeout_ms.to_string())
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
        Ok(options.connect(&config.database_url).await?)
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
        tokio::spawn(Self::read_pool_boot_ping_once(read_pool, aurora_identity));
    }

    async fn read_pool_boot_ping_once(
        read_pool: PgPool,
        aurora_identity: std::sync::Arc<std::sync::OnceLock<bool>>,
    ) {
        match observability::acquire_reader_with_legacy_metrics(
            &read_pool,
            observability::ReaderOperation::Bootstrap,
        )
        .await
        {
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
    }

    #[cfg(test)]
    pub(crate) async fn read_pool_boot_ping_for_tests(&self) {
        let Some(read_pool) = self.read_pool.clone() else {
            return;
        };
        Self::read_pool_boot_ping_once(read_pool, self.reader_aurora_identity.clone()).await;
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
        self.verify_replica_fence_at_boot().await?;
        tokio::spawn(replica_fence::run_probe(
            self.pool.clone(),
            std::sync::Arc::clone(&self.fence),
        ));
        Ok(true)
    }

    /// Verify replica-fence catalog shape and behavior through attributed
    /// writer/bootstrap acquisitions without starting the recurring probe.
    pub(crate) async fn verify_replica_fence_at_boot(&self) -> Result<()> {
        let mut connection =
            observability::acquire_writer(&self.pool, observability::WriterOperation::Bootstrap)
                .await?;
        replica_fence::verify_floor_guard_catalog(&mut *connection).await?;
        drop(connection);
        replica_fence::verify_floor_guard_behavior(&self.pool).await
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
        operation: observability::ReaderOperation,
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
        let conn = match observability::acquire_reader_with_legacy_metrics(read_pool, operation)
            .await
        {
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
    pub(crate) fn record_route(path: &'static str, decision: &'static str, reason: &'static str) {
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

    /// Returns `true` if the database is reachable.
    pub async fn ping(&self) -> bool {
        let Ok(mut connection) =
            observability::acquire_writer(&self.pool, observability::WriterOperation::Readiness)
                .await
        else {
            return false;
        };
        sqlx::query("SELECT 1")
            .execute(&mut *connection)
            .await
            .is_ok()
    }

    /// Checks writer-pool acquisition and query execution against one deadline.
    ///
    /// Unlike [`Self::ping`], this preserves whether readiness was blocked while
    /// borrowing a connection or failed after a connection had been acquired.
    /// The query runs on the already-acquired connection so the two phases
    /// cannot be collapsed into a second implicit pool acquisition.
    pub async fn readiness_check(&self, deadline: tokio::time::Instant) -> DbReadinessOutcome {
        self.readiness_check_sql(deadline, "SELECT 1").await
    }

    /// Production-bound seam for classifying failures after pool acquisition.
    /// Tests vary only the SQL so timeout/error/cancellation paths execute the
    /// same acquisition and classification code as [`Self::readiness_check`].
    async fn readiness_check_sql(
        &self,
        deadline: tokio::time::Instant,
        query: &'static str,
    ) -> DbReadinessOutcome {
        let mut connection = match observability::acquire_writer_until(
            &self.pool,
            observability::WriterOperation::Readiness,
            deadline,
        )
        .await
        {
            Err(sqlx::Error::PoolTimedOut) => return DbReadinessOutcome::PoolTimeout,
            Err(error) => {
                tracing::debug!(error = %error, "Postgres readiness pool acquisition failed");
                return DbReadinessOutcome::PoolError;
            }
            Ok(connection) => connection,
        };

        match tokio::time::timeout_at(deadline, sqlx::query(query).execute(&mut *connection)).await
        {
            Err(_) => DbReadinessOutcome::QueryTimeout,
            Ok(Err(error)) => {
                tracing::debug!(error = %error, "Postgres readiness query failed");
                DbReadinessOutcome::QueryError
            }
            Ok(Ok(_)) => DbReadinessOutcome::Success,
        }
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

    /// Refresh all expected operation-specific waiter gauges, including zero.
    ///
    /// The relay pool sampler calls this periodically so an exporter idle
    /// timeout cannot make a healthy zero indistinguishable from missing
    /// telemetry.
    pub fn refresh_pool_waiter_metrics(&self) {
        observability::refresh_pool_waiters(self.read_pool.is_some());
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

    /// Begin a database transaction for atomic multi-statement operations.
    ///
    /// Returns a `'static` transaction because `PgPool` is `Arc`-backed internally.
    /// The transaction holds an owned pool handle, not a borrow.
    pub async fn begin_event_write_transaction(
        &self,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        let connection = observability::acquire_writer_with_legacy_metrics(
            &self.pool,
            observability::WriterOperation::EventWrite,
        )
        .await?;
        sqlx::Transaction::begin(connection, None)
            .await
            .map_err(Into::into)
    }

    /// Begin an event-write transaction through the pre-operation API name.
    ///
    /// New callers should use [`Self::begin_event_write_transaction`] so the
    /// semantic intent is explicit. This alias preserves the crate's public
    /// API while emitting the same operation-aware and compatibility metrics.
    #[deprecated(note = "use Db::begin_event_write_transaction")]
    pub async fn begin_transaction(&self) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
        self.begin_event_write_transaction().await
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

        let connection =
            observability::acquire_writer(&self.pool, observability::WriterOperation::EventWrite)
                .await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
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
    pub(crate) async fn route_read(
        &self,
        path: &'static str,
        predicate: RoutePredicate,
        operation: observability::ReaderOperation,
    ) -> RouteDecision {
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
        match self.proved_reader(read_pool, operation).await {
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
}

#[cfg(test)]
#[path = "tests.rs"]
mod postgres_tests;
