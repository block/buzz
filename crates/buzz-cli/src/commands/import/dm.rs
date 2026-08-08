//! Native Buzz DM validation and event construction for Slack imports.

use std::collections::HashMap;

use nostr::{EventBuilder, Tag};
use uuid::Uuid;

use super::export::{SlackChannel, SlackConversationKind, SlackExport};
use super::state::ImportState;
use crate::error::CliError;

pub(super) fn dm_import_blockers(
    export: &SlackExport,
    selected: &[&SlackChannel],
    state: &ImportState,
    bindings: &HashMap<String, String>,
    importer_pubkey: &str,
) -> HashMap<String, String> {
    let mut blockers = HashMap::new();
    let mut participant_sets: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for conversation in selected.iter().copied().filter(|conversation| {
        matches!(
            conversation.kind,
            SlackConversationKind::DirectMessage | SlackConversationKind::GroupDirectMessage
        )
    }) {
        let is_new = !state.channels.contains_key(&conversation.id);
        match mapped_dm_participants(export, conversation, bindings, importer_pubkey) {
            Ok(participants) => {
                participant_sets
                    .entry(participants.join(":"))
                    .or_default()
                    .push((conversation.id.clone(), is_new));
            }
            Err(error) if is_new => {
                blockers.insert(conversation.id.clone(), error.to_string());
            }
            Err(_) => {}
        }
    }

    for conversations in participant_sets.values() {
        if conversations.len() < 2 || !conversations.iter().any(|(_, is_new)| *is_new) {
            continue;
        }
        let mut ids: Vec<&str> = conversations.iter().map(|(id, _)| id.as_str()).collect();
        ids.sort_unstable();
        let reason = format!(
            "Slack conversations {} map to the same Buzz DM participant set; Buzz has one \
             native DM per participant set, so importing them would merge separate histories",
            ids.join(", ")
        );
        for (id, is_new) in conversations {
            if *is_new {
                blockers.insert(id.clone(), reason.clone());
            }
        }
    }
    blockers
}

pub(super) fn response_payload(response: &str) -> Option<serde_json::Value> {
    let parsed: serde_json::Value = serde_json::from_str(response).ok()?;
    let message = parsed.get("message")?.as_str()?;
    serde_json::from_str(message.strip_prefix("response:")?).ok()
}

pub(super) fn mapped_dm_participants(
    export: &SlackExport,
    channel: &SlackChannel,
    bindings: &HashMap<String, String>,
    importer_pubkey: &str,
) -> Result<Vec<String>, CliError> {
    let mappable_ids: Vec<&str> = channel
        .members
        .iter()
        .map(String::as_str)
        .filter(|id| export.is_mappable_member(id))
        .collect();
    let missing = mappable_ids
        .iter()
        .filter(|id| !bindings.contains_key(**id))
        .count();
    if missing > 0 {
        return Err(CliError::Usage(format!(
            "Slack {} {} has {missing} unmapped active participant(s); provide every mapping \
             with --identity-map before importing DM history",
            channel.kind.as_str(),
            channel.id
        )));
    }

    let participant_pubkeys: std::collections::BTreeSet<String> = mappable_ids
        .into_iter()
        .filter_map(|id| bindings.get(id).cloned())
        .collect();
    if !participant_pubkeys.contains(importer_pubkey) {
        return Err(CliError::Usage(format!(
            "Slack {} {} does not map any participant to the importer public key; refusing \
             to add the migration operator as an extra DM member",
            channel.kind.as_str(),
            channel.id
        )));
    }
    if participant_pubkeys.len() < 2 {
        return Err(CliError::Usage(format!(
            "Slack {} {} has fewer than two mappable human participants",
            channel.kind.as_str(),
            channel.id
        )));
    }
    if participant_pubkeys.len() > 9 {
        return Err(CliError::Usage(format!(
            "Slack {} {} has {} mapped participants; Buzz DMs support at most 9",
            channel.kind.as_str(),
            channel.id,
            participant_pubkeys.len()
        )));
    }
    Ok(participant_pubkeys.into_iter().collect())
}

pub(super) fn build_import_dm_open(
    channel: &SlackChannel,
    team_id: &str,
    participant_pubkeys: &[String],
    importer_pubkey: &str,
) -> Result<EventBuilder, CliError> {
    let participant_refs: Vec<&str> = participant_pubkeys
        .iter()
        .filter(|pubkey| pubkey.as_str() != importer_pubkey)
        .map(String::as_str)
        .collect();
    let mut tags = Vec::new();
    let import_id = format!("slack:{team_id}:{}", channel.id);
    tags.push(
        Tag::parse(["d", import_id.as_str()])
            .map_err(|e| CliError::Other(format!("invalid DM import d-tag: {e}")))?,
    );
    tags.push(
        Tag::parse(["import", "slack"])
            .map_err(|e| CliError::Other(format!("invalid import tag: {e}")))?,
    );
    let foreign_id = format!("{team_id}:{}", channel.id);
    tags.push(
        Tag::parse([
            "import_conversation",
            foreign_id.as_str(),
            channel.kind.as_str(),
        ])
        .map_err(|e| CliError::Other(format!("invalid conversation provenance tag: {e}")))?,
    );
    buzz_sdk::build_dm_open(&participant_refs)
        .map_err(|e| CliError::Other(format!("build_dm_open failed: {e}")))
        .map(|builder| builder.tags(tags))
}

/// Reject a relay-assigned native DM UUID that is already owned by another
/// Slack conversation in the resume ledger. Buzz intentionally reuses a DM
/// for the same participant set, whereas Slack exports may contain multiple
/// distinct D/MPIM ids for that set; merging their histories is never safe.
pub(super) fn ensure_dm_uuid_unclaimed(
    state: &ImportState,
    channel: &SlackChannel,
    uuid: Uuid,
) -> Result<(), CliError> {
    let uuid = uuid.to_string();
    if let Some((other, _)) = state
        .channels
        .iter()
        .find(|(id, saved)| id.as_str() != channel.id && saved.uuid == uuid)
    {
        return Err(CliError::Usage(format!(
            "Slack {} {} resolves to the Buzz DM already imported for {other}; \
             refusing to merge separate histories",
            channel.kind.as_str(),
            channel.id
        )));
    }
    Ok(())
}
