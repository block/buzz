//! The `buzz import slack` execution engine: the stateful [`Importer`] run loop
//! plus the pure helpers that shape one imported event (message building,
//! provenance tags, emoji mapping, thread resolution) and the relay submit /
//! attestation calls. The command layer in the parent module parses inputs and
//! constructs an [`Importer`]; everything that actually writes to the relay
//! lives here.

use std::collections::HashMap;
use std::path::PathBuf;

use nostr::{EventBuilder, EventId, Timestamp};
use uuid::Uuid;

use super::channel_state::require_unarchived;
use super::dm::{
    build_import_dm_open, ensure_dm_uuid_unclaimed, mapped_dm_participants, response_payload,
};
use super::export::{ts_seconds, SlackChannel, SlackExport, SlackMessage};
use super::mrkdwn;
use super::print_json;
use super::render::{emoji_for_shortcode, render_attachment, render_slack_blocks};
use super::state::{ChannelState, ImportState};
use crate::client::BuzzClient;
use crate::error::CliError;

/// Abort after this many consecutive message-submit failures — a wall of
/// failures means the relay is down or rejecting everything, not a handful
/// of individually bad messages.
const MAX_CONSECUTIVE_FAILURES: usize = 5;

/// Persist the state ledger at most every N successful writes.
///
/// The ledger is serialized whole on every save, so saving per message costs
/// O(n²) bytes over a large workspace. Batching is safe because every import
/// write is deterministic and idempotent: a crash loses at most the last N
/// ledger entries, and re-running rebuilds byte-identical events that the relay
/// reports as duplicates (which [`submit`] treats as success), repopulating the
/// ledger. Channel creation, archival, and end-of-channel state are still
/// flushed immediately.
const STATE_CHECKPOINT_WRITES: usize = 50;

#[derive(Default)]
struct Summary {
    channels_created: u64,
    messages_imported: u64,
    reactions_imported: u64,
    bindings_published: u64,
    private_members_added: u64,
    private_members_unmapped: u64,
    skipped: u64,
    warnings: Vec<String>,
}

impl Summary {
    fn warn(&mut self, msg: String) {
        eprintln!("warning: {msg}");
        self.warnings.push(msg);
    }
}

/// A single `buzz import slack` run: borrowed export/index inputs plus the
/// mutable state ledger and summary threaded through every write.
pub(super) struct Importer<'a> {
    client: &'a BuzzClient,
    export: &'a SlackExport,
    /// Slack user id → display name.
    names: &'a HashMap<String, String>,
    /// Slack workspace id — namespaces identity bindings and channel UUIDs.
    team_id: &'a str,
    state: ImportState,
    state_path: PathBuf,
    summary: Summary,
    skip_reactions: bool,
    /// Slack user id → Buzz public key. Besides publishing identity
    /// attestations, explicit mappings populate private conversation members.
    bindings: &'a HashMap<String, String>,
    /// Writes recorded since the last state flush (see
    /// [`STATE_CHECKPOINT_WRITES`]).
    unsaved: usize,
}

/// Owned and borrowed run settings grouped separately from the parsed export
/// indexes supplied to [`Importer::new`].
pub(super) struct ImporterOptions<'a> {
    pub(super) team_id: &'a str,
    pub(super) state: ImportState,
    pub(super) state_path: PathBuf,
    pub(super) skip_reactions: bool,
    pub(super) bindings: &'a HashMap<String, String>,
}

impl<'a> Importer<'a> {
    /// Assemble a run from parsed inputs and a freshly loaded state ledger.
    pub(super) fn new(
        client: &'a BuzzClient,
        export: &'a SlackExport,
        names: &'a HashMap<String, String>,
        options: ImporterOptions<'a>,
    ) -> Self {
        Self {
            client,
            export,
            names,
            team_id: options.team_id,
            state: options.state,
            state_path: options.state_path,
            summary: Summary::default(),
            skip_reactions: options.skip_reactions,
            bindings: options.bindings,
            unsaved: 0,
        }
    }

    /// Persist the running state ledger.
    fn save(&self) -> Result<(), CliError> {
        self.state.save(&self.state_path)
    }

    /// Persist immediately and clear the checkpoint counter.
    fn flush(&mut self) -> Result<(), CliError> {
        self.save()?;
        self.unsaved = 0;
        Ok(())
    }

    /// Record one completed write, persisting once a checkpoint's worth has
    /// accumulated (see [`STATE_CHECKPOINT_WRITES`]).
    fn checkpoint(&mut self) -> Result<(), CliError> {
        self.unsaved += 1;
        if self.unsaved >= STATE_CHECKPOINT_WRITES {
            self.flush()?;
        }
        Ok(())
    }

    /// Create a channel once and independently resume its topic metadata.
    async fn ensure_channel(
        &mut self,
        channel: &SlackChannel,
        needs_writes: bool,
    ) -> Result<Uuid, CliError> {
        if matches!(
            channel.kind,
            super::export::SlackConversationKind::DirectMessage
                | super::export::SlackConversationKind::GroupDirectMessage
        ) {
            return self.ensure_dm(channel).await;
        }

        let (uuid, metadata_done) = match self.state.channels.get(&channel.id) {
            Some(state) => (
                Uuid::parse_str(&state.uuid)
                    .map_err(|e| CliError::Other(format!("state file holds invalid UUID: {e}")))?,
                state.metadata_done,
            ),
            None => {
                let uuid = channel_uuid(self.team_id, &channel.id);
                let about = if channel.purpose.value.is_empty() {
                    None
                } else {
                    Some(channel.purpose.value.as_str())
                };
                let builder = buzz_sdk::build_create_channel(
                    uuid,
                    &channel.name,
                    Some(if channel.kind.is_private() {
                        buzz_sdk::Visibility::Private
                    } else {
                        buzz_sdk::Visibility::Open
                    }),
                    Some(channel.kind.buzz_channel_kind()),
                    about,
                    None,
                )
                .map_err(|e| CliError::Other(format!("build_create_channel failed: {e}")))?;
                submit(self.client, builder).await.map_err(|e| {
                    CliError::Other(format!("channel create failed for #{}: {e}", channel.name))
                })?;

                // Persist the UUID before the separate topic write. If that
                // write or the process fails, a re-run resumes metadata on the
                // same channel instead of creating a duplicate.
                self.state.channels.insert(
                    channel.id.clone(),
                    ChannelState {
                        uuid: uuid.to_string(),
                        metadata_done: false,
                        archived_done: false,
                        private_visibility_done: channel.kind.is_private(),
                        prepared_for_import: true,
                        members_added: std::collections::HashSet::new(),
                    },
                );
                self.flush()?;
                self.summary.channels_created += 1;
                (uuid, false)
            }
        };

        let (prepared_for_import, archived_done) = self
            .state
            .channels
            .get(&channel.id)
            .map(|state| (state.prepared_for_import, state.archived_done))
            .unwrap_or((true, false));
        if !prepared_for_import || (channel.is_archived && archived_done && needs_writes) {
            // The crosswalk says which channel to adopt, but not whether that
            // Buzz channel is currently archived. Attempt the transition on
            // every first adoption, even when Slack says the source is active.
            // Kind:9002 side effects are best-effort at the relay, so its
            // accepted response cannot prove the channel is active. Read the
            // relay-generated channel metadata back before any history write.
            let builder = buzz_sdk::build_unarchive(uuid)
                .map_err(|e| CliError::Other(format!("build_unarchive failed: {e}")))?;
            submit(self.client, builder).await.map_err(|error| {
                CliError::Other(format!(
                    "could not prepare conversation {} for history import: {error}",
                    channel.id
                ))
            })?;
            require_unarchived(self.client, uuid, &channel.id).await?;
            if let Some(state) = self.state.channels.get_mut(&channel.id) {
                state.prepared_for_import = true;
                state.archived_done = false;
            }
            self.flush()?;
        }

        // Older importer ledgers could contain a Slackdump private record from
        // `channels.json` even though the old create path always emitted
        // `visibility=open`. Narrow it before publishing any history.
        let private_visibility_done = self
            .state
            .channels
            .get(&channel.id)
            .is_some_and(|state| state.private_visibility_done);
        if channel.kind.is_private() && !private_visibility_done {
            let builder =
                buzz_sdk::build_update_channel(uuid, None, None, Some("private"), None)
                    .map_err(|e| CliError::Other(format!("build_update_channel failed: {e}")))?;
            submit(self.client, builder).await.map_err(|e| {
                CliError::Other(format!(
                    "refusing to import private conversation {} because Buzz visibility \
                     could not be narrowed: {e}",
                    channel.id
                ))
            })?;
            if let Some(state) = self.state.channels.get_mut(&channel.id) {
                state.private_visibility_done = true;
            }
            self.flush()?;
        }

        if !metadata_done {
            let topic_result = if channel.topic.value.is_empty() {
                Ok(())
            } else {
                match buzz_sdk::build_set_topic(uuid, &channel.topic.value) {
                    Ok(builder) => submit(self.client, builder).await.map(|_| ()),
                    Err(e) => Err(CliError::Other(format!("build_set_topic failed: {e}"))),
                }
            };
            match topic_result {
                Ok(()) => {
                    if let Some(state) = self.state.channels.get_mut(&channel.id) {
                        state.metadata_done = true;
                    }
                    self.flush()?;
                }
                Err(e) => self
                    .summary
                    .warn(format!("topic set failed for #{}: {e}", channel.name)),
            }
        }

        Ok(uuid)
    }

    /// Open a native Buzz DM using the exact mapped Slack participant set.
    ///
    /// The relay always includes the caller in a DM. Requiring the importer to
    /// map to one Slack participant prevents an operator who merely has access
    /// to an export from becoming an extra participant in every imported DM.
    /// Requiring all active human participants up front avoids Buzz's immutable
    /// DM participant-set rule splitting later history across two DMs.
    async fn ensure_dm(&mut self, channel: &SlackChannel) -> Result<Uuid, CliError> {
        if let Some(state) = self.state.channels.get(&channel.id) {
            let uuid = Uuid::parse_str(&state.uuid)
                .map_err(|e| CliError::Other(format!("state file holds invalid UUID: {e}")))?;
            ensure_dm_uuid_unclaimed(&self.state, channel, uuid)?;
            return Ok(uuid);
        }

        let importer_pubkey = self.client.keys().public_key().to_hex();
        let participant_pubkeys =
            mapped_dm_participants(self.export, channel, self.bindings, &importer_pubkey)?;
        let builder = build_import_dm_open(
            channel,
            self.team_id,
            &participant_pubkeys,
            &importer_pubkey,
        )?;
        let (_, response) = submit_with_response(self.client, builder)
            .await
            .map_err(|e| {
                CliError::Other(format!(
                    "DM open failed for Slack conversation {}: {e}",
                    channel.id
                ))
            })?;
        let payload = response_payload(&response).ok_or_else(|| {
            CliError::Other(format!(
                "DM open response for Slack conversation {} did not contain a channel id",
                channel.id
            ))
        })?;
        let channel_id = payload
            .get("channel_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                CliError::Other(format!(
                    "DM open response for Slack conversation {} did not contain a channel id",
                    channel.id
                ))
            })?;
        let uuid = Uuid::parse_str(channel_id)
            .map_err(|e| CliError::Other(format!("relay returned invalid DM UUID: {e}")))?;
        ensure_dm_uuid_unclaimed(&self.state, channel, uuid)?;
        let created = payload
            .get("created")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        self.state.channels.insert(
            channel.id.clone(),
            ChannelState {
                uuid: uuid.to_string(),
                metadata_done: true,
                archived_done: true,
                private_visibility_done: true,
                prepared_for_import: true,
                members_added: participant_pubkeys.into_iter().collect(),
            },
        );
        self.flush()?;
        if created {
            self.summary.channels_created += 1;
        }
        Ok(uuid)
    }

    /// Add every explicitly mapped Slack participant to a private Buzz
    /// conversation. Unmapped people are counted, never substituted with the
    /// importer or another participant, so private history cannot be widened
    /// accidentally.
    async fn ensure_private_members(
        &mut self,
        channel: &SlackChannel,
        channel_uuid: Uuid,
    ) -> Result<(), CliError> {
        if !channel.kind.is_private()
            || matches!(
                channel.kind,
                super::export::SlackConversationKind::DirectMessage
                    | super::export::SlackConversationKind::GroupDirectMessage
            )
        {
            return Ok(());
        }

        let importer_pubkey = self.client.keys().public_key().to_hex();
        let mut mapped_pubkeys = std::collections::BTreeSet::new();
        let mut unmapped = 0u64;
        for slack_id in &channel.members {
            if !self.export.is_mappable_member(slack_id) {
                continue;
            }
            match self.bindings.get(slack_id) {
                Some(pubkey) => {
                    mapped_pubkeys.insert(pubkey.clone());
                }
                None => unmapped += 1,
            }
        }
        self.summary.private_members_unmapped += unmapped;

        for pubkey in mapped_pubkeys {
            let already_added = self
                .state
                .channels
                .get(&channel.id)
                .is_some_and(|state| state.members_added.contains(&pubkey));
            if already_added {
                continue;
            }

            // The channel creator is already its owner. Record an explicit
            // mapping to that same key without publishing a redundant
            // add-member event.
            if pubkey != importer_pubkey {
                let builder = buzz_sdk::build_add_member(
                    channel_uuid,
                    &pubkey,
                    Some(buzz_sdk::MemberRole::Member),
                )
                .map_err(|e| CliError::Other(format!("build_add_member failed: {e}")))?;
                if let Err(e) = submit(self.client, builder).await {
                    self.summary.warn(format!(
                        "mapped member add failed for private conversation {}: {e}",
                        channel.id
                    ));
                    continue;
                }
            }
            if let Some(state) = self.state.channels.get_mut(&channel.id) {
                state.members_added.insert(pubkey);
            }
            self.summary.private_members_added += 1;
            self.checkpoint()?;
        }
        Ok(())
    }

    pub(super) async fn import_channel(&mut self, channel: &SlackChannel) -> Result<(), CliError> {
        let export = self.export;
        let names = self.names;

        let messages = export.channel_messages(channel)?;
        eprintln!(
            "importing {} {} ({} messages)",
            channel.kind.as_str(),
            channel.name,
            messages.len()
        );

        let needs_writes = self.channel_needs_writes(channel, &messages);
        let channel_uuid = self.ensure_channel(channel, needs_writes).await?;
        self.ensure_private_members(channel, channel_uuid).await?;

        // Messages, oldest first; thread roots always precede replies.
        //
        // `channel_messages` already dropped every non-importable message and
        // sorted the rest chronologically, so an importable thread root always
        // appears before — and is imported before — its replies. Any root that
        // is *not* in this set (an empty `bot_message` root, a system subtype)
        // can never be imported, so a reply pointing at it must not be deferred
        // forever; it is promoted to a top-level message instead.
        let importable_ts: std::collections::HashSet<&str> =
            messages.iter().map(|m| m.ts.as_str()).collect();
        let mut consecutive_failures = 0usize;
        let mut imported_in_channel = 0u64;
        for msg in &messages {
            let key = ImportState::message_key(&channel.id, &msg.ts);
            if self.state.messages.contains_key(&key) {
                // Already imported — but a prior run may have stopped between
                // the message and its reactions, so reactions still get their
                // (state-deduped) pass below.
                if !self.skip_reactions {
                    self.import_reactions(channel, msg, &key).await?;
                }
                continue;
            }

            // Slack threads are flat: thread_ts is the root, every reply is a
            // direct reply to it. Roots resolved through the state ledger.
            let thread_ref = match thread_root_key(channel, msg) {
                Some(root_key) => match self.state.messages.get(&root_key) {
                    Some(root_hex) => {
                        let root = EventId::from_hex(root_hex).map_err(|e| {
                            CliError::Other(format!("state file holds invalid event id: {e}"))
                        })?;
                        Some(buzz_sdk::ThreadRef {
                            root_event_id: root,
                            parent_event_id: root,
                        })
                    }
                    None => {
                        // Root importable but not yet in state → an earlier run
                        // was interrupted between root and reply; re-running
                        // resumes. Root absent from the importable set → Slack
                        // filtered it (empty bot root / system subtype) and it
                        // will never import, so keep the reply's real content as
                        // a top-level message rather than deferring it forever.
                        let root_ts = msg.thread_ts.as_deref().unwrap_or_default();
                        if importable_ts.contains(root_ts) {
                            self.summary.warn(format!(
                                "thread root {root_key} is not imported yet — deferring reply \
                                 {key}; re-run to resume"
                            ));
                            self.summary.skipped += 1;
                            continue;
                        }
                        self.summary.warn(format!(
                            "thread root {root_key} was filtered out of the import (empty or \
                             system message); importing reply {key} as a top-level message"
                        ));
                        None
                    }
                },
                None => None,
            };

            let builder = match build_imported_message(
                channel,
                channel_uuid,
                msg,
                names,
                self.team_id,
                thread_ref.as_ref(),
            ) {
                Ok(b) => b,
                Err(e) => {
                    self.summary.warn(format!("skipping {key}: {e}"));
                    self.summary.skipped += 1;
                    continue;
                }
            };
            match submit(self.client, builder).await {
                Ok(event_id) => {
                    consecutive_failures = 0;
                    self.state.messages.insert(key.clone(), event_id);
                    self.checkpoint()?;
                    self.summary.messages_imported += 1;
                    imported_in_channel += 1;
                    if imported_in_channel.is_multiple_of(50) {
                        eprintln!("  #{}: {imported_in_channel} imported", channel.name);
                    }
                }
                Err(e @ CliError::Auth(_)) => return Err(e),
                Err(e) => {
                    consecutive_failures += 1;
                    self.summary.warn(format!("message {key} failed: {e}"));
                    self.summary.skipped += 1;
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        self.flush()?;
                        return Err(CliError::Other(format!(
                            "{MAX_CONSECUTIVE_FAILURES} consecutive submit failures — aborting; \
                             re-run to resume from the state file"
                        )));
                    }
                    continue;
                }
            }

            if self.skip_reactions {
                continue;
            }
            self.import_reactions(channel, msg, &key).await?;
        }

        // Mirror Slack's archived flag once the channel's history is in.
        let archive_done = self
            .state
            .channels
            .get(&channel.id)
            .is_some_and(|state| state.archived_done);
        if channel.is_archived
            && !matches!(
                channel.kind,
                super::export::SlackConversationKind::DirectMessage
                    | super::export::SlackConversationKind::GroupDirectMessage
            )
            && !archive_done
        {
            let builder = buzz_sdk::build_archive(channel_uuid)
                .map_err(|e| CliError::Other(format!("build_archive failed: {e}")))?;
            match submit(self.client, builder).await {
                Ok(_) => {
                    if let Some(state) = self.state.channels.get_mut(&channel.id) {
                        state.archived_done = true;
                    }
                    self.flush()?;
                }
                Err(e) => self
                    .summary
                    .warn(format!("archive failed for #{}: {e}", channel.name)),
            }
        }
        self.flush()?;
        Ok(())
    }

    /// Whether an archived stream must be reopened before this run can finish
    /// its metadata, membership, message, or reaction writes.
    fn channel_needs_writes(&self, channel: &SlackChannel, messages: &[SlackMessage]) -> bool {
        let Some(state) = self.state.channels.get(&channel.id) else {
            return true;
        };
        if !state.prepared_for_import
            || !state.metadata_done
            || (channel.kind.is_private() && !state.private_visibility_done)
        {
            return true;
        }

        if channel.kind == super::export::SlackConversationKind::PrivateChannel
            && channel.members.iter().any(|slack_id| {
                self.export.is_mappable_member(slack_id)
                    && self
                        .bindings
                        .get(slack_id)
                        .is_some_and(|pubkey| !state.members_added.contains(pubkey))
            })
        {
            return true;
        }

        messages.iter().any(|message| {
            let message_key = ImportState::message_key(&channel.id, &message.ts);
            !self.state.messages.contains_key(&message_key)
                || (!self.skip_reactions
                    && message.reactions.iter().any(|reaction| {
                        let emoji = emoji_for_shortcode(&reaction.name);
                        !self
                            .state
                            .reactions
                            .contains(&format!("{message_key}:{emoji}"))
                    }))
        })
    }

    /// Import reactions for one message. Bot mode signs with a single key, so
    /// each distinct emoji becomes one bot-signed reaction (a key can react
    /// only once per target); the count of reactors is not preserved.
    async fn import_reactions(
        &mut self,
        channel: &SlackChannel,
        msg: &SlackMessage,
        message_key: &str,
    ) -> Result<(), CliError> {
        if msg.reactions.is_empty() {
            return Ok(());
        }
        let client = self.client;

        let Some(target_hex) = self.state.messages.get(message_key).cloned() else {
            return Ok(());
        };
        let target = EventId::from_hex(&target_hex)
            .map_err(|e| CliError::Other(format!("state file holds invalid event id: {e}")))?;
        // Slack exports don't record reaction times; anchor just after the message.
        let created_at = ts_seconds(&msg.ts)?.saturating_add(1);

        for reaction in &msg.reactions {
            let emoji = emoji_for_shortcode(&reaction.name);
            let dedupe = format!("{message_key}:{emoji}");
            if self.state.reactions.contains(&dedupe) {
                continue;
            }
            let builder = match buzz_sdk::build_reaction(target, &emoji) {
                Ok(b) => b,
                Err(e) => {
                    self.summary.warn(format!(
                        "reaction :{}: on {message_key}: {e}",
                        reaction.name
                    ));
                    continue;
                }
            };
            let builder = builder
                .custom_created_at(Timestamp::from(created_at))
                .tags(provenance_tags("slack", "slack", &msg.ts)?);
            match submit(client, builder).await {
                Ok(_) => {
                    self.state.reactions.insert(dedupe);
                    self.checkpoint()?;
                    self.summary.reactions_imported += 1;
                }
                Err(e) => self.summary.warn(format!(
                    "reaction :{}: on {message_key} in #{} failed: {e}",
                    reaction.name, channel.name
                )),
            }
        }
        Ok(())
    }

    /// Publish owner/admin-signed identity bindings (public keys only).
    pub(super) async fn publish_bindings(
        &mut self,
        bindings: &[(String, String)],
    ) -> Result<(), CliError> {
        for (slack_id, pubkey_hex) in bindings {
            match publish_binding(self.client, self.team_id, slack_id, pubkey_hex).await {
                Ok(_) => self.summary.bindings_published += 1,
                Err(e) => self
                    .summary
                    .warn(format!("identity binding for {slack_id} failed: {e}")),
            }
        }
        Ok(())
    }

    /// Flush the final state and print the run summary as JSON.
    pub(super) fn finish(&self) -> Result<(), CliError> {
        self.save()?;
        print_json(&serde_json::json!({
            "channels_created": self.summary.channels_created,
            "messages_imported": self.summary.messages_imported,
            "reactions_imported": self.summary.reactions_imported,
            "bindings_published": self.summary.bindings_published,
            "private_members_added": self.summary.private_members_added,
            "private_members_unmapped": self.summary.private_members_unmapped,
            "skipped": self.summary.skipped,
            "warnings": self.summary.warnings,
            "state_file": self.state_path.display().to_string(),
        }))
    }
}

/// Build one imported message while preserving the channel/thread tags from
/// the SDK builder and appending Slack provenance.
pub(super) fn build_imported_message(
    channel: &SlackChannel,
    channel_uuid: Uuid,
    msg: &SlackMessage,
    names: &HashMap<String, String>,
    team_id: &str,
    thread_ref: Option<&buzz_sdk::ThreadRef>,
) -> Result<EventBuilder, CliError> {
    let author = author_id(msg);
    let author_name = author_display(msg, names);
    // The `import_author` id is the workspace-scoped foreign id `<team>:<user>`,
    // so it composes to the same `slack:<team>:<user>` key the identity binding
    // uses — that is how the client joins an imported message to its attributed
    // Buzz profile. Without the team prefix the join would miss.
    let author_foreign = format!("{team_id}:{}", author.as_deref().unwrap_or("unknown"));

    let mut content_parts = Vec::new();
    if !msg.text.trim().is_empty() {
        content_parts.push(mrkdwn::convert(&msg.text, names));
    } else {
        let blocks = render_slack_blocks(&msg.blocks, names);
        if !blocks.is_empty() {
            content_parts.push(blocks);
        }
    }
    for attachment in &msg.attachments {
        let rendered = render_attachment(attachment, names);
        if !rendered.is_empty() {
            content_parts.push(rendered);
        }
    }
    for file in &msg.files {
        match file.link() {
            Some(link) => content_parts.push(format!("📎 [{}]({link})", file.label())),
            None => content_parts.push(format!("📎 {}", file.label())),
        }
    }
    let content = content_parts.join("\n\n");
    // The prefix keeps imported history readable in clients that do not
    // understand identity-binding events. Buzz Desktop removes the redundant
    // prefix when rendering the provenance-aware message.
    let escaped_author_name = escape_markdown_emphasis(&author_name);
    let content = format!("**{escaped_author_name}**: {}", content.trim());

    let broadcast = matches!(
        msg.subtype.as_deref(),
        Some("thread_broadcast" | "reply_broadcast")
    );
    let mut import_tags = provenance_tags(&author_foreign, &author_name, &msg.ts)?;
    import_tags.push(
        nostr::Tag::parse([
            "import_conversation",
            &format!("{team_id}:{}", channel.id),
            channel.kind.as_str(),
        ])
        .map_err(|e| CliError::Other(format!("invalid conversation provenance tag: {e}")))?,
    );
    buzz_sdk::build_message(channel_uuid, &content, thread_ref, &[], broadcast, &[])
        .map_err(|e| CliError::Other(format!("build_message failed: {e}")))
        .and_then(|builder| {
            Ok(builder
                .custom_created_at(Timestamp::from(ts_seconds(&msg.ts)?))
                .tags(import_tags))
        })
}

fn escape_markdown_emphasis(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '*' | '_' | '`' | '[' | ']' | '(' | ')' | '~'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Sign with `client` and submit, returning the locally computed event id.
///
/// A 2xx response with `accepted: false` whose message marks a duplicate is
/// success (idempotent re-run after state loss); any other rejection is an
/// error.
///
/// Bulk imports run head-first into the relay's per-pubkey minute quotas,
/// and the relay's `retry in 0s` hint makes the client's built-in retry
/// spin uselessly — so 429s are absorbed here with a real backoff. The
/// signed event is resubmitted verbatim; a re-send that lands twice is a
/// relay-side duplicate, which the acceptance check below treats as success.
pub(super) async fn submit(client: &BuzzClient, builder: EventBuilder) -> Result<String, CliError> {
    submit_with_response(client, builder)
        .await
        .map(|(event_id, _)| event_id)
}

async fn submit_with_response(
    client: &BuzzClient,
    builder: EventBuilder,
) -> Result<(String, String), CliError> {
    let event = client.sign_event(builder)?;
    let event_id = event.id.to_hex();
    let mut backoff_secs = 1u64;
    let resp = loop {
        match client.submit_event(event.clone()).await {
            Ok(resp) => break resp,
            Err(CliError::Relay { status: 429, .. }) if backoff_secs <= 64 => {
                eprintln!("  rate-limited — retrying in {backoff_secs}s");
                tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
                backoff_secs *= 2;
            }
            Err(e) => return Err(e),
        }
    };
    let parsed: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
    let accepted = parsed
        .get("accepted")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !accepted {
        let message = parsed
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !message.contains("duplicate") {
            return Err(CliError::Other(format!(
                "relay rejected event: {}",
                if message.is_empty() { &resp } else { &message }
            )));
        }
    }
    Ok((event_id, resp))
}

/// Deterministic Buzz channel UUID for a Slack channel, derived from the
/// workspace + Slack channel id. Making it a pure function of stable Slack ids
/// means a re-run after a crash between the channel-create relay write and the
/// state save reuses the same UUID (an idempotent NIP-33 replace) instead of
/// minting a second channel. The `team_id` prefix keeps channel ids from two
/// workspaces from colliding onto one UUID.
pub(super) fn channel_uuid(team_id: &str, channel_id: &str) -> Uuid {
    let name = format!("buzz:slack-import:{team_id}:{channel_id}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, name.as_bytes())
}

/// Build and submit an owner/admin-signed Slack identity binding.
pub(super) async fn publish_binding(
    client: &BuzzClient,
    team_id: &str,
    slack_id: &str,
    pubkey_hex: &str,
) -> Result<String, CliError> {
    let d_tag = buzz_sdk::slack_identity_binding_d_tag(team_id, slack_id);
    let builder = buzz_sdk::build_import_identity_binding(&d_tag, pubkey_hex)
        .map_err(|e| CliError::Other(format!("build_import_identity_binding failed: {e}")))?;
    submit(client, builder).await
}

/// Provenance tags carried by every imported event.
pub(super) fn provenance_tags(
    author_id: &str,
    author_name: &str,
    slack_ts: &str,
) -> Result<Vec<nostr::Tag>, CliError> {
    let mk = |parts: &[&str]| {
        nostr::Tag::parse(parts.iter().copied())
            .map_err(|e| CliError::Other(format!("invalid provenance tag: {e}")))
    };
    Ok(vec![
        mk(&["import", "slack"])?,
        mk(&["import_author", author_id, author_name])?,
        mk(&["import_ts", slack_ts])?,
    ])
}

/// The Slack-side author identifier of a message: user id, else bot id.
pub(super) fn author_id(msg: &SlackMessage) -> Option<String> {
    msg.user
        .clone()
        .filter(|u| !u.is_empty())
        .or_else(|| msg.bot_id.clone().filter(|b| !b.is_empty()))
}

/// Human-readable author name for prefixes and attribution tags.
pub(super) fn author_display(msg: &SlackMessage, names: &HashMap<String, String>) -> String {
    if let Some(ref user) = msg.user {
        if let Some(name) = names.get(user) {
            return name.clone();
        }
    }
    if let Some(ref username) = msg.username {
        if !username.is_empty() {
            return username.clone();
        }
    }
    author_id(msg).unwrap_or_else(|| "unknown".to_string())
}

/// `Some(state key of the thread root)` when `msg` is a reply (its
/// `thread_ts` differs from its own `ts`).
pub(super) fn thread_root_key(channel: &SlackChannel, msg: &SlackMessage) -> Option<String> {
    let root_ts = msg.thread_ts.as_deref()?;
    if root_ts == msg.ts {
        return None;
    }
    Some(ImportState::message_key(&channel.id, root_ts))
}
