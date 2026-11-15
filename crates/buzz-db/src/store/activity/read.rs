//! The list: one read of a person's conversations, threads and mentions.

use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgRow, Connection as _, PgConnection, Row as _};
use uuid::Uuid;

use super::{fill, model::*};
use crate::{
    observability,
    store::{channel::legacy_channel_type_sql, personal_read::ELIGIBLE_KINDS},
    Db, DbError, Result,
};

/// Most rows one read may name.
const MAX_NAMED: usize = 200;

/// The person's unread mentions, from their newest mentions since their
/// floor. Stops counting one past the largest total reported.
const UNREAD_MENTIONS: &str = r#"SELECT count(*) FROM (
    SELECT 1
    FROM (
        SELECT event_id, event_created_at, channel_id, root_id, received_at
        FROM event_mentions
        WHERE community_id = $1 AND pubkey_hex = $3
            AND received_at > GREATEST(COALESCE($4::timestamptz, 'infinity'), $6::timestamptz)
            AND channel_id IS NOT NULL AND event_kind = ANY($5)
        ORDER BY received_at DESC, event_id LIMIT $7
    ) m
    JOIN channel_members cm ON cm.community_id = $1 AND cm.channel_id = m.channel_id
        AND cm.pubkey = $2 AND cm.removed_at IS NULL
    JOIN channels c ON c.community_id = $1 AND c.id = m.channel_id AND c.deleted_at IS NULL
    JOIN events e ON e.community_id = $1 AND e.created_at = m.event_created_at
        AND e.id = m.event_id
    LEFT JOIN personal_read_frontiers f ON f.community_id = $1 AND f.actor = $2
        AND f.channel_id = m.channel_id AND f.root_id = COALESCE(m.root_id, ''::bytea)
    WHERE m.received_at > COALESCE(f.through_timestamp, '-infinity')
        AND e.deleted_at IS NULL AND e.pubkey <> $2
    LIMIT $18
) unread"#;

/// The person's joined conversations, one row each. A conversation sorts by
/// the arrival of the message it shows, so a reply inside a thread does not
/// move it.
fn conversations_sql() -> String {
    format!(
        r#"SELECT CASE WHEN c.channel_type::text = 'dm' THEN 'dm'
                    WHEN 'buzz.solo' = ANY(c.labels) THEN 'solo' ELSE 'channel' END AS kind,
                c.id::text AS id, c.id AS channel_id, NULL::bytea AS root_id,
                COALESCE(h.head_received_at, c.created_at) AS sort_at,
                CASE WHEN e.id IS NOT NULL THEN h.head_id END AS preview_id,
                h.head_pubkey AS preview_pubkey, h.head_created_at AS preview_created_at,
                h.head_text AS preview_text,
                COALESCE(f.state, 'default') <> 'muted' AND (COALESCE(f.marked_unread, false)
                    OR (e.id IS NOT NULL AND h.head_pubkey <> $2 AND h.head_received_at >
                        GREATEST(COALESCE(f.through_timestamp, '-infinity'),
                            COALESCE($4::timestamptz, 'infinity')))) AS unread,
                COALESCE(f.state, 'default') AS state
            FROM channel_members cm
            JOIN channels c ON c.community_id = cm.community_id AND c.id = cm.channel_id
            LEFT JOIN channel_heads h ON h.community_id = c.community_id AND h.channel_id = c.id
            LEFT JOIN events e ON e.community_id = h.community_id
                AND e.created_at = h.head_created_at AND e.id = h.head_id
                AND e.deleted_at IS NULL
            LEFT JOIN personal_read_frontiers f ON f.community_id = $1 AND f.actor = $2
                AND f.channel_id = c.id AND f.root_id = ''::bytea
            WHERE cm.community_id = $1 AND cm.pubkey = $2 AND cm.removed_at IS NULL
                AND c.deleted_at IS NULL AND {legacy}
                AND ($16::uuid[] IS NULL OR c.id = ANY($16))"#,
        legacy = legacy_channel_type_sql("c.channel_type"),
    )
}

/// Threads the person follows, muted, flagged, or was part of within the
/// horizon, in channels they are still in. The horizon bounds the list by
/// recent activity rather than by a person's whole history.
const THREADS: &str = r#"SELECT 'thread' AS kind, encode(f.root_id, 'hex') AS id,
        f.channel_id, f.root_id, h.activity_at AS sort_at,
        CASE WHEN e.id IS NOT NULL THEN h.activity_id END AS preview_id,
        h.activity_pubkey AS preview_pubkey, h.activity_created_at AS preview_created_at,
        h.activity_text AS preview_text,
        f.state <> 'muted' AND (f.marked_unread
            OR ((f.state = 'following' OR f.engaged_at >= $8::timestamptz - make_interval(secs => $9))
                AND e.id IS NOT NULL AND h.activity_pubkey <> $2 AND h.activity_at >
                    GREATEST(COALESCE(f.through_timestamp, '-infinity'),
                        COALESCE($4::timestamptz, 'infinity')))) AS unread,
        f.state
    FROM personal_read_frontiers f
    JOIN channel_members cm ON cm.community_id = $1 AND cm.channel_id = f.channel_id
        AND cm.pubkey = $2 AND cm.removed_at IS NULL
    JOIN channels c ON c.community_id = $1 AND c.id = f.channel_id AND c.deleted_at IS NULL
    JOIN thread_heads h ON h.community_id = $1 AND h.root_id = f.root_id
    LEFT JOIN events e ON e.community_id = $1 AND e.created_at = h.activity_created_at
        AND e.id = h.activity_id AND e.deleted_at IS NULL
    WHERE f.community_id = $1 AND f.actor = $2 AND f.root_id <> ''::bytea
        AND h.activity_at IS NOT NULL
        AND (f.state <> 'default' OR f.marked_unread
            OR f.engaged_at >= $8::timestamptz - make_interval(secs => $9))
        AND ($17::bytea[] IS NULL OR f.root_id = ANY($17))"#;

/// Messages that tag the person, in channels they are still in. A row's
/// unread follows the read position of the channel or thread it is in.
///
/// A `since` read passes the channels in which the person's read position
/// moved (`$16`) and those they joined (`$21`): it returns the mentions that
/// arrived since the marker, those in a scope whose read position moved,
/// since that changes whether they are unread, and those of a joined channel,
/// which the client does not hold.
const MENTIONS: &str = r#"SELECT 'mention' AS kind, encode(m.event_id, 'hex') AS id,
        m.channel_id, m.root_id, m.received_at AS sort_at,
        m.event_id AS preview_id, e.pubkey AS preview_pubkey,
        e.created_at AS preview_created_at, left(e.content, $19) AS preview_text,
        m.received_at > GREATEST(COALESCE(f.through_timestamp, '-infinity'),
            COALESCE($4::timestamptz, 'infinity')) AS unread,
        COALESCE(f.state, 'default') AS state
    FROM event_mentions m
    JOIN channel_members cm ON cm.community_id = $1 AND cm.channel_id = m.channel_id
        AND cm.pubkey = $2 AND cm.removed_at IS NULL
    JOIN channels c ON c.community_id = $1 AND c.id = m.channel_id AND c.deleted_at IS NULL
    JOIN events e ON e.community_id = $1 AND e.created_at = m.event_created_at
        AND e.id = m.event_id AND e.deleted_at IS NULL AND e.pubkey <> $2
    LEFT JOIN personal_read_frontiers f ON f.community_id = $1 AND f.actor = $2
        AND f.channel_id = m.channel_id AND f.root_id = COALESCE(m.root_id, ''::bytea)
    WHERE m.community_id = $1 AND m.pubkey_hex = $3 AND m.received_at > $20::timestamptz
        AND m.channel_id IS NOT NULL AND m.event_kind = ANY($5)
        AND ($16::uuid[] IS NULL OR m.received_at >= $15 OR m.channel_id = ANY($21)
            OR (m.channel_id = ANY($16)
                AND COALESCE(f.seq > $14 OR f.through_timestamp >= $15, false)))"#;

/// The conversations and threads of the person's list that changed since a
/// marker: `(channel, thread root or '', whether their read position moved)`.
///
/// It walks the person's own memberships and thread rows and looks each
/// shared row up by its key, so its cost follows the size of the person's
/// list, whatever the marker's age and however busy the community is. A
/// conversation changed when the person joined it, its shared row changed, or
/// the person changed their own row; a thread, the last two. A post moves its
/// author's read position without their counter, so the position's own time
/// is compared too.
const CHANGED: &str = r#"(SELECT cm.channel_id, ''::bytea AS root_id,
        COALESCE(f.seq > $3 OR f.through_timestamp >= $4, false) AS read_moved,
        cm.sync_modified_at >= $4 AS joined
    FROM channel_members cm
    LEFT JOIN channel_heads h ON h.community_id = $1 AND h.channel_id = cm.channel_id
    LEFT JOIN personal_read_frontiers f ON f.community_id = $1 AND f.actor = $2
        AND f.channel_id = cm.channel_id AND f.root_id = ''::bytea
    WHERE cm.community_id = $1 AND cm.pubkey = $2 AND cm.removed_at IS NULL
        AND (cm.sync_modified_at >= $4 OR h.updated_at >= $4 OR f.seq > $3)
    LIMIT $5)
    UNION ALL
    (SELECT f.channel_id, f.root_id,
        f.seq > $3 OR COALESCE(f.through_timestamp >= $4, false) AS read_moved,
        false AS joined
    FROM personal_read_frontiers f
    LEFT JOIN thread_heads h ON h.community_id = $1 AND h.root_id = f.root_id
    WHERE f.community_id = $1 AND f.actor = $2 AND f.root_id <> ''::bytea
        AND (f.seq > $3 OR h.updated_at >= $4)
    LIMIT $5)"#;

/// The person's thread rows in channels they joined since the marker. A
/// client dropped them when the person left, and a first join can find
/// threads the person was tagged into before.
const JOINED_THREADS: &str = r#"SELECT f.root_id FROM personal_read_frontiers f
    WHERE f.community_id = $1 AND f.actor = $2 AND f.channel_id = ANY($3)
        AND f.root_id <> ''::bytea
    LIMIT $4"#;

/// Wrap a group's rows with the read's filter, order and limit. `rank` is
/// the group's place in the sort order, for the cursor.
fn select_sql(base: &str, rank: u8) -> String {
    format!(
        r#"SELECT q.* FROM ({base}) q
        WHERE q.kind = ANY($10)
            AND ($11::timestamptz IS NULL OR q.sort_at < $11
                OR (q.sort_at = $11 AND ({rank} > $12
                    OR ({rank} = $12 AND q.id COLLATE "C" > $13))))
        ORDER BY q.sort_at DESC, q.id COLLATE "C" LIMIT $18"#
    )
}

fn totals_sql(base: &str) -> String {
    format!(
        "SELECT q.kind, count(*) FILTER (WHERE q.unread) AS unread FROM ({base}) q GROUP BY q.kind"
    )
}

/// What every query of one read binds, in `$n` order.
struct Binds<'a> {
    community: Uuid,
    actor: &'a [u8; 32],
    actor_hex: &'a str,
    snapshot: Snapshot,
    /// Mentions that arrived before this are not listed or counted.
    mention_floor: DateTime<Utc>,
    kinds: Vec<&'static str>,
    cursor: Option<(DateTime<Utc>, i32, &'a str)>,
    since: Option<(i64, DateTime<Utc>)>,
}

/// What one query of a read adds to [`Binds`].
#[derive(Default)]
struct Scope<'q> {
    /// Only these conversations; for mentions, the channels in which the
    /// person's read position moved.
    channels: Option<&'q [Uuid]>,
    /// Only these threads.
    roots: Option<&'q [Vec<u8>]>,
    limit: Option<i64>,
    /// Mentions are scanned from here when it is later than the floor.
    mentions_from: Option<DateTime<Utc>>,
    /// For mentions, the channels the person joined since the marker.
    joined: Option<&'q [Uuid]>,
}

impl Binds<'_> {
    fn bind<'q>(
        &'q self,
        sql: String,
        scope: Scope<'q>,
    ) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(self.community)
            .bind(self.actor.as_slice())
            .bind(self.actor_hex)
            .bind(self.snapshot.started_at)
            .bind(ELIGIBLE_KINDS.to_vec())
            .bind(self.mention_floor)
            .bind(MENTION_SCAN)
            .bind(self.snapshot.now)
            .bind(HORIZON_SECONDS as f64)
            .bind(&self.kinds)
            .bind(self.cursor.map(|c| c.0))
            .bind(self.cursor.map(|c| c.1))
            .bind(self.cursor.map(|c| c.2))
            .bind(self.since.map(|s| s.0))
            .bind(self.since.map(|s| s.1))
            .bind(scope.channels)
            .bind(scope.roots)
            .bind(scope.limit)
            .bind(PREVIEW_CHARS as i32)
            .bind(
                scope
                    .mentions_from
                    .map_or(self.mention_floor, |from| from.max(self.mention_floor)),
            )
            .bind(scope.joined.unwrap_or_default())
    }
}

fn kind_of(name: &str) -> Result<RowKind> {
    RowKind::parse(name).ok_or_else(|| DbError::InvalidData("unknown row kind".into()))
}

fn row(row: &PgRow) -> Result<Row> {
    let kind = kind_of(row.try_get("kind")?)?;
    let sort_at: DateTime<Utc> = row.try_get("sort_at")?;
    let hex = |column: &str| -> Result<Option<String>> {
        Ok(row.try_get::<Option<Vec<u8>>, _>(column)?.map(hex::encode))
    };
    let preview = match hex("preview_id")? {
        Some(id) => Some(Preview {
            id,
            pubkey: hex("preview_pubkey")?.unwrap_or_default(),
            created_at: row
                .try_get::<DateTime<Utc>, _>("preview_created_at")?
                .timestamp(),
            text: row
                .try_get::<Option<String>, _>("preview_text")?
                .unwrap_or_default(),
        }),
        None => None,
    };
    let state = match row.try_get::<&str, _>("state")? {
        "following" => "following",
        "muted" => "muted",
        _ => "default",
    };
    Ok(Row {
        kind,
        id: row.try_get("id")?,
        channel_id: row.try_get("channel_id")?,
        root_id: hex("root_id")?,
        at: sort_at.timestamp(),
        preview,
        unread: row.try_get("unread")?,
        state,
        sort_at: sort_at.timestamp_micros(),
    })
}

fn count(value: i64, most: u32) -> u32 {
    value.clamp(0, i64::from(most)) as u32
}

fn micros(value: i64) -> Result<DateTime<Utc>> {
    DateTime::from_timestamp_micros(value).ok_or(DbError::InvalidTimestamp(value))
}

fn reset() -> Page {
    Page {
        reset: true,
        ..Page::default()
    }
}

/// What a `since` read found changed.
#[derive(Default)]
struct Changed {
    channels: Vec<Uuid>,
    roots: Vec<Vec<u8>>,
    /// Channels in which the person's read position moved, in the channel's
    /// timeline or one of its threads.
    read_moved: Vec<Uuid>,
    /// Channels the person joined, whose threads and mentions are all new
    /// to the client.
    joined: Vec<Uuid>,
}

impl Changed {
    fn is_empty(&self) -> bool {
        self.channels.is_empty() && self.roots.is_empty()
    }
}

/// `None` when more changed than a `since` read returns.
async fn changed(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8; 32],
    (seq, since): (i64, DateTime<Utc>),
) -> Result<Option<Changed>> {
    let rows = sqlx::query(CHANGED)
        .bind(community.as_uuid())
        .bind(actor.as_slice())
        .bind(seq)
        .bind(since)
        .bind(MAX_SINCE_ROWS as i64 + 1)
        .fetch_all(&mut *conn)
        .await?;
    let mut changed = Changed::default();
    let mut read_moved = std::collections::BTreeSet::new();
    for row in rows {
        let channel: Uuid = row.try_get("channel_id")?;
        let root: Vec<u8> = row.try_get("root_id")?;
        if row.try_get("read_moved")? {
            read_moved.insert(channel);
        }
        if row.try_get("joined")? {
            changed.joined.push(channel);
        }
        if root.is_empty() {
            changed.channels.push(channel);
        } else {
            changed.roots.push(root);
        }
    }
    if !changed.joined.is_empty() {
        let rows = sqlx::query(JOINED_THREADS)
            .bind(community.as_uuid())
            .bind(actor.as_slice())
            .bind(&changed.joined)
            .bind(MAX_SINCE_ROWS as i64 + 1)
            .fetch_all(&mut *conn)
            .await?;
        for row in rows {
            let root: Vec<u8> = row.try_get("root_id")?;
            if !changed.roots.contains(&root) {
                changed.roots.push(root);
            }
        }
    }
    if changed.channels.len().max(changed.roots.len()) > MAX_SINCE_ROWS {
        return Ok(None);
    }
    changed.read_moved = read_moved.into_iter().collect();
    Ok(Some(changed))
}

impl Db {
    /// One read of the person's activity list.
    ///
    /// The first read starts the person's account: nothing stored before it
    /// is unread. First pages, `since` reads and reads of named rows first
    /// rebuild shared rows that are behind the event log. Callers must
    /// recheck admission and membership before releasing rows.
    pub async fn activity(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        retention_seconds: u32,
        query: &Query,
    ) -> Result<Page> {
        let invalid = |what: &str| Err(DbError::InvalidData(what.into()));
        match &query.select {
            Select::Page { limit, .. } if !(1..=MAX_LIMIT).contains(limit) => {
                return invalid("invalid limit")
            }
            Select::Ids { channels, roots }
                if channels.len() + roots.len() > MAX_NAMED
                    || (channels.is_empty() && roots.is_empty()) =>
            {
                return invalid("invalid id list")
            }
            _ => {}
        }
        let actor_bytes = actor.to_bytes();
        let actor_hex = hex::encode(actor_bytes);
        let named = match &query.select {
            Select::Ids { channels, roots } => Some((
                channels.clone(),
                roots.iter().map(|id| id.to_vec()).collect::<Vec<_>>(),
            )),
            _ => None,
        };
        let first = !matches!(
            &query.select,
            Select::Page {
                before: Some(_),
                ..
            }
        );
        if first {
            self.start_account(community, &actor_bytes).await?;
            let wanting = {
                let mut conn = observability::acquire_writer(
                    &self.pool,
                    observability::WriterOperation::SubscriptionHistory,
                )
                .await?;
                // Bounded like the read itself, so a request that gives up
                // does not leave the check running.
                let mut tx = conn.begin().await?;
                sqlx::raw_sql("SET LOCAL statement_timeout = '3000ms'; SET LOCAL jit = off")
                    .execute(&mut *tx)
                    .await?;
                fill::wanting(
                    &mut tx,
                    community,
                    &actor_bytes,
                    named.as_ref().map(|named| named.0.as_slice()),
                    named.as_ref().map(|named| named.1.as_slice()),
                )
                .await?
            };
            if !wanting.is_empty() {
                self.repair_activity(community, &wanting).await;
            }
        }

        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        // One snapshot for every query, so the marker, the rows and the
        // totals describe the same moment.
        let mut tx = conn.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql("SET LOCAL statement_timeout = '5000ms'; SET LOCAL jit = off")
            .execute(&mut *tx)
            .await?;
        let snapshot = snapshot(&mut tx, community, &actor_bytes).await?;
        let marker = Marker {
            seq: snapshot.seq,
            micros: snapshot.now.timestamp_micros(),
        };
        let horizon = snapshot.now - chrono::Duration::seconds(HORIZON_SECONDS);
        let since = match &query.select {
            Select::Since(since) => {
                let at = micros(since.micros)?;
                if at < horizon || since.seq > snapshot.seq {
                    return Ok(reset());
                }
                Some((
                    since.seq,
                    at - chrono::Duration::seconds(COMMIT_SKEW_SECONDS),
                ))
            }
            _ => None,
        };
        // A `since` read first finds what changed, then reads only those rows.
        let changed = match since {
            Some(since) => match changed(&mut tx, community, &actor_bytes, since).await? {
                Some(changed) => Some(changed),
                None => return Ok(reset()),
            },
            None => None,
        };
        let (cursor, limit) = match &query.select {
            Select::Page { before, limit } => (before.as_ref(), Some(*limit)),
            Select::Since(_) => (None, Some(MAX_SINCE_ROWS)),
            Select::Ids { .. } => (None, None),
        };
        let cursor = match cursor {
            Some(cursor) => Some((
                micros(cursor.micros)?,
                i32::from(cursor.rank),
                cursor.id.as_str(),
            )),
            None => None,
        };
        let listing = matches!(query.select, Select::Page { .. });
        let binds = Binds {
            community: *community.as_uuid(),
            actor: &actor_bytes,
            actor_hex: &actor_hex,
            snapshot,
            mention_floor: snapshot.now - chrono::Duration::seconds(i64::from(retention_seconds)),
            kinds: query.kinds.iter().map(|kind| kind.as_str()).collect(),
            cursor,
            since,
        };
        let wants = |rank: u8| query.kinds.iter().any(|kind| kind.rank() == rank);
        let conversations = conversations_sql();
        // The rows each group is restricted to; `None` is the whole group.
        let (channels, roots) = match (&changed, &named) {
            (Some(changed), _) => (
                Some(changed.channels.as_slice()),
                Some(changed.roots.as_slice()),
            ),
            (None, Some((channels, roots))) => (Some(channels.as_slice()), Some(roots.as_slice())),
            (None, None) => (None, None),
        };
        let row_limit = limit.map(|limit| limit as i64 + 1);
        let groups: [(u8, &str, Option<Scope<'_>>); 3] = [
            (
                0,
                &conversations,
                channels.is_none_or(|ids| !ids.is_empty()).then(|| Scope {
                    channels,
                    limit: row_limit,
                    ..Scope::default()
                }),
            ),
            (
                1,
                THREADS,
                roots.is_none_or(|ids| !ids.is_empty()).then(|| Scope {
                    roots,
                    limit: row_limit,
                    ..Scope::default()
                }),
            ),
            (
                2,
                MENTIONS,
                // A read of named rows names no mention.
                named.is_none().then(|| Scope {
                    channels: changed
                        .as_ref()
                        .map(|changed| changed.read_moved.as_slice()),
                    limit: row_limit,
                    // With no read position moved and no channel joined,
                    // only new mentions matter.
                    mentions_from: match (&changed, since) {
                        (Some(changed), Some((_, at)))
                            if changed.read_moved.is_empty() && changed.joined.is_empty() =>
                        {
                            Some(at)
                        }
                        _ => None,
                    },
                    joined: changed.as_ref().map(|changed| changed.joined.as_slice()),
                    ..Scope::default()
                }),
            ),
        ];
        let mut rows = Vec::new();
        let mut more = false;
        for (rank, base, scope) in groups {
            let Some(scope) = scope.filter(|_| wants(rank)) else {
                continue;
            };
            let found = binds
                .bind(select_sql(base, rank), scope)
                .fetch_all(&mut *tx)
                .await?;
            more |= limit.is_some_and(|limit| found.len() > limit);
            for found in &found {
                rows.push(row(found)?);
            }
        }
        if since.is_some() && more {
            return Ok(reset());
        }
        rows.sort_by(|a, b| {
            (b.sort_at, a.kind.rank(), &a.id).cmp(&(a.sort_at, b.kind.rank(), &b.id))
        });
        let mut page = Page {
            marker: first.then_some(marker),
            ..Page::default()
        };
        if let (true, Some(limit)) = (listing, limit) {
            more |= rows.len() > limit;
            rows.truncate(limit);
            page.next = match (more, rows.last()) {
                (true, Some(last)) => Some(last.cursor()),
                _ => None,
            };
        }
        page.rows = rows;
        if let Some((seq, since)) = since {
            page.removed = removed(&mut tx, community, &actor_bytes, since).await?;
            page.removed.extend(
                removed_threads(&mut tx, community, &actor_bytes, seq, snapshot.now).await?,
            );
            // More removals than one read returns: the client reads again.
            if page.removed.len() > MAX_SINCE_ROWS {
                return Ok(reset());
            }
        }
        // A `since` read that found nothing has nothing new to count. One
        // that asked for only some kinds cannot tell, so it counts.
        let counted = match query.select {
            Select::Page { .. } => first,
            Select::Since(_) => {
                !page.rows.is_empty()
                    || !page.removed.is_empty()
                    || changed.as_ref().is_some_and(|changed| !changed.is_empty())
                    || !wants(2)
            }
            Select::Ids { .. } => false,
        };
        if counted {
            page.totals = Some(totals(&mut tx, &binds, &conversations).await?);
        }
        tx.commit().await?;
        Ok(page)
    }

    /// Start the person's account on their first read. Reading the list is
    /// what turns unread tracking on, so nothing older is ever unread.
    async fn start_account(&self, community: CommunityId, actor: &[u8; 32]) -> Result<()> {
        let started: Option<Option<DateTime<Utc>>> = sqlx::query_scalar(
            "SELECT started_at FROM personal_read_accounts WHERE community_id=$1 AND actor=$2",
        )
        .bind(community.as_uuid())
        .bind(actor.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        if started.flatten().is_some() {
            return Ok(());
        }
        let mut tx = crate::begin_community_write_transaction(
            &self.pool,
            community,
            observability::WriterOperation::EventWrite,
        )
        .await?;
        crate::store::personal_read::deadlines(tx.conn()).await?;
        sqlx::query(
            "INSERT INTO personal_read_accounts AS a (community_id,actor,started_at)
             VALUES ($1,$2,now())
             ON CONFLICT (community_id,actor) DO UPDATE SET started_at=now()
             WHERE a.started_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(actor.as_slice())
        .execute(tx.conn())
        .await?;
        tx.commit().await
    }
}

async fn snapshot(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8; 32],
) -> Result<Snapshot> {
    let row = sqlx::query(
        "SELECT now() AS now, a.started_at, COALESCE(a.seq, 0) AS seq
         FROM (SELECT 1) one LEFT JOIN personal_read_accounts a
            ON a.community_id=$1 AND a.actor=$2",
    )
    .bind(community.as_uuid())
    .bind(actor.as_slice())
    .fetch_one(&mut *conn)
    .await?;
    Ok(Snapshot {
        started_at: row.try_get("started_at")?,
        seq: row.try_get("seq")?,
        now: row.try_get("now")?,
    })
}

async fn totals(conn: &mut PgConnection, binds: &Binds<'_>, conversations: &str) -> Result<Totals> {
    let mut totals = Totals::default();
    for base in [conversations, THREADS] {
        let rows = binds
            .bind(totals_sql(base), Scope::default())
            .fetch_all(&mut *conn)
            .await?;
        for row in rows {
            let unread = count(row.try_get("unread")?, u32::MAX);
            match kind_of(row.try_get("kind")?)? {
                RowKind::Channel => totals.channel = unread,
                RowKind::Dm => totals.dm = unread,
                RowKind::Solo => totals.solo = unread,
                RowKind::Thread => totals.thread = unread,
                RowKind::Mention => {}
            }
        }
    }
    let scope = Scope {
        limit: Some(i64::from(MAX_MENTIONS) + 1),
        ..Scope::default()
    };
    let unread: i64 = binds
        .bind(UNREAD_MENTIONS.to_owned(), scope)
        .fetch_one(&mut *conn)
        .await?
        .try_get(0)?;
    totals.mention = count(unread, MAX_MENTIONS);
    Ok(totals)
}

/// Conversations the person left, or that were deleted, since `since`. Their
/// thread and mention rows go with them: a client drops rows by channel.
async fn removed(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8; 32],
    since: DateTime<Utc>,
) -> Result<Vec<Removed>> {
    let rows = sqlx::query(
        "SELECT c.id, c.channel_type::text AS channel_type, 'buzz.solo' = ANY(c.labels) AS solo
         FROM channel_members cm
         JOIN channels c ON c.community_id = cm.community_id AND c.id = cm.channel_id
         WHERE cm.community_id = $1 AND cm.pubkey = $2
            AND ((cm.removed_at IS NOT NULL AND cm.sync_modified_at >= $3)
                OR c.deleted_at >= $3)
         ORDER BY c.id LIMIT $4",
    )
    .bind(community.as_uuid())
    .bind(actor.as_slice())
    .bind(since)
    .bind(MAX_SINCE_ROWS as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            let kind = match (
                row.try_get::<&str, _>("channel_type")?,
                row.try_get("solo")?,
            ) {
                ("dm", _) => RowKind::Dm,
                (_, true) => RowKind::Solo,
                _ => RowKind::Channel,
            };
            Ok(Removed {
                kind,
                id: row.try_get::<Uuid, _>("id")?.to_string(),
            })
        })
        .collect()
}

/// Threads the person's own change since `seq` took off the list: a state set
/// back to `default`, or a flag cleared, on a thread they are not otherwise
/// part of. A thread that only aged out is not reported.
async fn removed_threads(
    conn: &mut PgConnection,
    community: CommunityId,
    actor: &[u8; 32],
    seq: i64,
    now: DateTime<Utc>,
) -> Result<Vec<Removed>> {
    let rows = sqlx::query(
        "SELECT encode(f.root_id, 'hex') AS id FROM personal_read_frontiers f
         WHERE f.community_id = $1 AND f.actor = $2 AND f.seq > $3 AND f.root_id <> ''::bytea
            AND NOT (f.state <> 'default' OR f.marked_unread
                OR COALESCE(f.engaged_at >= $4::timestamptz - make_interval(secs => $5), false))
         ORDER BY f.seq LIMIT $6",
    )
    .bind(community.as_uuid())
    .bind(actor.as_slice())
    .bind(seq)
    .bind(now)
    .bind(HORIZON_SECONDS as f64)
    .bind(MAX_SINCE_ROWS as i64 + 1)
    .fetch_all(&mut *conn)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Removed {
                kind: RowKind::Thread,
                id: row.try_get("id")?,
            })
        })
        .collect()
}
