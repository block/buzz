//! Locked admin/member discovery projections shared by relay and operator repair.

use buzz_core::{
    kind::{KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_MEMBERS},
    CommunityId,
};
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{acquire_channel_membership_lock, row_to_member_record, MemberRecord};
use crate::{AdmittedTx, Db, DbError, Result};

/// An active member roster captured while holding the channel's membership
/// serialization lock on one writer connection.
pub struct LockedMemberSnapshot {
    /// Canonical active members captured behind the lock.
    pub members: Vec<MemberRecord>,
    channel_id: Uuid,
    relay_pubkey: Vec<u8>,
    kind: u32,
    tx: AdmittedTx,
}

impl LockedMemberSnapshot {
    /// The channel whose membership lock this guard holds.
    pub fn channel_id(&self) -> Uuid {
        self.channel_id
    }

    /// Build the canonical admin/member tags from the roster held by this guard.
    pub fn snapshot_tags(&self) -> Result<Vec<nostr::Tag>> {
        discovery_tags(self.channel_id, self.kind, &self.members)
    }

    /// Advance beyond every prior head, including retired rows, so a repair
    /// cannot reproduce a deleted event ID within the same second.
    pub async fn snapshot_timestamp(&mut self) -> Result<nostr::Timestamp> {
        let value: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM events WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND channel_id = $4 ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(self.tx.community().as_uuid())
        .bind(self.kind as i32)
        .bind(self.relay_pubkey.as_slice())
        .bind(self.channel_id)
        .fetch_optional(self.tx.conn())
        .await?;
        let next = value
            .map(|timestamp| {
                u64::try_from(timestamp.timestamp())
                    .ok()
                    .and_then(|ts| ts.checked_add(1))
            })
            .unwrap_or(Some(0))
            .ok_or_else(|| DbError::InvalidData("discovery timestamp overflow".into()))?;
        Ok(nostr::Timestamp::from(
            next.max(nostr::Timestamp::now().as_secs()),
        ))
    }

    /// Replace a member snapshot, retaining the original member-only API.
    pub async fn replace_member_event(
        &mut self,
        event: &nostr::Event,
    ) -> Result<(buzz_core::StoredEvent, bool)> {
        if self.kind != KIND_NIP29_GROUP_MEMBERS {
            return Err(DbError::InvalidData(
                "member replacement requires a member guard".into(),
            ));
        }
        self.replace_discovery_event(event).await
    }

    /// Replace the relay-authored discovery snapshot on this guard's existing
    /// connection. The membership lock therefore spans capture and replacement
    /// without a nested pool checkout.
    pub async fn replace_discovery_event(
        &mut self,
        event: &nostr::Event,
    ) -> Result<(buzz_core::StoredEvent, bool)> {
        let community_id = self.tx.community();
        let channel_id = self.channel_id;
        if event.pubkey.to_bytes().as_slice() != self.relay_pubkey.as_slice()
            || crate::event::extract_d_tag(event) != Some(channel_id.to_string())
        {
            return Err(DbError::InvalidData(
                "member snapshot replacement does not match its locked coordinate".into(),
            ));
        }
        let kind = buzz_core::kind::event_kind_i32(event);
        if kind != self.kind as i32 {
            return Err(DbError::InvalidData(
                "discovery snapshot kind does not match its locked coordinate".into(),
            ));
        }
        let pubkey = event.pubkey.to_bytes();
        let created_at_secs = event.created_at.as_secs() as i64;
        let created_at = chrono::DateTime::from_timestamp(created_at_secs, 0)
            .ok_or(DbError::InvalidTimestamp(created_at_secs))?;
        let existing: Option<(chrono::DateTime<Utc>, Vec<u8>)> = sqlx::query_as(
            "SELECT created_at, id FROM events WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND channel_id = $4 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(kind)
        .bind(pubkey.as_slice())
        .bind(channel_id)
        .fetch_optional(self.tx.conn())
        .await?;
        let incoming_id = event.id.as_bytes().as_slice();
        if let Some((existing_ts, existing_id)) = existing {
            if created_at < existing_ts
                || (created_at == existing_ts && incoming_id >= existing_id.as_slice())
            {
                return Ok((
                    buzz_core::StoredEvent::with_received_at(
                        event.clone(),
                        Utc::now(),
                        Some(channel_id),
                        false,
                    ),
                    false,
                ));
            }
        }
        sqlx::query("UPDATE events SET deleted_at = NOW() WHERE community_id = $1 AND kind = $2 AND pubkey = $3 AND channel_id = $4 AND deleted_at IS NULL")
            .bind(community_id.as_uuid()).bind(kind).bind(pubkey.as_slice()).bind(channel_id)
            .execute(self.tx.conn()).await?;
        let received_at = Utc::now();
        let tags = serde_json::to_value(&event.tags)?;
        let sig = event.sig.serialize();
        let inserted = sqlx::query("INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) ON CONFLICT DO NOTHING")
            .bind(community_id.as_uuid()).bind(event.id.as_bytes().as_slice())
            .bind(pubkey.as_slice()).bind(created_at).bind(kind).bind(tags)
            .bind(&event.content).bind(sig.as_slice()).bind(received_at).bind(channel_id)
            .bind(crate::event::extract_d_tag(event)).execute(self.tx.conn()).await?;
        if inserted.rows_affected() == 0 {
            return Err(DbError::InvalidData(
                "member snapshot event id already exists".into(),
            ));
        }
        crate::store::event_follow_up::after_admitted_insert(
            &mut self.tx,
            event.id.as_bytes().as_slice(),
            kind,
            Some(channel_id),
        )
        .await?;
        crate::insert_mentions_in_transaction(&mut self.tx, event, Some(channel_id)).await?;
        Ok((
            buzz_core::StoredEvent::with_received_at(
                event.clone(),
                received_at,
                Some(channel_id),
                true,
            ),
            true,
        ))
    }

    /// Commit the replacement and release the membership lock.
    pub async fn release(self) -> Result<()> {
        self.tx.commit().await?;
        Ok(())
    }
}

/// Capture all active members while holding the same per-channel lock used by
/// membership writers.
///
/// The returned guard must remain alive through publication. This prevents a
/// rolling relay from publishing an older roster after a concurrent add or
/// remove has committed and published newer membership state.
pub async fn lock_member_snapshot(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    relay_pubkey: &[u8],
) -> Result<LockedMemberSnapshot> {
    lock_snapshot(
        pool,
        community_id,
        channel_id,
        relay_pubkey,
        KIND_NIP29_GROUP_MEMBERS,
    )
    .await
}

impl Db {
    /// Capture an admin roster behind the same membership lock as role writers.
    /// The guard owns its 39001 replacement coordinate through publication.
    pub async fn lock_admin_snapshot(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        relay_pubkey: &[u8],
    ) -> Result<LockedMemberSnapshot> {
        lock_snapshot(
            self.pool(),
            community_id,
            channel_id,
            relay_pubkey,
            KIND_NIP29_GROUP_ADMINS,
        )
        .await
    }
}

async fn lock_snapshot(
    pool: &PgPool,
    community_id: CommunityId,
    channel_id: Uuid,
    relay_pubkey: &[u8],
    kind: u32,
) -> Result<LockedMemberSnapshot> {
    let mut tx = crate::begin_community_event_write_transaction(
        pool,
        community_id,
        crate::observability::WriterOperation::EventWrite,
    )
    .await?;
    // Match the canonical replacement writer's lock order. Old binaries take
    // this key before INSERT; migration 0032 then takes the membership key in
    // the INSERT trigger. Taking both in that order avoids mixed-version
    // duplicate heads without introducing a lock-order inversion.
    let replacement_lock = crate::replaceable::event_replacement_lock_key(
        community_id,
        kind as i32,
        relay_pubkey,
        Some(channel_id.as_bytes()),
    );
    crate::observability::observe_advisory_lock(
        crate::observability::LockType::Replacement,
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(replacement_lock)
            .execute(tx.conn()),
    )
    .await?;
    acquire_channel_membership_lock(tx.conn(), community_id, channel_id).await?;
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))")
        .bind(crate::channel::channel_ttl_lock_key(
            community_id,
            channel_id,
        ))
        .execute(tx.conn())
        .await?;
    let live: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM channels WHERE community_id=$1 AND id=$2 AND deleted_at IS NULL FOR NO KEY UPDATE",
    ).bind(community_id.as_uuid()).bind(channel_id).fetch_optional(tx.conn()).await?;
    if live.is_none() {
        return Err(DbError::ChannelNotFound(channel_id));
    }
    let rows = sqlx::query(
        r#"
        SELECT cm.channel_id, cm.pubkey, cm.role::text AS role, cm.joined_at, cm.invited_by, cm.removed_at
        FROM channel_members cm
        JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id AND c.deleted_at IS NULL
        WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.removed_at IS NULL
        ORDER BY cm.joined_at ASC
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .fetch_all(tx.conn())
    .await?;
    let members = rows
        .into_iter()
        .map(row_to_member_record)
        .collect::<Result<Vec<_>>>()?;
    Ok(LockedMemberSnapshot {
        members,
        channel_id,
        relay_pubkey: relay_pubkey.to_vec(),
        kind,
        tx,
    })
}

fn discovery_tags(channel: Uuid, kind: u32, members: &[MemberRecord]) -> Result<Vec<nostr::Tag>> {
    let tag = |fields: Vec<String>| {
        nostr::Tag::parse(fields).map_err(|error| DbError::InvalidData(error.to_string()))
    };
    let mut tags = vec![tag(vec!["d".into(), channel.to_string()])?];
    for member in members {
        let pk = hex::encode(&member.pubkey);
        if kind == KIND_NIP29_GROUP_ADMINS {
            if member.role == "owner" || member.role == "admin" {
                tags.push(tag(vec!["p".into(), pk, member.role.clone()])?);
            }
        } else {
            // NIP-29: ["p", pubkey, relay_url, role]. The signer is the relay.
            tags.push(tag(vec!["p".into(), pk, "".into(), member.role.clone()])?);
        }
    }
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_members_snapshot_keeps_members_past_one_thousand() {
        let channel_id = Uuid::new_v4();
        let members: Vec<MemberRecord> = (0_u16..1_501)
            .map(|index| MemberRecord {
                channel_id,
                pubkey: vec![(index >> 8) as u8, index as u8],
                role: if index == 1_500 { "owner" } else { "member" }.to_string(),
                joined_at: chrono::Utc::now(),
                invited_by: None,
                removed_at: None,
            })
            .collect();

        let tags =
            discovery_tags(channel_id, KIND_NIP29_GROUP_MEMBERS, &members).expect("build tags");
        assert_eq!(tags.len(), 1_502, "d tag plus every member p tag");

        let late_pubkey = hex::encode(&members[1_500].pubkey);
        assert!(tags.iter().any(|tag| {
            let fields = tag.as_slice();
            fields.len() == 4
                && fields[0] == "p"
                && fields[1] == late_pubkey
                && fields[3] == "owner"
        }));
    }
}
