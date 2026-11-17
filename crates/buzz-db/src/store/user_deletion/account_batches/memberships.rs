//! Membership removal and the three signed discovery lists share one commit.
use super::*;
use buzz_core::CommunityId;
use nostr::{EventBuilder, Kind, Tag, Timestamp};

impl UserDeletionStore {
    pub(super) async fn preflight_memberships(
        &self,
        community: Uuid,
        targets: &[String],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        timeouts(&mut tx).await?;
        self.preflight_community_membership(&mut tx, community, targets)
            .await?;
        let targets: Vec<Vec<u8>> = targets
            .iter()
            .map(|key| hex::decode(key).map_err(|_| invalid("invalid target")))
            .collect::<Result<_>>()?;
        let has_members: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM channel_members WHERE community_id=$1 AND pubkey=ANY($2))",
        )
        .bind(community)
        .bind(&targets)
        .fetch_one(&mut *tx)
        .await?;
        if !has_members {
            tx.rollback().await?;
            return Ok(());
        }
        let signer = self.account_snapshot_signer.as_ref().ok_or_else(|| {
            invalid(
            "membership cleanup requires the existing relay signing key (BUZZ_RELAY_PRIVATE_KEY)",
        )
        })?;
        let mismatch: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM events e WHERE e.community_id=$1 AND e.kind IN (39000,39001,39002) AND e.deleted_at IS NULL AND e.pubkey<>$3 AND EXISTS(SELECT 1 FROM channel_members m WHERE m.community_id=e.community_id AND m.channel_id=e.channel_id AND m.pubkey=ANY($2)))",
        ).bind(community).bind(&targets).bind(signer.public_key().to_bytes().as_slice()).fetch_one(&mut *tx).await?;
        if mismatch {
            return Err(invalid(
                "relay signing key differs from existing channel lists; no cleanup performed",
            ));
        }
        tx.rollback().await?;
        Ok(())
    }

    pub(super) async fn delete_memberships(
        &self,
        options: &Options,
        op: &Operation,
        writer: &mut impl Write,
        stage: Option<(&crate::entity_deletion::EntityLease, Uuid)>,
        summary: &mut AccountBatchSummary,
    ) -> Result<()> {
        let community = CommunityId::from_uuid(
            options
                .community
                .ok_or_else(|| invalid("membership cleanup requires a community"))?,
        );
        loop {
            let mut raw = self.pool.begin().await?;
            timeouts(&mut raw).await?;
            if let Some((lease, id)) = stage {
                validate_stage(&mut raw, lease, id, options).await?;
            }
            ownership::recheck(&mut raw, options, true).await?;
            // Discover without row locks: replacement locks must precede the
            // membership lock, which must precede membership row locks.
            let keys = select(&mut raw, options, op, None, false, summary).await?;
            if keys.is_empty() {
                raw.rollback().await?;
                break;
            }
            if summary.items == options.max_items {
                summary.supported_scope_exhausted = false;
                raw.rollback().await?;
                break;
            }
            let channel: Uuid = keys[0]["channel_id"]
                .as_str()
                .ok_or_else(|| invalid("missing channel key"))?
                .parse()
                .map_err(|_| invalid("invalid channel key"))?;
            let signer = self
                .account_snapshot_signer
                .as_ref()
                .ok_or_else(|| invalid("missing relay signing key"))?;
            let relay = signer.public_key().to_bytes();
            let mut tx = crate::AdmittedTx::admit(
                raw,
                &crate::deletion::DeletionStore::new(self.pool.clone()),
                community,
            )
            .await?;
            for kind in [39000, 39001, 39002] {
                crate::replaceable::take_replacement_lock(&mut tx, kind, &relay, Some(channel))
                    .await?;
            }
            crate::channel_members::acquire_channel_membership_lock(tx.conn(), community, channel)
                .await?;
            let mismatch: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND channel_id=$2 AND kind IN (39000,39001,39002) AND deleted_at IS NULL AND pubkey<>$3)")
                .bind(community.as_uuid()).bind(channel).bind(relay.as_slice()).fetch_one(tx.conn()).await?;
            if mismatch {
                return Err(invalid(
                    "relay signing key changed; channel removal rolled back",
                ));
            }
            batch(writer, options, op, &keys, summary)?;
            let removed = sqlx::query("DELETE FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND pubkey=decode($3,'hex')")
                .bind(community.as_uuid()).bind(channel).bind(&options.pubkey).execute(tx.conn()).await?.rows_affected();
            if removed != 1 {
                return Err(invalid("membership changed; rerun cleanup"));
            }
            let saved_lists = rebuild(&mut tx, channel, signer).await?;
            if let Some((lease, id)) = stage {
                validate_stage(tx.conn(), lease, id, options).await?;
            }
            tx.commit().await?;
            summary.items += 1;
            summary.batches += 1;
            line(
                writer,
                &json!({"record":"batch_committed", "number":summary.batches,
                "table":op.table,"action":op.action,"items":1,"channel":channel,"saved_channel_lists":saved_lists}),
            )?;
        }
        Ok(())
    }
}

pub(super) async fn rebuild(
    tx: &mut crate::AdmittedTx,
    channel_id: Uuid,
    signer: &nostr::Keys,
) -> Result<u8> {
    let community = tx.community();
    let channel = match crate::channel::select_channel(tx.conn(), community, channel_id).await {
        Ok(channel) => channel,
        Err(DbError::ChannelNotFound(_)) => {
            // Deleted channels can retain membership rows. Never resurrect their
            // discovery heads; retire any remaining heads with the removal.
            sqlx::query(superseded_with_mentions_sql!(
                "UPDATE events SET deleted_at=NOW() WHERE community_id=$1 AND channel_id=$2 AND kind IN (39000,39001,39002) AND deleted_at IS NULL"
            )).bind(community.as_uuid()).bind(channel_id).execute(tx.conn()).await?;
            return Ok(0);
        }
        Err(error) => return Err(error),
    };
    let members =
        crate::channel_members::select_active_members(tx.conn(), community, channel_id).await?;
    for kind in [39000, 39001, 39002] {
        let tags = if kind == 39000 {
            crate::channel::group_metadata_tags(&channel, &members)
                .into_iter()
                .map(Tag::parse)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| invalid(e.to_string()))?
        } else if kind == 39001 {
            crate::channel::group_admins_tags(
                &channel_id.to_string(),
                &channel.identity(),
                &members,
            )
            .map_err(|e| invalid(e.to_string()))?
        } else {
            crate::channel::group_members_tags(
                &channel_id.to_string(),
                Some(&channel.identity()),
                &members,
            )
            .map_err(|e| invalid(e.to_string()))?
        };
        let latest: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT max(created_at) FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=$3 AND pubkey=$4 AND deleted_at IS NULL")
            .bind(community.as_uuid()).bind(channel_id).bind(kind).bind(signer.public_key().to_bytes().as_slice()).fetch_one(tx.conn()).await?;
        let now = Timestamp::now().as_secs();
        let timestamp = latest
            .map(|t| (t.timestamp() as u64).saturating_add(1))
            .unwrap_or(now)
            .max(now);
        let event = EventBuilder::new(Kind::Custom(kind as u16), "")
            .tags(tags)
            .allow_self_tagging()
            .custom_created_at(Timestamp::from(timestamp))
            .sign_with_keys(signer)
            .map_err(|e| invalid(e.to_string()))?;
        let (_, inserted) =
            crate::replaceable::replace_addressable_event_under_lock(tx, &event, Some(channel_id))
                .await?;
        if !inserted {
            return Err(invalid(
                "channel list replacement failed; membership removal rolled back",
            ));
        }
    }
    Ok(3)
}
