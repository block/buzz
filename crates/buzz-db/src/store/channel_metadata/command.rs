use buzz_core::channel::{canonical_channel_name, ChannelType, ChannelVisibility};
use buzz_core::channel_labels::{LabelCommand, LabelSet};
use nostr::Event;

use super::{read_channel, ChannelMetadataWrite};
use crate::{DbError, Result};

/// Validated ordinary creation fields supplied by the relay's existing parser.
pub struct ChannelCreate {
    /// Channel display name.
    pub name: String,
    /// Stream, forum, DM or workflow; only stream/forum may have labels.
    pub channel_type: ChannelType,
    /// Existing channel visibility contract.
    pub visibility: ChannelVisibility,
    /// Optional description.
    pub description: Option<String>,
    /// Resolved ephemeral TTL, including deployment policy.
    pub ttl_seconds: Option<i32>,
}

/// A serialized command decision, before committing or rolling back the guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandApplication {
    /// New command/evidence/state staged; caller must publish if needed and commit.
    Applied,
    /// Retained proof of earlier commit, after current authority/lifecycle checks.
    Committed,
    /// A legacy stored command has no proof of application; never reapply it.
    Unknown,
    /// A different create targets a used UUID (no stored metadata is disclosed).
    Conflict,
    /// Current authority or lifecycle denies this submission.
    Restricted,
    /// The command is no longer within the admission window after lock acquisition.
    Expired,
}

impl ChannelMetadataWrite {
    async fn has_label_authority(&mut self, actor: &[u8]) -> Result<bool> {
        // Ordinary membership writers share this guard's lock. Administrative
        // kicks need not: an overlapping command may serialize before the kick.
        // Ownership is write-once in user::set_agent_owner; check every owner.
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM channel_members cm \
             LEFT JOIN users u ON u.community_id = cm.community_id AND u.pubkey = cm.pubkey \
             WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.removed_at IS NULL \
             AND ((cm.pubkey = $3 AND cm.role IN ('owner', 'admin')) \
               OR (cm.role = 'owner' AND u.agent_owner_pubkey = $3)))",
        )
        .bind(self.tx.community().as_uuid())
        .bind(self.channel_id)
        .bind(actor)
        .fetch_one(self.tx.conn())
        .await?)
    }

    /// Stage one verified, admitted command on the locked channel.
    ///
    /// Signature, authenticated signer, host, token restrictions and feature
    /// enablement must already have passed relay admission. Current timestamp,
    /// channel role and lifecycle are checked again after serialization. Non-new
    /// outcomes never modify state and must not dispatch or republish anything.
    pub async fn apply_command(
        &mut self,
        event: &Event,
        create: Option<&ChannelCreate>,
    ) -> Result<CommandApplication> {
        if self.command_applied || self.published.is_some() || self.must_rollback {
            return Err(DbError::InvalidData(
                "one command per metadata transaction".into(),
            ));
        }
        // A failed statement must never leave a committable partial command.
        self.must_rollback = true;
        let result = self.apply_command_inner(event, create).await;
        if result.is_ok() {
            self.must_rollback = false;
        }
        result
    }

    async fn apply_command_inner(
        &mut self,
        event: &Event,
        create: Option<&ChannelCreate>,
    ) -> Result<CommandApplication> {
        let command = LabelCommand::parse(event)
            .map_err(|e| DbError::InvalidData(e.to_string()))?
            .ok_or_else(|| DbError::InvalidData("not a covered label command".into()))?;
        if command.channel() != self.channel_id {
            return Err(DbError::InvalidData(
                "command does not match locked channel".into(),
            ));
        }
        if event
            .created_at
            .as_secs()
            .abs_diff(nostr::Timestamp::now().as_secs())
            > 900
        {
            return Ok(CommandApplication::Expired);
        }
        // Soft-deleted commands retain their provenance. A current set is not a receipt.
        let evidence: Option<bool> = sqlx::query_scalar(
            "SELECT nip_cl_applied FROM events WHERE community_id = $1 AND id = $2 \
             AND created_at = to_timestamp($3)",
        )
        .bind(self.tx.community().as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(event.created_at.as_secs() as f64)
        .fetch_optional(self.tx.conn())
        .await?;
        if evidence.is_some() || matches!(command, LabelCommand::Mutate { .. }) {
            if !self
                .channel
                .as_ref()
                .is_some_and(|c| c.deleted_at.is_none() && c.archived_at.is_none())
                || !self.has_label_authority(event.pubkey.as_bytes()).await?
            {
                return Ok(CommandApplication::Restricted);
            }
            if let Some(committed) = evidence {
                return Ok(if committed {
                    CommandApplication::Committed
                } else {
                    CommandApplication::Unknown
                });
            }
        }
        match &command {
            LabelCommand::Create { labels, .. } => {
                if self.channel.is_some() {
                    return Ok(CommandApplication::Conflict);
                }
                let create =
                    create.ok_or_else(|| DbError::InvalidData("missing creation fields".into()))?;
                if !labels.is_empty()
                    && !matches!(
                        create.channel_type,
                        ChannelType::Stream | ChannelType::Forum
                    )
                {
                    return Err(DbError::InvalidData(
                        "channel type does not support labels".into(),
                    ));
                }
                self.create_channel(event, create, labels).await?;
            }
            LabelCommand::Mutate { .. } => {
                if !matches!(self.channel()?.channel_type.as_str(), "stream" | "forum") {
                    return Err(DbError::InvalidData(
                        "channel type does not support labels".into(),
                    ));
                }
                let labels = self
                    .labels
                    .apply(&command)
                    .map_err(|e| DbError::InvalidData(e.to_string()))?;
                if labels != self.labels {
                    sqlx::query("UPDATE channels SET labels = $3, updated_at = NOW() WHERE community_id = $1 AND id = $2")
                        .bind(self.tx.community().as_uuid()).bind(self.channel_id).bind(labels.values())
                        .execute(self.tx.conn()).await?;
                    self.labels = labels;
                }
            }
        }
        let (_, inserted) =
            crate::event::insert_event_in_transaction(&mut self.tx, event, Some(self.channel_id))
                .await?;
        if !inserted {
            // A competing unsupported/legacy insert is not proof of application.
            // Returning an error forces the caller to discard this transaction.
            return Err(DbError::InvalidData(
                "command appeared without serialized evidence".into(),
            ));
        }
        mark_command_applied(&mut self.tx, event).await?;
        self.command_applied = true;
        Ok(CommandApplication::Applied)
    }

    async fn create_channel(
        &mut self,
        event: &Event,
        create: &ChannelCreate,
        labels: &LabelSet,
    ) -> Result<()> {
        let name = canonical_channel_name(&create.name);
        if name.is_empty() || name.chars().count() > 255 {
            return Err(DbError::InvalidData("invalid channel name".into()));
        }
        sqlx::query(
            "INSERT INTO channels (community_id, id, name, channel_type, visibility, description, created_by, ttl_seconds, ttl_deadline, labels) \
             VALUES ($1, $2, $3, $4::channel_type, $5::channel_visibility, $6, $7, $8, \
             CASE WHEN $8 IS NOT NULL THEN NOW() + make_interval(secs => $8) ELSE NULL END, $9)",
        ).bind(self.tx.community().as_uuid()).bind(self.channel_id).bind(name)
            .bind(create.channel_type.as_str()).bind(create.visibility.as_str()).bind(&create.description)
            .bind(event.pubkey.as_bytes().as_slice()).bind(create.ttl_seconds).bind(labels.values())
            .execute(self.tx.conn()).await?;
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by) \
             VALUES ($1, $2, $3, 'owner', $3)",
        )
        .bind(self.tx.community().as_uuid())
        .bind(self.channel_id)
        .bind(event.pubkey.as_bytes().as_slice())
        .execute(self.tx.conn())
        .await?;
        (self.channel, self.labels) = read_channel(&mut self.tx, self.channel_id).await?;
        Ok(())
    }
}

// Only the serialized command path calls this; requiring the admitted transaction
// keeps provenance writes inside the supported-writer boundary.
async fn mark_command_applied(tx: &mut crate::AdmittedTx, event: &Event) -> Result<()> {
    sqlx::query(
        "UPDATE events SET nip_cl_applied = TRUE WHERE community_id = $1 AND id = $2 \
         AND created_at = to_timestamp($3)",
    )
    .bind(tx.community().as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .bind(event.created_at.as_secs() as f64)
    .execute(tx.conn())
    .await?;
    Ok(())
}
