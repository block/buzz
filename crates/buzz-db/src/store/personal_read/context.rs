//! Explicit selectors over the same private positions. No history API.
use super::{classification, model::*, writes};
use crate::{observability, Db, DbError, Result};
use buzz_core::CommunityId;
use sqlx::{Acquire, Row};
use std::collections::HashMap;
use uuid::Uuid;

impl Db {
    /// Resolve bounded explicit contexts/messages in a single read-only snapshot.
    /// Callers must recheck admission and resource access outside this snapshot.
    pub async fn personal_read_contexts(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        queries: &[ContextQuery],
    ) -> Result<ContextPage> {
        if queries.is_empty()
            || queries.len() > MAX_CONTEXTS
            || queries.iter().map(|q| q.message_ids.len()).sum::<usize>() > MAX_CONTEXT_MESSAGES
            || queries.iter().any(|q| {
                q.message_ids
                    .iter()
                    .any(|id| writes::event_id(id).is_none())
            })
        {
            return Err(DbError::InvalidData("invalid context selectors".into()));
        }
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        let mut tx = conn.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        writes::deadlines(&mut tx).await?;
        let actor_bytes = actor.to_bytes();
        let actor_hex = actor.to_hex();
        let mut contexts = Vec::with_capacity(queries.len());
        for query in queries {
            let Some(root) =
                writes::valid_target(&mut tx, community, &actor_bytes, &query.target).await?
            else {
                contexts.push(ContextState::Unavailable);
                continue;
            };
            // The channel-wide cuts the sidebar subtracts against, absent
            // before the account starts: the channel's for a member, and the
            // thread's, which exists only for the actor's threads. A cut in
            // channel numbering orders every message in either context.
            let positions = sqlx::query(
                "SELECT CASE WHEN m.joined_at IS NOT NULL AND s.started IS NOT NULL
                        THEN COALESCE(cf.through_channel_seq, 0) END AS channel,
                    CASE WHEN tf.root_id IS NOT NULL AND s.started IS NOT NULL
                        THEN tf.through_channel_seq END AS thread
                 FROM (SELECT (SELECT started_at FROM personal_read_accounts
                    WHERE community_id=$1 AND actor=$2) AS started) s
                 LEFT JOIN channel_members m ON m.community_id=$1 AND m.channel_id=$3
                    AND m.pubkey=$2 AND m.removed_at IS NULL
                 LEFT JOIN personal_read_frontiers cf ON cf.community_id=$1 AND cf.actor=$2
                    AND cf.channel_id=$3 AND cf.root_id=''::bytea
                 LEFT JOIN personal_read_frontiers tf ON tf.community_id=$1 AND tf.actor=$2
                    AND tf.channel_id=$3 AND tf.root_id=$4 AND $4<>''::bytea",
            )
            .bind(community.as_uuid())
            .bind(actor_bytes.as_slice())
            .bind(query.target.channel_id)
            .bind(&root)
            .fetch_one(&mut *tx)
            .await?;
            let channel_position: Option<i64> = positions.try_get("channel")?;
            let thread_position: Option<i64> = positions.try_get("thread")?;
            let ids: Vec<Vec<u8>> = query
                .message_ids
                .iter()
                .filter_map(|id| writes::event_id(id))
                .collect();
            let rows = sqlx::query(
                "SELECT encode(e.id,'hex') AS id,e.kind,e.channel_seq,
                    e.deleted_at IS NOT NULL AS deleted,e.pubkey=$3 AS own,e.tags,
                    tm.root_event_id,tm.depth,tm.broadcast,c.channel_type::text AS channel_type
                 FROM events e JOIN channels c ON c.community_id=e.community_id AND c.id=e.channel_id
                 LEFT JOIN thread_metadata tm ON tm.community_id=e.community_id
                    AND tm.event_id=e.id AND tm.event_created_at=e.created_at AND tm.channel_id=e.channel_id
                 WHERE e.community_id=$1 AND e.channel_id=$2 AND e.id=ANY($4)",
            )
            .bind(community.as_uuid())
            .bind(query.target.channel_id)
            .bind(actor_bytes.as_slice())
            .bind(&ids)
            .fetch_all(&mut *tx)
            .await?;
            let by_id: HashMap<String, _> = rows
                .into_iter()
                .map(|row| Ok((row.try_get::<String, _>("id")?, row)))
                .collect::<Result<_>>()?;
            let mut messages = Vec::with_capacity(ids.len());
            for id in &query.message_ids {
                let state = match by_id.get(&id.to_ascii_lowercase()) {
                    None => MessageReadState::Unavailable,
                    Some(row) => {
                        let message_id = writes::event_id(id).unwrap_or_default();
                        let thread = row
                            .try_get::<Option<Vec<u8>>, _>("root_event_id")?
                            .filter(|r| r != &message_id);
                        let on_timeline = thread.is_none()
                            || (row.try_get::<Option<i32>, _>("depth")? == Some(1)
                                && row.try_get::<Option<bool>, _>("broadcast")? == Some(true));
                        // A thread context holds its replies, not its root; a
                        // broadcast reply belongs to both of its contexts.
                        let in_context = if root.is_empty() {
                            on_timeline
                        } else {
                            thread.as_ref() == Some(&root)
                        };
                        if !in_context {
                            MessageReadState::Unavailable
                        } else if !ELIGIBLE_KINDS.contains(&row.try_get("kind")?)
                            || row.try_get("own")?
                            || row.try_get("deleted")?
                        {
                            MessageReadState::NotCounted
                        } else {
                            // One state whichever context asks: a message
                            // counts against the timeline if it is on it.
                            let position = if on_timeline {
                                channel_position
                            } else {
                                thread_position
                            };
                            // Unnumbered messages predate numbering: read.
                            let seq: Option<i64> = row.try_get("channel_seq")?;
                            let tags: Vec<Vec<String>> =
                                serde_json::from_value(row.try_get("tags")?).unwrap_or_default();
                            let reason = classification::reason(
                                &row.try_get::<String, _>("channel_type")?,
                                &actor_hex,
                                &tags,
                            );
                            match position {
                                None => MessageReadState::NotCounted,
                                Some(p) if seq.is_none_or(|n| n <= p) => MessageReadState::Read,
                                Some(_) if on_timeline => MessageReadState::Unread { reason },
                                Some(_) => MessageReadState::Unread {
                                    reason: match reason {
                                        Some(Reason::Direct | Reason::Mention) => reason,
                                        _ => Some(Reason::Conversation),
                                    },
                                },
                            }
                        }
                    }
                };
                messages.push(ContextMessage {
                    message_id: id.clone(),
                    state,
                });
            }
            contexts.push(ContextState::Available { messages });
        }
        tx.commit().await?;
        Ok(ContextPage { contexts })
    }

    /// Final bounded access check on the writer, outside a projection snapshot.
    /// Open-channel access is independent of joined-sidebar membership.
    pub async fn personal_read_accessible_contexts(
        &self,
        community: CommunityId,
        actor: &nostr::PublicKey,
        channels: &[Uuid],
    ) -> Result<Vec<Uuid>> {
        if channels.len() > MAX_CONTEXTS {
            return Err(DbError::InvalidData("too many context channels".into()));
        }
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::Authorization,
        )
        .await?;
        Ok(sqlx::query_scalar(
            "SELECT c.id FROM channels c WHERE c.community_id=$1 AND c.id=ANY($2)
             AND c.deleted_at IS NULL AND (c.visibility='open' OR EXISTS (
                SELECT 1 FROM channel_members cm WHERE cm.community_id=$1
                AND cm.channel_id=c.id AND cm.pubkey=$3 AND cm.removed_at IS NULL))",
        )
        .bind(community.as_uuid())
        .bind(channels)
        .bind(actor.to_bytes().as_slice())
        .fetch_all(&mut *conn)
        .await?)
    }
}
