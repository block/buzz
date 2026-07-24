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
mod mrkdwn;
mod state;

use std::collections::HashMap;
use std::path::PathBuf;

use nostr::{EventBuilder, EventId, PublicKey, Timestamp};
use uuid::Uuid;

use crate::client::BuzzClient;
use crate::error::CliError;
use export::{ts_seconds, SlackChannel, SlackExport, SlackMessage};
use state::{ChannelState, ImportState};

/// Abort after this many consecutive message-submit failures — a wall of
/// failures means the relay is down or rejecting everything, not a handful
/// of individually bad messages.
const MAX_CONSECUTIVE_FAILURES: usize = 5;

/// Parameters for `buzz import slack`.
pub struct ImportSlackParams {
    /// Unzipped Slack export directory.
    pub export_dir: String,
    /// State file path override.
    pub state: Option<String>,
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

#[derive(Default)]
struct Summary {
    channels_created: u64,
    messages_imported: u64,
    reactions_imported: u64,
    bindings_published: u64,
    skipped: u64,
    warnings: Vec<String>,
}

impl Summary {
    fn warn(&mut self, msg: String) {
        eprintln!("warning: {msg}");
        self.warnings.push(msg);
    }
}

pub async fn cmd_import_slack(client: &BuzzClient, p: ImportSlackParams) -> Result<(), CliError> {
    let export_dir = PathBuf::from(&p.export_dir);
    let export = SlackExport::load(&export_dir)?;

    let state_path = p
        .state
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| export_dir.join("buzz-import-state.json"));
    let state = ImportState::load(&state_path)?;

    // Slack user id → display name, for mrkdwn mention rewriting and
    // author attribution tags.
    let names: HashMap<String, String> = export
        .users
        .iter()
        .map(|(id, u)| (id.clone(), u.best_name().to_string()))
        .collect();

    // Parse identity bindings (Slack id → Buzz pubkey), public keys only.
    let bindings = parse_identity_map(p.identity_map.as_deref())?;

    let selected = select_channels(&export, p.channels.as_deref())?;

    if p.dry_run {
        return dry_run_report(&export, &selected, &state, &bindings);
    }

    let mut importer = Importer {
        client,
        export: &export,
        names: &names,
        state,
        state_path,
        summary: Summary::default(),
        skip_reactions: p.skip_reactions,
    };

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
    slack_id: &str,
    pubkey: &str,
) -> Result<(), CliError> {
    let pubkey_hex = parse_pubkey(pubkey)?;
    let event_id = publish_binding(client, slack_id, &pubkey_hex).await?;
    print_json(&serde_json::json!({
        "event_id": event_id,
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
pub async fn cmd_import_claim(client: &BuzzClient, slack_id: &str) -> Result<(), CliError> {
    let d_tag = buzz_sdk::slack_identity_binding_d_tag(slack_id);
    let builder = buzz_sdk::build_import_identity_claim(&d_tag)
        .map_err(|e| CliError::Other(format!("build_import_identity_claim failed: {e}")))?;
    let event_id = submit(client, builder).await?;
    print_json(&serde_json::json!({
        "event_id": event_id,
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
    spec.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|entry| {
            let (slack_id, key) = entry.split_once('=').ok_or_else(|| {
                CliError::Usage(format!(
                    "--identity-map entry must be SLACKID=npub-or-hex (got {entry:?})"
                ))
            })?;
            Ok((slack_id.trim().to_string(), parse_pubkey(key.trim())?))
        })
        .collect()
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

/// Resolve the `--channels` filter against the export's channel list,
/// erroring if the filter selects nothing.
fn select_channels<'e>(
    export: &'e SlackExport,
    filter: Option<&str>,
) -> Result<Vec<&'e SlackChannel>, CliError> {
    let filter: Option<Vec<String>> = filter.map(|list| {
        list.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let selected: Vec<&SlackChannel> = export
        .channels
        .iter()
        .filter(|c| {
            filter
                .as_ref()
                .is_none_or(|f| f.iter().any(|name| name == &c.name))
        })
        .collect();
    if selected.is_empty() {
        return Err(CliError::Usage(
            "no channels selected — check --channels against channels.json".into(),
        ));
    }
    Ok(selected)
}

/// A single `buzz import slack` run: borrowed export/index inputs plus the
/// mutable state ledger and summary threaded through every write.
struct Importer<'a> {
    client: &'a BuzzClient,
    export: &'a SlackExport,
    /// Slack user id → display name.
    names: &'a HashMap<String, String>,
    state: ImportState,
    state_path: PathBuf,
    summary: Summary,
    skip_reactions: bool,
}

impl Importer<'_> {
    /// Persist the running state ledger.
    fn save(&self) -> Result<(), CliError> {
        self.state.save(&self.state_path)
    }

    async fn import_channel(&mut self, channel: &SlackChannel) -> Result<(), CliError> {
        let client = self.client;
        let export = self.export;
        let names = self.names;

        let messages = export.channel_messages(&channel.name)?;
        eprintln!("importing #{} ({} messages)", channel.name, messages.len());

        // Channel create + metadata (once).
        let channel_uuid = match self.state.channels.get(&channel.id) {
            Some(cs) => Uuid::parse_str(&cs.uuid)
                .map_err(|e| CliError::Other(format!("state file holds invalid UUID: {e}")))?,
            None => {
                let uuid = Uuid::new_v4();
                let about = if channel.purpose.value.is_empty() {
                    None
                } else {
                    Some(channel.purpose.value.as_str())
                };
                let builder = buzz_sdk::build_create_channel(
                    uuid,
                    &channel.name,
                    Some(buzz_sdk::Visibility::Open),
                    Some(buzz_sdk::ChannelKind::Stream),
                    about,
                    None,
                )
                .map_err(|e| CliError::Other(format!("build_create_channel failed: {e}")))?;
                submit(client, builder).await.map_err(|e| {
                    CliError::Other(format!("channel create failed for #{}: {e}", channel.name))
                })?;
                if !channel.topic.value.is_empty() {
                    let topic = buzz_sdk::build_set_topic(uuid, &channel.topic.value)
                        .map_err(|e| CliError::Other(format!("build_set_topic failed: {e}")))?;
                    if let Err(e) = submit(client, topic).await {
                        self.summary
                            .warn(format!("topic set failed for #{}: {e}", channel.name));
                    }
                }
                self.state.channels.insert(
                    channel.id.clone(),
                    ChannelState {
                        uuid: uuid.to_string(),
                        metadata_done: true,
                    },
                );
                self.save()?;
                self.summary.channels_created += 1;
                uuid
            }
        };

        // Messages, oldest first; thread roots always precede replies.
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

            let author = author_id(msg);
            let author_name = author_display(msg, names);

            let mut content = mrkdwn::convert(&msg.text, names);
            for file in &msg.files {
                match file.link() {
                    Some(link) => content.push_str(&format!("\n📎 [{}]({link})", file.label())),
                    None => content.push_str(&format!("\n📎 {}", file.label())),
                }
            }
            // Bot-signed history keeps the author's name in a content prefix so
            // it stays readable even before an identity binding is published.
            let content = format!("**{author_name}**: {}", content.trim());

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
                        self.summary.warn(format!(
                            "thread root {root_key} not imported — posting {key} as top-level"
                        ));
                        None
                    }
                },
                None => None,
            };

            let created_at = ts_seconds(&msg.ts)?;
            let builder = match buzz_sdk::build_message(
                channel_uuid,
                &content,
                thread_ref.as_ref(),
                &[],
                false,
                &[],
            ) {
                Ok(b) => b,
                Err(e) => {
                    self.summary.warn(format!("skipping {key}: {e}"));
                    self.summary.skipped += 1;
                    continue;
                }
            };
            let builder =
                builder
                    .custom_created_at(Timestamp::from(created_at))
                    .tags(provenance_tags(
                        author.as_deref().unwrap_or("unknown"),
                        &author_name,
                        &msg.ts,
                    )?);

            match submit(client, builder).await {
                Ok(event_id) => {
                    consecutive_failures = 0;
                    self.state.messages.insert(key.clone(), event_id);
                    self.save()?;
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
                        self.save()?;
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
        if channel.is_archived {
            let builder = buzz_sdk::build_archive(channel_uuid)
                .map_err(|e| CliError::Other(format!("build_archive failed: {e}")))?;
            if let Err(e) = submit(client, builder).await {
                self.summary
                    .warn(format!("archive failed for #{}: {e}", channel.name));
            }
        }
        self.save()?;
        Ok(())
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
                    self.save()?;
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
    async fn publish_bindings(&mut self, bindings: &[(String, String)]) -> Result<(), CliError> {
        for (slack_id, pubkey_hex) in bindings {
            match publish_binding(self.client, slack_id, pubkey_hex).await {
                Ok(_) => self.summary.bindings_published += 1,
                Err(e) => self
                    .summary
                    .warn(format!("identity binding for {slack_id} failed: {e}")),
            }
        }
        Ok(())
    }

    /// Flush the final state and print the run summary as JSON.
    fn finish(&self) -> Result<(), CliError> {
        self.save()?;
        print_json(&serde_json::json!({
            "channels_created": self.summary.channels_created,
            "messages_imported": self.summary.messages_imported,
            "reactions_imported": self.summary.reactions_imported,
            "bindings_published": self.summary.bindings_published,
            "skipped": self.summary.skipped,
            "warnings": self.summary.warnings,
            "state_file": self.state_path.display().to_string(),
        }))
    }
}

fn dry_run_report(
    export: &SlackExport,
    selected: &[&SlackChannel],
    st: &ImportState,
    bindings: &[(String, String)],
) -> Result<(), CliError> {
    let mut channels_to_create = 0u64;
    let mut messages = 0u64;
    let mut reactions = 0u64;
    for channel in selected {
        if !st.channels.contains_key(&channel.id) {
            channels_to_create += 1;
        }
        for msg in export.channel_messages(&channel.name)? {
            if st
                .messages
                .contains_key(&ImportState::message_key(&channel.id, &msg.ts))
            {
                continue;
            }
            messages += 1;
            // One bot reaction per distinct emoji (bot mode dedup).
            let mut emojis: std::collections::HashSet<String> = std::collections::HashSet::new();
            for r in &msg.reactions {
                emojis.insert(emoji_for_shortcode(&r.name));
            }
            reactions += emojis.len() as u64;
        }
    }
    print_json(&serde_json::json!({
        "dry_run": true,
        "channels_selected": selected.len(),
        "channels_to_create": channels_to_create,
        "messages_to_import": messages,
        "reactions_to_import": reactions,
        "bindings_to_publish": bindings.len(),
    }))
}

/// Serialize `value` to compact JSON on stdout.
fn print_json(value: &serde_json::Value) -> Result<(), CliError> {
    let rendered = serde_json::to_string(value)
        .map_err(|e| CliError::Other(format!("summary serialization failed: {e}")))?;
    println!("{rendered}");
    Ok(())
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
async fn submit(client: &BuzzClient, builder: EventBuilder) -> Result<String, CliError> {
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
    Ok(event_id)
}

/// Build and submit an owner/admin-signed Slack identity binding.
async fn publish_binding(
    client: &BuzzClient,
    slack_id: &str,
    pubkey_hex: &str,
) -> Result<String, CliError> {
    let d_tag = buzz_sdk::slack_identity_binding_d_tag(slack_id);
    let builder = buzz_sdk::build_import_identity_binding(&d_tag, pubkey_hex)
        .map_err(|e| CliError::Other(format!("build_import_identity_binding failed: {e}")))?;
    submit(client, builder).await
}

/// Provenance tags carried by every imported event.
fn provenance_tags(
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
fn author_id(msg: &SlackMessage) -> Option<String> {
    msg.user
        .clone()
        .filter(|u| !u.is_empty())
        .or_else(|| msg.bot_id.clone().filter(|b| !b.is_empty()))
}

/// Human-readable author name for prefixes and attribution tags.
fn author_display(msg: &SlackMessage, names: &HashMap<String, String>) -> String {
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

/// Map common Slack reaction shortcodes to Unicode; anything unknown keeps
/// the `:shortcode:` form (rendered when the custom emoji is registered).
/// Skin-tone suffixes (`::skin-tone-N`) are dropped.
fn emoji_for_shortcode(name: &str) -> String {
    let base = name.split("::").next().unwrap_or(name);
    let mapped = match base {
        "+1" | "thumbsup" => "👍",
        "-1" | "thumbsdown" => "👎",
        "heart" => "❤️",
        "joy" => "😂",
        "smile" => "😄",
        "grin" => "😁",
        "laughing" => "😆",
        "sweat_smile" => "😅",
        "sob" => "😭",
        "cry" => "😢",
        "tada" => "🎉",
        "eyes" => "👀",
        "fire" => "🔥",
        "rocket" => "🚀",
        "pray" => "🙏",
        "clap" => "👏",
        "wave" => "👋",
        "raised_hands" => "🙌",
        "ok_hand" => "👌",
        "muscle" => "💪",
        "100" => "💯",
        "thinking_face" => "🤔",
        "white_check_mark" => "✅",
        "heavy_check_mark" => "✔️",
        "x" => "❌",
        "heart_eyes" => "😍",
        "sunglasses" => "😎",
        "sparkles" => "✨",
        "star" => "⭐",
        "zap" => "⚡",
        "warning" => "⚠️",
        "question" => "❓",
        "exclamation" => "❗",
        "bulb" => "💡",
        "memo" => "📝",
        "bug" => "🐛",
        "wink" => "😉",
        "point_up" => "☝️",
        "point_down" => "👇",
        "seedling" => "🌱",
        "bee" | "honeybee" => "🐝",
        _ => return format!(":{base}:"),
    };
    mapped.to_string()
}

/// `Some(state key of the thread root)` when `msg` is a reply (its
/// `thread_ts` differs from its own `ts`).
fn thread_root_key(channel: &SlackChannel, msg: &SlackMessage) -> Option<String> {
    let root_ts = msg.thread_ts.as_deref()?;
    if root_ts == msg.ts {
        return None;
    }
    Some(ImportState::message_key(&channel.id, root_ts))
}

/// Dispatch for `buzz import`.
pub async fn dispatch(cmd: crate::ImportCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        crate::ImportCmd::Slack {
            export_dir,
            state,
            channels,
            dry_run,
            skip_reactions,
            identity_map,
        } => {
            cmd_import_slack(
                client,
                ImportSlackParams {
                    export_dir,
                    state,
                    channels,
                    dry_run,
                    skip_reactions,
                    identity_map,
                },
            )
            .await
        }
        crate::ImportCmd::Bind { slack_id, pubkey } => {
            cmd_import_bind(client, &slack_id, &pubkey).await
        }
        crate::ImportCmd::Claim { slack_id } => cmd_import_claim(client, &slack_id).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;

    fn msg(json: &str) -> SlackMessage {
        serde_json::from_str(json).expect("test message parses")
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
        assert!(parse_identity_map(None).expect("none ok").is_empty());
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
                export_dir: dir.display().to_string(),
                state: None,
                channels: None,
                dry_run: true,
                skip_reactions: false,
                identity_map: None,
            },
        )
        .await
        .expect("dry run succeeds offline");

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
