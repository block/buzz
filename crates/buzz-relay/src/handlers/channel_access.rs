//! What one viewer may do in named channels (NOSTR.md, Channel access), and
//! the push that tells live subscribers to ask again.
//!
//! The answer applies the rules the read and post gates apply
//! ([`super::ingest::check_channel_membership`], [`super::ingest::may_post`],
//! [`super::ingest::closed_by_archive`]). It is a snapshot: every post is
//! still checked when it arrives.

use buzz_core::channel::MemberRole;
use buzz_core::kind::KIND_CHANNEL_ACCESS_CHANGED;
use buzz_core::{CommunityId, TenantContext};
use buzz_db::channel::{ChannelJoinRole, ChannelRecord};
use nostr::{EventBuilder, Kind, Tag};
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

use crate::state::AppState;

/// The most channels one request may name.
pub const MAX_IDS: usize = 100;

/// The most channels a request without `ids` answers. Past it, the whole
/// request fails and the client names channels instead.
pub const MAX_REACHABLE: usize = 500;

/// One readable channel's answer for the viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelAccess {
    /// The channel.
    pub id: Uuid,
    /// Always true: unreadable channels are left out.
    pub read: bool,
    /// Whether a kind:9 from the viewer would pass the post gates now.
    pub post: bool,
    /// Whether the viewer has an active member row.
    pub joined: bool,
    /// The role the viewer acts with: their row's, else an open channel's
    /// `join_role`, else what the parents give.
    pub role: &'static str,
}

/// The viewer's answer for each readable channel in `ids`, in request order.
/// Unreadable channels are left out; any lookup failure fails the whole
/// batch, so a missing entry always means "cannot read".
pub(crate) async fn answer(
    state: &AppState,
    community_id: CommunityId,
    pubkey: &[u8],
    ids: &[Uuid],
) -> Result<Vec<ChannelAccess>, buzz_db::DbError> {
    // A push can arrive before another node has dropped its cached rows for
    // this viewer, so this node reads them fresh.
    state.invalidate_membership_local(community_id, Uuid::nil(), pubkey);
    let mut answers = Vec::with_capacity(ids.len());
    for &id in ids {
        if let Some(access) = access(state, community_id, pubkey, id).await? {
            answers.push(access);
        }
    }
    Ok(answers)
}

/// The viewer's answer for every channel they can reach, ordered by id:
/// channels they joined, listed open channels, and the children and
/// grandchildren of both. These are the channels a read that names none
/// covers, so artifact conversations and unlisted channels stay out.
/// `None` past [`MAX_REACHABLE`] candidates, so a list is never cut short.
pub(crate) async fn reachable(
    state: &AppState,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Option<Vec<ChannelAccess>>, buzz_db::DbError> {
    let mut ids = state
        .db
        .get_accessible_channel_ids(community_id, pubkey)
        .await?;
    if ids.len() > MAX_REACHABLE {
        return Ok(None);
    }
    ids.sort_unstable();
    answer(state, community_id, pubkey, &ids).await.map(Some)
}

fn role_name(role: MemberRole) -> &'static str {
    match role {
        MemberRole::Owner => "owner",
        MemberRole::Admin => "admin",
        MemberRole::Member => "member",
        MemberRole::Guest => "guest",
        // The contract has no bot role: a bot acts as a member.
        MemberRole::Bot => "member",
    }
}

/// `pubkey`'s access to `channel_id`; `None` when they cannot read it,
/// whether it is private, deleted or missing.
async fn access(
    state: &AppState,
    community_id: CommunityId,
    pubkey: &[u8],
    channel_id: Uuid,
) -> Result<Option<ChannelAccess>, buzz_db::DbError> {
    let channel = match state
        .db
        .get_channel_for_event_write(community_id, channel_id)
        .await
    {
        Ok(channel) if channel.deleted_at.is_none() => channel,
        Ok(_) | Err(buzz_db::DbError::ChannelNotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some((role, joined)) = acting_role(state, community_id, &channel, pubkey).await? else {
        return Ok(None);
    };
    let post =
        role.can_post() && !super::ingest::closed_by_archive(community_id, state, &channel).await?;
    Ok(Some(ChannelAccess {
        id: channel_id,
        read: true,
        post,
        joined,
        role: role_name(role),
    }))
}

/// The role `pubkey` acts with in `channel` and whether it is their own
/// row's, or `None` when they cannot read it. Mirrors
/// [`super::ingest::may_post`]: an active row's role, else an open channel's
/// `join_role` (failing closed to guest), else what the parents give. A row
/// removed between the two reads counts as no row.
async fn acting_role(
    state: &AppState,
    community_id: CommunityId,
    channel: &ChannelRecord,
    pubkey: &[u8],
) -> Result<Option<(MemberRole, bool)>, buzz_db::DbError> {
    if state
        .is_member_cached(community_id, channel.id, pubkey)
        .await?
    {
        let role = state
            .db
            .get_member_role(community_id, channel.id, pubkey)
            .await?;
        if let Some(role) = role.and_then(|r| r.parse().ok()) {
            return Ok(Some((role, true)));
        }
    }
    if channel.visibility == "open" {
        let role = match ChannelJoinRole::parse_fail_closed(&channel.join_role) {
            ChannelJoinRole::Member => MemberRole::Member,
            ChannelJoinRole::Guest => MemberRole::Guest,
        };
        return Ok(Some((role, false)));
    }
    Ok(
        super::parent_access::inherited_role(state, community_id, channel, pubkey)
            .await?
            .map(|role| (role, false)),
    )
}

/// Tell live subscribers of `channel` and of every private channel under it,
/// whose readers can come from it, to ask again. Call after a committed
/// change to who may read or post there, once per change. The pushes are
/// sent off the caller's path, best effort: a failure is logged, and a
/// client also asks again when it reconnects or a post is refused. Fan-out
/// delivers a push only to subscribers who can still read the channel; the
/// eviction beside each change closes the rest. Open channels under it are
/// not told: their access does not come from parents.
pub fn announce_changed(tenant: &TenantContext, state: &AppState, channel: Uuid) {
    let (tenant, db, pubsub, keys) = (
        tenant.clone(),
        state.db.clone(),
        state.pubsub.clone(),
        state.relay_keypair.clone(),
    );
    tokio::spawn(async move {
        let mut channels = vec![channel];
        match db
            .get_private_descendant_ids(tenant.community(), channel)
            .await
        {
            Ok(descendants) => channels.extend(descendants),
            Err(e) => warn!(%channel, error = %e, "channel access push: descendant read failed"),
        }
        for channel in channels {
            let tag = Tag::parse(["h".to_string(), channel.to_string()]).expect("h tag");
            let event =
                match EventBuilder::new(Kind::Custom(KIND_CHANNEL_ACCESS_CHANGED as u16), "")
                    .tags([tag])
                    .sign_with_keys(&keys)
                {
                    Ok(event) => event,
                    Err(e) => {
                        warn!(%channel, error = %e, "channel access push: signing failed");
                        continue;
                    }
                };
            if let Err(e) = pubsub
                .publish_event(&tenant, buzz_pubsub::EventTopic::Channel(channel), &event)
                .await
            {
                warn!(%channel, error = %e, "channel access push failed");
            }
        }
    });
}
