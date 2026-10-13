use buzz_core::channel_labels::{verify_snapshot, LabelSet};
use nostr::{Event, Tag};

use super::ChannelMetadataWrite;
use crate::{DbError, Result};

impl ChannelMetadataWrite {
    /// Build complete metadata from locked current state, never captured caller tags.
    pub async fn snapshot_tags(&mut self) -> Result<Vec<Tag>> {
        let channel = self.channel()?;
        let mut parts = vec![
            vec!["d".into(), self.channel_id.to_string()],
            vec!["name".into(), channel.name.clone()],
            vec!["closed".into()],
            vec!["t".into(), channel.channel_type.clone()],
            vec![if channel.visibility == "private" {
                "private"
            } else {
                "public"
            }
            .into()],
        ];
        for (key, value) in [
            ("about", &channel.description),
            ("topic", &channel.topic),
            ("purpose", &channel.purpose),
        ] {
            if let Some(value) = value.as_ref().filter(|value| !value.is_empty()) {
                parts.push(vec![key.into(), value.clone()]);
            }
        }
        if channel.archived_at.is_some() {
            parts.push(vec!["archived".into(), "true".into()]);
        }
        if let Some(ttl) = channel.ttl_seconds {
            parts.push(vec!["ttl".into(), ttl.to_string()]);
        }
        if let Some(deadline) = channel.ttl_deadline {
            parts.push(vec!["ttl_deadline".into(), deadline.to_rfc3339()]);
        }
        if channel.channel_type == "dm" {
            parts.push(vec!["hidden".into()]);
            let members: Vec<Vec<u8>> = sqlx::query_scalar(
                "SELECT pubkey FROM channel_members WHERE community_id = $1 AND channel_id = $2 \
                 AND removed_at IS NULL ORDER BY pubkey",
            )
            .bind(self.tx.community().as_uuid())
            .bind(self.channel_id)
            .fetch_all(self.tx.conn())
            .await?;
            parts.extend(
                members
                    .into_iter()
                    .map(|key| vec!["p".into(), hex::encode(key)]),
            );
        }
        parts.extend(self.labels.snapshot_tags());
        parts
            .into_iter()
            .map(|parts| Tag::parse(parts).map_err(|e| DbError::InvalidData(e.to_string())))
            .collect()
    }

    /// Timestamp for a new snapshot, strictly newer than the prior live head.
    pub fn snapshot_timestamp(&self) -> Result<nostr::Timestamp> {
        let now = nostr::Timestamp::now().as_secs();
        let next = self
            .previous
            .as_ref()
            .map(|e| e.event.created_at.as_secs().checked_add(1))
            .unwrap_or(Some(now))
            .ok_or_else(|| DbError::InvalidData("metadata timestamp overflow".into()))?;
        Ok(nostr::Timestamp::from(next.max(now)))
    }

    /// Compare canonical content, not existence. A no-op reuses a sound snapshot.
    pub async fn needs_snapshot(&mut self) -> Result<bool> {
        let tags = self.snapshot_tags().await?;
        let Some(previous) = self.previous.as_ref() else {
            return Ok(true);
        };
        if !self.has_current_label_snapshot().await? {
            return Ok(true);
        }
        // The existing TTL protocol refreshes deadlines on event commits. It is
        // not a reason to publish a fresh snapshot for a label no-op. Compare all
        // other fields without imposing a new order on ordinary metadata tags.
        let comparable = |tags: Vec<Tag>| {
            let mut parts: Vec<_> = tags
                .into_iter()
                .map(|t| t.as_slice().to_vec())
                .filter(|p| p[0] != "ttl_deadline")
                .collect();
            parts.sort();
            parts
        };
        Ok(!previous.event.content.is_empty()
            || comparable(tags) != comparable(previous.event.tags.iter().cloned().collect()))
    }

    /// Check only the atomic label projection, not commit-then-publish metadata.
    pub(super) async fn has_current_label_snapshot(&self) -> Result<bool> {
        self.channel()?;
        let Some(previous) = self.previous.as_ref() else {
            return Ok(false);
        };
        Ok(
            verify_snapshot_async(&previous.event, self.relay, self.channel_id)
                .await?
                .ok()
                .as_ref()
                == Some(&self.labels),
        )
    }

    /// Store a freshly signed snapshot on this transaction after validating its projection.
    ///
    /// The caller signs `snapshot_tags()` with `snapshot_timestamp()`. No network
    /// work or pool checkout belongs between capture and this method.
    pub async fn store_snapshot(&mut self, event: &Event, max_bytes: usize) -> Result<()> {
        if self.published.is_some() || self.must_rollback {
            return Err(DbError::InvalidData(
                "one snapshot per metadata transaction".into(),
            ));
        }
        let expected = self.snapshot_tags().await?;
        let labels = verify_snapshot_async(event, self.relay, self.channel_id)
            .await?
            .map_err(|e| DbError::InvalidData(e.to_string()))?;
        if labels != self.labels
            || !event.content.is_empty()
            || event.tags.iter().ne(expected.iter())
            || self
                .previous
                .as_ref()
                .is_some_and(|old| event.created_at <= old.event.created_at)
            || serde_json::to_vec(event)?.len() > max_bytes
        {
            return Err(DbError::InvalidData(
                "snapshot does not match locked metadata or limits".into(),
            ));
        }
        self.must_rollback = true;
        retire_previous_snapshots(&mut self.tx, self.relay, self.channel_id).await?;
        let (stored, inserted) =
            crate::event::insert_event_in_transaction(&mut self.tx, event, Some(self.channel_id))
                .await?;
        if !inserted {
            return Err(DbError::InvalidData(
                "metadata snapshot already exists".into(),
            ));
        }
        self.published = Some(stored);
        self.must_rollback = false;
        Ok(())
    }
}

// Cryptography cannot occupy the async runtime while a transaction waits on it.
async fn verify_snapshot_async(
    event: &Event,
    relay: nostr::PublicKey,
    channel: uuid::Uuid,
) -> Result<std::result::Result<LabelSet, buzz_core::channel_labels::LabelError>> {
    let event = event.clone();
    tokio::task::spawn_blocking(move || verify_snapshot(&event, relay, channel))
        .await
        .map_err(|error| {
            DbError::InvalidData(format!("snapshot verification task failed: {error}"))
        })
}

async fn retire_previous_snapshots(
    tx: &mut crate::AdmittedTx,
    relay: nostr::PublicKey,
    channel: uuid::Uuid,
) -> Result<()> {
    sqlx::query(
        "UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND kind = 39000 \
         AND pubkey = $2 AND channel_id = $3 AND deleted_at IS NULL",
    )
    .bind(tx.community().as_uuid())
    .bind(relay.as_bytes().as_slice())
    .bind(channel)
    .execute(tx.conn())
    .await?;
    Ok(())
}
