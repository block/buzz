//! Transactional experimental interactions. Every decision and its signed state
//! commit together. The prompt row is the serialization point across relay pods.

use buzz_core::interaction::{self, Answer, InteractionState, Prompt};
use buzz_core::kind::{
    KIND_INTERACTION_CLOSE, KIND_INTERACTION_PROMPT, KIND_INTERACTION_RESPONSE, KIND_STREAM_MESSAGE,
};
use buzz_core::{CommunityId, StoredEvent};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{event::ThreadMetadataParams, Db, DbError, Result};

struct StateUpdate<'a> {
    prompt: &'a Event,
    schema: &'a Prompt,
    state: &'a InteractionState,
    timestamp: u64,
}

fn invalid(error: impl std::fmt::Display) -> DbError {
    DbError::InvalidData(error.to_string())
}
fn tag(parts: Vec<String>) -> Result<Tag> {
    Tag::parse(parts).map_err(invalid)
}
fn sign(builder: EventBuilder, keys: &Keys) -> Result<Event> {
    builder.sign_with_keys(keys).map_err(invalid)
}

async fn now(tx: &mut Transaction<'_, Postgres>) -> Result<u64> {
    let seconds: i64 =
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&mut **tx)
            .await?;
    Ok(seconds as u64)
}

async fn active_member(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    channel: Uuid,
    author: &nostr::PublicKey,
) -> Result<()> {
    // Holding the actual row prevents removal from committing during acceptance.
    let member: Option<Vec<u8>> = sqlx::query_scalar("SELECT pubkey FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3 AND removed_at IS NULL FOR SHARE")
        .bind(community.as_uuid()).bind(channel).bind(author.to_bytes().as_slice()).fetch_optional(&mut **tx).await?;
    if member.is_none() {
        return Err(DbError::AccessDenied(
            "interactions require current channel membership".into(),
        ));
    }
    Ok(())
}

async fn allowed(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    p: &Prompt,
    actor: &nostr::PublicKey,
) -> Result<()> {
    active_member(tx, community, p.channel, actor).await?;
    let yes = match p.responders.as_str() {
        "members" => true,
        "listed" => p.listed.contains(&actor.to_hex()),
        rule => {
            let role: Option<String> = sqlx::query_scalar(
                "SELECT role FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR SHARE",
            )
            .bind(community.as_uuid())
            .bind(actor.to_hex())
            .fetch_optional(&mut **tx)
            .await?;
            match rule {
                "role:owner" => role.as_deref() == Some("owner"),
                "role:admin" => matches!(role.as_deref(), Some("owner" | "admin")),
                _ => false,
            }
        }
    };
    if yes {
        Ok(())
    } else {
        Err(DbError::AccessDenied("not an eligible responder".into()))
    }
}

async fn queue(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    channel: Uuid,
    event: &Event,
) -> Result<()> {
    sqlx::query("INSERT INTO interaction_outbox (community_id,event_id,channel_id,event) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING")
        .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice()).bind(channel).bind(serde_json::to_value(event)?).execute(&mut **tx).await?;
    Ok(())
}

async fn store(
    tx: &mut Transaction<'_, Postgres>,
    community: CommunityId,
    channel: Uuid,
    event: &Event,
    meta: Option<ThreadMetadataParams<'_>>,
) -> Result<StoredEvent> {
    let (stored, inserted) = crate::event::insert_event_with_thread_metadata_tx(
        tx,
        community,
        event,
        Some(channel),
        meta,
    )
    .await?;
    if inserted {
        crate::insert_mentions_in_transaction(tx, community, event, Some(channel)).await?;
        queue(tx, community, channel, event).await?;
    }
    Ok(stored)
}

fn projection(prompt: &Event, p: &Prompt, keys: &Keys) -> Result<Event> {
    let mut content = format!("{}\n\n", prompt.content);
    for option in &p.options {
        content.push_str(&format!("• {} — {}\n", option.id, option.label));
    }
    if p.itype == "form" || p.fields.iter().any(|f| f.required) {
        content.push_str("Answer with buzz interactions answer (this prompt has form fields).");
    } else {
        content.push_str("Reply directly to this message with one option ID or label to answer.");
    }
    content.push_str(&format!(
        "\nExperimental interaction · channel-visible answers · prompt {}",
        prompt.id
    ));
    let mut ts: Vec<Tag> = prompt
        .tags
        .iter()
        .filter(|t| ["h", "e", "p"].contains(&t.kind().to_string().as_str()))
        .cloned()
        .collect();
    ts.push(tag(vec!["interaction".into(), prompt.id.to_hex()])?);
    ts.push(tag(vec!["actor".into(), prompt.pubkey.to_hex()])?);
    sign(
        EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), content)
            .tags(ts)
            .custom_created_at(prompt.created_at),
        keys,
    )
}

fn fallback_response(
    source: &Event,
    prompt: &Event,
    p: &Prompt,
    answer: &Answer,
    keys: &Keys,
) -> Result<Event> {
    let mut ts = vec![
        tag(vec![
            "e".into(),
            prompt.id.to_hex(),
            "".into(),
            "prompt".into(),
        ])?,
        tag(vec!["h".into(), p.channel.to_string()])?,
        tag(vec!["via".into(), source.id.to_hex()])?,
        tag(vec!["actor".into(), source.pubkey.to_hex()])?,
    ];
    for choice in &answer.choices {
        ts.push(tag(vec!["choice".into(), choice.clone()])?);
    }
    sign(
        EventBuilder::new(Kind::Custom(KIND_INTERACTION_RESPONSE as u16), "")
            .tags(ts)
            .custom_created_at(source.created_at),
        keys,
    )
}

impl Db {
    /// Accept a prompt, answer, close, or exact ordinary-message fallback.
    ///
    /// None means an ordinary message was not an eligible answer and should
    /// continue through normal message ingest. Some(empty) is an exact replay.
    /// All authorization reads use the writer inside the decision transaction.
    pub async fn accept_interaction(
        &self,
        community: CommunityId,
        event: &Event,
        keys: &Keys,
        meta: Option<ThreadMetadataParams<'_>>,
    ) -> Result<Option<Vec<StoredEvent>>> {
        let kind = event.kind.as_u16() as u32;
        let fallback = ![
            KIND_INTERACTION_PROMPT,
            KIND_INTERACTION_RESPONSE,
            KIND_INTERACTION_CLOSE,
        ]
        .contains(&kind);
        if fallback {
            if interaction::validate_envelope(event).is_err() {
                return Ok(None);
            }
        } else {
            interaction::validate_envelope(event).map_err(invalid)?;
        }
        let target = if kind == KIND_INTERACTION_PROMPT {
            event.id.to_bytes().to_vec()
        } else if fallback {
            if event.pubkey == keys.public_key() {
                return Ok(None);
            }
            let Some((_, parent)) = buzz_core::nip10::parse_thread_markers(&event.tags).resolve()
            else {
                return Ok(None);
            };
            hex::decode(parent).map_err(invalid)?
        } else {
            interaction::prompt_id(event)
                .map_err(invalid)?
                .to_bytes()
                .to_vec()
        };
        let channel = match interaction::channel(event) {
            Ok(channel) => channel,
            Err(_) if fallback => return Ok(None),
            Err(error) => return Err(invalid(error)),
        };
        let mut tx = self.begin_event_write_transaction().await?;
        let mut emitted = Vec::new();
        if kind == KIND_INTERACTION_PROMPT {
            let p = Prompt::parse(event).map_err(invalid)?;
            // Stable lock ordering: author rate-limit lock, then channel cap lock.
            for scope in [
                format!("interaction-author:{community}:{}", event.pubkey),
                format!("interaction-channel:{community}:{channel}"),
            ] {
                sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
                    .bind(scope)
                    .execute(&mut *tx)
                    .await?;
            }
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM interactions WHERE community_id=$1 AND prompt_id=$2)",
            )
            .bind(community.as_uuid())
            .bind(&target)
            .fetch_one(&mut *tx)
            .await?;
            if exists {
                return Ok(Some(emitted));
            }
            let clock = now(&mut tx).await?;
            p.validate_lifetime(clock).map_err(invalid)?;
            active_member(&mut tx, community, channel, &event.pubkey).await?;
            let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE community_id=$1 AND id=$2 AND archived_at IS NULL AND deleted_at IS NULL)").bind(community.as_uuid()).bind(channel).fetch_one(&mut *tx).await?;
            if !active {
                return Err(DbError::AccessDenied("channel is unavailable".into()));
            }
            let recent: i64 = sqlx::query_scalar("SELECT count(*) FROM interactions WHERE community_id=$1 AND author=$2 AND received_at > now()-interval '1 minute'").bind(community.as_uuid()).bind(event.pubkey.to_bytes().as_slice()).fetch_one(&mut *tx).await?;
            let open: i64 = sqlx::query_scalar("SELECT count(*) FROM interactions WHERE community_id=$1 AND channel_id=$2 AND NOT closed AND expiration>$3").bind(community.as_uuid()).bind(channel).bind(clock as i64).fetch_one(&mut *tx).await?;
            if recent >= 10 || open >= 32 {
                return Err(DbError::AccessDenied(
                    "interaction prompt limit reached (10/minute/key, 32 open/channel)".into(),
                ));
            }
            let text = projection(event, &p, keys)?;
            let projection_meta = meta.as_ref().map(|m| ThreadMetadataParams {
                event_id: text.id.as_bytes(),
                event_created_at: m.event_created_at,
                channel_id: m.channel_id,
                parent_event_id: m.parent_event_id,
                parent_event_created_at: m.parent_event_created_at,
                root_event_id: m.root_event_id,
                root_event_created_at: m.root_event_created_at,
                depth: m.depth,
                broadcast: m.broadcast,
            });
            // Only the ordinary message projection contributes to timeline counters.
            emitted.push(store(&mut tx, community, channel, &text, projection_meta).await?);
            emitted.push(store(&mut tx, community, channel, event, None).await?);
            let state = InteractionState::default();
            sqlx::query("INSERT INTO interactions (community_id,prompt_id,channel_id,author,prompt,state,projection_id,state_timestamp,expiration) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                .bind(community.as_uuid()).bind(&target).bind(channel).bind(event.pubkey.to_bytes().as_slice()).bind(serde_json::to_value(event)?).bind(serde_json::to_value(&state)?).bind(text.id.as_bytes().as_slice()).bind(clock as i64).bind(p.deadline as i64).execute(&mut *tx).await?;
            emitted.push(
                self.persist_interaction_state(
                    &mut tx,
                    community,
                    StateUpdate {
                        prompt: event,
                        schema: &p,
                        state: &state,
                        timestamp: clock,
                    },
                    keys,
                )
                .await?,
            );
        } else {
            let row = sqlx::query("SELECT i.prompt,i.state,i.state_timestamp FROM interactions i WHERE i.community_id=$1 AND (i.prompt_id=$2 OR i.projection_id=$2) AND i.channel_id=$3 FOR UPDATE")
                .bind(community.as_uuid()).bind(&target).bind(channel).fetch_optional(&mut *tx).await?;
            let Some(row) = row else {
                return if fallback {
                    Ok(None)
                } else {
                    Err(invalid("prompt not found in channel"))
                };
            };
            let prompt: Event = serde_json::from_value(row.try_get("prompt")?)?;
            // A response/close must reference the signed prompt, never its projection.
            if !fallback && target != prompt.id.as_bytes().as_slice() {
                return Err(invalid("reference the prompt event ID"));
            }
            let p = Prompt::parse(&prompt).map_err(invalid)?;
            let mut state: InteractionState = serde_json::from_value(row.try_get("state")?)?;
            let clock = now(&mut tx).await?;
            let duplicate: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)",
            )
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .fetch_one(&mut *tx)
            .await?;
            if duplicate {
                return Ok(Some(emitted));
            }
            let visible: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events e JOIN channels c ON c.community_id=e.community_id AND c.id=e.channel_id WHERE e.community_id=$1 AND e.id=$2 AND e.deleted_at IS NULL AND c.archived_at IS NULL AND c.deleted_at IS NULL)").bind(community.as_uuid()).bind(prompt.id.as_bytes().as_slice()).fetch_one(&mut *tx).await?;
            if !visible {
                return if fallback {
                    Ok(None)
                } else {
                    Err(invalid("prompt is unavailable"))
                };
            }
            if kind == KIND_INTERACTION_CLOSE {
                if event.pubkey != prompt.pubkey {
                    return Err(DbError::AccessDenied(
                        "only the asker may close a prompt".into(),
                    ));
                }
                active_member(&mut tx, community, channel, &event.pubkey).await?;
                // Parse the same reserved provenance and size envelope as responses.
                for reserved in ["via", "actor", "interaction", "choice", "value"] {
                    if event.tags.iter().any(|t| t.kind().to_string() == reserved) {
                        return Err(invalid("close carries invalid tags"));
                    }
                }
                if !state.close(if clock >= p.deadline {
                    "expiry"
                } else {
                    "manual"
                }) {
                    return Err(invalid("prompt is closed"));
                }
            } else {
                let answer = if fallback {
                    let Some(answer) = p.text_answer(&event.content) else {
                        return Ok(None);
                    };
                    answer
                } else {
                    p.answer(event).map_err(invalid)?
                };
                if let Err(error) = allowed(&mut tx, community, &p, &event.pubkey).await {
                    return if fallback && matches!(error, DbError::AccessDenied(_)) {
                        Ok(None)
                    } else {
                        Err(error)
                    };
                }
                if state.revision >= 4096 {
                    if fallback {
                        return Ok(None);
                    }
                    return Err(invalid("interaction transition limit reached"));
                }
                if let Err(error) = state.respond(&p, event.clone(), answer.clone(), clock) {
                    return if fallback {
                        Ok(None)
                    } else {
                        Err(invalid(error))
                    };
                }
                if fallback {
                    let response = fallback_response(event, &prompt, &p, &answer, keys)?;
                    emitted.push(store(&mut tx, community, channel, &response, None).await?);
                }
            }
            emitted.push(store(&mut tx, community, channel, event, meta).await?);
            let previous: i64 = row.try_get("state_timestamp")?;
            let timestamp = clock.max(previous as u64 + 1);
            emitted.push(
                self.persist_interaction_state(
                    &mut tx,
                    community,
                    StateUpdate {
                        prompt: &prompt,
                        schema: &p,
                        state: &state,
                        timestamp,
                    },
                    keys,
                )
                .await?,
            );
        }
        tx.commit().await?;
        Ok(Some(emitted))
    }

    async fn persist_interaction_state(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        community: CommunityId,
        update: StateUpdate<'_>,
        keys: &Keys,
    ) -> Result<StoredEvent> {
        let StateUpdate {
            prompt,
            schema: p,
            state,
            timestamp,
        } = update;
        let event = sign(
            state
                .event_builder(prompt.id, p, timestamp)
                .map_err(invalid)?,
            keys,
        )?;
        let result = self
            .replace_parameterized_event_in_transaction(
                tx,
                community,
                &event,
                &prompt.id.to_hex(),
                Some(p.channel),
                crate::replaceable::ParameterizedReplacePrecondition::Unconditional,
            )
            .await?;
        if result.status != crate::replaceable::ParameterizedReplaceStatus::Inserted {
            return Err(invalid("interaction state did not advance"));
        }
        sqlx::query("UPDATE interactions SET state=$3, state_timestamp=$4, closed=$5 WHERE community_id=$1 AND prompt_id=$2")
            .bind(community.as_uuid()).bind(prompt.id.as_bytes().as_slice()).bind(serde_json::to_value(state)?).bind(timestamp as i64).bind(state.close_reason.is_some()).execute(&mut **tx).await?;
        queue(tx, community, p.channel, &event).await?;
        Ok(result.event)
    }

    /// Close a bounded batch of expired prompts, atomically and safely across pods.
    /// Authoritative deadlines are checked again after taking each row lock.
    pub async fn expire_interactions(&self, keys: &Keys) -> Result<usize> {
        let mut tx = self.begin_event_write_transaction().await?;
        let clock = now(&mut tx).await?;
        let rows = sqlx::query("SELECT i.community_id,i.prompt,i.state,i.state_timestamp FROM interactions i JOIN communities c ON c.id=i.community_id WHERE NOT i.closed AND i.expiration<=$1 AND c.deletion_state='active' AND c.archived_at IS NULL ORDER BY i.expiration LIMIT 100 FOR UPDATE OF i SKIP LOCKED")
            .bind(clock as i64).fetch_all(&mut *tx).await?;
        for row in &rows {
            let community = CommunityId::from_uuid(row.try_get("community_id")?);
            let prompt: Event = serde_json::from_value(row.try_get("prompt")?)?;
            let p = Prompt::parse(&prompt).map_err(invalid)?;
            let mut state: InteractionState = serde_json::from_value(row.try_get("state")?)?;
            state.close("expiry");
            let previous: i64 = row.try_get("state_timestamp")?;
            self.persist_interaction_state(
                &mut tx,
                community,
                StateUpdate {
                    prompt: &prompt,
                    schema: &p,
                    state: &state,
                    timestamp: clock.max(previous as u64 + 1),
                },
                keys,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(rows.len())
    }

    /// Read a bounded, deployment-wide delivery batch, labelled with each row's tenant.
    pub async fn pending_interaction_events(&self) -> Result<Vec<InteractionDelivery>> {
        let mut tx = self.begin_event_write_transaction().await?;
        // Replacement or moderation may remove a queued event before delivery.
        // Do not resurrect it from the outbox's retained signed payload.
        sqlx::query("DELETE FROM interaction_outbox WHERE (community_id,event_id) IN (SELECT o.community_id,o.event_id FROM interaction_outbox o WHERE NOT EXISTS (SELECT 1 FROM events e WHERE e.community_id=o.community_id AND e.id=o.event_id AND e.deleted_at IS NULL) ORDER BY o.queued_at LIMIT 100)")
            .execute(&mut *tx).await?;
        // Pruning is bounded, so more removed rows may remain in the queue.
        // Independently require a live event for every delivery in this batch.
        let rows = sqlx::query("SELECT o.community_id,c.host,o.channel_id,o.event FROM interaction_outbox o JOIN events e ON e.community_id=o.community_id AND e.id=o.event_id AND e.deleted_at IS NULL JOIN communities c ON c.id=o.community_id JOIN channels ch ON ch.community_id=o.community_id AND ch.id=o.channel_id WHERE c.deletion_state='active' AND c.archived_at IS NULL AND ch.archived_at IS NULL AND ch.deleted_at IS NULL ORDER BY o.queued_at LIMIT 100").fetch_all(&mut *tx).await?;
        let deliveries = rows
            .into_iter()
            .map(|r| {
                Ok(InteractionDelivery {
                    community: CommunityId::from_uuid(r.try_get("community_id")?),
                    host: r.try_get("host")?,
                    channel: r.try_get("channel_id")?,
                    event: serde_json::from_value(r.try_get("event")?)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        tx.commit().await?;
        Ok(deliveries)
    }

    /// Acknowledge successful Redis publication. Failures leave the event retryable.
    pub async fn acknowledge_interaction_event(
        &self,
        community: CommunityId,
        event: &Event,
    ) -> Result<()> {
        let mut tx = self.begin_event_write_transaction().await?;
        sqlx::query("DELETE FROM interaction_outbox WHERE community_id=$1 AND event_id=$2")
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Durable event delivery addressed to its server-resolved community.
pub struct InteractionDelivery {
    /// Tenant ID from the stored row.
    pub community: CommunityId,
    /// Current community host.
    pub host: String,
    /// Channel boundary for fan-out.
    pub channel: Uuid,
    /// Exact committed, signed event.
    pub event: Event,
}

#[cfg(test)]
#[path = "interaction_postgres_tests.rs"]
mod postgres_tests;

#[cfg(test)]
#[path = "interaction_unit_tests.rs"]
mod tests;
