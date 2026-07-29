//! `buzz import` — migrate history from external workspaces.
//!
//! v1 supports Slack workspace exports; see `docs/slack-import.md` for the
//! full design (attribution model, security, limitations).
//!
//! ## Attribution model (zero key custody, two-party consent)
//!
//! Every imported event is signed by the CLI identity (bot mode) and carries
//! `import`/`import_author`/`import_ts` provenance tags. Real people are
//! attributed by a **two-party identity binding**, using **public keys only** —
//! no private key is ever generated for or distributed to anyone:
//!
//! 1. An owner/admin **attestation** (kind `KIND_IMPORT_IDENTITY_BINDING`)
//!    mapping a Slack user id to a person's Buzz pubkey — `buzz import bind` /
//!    `--identity-map`.
//! 2. The subject's own **claim** (kind `KIND_IMPORT_IDENTITY_CLAIM`),
//!    self-signed with their key — `buzz import claim`.
//!
//! History renders under the real person only when both exist for the same
//! Slack id and the attestation's pubkey equals the claim's signer. So a member
//! cannot claim another person's history (no admin attestation), and an admin
//! cannot make someone appear to author history they never wrote (no subject
//! claim). See `docs/slack-import.md` for the residual trust in a colluding
//! admin + subject.

mod export;
mod importer;
mod mapping;
mod mrkdwn;
mod state;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use nostr::PublicKey;

use crate::client::BuzzClient;
use crate::error::CliError;
use export::{SlackChannel, SlackConversationKind, SlackExport, SlackMessage};
use importer::{
    emoji_for_shortcode, mapped_dm_participants, publish_binding, submit, Importer, ImporterOptions,
};
use mapping::load_channel_map;
use state::{ChannelState, ImportState};

/// Parameters for `buzz import slack`.
pub struct ImportSlackParams {
    /// One or more unzipped Slack export directories. Multiple roots support
    /// Slackdump's separate public/private export passes.
    pub export_dirs: Vec<String>,
    /// Slack workspace id (team id) — namespaces identity bindings and channel
    /// UUIDs so ids can't collide across workspaces.
    pub team_id: String,
    /// State file path override.
    pub state: Option<String>,
    /// Optional Slack conversation id → existing Buzz channel UUID crosswalk.
    pub channel_map: Option<String>,
    /// Optional comma-separated channel-name filter.
    pub channels: Option<String>,
    /// Report the plan without writing anything.
    pub dry_run: bool,
    /// Skip reaction import.
    pub skip_reactions: bool,
    /// Optional `SLACKID=npub,SLACKID=hex,…` identity bindings to publish
    /// (owner/admin-signed) so imported history renders under real people.
    pub identity_map: Option<String>,
}

pub async fn cmd_import_slack(client: &BuzzClient, p: ImportSlackParams) -> Result<(), CliError> {
    let team_id = validate_team_id(&p.team_id)?.to_string();
    let export_dirs: Vec<PathBuf> = p.export_dirs.iter().map(PathBuf::from).collect();
    let export = SlackExport::load_many(&export_dirs)?;
    let channel_map = p
        .channel_map
        .as_deref()
        .map(PathBuf::from)
        .map(|path| load_channel_map(&path))
        .transpose()?
        .unwrap_or_default();

    let state_path = p
        .state
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| export_dirs[0].join("buzz-import-state.json"));
    let mut state = ImportState::load_for_workspace(&state_path, &team_id)?;
    validate_channel_map(&export, &state, &channel_map)?;

    // Slack user id → display name, for mrkdwn mention rewriting and
    // author attribution tags.
    let names: HashMap<String, String> = export
        .users
        .iter()
        .map(|(id, u)| (id.clone(), u.best_name().to_string()))
        .collect();

    // Parse identity bindings (Slack id → Buzz pubkey), public keys only.
    let bindings = parse_identity_map(p.identity_map.as_deref())?;
    let binding_map: HashMap<String, String> = bindings.iter().cloned().collect();

    let selected = select_channels(&export, p.channels.as_deref())?;
    let dm_blockers = dm_import_blockers(
        &export,
        &selected,
        &state,
        &binding_map,
        &client.keys().public_key().to_hex(),
    );

    if p.dry_run {
        return dry_run_report(
            &export,
            &selected,
            &state,
            &channel_map,
            &binding_map,
            &dm_blockers,
            p.skip_reactions,
        );
    }
    if !dm_blockers.is_empty() {
        let mut blocked: Vec<String> = dm_blockers
            .iter()
            .map(|(id, reason)| format!("{id}: {reason}"))
            .collect();
        blocked.sort();
        return Err(CliError::Usage(format!(
            "{} Slack DM/MPIM conversation(s) are not safe to import:\n{}",
            blocked.len(),
            blocked.join("\n")
        )));
    }

    seed_channel_map(&mut state, &channel_map);
    let mut importer = Importer::new(
        client,
        &export,
        &names,
        ImporterOptions {
            team_id: &team_id,
            state,
            state_path,
            skip_reactions: p.skip_reactions,
            bindings: &binding_map,
        },
    );

    for channel in &selected {
        importer.import_channel(channel).await?;
    }

    // Publish owner/admin-signed identity bindings last, so the history they
    // attribute is already in place.
    importer.publish_bindings(&bindings).await?;

    importer.finish()
}

/// Publish the owner/admin half of a two-party binding: an attestation that
/// `slack_id` maps to `pubkey`. Inert until the subject also runs
/// `cmd_import_claim` with their own key.
pub async fn cmd_import_bind(
    client: &BuzzClient,
    team_id: &str,
    slack_id: &str,
    pubkey: &str,
) -> Result<(), CliError> {
    let team_id = validate_team_id(team_id)?;
    let slack_id = validate_slack_id(slack_id)?;
    let pubkey_hex = parse_pubkey(pubkey)?;
    let event_id = publish_binding(client, team_id, slack_id, &pubkey_hex).await?;
    print_json(&serde_json::json!({
        "event_id": event_id,
        "team_id": team_id,
        "slack_id": slack_id,
        "pubkey": pubkey_hex,
        "accepted": true,
    }))
}

/// Publish the subject half of a two-party binding: the caller's self-signed
/// consent to being attributed `slack_id`. Signed by the CLI identity, so the
/// person whose history it is runs this with their own key. Inert until a
/// community owner/admin has published the matching attestation for this
/// pubkey.
pub async fn cmd_import_claim(
    client: &BuzzClient,
    team_id: &str,
    slack_id: &str,
) -> Result<(), CliError> {
    let team_id = validate_team_id(team_id)?;
    let slack_id = validate_slack_id(slack_id)?;
    let d_tag = buzz_sdk::slack_identity_binding_d_tag(team_id, slack_id);
    let builder = buzz_sdk::build_import_identity_claim(&d_tag)
        .map_err(|e| CliError::Other(format!("build_import_identity_claim failed: {e}")))?;
    let event_id = submit(client, builder).await?;
    print_json(&serde_json::json!({
        "event_id": event_id,
        "team_id": team_id,
        "slack_id": slack_id,
        "pubkey": client.keys().public_key().to_hex(),
        "accepted": true,
    }))
}

/// Parse a `SLACKID=key,SLACKID=key` list into `(slack_id, pubkey_hex)` pairs.
/// Each key may be an `npub1…` or a 64-char hex pubkey — **public keys only**.
fn parse_identity_map(spec: Option<&str>) -> Result<Vec<(String, String)>, CliError> {
    let Some(spec) = spec else {
        return Ok(Vec::new());
    };
    let entries: Vec<(String, String)> = spec
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|entry| {
            let (slack_id, key) = entry.split_once('=').ok_or_else(|| {
                CliError::Usage(format!(
                    "--identity-map entry must be SLACKID=npub-or-hex (got {entry:?})"
                ))
            })?;
            Ok((
                validate_slack_id(slack_id)?.to_string(),
                parse_pubkey(key.trim())?,
            ))
        })
        .collect::<Result<_, CliError>>()?;
    let mut seen = HashSet::new();
    for (slack_id, _) in &entries {
        if !seen.insert(slack_id) {
            return Err(CliError::Usage(format!(
                "--identity-map contains duplicate Slack id {slack_id}"
            )));
        }
    }
    Ok(entries)
}

/// Validate and normalize a Slack user id supplied on the command line.
fn validate_slack_id(slack_id: &str) -> Result<&str, CliError> {
    validate_slack_ident(slack_id, "user id")
}

/// Validate and normalize a Slack workspace (team) id supplied on the command
/// line. Same character rules as a user id.
fn validate_team_id(team_id: &str) -> Result<&str, CliError> {
    validate_slack_ident(team_id, "workspace (team) id")
}

fn validate_slack_ident<'a>(value: &'a str, label: &str) -> Result<&'a str, CliError> {
    let value = value.trim();
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CliError::Usage(format!("invalid Slack {label} {value:?}")));
    }
    Ok(value)
}

/// Parse an `npub1…` or 64-char hex string into a hex pubkey. Rejects nsec so
/// a private key can never be passed where a public key belongs.
fn parse_pubkey(key: &str) -> Result<String, CliError> {
    if key.starts_with("nsec1") {
        return Err(CliError::Usage(
            "identity bindings take a PUBLIC key (npub or hex), not an nsec".into(),
        ));
    }
    PublicKey::parse(key)
        .map(|pk| pk.to_hex())
        .map_err(|_| CliError::Usage(format!("invalid pubkey (expected npub or 64-hex): {key}")))
}

/// Resolve the `--channels` filter against conversation names or Slack IDs,
/// erroring if any selector is unknown.
fn select_channels<'e>(
    export: &'e SlackExport,
    filter: Option<&str>,
) -> Result<Vec<&'e SlackChannel>, CliError> {
    let filter: Option<HashSet<String>> = filter.map(|list| {
        list.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    if let Some(requested) = &filter {
        let available: HashSet<&str> = export
            .channels
            .iter()
            .flat_map(|c| [c.name.as_str(), c.id.as_str()])
            .collect();
        let mut missing: Vec<&str> = requested
            .iter()
            .map(String::as_str)
            .filter(|name| !available.contains(name))
            .collect();
        missing.sort_unstable();
        if !missing.is_empty() {
            return Err(CliError::Usage(format!(
                "unknown channel(s) in --channels: {}",
                missing.join(", ")
            )));
        }
    }
    let selected: Vec<&SlackChannel> = export
        .channels
        .iter()
        .filter(|c| {
            filter
                .as_ref()
                .is_none_or(|f| f.contains(&c.name) || f.contains(&c.id))
        })
        .collect();
    if selected.is_empty() {
        return Err(CliError::Usage(
            "no conversations selected — check --channels against Slack names or ids".into(),
        ));
    }
    Ok(selected)
}

fn validate_channel_map(
    export: &SlackExport,
    state: &ImportState,
    channel_map: &HashMap<String, String>,
) -> Result<(), CliError> {
    let conversations: HashMap<&str, &SlackChannel> = export
        .channels
        .iter()
        .map(|conversation| (conversation.id.as_str(), conversation))
        .collect();
    for (slack_id, buzz_id) in channel_map {
        let conversation = conversations.get(slack_id.as_str()).ok_or_else(|| {
            CliError::Usage(format!(
                "channel map references Slack conversation {slack_id}, which is absent from the export"
            ))
        })?;
        if matches!(
            conversation.kind,
            SlackConversationKind::DirectMessage | SlackConversationKind::GroupDirectMessage
        ) {
            return Err(CliError::Usage(format!(
                "channel map cannot adopt Slack {} {slack_id}; DMs and MPIMs must be opened \
                 natively from their complete mapped participant set",
                conversation.kind.as_str()
            )));
        }
        if let Some(existing) = state.channels.get(slack_id) {
            if existing.uuid != *buzz_id {
                return Err(CliError::Usage(format!(
                    "channel map assigns Slack conversation {slack_id} to {buzz_id}, but the \
                     state file already assigns it to {}",
                    existing.uuid
                )));
            }
        }
    }
    Ok(())
}

fn seed_channel_map(state: &mut ImportState, channel_map: &HashMap<String, String>) {
    for (slack_id, buzz_id) in channel_map {
        state
            .channels
            .entry(slack_id.clone())
            .or_insert_with(|| ChannelState {
                uuid: buzz_id.clone(),
                metadata_done: false,
                archived_done: false,
                private_visibility_done: false,
                prepared_for_import: false,
                members_added: HashSet::new(),
            });
    }
}

fn dm_import_blockers(
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

fn dry_run_report(
    export: &SlackExport,
    selected: &[&SlackChannel],
    st: &ImportState,
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
        let already_in_state = st.channels.contains_key(&channel.id);
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
            if !st.messages.contains_key(&message_key) {
                if dm_blocker.is_some() {
                    dm_messages_blocked += 1;
                } else {
                    messages += 1;
                }
            }
            if !skip_reactions && dm_blocker.is_none() {
                reaction_groups += pending_reaction_count(st, &message_key, &msg) as u64;
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
        "private_memberships_unmapped": mappable_private_members.saturating_sub(mapped_private_members),
        "bindings_to_publish": bindings.len(),
    }))
}

/// Number of distinct bot-signed reactions that still need publishing.
fn pending_reaction_count(st: &ImportState, message_key: &str, msg: &SlackMessage) -> usize {
    msg.reactions
        .iter()
        .map(|reaction| emoji_for_shortcode(&reaction.name))
        .filter(|emoji| !st.reactions.contains(&format!("{message_key}:{emoji}")))
        .collect::<HashSet<_>>()
        .len()
}

/// Serialize `value` to compact JSON on stdout.
fn print_json(value: &serde_json::Value) -> Result<(), CliError> {
    let rendered = serde_json::to_string(value)
        .map_err(|e| CliError::Other(format!("summary serialization failed: {e}")))?;
    println!("{rendered}");
    Ok(())
}

/// Dispatch for `buzz import`.
pub async fn dispatch(cmd: crate::ImportCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::ImportCmd::Slack {
            export_dirs,
            team_id,
            state,
            channel_map,
            channels,
            dry_run,
            skip_reactions,
            identity_map,
        } => {
            cmd_import_slack(
                client,
                ImportSlackParams {
                    export_dirs,
                    team_id,
                    state,
                    channel_map,
                    channels,
                    dry_run,
                    skip_reactions,
                    identity_map,
                },
            )
            .await
        }
        crate::ImportCmd::Bind {
            team_id,
            slack_id,
            pubkey,
        } => cmd_import_bind(client, &team_id, &slack_id, &pubkey).await,
        crate::ImportCmd::Claim { team_id, slack_id } => {
            cmd_import_claim(client, &team_id, &slack_id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::importer::{
        author_display, author_id, build_import_dm_open, build_imported_message, channel_uuid,
        ensure_dm_uuid_unclaimed, is_already_unarchived, mapped_dm_participants, provenance_tags,
        render_attachment, render_slack_blocks, response_payload, thread_root_key,
    };
    use super::*;
    use nostr::{EventId, Keys};
    use uuid::Uuid;

    fn msg(json: &str) -> SlackMessage {
        serde_json::from_str(json).expect("test message parses")
    }

    fn channel() -> SlackChannel {
        serde_json::from_str(r#"{"id":"C1","name":"general"}"#).expect("channel parses")
    }

    #[test]
    fn channel_uuid_is_deterministic_and_team_scoped() {
        // Same inputs → same UUID, so a crash-resumed run reuses the channel
        // instead of minting a duplicate.
        assert_eq!(channel_uuid("T1", "C1"), channel_uuid("T1", "C1"));
        // Distinct team or channel → distinct UUID (no cross-workspace or
        // cross-channel collision).
        assert_ne!(channel_uuid("T1", "C1"), channel_uuid("T2", "C1"));
        assert_ne!(channel_uuid("T1", "C1"), channel_uuid("T1", "C2"));
    }

    #[test]
    fn existing_channel_map_is_validated_and_seeded_as_unprepared() {
        let dir =
            std::env::temp_dir().join(format!("buzz-import-channel-map-export-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("channels.json"),
            r#"[{"id":"C1","name":"general"}]"#,
        )
        .expect("write channels");
        std::fs::write(dir.join("users.json"), "[]").expect("write users");
        let export = SlackExport::load_many(std::slice::from_ref(&dir)).expect("export");
        let mut state =
            ImportState::load_for_workspace(&dir.join("state.json"), "T1").expect("fresh state");
        let buzz_id = Uuid::new_v4().to_string();
        let channel_map = HashMap::from([("C1".to_string(), buzz_id.clone())]);

        validate_channel_map(&export, &state, &channel_map).expect("valid map");
        seed_channel_map(&mut state, &channel_map);
        let adopted = &state.channels["C1"];
        assert_eq!(adopted.uuid, buzz_id);
        assert!(!adopted.prepared_for_import);
        assert!(!adopted.private_visibility_done);
        assert!(!adopted.archived_done);

        let unknown = HashMap::from([("C9".to_string(), Uuid::new_v4().to_string())]);
        assert!(validate_channel_map(&export, &state, &unknown).is_err());
        let conflict = HashMap::from([("C1".to_string(), Uuid::new_v4().to_string())]);
        assert!(validate_channel_map(&export, &state, &conflict).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn native_dm_open_requires_the_exact_mapped_participant_set() {
        let dir =
            std::env::temp_dir().join(format!("buzz-import-dm-participants-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("D1")).expect("mkdir");
        std::fs::write(dir.join("channels.json"), "[]").expect("write channels");
        std::fs::write(
            dir.join("users.json"),
            r#"[{"id":"U1","name":"alice"},{"id":"U2","name":"bob"}]"#,
        )
        .expect("write users");
        std::fs::write(
            dir.join("dms.json"),
            r#"[{"id":"D1","created":1700000000,"members":["U1","U2"]}]"#,
        )
        .expect("write dms");
        let export = SlackExport::load_many(std::slice::from_ref(&dir)).expect("load export");
        let dm = export
            .channels
            .iter()
            .find(|conversation| conversation.id == "D1")
            .expect("DM");

        let importer_keys = Keys::generate();
        let importer_pubkey = importer_keys.public_key().to_hex();
        let other_pubkey = Keys::generate().public_key().to_hex();
        let mut bindings = HashMap::from([("U1".to_string(), importer_pubkey.clone())]);
        assert!(mapped_dm_participants(&export, dm, &bindings, &importer_pubkey).is_err());

        bindings.insert("U2".to_string(), other_pubkey.clone());
        let participants =
            mapped_dm_participants(&export, dm, &bindings, &importer_pubkey).expect("complete map");
        assert_eq!(
            participants.iter().cloned().collect::<HashSet<_>>(),
            HashSet::from([importer_pubkey.clone(), other_pubkey.clone()])
        );
        assert!(
            mapped_dm_participants(
                &export,
                dm,
                &bindings,
                &Keys::generate().public_key().to_hex()
            )
            .is_err(),
            "migration operator cannot become an extra DM participant"
        );

        let event = build_import_dm_open(dm, "T1", &participants, &importer_pubkey)
            .expect("build native DM open")
            .sign_with_keys(&importer_keys)
            .expect("sign");
        assert_eq!(event.kind.as_u16(), 41010);
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert!(tags.contains(&vec!["p".into(), other_pubkey]));
        assert!(!tags.contains(&vec!["p".into(), importer_pubkey]));
        assert!(tags.contains(&vec!["d".into(), "slack:T1:D1".into()]));
        assert!(tags.contains(&vec!["import".into(), "slack".into()]));
        assert!(tags.contains(&vec![
            "import_conversation".into(),
            "T1:D1".into(),
            "direct_message".into()
        ]));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn native_dm_response_payload_extracts_relay_channel_id() {
        let channel_id = Uuid::new_v4();
        let response = serde_json::json!({
            "event_id": "abc",
            "accepted": true,
            "message": format!(
                "response:{}",
                serde_json::json!({"channel_id": channel_id, "created": true})
            ),
        })
        .to_string();
        let payload = response_payload(&response).expect("response payload");
        let channel_id_string = channel_id.to_string();
        assert_eq!(
            payload
                .get("channel_id")
                .and_then(serde_json::Value::as_str),
            Some(channel_id_string.as_str())
        );
        assert_eq!(
            payload.get("created").and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn separate_slack_dms_cannot_collapse_into_one_buzz_participant_set() {
        let dir = std::env::temp_dir().join(format!("buzz-import-dm-collision-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("D1")).expect("mkdir D1");
        std::fs::create_dir_all(dir.join("D2")).expect("mkdir D2");
        std::fs::write(dir.join("channels.json"), "[]").expect("channels");
        std::fs::write(
            dir.join("users.json"),
            r#"[{"id":"U1","name":"alice"},{"id":"U2","name":"bob"}]"#,
        )
        .expect("users");
        std::fs::write(
            dir.join("dms.json"),
            r#"[{"id":"D1","members":["U1","U2"]},{"id":"D2","members":["U1","U2"]}]"#,
        )
        .expect("dms");
        let export = SlackExport::load_many(std::slice::from_ref(&dir)).expect("export");
        let selected: Vec<&SlackChannel> = export.channels.iter().collect();
        let importer_pubkey = Keys::generate().public_key().to_hex();
        let bindings = HashMap::from([
            ("U1".to_string(), importer_pubkey.clone()),
            ("U2".to_string(), Keys::generate().public_key().to_hex()),
        ]);
        let state = ImportState::load_for_workspace(&dir.join("state.json"), "T1").expect("state");
        let blockers = dm_import_blockers(&export, &selected, &state, &bindings, &importer_pubkey);
        assert_eq!(blockers.len(), 2);
        assert!(blockers["D1"].contains("merge separate histories"));
        assert!(blockers["D2"].contains("merge separate histories"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resumed_dm_cannot_reuse_another_slack_conversations_buzz_uuid() {
        let uuid = Uuid::new_v4();
        let mut state = ImportState::default();
        state.channels.insert(
            "D1".into(),
            ChannelState {
                uuid: uuid.to_string(),
                metadata_done: true,
                archived_done: true,
                private_visibility_done: true,
                prepared_for_import: true,
                members_added: HashSet::new(),
            },
        );
        let mut d2 = channel();
        d2.id = "D2".into();
        d2.name = "dm-two".into();
        d2.kind = SlackConversationKind::DirectMessage;

        let error = ensure_dm_uuid_unclaimed(&state, &d2, uuid)
            .expect_err("different Slack DM must not claim an existing Buzz DM");
        assert!(error.to_string().contains("already imported for D1"));
        assert!(error.to_string().contains("merge separate histories"));
    }

    #[test]
    fn active_channel_unarchive_rejection_is_the_only_accepted_no_op() {
        assert!(is_already_unarchived(&CliError::Other(
            "relay rejected event: channel is not archived".into()
        )));
        assert!(is_already_unarchived(&CliError::Relay {
            status: 409,
            body: "channel is not archived".into(),
        }));
        assert!(!is_already_unarchived(&CliError::Other(
            "channel does not exist".into()
        )));
    }

    #[test]
    fn author_resolution() {
        let user_msg = msg(r#"{"type":"message","user":"U1","text":"x","ts":"1.0"}"#);
        assert_eq!(author_id(&user_msg).as_deref(), Some("U1"));

        let bot_msg = msg(
            r#"{"type":"message","subtype":"bot_message","bot_id":"B9","username":"CI","text":"x","ts":"1.0"}"#,
        );
        assert_eq!(author_id(&bot_msg).as_deref(), Some("B9"));

        let mut names = HashMap::new();
        names.insert("U1".to_string(), "alice".to_string());
        assert_eq!(author_display(&user_msg, &names), "alice");
        assert_eq!(author_display(&bot_msg, &names), "CI");
    }

    #[test]
    fn imported_message_escapes_markdown_in_the_author_prefix() {
        let message = msg(r#"{"type":"message","user":"U1","text":"hello","ts":"100.0"}"#);
        let names = HashMap::from([("U1".to_string(), r"A*B_C\D[bot]`".to_string())]);
        let event =
            build_imported_message(&channel(), Uuid::new_v4(), &message, &names, "T1", None)
                .expect("builder")
                .sign_with_keys(&Keys::generate())
                .expect("signs");

        assert_eq!(event.content, r"**A\*B\_C\\D\[bot\]\`**: hello");
    }

    #[test]
    fn thread_root_key_only_for_replies() {
        let channel: SlackChannel =
            serde_json::from_str(r#"{"id":"C1","name":"general"}"#).expect("channel parses");
        let root = msg(r#"{"type":"message","user":"U1","text":"x","ts":"5.0","thread_ts":"5.0"}"#);
        assert_eq!(thread_root_key(&channel, &root), None);
        let reply =
            msg(r#"{"type":"message","user":"U1","text":"y","ts":"6.0","thread_ts":"5.0"}"#);
        assert_eq!(
            thread_root_key(&channel, &reply),
            Some("C1:5.0".to_string())
        );
        let plain = msg(r#"{"type":"message","user":"U1","text":"z","ts":"7.0"}"#);
        assert_eq!(thread_root_key(&channel, &plain), None);
    }

    #[test]
    fn emoji_mapping() {
        assert_eq!(emoji_for_shortcode("+1"), "👍");
        assert_eq!(emoji_for_shortcode("thumbsup::skin-tone-3"), "👍");
        assert_eq!(emoji_for_shortcode("party_parrot"), ":party_parrot:");
    }

    #[test]
    fn identity_map_parses_npub_and_hex_and_rejects_nsec() {
        let hex = "8f3904246ba9d9cc7e821e7752e123d435234d17c2513d85785f4a0b1ca07e56";
        let parsed = parse_identity_map(Some(&format!("U1={hex}"))).expect("parses hex");
        assert_eq!(parsed, vec![("U1".to_string(), hex.to_string())]);

        assert!(
            parse_identity_map(Some("U1=nsec1abc")).is_err(),
            "nsec rejected"
        );
        assert!(
            parse_identity_map(Some("U1")).is_err(),
            "missing = rejected"
        );
        assert!(
            parse_identity_map(Some(&format!("U1={hex},U1={hex}"))).is_err(),
            "duplicate Slack ids rejected"
        );
        assert!(
            parse_identity_map(Some(&format!("={hex}"))).is_err(),
            "empty Slack id rejected"
        );
        assert!(parse_identity_map(None).expect("none ok").is_empty());
    }

    #[test]
    fn pending_reactions_include_resumable_work_and_dedupe_aliases() {
        let msg = msg(r#"{"type":"message","user":"U1","text":"x","ts":"1.0",
                "reactions":[{"name":"+1"},{"name":"thumbsup"},{"name":"heart"}]}"#);
        let mut state = ImportState::default();
        assert_eq!(pending_reaction_count(&state, "C1:1.0", &msg), 2);
        state.reactions.insert("C1:1.0:👍".into());
        assert_eq!(pending_reaction_count(&state, "C1:1.0", &msg), 1);
    }

    #[test]
    fn imported_message_keeps_routing_and_provenance_tags() {
        let channel_id = Uuid::new_v4();
        let root = EventId::from_hex(&"11".repeat(32)).expect("event id");
        let thread_ref = buzz_sdk::ThreadRef {
            root_event_id: root,
            parent_event_id: root,
        };
        let message = msg(
            r#"{"type":"message","user":"U1","text":"hello","ts":"100.000002",
                "thread_ts":"99.000001"}"#,
        );
        let mut names = HashMap::new();
        names.insert("U1".to_string(), "Alice".to_string());

        let event = build_imported_message(
            &channel(),
            channel_id,
            &message,
            &names,
            "T1",
            Some(&thread_ref),
        )
        .expect("builder")
        .sign_with_keys(&Keys::generate())
        .expect("signs");
        let tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();

        assert!(tags.contains(&vec!["h".into(), channel_id.to_string()]));
        assert!(tags.iter().any(|tag| tag.first().is_some_and(|v| v == "e")));
        assert!(tags.contains(&vec!["import".into(), "slack".into()]));
        // The import_author id is workspace-scoped (`<team>:<user>`) so it
        // composes to the same `slack:T1:U1` key the identity binding uses.
        assert!(tags.contains(&vec![
            "import_author".into(),
            "T1:U1".into(),
            "Alice".into()
        ]));
        assert!(tags.contains(&vec!["import_ts".into(), "100.000002".into()]));
        assert!(tags.contains(&vec![
            "import_conversation".into(),
            "T1:C1".into(),
            "public_channel".into()
        ]));
        assert_eq!(event.created_at.as_secs(), 100);
    }

    #[test]
    fn imported_thread_broadcast_remains_visible_in_the_channel_timeline() {
        let channel_id = Uuid::new_v4();
        let root = EventId::from_hex(&"11".repeat(32)).expect("event id");
        let thread_ref = buzz_sdk::ThreadRef {
            root_event_id: root,
            parent_event_id: root,
        };
        let message = msg(
            r#"{"type":"message","subtype":"thread_broadcast","user":"U1",
                "text":"shared reply","ts":"100.000002","thread_ts":"99.000001"}"#,
        );
        let event = build_imported_message(
            &channel(),
            channel_id,
            &message,
            &HashMap::new(),
            "T1",
            Some(&thread_ref),
        )
        .expect("builder")
        .sign_with_keys(&Keys::generate())
        .expect("signs");

        assert!(event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.first().map(String::as_str) == Some("broadcast")
                && parts.get(1).map(String::as_str) == Some("1")
        }));
    }

    #[test]
    fn attachment_only_message_retains_visible_card_content() {
        let message = msg(
            r#"{"type":"message","subtype":"bot_message","bot_id":"B1","username":"CI",
                "text":"","ts":"100.000002","attachments":[{
                    "author_name":"Buildkite","author_link":"https://ci.example.test",
                    "title":"main passed","title_link":"https://ci.example.test/build/1",
                    "text":"*42 tests* passed",
                    "fields":[{"title":"Commit","value":"`abc123`"}]
                }]}"#,
        );
        let event = build_imported_message(
            &channel(),
            Uuid::new_v4(),
            &message,
            &HashMap::new(),
            "T1",
            None,
        )
        .expect("builder")
        .sign_with_keys(&Keys::generate())
        .expect("signs");

        assert!(event
            .content
            .contains("[Buildkite](https://ci.example.test)"));
        assert!(event
            .content
            .contains("**[main passed](https://ci.example.test/build/1)**"));
        assert!(event.content.contains("**42 tests** passed"));
        assert!(event.content.contains("**Commit:** `abc123`"));
    }

    #[test]
    fn rich_text_blocks_render_lists_mentions_links_and_styles() {
        let blocks: Vec<serde_json::Value> = serde_json::from_str(
            r#"[{"type":"rich_text","elements":[
                {"type":"rich_text_section","elements":[
                    {"type":"text","text":"Hello ","style":{"bold":true}},
                    {"type":"user","user_id":"U1"},
                    {"type":"text","text":" — "},
                    {"type":"link","url":"https://example.test","text":"details"}
                ]},
                {"type":"rich_text_list","style":"bullet","elements":[
                    {"type":"rich_text_section","elements":[{"type":"text","text":"first"}]},
                    {"type":"rich_text_section","elements":[{"type":"text","text":"second"}]}
                ]}
            ]}]"#,
        )
        .expect("blocks");
        let names = HashMap::from([("U1".to_string(), "Alice".to_string())]);
        let rendered = render_slack_blocks(&blocks, &names);
        assert!(rendered.contains("**Hello **@Alice"));
        assert!(rendered.contains("[details](https://example.test)"));
        assert!(rendered.contains("- first"));
        assert!(rendered.contains("- second"));

        let attachment: export::SlackAttachment = serde_json::from_str(
            r#"{"fallback":"fallback only","original_url":"https://example.test/source"}"#,
        )
        .expect("attachment");
        assert_eq!(
            render_attachment(&attachment, &HashMap::new()),
            "fallback only\n[Slack attachment](https://example.test/source)"
        );
    }

    #[tokio::test]
    async fn dry_run_is_offline_and_reports_counts() {
        let dir = std::env::temp_dir().join(format!("buzz-import-dryrun-{}", std::process::id()));
        let general = dir.join("general");
        std::fs::create_dir_all(&general).expect("mkdir");
        std::fs::write(
            dir.join("channels.json"),
            r#"[{"id":"C1","name":"general"}]"#,
        )
        .expect("write channels");
        std::fs::write(dir.join("users.json"), r#"[{"id":"U1","name":"alice"}]"#)
            .expect("write users");
        std::fs::write(
            general.join("2024-01-01.json"),
            r#"[{"type":"message","user":"U1","text":"hello","ts":"100.0"}]"#,
        )
        .expect("write day");

        // Points at a port nothing listens on — dry run must never dial it.
        let client = BuzzClient::new(
            "http://127.0.0.1:1".to_string(),
            Keys::generate(),
            None,
            None,
        )
        .expect("client");
        cmd_import_slack(
            &client,
            ImportSlackParams {
                export_dirs: vec![dir.display().to_string()],
                team_id: "T1".into(),
                state: None,
                channel_map: None,
                channels: None,
                dry_run: true,
                skip_reactions: false,
                identity_map: None,
            },
        )
        .await
        .expect("dry run succeeds offline");

        let export = SlackExport::load_many(std::slice::from_ref(&dir)).expect("export");
        assert!(select_channels(&export, Some("general,missing")).is_err());
        assert!(!dir.join("buzz-import-state.json").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn provenance_tags_shape() {
        let tags = provenance_tags("U1", "alice", "1.000200").expect("tags build");
        let flat: Vec<Vec<String>> = tags
            .iter()
            .map(|t| t.as_slice().iter().map(|s| s.to_string()).collect())
            .collect();
        assert_eq!(
            flat,
            vec![
                vec!["import".to_string(), "slack".to_string()],
                vec![
                    "import_author".to_string(),
                    "U1".to_string(),
                    "alice".to_string()
                ],
                vec!["import_ts".to_string(), "1.000200".to_string()],
            ]
        );
    }
}
