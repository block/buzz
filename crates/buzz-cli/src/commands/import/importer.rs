//! The `buzz import slack` execution engine: the stateful [`Importer`] run loop
//! plus the pure helpers that shape one imported event (message building,
//! the provenance tag, emoji mapping, thread resolution) and the relay submit
//! calls. The command layer in the parent module parses inputs and constructs
//! an [`Importer`]; everything that actually writes to the relay lives here.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use buzz_core::kind::KIND_STREAM_MESSAGE;
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
use super::state::{ChannelState, ImportState, Journal, JournalRecord};
use crate::client::BuzzClient;
use crate::error::CliError;

/// Abort after this many consecutive message-submit failures — a wall of
/// failures means the relay is down or rejecting everything, not a handful
/// of individually bad messages.
const MAX_CONSECUTIVE_FAILURES: usize = 5;

/// Unconfirmed event ids looked up per relay query when settling the pending
/// intents a previous run left behind.
const PENDING_LOOKUP_BATCH: usize = 100;

#[derive(Default)]
struct Summary {
    channels_created: u64,
    messages_imported: u64,
    reactions_imported: u64,
    private_members_added: u64,
    private_members_unmapped: u64,
    /// Records that can never be imported (for example, over the size limit).
    skipped: u64,
    /// Writes the relay did not confirm; a re-run retries them.
    failed: u64,
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
    /// Slack workspace id — namespaces provenance references and channel UUIDs.
    team_id: &'a str,
    state: ImportState,
    state_path: PathBuf,
    journal: Journal,
    clock: ImportClock,
    summary: Summary,
    skip_reactions: bool,
    /// Slack user id → Buzz public key, for private-conversation membership.
    user_map: &'a HashMap<String, String>,
}

/// Owned and borrowed run settings grouped separately from the parsed export
/// indexes supplied to [`Importer::new`].
pub(super) struct ImporterOptions<'a> {
    pub(super) team_id: &'a str,
    pub(super) state: ImportState,
    pub(super) state_path: PathBuf,
    pub(super) skip_reactions: bool,
    pub(super) user_map: &'a HashMap<String, String>,
}

impl<'a> Importer<'a> {
    /// Assemble a run from parsed inputs and a freshly loaded ledger, opening
    /// the journal that records this run's writes.
    pub(super) fn new(
        client: &'a BuzzClient,
        export: &'a SlackExport,
        names: &'a HashMap<String, String>,
        options: ImporterOptions<'a>,
    ) -> Result<Self, CliError> {
        let journal = Journal::open(&options.state_path)?;
        Ok(Self {
            client,
            export,
            names,
            team_id: options.team_id,
            state: options.state,
            state_path: options.state_path,
            journal,
            clock: ImportClock::starting_after(unix_now()),
            summary: Summary::default(),
            skip_reactions: options.skip_reactions,
            user_map: options.user_map,
        })
    }

    /// Compact the ledger: rewrite the snapshot, then drop the journal entries
    /// it now covers.
    fn flush(&mut self) -> Result<(), CliError> {
        self.state.save(&self.state_path)?;
        self.journal.clear()
    }

    /// Journal one write durably and fold it into the in-memory ledger.
    fn record(&mut self, record: JournalRecord) -> Result<(), CliError> {
        self.journal.append(&record)?;
        self.state.apply(record);
        Ok(())
    }

    /// Settle the messages an earlier run signed but never saw confirmed.
    ///
    /// Each pending intent names the exact event id that was handed to the
    /// relay, so one lookup decides it: an event the relay stored is adopted
    /// into the ledger, and anything else is forgotten and signed afresh by
    /// this run. A failed lookup aborts the run instead of risking a message
    /// published twice.
    pub(super) async fn resolve_pending(&mut self) -> Result<(), CliError> {
        if self.state.pending.is_empty() {
            return Ok(());
        }
        let pending: Vec<(String, String)> = self
            .state
            .pending
            .iter()
            .map(|(key, id)| (key.clone(), id.clone()))
            .collect();
        let mut stored = HashSet::new();
        for batch in pending.chunks(PENDING_LOOKUP_BATCH) {
            let ids: Vec<&str> = batch.iter().map(|(_, id)| id.as_str()).collect();
            let filter = serde_json::json!({
                "ids": ids,
                "kinds": [KIND_STREAM_MESSAGE],
                "limit": ids.len(),
            });
            let response = self.client.query(&filter).await?;
            stored.extend(stored_event_ids(&response)?);
        }

        let mut adopted = 0u64;
        for (key, id) in pending {
            self.state.pending.remove(&key);
            if stored.contains(&id) {
                self.state.messages.insert(key, id);
                adopted += 1;
            }
        }
        eprintln!("settled unconfirmed messages from an earlier run: {adopted} already stored");
        self.flush()
    }

    /// Reserve the next message timestamp and wait for the wall clock to reach
    /// it, so no imported message is ever dated in the future.
    async fn next_created_at(&mut self) -> Timestamp {
        let (created_at, wait_secs) = self.clock.tick(unix_now());
        if wait_secs > 0 {
            tokio::time::sleep(Duration::from_secs(wait_secs)).await;
        }
        Timestamp::from(created_at)
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
                    false,
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
            let builder = buzz_sdk::build_update_channel(
                uuid,
                None,
                None,
                Some("private"),
                None,
                None,
                None,
                None,
            )
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

        let importer_pubkey = self.client.public_key().to_hex();
        let participant_pubkeys =
            mapped_dm_participants(self.export, channel, self.user_map, &importer_pubkey)?;
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

        let importer_pubkey = self.client.public_key().to_hex();
        let mut mapped_pubkeys = std::collections::BTreeSet::new();
        let mut unmapped = 0u64;
        for slack_id in &channel.members {
            if !self.export.is_mappable_member(slack_id) {
                continue;
            }
            match self.user_map.get(slack_id) {
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
            self.record(JournalRecord::Member {
                channel: channel.id.clone(),
                pubkey,
            })?;
            self.summary.private_members_added += 1;
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
                            self.summary.failed += 1;
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
            // Journal the signed id before the relay sees it: if the answer is
            // lost, the next run can tell whether this exact event was stored.
            let created_at = self.next_created_at().await;
            let event = self
                .client
                .sign_event(builder.custom_created_at(created_at))
                .await?;
            let event_id = event.id.to_hex();
            self.record(JournalRecord::Intent {
                key: key.clone(),
                id: event_id.clone(),
            })?;
            match submit_signed(self.client, event).await {
                Ok(_) => {
                    consecutive_failures = 0;
                    self.record(JournalRecord::Message {
                        key: key.clone(),
                        id: event_id,
                    })?;
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
                    self.summary.failed += 1;
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
                        .user_map
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

    /// Import reactions for one message. The importer signs with a single key,
    /// so each distinct emoji becomes one reaction (a key can react only once
    /// per target); the count of reactors is not preserved. The relay refuses
    /// a repeated reaction as a duplicate, which makes these writes idempotent
    /// without an intent.
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
            let source_ref = format!(
                "{}/{}/{}/{}",
                self.team_id, channel.id, msg.ts, reaction.name
            );
            let builder = builder.tags([imported_from_tag(&source_ref)?]);
            match submit(client, builder).await {
                Ok(_) => {
                    self.record(JournalRecord::Reaction { key: dedupe })?;
                    self.summary.reactions_imported += 1;
                }
                Err(e @ CliError::Auth(_)) => return Err(e),
                Err(e) => {
                    self.summary.warn(format!(
                        "reaction :{}: on {message_key} in #{} failed: {e}",
                        reaction.name, channel.name
                    ));
                    self.summary.failed += 1;
                }
            }
        }
        Ok(())
    }

    /// Compact the ledger and print the run summary as JSON.
    ///
    /// A run that left retryable writes behind exits non-zero, so a caller
    /// never mistakes a partial import for a finished one.
    pub(super) fn finish(mut self) -> Result<(), CliError> {
        self.flush()?;
        print_json(&serde_json::json!({
            "channels_created": self.summary.channels_created,
            "messages_imported": self.summary.messages_imported,
            "reactions_imported": self.summary.reactions_imported,
            "private_members_added": self.summary.private_members_added,
            "private_members_unmapped": self.summary.private_members_unmapped,
            "skipped": self.summary.skipped,
            "failed": self.summary.failed,
            "warnings": self.summary.warnings,
            "state_file": self.state_path.display().to_string(),
        }))?;
        if self.summary.failed > 0 {
            return Err(CliError::Other(format!(
                "{} write(s) were not confirmed by the relay — re-run to resume from {}",
                self.summary.failed,
                self.state_path.display()
            )));
        }
        Ok(())
    }
}

/// Build one imported message: the SDK's channel/thread tags, an attribution
/// header in the content, and the `imported_from` provenance tag. The caller
/// stamps `created_at` when it is ready to submit.
pub(super) fn build_imported_message(
    channel: &SlackChannel,
    channel_uuid: Uuid,
    msg: &SlackMessage,
    names: &HashMap<String, String>,
    team_id: &str,
    thread_ref: Option<&buzz_sdk::ThreadRef>,
) -> Result<EventBuilder, CliError> {
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
    let header = attribution_header(&author_display(msg, names), &msg.ts);
    let content = with_header(&header, content_parts.join("\n\n").trim());

    let broadcast = matches!(
        msg.subtype.as_deref(),
        Some("thread_broadcast" | "reply_broadcast")
    );
    let provenance = imported_from_tag(&format!("{team_id}/{}/{}", channel.id, msg.ts))?;
    buzz_sdk::build_message(channel_uuid, &content, thread_ref, &[], broadcast, &[], &[])
        .map(|builder| builder.tags([provenance]))
        .map_err(|e| CliError::Other(format!("build_message failed: {e}")))
}

/// `**Author** (2024-03-04 09:43 UTC):` — who wrote the message and when.
///
/// The importer signs every event and the relay only accepts a current
/// `created_at`, so the original author and time are kept where every client
/// can show them: in the content.
fn attribution_header(author_name: &str, slack_ts: &str) -> String {
    let name = escape_markdown_emphasis(author_name);
    let written_at = ts_seconds(slack_ts)
        .ok()
        .and_then(|secs| i64::try_from(secs).ok())
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string());
    match written_at {
        Some(written_at) => format!("**{name}** ({written_at}):"),
        None => format!("**{name}**:"),
    }
}

/// Put `header` in front of `body`: on the same line for ordinary text, on a
/// line of its own when the body opens with block-level markdown that must
/// start a line to render (a code fence, quote, list, heading, or table).
fn with_header(header: &str, body: &str) -> String {
    if body.is_empty() {
        return header.to_string();
    }
    let first_line = body.lines().next().unwrap_or_default();
    let opens_block = ["```", "~~~", ">", "|", "- ", "* ", "+ "]
        .iter()
        .any(|marker| first_line.starts_with(marker))
        || (first_line.starts_with('#') && first_line.trim_start_matches('#').starts_with(' '))
        || first_line
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .strip_prefix(['.', ')'])
            .is_some_and(|rest| rest.starts_with(' ') && !first_line.starts_with(['.', ')']));
    if opens_block {
        format!("{header}\n\n{body}")
    } else {
        format!("{header} {body}")
    }
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
pub(super) async fn submit(client: &BuzzClient, builder: EventBuilder) -> Result<String, CliError> {
    submit_with_response(client, builder)
        .await
        .map(|(event_id, _)| event_id)
}

async fn submit_with_response(
    client: &BuzzClient,
    builder: EventBuilder,
) -> Result<(String, String), CliError> {
    let event = client.sign_event(builder).await?;
    let event_id = event.id.to_hex();
    let response = submit_signed(client, event).await?;
    Ok((event_id, response))
}

/// Submit an already-signed event and return the relay's response body.
///
/// A 2xx response with `accepted: false` whose message marks a duplicate is
/// success: the relay already holds this event, or — for a reaction — the same
/// reaction from this key. Any other rejection is an error.
///
/// Bulk imports run head-first into the relay's per-pubkey minute quotas, so
/// 429s are absorbed here with a bounded backoff. The same signed event is
/// resubmitted each time; it cannot land twice.
async fn submit_signed(client: &BuzzClient, event: nostr::Event) -> Result<String, CliError> {
    let mut backoff_secs = 1u64;
    let resp = loop {
        match client.submit_event(event.clone()).await {
            Ok(resp) => break resp,
            Err(CliError::Relay { status: 429, .. }) if backoff_secs <= 64 => {
                eprintln!("  rate-limited — retrying in {backoff_secs}s");
                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
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
    Ok(resp)
}

/// The ids of the events in a relay query response.
fn stored_event_ids(response: &str) -> Result<HashSet<String>, CliError> {
    let events: Vec<serde_json::Value> = serde_json::from_str(response).map_err(|e| {
        CliError::Other(format!(
            "could not parse the relay's answer about unconfirmed messages: {e}"
        ))
    })?;
    Ok(events
        .iter()
        .filter_map(|event| event.get("id")?.as_str().map(str::to_string))
        .collect())
}

/// Hands out strictly increasing `created_at` seconds for imported messages.
///
/// The relay orders a channel by `created_at` in whole seconds and breaks ties
/// by event id, so two messages stamped in the same second can swap places.
/// Giving each message its own second keeps the Slack order.
pub(super) struct ImportClock {
    last: u64,
}

impl ImportClock {
    /// A clock whose first timestamp is later than `now`, so a run that
    /// starts within a second of the previous one cannot reuse its last stamp.
    pub(super) fn starting_after(now: u64) -> Self {
        Self { last: now }
    }

    /// The next timestamp, and how many seconds remain until the wall clock
    /// (`now`) reaches it.
    pub(super) fn tick(&mut self, now: u64) -> (u64, u64) {
        let created_at = now.max(self.last.saturating_add(1));
        self.last = created_at;
        (created_at, created_at - now)
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
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

/// The provenance tag carried by every imported event:
/// `["imported_from", "slack", <source_ref>]`, where `source_ref` is a
/// slash-delimited path of Slack's own ids (`<team>/<channel>/<ts>` for a
/// message). It marks the event as imported and names the record it came from;
/// it does not claim the signer wrote it.
pub(super) fn imported_from_tag(source_ref: &str) -> Result<nostr::Tag, CliError> {
    nostr::Tag::parse(["imported_from", "slack", source_ref])
        .map_err(|e| CliError::Other(format!("invalid provenance tag: {e}")))
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
