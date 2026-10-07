//! Explicit selectors over the same private frontier authority. No history API.
use super::{classification, model::*, participation, projection::read_account, writes};
use crate::{observability, Db, DbError, Result};
use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
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
        retention_seconds: u32,
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
        let account = read_account(&mut tx, retention_seconds).await?;
        let mut contexts = Vec::with_capacity(queries.len());
        // Replies whose state turns on conversation membership, by position.
        let mut pending = Vec::new();
        for query in queries {
            let root =
                match writes::valid_target(&mut tx, community, &actor_bytes, &query.target).await {
                    Ok(Some(root)) => root,
                    Ok(None) => {
                        contexts.push(ContextState::Unavailable);
                        continue;
                    }
                    Err(DbError::InvalidData(_)) => {
                        contexts.push(ContextState::Unknown);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
            // A thread's effective prefix includes the channel's whole-channel cut,
            // exactly as the sidebar projection counts it.
            let prefix: Option<DateTime<Utc>> = sqlx::query_scalar(
                "SELECT GREATEST(
                    (SELECT through_timestamp FROM personal_read_frontiers
                     WHERE community_id=$1 AND actor=$2 AND channel_id=$3 AND root_id=$4),
                    (SELECT threads_through_timestamp FROM personal_read_frontiers
                     WHERE community_id=$1 AND actor=$2 AND channel_id=$3 AND root_id=''::bytea
                        AND $4<>''::bytea))",
            )
            .bind(community.as_uuid())
            .bind(actor_bytes.as_slice())
            .bind(query.target.channel_id)
            .bind(&root)
            .fetch_one(&mut *tx)
            .await?;
            let ids: Vec<Vec<u8>> = query
                .message_ids
                .iter()
                .filter_map(|id| writes::event_id(id))
                .collect();
            let rows = sqlx::query(
                "SELECT encode(e.id,'hex') AS id,e.kind,e.created_at,e.received_at,
                    e.deleted_at IS NOT NULL AS deleted,e.pubkey=$3 AS own,
                    CASE WHEN octet_length(e.tags::text)<=8192 THEN e.tags ELSE NULL END AS tags,
                    tm.root_event_id,tm.parent_event_id,c.channel_type::text AS channel_type
                 FROM events e JOIN channels c ON c.community_id=e.community_id AND c.id=e.channel_id
                 LEFT JOIN thread_metadata tm ON tm.community_id=e.community_id
                    AND tm.event_id=e.id AND tm.event_created_at=e.created_at AND tm.channel_id=e.channel_id
                 WHERE e.community_id=$1 AND e.channel_id=$2 AND e.id=ANY($4)",
            ).bind(community.as_uuid()).bind(query.target.channel_id)
                .bind(actor_bytes.as_slice()).bind(&ids).fetch_all(&mut *tx).await?;
            let by_id: HashMap<String, _> = rows
                .into_iter()
                .map(|row| Ok((row.try_get::<String, _>("id")?, row)))
                .collect::<Result<_>>()?;
            let mut messages = Vec::with_capacity(ids.len());
            for id in &query.message_ids {
                let state = if let Some(row) = by_id.get(&id.to_ascii_lowercase()) {
                    let tags: Option<serde_json::Value> = row.try_get("tags")?;
                    let parsed =
                        tags.and_then(|v| serde_json::from_value::<Vec<Vec<String>>>(v).ok());
                    if let Some(tags) = parsed {
                        let canonical: Option<Vec<u8>> = row.try_get("root_event_id")?;
                        let marked_reply = buzz_core::nip10::parse_thread_markers_from_parts(
                            tags.iter().map(Vec::as_slice),
                        )
                        .resolve()
                        .is_some();
                        let message_id = writes::event_id(id).unwrap_or_default();
                        let is_reply = canonical.as_ref().is_some_and(|r| r != &message_id);
                        if marked_reply && canonical.is_none() {
                            MessageReadState::Unknown
                        } else if (root.is_empty() && is_reply)
                            || (!root.is_empty()
                                && (!is_reply || canonical.as_ref() != Some(&root)))
                        {
                            // The root's own timeline state is never the thread's state.
                            MessageReadState::Unavailable
                        } else {
                            let created: DateTime<Utc> = row.try_get("created_at")?;
                            let received: DateTime<Utc> = row.try_get("received_at")?;
                            let kind: i32 = row.try_get("kind")?;
                            if !classification::eligible(
                                kind,
                                row.try_get("own")?,
                                row.try_get("deleted")?,
                                created.timestamp_millis(),
                                account.cutoff_ms,
                            ) {
                                MessageReadState::NotCounted
                            } else if prefix.is_some_and(|p| received <= p) {
                                MessageReadState::Read
                            } else {
                                let reason = classification::reason(
                                    &row.try_get::<String, _>("channel_type")?,
                                    &actor.to_hex(),
                                    &tags,
                                );
                                // Membership outranks a broadcast and decides a
                                // plain reply. A DM or mention needs no lookup.
                                if is_reply
                                    && !matches!(reason, Some(Reason::Direct | Reason::Mention))
                                {
                                    let parent: Option<Vec<u8>> = row.try_get("parent_event_id")?;
                                    if let Some(parent) = parent {
                                        pending.push((
                                            contexts.len(),
                                            messages.len(),
                                            (query.target.channel_id, parent),
                                        ));
                                    }
                                }
                                // Provisional for a pending reply: see `settle`.
                                if is_reply && reason.is_none() {
                                    MessageReadState::Unknown
                                } else {
                                    MessageReadState::Unread { reason }
                                }
                            }
                        }
                    } else {
                        MessageReadState::Unknown
                    }
                } else {
                    MessageReadState::Unavailable
                };
                messages.push(ContextMessage {
                    message_id: id.clone(),
                    state,
                });
            }
            contexts.push(ContextState::Available { messages });
        }
        let targets: Vec<_> = pending.iter().map(|(.., key)| key.clone()).collect();
        let members = participation::resolve(&mut tx, community, &actor_bytes, &targets).await?;
        for (context, message, key) in pending {
            if let ContextState::Available { messages } = &mut contexts[context] {
                settle(&mut messages[message].state, members.get(&key).copied());
            }
        }
        tx.commit().await?;
        Ok(ContextPage { account, contexts })
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

/// Apply conversation membership to a pending reply's provisional state:
/// `unknown` for a plain reply, `unread` with reason `broadcast` for a
/// broadcast. An undecided lookup (`None`) leaves it as it is.
fn settle(state: &mut MessageReadState, member: Option<bool>) {
    match member {
        Some(true) => {
            *state = MessageReadState::Unread {
                reason: Some(Reason::Conversation),
            }
        }
        // A broadcast counts outside the actor's conversations too.
        Some(false) if matches!(state, MessageReadState::Unknown) => {
            *state = MessageReadState::NotCounted
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_settles_a_pending_reply_and_an_undecided_lookup_fabricates_nothing() {
        let broadcast = || MessageReadState::Unread {
            reason: Some(Reason::Broadcast),
        };
        for (provisional, member, expected) in [
            (MessageReadState::Unknown, Some(true), "conversation"),
            (MessageReadState::Unknown, Some(false), "not_counted"),
            (MessageReadState::Unknown, None, "unknown"),
            (broadcast(), Some(true), "conversation"),
            (broadcast(), Some(false), "broadcast"),
            (broadcast(), None, "broadcast"),
        ] {
            let mut state = provisional;
            settle(&mut state, member);
            let wire = serde_json::to_value(&state).unwrap();
            let got = wire["reason"].as_str().or(wire["status"].as_str());
            assert_eq!(got, Some(expected), "{member:?}");
        }
    }
}
