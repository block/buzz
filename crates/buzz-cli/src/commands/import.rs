//! `buzz import` — migrate history from external workspaces.
//!
//! v1 supports Slack workspace exports; see `docs/slack-import.md` for the
//! full design (identity modes, security model, limitations).

mod export;
mod mrkdwn;
mod state;

use std::collections::HashMap;
use std::path::PathBuf;

use nostr::{EventBuilder, EventId, Keys, Kind, Tag, Timestamp};
use serde::Deserialize;
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
    /// Optional Slack-user-ID → private-key JSON file (mapping mode).
    pub mapping: Option<String>,
    /// State file path override.
    pub state: Option<String>,
    /// Optional comma-separated channel-name filter.
    pub channels: Option<String>,
    /// Report the plan without writing anything.
    pub dry_run: bool,
    /// Skip reaction import.
    pub skip_reactions: bool,
    /// Skip kind 0 profile publishing for mapped users.
    pub skip_profiles: bool,
}

/// One entry in the `--mapping` file.
#[derive(Deserialize)]
struct MappingEntry {
    private_key: String,
}

#[derive(Default)]
struct Summary {
    channels_created: u64,
    messages_imported: u64,
    reactions_imported: u64,
    profiles_published: u64,
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
    let mut st = ImportState::load(&state_path)?;

    // Slack user id → display name, for mrkdwn mention rewriting and
    // author attribution.
    let names: HashMap<String, String> = export
        .users
        .iter()
        .map(|(id, u)| (id.clone(), u.best_name().to_string()))
        .collect();

    // Mapping mode: sign each mapped user's history with their own key
    // locally; every event is submitted over the single CLI connection. The
    // relay accepts third-party-signed events carrying `import` provenance
    // tags when the submitter is a community owner/admin (the Schnorr
    // signature proves authorship), so no per-user connection or relay
    // membership is needed at import time.
    let mut user_keys: HashMap<String, Keys> = HashMap::new();
    if let Some(ref mapping_path) = p.mapping {
        let raw = std::fs::read_to_string(mapping_path)
            .map_err(|e| CliError::Usage(format!("cannot read --mapping {mapping_path}: {e}")))?;
        let entries: HashMap<String, MappingEntry> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Usage(format!("cannot parse --mapping {mapping_path}: {e}")))?;
        for (slack_id, entry) in entries {
            let keys = Keys::parse(&entry.private_key).map_err(|e| {
                CliError::Key(format!(
                    "invalid private key for {slack_id} in mapping: {e}"
                ))
            })?;
            user_keys.insert(slack_id, keys);
        }
    }

    let channel_filter: Option<Vec<String>> = p.channels.as_ref().map(|list| {
        list.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let selected: Vec<&SlackChannel> = export
        .channels
        .iter()
        .filter(|c| {
            channel_filter
                .as_ref()
                .is_none_or(|f| f.iter().any(|name| name == &c.name))
        })
        .collect();
    if selected.is_empty() {
        return Err(CliError::Usage(
            "no channels selected — check --channels against channels.json".into(),
        ));
    }

    if p.dry_run {
        return dry_run_report(&export, &selected, &st, &user_keys);
    }

    let mut summary = Summary::default();

    // Relay membership for mapped users (best-effort: lets them read once
    // they log in with their key; posting during import does not need it).
    for (slack_id, keys) in &user_keys {
        let pk = keys.public_key().to_hex();
        if st.relay_members.contains(&pk) {
            continue;
        }
        match add_relay_member(client, &pk).await {
            Ok(()) => {
                st.relay_members.insert(pk);
                st.save(&state_path)?;
            }
            Err(e) => summary.warn(format!(
                "relay add-member failed for {slack_id} ({pk}): {e}"
            )),
        }
    }

    // Profiles for mapped users — signed by the user's key, submitted over
    // the CLI connection (import tags make the third-party signature
    // acceptable to the relay).
    if !p.skip_profiles {
        for (slack_id, keys) in &user_keys {
            if st.profiles.contains(slack_id) {
                continue;
            }
            let Some(user) = export.users.get(slack_id) else {
                summary.warn(format!("mapping entry {slack_id} not found in users.json"));
                continue;
            };
            let name = user.best_name().to_string();
            let builder = buzz_sdk::build_profile(
                Some(&name),
                Some(if user.name.is_empty() {
                    &name
                } else {
                    &user.name
                }),
                user.profile.image_512.as_deref(),
                None,
                None,
            )
            .map_err(|e| CliError::Other(format!("build_profile failed: {e}")))?
            .tags(provenance_tags(slack_id, &name, "")?);
            match submit_as(client, keys, builder).await {
                Ok(_) => {
                    st.profiles.insert(slack_id.clone());
                    st.save(&state_path)?;
                    summary.profiles_published += 1;
                }
                Err(e) => summary.warn(format!("profile publish failed for {slack_id}: {e}")),
            }
        }
    }

    for channel in selected {
        import_channel(
            client,
            &export,
            channel,
            &names,
            &user_keys,
            &mut st,
            &state_path,
            &mut summary,
            p.skip_reactions,
        )
        .await?;
    }

    st.save(&state_path)?;
    let output = serde_json::json!({
        "channels_created": summary.channels_created,
        "messages_imported": summary.messages_imported,
        "reactions_imported": summary.reactions_imported,
        "profiles_published": summary.profiles_published,
        "skipped": summary.skipped,
        "warnings": summary.warnings,
        "state_file": state_path.display().to_string(),
    });
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|e| CliError::Other(format!("summary serialization failed: {e}")))?
    );
    Ok(())
}

fn dry_run_report(
    export: &SlackExport,
    selected: &[&SlackChannel],
    st: &ImportState,
    user_keys: &HashMap<String, Keys>,
) -> Result<(), CliError> {
    let mut channels_to_create = 0u64;
    let mut messages = 0u64;
    let mut reactions = 0u64;
    let mut unmapped_authors: std::collections::HashSet<String> = std::collections::HashSet::new();
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
            reactions += msg
                .reactions
                .iter()
                .map(|r| r.users.len() as u64)
                .sum::<u64>();
            if let Some(author) = author_id(&msg) {
                if !user_keys.contains_key(&author) {
                    unmapped_authors.insert(author);
                }
            }
        }
    }
    let mut unmapped: Vec<String> = unmapped_authors.into_iter().collect();
    unmapped.sort();
    let output = serde_json::json!({
        "dry_run": true,
        "channels_selected": selected.len(),
        "channels_to_create": channels_to_create,
        "messages_to_import": messages,
        "reactions_to_import": reactions,
        "mapped_users": user_keys.len(),
        "unmapped_authors": unmapped,
    });
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|e| CliError::Other(format!("summary serialization failed: {e}")))?
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn import_channel(
    client: &BuzzClient,
    export: &SlackExport,
    channel: &SlackChannel,
    names: &HashMap<String, String>,
    user_keys: &HashMap<String, Keys>,
    st: &mut ImportState,
    state_path: &std::path::Path,
    summary: &mut Summary,
    skip_reactions: bool,
) -> Result<(), CliError> {
    let messages = export.channel_messages(&channel.name)?;
    eprintln!("importing #{} ({} messages)", channel.name, messages.len());

    // Channel create + metadata (once).
    let channel_uuid = match st.channels.get(&channel.id) {
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
                    summary.warn(format!("topic set failed for #{}: {e}", channel.name));
                }
            }
            st.channels.insert(
                channel.id.clone(),
                ChannelState {
                    uuid: uuid.to_string(),
                    metadata_done: true,
                },
            );
            st.save(state_path)?;
            summary.channels_created += 1;
            uuid
        }
    };

    // Channel membership for mapped users who speak in this channel.
    for msg in &messages {
        let Some(author) = author_id(msg) else {
            continue;
        };
        let Some(keys) = user_keys.get(&author) else {
            continue;
        };
        let pk = keys.public_key().to_hex();
        let member_key = format!("{}:{pk}", channel.id);
        if st.channel_members.contains(&member_key) {
            continue;
        }
        let builder = buzz_sdk::build_add_member(channel_uuid, &pk, None)
            .map_err(|e| CliError::Other(format!("build_add_member failed: {e}")))?;
        match submit(client, builder).await {
            Ok(_) => {
                st.channel_members.insert(member_key);
                st.save(state_path)?;
            }
            Err(e) => summary.warn(format!(
                "channel add-member failed for {author} in #{}: {e}",
                channel.name
            )),
        }
    }

    // Messages, oldest first; thread roots always precede replies.
    let mut consecutive_failures = 0usize;
    let mut imported_in_channel = 0u64;
    for msg in &messages {
        let key = ImportState::message_key(&channel.id, &msg.ts);
        if st.messages.contains_key(&key) {
            // Already imported — but a prior run may have stopped between
            // the message and its reactions, so reactions still get their
            // (state-deduped) pass below.
            if !skip_reactions {
                import_reactions(
                    client, channel, msg, &key, names, user_keys, st, state_path, summary,
                )
                .await?;
            }
            continue;
        }

        let author = author_id(msg);
        let author_name = author_display(msg, names);
        let signing_keys = author.as_ref().and_then(|a| user_keys.get(a));
        let bot_signed = signing_keys.is_none();

        let mut content = mrkdwn::convert(&msg.text, names);
        for file in &msg.files {
            match file.link() {
                Some(link) => {
                    content.push_str(&format!("\n📎 [{}]({link})", file.label()));
                }
                None => content.push_str(&format!("\n📎 {}", file.label())),
            }
        }
        let content = content.trim().to_string();
        let content = if bot_signed {
            format!("**{author_name}**: {content}")
        } else {
            content
        };

        // Slack threads are flat: thread_ts is the root, every reply is a
        // direct reply to it. Roots resolved through the state ledger.
        let thread_ref = match thread_root_key(channel, msg) {
            Some(root_key) => match st.messages.get(&root_key) {
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
                    summary.warn(format!(
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
                summary.warn(format!("skipping {key}: {e}"));
                summary.skipped += 1;
                continue;
            }
        };
        let builder = builder
            .custom_created_at(Timestamp::from(created_at))
            .tags(provenance_tags(
                author.as_deref().unwrap_or("unknown"),
                &author_name,
                &msg.ts,
            )?);

        let submitted = match signing_keys {
            Some(keys) => submit_as(client, keys, builder).await,
            None => submit(client, builder).await,
        };
        match submitted {
            Ok(event_id) => {
                consecutive_failures = 0;
                st.messages.insert(key.clone(), event_id);
                st.save(state_path)?;
                summary.messages_imported += 1;
                imported_in_channel += 1;
                if imported_in_channel.is_multiple_of(50) {
                    eprintln!("  #{}: {imported_in_channel} imported", channel.name);
                }
            }
            Err(e @ CliError::Auth(_)) => return Err(e),
            Err(e) => {
                consecutive_failures += 1;
                summary.warn(format!("message {key} failed: {e}"));
                summary.skipped += 1;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    st.save(state_path)?;
                    return Err(CliError::Other(format!(
                        "{MAX_CONSECUTIVE_FAILURES} consecutive submit failures — aborting; \
                         re-run to resume from the state file"
                    )));
                }
                continue;
            }
        }

        if skip_reactions {
            continue;
        }
        import_reactions(
            client, channel, msg, &key, names, user_keys, st, state_path, summary,
        )
        .await?;
    }

    // Mirror Slack's archived flag once the channel's history is in.
    if channel.is_archived {
        let builder = buzz_sdk::build_archive(channel_uuid)
            .map_err(|e| CliError::Other(format!("build_archive failed: {e}")))?;
        if let Err(e) = submit(client, builder).await {
            summary.warn(format!("archive failed for #{}: {e}", channel.name));
        }
    }
    st.save(state_path)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn import_reactions(
    client: &BuzzClient,
    channel: &SlackChannel,
    msg: &SlackMessage,
    message_key: &str,
    names: &HashMap<String, String>,
    user_keys: &HashMap<String, Keys>,
    st: &mut ImportState,
    state_path: &std::path::Path,
    summary: &mut Summary,
) -> Result<(), CliError> {
    if msg.reactions.is_empty() {
        return Ok(());
    }
    let Some(target_hex) = st.messages.get(message_key).cloned() else {
        return Ok(());
    };
    let target = EventId::from_hex(&target_hex)
        .map_err(|e| CliError::Other(format!("state file holds invalid event id: {e}")))?;
    // Slack exports don't record reaction times; anchor just after the message.
    let created_at = ts_seconds(&msg.ts)?.saturating_add(1);

    for reaction in &msg.reactions {
        let emoji = emoji_for_shortcode(&reaction.name);
        let mut bot_reacted = false;
        for user in &reaction.users {
            let signing_keys = match user_keys.get(user) {
                Some(keys) => Some(keys),
                None => {
                    // All unmapped reactors collapse into one bot-signed
                    // reaction per emoji — one key can't react twice.
                    if bot_reacted {
                        continue;
                    }
                    bot_reacted = true;
                    None
                }
            };
            let signer_pk = signing_keys
                .map(|k| k.public_key().to_hex())
                .unwrap_or_else(|| client.keys().public_key().to_hex());
            let dedupe = format!("{message_key}:{emoji}:{signer_pk}");
            if st.reactions.contains(&dedupe) {
                continue;
            }
            let builder = match buzz_sdk::build_reaction(target, &emoji) {
                Ok(b) => b,
                Err(e) => {
                    summary.warn(format!(
                        "reaction :{}: on {message_key}: {e}",
                        reaction.name
                    ));
                    continue;
                }
            };
            let reactor_name = names.get(user).map(String::as_str).unwrap_or(user.as_str());
            let builder = builder
                .custom_created_at(Timestamp::from(created_at))
                .tags(provenance_tags(user, reactor_name, &msg.ts)?);
            let submitted = match signing_keys {
                Some(keys) => submit_as(client, keys, builder).await,
                None => submit(client, builder).await,
            };
            match submitted {
                Ok(_) => {
                    st.reactions.insert(dedupe);
                    st.save(state_path)?;
                    summary.reactions_imported += 1;
                }
                Err(e) => summary.warn(format!(
                    "reaction :{}: on {message_key} in #{} failed: {e}",
                    reaction.name, channel.name
                )),
            }
        }
    }
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
/// relay-side duplicate, which the acceptance check below treats as
/// success.
async fn submit(client: &BuzzClient, builder: EventBuilder) -> Result<String, CliError> {
    let event = client.sign_event(builder)?;
    submit_signed(client, event).await
}

/// Sign with a mapped user's key and submit over the CLI connection.
///
/// The relay accepts the author/submitter mismatch because imported events
/// carry `import` provenance tags and the CLI identity is a community
/// owner/admin — the event's own Schnorr signature proves authorship.
async fn submit_as(
    client: &BuzzClient,
    keys: &Keys,
    builder: EventBuilder,
) -> Result<String, CliError> {
    let event = builder
        .sign_with_keys(keys)
        .map_err(|e| CliError::Other(format!("signing failed: {e}")))?;
    submit_signed(client, event).await
}

async fn submit_signed(client: &BuzzClient, event: nostr::Event) -> Result<String, CliError> {
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

/// NIP-43 relay-admin add-member (kind 9030), signed by the CLI identity.
/// Requires community owner/admin; freshness-gated ±120s by the relay, so
/// the default (current) timestamp is correct here.
async fn add_relay_member(client: &BuzzClient, pubkey_hex: &str) -> Result<(), CliError> {
    let tag = Tag::parse(["p", pubkey_hex])
        .map_err(|e| CliError::Other(format!("invalid p tag: {e}")))?;
    let builder = EventBuilder::new(Kind::Custom(9030), "").tags([tag]);
    submit(client, builder).await.map(|_| ())
}

/// Provenance tags carried by every imported event.
fn provenance_tags(
    author_id: &str,
    author_name: &str,
    slack_ts: &str,
) -> Result<Vec<Tag>, CliError> {
    let mk = |parts: &[&str]| {
        Tag::parse(parts.iter().copied())
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
            mapping,
            state,
            channels,
            dry_run,
            skip_reactions,
            skip_profiles,
        } => {
            cmd_import_slack(
                client,
                ImportSlackParams {
                    export_dir,
                    mapping,
                    state,
                    channels,
                    dry_run,
                    skip_reactions,
                    skip_profiles,
                },
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                mapping: None,
                state: None,
                channels: None,
                dry_run: true,
                skip_reactions: false,
                skip_profiles: false,
            },
        )
        .await
        .expect("dry run succeeds offline");

        // Dry run writes no state file.
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
