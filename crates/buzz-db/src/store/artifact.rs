//! NIP-AR atomic artifact acceptance. Payloads remain opaque signed events.
//!
//! Channel write permission is checked by the relay's ordinary ingest gates
//! before this transaction; here the relay only serializes each identity and
//! compares the expected head.
use crate::{Db, DbError, Result};
use buzz_core::artifact::{ArtifactEnvelope, ArtifactOp, FeedbackWake};
use buzz_core::{CommunityId, StoredEvent};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use sqlx::Row;
use uuid::Uuid;

/// Acceptance outcome. Only a stale-`prev` conflict carries the current head,
/// which the caller discloses after authorizing its channel.
#[derive(Debug)]
pub enum ArtifactOutcome {
    /// Accepted once; stored events to publish (the revision, plus the source
    /// removal on a move).
    Accepted(Vec<StoredEvent>),
    /// Identical previously accepted event; no repeated side effects.
    Duplicate,
    /// `prev` or identity no longer matches; the client should reconcile.
    Conflict(&'static str),
    /// Protocol violation with a public-safe reason.
    Rejected(&'static str),
}

/// Admission outcome of a review-feedback wake.
#[derive(Debug)]
pub enum FeedbackWakeOutcome {
    /// Recorded and stored once; the event to publish.
    Inserted(StoredEvent),
    /// The feedback revision's wake is already recorded with the same
    /// bindings, so nothing is stored or emitted again. A retry with a fresh
    /// timestamp (hence a different event ID) lands here too.
    Duplicate,
    /// Not admitted, with a public-safe reason.
    Rejected(&'static str),
}

fn invalid(message: &str) -> DbError {
    DbError::InvalidData(message.into())
}

/// Public-safe reason for a feedback create whose target cannot be confirmed
/// as the current review revision; deliberately identical for a missing,
/// deleted, moved-channel, or advanced head so nothing unreadable is disclosed.
const FEEDBACK_TARGET_STALE: &str = "feedback target is not the current review revision";

/// Admission rule for `synaxis.artifact-feedback/v1`: feedback is a create-only
/// record, and a create is accepted only while its `target_revision` is the
/// current, undeleted `synaxis.html-review` head of the named artifact in the
/// feedback's own channel, carrying the claimed Synaxis artifact and payload
/// digest. The head row is share-locked until commit, so a concurrent review
/// revision (which locks it for update) either commits first and makes this
/// target stale, or waits until this feedback is stored.
async fn reject_unless_feedback_targets_head(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    event: &Event,
    env: &ArtifactEnvelope,
) -> Result<Option<ArtifactOutcome>> {
    use buzz_core::artifact::{
        review_binds_feedback, validate_feedback_create, SYNAXIS_REVIEW_TYPE,
    };
    let target = match validate_feedback_create(event, env) {
        Ok(target) => target,
        Err(reason) => return Ok(Some(ArtifactOutcome::Rejected(reason))),
    };
    let head = sqlx::query(
        "SELECT event_id,channel_id,artifact_type,deleted FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2 FOR SHARE",
    )
    .bind(community.as_uuid())
    .bind(target.review_artifact)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(head) = head else {
        return Ok(Some(ArtifactOutcome::Conflict(FEEDBACK_TARGET_STALE)));
    };
    if head.get::<Vec<u8>, _>("event_id") != target.revision
        || head.get::<Uuid, _>("channel_id") != env.home
        || head.get::<String, _>("artifact_type") != SYNAXIS_REVIEW_TYPE
        || head.get::<bool, _>("deleted")
    {
        return Ok(Some(ArtifactOutcome::Conflict(FEEDBACK_TARGET_STALE)));
    }
    let content: Option<String> = sqlx::query_scalar(
        "SELECT content FROM events WHERE community_id=$1 AND id=$2 AND deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(&target.revision)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(match content {
        None => Some(ArtifactOutcome::Conflict(FEEDBACK_TARGET_STALE)),
        Some(content) if review_binds_feedback(&content, &target) => None,
        Some(_) => Some(ArtifactOutcome::Rejected(
            "feedback binding does not match the review revision",
        )),
    })
}

/// Public-safe reason for any `synaxis.html-review` revision outside the
/// adapter's lifecycle: a same-home update by the review's original signer.
const REVIEW_SIGNER_UPDATE_ONLY: &str =
    "a synaxis.html-review accepts only same-channel updates from its original signer";

/// Admission rule for `synaxis.html-review` revisions after the create: the
/// review is owned by the pubkey that signed the retained current head. Only an
/// `update` in the head's home channel by that same signer is accepted; `move`,
/// `delete`, and `restore` are refused for everyone, so another channel writer
/// can neither take over, remove, nor relocate the adapter's artifact. The
/// head row is already locked `FOR UPDATE` by the caller, so the signer read
/// and the head advance are one atomic step. A head whose payload is no longer
/// retained has no provable signer and fails closed.
async fn reject_unless_review_signer_updates(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    event: &Event,
    env: &ArtifactEnvelope,
    head: &sqlx::postgres::PgRow,
) -> Result<Option<ArtifactOutcome>> {
    if env.op != ArtifactOp::Update || head.get::<Uuid, _>("channel_id") != env.home {
        return Ok(Some(ArtifactOutcome::Rejected(REVIEW_SIGNER_UPDATE_ONLY)));
    }
    let signer: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT pubkey FROM events WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(head.get::<Vec<u8>, _>("event_id"))
            .fetch_optional(&mut *conn)
            .await?;
    Ok(match signer {
        Some(signer) if signer.as_slice() == event.pubkey.to_bytes().as_slice() => None,
        _ => Some(ArtifactOutcome::Rejected(REVIEW_SIGNER_UPDATE_ONLY)),
    })
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

/// Public-safe reason for a wake naming no feedback revision its author can
/// wake for; identical for every miss so nothing unreadable is disclosed.
const WAKE_NAMES_NO_FEEDBACK: &str = "wake names no accepted feedback revision of this author";

/// Public-safe reason for a second wake claim on one feedback revision.
const WAKE_BINDINGS_CONFLICT: &str =
    "a wake for this feedback revision already exists with different bindings";

/// The recorded wake for `(author, wake.feedback_event)`, judged against the
/// claim being made now: identical bindings are the same wake retried, any
/// other binding is refused. `None` when no wake is recorded yet.
async fn recorded_wake_outcome(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    author: &[u8],
    channel_id: Uuid,
    wake: &FeedbackWake,
) -> Result<Option<FeedbackWakeOutcome>> {
    let Some(row) = sqlx::query(
        "SELECT channel_id,feedback_artifact_id,reviewed_artifact_id,reviewed_event_id,root_event_id,parent_event_id,agent_pubkey FROM artifact_feedback_wakes WHERE community_id=$1 AND author_pubkey=$2 AND feedback_event_id=$3",
    )
    .bind(community.as_uuid())
    .bind(author)
    .bind(&wake.feedback_event)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(None);
    };
    let same = row.get::<Uuid, _>("channel_id") == channel_id
        && row.get::<Uuid, _>("feedback_artifact_id") == wake.feedback_artifact
        && row.get::<Uuid, _>("reviewed_artifact_id") == wake.reviewed_artifact
        && row.get::<Vec<u8>, _>("reviewed_event_id") == wake.reviewed_event
        && row.get::<Vec<u8>, _>("root_event_id") == wake.root_event
        && row.get::<Vec<u8>, _>("parent_event_id") == wake.parent_event
        && row.get::<Vec<u8>, _>("agent_pubkey") == wake.agent;
    Ok(Some(if same {
        FeedbackWakeOutcome::Duplicate
    } else {
        FeedbackWakeOutcome::Rejected(WAKE_BINDINGS_CONFLICT)
    }))
}

/// Whether the wake's feedback revision is a retained, accepted
/// `synaxis.artifact-feedback` revision signed by `author` in `channel_id`,
/// whose signed content names the reviewed artifact and revision the wake
/// claims. Whether that reviewed revision is still the head is irrelevant.
/// Feedback is create-only and immutable, so these facts cannot change after
/// acceptance.
async fn feedback_revision_is_authors(
    conn: &mut sqlx::PgConnection,
    community: CommunityId,
    author: &[u8],
    channel_id: Uuid,
    wake: &FeedbackWake,
) -> Result<bool> {
    let Some(row) = sqlx::query(
        "SELECT e.pubkey,e.channel_id,e.tags,e.content FROM events e JOIN artifact_revisions r ON r.community_id=e.community_id AND r.event_id=e.id WHERE e.community_id=$1 AND e.id=$2 AND e.kind=45010 AND e.deleted_at IS NULL",
    )
    .bind(community.as_uuid())
    .bind(&wake.feedback_event)
    .fetch_optional(&mut *conn)
    .await?
    else {
        return Ok(false);
    };
    if row.get::<Vec<u8>, _>("pubkey").as_slice() != author
        || row.get::<Option<Uuid>, _>("channel_id") != Some(channel_id)
    {
        return Ok(false);
    }
    let tags: serde_json::Value = row.get("tags");
    let has_tag = |name: &str, value: &str| {
        tags.as_array().is_some_and(|list| {
            list.iter().any(|tag| {
                tag.as_array().is_some_and(|parts| {
                    parts.len() == 2
                        && parts[0].as_str() == Some(name)
                        && parts[1].as_str() == Some(value)
                })
            })
        })
    };
    let feedback_artifact = wake.feedback_artifact.to_string();
    if !has_tag("type", buzz_core::artifact::SYNAXIS_FEEDBACK_TYPE)
        || !has_tag("d", &feedback_artifact)
    {
        return Ok(false);
    }
    let content: String = row.get("content");
    let Ok(content) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Ok(false);
    };
    let reviewed = content.get("reviewed");
    let named = |field: &str| reviewed.and_then(|r| r.get(field)).and_then(|v| v.as_str());
    let reviewed_artifact = wake.reviewed_artifact.to_string();
    let reviewed_event = hex::encode(&wake.reviewed_event);
    Ok(
        named("buzz_artifact_id") == Some(reviewed_artifact.as_str())
            && named("buzz_revision_event_id") == Some(reviewed_event.as_str()),
    )
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
            .execute(&mut *tx)
            .await?;
        // A coordinate lock handles the missing-row create race as well as edits.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("artifact:{}:{}", community.as_uuid(), env.id))
            .execute(&mut *tx)
            .await?;
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifact_revisions WHERE community_id=$1 AND event_id=$2)",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&mut *tx)
        .await?;
        if duplicate {
            return Ok(ArtifactOutcome::Duplicate);
        }
        if env.artifact_type == buzz_core::artifact::SYNAXIS_FEEDBACK_TYPE {
            if let Some(outcome) =
                reject_unless_feedback_targets_head(&mut tx, community, event, env).await?
            {
                return Ok(outcome);
            }
        }
        let head = sqlx::query(
            "SELECT event_id,channel_id,artifact_type,root,deleted FROM artifact_heads WHERE community_id=$1 AND artifact_id=$2 FOR UPDATE",
        )
        .bind(community.as_uuid())
        .bind(env.id)
        .fetch_optional(&mut *tx)
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
            (None, _) => return Ok(ArtifactOutcome::Conflict("artifact head unavailable")),
            (Some(head), op) => {
                let current: Vec<u8> = head.get("event_id");
                if env.prev.as_deref() != Some(current.as_slice()) {
                    return Ok(ArtifactOutcome::Conflict("artifact head changed"));
                }
                if env.artifact_type != head.get::<String, _>("artifact_type") {
                    return Ok(ArtifactOutcome::Rejected("artifact type is immutable"));
                }
                if env.artifact_type == buzz_core::artifact::SYNAXIS_REVIEW_TYPE {
                    if let Some(outcome) =
                        reject_unless_review_signer_updates(&mut tx, community, event, env, head)
                            .await?
                    {
                        return Ok(outcome);
                    }
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
                if op == ArtifactOp::Move && authorized_source != source {
                    return Ok(ArtifactOutcome::Conflict("artifact home changed"));
                }
                if op == ArtifactOp::Delete && env.root != old_root {
                    return Ok(ArtifactOutcome::Rejected("delete preserves root"));
                }
            }
        }
        if head.is_none() || env.root != old_root || source != Some(env.home) {
            if let Some(root) = &env.root {
                let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2 AND channel_id=$3 AND deleted_at IS NULL AND kind IN (9,40002,45001,45003))")
                    .bind(community.as_uuid()).bind(root).bind(env.home).fetch_one(&mut *tx).await?;
                if !exists {
                    return Ok(ArtifactOutcome::Rejected(
                        "root must be an existing conversation anchor in home",
                    ));
                }
            }
        }
        sqlx::query(
            "INSERT INTO artifact_revisions (community_id,event_id,artifact_id) VALUES ($1,$2,$3)",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(env.id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO artifact_heads (community_id,artifact_id,event_id,channel_id,artifact_type,root,deleted) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (community_id,artifact_id) DO UPDATE SET event_id=EXCLUDED.event_id,channel_id=EXCLUDED.channel_id,root=EXCLUDED.root,deleted=EXCLUDED.deleted")
            .bind(community.as_uuid()).bind(env.id).bind(event.id.as_bytes().as_slice()).bind(env.home).bind(&env.artifact_type).bind(&env.root).bind(env.op == ArtifactOp::Delete).execute(&mut *tx).await?;
        let (stored, _) =
            crate::event::insert_event_in_transaction(&mut tx, community, event, Some(env.home))
                .await?;
        crate::insert_mentions_in_transaction(&mut tx, community, event, Some(env.home)).await?;
        let mut accepted = vec![stored];
        if let (ArtifactOp::Move, Some(source), Some(prev)) = (env.op, source, &env.prev) {
            let removal = removal_marker(relay_keys, env.id, source, prev)?;
            let (stored, _) = crate::event::insert_event_in_transaction(
                &mut tx,
                community,
                &removal,
                Some(source),
            )
            .await?;
            accepted.push(stored);
        }
        tx.commit().await?;
        Ok(ArtifactOutcome::Accepted(accepted))
    }

    /// True when `event_id` is the recorded wake of one of `author`'s feedback
    /// revisions. The ledger survives redaction and retention, so a resend of an
    /// accepted wake is recognised even after its event is gone.
    pub async fn feedback_wake_event_accepted(
        &self,
        community: CommunityId,
        author: &[u8],
        event_id: &[u8],
    ) -> Result<bool> {
        let mut conn = crate::observability::acquire_writer(
            &self.pool,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM artifact_feedback_wakes WHERE community_id=$1 AND author_pubkey=$2 AND wake_event_id=$3)",
        )
        .bind(community.as_uuid())
        .bind(author)
        .bind(event_id)
        .fetch_one(&mut *conn)
        .await?)
    }

    /// Atomically admit one validated review-feedback wake.
    ///
    /// One transaction records the wake against `(author, feedback revision)`
    /// and stores the event, so there is at most one wake per feedback revision
    /// no matter how often or with what timestamp the author retries. A repeat
    /// with identical bindings is `Duplicate` (it stores nothing, even though
    /// its event ID may differ from the recorded wake's); a repeat with other
    /// bindings, a wake for another author's feedback, or one naming no
    /// accepted feedback revision is `Rejected`. The primary key serializes
    /// concurrent claimants: the loser blocks on the winner's claim, then reads
    /// the committed row.
    pub async fn accept_feedback_wake(
        &self,
        community: CommunityId,
        event: &Event,
        channel_id: Uuid,
        wake: &FeedbackWake,
        thread_meta: Option<crate::event::ThreadMetadataParams<'_>>,
    ) -> Result<FeedbackWakeOutcome> {
        let mut tx = crate::begin_community_event_write_transaction(
            &self.pool,
            community,
            crate::observability::WriterOperation::EventWrite,
        )
        .await?;
        let author = event.pubkey.to_bytes();
        if let Some(outcome) =
            recorded_wake_outcome(&mut tx, community, author.as_slice(), channel_id, wake).await?
        {
            return Ok(outcome);
        }
        if !feedback_revision_is_authors(&mut tx, community, author.as_slice(), channel_id, wake)
            .await?
        {
            return Ok(FeedbackWakeOutcome::Rejected(WAKE_NAMES_NO_FEEDBACK));
        }
        let claimed = sqlx::query(
            "INSERT INTO artifact_feedback_wakes (community_id,author_pubkey,feedback_event_id,wake_event_id,channel_id,feedback_artifact_id,reviewed_artifact_id,reviewed_event_id,root_event_id,parent_event_id,agent_pubkey) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT (community_id,author_pubkey,feedback_event_id) DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(author.as_slice())
        .bind(&wake.feedback_event)
        .bind(event.id.as_bytes().as_slice())
        .bind(channel_id)
        .bind(wake.feedback_artifact)
        .bind(wake.reviewed_artifact)
        .bind(&wake.reviewed_event)
        .bind(&wake.root_event)
        .bind(&wake.parent_event)
        .bind(&wake.agent)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if claimed == 0 {
            // A concurrent claimant committed first; judge this claim against
            // the row it recorded.
            return recorded_wake_outcome(&mut tx, community, author.as_slice(), channel_id, wake)
                .await?
                .ok_or_else(|| invalid("feedback wake claim vanished"));
        }
        let (stored, was_inserted) = crate::event::insert_event_with_thread_metadata_tx(
            &mut tx,
            community,
            event,
            Some(channel_id),
            thread_meta,
        )
        .await?;
        if !was_inserted {
            // The event ID is already stored, so this is a replay: undo the
            // claim rather than record a wake whose event we did not add.
            tx.rollback().await?;
            return Ok(FeedbackWakeOutcome::Duplicate);
        }
        crate::insert_mentions_in_transaction(&mut tx, community, event, Some(channel_id)).await?;
        tx.commit().await?;
        Ok(FeedbackWakeOutcome::Inserted(stored))
    }
}
