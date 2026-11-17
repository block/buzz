//! NIP-AR atomic artifact acceptance. Payloads remain opaque signed events.
//!
//! Channel write permission is checked by the relay's ordinary ingest gates
//! before this transaction; here the relay only serializes each identity and
//! compares the expected head.
use crate::{Db, DbError, Result};
use buzz_core::artifact::{ArtifactEnvelope, ArtifactOp};
use buzz_core::{CommunityId, StoredEvent};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::Row;
use uuid::Uuid;

/// Acceptance outcome. Only a stale-`prev` conflict carries the current head,
/// which the caller discloses after authorizing its channel.
#[derive(Debug)]
pub enum ArtifactOutcome {
    /// Accepted once.
    Accepted {
        /// Stored events to publish (the revision, plus the source removal on
        /// a move).
        events: Vec<StoredEvent>,
        /// The artifact's conversation, when this revision changed who may
        /// read it (a move, delete or restore). Its readers' caches and live
        /// subscriptions need refreshing after commit.
        conversation: Option<Uuid>,
    },
    /// Identical previously accepted event; no repeated side effects.
    Duplicate,
    /// `prev` or identity no longer matches; the client should reconcile.
    Conflict(&'static str),
    /// Protocol violation with a public-safe reason.
    Rejected(&'static str),
}

/// Outcome of a request to open an artifact's conversation.
#[derive(Debug)]
pub enum ConversationOutcome {
    /// Created, with the stored kind:9007 to publish.
    Created(Box<StoredEvent>),
    /// It already existed; nothing was written.
    Exists,
    /// The artifact is missing, deleted, or not in the home the caller
    /// authorized. Answered like a missing artifact.
    Unavailable,
    /// Refused with a public-safe reason.
    Rejected(&'static str),
}

/// Refusal for a move that would take a conversation where none may be.
pub const CONVERSATION_CANNOT_MOVE: &str = "this artifact's comments can't move there";

/// Conflict for a non-create revision whose artifact is missing or lives
/// outside every channel the sender was authorized for.
pub const HEAD_UNAVAILABLE: &str = "artifact head unavailable";

fn invalid(message: &str) -> DbError {
    DbError::InvalidData(message.into())
}

/// The replaced revision (`prev`) was readable in the source, so it
/// distinguishes repeated moves without revealing other activity.
fn removal_marker(keys: &Keys, artifact: Uuid, source: Uuid, prev: &[u8]) -> Result<Event> {
    let tags = [
        ["ar", "1"].map(str::to_owned),
        ["d".into(), artifact.to_string()],
        ["h".into(), source.to_string()],
        ["reason".into(), "moved".into()],
        ["prev".into(), hex::encode(prev)],
    ]
    .into_iter()
    .map(Tag::parse)
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(|e| invalid(&e.to_string()))?;
    EventBuilder::new(Kind::Custom(45011), "")
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|e| invalid(&e.to_string()))
}

impl Db {
    /// Check the durable acceptance ledger, which survives redaction and retention.
    pub async fn artifact_accepted(&self, community: CommunityId, id: &[u8]) -> Result<bool> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND event_id=$2)",
        )
        .bind(community.as_uuid())
        .bind(id)
        .fetch_one(&mut *conn)
        .await?)
    }

    /// Current home channel, read before authorizing a move's source.
    pub async fn artifact_home(
        &self,
        community: CommunityId,
        artifact: Uuid,
    ) -> Result<Option<Uuid>> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        Ok(sqlx::query_scalar(
            "SELECT channel_id FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2",
        )
        .bind(community.as_uuid())
        .bind(artifact)
        .fetch_optional(&mut *conn)
        .await?)
    }

    /// Atomically compare the expected head, store the full revision, advance
    /// the head, and on a move store the source removal. `authorized_source` is
    /// the move source the caller authorized; a head that has since moved elsewhere
    /// conflicts. Relay keys only sign removals.
    pub async fn accept_artifact(
        &self,
        community: CommunityId,
        event: &Event,
        env: &ArtifactEnvelope,
        authorized_source: Option<Uuid>,
        relay_keys: &Keys,
    ) -> Result<ArtifactOutcome> {
        let mut tx = self.begin_event_write_transaction(community).await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(tx.conn())
            .await?;
        // A coordinate lock handles the missing-row create race as well as edits.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("artifact:{}:{}", community.as_uuid(), env.id))
            .execute(tx.conn())
            .await?;
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND event_id=$2)",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(tx.conn())
        .await?;
        if duplicate {
            return Ok(ArtifactOutcome::Duplicate);
        }
        let head = sqlx::query(
            "SELECT event_id,channel_id,artifact_type,root,deleted,creator FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(env.id)
        .fetch_optional(tx.conn())
        .await?;
        let source = head.as_ref().map(|h| h.get::<Uuid, _>("channel_id"));
        let old_root = head
            .as_ref()
            .and_then(|h| h.get::<Option<Vec<u8>>, _>("root"));
        match (&head, env.op) {
            (Some(_), ArtifactOp::Create) => {
                return Ok(ArtifactOutcome::Conflict("artifact identity is taken"))
            }
            (None, ArtifactOp::Create) => {}
            (None, _) => return Ok(ArtifactOutcome::Conflict(HEAD_UNAVAILABLE)),
            // The caller authorized writes only in `h`, or for a move in the
            // source it read before this transaction. A head anywhere else
            // answers like a missing one, before any check that describes it.
            (Some(_), op)
                if source
                    != match op {
                        ArtifactOp::Move => authorized_source,
                        _ => Some(env.home),
                    } =>
            {
                return Ok(ArtifactOutcome::Conflict(HEAD_UNAVAILABLE))
            }
            (Some(head), op) => {
                let current: Vec<u8> = head.get("event_id");
                if env.prev.as_deref() != Some(current.as_slice()) {
                    return Ok(ArtifactOutcome::Conflict("artifact head changed"));
                }
                if env.artifact_type != head.get::<String, _>("artifact_type") {
                    return Ok(ArtifactOutcome::Rejected("artifact type is immutable"));
                }
                if head.get::<bool, _>("deleted") != (op == ArtifactOp::Restore) {
                    return Ok(ArtifactOutcome::Rejected(
                        "deleted artifacts require restore; live artifacts cannot restore",
                    ));
                }
                if (op == ArtifactOp::Move) != (source != Some(env.home)) {
                    return Ok(ArtifactOutcome::Rejected(
                        "only move changes home and move must change home",
                    ));
                }
                if op == ArtifactOp::Delete && env.root != old_root {
                    return Ok(ArtifactOutcome::Rejected("delete preserves root"));
                }
            }
        }
        // The creator is the creating signer, and every later revision keeps
        // it, so the current revision's creator can be trusted without its
        // history. The head holds it, so it outlives retention and
        // redaction. A delete carries no creator; a restore keeps the one
        // from before the delete.
        match (&head, env.op) {
            (None, _) => {
                if env
                    .creator
                    .as_ref()
                    .is_some_and(|c| *c != event.pubkey.to_hex())
                {
                    return Ok(ArtifactOutcome::Rejected(
                        "creator must be the signer on create",
                    ));
                }
            }
            (Some(_), ArtifactOp::Delete) => {}
            (Some(head), _) => {
                if env.creator != head.get::<Option<String>, _>("creator") {
                    return Ok(ArtifactOutcome::Rejected("creator cannot change"));
                }
            }
        }
        if source != Some(env.home) {
            // A conversation holds comments, never artifacts.
            let home_type: Option<String> = sqlx::query_scalar(
                "SELECT channel_type::text FROM channels WHERE community_id=$1 AND id=$2",
            )
            .bind(community.as_uuid())
            .bind(env.home)
            .fetch_optional(tx.conn())
            .await?;
            if home_type.as_deref() == Some(buzz_core::channel::ChannelType::Artifact.as_str()) {
                return Ok(ArtifactOutcome::Rejected(
                    "an artifact cannot live in a conversation",
                ));
            }
        }
        if head.is_none() || env.root != old_root || source != Some(env.home) {
            if let Some(root) = &env.root {
                let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2 AND channel_id=$3 AND deleted_at IS NULL AND kind IN (9,40002,45001,45003))")
                    .bind(community.as_uuid()).bind(root).bind(env.home).fetch_one(tx.conn()).await?;
                if !exists {
                    return Ok(ArtifactOutcome::Rejected(
                        "root must be an existing conversation anchor in home",
                    ));
                }
            }
        }
        sqlx::query(
            "INSERT INTO artifact_revisions (community_id,event_id,artifact_id,source_channel_id,modified_at) VALUES ($1,$2,$3,$4,clock_timestamp())",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(env.id)
        .bind(if env.op == ArtifactOp::Move { source } else { None })
        .execute(tx.conn())
        .await?;
        sqlx::query("INSERT INTO artifact_heads (community_id,artifact_id,event_id,channel_id,artifact_type,root,deleted,creator,modified_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,clock_timestamp()) ON CONFLICT (community_id,artifact_id) DO UPDATE SET event_id=EXCLUDED.event_id,channel_id=EXCLUDED.channel_id,root=EXCLUDED.root,deleted=EXCLUDED.deleted,modified_at=clock_timestamp()")
            .bind(community.as_uuid()).bind(env.id).bind(event.id.as_bytes().as_slice()).bind(env.home).bind(&env.artifact_type).bind(&env.root).bind(env.op == ArtifactOp::Delete).bind(&env.creator).execute(tx.conn()).await?;
        let conversation = match env.op {
            ArtifactOp::Move | ArtifactOp::Delete | ArtifactOp::Restore => {
                match follow_artifact(&mut tx, env).await? {
                    Follow::Unchanged => None,
                    Follow::Changed(id) => Some(id),
                    Follow::Refused(reason) => return Ok(ArtifactOutcome::Rejected(reason)),
                }
            }
            _ => None,
        };
        let (stored, _) =
            crate::event::insert_event_in_transaction(&mut tx, event, Some(env.home)).await?;
        crate::insert_mentions_in_transaction(&mut tx, event, Some(env.home)).await?;
        let mut accepted = vec![stored];
        if let (ArtifactOp::Move, Some(source), Some(prev)) = (env.op, source, &env.prev) {
            let removal = removal_marker(relay_keys, env.id, source, prev)?;
            let (stored, _) =
                crate::event::insert_event_in_transaction(&mut tx, &removal, Some(source)).await?;
            accepted.push(stored);
        }
        tx.commit().await?;
        Ok(ArtifactOutcome::Accepted {
            events: accepted,
            conversation,
        })
    }

    /// Open `artifact`'s conversation with the kind:9007 `event`, whose `h`
    /// the caller checked is [`buzz_core::artifact::conversation_id`]. The
    /// caller authorized the sender to post in `authorized_home`; the
    /// artifact must live there and not be deleted. The conversation is a
    /// private `artifact` channel filed under the home, with no members:
    /// everyone reaches it through the home. Opening one that exists writes
    /// nothing.
    pub async fn ensure_artifact_conversation(
        &self,
        community: CommunityId,
        event: &Event,
        artifact: Uuid,
        authorized_home: Uuid,
    ) -> Result<ConversationOutcome> {
        let conversation = buzz_core::artifact::conversation_id(&artifact.to_string());
        let mut tx = self.begin_event_write_transaction(community).await?;
        sqlx::query("SET LOCAL statement_timeout='5s'")
            .execute(tx.conn())
            .await?;
        // The artifact's own lock: a revision that moves or deletes it waits.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("artifact:{}:{}", community.as_uuid(), artifact))
            .execute(tx.conn())
            .await?;
        let head: Option<(Uuid, bool)> = sqlx::query_as(
            "SELECT channel_id, deleted FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2",
        )
        .bind(community.as_uuid())
        .bind(artifact)
        .fetch_optional(tx.conn())
        .await?;
        let Some((home, false)) = head else {
            return Ok(ConversationOutcome::Unavailable);
        };
        if home != authorized_home {
            return Ok(ConversationOutcome::Unavailable);
        }
        let existing: Option<String> = sqlx::query_scalar(
            "SELECT channel_type::text FROM channels WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(conversation)
        .fetch_optional(tx.conn())
        .await?;
        match existing.as_deref() {
            None => {}
            Some(t) if t == buzz_core::channel::ChannelType::Artifact.as_str() => {
                return Ok(ConversationOutcome::Exists)
            }
            // Ingest refuses other channels at derived addresses; one made
            // before that rule is never treated as the conversation.
            Some(_) => {
                return Ok(ConversationOutcome::Rejected(
                    "conversation address is taken",
                ))
            }
        }
        if let Some(reason) = crate::channel_tree::refuse_conversation_home(&mut tx, home).await? {
            return Ok(ConversationOutcome::Rejected(reason));
        }
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by, parent_id) \
             VALUES ($1, $2, $3, 'artifact', 'private', $4, $5)",
        )
        .bind(conversation)
        .bind(community.as_uuid())
        .bind(artifact.to_string())
        .bind(event.pubkey.to_bytes().as_slice())
        .bind(home)
        .execute(tx.conn())
        .await?;
        let (stored, _) =
            crate::event::insert_event_in_transaction(&mut tx, event, Some(conversation)).await?;
        tx.commit().await?;
        Ok(ConversationOutcome::Created(Box::new(stored)))
    }
}

/// What a move, delete or restore did to the artifact's conversation.
enum Follow {
    /// There is no conversation.
    Unchanged,
    /// The conversation was refiled or its access changed.
    Changed(Uuid),
    /// The conversation cannot follow, so the revision is refused.
    Refused(&'static str),
}

/// Keep `env`'s conversation, if it has one, with its artifact, on the
/// revision's transaction. A move refiles it under the new home, a delete
/// closes it to the home's readers (`parent_access = 'none'`; it has no rows
/// of its own), and a restore reopens it.
async fn follow_artifact(tx: &mut crate::AdmittedTx, env: &ArtifactEnvelope) -> Result<Follow> {
    let community = tx.community();
    let conversation = buzz_core::artifact::conversation_id(&env.id.to_string());
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM channels WHERE community_id=$1 AND id=$2 \
                        AND channel_type='artifact')",
    )
    .bind(community.as_uuid())
    .bind(conversation)
    .fetch_one(tx.conn())
    .await?;
    if !exists {
        return Ok(Follow::Unchanged);
    }
    match env.op {
        ArtifactOp::Move => {
            if crate::channel_tree::refuse_conversation_home(tx, env.home)
                .await?
                .is_some()
            {
                return Ok(Follow::Refused(CONVERSATION_CANNOT_MOVE));
            }
            sqlx::query(
                "UPDATE channels SET parent_id=$3, updated_at=NOW() WHERE community_id=$1 AND id=$2",
            )
            .bind(community.as_uuid())
            .bind(conversation)
            .bind(env.home)
            .execute(tx.conn())
            .await?;
        }
        ArtifactOp::Delete | ArtifactOp::Restore => {
            let access = if env.op == ArtifactOp::Delete {
                crate::channel_tree::ParentAccess::NoAccess
            } else {
                crate::channel_tree::ParentAccess::Member
            };
            sqlx::query(
                "UPDATE channels SET parent_access=$3, updated_at=NOW() WHERE community_id=$1 AND id=$2",
            )
            .bind(community.as_uuid())
            .bind(conversation)
            .bind(access.as_str())
            .execute(tx.conn())
            .await?;
        }
        _ => return Ok(Follow::Unchanged),
    }
    Ok(Follow::Changed(conversation))
}
