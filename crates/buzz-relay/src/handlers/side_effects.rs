//! NIP-29 and NIP-25 side-effect handlers.

use std::sync::Arc;

use nostr::{Event, EventBuilder, Kind, Tag};
use tracing::{info, warn};
use uuid::Uuid;

use buzz_core::kind::{
    event_kind_u32, is_parameterized_replaceable, KIND_AGENT_PROFILE, KIND_DM_VISIBILITY,
    KIND_GIT_REPO_ANNOUNCEMENT, KIND_IA_ARCHIVED, KIND_IA_ARCHIVED_LIST, KIND_IA_UNARCHIVED,
    KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION, KIND_NIP29_GROUP_ADMINS,
    KIND_NIP29_GROUP_MEMBERS, KIND_NIP29_GROUP_METADATA, KIND_NIP43_MEMBERSHIP_LIST, KIND_REACTION,
    KIND_THREAD_SUMMARY,
};
use buzz_core::StoredEvent;
use buzz_db::channel::MemberRole;

use super::event::dispatch_persistent_event;
use crate::protocol::RelayMessage;
use crate::state::AppState;
use buzz_core::tenant::TenantContext;
use buzz_pubsub::EventTopic;

/// Check if a kind is an admin kind (9000-9022) that needs pre-storage validation.
pub fn is_admin_kind(kind: u32) -> bool {
    matches!(kind, 9000..=9022)
}

/// Check if a kind triggers side effects after storage.
///
/// NOTE: kind:7 (reaction) is intentionally excluded — dedup and DB writes are
/// handled in `ingest_event()` before storage so we can short-circuit on
/// duplicates without storing the event at all.
pub fn is_side_effect_kind(kind: u32) -> bool {
    matches!(kind, 0 | 5 | 9000..=9022 | KIND_GIT_REPO_ANNOUNCEMENT | KIND_AGENT_PROFILE | 41001..=41003 | 40099)
}

async fn evict_live_channel_subscriptions(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
    target_pubkey: &[u8],
) {
    let conn_ids = state.conn_manager.connection_ids_for_pubkey(target_pubkey);

    for conn_id in conn_ids {
        evict_conn_channel_subscriptions(tenant, state, channel_id, conn_id).await;
    }
}

/// Close every live channel-scoped subscription on `conn_id`, removing them from
/// the connection's local map and sending `CLOSED restricted` for each.
async fn evict_conn_channel_subscriptions(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
    conn_id: uuid::Uuid,
) {
    let removed = state.sub_registry.remove_channel_subscriptions_scoped(
        tenant.community(),
        conn_id,
        channel_id,
    );
    if removed.is_empty() {
        return;
    }

    if let Some(subscriptions) = state.conn_manager.subscriptions_for(conn_id) {
        let mut conn_subscriptions = subscriptions.lock().await;
        for (sub_id, _) in &removed {
            conn_subscriptions.remove(sub_id);
        }
    }

    for (sub_id, removed_scope) in removed {
        state
            .pubsub
            .release_topic(tenant, topic_for_subscription(removed_scope.channel_id))
            .await;
        let _ = state.conn_manager.send_to(
            conn_id,
            RelayMessage::closed(&sub_id, "restricted: channel access revoked"),
        );
    }
}

/// Revoke live channel subscriptions held by connections whose authenticated
/// pubkey is not a current member. Used when an open channel flips to private:
/// non-members could have subscribed while it was open. Fan-out now re-checks
/// membership per event as the delivery-time safety net; this eviction closes
/// subscriptions promptly so clients stop treating the channel as live.
async fn evict_non_member_channel_subscriptions(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
) -> anyhow::Result<()> {
    let members = state.db.get_members(tenant.community(), channel_id).await?;
    let member_pubkeys: std::collections::HashSet<Vec<u8>> =
        members.into_iter().map(|m| m.pubkey).collect();

    for conn_id in state
        .sub_registry
        .channel_subscriber_conns_scoped(tenant.community(), channel_id)
    {
        let is_member = match state.conn_manager.pubkey_for_conn(conn_id) {
            Some(pubkey) => member_pubkeys.contains(&pubkey),
            None => false,
        };
        if !is_member {
            evict_conn_channel_subscriptions(tenant, state, channel_id, conn_id).await;
        }
    }
    Ok(())
}

/// Close every live subscription on a channel, for all subscribers, sending
/// `CLOSED restricted: channel access revoked` to each.
///
/// Used when a channel is archived (e.g. the ephemeral-channel reaper): the
/// channel becomes unusable for everyone, so all live subscriptions must close.
/// The `channel access revoked` reason is in the client's drop-set, so a
/// connected agent drops just that channel and keeps its socket — no reconnect
/// storm. Offline/reconnecting clients are covered by the discovery-time
/// `archived=true` skip in `discover_channels`.
pub async fn evict_all_channel_subscriptions(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
) {
    for conn_id in state
        .sub_registry
        .channel_subscriber_conns_scoped(tenant.community(), channel_id)
    {
        evict_conn_channel_subscriptions(tenant, state, channel_id, conn_id).await;
    }
}

/// Dispatch side effects for a stored event.
pub async fn handle_side_effects(
    tenant: &TenantContext,
    kind: u32,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    match kind {
        0 => handle_kind0_profile(tenant, event, state).await,
        5 => handle_standard_deletion_event(tenant, event, state).await,
        9000 => handle_put_user(tenant, event, state).await,
        9001 => handle_remove_user(tenant, event, state).await,
        9002 => handle_edit_metadata(tenant, event, state).await,
        9005 => handle_delete_event_side_effect(tenant, event, state).await,
        9007 => handle_create_group(tenant, event, state).await,
        9008 => handle_delete_group(tenant, event, state).await,
        9009 => {
            warn!(
                kind = kind,
                "NIP-29 kind 9009 handler deferred to future phase"
            );
            Ok(())
        }
        9021 => handle_join_request(tenant, event, state).await,
        9022 => handle_leave_request(tenant, event, state).await,
        // NIP-34: Git repo announcement → reserve name + seed manifest pointer.
        KIND_GIT_REPO_ANNOUNCEMENT => handle_git_repo_announcement(tenant, event, state).await,
        KIND_AGENT_PROFILE => handle_agent_profile(tenant, event, state).await,
        // kind:7 (reaction) handled inline in ingest_event() before storage.
        _ => Ok(()),
    }
}

/// Validate a standard NIP-09 deletion event before it is stored.
///
/// Buzz accepts standard deletions for self-authored events, plus the owning
/// human deleting their agent's events (mirrors `validate_edit_ownership`).
/// Channel admin deletions continue to use kind 9005.
pub async fn validate_standard_deletion_event(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let actor_bytes = effective_message_author(event, &state.relay_keypair.public_key());
    let target_ids = extract_target_event_ids(event);

    if !has_e_tag(event) {
        // a-tag deletion: verify author owns the addressable event
        let a_tag = event
            .tags
            .iter()
            .find(|t| t.kind().to_string() == "a")
            .and_then(|t| t.content().map(|s| s.to_string()))
            .ok_or_else(|| anyhow::anyhow!("missing e or a tag for target"))?;
        let parts: Vec<&str> = a_tag.splitn(3, ':').collect();
        if parts.len() < 2 {
            return Err(anyhow::anyhow!("invalid a-tag format"));
        }
        let target_pubkey_bytes =
            hex::decode(parts[1]).map_err(|_| anyhow::anyhow!("invalid pubkey in a-tag"))?;
        if target_pubkey_bytes != actor_bytes
            && !state
                .db
                .is_agent_owner(tenant.community(), &target_pubkey_bytes, &actor_bytes)
                .await?
        {
            return Err(anyhow::anyhow!("must be event author"));
        }
        return Ok(());
    }

    for target_id in target_ids {
        let target_event = state
            .db
            .get_event_by_id_including_deleted(tenant.community(), &target_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("target event not found"))?;

        let target_author =
            effective_message_author(&target_event.event, &state.relay_keypair.public_key());
        if target_author != actor_bytes
            && !state
                .db
                .is_agent_owner(tenant.community(), &target_author, &actor_bytes)
                .await?
        {
            return Err(anyhow::anyhow!("must be event author"));
        }
    }

    Ok(())
}

/// Returns `true` if `actor_bytes` is the NIP-OA owner of **any** active owner-role
/// member in `members`. Used by kind:9002 and kind:9008 to authorize the owning
/// human of a channel's agent-owner(s) even when the human is not a channel member.
///
/// Checking all active owners (not just the first) is correct under co-ownership:
/// an agent may be promoted to owner later, or multiple agents may co-own a channel.
async fn actor_owns_any_owner_agent(
    state: &Arc<AppState>,
    community_id: buzz_core::CommunityId,
    members: &[buzz_db::channel::MemberRecord],
    actor_bytes: &[u8],
) -> anyhow::Result<bool> {
    for owner in members.iter().filter(|m| m.role == "owner") {
        if state
            .db
            .is_agent_owner(community_id, &owner.pubkey, actor_bytes)
            .await?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Validate an admin kind event BEFORE storage.
pub async fn validate_admin_event(
    tenant: &TenantContext,
    kind: u32,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    // CREATE_GROUP doesn't need an existing channel — skip h-tag extraction
    if kind == 9007 {
        return Ok(());
    }

    // Extract channel from h tag
    let channel_id =
        extract_h_tag_channel(event).ok_or_else(|| anyhow::anyhow!("missing or invalid h tag"))?;

    let actor_bytes = event.pubkey.to_bytes().to_vec();

    // Reject mutations on archived channels — except kind:9002 with archived=false
    // (unarchive), which must be allowed through so the channel can be restored.
    let channel = state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .map_err(|_| anyhow::anyhow!("channel not found"))?;
    let is_unarchive_request = kind == 9002
        && event.tags.iter().any(|t| {
            let parts = t.as_slice();
            parts.len() >= 2 && parts[0] == "archived" && parts[1] == "false"
        });
    if channel.archived_at.is_some() && !is_unarchive_request {
        return Err(anyhow::anyhow!("channel is archived"));
    }

    match kind {
        9000 => {
            // Validate role tag if present
            let role_str = extract_tag_value(event, "role").unwrap_or_else(|| "member".to_string());
            if role_str.parse::<buzz_db::channel::MemberRole>().is_err() {
                return Err(anyhow::anyhow!("invalid role: {role_str}"));
            }

            // PUT_USER: open channels allow any authenticated user; private channels
            // require the actor to be an existing member (any role can invite).
            if channel.visibility == "private" {
                let members = state.db.get_members(tenant.community(), channel_id).await?;
                let actor_member = members.iter().find(|m| m.pubkey == actor_bytes);
                match actor_member {
                    Some(_) => {}
                    None => return Err(anyhow::anyhow!("actor not authorized")),
                }

                // Only owners/admins may grant elevated roles.
                let role: buzz_db::channel::MemberRole = role_str.parse().unwrap();
                if role.is_elevated() {
                    let actor_role: buzz_db::channel::MemberRole = actor_member
                        .unwrap()
                        .role
                        .parse()
                        .unwrap_or(buzz_db::channel::MemberRole::Member);
                    if !actor_role.is_elevated() {
                        return Err(anyhow::anyhow!(
                            "only owners/admins may grant elevated roles"
                        ));
                    }
                }
            }

            // Extract target pubkey from p tag
            let target_pubkey =
                extract_p_tag(event).ok_or_else(|| anyhow::anyhow!("missing p tag"))?;

            // Self-add: always allowed regardless of policy.
            if target_pubkey == actor_bytes {
                return Ok(());
            }

            // Third-party add: check channel_add_policy on the target.
            if let Some((policy, owner)) = state
                .db
                .get_agent_channel_policy(tenant.community(), &target_pubkey)
                .await?
            {
                match policy.as_str() {
                    "owner_only" => {
                        let owner_bytes = owner.ok_or_else(|| {
                            anyhow::anyhow!("policy:owner_only — agent has no owner set")
                        })?;
                        if actor_bytes != owner_bytes {
                            return Err(anyhow::anyhow!(
                                "policy:owner_only — only the agent owner can add this agent"
                            ));
                        }
                    }
                    "nobody" => {
                        return Err(anyhow::anyhow!(
                            "policy:nobody — this agent has disabled external channel additions"
                        ));
                    }
                    // "anyone" or any unknown value → allow.
                    // NOTE: DB ENUM constraint prevents unknown values from being stored.
                    // If a new policy value is added to the ENUM, update this match.
                    _ => {}
                }
            }

            Ok(())
        }
        9001 => {
            // REMOVE_USER: self-remove allowed unless actor is the last owner; removing others requires owner/admin
            let target_pubkey =
                extract_p_tag(event).ok_or_else(|| anyhow::anyhow!("missing p tag"))?;
            if target_pubkey == actor_bytes {
                // Self-removal: must be an active member, and cannot be the last owner.
                let members = state.db.get_members(tenant.community(), channel_id).await?;
                let actor_member = members.iter().find(|m| m.pubkey == actor_bytes);
                match actor_member {
                    None => {
                        return Err(anyhow::anyhow!("actor is not an active member"));
                    }
                    Some(m) if m.role == "owner" => {
                        let owner_count = members.iter().filter(|m| m.role == "owner").count();
                        if owner_count <= 1 {
                            return Err(anyhow::anyhow!("cannot remove the last owner"));
                        }
                    }
                    _ => {}
                }
                Ok(())
            } else {
                let members = state.db.get_members(tenant.community(), channel_id).await?;
                let actor_member = members.iter().find(|m| m.pubkey == actor_bytes);
                match actor_member {
                    Some(m) if m.role == "owner" || m.role == "admin" => Ok(()),
                    Some(_) => {
                        if state
                            .db
                            .is_agent_owner(tenant.community(), &target_pubkey, &actor_bytes)
                            .await?
                        {
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!("actor not authorized"))
                        }
                    }
                    // Non-members fall here. We intentionally do NOT check
                    // is_agent_owner for non-members — you must be in the channel
                    // to remove anyone, even your own bot.
                    _ => Err(anyhow::anyhow!("actor not authorized")),
                }
            }
        }
        9002 => {
            // EDIT_METADATA: require at least one recognized metadata tag.
            const RECOGNIZED_TAGS: &[&str] = &[
                "name",
                "about",
                "archived",
                "topic",
                "purpose",
                "visibility",
                "ttl",
            ];
            let has_recognized = event
                .tags
                .iter()
                .any(|t| RECOGNIZED_TAGS.contains(&t.kind().to_string().as_str()));
            if !has_recognized {
                return Err(anyhow::anyhow!(
                    "kind:9002 must include at least one metadata tag (name, about, archived, topic, purpose, visibility, ttl)"
                ));
            }

            // Validate archived values before storage.
            for t in event.tags.iter() {
                if t.kind().to_string() == "archived" {
                    match t.content() {
                        Some("true") | Some("false") => {}
                        Some(v) => {
                            return Err(anyhow::anyhow!(
                                "invalid archived value: {v} (must be \"true\" or \"false\")"
                            ));
                        }
                        None => {
                            return Err(anyhow::anyhow!("archived tag must have a value"));
                        }
                    }
                }
            }

            // Validate visibility values before storage.
            for t in event.tags.iter() {
                if t.kind().to_string() == "visibility" {
                    match t.content() {
                        Some("open") | Some("private") => {}
                        Some(v) => {
                            return Err(anyhow::anyhow!(
                                "invalid visibility value: {v} (must be \"open\" or \"private\")"
                            ));
                        }
                        None => {
                            return Err(anyhow::anyhow!("visibility tag must have a value"));
                        }
                    }
                }
            }

            // Validate ttl values before storage. Empty string clears the TTL
            // (channel becomes permanent); any other value must parse as a
            // positive integer number of seconds. A bare tag with no value is
            // rejected so clearing is always explicit (`["ttl", ""]`).
            for t in event.tags.iter() {
                if t.kind().to_string() == "ttl" {
                    match t.content() {
                        Some("") => {}
                        Some(v) => match v.parse::<i32>() {
                            Ok(n) if n > 0 => {}
                            _ => {
                                return Err(anyhow::anyhow!(
                                    "invalid ttl value: {v} (must be a positive integer of seconds, or empty to clear)"
                                ));
                            }
                        },
                        None => {
                            return Err(anyhow::anyhow!(
                                "ttl tag must have a value (seconds, or empty string to clear)"
                            ));
                        }
                    }
                }
            }

            // name/about/archived/visibility/ttl require owner/admin;
            // topic/purpose allow any member.
            let has_privileged_tag = event.tags.iter().any(|t| {
                let k = t.kind().to_string();
                k == "name" || k == "about" || k == "archived" || k == "visibility" || k == "ttl"
            });
            if has_privileged_tag {
                let members = state.db.get_members(tenant.community(), channel_id).await?;
                let actor_member = members.iter().find(|m| m.pubkey == actor_bytes);
                match actor_member {
                    Some(m) if m.role == "owner" || m.role == "admin" => Ok(()),
                    _ => {
                        // Allow the owning human of any active owner-role agent in the
                        // channel, even when the human is not a channel member —
                        // diverges from kind:9001 intentionally.
                        if actor_owns_any_owner_agent(
                            state,
                            tenant.community(),
                            &members,
                            &actor_bytes,
                        )
                        .await?
                        {
                            return Ok(());
                        }
                        Err(anyhow::anyhow!(
                            "actor not authorized for name/about/archived/visibility/ttl changes"
                        ))
                    }
                }
            } else {
                // topic/purpose: any member
                let is_member = state
                    .is_member_cached(tenant.community(), channel_id, &actor_bytes)
                    .await?;
                if is_member {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("not a member"))
                }
            }
        }
        9005 => {
            // DELETE_EVENT: event author OR channel owner/admin.
            // Extract target event from e tag to check authorship.
            let target_id = event
                .tags
                .iter()
                .find_map(|tag| {
                    if tag.kind().to_string() == "e" {
                        tag.content().and_then(|v| hex::decode(v).ok())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| anyhow::anyhow!("missing e tag for target event"))?;

            // Verify the target event exists and belongs to the h-tag channel
            // BEFORE storage. Fail closed: missing target → reject.
            let target_event = state
                .db
                .get_event_by_id(tenant.community(), &target_id)
                .await
                .map_err(|e| anyhow::anyhow!("db error looking up target: {e}"))?
                .ok_or_else(|| anyhow::anyhow!("target event not found"))?;

            match target_event.channel_id {
                Some(target_ch) if target_ch != channel_id => {
                    return Err(anyhow::anyhow!(
                        "target event belongs to a different channel"
                    ));
                }
                None => {
                    return Err(anyhow::anyhow!("target event has no channel"));
                }
                _ => {} // Same channel — OK
            }

            // Check if actor is the event author.
            // For relay-signed REST messages, the real author is in the p tag.
            let author =
                effective_message_author(&target_event.event, &state.relay_keypair.public_key());
            if author == actor_bytes {
                // Author deleting their own message: re-gate on membership/open visibility so that
                // a removed private-channel member cannot mutate old messages after access is revoked.
                let is_member = state
                    .is_member_cached(tenant.community(), channel_id, &actor_bytes)
                    .await?;
                if is_member {
                    return Ok(());
                }
                let is_open = state
                    .db
                    .get_channel(tenant.community(), channel_id)
                    .await
                    .map(|ch| ch.visibility == "open")
                    .unwrap_or(false);
                if is_open {
                    return Ok(());
                }
                // Not a member and channel is private — fall through to owner/admin/owner-of-agent check.
            }

            // Not the author, or author who is no longer a member of a private channel —
            // must be owner/admin or the owning human of the message's agent-author.
            let members = state.db.get_members(tenant.community(), channel_id).await?;
            let actor_member = members.iter().find(|m| m.pubkey == actor_bytes);
            match actor_member {
                Some(m) if m.role == "owner" || m.role == "admin" => Ok(()),
                _ => {
                    // Allow the owning human of the agent that authored the target message,
                    // even when the human is not a channel member.
                    if state
                        .db
                        .is_agent_owner(tenant.community(), &author, &actor_bytes)
                        .await?
                    {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!(
                            "must be event author or channel owner/admin"
                        ))
                    }
                }
            }
        }
        9008 => {
            // DELETE_GROUP: owner only, or the owning human of the channel's agent-owner.
            let members = state.db.get_members(tenant.community(), channel_id).await?;
            let actor_member = members.iter().find(|m| m.pubkey == actor_bytes);
            match actor_member {
                Some(m) if m.role == "owner" => Ok(()),
                _ => {
                    // Allow the owning human of any active owner-role agent in the
                    // channel, even when the human is not a channel member —
                    // diverges from kind:9001 intentionally.
                    if actor_owns_any_owner_agent(state, tenant.community(), &members, &actor_bytes)
                        .await?
                    {
                        return Ok(());
                    }
                    Err(anyhow::anyhow!("only owner can delete group"))
                }
            }
        }
        9022 => {
            // LEAVE_REQUEST: must be an active member, and cannot be the last owner.
            let members = state.db.get_members(tenant.community(), channel_id).await?;
            let actor_member = members.iter().find(|m| m.pubkey == actor_bytes);
            match actor_member {
                None => {
                    return Err(anyhow::anyhow!("actor is not an active member"));
                }
                Some(m) if m.role == "owner" => {
                    let owner_count = members.iter().filter(|m| m.role == "owner").count();
                    if owner_count <= 1 {
                        return Err(anyhow::anyhow!("cannot remove the last owner"));
                    }
                }
                _ => {}
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Emit a system message (kind 40099) signed by the relay keypair.
pub async fn emit_system_message(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
    content: serde_json::Value,
) -> anyhow::Result<()> {
    let channel_tag = Tag::parse(["h", &channel_id.to_string()])?;

    let event = EventBuilder::new(Kind::Custom(40099), content.to_string())
        .tags([channel_tag])
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign system message: {e}"))?;

    if let Err(e) = state
        .db
        .insert_event(tenant.community(), &event, Some(channel_id))
        .await
    {
        warn!(channel = %channel_id, error = %e, "system message insert failed");
    }

    // Fan out to subscribers
    if let Err(e) = state
        .pubsub
        .publish_event(tenant, EventTopic::Channel(channel_id), &event)
        .await
    {
        warn!("System message fan-out failed: {e}");
    }

    Ok(())
}

/// Sign and fan out a fresh relay-signed `kind:39005` thread-summary overlay
/// for `root_id` after a thread mutation (reply insert or threaded delete).
///
/// Fan-out only — never stored. Channel-window pages recompute summaries from
/// `thread_metadata` on every fetch (`api/bridge.rs`), so a persisted copy
/// would only add staleness; this live emit exists purely so subscribed
/// clients can update badge counts without refetching the head window. The
/// counts are re-read from `thread_metadata` post-commit rather than
/// incremented, so the emitted summary is exact even under concurrent
/// replies/deletes (newest `created_at` wins client-side).
///
/// Spawned: runs after the triggering write committed and must not add
/// latency to the ingest acknowledgement, mirroring
/// `dispatch_persistent_event`.
pub fn emit_live_thread_summary(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
    root_id: Vec<u8>,
) {
    let tenant = tenant.clone();
    let state = Arc::clone(state);
    tokio::spawn(async move {
        let summary = match state
            .db
            .get_thread_summary(tenant.community(), &root_id)
            .await
        {
            Ok(Some(summary)) => summary,
            // Root has no thread row — nothing to summarize (e.g., the root
            // itself was just deleted).
            Ok(None) => return,
            Err(e) => {
                warn!(
                    root = %hex::encode(&root_id),
                    "live thread summary lookup failed: {e}"
                );
                return;
            }
        };

        let root_hex = hex::encode(&root_id);
        // Same tags/content shape as the channel-window page overlay in
        // `api/bridge.rs` — one contract, two delivery doors.
        let content = serde_json::json!({
            "reply_count": summary.reply_count,
            "descendant_count": summary.descendant_count,
            "last_reply_at": summary.last_reply_at.map(|t| t.timestamp()),
            "participants": summary.participants.iter().map(hex::encode).collect::<Vec<_>>(),
        });
        let tags = [
            Tag::parse(["e", &root_hex]),
            Tag::parse(["d", &root_hex]),
            Tag::parse(["h", &channel_id.to_string()]),
        ];
        let mut parsed = Vec::with_capacity(tags.len());
        for tag in tags {
            match tag {
                Ok(tag) => parsed.push(tag),
                Err(e) => {
                    warn!(root = %root_hex, "live thread summary tag failed: {e}");
                    return;
                }
            }
        }
        let event = match EventBuilder::new(
            Kind::Custom(KIND_THREAD_SUMMARY as u16),
            content.to_string(),
        )
        .tags(parsed)
        .sign_with_keys(&state.relay_keypair)
        {
            Ok(event) => event,
            Err(e) => {
                warn!(root = %root_hex, "live thread summary sign failed: {e}");
                return;
            }
        };

        // Redis before local fan-out so subscribers on other relay pods
        // receive it too, matching `dispatch_persistent_event`.
        state.mark_local_event(tenant.community(), &event.id);
        if let Err(e) = state
            .pubsub
            .publish_event(&tenant, EventTopic::Channel(channel_id), &event)
            .await
        {
            state
                .local_event_ids
                .invalidate(&(tenant.community(), event.id.to_bytes()));
            warn!(root = %root_hex, "live thread summary Redis publish failed: {e}");
        }
        let stored = StoredEvent::new(event, Some(channel_id));
        crate::handlers::event::fan_out_event_to_local_subscribers(
            &state,
            tenant.community(),
            &stored,
        )
        .await;
    });
}

/// Emit a relay-signed membership notification event stored globally (channel_id = None).
///
/// kind:44100 = member added, kind:44101 = member removed.
/// The p tag addresses the target pubkey; the h tag carries the channel UUID as metadata.
/// Stored with channel_id = None so global subscribers receive it via slow-path fan-out.
pub async fn emit_membership_notification(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
    target_pubkey: &[u8],
    actor_pubkey: &[u8],
    notification_kind: u32,
) -> anyhow::Result<()> {
    let target_hex = hex::encode(target_pubkey);
    let actor_hex = hex::encode(actor_pubkey);
    let channel_id_str = channel_id.to_string();

    let p_tag = Tag::parse(["p", &target_hex])
        .map_err(|e| anyhow::anyhow!("failed to build p tag: {e}"))?;
    let h_tag = Tag::parse(["h", &channel_id_str])
        .map_err(|e| anyhow::anyhow!("failed to build h tag: {e}"))?;

    let event_type = match notification_kind {
        KIND_MEMBER_ADDED_NOTIFICATION => "member_added",
        KIND_MEMBER_REMOVED_NOTIFICATION => "member_removed",
        _ => {
            return Err(anyhow::anyhow!(
                "invalid notification kind: {notification_kind}"
            ))
        }
    };

    let content = serde_json::json!({
        "type": event_type,
        "channel_id": channel_id_str,
        "actor": actor_hex,
    })
    .to_string();

    let event = EventBuilder::new(Kind::Custom(notification_kind as u16), content)
        .tags([p_tag, h_tag])
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign membership notification: {e}"))?;

    // Store with channel_id = None → globally scoped, reachable by global subscribers.
    let (stored, was_inserted) = state
        .db
        .insert_event(tenant.community(), &event, None)
        .await?;
    if !was_inserted {
        return Ok(());
    }

    // Fan-out only — skip search indexing and workflow evaluation. Publish through
    // Redis before local fan-out so agents connected to other relay pods receive
    // the global membership notification and can subscribe to the new channel.
    // Use the nil UUID sentinel for globally-scoped events, matching
    // `dispatch_persistent_event` and `fan_out_pubsub_event`.
    state.mark_local_event(tenant.community(), &stored.event.id);
    if let Err(e) = state
        .pubsub
        .publish_event(tenant, EventTopic::Global, &stored.event)
        .await
    {
        state
            .local_event_ids
            .invalidate(&(tenant.community(), stored.event.id.to_bytes()));
        warn!(
            channel = %channel_id,
            target = %target_hex,
            kind = notification_kind,
            "membership notification Redis publish failed: {e}"
        );
    }

    // Routed through the guarded send path for uniformity; the access gate no-ops
    // for these globally-scoped (channel_id = None) events.
    crate::handlers::event::fan_out_event_to_local_subscribers(state, tenant.community(), &stored)
        .await;

    info!(
        channel = %channel_id,
        target = %target_hex,
        kind = notification_kind,
        "membership notification emitted"
    );
    Ok(())
}

/// Sign, store (replacing previous), and fan-out a single addressable discovery event.
async fn emit_addressable_discovery_event(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
    kind: u32,
    tags: Vec<Tag>,
    relay_pubkey_hex: &str,
) -> anyhow::Result<()> {
    // Ensure the new event's created_at is strictly greater than any existing event
    // of the same (kind, pubkey, channel_id). Without this, rapid successive updates
    // (e.g. set topic then set purpose in the same second) can produce events with
    // identical created_at, causing the second to be rejected by stale-write protection
    // (NIP-16 tiebreaker: lower event ID wins, which is random).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let min_ts = {
        let existing = state
            .db
            .query_events(&buzz_db::event::EventQuery {
                kinds: Some(vec![kind as i32]),
                channel_id: Some(channel_id),
                limit: Some(1),
                ..buzz_db::event::EventQuery::for_community(tenant.community())
            })
            .await
            .unwrap_or_default();
        existing
            .first()
            .map(|e| e.event.created_at.as_secs() + 1)
            .unwrap_or(now)
    };
    let ts = now.max(min_ts);

    let event = EventBuilder::new(Kind::Custom(kind as u16), "")
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(ts))
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:{kind}: {e}"))?;

    let (stored, was_inserted) = state
        .db
        .replace_addressable_event(tenant.community(), &event, Some(channel_id))
        .await?;
    if was_inserted {
        let kind_u32 = event_kind_u32(&stored.event);
        dispatch_persistent_event(tenant, state, &stored, kind_u32, relay_pubkey_hex, None).await;
    }
    Ok(())
}

/// Emit NIP-29 group discovery events (39000, 39001, 39002) signed by the relay keypair.
/// Called after group creation, metadata changes, or membership changes.
/// Events are stored channel-scoped (`channel_id = Some(...)`) so that existing
/// access control applies — private channel member lists are only visible to members.
///
/// NOTE: Channel-scoped storage means live global subscriptions (e.g. `{kinds:[39000]}`)
/// won't receive these events via fan-out. Clients discover groups via historical REQ
/// queries. Live push for open-channel discovery is a future enhancement.
pub async fn emit_group_discovery_events(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    channel_id: Uuid,
) -> anyhow::Result<()> {
    let channel = state.db.get_channel(tenant.community(), channel_id).await?;
    let members = state.db.get_members(tenant.community(), channel_id).await?;

    let relay_pubkey_hex = hex::encode(state.relay_keypair.public_key().to_bytes());
    let group_id = channel_id.to_string();

    {
        let mut tags: Vec<Tag> = vec![Tag::parse(["d", &group_id])?];
        tags.push(Tag::parse(["name", &channel.name])?);
        if let Some(ref desc) = channel.description {
            if !desc.is_empty() {
                tags.push(Tag::parse(["about", desc])?);
            }
        }
        if channel.visibility == "private" {
            tags.push(Tag::parse(["private"])?);
        } else {
            // Explicit "public" tag complements NIP-29's absence-of-"private" convention,
            // making channel visibility self-describing for clients.
            tags.push(Tag::parse(["public"])?);
        }
        // NIP-29 hidden tag: hint to clients not to show DMs/chats in public group lists.
        // Not a security boundary — access control is handled by channel-scoped storage.
        if channel.channel_type == "dm" || channel.channel_type == "chat" {
            tags.push(Tag::parse(["hidden"])?);
        }

        if channel.channel_type == "dm" {
            // Include participant pubkeys in kind:39000 for DMs so clients can
            // resolve display names without a separate kind:39002 fetch.
            for m in &members {
                let pubkey_hex = hex::encode(&m.pubkey);
                tags.push(Tag::parse(["p", &pubkey_hex])?);
            }
        }
        // Buzz channels always require explicit membership
        tags.push(Tag::parse(["closed"])?);
        // Channel type tag so clients can distinguish stream/forum/dm without inference
        tags.push(Tag::parse(["t", &channel.channel_type])?);
        // Optional topic / purpose for richer client UX
        if let Some(ref topic) = channel.topic {
            if !topic.is_empty() {
                tags.push(Tag::parse(["topic", topic])?);
            }
        }
        if let Some(ref purpose) = channel.purpose {
            if !purpose.is_empty() {
                tags.push(Tag::parse(["purpose", purpose])?);
            }
        }
        // Archived state — clients use this to hide channels from the sidebar.
        if channel.archived_at.is_some() {
            tags.push(Tag::parse(["archived", "true"])?);
        }
        // Ephemeral channel TTL — clients use this to show countdown timers.
        if let Some(ttl) = channel.ttl_seconds {
            tags.push(Tag::parse(["ttl", &ttl.to_string()])?);
        }
        if let Some(ref deadline) = channel.ttl_deadline {
            tags.push(Tag::parse(["ttl_deadline", &deadline.to_rfc3339()])?);
        }
        emit_addressable_discovery_event(
            tenant,
            state,
            channel_id,
            KIND_NIP29_GROUP_METADATA,
            tags,
            &relay_pubkey_hex,
        )
        .await?;
    }

    {
        let mut tags: Vec<Tag> = vec![Tag::parse(["d", &group_id])?];
        for m in members
            .iter()
            .filter(|m| m.role == "owner" || m.role == "admin")
        {
            let pubkey_hex = hex::encode(&m.pubkey);
            tags.push(Tag::parse(["p", &pubkey_hex, &m.role])?);
        }
        emit_addressable_discovery_event(
            tenant,
            state,
            channel_id,
            KIND_NIP29_GROUP_ADMINS,
            tags,
            &relay_pubkey_hex,
        )
        .await?;
    }

    {
        let mut tags: Vec<Tag> = vec![Tag::parse(["d", &group_id])?];
        for m in &members {
            let pubkey_hex = hex::encode(&m.pubkey);
            // NIP-29 convention: ["p", pubkey, relay_url, role]. Empty relay_url
            // because the canonical relay is implicit (this event is signed by it).
            tags.push(Tag::parse(["p", &pubkey_hex, "", &m.role])?);
        }
        emit_addressable_discovery_event(
            tenant,
            state,
            channel_id,
            KIND_NIP29_GROUP_MEMBERS,
            tags,
            &relay_pubkey_hex,
        )
        .await?;
    }

    Ok(())
}

async fn handle_agent_profile(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let content: serde_json::Value = serde_json::from_str(&event.content)
        .map_err(|e| anyhow::anyhow!("kind:10100 content parse error: {e}"))?;

    let policy = content
        .get("channel_add_policy")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("kind:10100 missing channel_add_policy field"))?;

    let pubkey_bytes = event.pubkey.to_bytes().to_vec();
    state
        .db
        .ensure_user(tenant.community(), &pubkey_bytes)
        .await?;
    state
        .db
        .set_channel_add_policy(tenant.community(), &pubkey_bytes, policy)
        .await?;

    info!(pubkey = %hex::encode(&pubkey_bytes), policy, "kind:10100 channel_add_policy updated");
    Ok(())
}

/// Kind:0 (NIP-01 profile metadata) side effect — sync profile fields to users table.
async fn handle_kind0_profile(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let content: serde_json::Value = serde_json::from_str(&event.content)
        .map_err(|e| anyhow::anyhow!("kind:0 content parse error: {e}"))?;

    // Kind:0 is absolute state (NIP-01 replaceable event). Fields present in the
    // event are set; fields absent are cleared. We use Some("") to clear absent
    // fields, since update_user_profile only writes Some values.
    let display_name = content
        .get("display_name")
        .or_else(|| content.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let avatar_url = content
        .get("picture")
        .or_else(|| content.get("image"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let about = content.get("about").and_then(|v| v.as_str()).unwrap_or("");

    // Validate NIP-05 handle: must be user@domain where domain matches the
    // bound tenant host. Invalid or off-domain handles are silently cleared
    // (treated as absent) rather than stored, since the event is already
    // persisted and can't be rejected.
    let nip05_owned = content
        .get("nip05")
        .and_then(|v| v.as_str())
        .and_then(|raw| crate::api::nip05::canonicalize_nip05(raw, tenant.host()).ok());
    let nip05_handle = nip05_owned.as_deref().unwrap_or("");

    let pubkey_bytes = event.pubkey.to_bytes().to_vec();

    state
        .db
        .ensure_user(tenant.community(), &pubkey_bytes)
        .await?;

    // Pass all fields as Some — empty string clears the field in the DB.
    // This ensures kind:0 is treated as absolute state, not a partial update.
    // If the NIP-05 handle collides with another user's UNIQUE constraint, retry
    // without it so display_name/about/avatar_url are still written.
    let result = state
        .db
        .update_user_profile(
            tenant.community(),
            &pubkey_bytes,
            Some(display_name),
            Some(avatar_url),
            Some(about),
            Some(nip05_handle),
        )
        .await;

    if let Err(ref e) = result {
        let msg = format!("{e}");
        if msg.contains("duplicate key value") || msg.contains("23505") {
            warn!(pubkey = %hex::encode(&pubkey_bytes),
                "kind:0 NIP-05 handle contested, syncing profile without it");
            state
                .db
                .update_user_profile(
                    tenant.community(),
                    &pubkey_bytes,
                    Some(display_name),
                    Some(avatar_url),
                    Some(about),
                    None, // skip contested NIP-05
                )
                .await?;
        } else {
            result?;
        }
    }

    info!(pubkey = %hex::encode(&pubkey_bytes), "kind:0 profile synced to users table");
    Ok(())
}

async fn handle_put_user(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let channel_id =
        extract_h_tag_channel(event).ok_or_else(|| anyhow::anyhow!("missing h tag"))?;
    let target_pubkey = extract_p_tag(event).ok_or_else(|| anyhow::anyhow!("missing p tag"))?;
    let role_str = extract_tag_value(event, "role").unwrap_or_else(|| "member".to_string());
    let role: MemberRole = role_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid role: {role_str}"))?;

    let actor_bytes = event.pubkey.to_bytes().to_vec();

    state
        .db
        .add_member(
            tenant.community(),
            channel_id,
            &target_pubkey,
            role,
            Some(&actor_bytes),
        )
        .await?;
    state.invalidate_membership(tenant, channel_id, &target_pubkey);

    let actor_hex = hex::encode(&actor_bytes);
    let target_hex = hex::encode(&target_pubkey);
    emit_system_message(
        tenant,
        state,
        channel_id,
        serde_json::json!({
            "type": "member_joined",
            "actor": actor_hex,
            "target": target_hex,
        }),
    )
    .await?;

    if let Err(e) = emit_group_discovery_events(tenant, state, channel_id).await {
        warn!(channel = %channel_id, error = %e, "NIP-29 group discovery emission failed");
    }

    if let Err(e) = emit_membership_notification(
        tenant,
        state,
        channel_id,
        &target_pubkey,
        &actor_bytes,
        KIND_MEMBER_ADDED_NOTIFICATION,
    )
    .await
    {
        warn!(channel = %channel_id, error = %e, "membership notification emission failed");
    }

    info!(channel = %channel_id, target = %target_hex, "NIP-29 PUT_USER processed");
    Ok(())
}

async fn handle_remove_user(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let channel_id =
        extract_h_tag_channel(event).ok_or_else(|| anyhow::anyhow!("missing h tag"))?;
    let target_pubkey = extract_p_tag(event).ok_or_else(|| anyhow::anyhow!("missing p tag"))?;
    let actor_bytes = event.pubkey.to_bytes().to_vec();

    // Guard: prevent last-owner orphaning on self-removal (kind 9001).
    if target_pubkey == actor_bytes {
        let members = state.db.get_members(tenant.community(), channel_id).await?;
        let owner_count = members.iter().filter(|m| m.role == "owner").count();
        let actor_is_owner = members
            .iter()
            .any(|m| m.pubkey == actor_bytes && m.role == "owner");
        if actor_is_owner && owner_count <= 1 {
            return Err(anyhow::anyhow!(
                "cannot remove the last owner — transfer ownership first"
            ));
        }
    }

    state
        .db
        .remove_member(tenant.community(), channel_id, &target_pubkey, &actor_bytes)
        .await?;
    state.invalidate_membership(tenant, channel_id, &target_pubkey);
    evict_live_channel_subscriptions(tenant, state, channel_id, &target_pubkey).await;

    let actor_hex = hex::encode(&actor_bytes);
    let target_hex = hex::encode(&target_pubkey);
    let msg_type = if target_pubkey == actor_bytes {
        "member_left"
    } else {
        "member_removed"
    };
    emit_system_message(
        tenant,
        state,
        channel_id,
        serde_json::json!({
            "type": msg_type,
            "actor": actor_hex,
            "target": target_hex,
        }),
    )
    .await?;

    if let Err(e) = emit_group_discovery_events(tenant, state, channel_id).await {
        warn!(channel = %channel_id, error = %e, "NIP-29 group discovery emission failed");
    }

    if let Err(e) = emit_membership_notification(
        tenant,
        state,
        channel_id,
        &target_pubkey,
        &actor_bytes,
        KIND_MEMBER_REMOVED_NOTIFICATION,
    )
    .await
    {
        warn!(channel = %channel_id, error = %e, "membership notification emission failed");
    }

    Ok(())
}

async fn handle_edit_metadata(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let channel_id =
        extract_h_tag_channel(event).ok_or_else(|| anyhow::anyhow!("missing h tag"))?;
    let actor_bytes = event.pubkey.to_bytes().to_vec();
    let actor_hex = hex::encode(&actor_bytes);

    for tag in event.tags.iter() {
        let key = tag.kind().to_string();
        if let Some(val) = tag.content() {
            match key.as_str() {
                "name" => {
                    state
                        .db
                        .update_channel(
                            tenant.community(),
                            channel_id,
                            buzz_db::channel::ChannelUpdate {
                                name: Some(val.to_string()),
                                ..Default::default()
                            },
                        )
                        .await?;
                }
                "about" => {
                    state
                        .db
                        .update_channel(
                            tenant.community(),
                            channel_id,
                            buzz_db::channel::ChannelUpdate {
                                description: Some(val.to_string()),
                                ..Default::default()
                            },
                        )
                        .await?;
                }
                "topic" => {
                    state
                        .db
                        .set_topic(tenant.community(), channel_id, val, &actor_bytes)
                        .await?;
                    emit_system_message(
                        tenant,
                        state,
                        channel_id,
                        serde_json::json!({
                            "type": "topic_changed", "actor": actor_hex, "topic": val
                        }),
                    )
                    .await?;
                }
                "purpose" => {
                    state
                        .db
                        .set_purpose(tenant.community(), channel_id, val, &actor_bytes)
                        .await?;
                    emit_system_message(
                        tenant,
                        state,
                        channel_id,
                        serde_json::json!({
                            "type": "purpose_changed", "actor": actor_hex, "purpose": val
                        }),
                    )
                    .await?;
                }
                "visibility" => {
                    let was_open = state
                        .db
                        .get_channel(tenant.community(), channel_id)
                        .await
                        .map(|c| c.visibility == "open")
                        .unwrap_or(false);
                    state
                        .db
                        .update_channel(
                            tenant.community(),
                            channel_id,
                            buzz_db::channel::ChannelUpdate {
                                visibility: Some(val.to_string()),
                                ..Default::default()
                            },
                        )
                        .await?;
                    // A visibility flip changes who can see the channel, so the
                    // accessible-channels and visibility caches must be cleared
                    // before any later event for this channel fans out.
                    state.invalidate_all_accessible_channels(tenant);
                    state.invalidate_channel_visibility(tenant, channel_id);
                    // On open -> private, eagerly close non-members' live subs
                    // for an immediate CLOSED on this node. The fan-out access
                    // filter is the cluster-wide correctness backstop.
                    if was_open && val == "private" {
                        evict_non_member_channel_subscriptions(tenant, state, channel_id).await?;
                    }
                    emit_system_message(
                        tenant,
                        state,
                        channel_id,
                        serde_json::json!({
                            "type": "visibility_changed", "actor": actor_hex, "visibility": val
                        }),
                    )
                    .await?;
                }
                "ttl" => {
                    // Empty string clears the TTL (permanent); otherwise it is a
                    // positive integer of seconds, validated during authorization.
                    // Fail closed: a parse failure must reject, never silently
                    // clear the TTL to permanent.
                    let ttl_change: Option<i32> = if val.is_empty() {
                        None
                    } else {
                        Some(val.parse::<i32>().map_err(|_| {
                            anyhow::anyhow!("invalid ttl value: {val} (must be a positive integer)")
                        })?)
                    };
                    state
                        .db
                        .update_channel(
                            tenant.community(),
                            channel_id,
                            buzz_db::channel::ChannelUpdate {
                                ttl_seconds: Some(ttl_change),
                                ..Default::default()
                            },
                        )
                        .await?;
                    emit_system_message(
                        tenant,
                        state,
                        channel_id,
                        serde_json::json!({
                            "type": "ttl_changed", "actor": actor_hex, "ttl_seconds": ttl_change
                        }),
                    )
                    .await?;
                }
                "archived" => {
                    match val {
                        "true" => {
                            state
                                .db
                                .archive_channel(tenant.community(), channel_id)
                                .await?;
                            emit_system_message(
                                tenant,
                                state,
                                channel_id,
                                serde_json::json!({
                                    "type": "channel_archived", "actor": actor_hex
                                }),
                            )
                            .await?;
                        }
                        "false" => {
                            state
                                .db
                                .unarchive_channel(tenant.community(), channel_id)
                                .await?;
                            emit_system_message(
                                tenant,
                                state,
                                channel_id,
                                serde_json::json!({
                                    "type": "channel_unarchived", "actor": actor_hex
                                }),
                            )
                            .await?;

                            // Resubscribe connected agents after restore: archiving evicts their
                            // live subscriptions (CLOSED "channel access revoked") and unarchive
                            // otherwise emits no signal that makes a connected agent resubscribe.
                            // We reuse the member_added notification (44100) purely as a resubscribe
                            // trigger — no membership actually changed here — because it flows on the
                            // agent's always-live global membership subscription, the same path
                            // remove/re-add uses to recover. Humans self-heal via the re-emitted
                            // kind:39000 discovery, so this is intentionally agent-scoped.
                            //
                            // Known limitation: emit_membership_notification builds a created_at=now
                            // event with no nonce, and insert_event skips fan-out on a duplicate id.
                            // Four sub-second toggles (archive->unarchive->archive->unarchive) on the
                            // same channel by the same actor could collide ids and skip a fan-out.
                            // Not reachable in practice — unarchive has a single human-driven caller;
                            // the reaper only auto-archives — so we don't engineer around it.
                            for member in
                                state.db.get_members(tenant.community(), channel_id).await?
                            {
                                if let Err(e) = emit_membership_notification(
                                    tenant,
                                    state,
                                    channel_id,
                                    &member.pubkey,
                                    &actor_bytes,
                                    KIND_MEMBER_ADDED_NOTIFICATION,
                                )
                                .await
                                {
                                    warn!(
                                        channel = %channel_id,
                                        error = %e,
                                        "post-unarchive resubscribe notification failed"
                                    );
                                }
                            }
                        }
                        _ => {} // ignore invalid values
                    }
                }
                _ => {}
            }
        }
    }

    if let Err(e) = emit_group_discovery_events(tenant, state, channel_id).await {
        warn!(channel = %channel_id, error = %e, "NIP-29 group discovery emission failed");
    }

    Ok(())
}

async fn handle_delete_event_side_effect(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let channel_id =
        extract_h_tag_channel(event).ok_or_else(|| anyhow::anyhow!("missing h tag"))?;

    // Extract target event ID from e tag
    let target_id = event
        .tags
        .iter()
        .find_map(|tag| {
            if tag.kind().to_string() == "e" {
                tag.content().and_then(|v| {
                    let bytes = hex::decode(v).ok()?;
                    if bytes.len() == 32 {
                        Some(bytes)
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("missing e tag for target event"))?;

    // Verify the target event belongs to the same channel as the h-tag.
    // Without this check, an admin of channel A could delete events in channel B
    // by sending h=A, e=<event-in-B>.
    if let Some(target_event) = state
        .db
        .get_event_by_id_including_deleted(tenant.community(), &target_id)
        .await
        .map_err(|e| anyhow::anyhow!("get_event_by_id failed: {e}"))?
    {
        match target_event.channel_id {
            Some(target_ch) if target_ch != channel_id => {
                return Err(anyhow::anyhow!(
                    "target event belongs to a different channel"
                ));
            }
            None => {
                return Err(anyhow::anyhow!("target event has no channel"));
            }
            _ => {} // Same channel — OK
        }
    }

    // Look up thread metadata so we can pass parent/root IDs to the
    // transactional delete function.
    let meta = state
        .db
        .get_thread_metadata_by_event(tenant.community(), &target_id)
        .await
        .map_err(|e| anyhow::anyhow!("get_thread_metadata failed: {e}"))?;

    let parent_id = meta.as_ref().and_then(|m| m.parent_event_id.clone());
    let root_id = meta.as_ref().and_then(|m| m.root_event_id.clone());

    // Atomically soft-delete the event and decrement thread counters in one transaction.
    let deleted = state
        .db
        .soft_delete_event_and_update_thread(
            tenant.community(),
            &target_id,
            parent_id.as_deref(),
            root_id.as_deref(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("soft_delete_event failed: {e}"))?;

    if !deleted {
        warn!(target_event = %hex::encode(&target_id), "event already deleted or not found");
        return Ok(()); // No-op: skip system message to avoid false audit records.
    }

    // Thread counters were decremented in the same transaction — push a fresh
    // relay-signed 39005 so live badge counts also count *down*.
    if let Some(root_id) = root_id {
        emit_live_thread_summary(tenant, state, channel_id, root_id);
    }

    let actor_hex = hex::encode(event.pubkey.to_bytes());
    emit_system_message(
        tenant,
        state,
        channel_id,
        serde_json::json!({
            "type": "message_deleted",
            "actor": actor_hex,
            "target_event_id": hex::encode(&target_id),
        }),
    )
    .await?;

    info!(target_event = %hex::encode(&target_id), "NIP-29 DELETE_EVENT processed");
    Ok(())
}

async fn handle_create_group(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let name =
        extract_tag_value(event, "name").ok_or_else(|| anyhow::anyhow!("missing name tag"))?;
    let visibility_str =
        extract_tag_value(event, "visibility").unwrap_or_else(|| "open".to_string());
    let channel_type_str =
        extract_tag_value(event, "channel_type").unwrap_or_else(|| "stream".to_string());

    let visibility: buzz_db::channel::ChannelVisibility = visibility_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid visibility: {visibility_str}"))?;
    let channel_type: buzz_db::channel::ChannelType = channel_type_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid channel_type: {channel_type_str}"))?;

    let actor_bytes = event.pubkey.to_bytes().to_vec();
    let description = extract_tag_value(event, "about");
    let ttl_seconds = super::resolve_ttl(event, state.config.ephemeral_ttl_override);

    // If the event has an h-tag UUID, ingest_event() already created the channel
    // via create_channel_with_id(). Fetch it rather than creating a duplicate.
    // If no h-tag, fall back to the original auto-UUID creation path.
    let channel = if let Some(client_uuid) = extract_h_tag_channel(event) {
        match state.db.get_channel(tenant.community(), client_uuid).await {
            Ok(ch) => ch,
            Err(_) => {
                // Channel not found — shouldn't happen (ingest_event pre-created it),
                // but fall back to creation to stay resilient.
                state
                    .db
                    .create_channel(
                        tenant.community(),
                        &name,
                        channel_type,
                        visibility,
                        description.as_deref(),
                        &actor_bytes,
                        ttl_seconds,
                    )
                    .await?
            }
        }
    } else {
        state
            .db
            .create_channel(
                tenant.community(),
                &name,
                channel_type,
                visibility,
                description.as_deref(),
                &actor_bytes,
                ttl_seconds,
            )
            .await?
    };

    // Creator becomes owner — evict any stale negative membership lookup.
    state.invalidate_membership(tenant, channel.id, &actor_bytes);
    // Open channels appear in everyone's accessible set; private channels only
    // affect the creator (the sole initial member).
    if visibility == buzz_db::channel::ChannelVisibility::Open {
        state.invalidate_all_accessible_channels(tenant);
    }

    let actor_hex = hex::encode(&actor_bytes);
    emit_system_message(
        tenant,
        state,
        channel.id,
        serde_json::json!({
            "type": "channel_created", "actor": actor_hex
        }),
    )
    .await?;

    if let Err(e) = emit_group_discovery_events(tenant, state, channel.id).await {
        warn!(channel = %channel.id, error = %e, "NIP-29 group discovery emission failed");
    }

    if let Err(e) = emit_membership_notification(
        tenant,
        state,
        channel.id,
        &actor_bytes,
        &actor_bytes, // creator is both actor and target
        KIND_MEMBER_ADDED_NOTIFICATION,
    )
    .await
    {
        warn!(channel = %channel.id, error = %e, "membership notification emission failed");
    }

    info!(channel_id = %channel.id, name = %name, "NIP-29 CREATE_GROUP processed");
    Ok(())
}

async fn handle_delete_group(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let channel_id =
        extract_h_tag_channel(event).ok_or_else(|| anyhow::anyhow!("missing h tag"))?;
    let actor_bytes = event.pubkey.to_bytes().to_vec();

    // Soft-delete the channel.
    let deleted = state
        .db
        .soft_delete_channel(tenant.community(), channel_id)
        .await
        .map_err(|e| anyhow::anyhow!("soft_delete_channel failed: {e}"))?;

    if !deleted {
        warn!(channel = %channel_id, "channel already deleted or not found");
    }

    // Clean up NIP-29 discovery events for the deleted group.
    if let Err(e) = state
        .db
        .soft_delete_discovery_events(
            tenant.community(),
            channel_id,
            state.relay_keypair.public_key().as_bytes(),
        )
        .await
    {
        warn!(channel = %channel_id, error = %e, "failed to clean up NIP-29 discovery events");
    }

    // Deleted channel: clear both membership and accessible-channels caches.
    // Stale is_member=true entries would bypass the DB's deleted_at guard.
    state.invalidate_channel_deleted(tenant);

    let actor_hex = hex::encode(&actor_bytes);
    emit_system_message(
        tenant,
        state,
        channel_id,
        serde_json::json!({
            "type": "channel_deleted", "actor": actor_hex
        }),
    )
    .await?;

    info!(channel = %channel_id, "NIP-29 DELETE_GROUP processed");
    Ok(())
}

async fn handle_join_request(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let channel_id =
        extract_h_tag_channel(event).ok_or_else(|| anyhow::anyhow!("missing h tag"))?;
    let actor_bytes = event.pubkey.to_bytes().to_vec();

    // Only open channels allow self-join via kind:9021.
    let channel = state
        .db
        .get_channel(tenant.community(), channel_id)
        .await
        .map_err(|_| anyhow::anyhow!("channel not found"))?;
    if channel.visibility != "open" {
        return Err(anyhow::anyhow!(
            "channel is private — request an invitation"
        ));
    }

    // Skip if already an active member — prevents duplicate join notifications.
    // Fail closed on DB errors rather than falling through to add_member.
    if state
        .is_member_cached(tenant.community(), channel_id, &actor_bytes)
        .await?
    {
        info!(channel = %channel_id, "kind:9021 join — already a member, skipping");
        return Ok(());
    }

    // Add as member (idempotent — add_member handles duplicates).
    state
        .db
        .add_member(
            tenant.community(),
            channel_id,
            &actor_bytes,
            buzz_db::channel::MemberRole::Member,
            None,
        )
        .await?;
    state.invalidate_membership(tenant, channel_id, &actor_bytes);

    let actor_hex = hex::encode(&actor_bytes);
    emit_system_message(
        tenant,
        state,
        channel_id,
        serde_json::json!({
            "type": "member_joined",
            "actor": actor_hex,
            "target": actor_hex,
        }),
    )
    .await?;

    if let Err(e) = emit_group_discovery_events(tenant, state, channel_id).await {
        warn!(channel = %channel_id, error = %e, "NIP-29 group discovery emission failed");
    }

    if let Err(e) = emit_membership_notification(
        tenant,
        state,
        channel_id,
        &actor_bytes,
        &actor_bytes,
        buzz_core::kind::KIND_MEMBER_ADDED_NOTIFICATION,
    )
    .await
    {
        warn!("membership notification for join failed: {e}");
    }

    info!(channel = %channel_id, "kind:9021 join processed");
    Ok(())
}

async fn handle_leave_request(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    // Kind 9022: functionally identical to self-remove via kind 9001
    let channel_id =
        extract_h_tag_channel(event).ok_or_else(|| anyhow::anyhow!("missing h tag"))?;
    let actor_bytes = event.pubkey.to_bytes().to_vec();

    // Guard: prevent last-owner orphaning on leave.
    let members = state.db.get_members(tenant.community(), channel_id).await?;
    let owner_count = members.iter().filter(|m| m.role == "owner").count();
    let actor_is_owner = members
        .iter()
        .any(|m| m.pubkey == actor_bytes && m.role == "owner");
    if actor_is_owner && owner_count <= 1 {
        return Err(anyhow::anyhow!(
            "cannot remove the last owner — transfer ownership first"
        ));
    }

    state
        .db
        .remove_member(tenant.community(), channel_id, &actor_bytes, &actor_bytes)
        .await?;
    state.invalidate_membership(tenant, channel_id, &actor_bytes);
    evict_live_channel_subscriptions(tenant, state, channel_id, &actor_bytes).await;

    let actor_hex = hex::encode(&actor_bytes);
    emit_system_message(
        tenant,
        state,
        channel_id,
        serde_json::json!({
            "type": "member_left",
            "actor": actor_hex,
        }),
    )
    .await?;

    if let Err(e) = emit_group_discovery_events(tenant, state, channel_id).await {
        warn!(channel = %channel_id, error = %e, "NIP-29 group discovery emission failed");
    }

    if let Err(e) = emit_membership_notification(
        tenant,
        state,
        channel_id,
        &actor_bytes,
        &actor_bytes, // self-leave: actor == target
        KIND_MEMBER_REMOVED_NOTIFICATION,
    )
    .await
    {
        warn!(channel = %channel_id, error = %e, "membership notification emission failed");
    }

    Ok(())
}

// handle_reaction() removed — kind:7 reaction dedup and DB writes are now
// handled inline in ingest_event() before storage (see ingest.rs step 20a).

/// Handle NIP-09 deletion via `a` tag (addressable/parameterized-replaceable events).
/// Parses "kind:pubkey:d-tag" and deletes the corresponding DB record.
async fn handle_a_tag_deletion(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let a_value = event
        .tags
        .iter()
        .find(|t| t.kind().to_string() == "a")
        .and_then(|t| t.content().map(|s| s.to_string()))
        .ok_or_else(|| anyhow::anyhow!("missing a tag for addressable deletion"))?;

    let parts: Vec<&str> = a_value.splitn(3, ':').collect();
    if parts.len() < 3 {
        return Err(anyhow::anyhow!("invalid a-tag format: {a_value}"));
    }
    let kind_num: u32 = parts[0]
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid kind in a-tag"))?;
    let pubkey_hex = parts[1];
    let d_tag = parts[2];
    let actor_bytes = effective_message_author(event, &state.relay_keypair.public_key());

    match kind_num {
        buzz_core::kind::KIND_WORKFLOW_DEF => {
            // Try UUID first (workflow_id); fall back to name-based lookup.
            if let Ok(wf_id) = uuid::Uuid::parse_str(d_tag) {
                let channel_id = state
                    .db
                    .delete_workflow_for_owner(tenant.community(), wf_id, &actor_bytes)
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to delete workflow {wf_id}: {e}"))?;
                if let Some(channel_id) = channel_id {
                    state
                        .workflow_engine
                        .invalidate_channel_workflows(tenant.community(), channel_id);
                }
                tracing::info!(workflow_id = %wf_id, "Workflow deleted via NIP-09 a-tag (UUID)");
            } else {
                // Name-based lookup
                match state
                    .db
                    .find_workflow_by_owner_and_name(tenant.community(), &actor_bytes, d_tag)
                    .await
                {
                    Ok(Some(wf)) => {
                        let channel_id = state
                            .db
                            .delete_workflow_for_owner(tenant.community(), wf.id, &actor_bytes)
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("failed to delete workflow {}: {e}", wf.id)
                            })?;
                        if let Some(channel_id) = channel_id {
                            state
                                .workflow_engine
                                .invalidate_channel_workflows(tenant.community(), channel_id);
                        }
                        tracing::info!(workflow_id = %wf.id, name = d_tag, "Workflow deleted via NIP-09 a-tag (name)");
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "NIP-09 a-tag deletion: no workflow '{d_tag}' found for owner"
                        );
                    }
                    Err(e) => {
                        tracing::warn!("NIP-09 a-tag deletion: DB lookup failed: {e}");
                    }
                }
            }
        }
        // Generic NIP-33 (parameterized-replaceable) soft-delete by coordinate.
        //
        // Listed after the workflow branch so workflow's bespoke deletion
        // (which doesn't soft-delete the `events` row by design — that's a
        // separate concern) takes precedence. For every other addressable
        // kind, including kind:30023 (NIP-23 long-form), we soft-delete the
        // live row matching `(kind, pubkey, d_tag)` so REQs stop returning it.
        // See https://github.com/block/sprout/issues/714.
        k if is_parameterized_replaceable(k) => {
            let pubkey_bytes = match hex::decode(pubkey_hex) {
                Ok(b) => b,
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "invalid pubkey hex in a-tag {pubkey_hex}: {e}"
                    ));
                }
            };
            // Safe cast: NIP-33 kinds are 30000–39999, well within i32.
            let kind_i32 = k as i32;
            let deleted = state
                .db
                .soft_delete_by_coordinate(tenant.community(), kind_i32, &pubkey_bytes, d_tag)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "failed to soft-delete by coordinate {kind_i32}:{pubkey_hex}:{d_tag}: {e}"
                    )
                })?;
            if deleted {
                tracing::info!(
                    kind = k,
                    d_tag = d_tag,
                    "NIP-09 a-tag deletion: soft-deleted addressable event by coordinate"
                );
            } else {
                tracing::debug!(
                    kind = k,
                    d_tag = d_tag,
                    "NIP-09 a-tag deletion: no live row matched coordinate"
                );
            }
        }
        _ => {
            tracing::debug!(
                kind = kind_num,
                d_tag = d_tag,
                "NIP-09 a-tag deletion for non-NIP-33 kind — no side effect"
            );
        }
    }

    Ok(())
}

async fn handle_standard_deletion_event(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let target_ids = extract_target_event_ids(event);
    if !has_e_tag(event) {
        // NIP-09 a-tag deletion path for addressable events. Keyed on the
        // absence of *any* e tag (not just valid e-ids): a malformed e + a must
        // not route here and silently soft-delete the coordinate.
        return handle_a_tag_deletion(tenant, event, state).await;
    }

    for target_id in target_ids {
        let target_event = match state
            .db
            .get_event_by_id_including_deleted(tenant.community(), &target_id)
            .await?
        {
            Some(target) => target,
            None => continue,
        };

        let meta = state
            .db
            .get_thread_metadata_by_event(tenant.community(), &target_id)
            .await?;
        let parent_id = meta.as_ref().and_then(|m| m.parent_event_id.clone());
        let root_id = meta.as_ref().and_then(|m| m.root_event_id.clone());

        let deleted = state
            .db
            .soft_delete_event_and_update_thread(
                tenant.community(),
                &target_id,
                parent_id.as_deref(),
                root_id.as_deref(),
            )
            .await?;

        if !deleted {
            continue;
        }

        // Thread counters were decremented in the same transaction — push a
        // fresh relay-signed 39005 so live badge counts also count *down*.
        if let (Some(root_id), Some(channel_id)) = (root_id, target_event.channel_id) {
            emit_live_thread_summary(tenant, state, channel_id, root_id);
        }

        if u32::from(target_event.event.kind.as_u16()) == KIND_REACTION {
            // Try by reaction_event_id first; fall back to tuple-based removal
            // if the backfill was missed (set_reaction_event_id is best-effort).
            let removed = state
                .db
                .remove_reaction_by_source_event_id(tenant.community(), &target_id)
                .await
                .unwrap_or(false);
            if !removed {
                // Derive (target, actor, emoji) from the reaction event itself.
                // Use effective_message_author to handle legacy relay-signed
                // reactions where event.pubkey is the relay key, not the user.
                let actor = super::ingest::effective_message_author(
                    &target_event.event,
                    &state.relay_keypair.public_key(),
                );
                let emoji = if target_event.event.content.is_empty() {
                    "+"
                } else {
                    &target_event.event.content
                };
                if let Some(react_target_hex) = target_event.event.tags.iter().rev().find_map(|t| {
                    if t.kind().to_string() == "e" {
                        t.content().and_then(|v| {
                            if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                                Some(v.to_string())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    }
                }) {
                    if let Ok(react_target_id) = hex::decode(&react_target_hex) {
                        if let Ok(Some(react_target_event)) = state
                            .db
                            .get_event_by_id(tenant.community(), &react_target_id)
                            .await
                        {
                            let react_target_ts = chrono::DateTime::from_timestamp(
                                react_target_event.event.created_at.as_secs() as i64,
                                0,
                            )
                            .unwrap_or_else(chrono::Utc::now);
                            if let Err(e) = state
                                .db
                                .remove_reaction(
                                    tenant.community(),
                                    &react_target_id,
                                    react_target_ts,
                                    &actor,
                                    emoji,
                                )
                                .await
                            {
                                tracing::warn!(
                                    error = %e,
                                    "failed to remove reaction from DB during NIP-09 deletion"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Extract channel UUID from `h` tag (NIP-29 group ID).
fn extract_h_tag_channel(event: &Event) -> Option<Uuid> {
    for tag in event.tags.iter() {
        if tag.kind().to_string() == "h" {
            if let Some(val) = tag.content() {
                if let Ok(id) = val.parse::<Uuid>() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// Extract target pubkey from first `p` tag.
fn extract_p_tag(event: &Event) -> Option<Vec<u8>> {
    for tag in event.tags.iter() {
        if tag.kind().to_string() == "p" {
            if let Some(val) = tag.content() {
                if let Ok(bytes) = hex::decode(val) {
                    if bytes.len() == 32 {
                        return Some(bytes);
                    }
                }
            }
        }
    }
    None
}

/// Extract the effective message author from a stored event.
///
/// REST-created messages are signed by the relay keypair and attribute the real
/// sender via a `p` tag. For user-signed events (WebSocket), `event.pubkey` is
/// the author. Returns the correct author bytes in both cases.
fn effective_message_author(event: &Event, relay_pubkey: &nostr::PublicKey) -> Vec<u8> {
    if event.pubkey == *relay_pubkey {
        if let Some(actor_hex) = extract_tag_value(event, "actor") {
            if let Ok(bytes) = hex::decode(actor_hex) {
                if bytes.len() == 32 {
                    return bytes;
                }
            }
        }
        for tag in event.tags.iter() {
            if tag.kind().to_string() == "p" {
                if let Some(hex) = tag.content() {
                    if let Ok(bytes) = hex::decode(hex) {
                        if bytes.len() == 32 {
                            return bytes;
                        }
                    }
                }
            }
        }
    }
    event.pubkey.to_bytes().to_vec()
}

/// True if the event carries any `e` tag at all, regardless of whether its
/// value decodes to a valid 32-byte id. NIP-09 treats `e`/`a` as target
/// classes: a malformed `e` makes the deletion ambiguous, not addressable-only.
/// Routing keys on this rather than on decoded-target count so a malformed `e`
/// alongside an `a` never silently soft-deletes a coordinate.
fn has_e_tag(event: &Event) -> bool {
    event.tags.iter().any(|t| t.kind().to_string() == "e")
}

fn extract_target_event_ids(event: &Event) -> Vec<Vec<u8>> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            if tag.kind().to_string() != "e" {
                return None;
            }

            tag.content().and_then(|value| {
                if value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()) {
                    hex::decode(value).ok().filter(|bytes| bytes.len() == 32)
                } else {
                    None
                }
            })
        })
        .collect()
}

/// Extract value of a named tag.
fn extract_tag_value(event: &Event, tag_name: &str) -> Option<String> {
    for tag in event.tags.iter() {
        if tag.kind().to_string() == tag_name {
            return tag.content().map(|s| s.to_string());
        }
    }
    None
}

/// Validate a git repo identifier (d-tag value from kind:30617).
///
/// Rules: `[a-zA-Z0-9._-]{1,64}`, no leading dots, no `..`.
fn validate_repo_id(repo_id: &str) -> bool {
    !repo_id.is_empty()
        && repo_id.len() <= 64
        && !repo_id.starts_with('.')
        && !repo_id.contains("..")
        && repo_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Handle kind:30617 (NIP-34 Git Repository Announcement).
///
/// Reserves the repo name and seeds its empty-manifest pointer when a repo
/// announcement event is stored. The event's `d` tag is the repo identifier;
/// the pubkey is the owner. No bare repo is created on disk — runtime reads
/// and writes hydrate an ephemeral repo from object storage per request.
///
/// Security hardening:
/// - Repo name validated: `[a-zA-Z0-9._-]{1,64}`, no leading dots, no `..`
/// - Name reserved atomically in Postgres (`git_repo_names`), unique per community
/// - Per-pubkey repo count limit enforced
async fn handle_git_repo_announcement(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    // Extract repo identifier from d tag (required for NIP-33 parameterized replaceable events).
    let repo_id =
        extract_tag_value(event, "d").ok_or_else(|| anyhow::anyhow!("kind:30617 missing d tag"))?;

    if !validate_repo_id(&repo_id) {
        return Err(anyhow::anyhow!(
            "invalid repo identifier: must be [a-zA-Z0-9._-]{{1,64}}, no leading dots, no '..'"
        ));
    }

    let owner_hex = hex::encode(event.pubkey.to_bytes());

    // The relay holds no persistent per-repo disk state: runtime reads and
    // writes hydrate an ephemeral bare repo from object storage per request
    // (see `api::git::hydrate`). Announce only (1) reserves the repo name and
    // (2) seeds the empty-manifest pointer that makes the repo clone-able.
    //
    // The `git_repo_names` table (Postgres) is the relay's name registry,
    // keyed `(community_id, repo_id)`. It serves three jobs at once inside the
    // server-resolved community boundary:
    //   - uniqueness: `INSERT … ON CONFLICT DO NOTHING` is atomic, so
    //     concurrent kind:30617 events for the same community/name can't both
    //     claim it (TOCTOU-free — the DB PK is the race guard);
    //   - idempotent re-announce: a reservation owned by the same pubkey is an
    //     update, not a collision;
    //   - per-pubkey quota: `COUNT` reservations owned by this pubkey.
    //
    // This replaces the v1 local-disk `.names/` index. Moving it into Postgres
    // (which the relay already requires) removes the last persistent local-disk
    // state, so separate replicas no longer need a shared ReadWriteMany volume
    // to agree on name ownership. Actual ref-state safety remains the
    // object-store pointer CAS (`api::git::cas_publish`, `Inv_NoFork`); this
    // registry only governs name allocation.
    let community = tenant.community();
    use buzz_db::git_repo::ReserveOutcome;

    // Classify the name first: same-owner re-announce is idempotent; a name
    // held by anyone else is a collision (the relay signs kind:30618 with
    // d-tag = repo_name, so a shared name would let one owner overwrite
    // another's ref state). For a not-yet-owned name we must check quota
    // *before* claiming, so we peek the current holder rather than inserting
    // blindly.
    //
    // Crucially, we do NOT return early on a same-owner existing row: the row
    // proves name *ownership*, not that the manifest pointer was actually
    // seeded. A concurrent same-owner announce could hold the row while its
    // seed is still in flight (or failed and rolled back), so trusting the row
    // alone would let this handler "accept" an uncloneable repo. Instead we
    // fall through to `seed_manifest_pointer`, which is idempotent under
    // concurrency (create-only `put_pointer(IfNoneMatchStar)`; a `LostRace` on
    // the same empty digest is success, a different non-empty pointer is a
    // hard error). So re-announce *ensures* the pointer rather than assuming it.
    let outcome =
        if let Some(existing_owner) = state.db.repo_name_owner(community, &repo_id).await? {
            if existing_owner != owner_hex {
                return Err(anyhow::anyhow!(
                    "repo name '{repo_id}' already taken by another owner"
                ));
            }
            // Same owner: the reservation already exists (this attempt did not
            // create it), so it must never be rolled back by this attempt, and the
            // per-pubkey quota is unchanged (re-announce never grows the count).
            ReserveOutcome::AlreadyOwned
        } else {
            // Not yet owned by anyone we saw: enforce the per-pubkey quota, then
            // claim the name atomically. The `ON CONFLICT` guard resolves a
            // concurrent announce even though the peek above missed it —
            // `Reserved` means *this attempt* won the insert, `AlreadyOwned` means
            // a same-owner sibling won it, `TakenByOther` is a cross-owner
            // collision.
            let limit = state.config.git_max_repos_per_pubkey as i64;
            let owned = state
                .db
                .count_repos_for_owner(community, &owner_hex)
                .await?;
            if owned >= limit {
                return Err(anyhow::anyhow!("repo limit exceeded: {owned} >= {limit}"));
            }
            match state
                .db
                .reserve_repo_name(community, &repo_id, &owner_hex)
                .await?
            {
                outcome @ (ReserveOutcome::Reserved | ReserveOutcome::AlreadyOwned) => outcome,
                ReserveOutcome::TakenByOther => {
                    return Err(anyhow::anyhow!(
                        "repo name '{repo_id}' already taken by another owner"
                    ));
                }
            }
        };

    // Only a genuinely fresh claim by *this* attempt may be rolled back on a
    // pointer failure. An `AlreadyOwned` outcome means the row is owned by some
    // other attempt (a same-owner sibling, or a prior announce that has since
    // pushed), and deleting it here would strand a repo whose pointer that
    // other attempt already established.
    let reserved_by_this_attempt = matches!(outcome, ReserveOutcome::Reserved);

    // Establish/confirm the manifest pointer, keeping the invariant
    // "repo announced ⟺ pointer exists" so the read path can rely on
    // pointer-absent meaning never-announced (keeping `info_refs`'s fail-closed
    // `Ok(None) → 404` unambiguous). Two distinct cases:
    //
    // - Fresh `Reserved` claim → `seed_manifest_pointer` (strict). This creates
    //   the empty pointer, and correctly *fails* if a non-empty pointer already
    //   exists for a name we just reserved — that would be a suspicious stale
    //   pointer from a prior repo lifecycle, not a legitimate re-announce.
    // - Same-owner `AlreadyOwned` (re-announce) → `ensure_manifest_pointer`
    //   (tolerant). A non-empty pointer is the *normal* post-push state, so
    //   re-announce must accept it untouched; only an absent pointer is
    //   repaired by seeding. Using the strict seed here would wrongly reject
    //   every re-announce after the first push.
    let pointer_result = if reserved_by_this_attempt {
        seed_manifest_pointer(state, tenant, &owner_hex, &repo_id).await
    } else {
        ensure_manifest_pointer(state, tenant, &owner_hex, &repo_id).await
    };
    if let Err(pointer_err) = pointer_result {
        // A reserved name without a clone-able pointer is exactly the broken
        // state this step exists to prevent — but ONLY roll back the
        // reservation if this attempt is the one that freshly created it. A
        // genuine failure from a fresh `Reserved` attempt means the pointer
        // truly could not be established, so releasing our own just-inserted
        // row is safe and correct (all-or-nothing). For an `AlreadyOwned`
        // attempt we release nothing: the row belongs to another attempt that
        // may have seeded (or pushed) successfully.
        if reserved_by_this_attempt {
            if let Err(release_err) = state
                .db
                .release_repo_name(community, &repo_id, &owner_hex)
                .await
            {
                warn!(
                    repo_id = %repo_id,
                    error = %release_err,
                    "failed to release repo name reservation after seed failure"
                );
            }
        }
        return Err(anyhow::anyhow!(
            "failed to ensure manifest pointer: {pointer_err}"
        ));
    }

    info!(
        repo_id = %repo_id,
        owner = %owner_hex,
        reserved = reserved_by_this_attempt,
        "kind:30617 repo announced (name reserved, manifest pointer ensured)"
    );

    // Derived after the pointer commits: kind:30618 ref-state event over the
    // seeded empty manifest. Pointer is the commit; this event is the
    // notification that the repo exists (with empty refs) so subscribers see
    // a first signal without waiting for the first push.
    //
    // Emit ONLY on a fresh `Reserved` claim. On a same-owner `AlreadyOwned`
    // re-announce the pointer already exists (and, after the first push, holds
    // real refs). Re-emitting the empty-refs 30618 here would publish a *newer*
    // replaceable event that, under NIP-16 latest-wins ordering, shadows the
    // real pushed refs — making a live repo look empty to subscribers. The
    // initial empty signal is a one-time seeding notification, not something a
    // re-announce should replay.
    if reserved_by_this_attempt {
        if let Err(e) = emit_initial_ref_state(tenant, state, &owner_hex, &repo_id).await {
            // Non-fatal: the manifest is the source of truth; this is just the
            // derived notification. A failure here means subscribers miss the
            // "repo now exists" event, but clone/push still works.
            warn!(
                repo_id = %repo_id,
                owner = %owner_hex,
                error = %e,
                "failed to emit initial kind:30618 ref state (non-fatal)"
            );
        }
    }

    Ok(())
}

/// Default symbolic HEAD for a freshly-announced (empty) repo. Matches
/// `init.defaultBranch=main` (git ≥ 2.28) and the seed used by
/// `live_hydrate_empty_repo`. Pinned in one place so the seeded manifest
/// and the initial kind:30618 emission can't drift.
///
/// The first push's `cas_publish` overwrites this with the real symbolic
/// HEAD observed in the receive-pack workspace via standard CAS, so the
/// default is a stand-in, not a permanent commitment.
const DEFAULT_HEAD: &str = "refs/heads/main";

/// Seed the manifest-pointer for a newly-announced repo with an empty manifest.
///
/// Idempotent: a `CasOutcome::LostRace` is treated as success **only if** the
/// existing pointer names the same empty manifest digest. Any other pre-existing
/// pointer body (e.g. a non-empty manifest from a previous announce/push pair
/// for the same `(owner, repo)`) surfaces as an error rather than silently
/// succeeding — that would mask a real misconfiguration.
async fn seed_manifest_pointer(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    owner_hex: &str,
    repo_id: &str,
) -> anyhow::Result<()> {
    use crate::api::git::manifest::{pointer_key, Manifest, MANIFEST_VERSION};
    use crate::api::git::store::{CasOutcome, Precond};
    use std::collections::BTreeMap;

    // The empty manifest. All empty manifests across all repos share canonical
    // bytes — by design — so `put_manifest` is idempotent at the store level
    // too.
    let empty = Manifest {
        version: MANIFEST_VERSION,
        head: DEFAULT_HEAD.to_string(),
        refs: BTreeMap::new(),
        packs: Vec::new(),
        parent: None,
    };
    empty
        .validate()
        .map_err(|e| anyhow::anyhow!("empty manifest failed validation: {e}"))?;
    let bytes = empty
        .canonical_bytes()
        .map_err(|e| anyhow::anyhow!("empty manifest serialize: {e}"))?;
    let manifest_key = state
        .git_store
        .put_manifest(&bytes)
        .await
        .map_err(|e| anyhow::anyhow!("put_manifest: {e}"))?;
    let digest = manifest_key
        .strip_prefix("manifests/")
        .ok_or_else(|| anyhow::anyhow!("put_manifest returned non-standard key: {manifest_key}"))?;

    let pkey = pointer_key(tenant.community(), owner_hex, repo_id);
    let outcome = state
        .git_store
        .put_pointer(&pkey, digest.as_bytes(), Precond::IfNoneMatchStar)
        .await
        .map_err(|e| anyhow::anyhow!("put_pointer: {e}"))?;
    match outcome {
        CasOutcome::Won(_) => Ok(()),
        CasOutcome::LostRace => {
            // Pointer already exists. Idempotency check: only treat as success
            // if it names the same empty manifest digest. Any other value is
            // either a stale pointer from a prior repo lifecycle for the same
            // (owner, repo) or a real misconfiguration — surface, don't swallow.
            let (_etag, body) = state
                .git_store
                .get_pointer(&pkey)
                .await
                .map_err(|e| anyhow::anyhow!("re-read pointer after LostRace: {e}"))?
                .ok_or_else(|| anyhow::anyhow!("pointer vanished after LostRace race"))?;
            let existing = std::str::from_utf8(&body)
                .map_err(|e| anyhow::anyhow!("pointer body not utf-8: {e}"))?
                .trim();
            if existing != digest {
                return Err(anyhow::anyhow!(
                    "repo '{repo_id}' for owner {owner_hex} already has a non-empty pointer \
                     ({existing}); refusing to overwrite via announce"
                ));
            }
            Ok(())
        }
    }
}

/// Ensure a manifest pointer exists for an already-owned repo (same-owner
/// re-announce path). Unlike [`seed_manifest_pointer`], which is strict for
/// *creation* (it refuses when a non-empty pointer already exists, since a
/// freshly-reserved name with a populated pointer is suspicious), this is the
/// tolerant *idempotent* path for a name this owner already holds:
///
/// - **pointer present** (empty *or* non-empty) → success, left untouched. A
///   non-empty pointer is the normal state after the owner has pushed; a
///   re-announce must not fail just because the repo has commits, and must
///   never overwrite real ref state.
/// - **pointer absent** → seed the empty pointer (repair the "row exists but
///   pointer missing" window, e.g. a prior announce whose seed failed after
///   the row was inserted by a sibling attempt). This restores the
///   "announced ⟺ pointer exists" invariant.
///
/// The read-then-conditional-seed is race-safe: the repair uses
/// `seed_manifest_pointer`'s create-only `put_pointer(IfNoneMatchStar)`, so a
/// concurrent seeder that wins is resolved by that function's `LostRace`
/// handling (same empty digest → Ok), and a concurrent *pusher* that populates
/// the pointer between our read and our seed loses the create race and is
/// likewise treated as an already-present pointer, not an overwrite.
async fn ensure_manifest_pointer(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    owner_hex: &str,
    repo_id: &str,
) -> anyhow::Result<()> {
    use crate::api::git::manifest::pointer_key;

    let pkey = pointer_key(tenant.community(), owner_hex, repo_id);
    let existing = state
        .git_store
        .get_pointer(&pkey)
        .await
        .map_err(|e| anyhow::anyhow!("get_pointer: {e}"))?;
    match existing {
        // Any existing pointer (empty or non-empty) is valid for a same-owner
        // re-announce — leave it exactly as-is.
        Some(_) => Ok(()),
        // No pointer yet: repair by seeding the empty pointer. `LostRace` to a
        // concurrent seeder/pusher is handled by `seed_manifest_pointer`.
        None => seed_manifest_pointer(state, tenant, owner_hex, repo_id).await,
    }
}

/// Emit the initial kind:30618 ref-state event for a freshly-announced repo.
///
/// The seeded empty manifest is the source of truth; this event is the
/// derived notification. Fires once per announce, signed by the relay,
/// carrying the announcer's pubkey in the `p` tag (buzz extension).
async fn emit_initial_ref_state(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    owner_hex: &str,
    repo_id: &str,
) -> anyhow::Result<()> {
    use crate::api::git::manifest_event::{build_ref_state_event, RefStateInputs};
    use std::collections::BTreeMap;

    let empty_refs: BTreeMap<String, String> = BTreeMap::new();
    let inputs = RefStateInputs {
        repo_id,
        head: DEFAULT_HEAD,
        refs: &empty_refs,
        actor_pubkey_hex: owner_hex,
    };
    let event = build_ref_state_event(&inputs, &state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("build_ref_state_event: {e}"))?;
    let (stored, was_inserted) = state
        .db
        .insert_event(tenant.community(), &event, None)
        .await
        .map_err(|e| anyhow::anyhow!("insert kind:30618: {e}"))?;
    if was_inserted {
        // Routed through the guarded send path for uniformity; the access gate
        // no-ops for this globally-scoped (channel_id = None) ref-state event.
        crate::handlers::event::fan_out_event_to_local_subscribers(
            state,
            tenant.community(),
            &stored,
        )
        .await;
    }
    Ok(())
}

/// Publish a kind:13534 relay membership list event (NIP-43).
///
/// Queries all current relay members and emits a relay-signed, NIP-70-protected
/// addressable event listing every member pubkey. Replaces any previous list.
pub async fn publish_nip43_membership_list(
    tenant: &TenantContext,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let members = state.db.list_relay_members(tenant.community()).await?;
    let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();

    let mut tags: Vec<Tag> = Vec::with_capacity(members.len() + 1);

    // NIP-70 protected-event marker — prevents re-broadcasting by third parties.
    tags.push(Tag::parse(["-"]).map_err(|e| anyhow::anyhow!("failed to build '-' tag: {e}"))?);

    for member in &members {
        tags.push(
            Tag::parse(["member", &member.pubkey, &member.role])
                .map_err(|e| anyhow::anyhow!("failed to build member tag: {e}"))?,
        );
    }

    let event = EventBuilder::new(Kind::Custom(KIND_NIP43_MEMBERSHIP_LIST as u16), "")
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:13534: {e}"))?;

    // NOTE: kind 13534 is technically a regular event (not in the NIP-16 replaceable
    // range), but we intentionally use replace_addressable_event to get replacement
    // semantics — only the latest membership snapshot matters. This function keys on
    // (kind, pubkey, channel_id) and atomically replaces older events, which is exactly
    // what Pyramid (the reference NIP-43 implementation) does with store.ReplaceEvent().
    let (stored, was_inserted) = state
        .db
        .replace_addressable_event(tenant.community(), &event, None)
        .await?;
    if was_inserted {
        dispatch_persistent_event(
            tenant,
            state,
            &stored,
            KIND_NIP43_MEMBERSHIP_LIST,
            &relay_pubkey_hex,
            None,
        )
        .await;
    }

    info!(
        member_count = members.len(),
        "NIP-43 membership list published"
    );
    Ok(())
}

/// Shared helper: publish a NIP-43 membership delta event (kind 8000 or 8001).
///
/// Signs a relay event with `["-"]` (NIP-70) + `["p", target]` tags, stores it
/// globally, and fans out to matching subscribers.
async fn publish_nip43_delta(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    kind: u16,
    target_pubkey_hex: &str,
    label: &str,
) -> anyhow::Result<()> {
    let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();

    let tags = vec![
        Tag::parse(["-"]).map_err(|e| anyhow::anyhow!("failed to build '-' tag: {e}"))?,
        Tag::parse(["p", target_pubkey_hex])
            .map_err(|e| anyhow::anyhow!("failed to build p tag: {e}"))?,
    ];

    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:{kind}: {e}"))?;

    let (stored, was_inserted) = state
        .db
        .insert_event(tenant.community(), &event, None)
        .await?;
    if !was_inserted {
        return Ok(());
    }

    // Routed through the guarded send path for uniformity; the access gate
    // no-ops for this globally-scoped (channel_id = None) NIP-43 event.
    crate::handlers::event::fan_out_event_to_local_subscribers(state, tenant.community(), &stored)
        .await;

    info!(
        target = %target_pubkey_hex,
        relay = %relay_pubkey_hex,
        "NIP-43 {label} event published"
    );
    Ok(())
}

/// Publish a kind:8000 relay member-added announcement event (NIP-43).
pub async fn publish_nip43_member_added(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    target_pubkey_hex: &str,
) -> anyhow::Result<()> {
    publish_nip43_delta(tenant, state, 8000, target_pubkey_hex, "member-added").await
}

/// Publish a kind:8001 relay member-removed announcement event (NIP-43).
pub async fn publish_nip43_member_removed(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    target_pubkey_hex: &str,
) -> anyhow::Result<()> {
    publish_nip43_delta(tenant, state, 8001, target_pubkey_hex, "member-removed").await
}

/// Reconcile channels that exist in the DB but don't have kind:39000 events.
///
/// This handles the case where channels were created via direct SQL inserts
/// (e.g. test seed scripts) rather than through the Nostr event pipeline.
/// Emits kind:39000 (metadata) and kind:39002 (members) for each channel
/// that is missing its discovery events.
///
/// Idempotent: checks for existing kind:39000 events before emitting.
pub async fn reconcile_channel_events(
    tenant: &TenantContext,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    use buzz_db::event::EventQuery;

    let channels = state.db.list_channels(tenant.community(), None).await?;
    if channels.is_empty() {
        return Ok(());
    }

    let mut reconciled = 0u32;
    for channel in &channels {
        // Check if kind:39000 event already exists for this channel.
        let channel_id_str = channel.id.to_string();
        let existing = match state
            .db
            .query_events(&EventQuery {
                kinds: Some(vec![39000]),
                d_tag: Some(channel_id_str.clone()),
                limit: Some(1),
                ..EventQuery::for_community(tenant.community())
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    channel_id = %channel.id,
                    error = %e,
                    "reconcile: failed to query existing discovery events"
                );
                continue;
            }
        };

        if existing.is_empty() {
            // No discovery event — emit one.
            if let Err(e) = emit_group_discovery_events(tenant, state, channel.id).await {
                tracing::warn!(
                    channel_id = %channel.id,
                    error = %e,
                    "reconcile: failed to emit discovery events"
                );
            } else {
                reconciled += 1;
            }
        }
    }

    if reconciled > 0 {
        tracing::info!(count = reconciled, "reconciled channel discovery events");
    }
    Ok(())
}

/// Publish a kind:13535 archived identities list event (NIP-IA).
///
/// Queries all current archived identities and emits a relay-signed,
/// NIP-70-protected replaceable-by-convention snapshot with bare `p` tags.
pub async fn publish_nipia_archival_list(
    tenant: &TenantContext,
    state: &Arc<AppState>,
) -> anyhow::Result<()> {
    let archived = state.db.list_archived(tenant.community()).await?;
    let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();

    let mut tags: Vec<Tag> = Vec::with_capacity(archived.len() + 1);
    tags.push(Tag::parse(["-"]).map_err(|e| anyhow::anyhow!("failed to build '-' tag: {e}"))?);

    for identity in &archived {
        tags.push(
            Tag::parse(["p", &identity.pubkey])
                .map_err(|e| anyhow::anyhow!("failed to build p tag: {e}"))?,
        );
    }

    let event = EventBuilder::new(Kind::Custom(KIND_IA_ARCHIVED_LIST as u16), "")
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:{KIND_IA_ARCHIVED_LIST}: {e}"))?;

    let (stored, was_inserted) = state
        .db
        .replace_addressable_event(tenant.community(), &event, None)
        .await?;
    if was_inserted {
        dispatch_persistent_event(
            tenant,
            state,
            &stored,
            KIND_IA_ARCHIVED_LIST,
            &relay_pubkey_hex,
            None,
        )
        .await;
    }

    info!(
        archived_count = archived.len(),
        "NIP-IA archived identities list published"
    );
    Ok(())
}

/// NIP-DV: publish the relay-signed, per-viewer DM visibility snapshot for
/// `viewer`. The event is parameterized-replaceable (`d` = viewer pubkey) and
/// carries one `h` tag per DM the viewer currently has hidden. Called after any
/// hide (41012) or unhide (41010 that clears `hidden_at`); the latest event is
/// always the authoritative hidden set, so no client-side delta merge is needed.
pub async fn publish_dm_visibility_snapshot(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    viewer: &[u8],
) -> anyhow::Result<()> {
    let viewer_hex = hex::encode(viewer);
    let hidden = state.db.list_hidden_dms(tenant.community(), viewer).await?;
    let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();

    let mut tags: Vec<Tag> = Vec::with_capacity(hidden.len() + 2);
    tags.push(
        Tag::parse(["d", &viewer_hex])
            .map_err(|e| anyhow::anyhow!("failed to build d tag: {e}"))?,
    );
    // `p` = viewer so the relay's `#p`-gated read path scopes the snapshot to
    // its owner; no one else may query another viewer's hidden-DM set.
    tags.push(
        Tag::parse(["p", &viewer_hex])
            .map_err(|e| anyhow::anyhow!("failed to build p tag: {e}"))?,
    );
    for channel_id in &hidden {
        tags.push(
            Tag::parse(["h", &channel_id.to_string()])
                .map_err(|e| anyhow::anyhow!("failed to build h tag: {e}"))?,
        );
    }

    // Force created_at strictly past any prior snapshot for this viewer: a same-second
    // replacement whose random event id sorts higher is rejected by stale-write
    // protection, so a hide→re-open within one second could otherwise strand the stale
    // snapshot. Same guard as emit_addressable_discovery_event.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let ts = {
        let existing = state
            .db
            .query_events(&buzz_db::event::EventQuery {
                kinds: Some(vec![KIND_DM_VISIBILITY as i32]),
                pubkey: Some(state.relay_keypair.public_key().to_bytes().to_vec()),
                d_tag: Some(viewer_hex.clone()),
                limit: Some(1),
                ..buzz_db::event::EventQuery::for_community(tenant.community())
            })
            .await
            .unwrap_or_default();
        existing
            .first()
            .map(|e| (e.event.created_at.as_secs() + 1).max(now))
            .unwrap_or(now)
    };

    let event = EventBuilder::new(Kind::Custom(KIND_DM_VISIBILITY as u16), "")
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(ts))
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:{KIND_DM_VISIBILITY}: {e}"))?;

    let (stored, was_inserted) = state
        .db
        .replace_parameterized_event(tenant.community(), &event, &viewer_hex, None)
        .await?;
    if was_inserted {
        dispatch_persistent_event(
            tenant,
            state,
            &stored,
            KIND_DM_VISIBILITY,
            &relay_pubkey_hex,
            None,
        )
        .await;
    }

    info!(
        viewer = %viewer_hex,
        hidden_count = hidden.len(),
        "NIP-DV DM visibility snapshot published"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn publish_nipia_delta(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    kind: u32,
    target_pubkey_hex: &str,
    consent_path: &str,
    actor_pubkey_hex: &str,
    request_event_id: &str,
    content: &str,
    reason: Option<&str>,
    replaced_by: Option<&str>,
) -> anyhow::Result<()> {
    let relay_pubkey_hex = state.relay_keypair.public_key().to_hex();

    let mut tags = vec![
        Tag::parse(["-"]).map_err(|e| anyhow::anyhow!("failed to build '-' tag: {e}"))?,
        Tag::parse(["p", target_pubkey_hex])
            .map_err(|e| anyhow::anyhow!("failed to build p tag: {e}"))?,
        Tag::parse(["consent", consent_path, actor_pubkey_hex])
            .map_err(|e| anyhow::anyhow!("failed to build consent tag: {e}"))?,
        Tag::parse(["e", request_event_id])
            .map_err(|e| anyhow::anyhow!("failed to build e tag: {e}"))?,
    ];

    if let Some(reason) = reason {
        tags.push(
            Tag::parse(["reason", reason])
                .map_err(|e| anyhow::anyhow!("failed to build reason tag: {e}"))?,
        );
    }
    if let Some(replaced_by) = replaced_by {
        tags.push(
            Tag::parse(["replaced-by", replaced_by])
                .map_err(|e| anyhow::anyhow!("failed to build replaced-by tag: {e}"))?,
        );
    }

    let event = EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags(tags)
        .sign_with_keys(&state.relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:{kind}: {e}"))?;

    let (stored, was_inserted) = state
        .db
        .insert_event(tenant.community(), &event, None)
        .await?;
    if !was_inserted {
        return Ok(());
    }

    dispatch_persistent_event(tenant, state, &stored, kind, &relay_pubkey_hex, None).await;

    info!(
        target = %target_pubkey_hex,
        relay = %relay_pubkey_hex,
        kind,
        consent = %consent_path,
        "NIP-IA delta event published"
    );
    Ok(())
}

/// Publish a kind:8002 archived-identity delta event (NIP-IA).
#[allow(clippy::too_many_arguments)]
pub async fn publish_nipia_archived(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    target_pubkey_hex: &str,
    consent_path: &str,
    actor_pubkey_hex: &str,
    request_event_id: &str,
    content: &str,
    reason: Option<&str>,
    replaced_by: Option<&str>,
) -> anyhow::Result<()> {
    publish_nipia_delta(
        tenant,
        state,
        KIND_IA_ARCHIVED,
        target_pubkey_hex,
        consent_path,
        actor_pubkey_hex,
        request_event_id,
        content,
        reason,
        replaced_by,
    )
    .await
}

/// Publish a kind:8003 unarchived-identity delta event (NIP-IA).
#[allow(clippy::too_many_arguments)]
pub async fn publish_nipia_unarchived(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    target_pubkey_hex: &str,
    consent_path: &str,
    actor_pubkey_hex: &str,
    request_event_id: &str,
    content: &str,
    reason: Option<&str>,
) -> anyhow::Result<()> {
    publish_nipia_delta(
        tenant,
        state,
        KIND_IA_UNARCHIVED,
        target_pubkey_hex,
        consent_path,
        actor_pubkey_hex,
        request_event_id,
        content,
        reason,
        None,
    )
    .await
}

fn topic_for_subscription(channel_id: Option<Uuid>) -> EventTopic {
    match channel_id {
        Some(channel_id) => EventTopic::Channel(channel_id),
        None => EventTopic::Global,
    }
}
