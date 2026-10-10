//! Operator signing for application-owned canonical metadata repair.

use anyhow::Result;
use buzz_core::{
    kind::{KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_MEMBERS, KIND_NIP29_GROUP_METADATA},
    TenantContext,
};
use buzz_db::Db;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

pub(crate) async fn repair(
    db: &Db,
    tenant: &TenantContext,
    channel: Uuid,
    keys: &Keys,
) -> Result<bool> {
    let mut write = db
        .begin_channel_metadata_write(tenant.community(), channel, keys.public_key())
        .await?;
    if !write.needs_snapshot().await? {
        write.rollback().await?;
        return Ok(false);
    }
    let tags = write.snapshot_tags().await?;
    let timestamp = write.snapshot_timestamp()?;
    let keys = keys.clone();
    let event = tokio::task::spawn_blocking(move || {
        EventBuilder::new(Kind::Custom(KIND_NIP29_GROUP_METADATA as u16), "")
            .tags(tags)
            .allow_self_tagging()
            .custom_created_at(timestamp)
            .sign_with_keys(&keys)
    })
    .await??;
    let max_bytes = buzz_core::relay::max_frame_bytes_from_env();
    write.store_snapshot(&event, max_bytes).await?;
    write.commit().await?;
    Ok(true)
}

#[derive(Default)]
pub(crate) struct RepairSummary {
    pub repaired: u64,
    pub skipped: u64,
    pub failed: u64,
}

pub(crate) async fn reconcile(
    db: &Db,
    tenant: &TenantContext,
    target: Option<Uuid>,
    keys: &Keys,
) -> Result<RepairSummary> {
    let mut cursor = Uuid::nil();
    let mut summary = RepairSummary::default();
    loop {
        let channels = if let Some(channel) = target {
            vec![channel]
        } else {
            db.channel_metadata_repair_page(tenant.community(), cursor)
                .await?
        };
        if channels.is_empty() {
            break;
        }
        for channel in channels {
            cursor = channel;
            match reconcile_one(db, tenant, channel, keys, target.is_some()).await {
                Ok(true) => summary.repaired += 1,
                Ok(false) => summary.skipped += 1,
                Err(error) => {
                    summary.failed += 1;
                    eprintln!("channel {channel}: reconciliation failed: {error:#}");
                }
            }
        }
        if target.is_some() {
            break;
        }
    }
    Ok(summary)
}

async fn reconcile_one(
    db: &Db,
    tenant: &TenantContext,
    channel: Uuid,
    keys: &Keys,
    roster_only: bool,
) -> Result<bool> {
    // Keep targeted reconciliation's contract: only refresh kind 39002.
    db.get_channel(tenant.community(), channel).await?;
    let repaired = if roster_only {
        false
    } else {
        repair(db, tenant, channel, keys).await?
    };
    // Auxiliary snapshots commit independently of metadata. A current 39000 is
    // not evidence that a previous bootstrap finished publishing 39001/39002.
    let discovery = if roster_only {
        Vec::new()
    } else {
        db.query_events_for_bootstrap(&buzz_db::event::EventQuery {
            kinds: Some(vec![
                KIND_NIP29_GROUP_ADMINS as i32,
                KIND_NIP29_GROUP_MEMBERS as i32,
            ]),
            authors: Some(vec![keys.public_key().to_bytes().to_vec()]),
            d_tag: Some(channel.to_string()),
            ..buzz_db::event::EventQuery::for_community(tenant.community())
        })
        .await?
    };
    let missing = |kind| {
        !discovery
            .iter()
            .any(|stored| buzz_core::kind::event_kind_u32(&stored.event) == kind)
    };
    let admins = !roster_only && missing(KIND_NIP29_GROUP_ADMINS);
    let members = roster_only || missing(KIND_NIP29_GROUP_MEMBERS);
    if !admins && !members {
        return Ok(repaired);
    }
    let roster = db.get_members(tenant.community(), channel).await?;
    for kind in [KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_MEMBERS] {
        if (kind == KIND_NIP29_GROUP_ADMINS && !admins)
            || (kind == KIND_NIP29_GROUP_MEMBERS && !members)
        {
            continue;
        }
        let mut tags = vec![Tag::parse(["d", &channel.to_string()])?];
        for member in &roster {
            let pk = hex::encode(&member.pubkey);
            if kind == KIND_NIP29_GROUP_ADMINS {
                if member.role == "owner" || member.role == "admin" {
                    tags.push(Tag::parse(["p", &pk, &member.role])?);
                }
            } else {
                tags.push(Tag::parse(["p", &pk, "", &member.role])?);
            }
        }
        // Include retired rows: signing the same tags in the same second as a
        // deleted head would reproduce its ID and silently lose ON CONFLICT.
        let previous: Option<i64> = sqlx::query_scalar(
            "SELECT EXTRACT(EPOCH FROM max(created_at))::bigint FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=$3 AND pubkey=$4",
        ).bind(tenant.community().as_uuid()).bind(channel).bind(kind as i32)
            .bind(keys.public_key().as_bytes().as_slice()).fetch_one(db.pool()).await?;
        let next = previous
            .map(|ts| u64::try_from(ts).ok().and_then(|ts| ts.checked_add(1)))
            .unwrap_or(Some(0))
            .ok_or_else(|| anyhow::anyhow!("discovery timestamp overflow"))?;
        let timestamp = nostr::Timestamp::from(next.max(nostr::Timestamp::now().as_secs()));
        let keys = keys.clone();
        let event = tokio::task::spawn_blocking(move || {
            EventBuilder::new(Kind::Custom(kind as u16), "")
                .tags(tags)
                .allow_self_tagging()
                .custom_created_at(timestamp)
                .sign_with_keys(&keys)
        })
        .await??;
        let (_, inserted) = db
            .replace_addressable_event(tenant.community(), &event, Some(channel))
            .await?;
        anyhow::ensure!(
            inserted,
            "discovery publication superseded; rerun reconciliation"
        );
    }
    Ok(true)
}
