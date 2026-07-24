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
mod mrkdwn;
mod state;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use nostr::PublicKey;

use crate::client::BuzzClient;
use crate::error::CliError;
use export::{SlackChannel, SlackExport, SlackMessage};
use importer::{emoji_for_shortcode, publish_binding, submit, Importer};
use state::ImportState;

/// Parameters for `buzz import slack`.
pub struct ImportSlackParams {
    /// Unzipped Slack export directory.
    pub export_dir: String,
    /// Slack workspace id (team id) — namespaces identity bindings and channel
    /// UUIDs so ids can't collide across workspaces.
    pub team_id: String,
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

pub async fn cmd_import_slack(client: &BuzzClient, p: ImportSlackParams) -> Result<(), CliError> {
    let team_id = validate_team_id(&p.team_id)?.to_string();
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
        return dry_run_report(&export, &selected, &state, &bindings, p.skip_reactions);
    }

    let mut importer = Importer::new(
        client,
        &export,
        &names,
        &team_id,
        state,
        state_path,
        p.skip_reactions,
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

/// Resolve the `--channels` filter against the export's channel list,
/// erroring if the filter selects nothing.
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
        let available: HashSet<&str> = export.channels.iter().map(|c| c.name.as_str()).collect();
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
        .filter(|c| filter.as_ref().is_none_or(|f| f.contains(&c.name)))
        .collect();
    if selected.is_empty() {
        return Err(CliError::Usage(
            "no channels selected — check --channels against channels.json".into(),
        ));
    }
    Ok(selected)
}

fn dry_run_report(
    export: &SlackExport,
    selected: &[&SlackChannel],
    st: &ImportState,
    bindings: &[(String, String)],
    skip_reactions: bool,
) -> Result<(), CliError> {
    let mut channels_to_create = 0u64;
    let mut messages = 0u64;
    let mut reactions = 0u64;
    for channel in selected {
        if !st.channels.contains_key(&channel.id) {
            channels_to_create += 1;
        }
        for msg in export.channel_messages(&channel.name)? {
            let message_key = ImportState::message_key(&channel.id, &msg.ts);
            if !st.messages.contains_key(&message_key) {
                messages += 1;
            }
            if !skip_reactions {
                reactions += pending_reaction_count(st, &message_key, &msg) as u64;
            }
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
            export_dir,
            team_id,
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
                    team_id,
                    state,
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
        author_display, author_id, build_imported_message, channel_uuid, provenance_tags,
        thread_root_key,
    };
    use super::*;
    use nostr::{EventId, Keys};
    use uuid::Uuid;

    fn msg(json: &str) -> SlackMessage {
        serde_json::from_str(json).expect("test message parses")
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

        let event = build_imported_message(channel_id, &message, &names, "T1", Some(&thread_ref))
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
                team_id: "T1".into(),
                state: None,
                channels: None,
                dry_run: true,
                skip_reactions: false,
                identity_map: None,
            },
        )
        .await
        .expect("dry run succeeds offline");

        let export = SlackExport::load(&dir).expect("export");
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
