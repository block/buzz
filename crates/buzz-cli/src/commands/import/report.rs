//! Dry-run planning and fidelity totals for Slack imports.

use std::collections::{HashMap, HashSet};

use super::export::{SlackChannel, SlackConversationKind, SlackExport, SlackMessage};
use super::print_json;
use super::render::emoji_for_shortcode;
use super::state::ImportState;
use crate::error::CliError;

pub(super) fn dry_run_report(
    export: &SlackExport,
    selected: &[&SlackChannel],
    state: &ImportState,
    channel_map: &HashMap<String, String>,
    bindings: &HashMap<String, String>,
    dm_blockers: &HashMap<String, String>,
    skip_reactions: bool,
) -> Result<(), CliError> {
    let mut channels_to_create = 0u64;
    let mut channels_to_adopt = 0u64;
    let mut channels_already_in_state = 0u64;
    let mut messages = 0u64;
    let mut reaction_groups = 0u64;
    let mut reaction_uses = 0u64;
    let mut public_channels = 0u64;
    let mut private_channels = 0u64;
    let mut direct_messages = 0u64;
    let mut group_direct_messages = 0u64;
    let mut archived = 0u64;
    let mut private_members = 0u64;
    let mut mappable_private_members = 0u64;
    let mut mapped_private_members = 0u64;
    let mut message_records_seen = 0u64;
    let mut non_message_records = 0u64;
    let mut system_records = 0u64;
    let mut mutation_records = 0u64;
    let mut contentless_records = 0u64;
    let mut duplicate_records = 0u64;
    let mut dm_conversations_blocked = Vec::new();
    let mut dm_messages_blocked = 0u64;
    let mapped: HashSet<&str> = bindings.keys().map(String::as_str).collect();
    for channel in selected {
        match channel.kind {
            SlackConversationKind::PublicChannel => public_channels += 1,
            SlackConversationKind::PrivateChannel => private_channels += 1,
            SlackConversationKind::DirectMessage => direct_messages += 1,
            SlackConversationKind::GroupDirectMessage => group_direct_messages += 1,
        }
        if channel.is_archived {
            archived += 1;
        }
        if channel.kind.is_private() {
            private_members += channel.members.len() as u64;
            let mappable: Vec<&String> = channel
                .members
                .iter()
                .filter(|id| export.is_mappable_member(id))
                .collect();
            mappable_private_members += mappable.len() as u64;
            mapped_private_members += mappable
                .into_iter()
                .filter(|id| mapped.contains(id.as_str()))
                .count() as u64;
        }
        let already_in_state = state.channels.contains_key(&channel.id);
        let mapped_existing_channel = channel_map.contains_key(&channel.id);
        let dm_blocker = dm_blockers.get(&channel.id);
        if let Some(reason) = &dm_blocker {
            dm_conversations_blocked.push(serde_json::json!({
                "conversation_id": channel.id,
                "conversation_name": channel.name,
                "kind": channel.kind.as_str(),
                "reason": reason,
            }));
        } else if already_in_state {
            channels_already_in_state += 1;
        } else if mapped_existing_channel {
            channels_to_adopt += 1;
        } else {
            channels_to_create += 1;
        }
        let scan = export.channel_message_scan(channel)?;
        message_records_seen += scan.records_seen;
        non_message_records += scan.non_message_records;
        system_records += scan.system_records;
        mutation_records += scan.mutation_records;
        contentless_records += scan.contentless_records;
        duplicate_records += scan.duplicate_records;
        for msg in scan.messages {
            let message_key = ImportState::message_key(&channel.id, &msg.ts);
            if !state.messages.contains_key(&message_key) {
                if dm_blocker.is_some() {
                    dm_messages_blocked += 1;
                } else {
                    messages += 1;
                }
            }
            if !skip_reactions && dm_blocker.is_none() {
                reaction_groups += pending_reaction_count(state, &message_key, &msg) as u64;
                reaction_uses += msg
                    .reactions
                    .iter()
                    .map(|reaction| {
                        reaction
                            .count
                            .max(u64::try_from(reaction.users.len()).unwrap_or(u64::MAX))
                    })
                    .sum::<u64>();
            }
        }
    }
    print_json(&serde_json::json!({
        "dry_run": true,
        "conversations_selected": selected.len(),
        "public_channels_selected": public_channels,
        "private_channels_selected": private_channels,
        "direct_messages_selected": direct_messages,
        "group_direct_messages_selected": group_direct_messages,
        "archived_conversations_selected": archived,
        "channels_to_create": channels_to_create,
        "channels_to_adopt": channels_to_adopt,
        "channels_already_in_state": channels_already_in_state,
        "dm_conversations_blocked": dm_conversations_blocked,
        "dm_messages_blocked": dm_messages_blocked,
        "message_records_seen": message_records_seen,
        "messages_to_import": messages,
        "non_message_records_skipped": non_message_records,
        "system_records_skipped": system_records,
        "mutation_records_skipped": mutation_records,
        "contentless_records_skipped": contentless_records,
        "duplicate_records_collapsed": duplicate_records,
        "reaction_groups_to_import": reaction_groups,
        "reaction_uses_in_export": reaction_uses,
        "private_memberships_in_export": private_members,
        "private_memberships_mappable": mappable_private_members,
        "private_memberships_mapped": mapped_private_members,
        "private_memberships_unmapped": mappable_private_members
            .saturating_sub(mapped_private_members),
        "bindings_to_publish": bindings.len(),
    }))
}

/// Number of distinct bot-signed reactions that still need publishing.
pub(super) fn pending_reaction_count(
    state: &ImportState,
    message_key: &str,
    msg: &SlackMessage,
) -> usize {
    msg.reactions
        .iter()
        .map(|reaction| emoji_for_shortcode(&reaction.name))
        .filter(|emoji| !state.reactions.contains(&format!("{message_key}:{emoji}")))
        .collect::<HashSet<_>>()
        .len()
}
