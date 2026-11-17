//! What one viewer may do in named channels (NOSTR.md, Channel access), and
//! the push that tells live subscribers to ask again.
//!
//! The answer applies the rules the read and post gates apply
//! ([`super::ingest::check_channel_membership`], [`super::ingest::may_post`],
//! [`super::ingest::closed_by_archive`]). It is a snapshot: every post is
//! still checked when it arrives.

use buzz_core::channel::MemberRole;
use buzz_core::kind::{
    KIND_CHANNEL_ACCESS_CHANGED, KIND_STREAM_MESSAGE, KIND_VIEWER_ACCESS_CHANGED,
};
use buzz_core::{CommunityId, TenantContext};
use buzz_db::channel::{ChannelJoinRole, ChannelRecord};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use nostr::{EventBuilder, Kind, Tag};
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

use crate::state::AppState;

/// The most channels one request may name, and the most one page lists.
pub const MAX_IDS: usize = 100;

/// One readable channel's answer for the viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelAccess {
    /// The channel.
    pub id: Uuid,
    /// Always true: unreadable channels are left out.
    pub read: bool,
    /// Whether a kind:9 from the viewer would be accepted now.
    pub post: bool,
    /// Whether the viewer has an active member row.
    pub joined: bool,
    /// The role the viewer acts with: their row's, else an open channel's
    /// `join_role`, else what the parents give.
    pub role: &'static str,
    /// Whether the channel itself is archived.
    pub archived: bool,
    /// While the viewer is timed out in the community, when that ends (unix
    /// seconds). `post` is false until then; ask again at that time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out_until: Option<i64>,
}

/// One page of the channels a viewer can reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReachablePage {
    /// The readable channels of this page, ordered by id.
    pub channels: Vec<ChannelAccess>,
    /// The `after` for the next page, or `None` when there are no more.
    pub next: Option<Uuid>,
}

/// Whether the viewer's community restriction lets a kind:9 through now.
#[derive(Debug, Clone, Copy)]
struct Restriction {
    may_post: bool,
    timed_out_until: Option<i64>,
}

/// The write-path restriction verdict ingest applies to a kind:9
/// ([`super::ingest::write_restriction_denial`]), read fresh so an answer
/// given right after a timeout push is not a stale cached row.
async fn restriction(
    state: &AppState,
    community_id: CommunityId,
    pubkey: &[u8],
) -> Result<Restriction, buzz_db::DbError> {
    let restriction = state
        .db
        .moderation_restriction_state(community_id, pubkey)
        .await?;
    let now = chrono::Utc::now();
    let refused =
        super::ingest::write_restriction_denial(KIND_STREAM_MESSAGE, &restriction, now).is_some();
    let timed_out_until = restriction
        .muted_until
        .filter(|until| refused && !restriction.banned && *until > now)
        .map(|until| until.timestamp());
    Ok(Restriction {
        may_post: !refused,
        timed_out_until,
    })
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
    let restriction = restriction(state, community_id, pubkey).await?;
    // The channels are independent reads: overlap a few, in request order,
    // without monopolising the pool.
    let answers: Vec<Option<ChannelAccess>> = stream::iter(ids.iter().copied())
        .map(|id| access(state, community_id, pubkey, id, restriction))
        .buffered(super::req::FILTER_QUERY_CONCURRENCY)
        .try_collect()
        .await?;
    Ok(answers.into_iter().flatten().collect())
}

/// One page of the viewer's answers for the channels they can reach, ordered
/// by id, starting after `after`: channels they joined, listed open
/// channels, and the children and grandchildren of both. These are the
/// channels a read that names none covers, so artifact conversations and
/// unlisted channels stay out. A page looks at up to `limit` candidates and
/// leaves out the unreadable ones, so it can hold fewer than `limit` and
/// still have a `next`.
pub(crate) async fn reachable(
    state: &AppState,
    community_id: CommunityId,
    pubkey: &[u8],
    after: Option<Uuid>,
    limit: usize,
) -> Result<ReachablePage, buzz_db::DbError> {
    let mut ids = state
        .db
        .get_accessible_channel_ids(community_id, pubkey)
        .await?;
    ids.retain(|id| after.is_none_or(|after| *id > after));
    ids.sort_unstable();
    let next = (ids.len() > limit).then(|| ids[limit - 1]);
    ids.truncate(limit);
    let channels = answer(state, community_id, pubkey, &ids).await?;
    Ok(ReachablePage { channels, next })
}

/// Whether one named person can read the place asked about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reader {
    /// The person, as lowercase hex.
    pub pubkey: String,
    /// Whether they can read the place now.
    pub read: bool,
}

/// Which of `pubkeys` can read `channel_id`, in request order, by the check
/// a task's assignees meet ([`can_read`]). `None` when `asker` cannot read
/// it themselves, whether it is private to them, deleted or missing, so the
/// answer never covers a place the asker can't see. Any lookup failure
/// fails the whole batch.
pub(crate) async fn readers(
    state: &AppState,
    community_id: CommunityId,
    asker: &[u8],
    channel_id: Uuid,
    pubkeys: &[[u8; 32]],
) -> Result<Option<Vec<Reader>>, buzz_db::DbError> {
    // Read every row fresh: a push can arrive before another node has
    // dropped its cached rows for these people.
    state.invalidate_membership_local(community_id, Uuid::nil(), asker);
    if !can_read(state, community_id, asker, channel_id).await? {
        return Ok(None);
    }
    for pubkey in pubkeys {
        state.invalidate_membership_local(community_id, Uuid::nil(), pubkey);
    }
    let read: Vec<bool> = stream::iter(pubkeys.iter().copied())
        .map(|pubkey| async move { can_read(state, community_id, &pubkey, channel_id).await })
        .buffered(super::req::FILTER_QUERY_CONCURRENCY)
        .try_collect()
        .await?;
    Ok(Some(
        pubkeys
            .iter()
            .zip(read)
            .map(|(pubkey, read)| Reader {
                pubkey: hex::encode(pubkey),
                read,
            })
            .collect(),
    ))
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

/// Whether `pubkey` can read `channel_id`, by the rule the answer leaves
/// unreadable channels out with.
pub(crate) async fn can_read(
    state: &AppState,
    community_id: CommunityId,
    pubkey: &[u8],
    channel_id: Uuid,
) -> Result<bool, buzz_db::DbError> {
    Ok(readable(state, community_id, pubkey, channel_id)
        .await?
        .is_some())
}

/// `pubkey`'s access to `channel_id` under their community `restriction`;
/// `None` when they cannot read it, whether it is private, deleted or missing.
async fn access(
    state: &AppState,
    community_id: CommunityId,
    pubkey: &[u8],
    channel_id: Uuid,
    restriction: Restriction,
) -> Result<Option<ChannelAccess>, buzz_db::DbError> {
    let Some((channel, role, joined)) = readable(state, community_id, pubkey, channel_id).await?
    else {
        return Ok(None);
    };
    let post = role.can_post()
        && restriction.may_post
        && !super::ingest::closed_by_archive(community_id, state, &channel).await?;
    Ok(Some(ChannelAccess {
        id: channel_id,
        read: true,
        post,
        joined,
        role: role_name(role),
        archived: channel.archived_at.is_some(),
        timed_out_until: restriction.timed_out_until,
    }))
}

/// A live channel `pubkey` can read, with the role they act with and
/// whether it is their own row's; `None` when it is private to them,
/// deleted or missing.
async fn readable(
    state: &AppState,
    community_id: CommunityId,
    pubkey: &[u8],
    channel_id: Uuid,
) -> Result<Option<(ChannelRecord, MemberRole, bool)>, buzz_db::DbError> {
    let channel = match state
        .db
        .get_channel_for_event_write(community_id, channel_id)
        .await
    {
        Ok(channel) if channel.deleted_at.is_none() => channel,
        Ok(_) | Err(buzz_db::DbError::ChannelNotFound(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(acting_role(state, community_id, &channel, pubkey)
        .await?
        .map(|(role, joined)| (channel, role, joined)))
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

/// Tell `pubkey` and every agent it owns, and no one else, to ask again
/// what they may do in every channel: an agent is restricted by its owner's
/// row. Sent only by [`AppState::restrictions_changed`], the step every
/// restriction change passes through. Off the caller's path, best effort
/// like [`announce_changed`]. The kind is p-gated, so only the named pubkey
/// may subscribe to it or receive it.
pub(crate) fn announce_restriction_changed(
    tenant: &TenantContext,
    state: &AppState,
    pubkey: &[u8],
) {
    let (tenant, db, pubsub, keys, pubkey) = (
        tenant.clone(),
        state.db.clone(),
        state.pubsub.clone(),
        state.relay_keypair.clone(),
        pubkey.to_vec(),
    );
    tokio::spawn(async move {
        let mut viewers = vec![pubkey.clone()];
        match db.list_agents_for_owner(tenant.community(), &pubkey).await {
            Ok(agents) => viewers.extend(agents),
            Err(e) => warn!(error = %e, "viewer access push: agent read failed"),
        }
        for viewer in viewers {
            let tag = Tag::parse(["p".to_string(), hex::encode(&viewer)]).expect("p tag");
            let event = match EventBuilder::new(Kind::Custom(KIND_VIEWER_ACCESS_CHANGED as u16), "")
                .tags([tag])
                .sign_with_keys(&keys)
            {
                Ok(event) => event,
                Err(e) => {
                    warn!(error = %e, "viewer access push: signing failed");
                    continue;
                }
            };
            if let Err(e) = pubsub
                .publish_event(&tenant, buzz_pubsub::EventTopic::Global, &event)
                .await
            {
                warn!(error = %e, "viewer access push failed");
            }
        }
    });
}
