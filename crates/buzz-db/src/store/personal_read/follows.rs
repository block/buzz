//! Read state that ingest writes. A thread frontier row exists for a thread
//! its actor follows or has unfollowed; ingest and follow intents create one.
use super::model::{DEFAULT_RETENTION_SECONDS, ELIGIBLE_KINDS};
use crate::{AdmittedTx, Result};
use chrono::{DateTime, Utc};
use nostr::Event;
use std::sync::atomic::{AtomicU32, Ordering};
use uuid::Uuid;

/// The relay's follow dormancy window, in seconds (`BUZZ_V1_RETENTION_SECONDS`).
static FOLLOW_WINDOW_SECONDS: AtomicU32 = AtomicU32::new(DEFAULT_RETENTION_SECONDS);

/// Set the follow dormancy window ingest refreshes against: the same window
/// the read projections are given. Process-wide; the relay sets it once at
/// startup.
pub fn set_follow_window_seconds(seconds: u32) {
    FOLLOW_WINDOW_SECONDS.store(seconds, Ordering::Relaxed);
}

/// How stale a live follow's involvement may grow before an involvement that
/// does not otherwise write its row refreshes it: a day, or a quarter of the
/// window when that is shorter. Never longer than the window, so such an
/// involvement always revives a dormant follow.
fn refresh_after_seconds() -> f64 {
    f64::from((FOLLOW_WINDOW_SECONDS.load(Ordering::Relaxed) / 4).min(24 * 60 * 60))
}

/// Where a newly stored channel message sits.
#[derive(Clone, Copy)]
pub(crate) enum Place<'a> {
    /// A top-level timeline message.
    TopLevel,
    /// A reply in the thread rooted at `root`, authored at `root_created_at`;
    /// a broadcast reply is also on the channel timeline.
    Reply {
        root: &'a [u8],
        root_created_at: DateTime<Utc>,
        broadcast: bool,
    },
}

impl Place<'_> {
    /// Whether the message counts on its channel timeline (`read_timeline`).
    pub(crate) fn on_timeline(&self) -> bool {
        matches!(
            self,
            Place::TopLevel
                | Place::Reply {
                    broadcast: true,
                    ..
                }
        )
    }
}

/// A reply's recorded ancestry: its root, the root's author time, and whether
/// it is a broadcast.
pub(crate) type Ancestry<'a> = (&'a [u8], Option<DateTime<Utc>>, bool);

/// Where an eligible channel message sits, from its recorded ancestry. None
/// for other events, and for a reply whose ancestry was never (fully)
/// recorded: it is on neither read path.
pub(crate) fn place<'a>(
    event: &Event,
    channel: Option<Uuid>,
    reply: Option<Ancestry<'a>>,
) -> Option<(Uuid, Place<'a>)> {
    let channel = channel?;
    if !ELIGIBLE_KINDS.contains(&i32::from(event.kind.as_u16())) {
        return None;
    }
    match reply {
        Some((root, root_created_at, broadcast)) => Some((
            channel,
            Place::Reply {
                root,
                root_created_at: root_created_at?,
                broadcast,
            },
        )),
        None if buzz_core::nip10::parse_thread_markers(&event.tags)
            .resolve()
            .is_some() =>
        {
            None
        }
        None => Some((channel, Place::TopLevel)),
    }
}

/// Record what a newly stored eligible message changes, in its transaction:
/// - its author has read through it: the timeline for a top-level message,
///   otherwise the thread, which the author then follows (and the timeline
///   too for a broadcast reply);
/// - the author of a top-level message follows the thread rooted at it, read
///   through the root;
/// - each channel member it mentions follows its thread (for a top-level
///   message, the thread rooted at it);
/// - a reply makes the root's author, and in a DM every member, follow too.
///
/// Everyone but the author starts just before the message, so it is unread
/// to them. A follower keeps an existing position. An unfollow sticks until
/// its actor posts in the thread or is mentioned there again.
///
/// Each of these involves its actor in the thread (`active_at`), which keeps
/// an implicit follow live. Agent accounts keep no read state.
pub(crate) async fn record_message(
    tx: &mut AdmittedTx,
    event: &Event,
    channel: Uuid,
    place: Place<'_>,
    received_at: DateTime<Utc>,
) -> Result<()> {
    if !ELIGIBLE_KINDS.contains(&i32::from(event.kind.as_u16())) {
        return Ok(());
    }
    let community = tx.community();
    let id = event.id.as_bytes().as_slice();
    let mentioned: Vec<Vec<u8>> = event
        .tags
        .public_keys()
        .map(|key| key.to_bytes().to_vec())
        .collect();
    let (read, thread, reply, root_created_at, broadcast) = match place {
        Place::TopLevel => (&[][..], id, false, None, false),
        Place::Reply {
            root,
            root_created_at,
            broadcast,
        } => (root, root, true, Some(root_created_at), broadcast),
    };
    // Accounts, then frontiers, each in key order, so concurrent messages that
    // involve the same people cannot deadlock: the frontier insert joins the
    // finished account insert. Foreign-key checks run at the end of the
    // statement, after the account rows exist.
    sqlx::query(
        "WITH involved (actor, root_id, through, message, refollow) AS (
            SELECT $4::bytea, $5::bytea, $6::timestamptz, $3::bytea, true
            UNION ALL
            SELECT $4, ''::bytea, $6, $3, true WHERE $11
            UNION ALL
            -- A top-level message's author follows its thread from it, so
            -- every install lists the new root before its first reply.
            SELECT $4, $7, $6, $3, true WHERE NOT $9
            UNION ALL
            SELECT cm.pubkey, $7, $6-interval '1 microsecond', NULL, true FROM channel_members cm
            WHERE cm.community_id=$1 AND cm.channel_id=$2 AND cm.removed_at IS NULL
                AND cm.pubkey=ANY($8) AND cm.pubkey<>$4
            UNION ALL
            SELECT e.pubkey, $7, $6-interval '1 microsecond', NULL, false FROM events e
            WHERE $9 AND e.community_id=$1 AND e.created_at=$10 AND e.id=$7
                AND e.channel_id=$2 AND e.pubkey<>$4
            UNION ALL
            SELECT cm.pubkey, $7, $6-interval '1 microsecond', NULL, false
            FROM channel_members cm JOIN channels c
                ON c.community_id=cm.community_id AND c.id=cm.channel_id
            WHERE $9 AND cm.community_id=$1 AND cm.channel_id=$2 AND cm.removed_at IS NULL
                AND c.channel_type='dm' AND cm.pubkey<>$4
         ), rows AS (
            -- An existing follower keeps their position, so is only touched
            -- to keep the follow live, at most once per refresh interval ($12)
            -- rather than per message;
            -- an unfollow is not touched unless this message re-follows (the
            -- author posting, or a mention). Existence probes are LIMIT 1
            -- laterals: always one index lookup.
            SELECT DISTINCT ON (i.actor, i.root_id) i.* FROM involved i
            LEFT JOIN LATERAL (
                SELECT f.unfollowed, f.active_at FROM personal_read_frontiers f
                WHERE f.community_id=$1 AND f.actor=i.actor AND f.channel_id=$2
                    AND f.root_id=i.root_id LIMIT 1
            ) f ON true
            WHERE NOT EXISTS (SELECT 1 FROM users u WHERE u.community_id=$1
                    AND u.pubkey=i.actor AND u.agent_owner_pubkey IS NOT NULL)
                AND (i.message IS NOT NULL OR f.unfollowed IS NULL
                    OR (i.refollow AND f.unfollowed)
                    OR (i.root_id<>''::bytea AND NOT f.unfollowed
                        AND f.active_at < now()-make_interval(secs=>$12)))
            ORDER BY i.actor, i.root_id, i.message IS NULL, NOT i.refollow
         ), accounts AS (
            INSERT INTO personal_read_accounts (community_id, actor)
            SELECT $1, r.actor FROM (SELECT DISTINCT actor FROM rows) r
            LEFT JOIN LATERAL (
                SELECT true AS found FROM personal_read_accounts a
                WHERE a.community_id=$1 AND a.actor=r.actor LIMIT 1
            ) a ON true
            WHERE a.found IS NULL ORDER BY r.actor
            ON CONFLICT DO NOTHING RETURNING 1
         )
         -- A re-follow starts just before this message, like a new follow,
         -- unless the unfollow is later. A row that a concurrent message
         -- created is only advanced when it carries an anchor; one kept live
         -- keeps its position.
         INSERT INTO personal_read_frontiers
            (community_id, actor, channel_id, root_id, through_timestamp, through_message_id)
         SELECT $1, actor, $2, root_id, through, message
         FROM rows CROSS JOIN (SELECT count(*) FROM accounts) written
         ORDER BY actor, root_id
         ON CONFLICT (community_id, actor, channel_id, root_id) DO UPDATE
         SET through_timestamp=CASE
                WHEN excluded.through_message_id IS NOT NULL OR personal_read_frontiers.unfollowed
                THEN GREATEST(personal_read_frontiers.through_timestamp, excluded.through_timestamp)
                ELSE personal_read_frontiers.through_timestamp END,
            through_message_id=CASE
                WHEN (excluded.through_message_id IS NOT NULL OR personal_read_frontiers.unfollowed)
                    AND excluded.through_timestamp > personal_read_frontiers.through_timestamp
                THEN excluded.through_message_id ELSE personal_read_frontiers.through_message_id END,
            unfollowed=false,
            active_at=CASE WHEN excluded.root_id<>''::bytea THEN now()
                ELSE personal_read_frontiers.active_at END
         WHERE excluded.through_message_id IS NOT NULL OR (personal_read_frontiers.unfollowed
            AND excluded.through_timestamp > personal_read_frontiers.through_timestamp)
            OR (excluded.root_id<>''::bytea AND NOT personal_read_frontiers.unfollowed
                AND personal_read_frontiers.active_at < now()-make_interval(secs=>$12))",
    )
    .bind(community.as_uuid())
    .bind(channel)
    .bind(id)
    .bind(event.pubkey.to_bytes().as_slice())
    .bind(read)
    .bind(received_at)
    .bind(thread)
    .bind(&mentioned)
    .bind(reply)
    .bind(root_created_at)
    .bind(broadcast)
    .bind(refresh_after_seconds())
    .execute(tx.conn())
    .await?;
    Ok(())
}
