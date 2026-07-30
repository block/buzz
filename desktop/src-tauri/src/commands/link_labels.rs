//! Rich link labels resolved through the locally installed `sq agent-tools`
//! CLI. Slack permalinks sit behind an auth wall, and private Google files
//! return "Sign in" pages, so plain HTML title fetching (link_preview.rs)
//! can't label them. When the user has `sq` on their machine we ask the
//! authenticated agent-tools extensions instead:
//!
//! - Slack channel message  → "author in #channel"
//! - Slack DM message       → "author with partner"
//! - Slack channel link     → "#channel"
//! - Google Docs/Drive file → the file's real title
//!
//! Everything is best-effort: a missing `sq` binary, CLI errors, and
//! timeouts all yield `Ok(None)` so the caller falls back to the generic
//! page-title path.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use url::Url;

const SQ_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, PartialEq)]
enum AgentLink {
    SlackMessage { channel: String, ts: String },
    SlackChannel { channel: String },
    GoogleFile,
}

#[tauri::command]
pub async fn fetch_agent_link_label(href: String) -> Result<Option<String>, String> {
    let url = Url::parse(href.trim()).map_err(|error| format!("invalid URL: {error}"))?;
    let Some(link) = classify_agent_link(&url) else {
        return Ok(None);
    };
    let Some(sq) = sq_binary() else {
        return Ok(None);
    };

    let label = match link {
        AgentLink::SlackMessage { channel, ts } => {
            resolve_slack_message_label(&sq, &channel, &ts).await
        }
        AgentLink::SlackChannel { channel } => resolve_slack_conversation_label(&sq, &channel)
            .await
            .map(|conversation| conversation.label),
        AgentLink::GoogleFile => resolve_google_file_label(&sq, url.as_str()).await,
    };
    Ok(label)
}

fn classify_agent_link(url: &Url) -> Option<AgentLink> {
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();

    if host == "slack.com" || host.ends_with(".slack.com") {
        return match segments.as_slice() {
            ["archives", channel] if looks_like_slack_conversation_id(channel) => {
                Some(AgentLink::SlackChannel {
                    channel: (*channel).to_string(),
                })
            }
            ["archives", channel, message] if looks_like_slack_conversation_id(channel) => {
                let ts = parse_slack_message_ts(message)?;
                Some(AgentLink::SlackMessage {
                    channel: (*channel).to_string(),
                    ts,
                })
            }
            _ => None,
        };
    }

    super::is_supported_google_link(url).then_some(AgentLink::GoogleFile)
}

fn looks_like_slack_conversation_id(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('C' | 'D' | 'G'))
        && value.len() >= 9
        && chars.all(|c| c.is_ascii_alphanumeric())
}

/// Slack permalinks encode the message timestamp as `p1738012345123456`;
/// the API wants `1738012345.123456`.
fn parse_slack_message_ts(segment: &str) -> Option<String> {
    let digits = segment.strip_prefix('p')?;
    if digits.len() <= 10 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (seconds, fraction) = digits.split_at(10);
    Some(format!("{seconds}.{fraction}"))
}

struct SlackConversation {
    label: String,
    is_dm: bool,
}

async fn resolve_slack_message_label(sq: &PathBuf, channel: &str, ts: &str) -> Option<String> {
    let author_args = [
        "agent-tools",
        "slack",
        "read-thread",
        "--channel",
        channel,
        "--thread-ts",
        ts,
        "--limit",
        "1",
    ];
    let author = run_sq_json(sq, &author_args);
    let conversation = resolve_slack_conversation_label(sq, channel);
    let (author, conversation) = tokio::join!(author, conversation);

    let author = author.as_ref().and_then(extract_message_author)?;
    let conversation = conversation?;
    let joiner = if conversation.is_dm { "with" } else { "in" };
    Some(format!("{author} {joiner} {}", conversation.label))
}

async fn resolve_slack_conversation_label(
    sq: &PathBuf,
    channel: &str,
) -> Option<SlackConversation> {
    if channel.starts_with('D') {
        // resolve-name can't see DM conversation IDs, but search-channels
        // matches them and returns the DM partner.
        let result = run_sq_json(
            sq,
            &[
                "agent-tools",
                "slack",
                "search-channels",
                "--query",
                channel,
                "--limit",
                "3",
            ],
        )
        .await?;
        let partner = extract_dm_partner(&result, channel)?;
        return Some(SlackConversation {
            label: partner,
            is_dm: true,
        });
    }

    let result = run_sq_json(
        sq,
        &[
            "agent-tools",
            "slack",
            "resolve-name",
            "--name",
            channel,
            "--entity-type",
            "channel",
        ],
    )
    .await?;
    let name = result.get("name")?.as_str()?.trim();
    if name.is_empty() || name == channel {
        return None;
    }
    Some(SlackConversation {
        label: format!("#{}", name.trim_start_matches('#')),
        is_dm: result.get("type").and_then(Value::as_str) == Some("im"),
    })
}

fn extract_message_author(result: &Value) -> Option<String> {
    let user = result.get("messages")?.as_array()?.first()?.get("user")?;
    let author = user
        .get("username")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            user.get("real_name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
        })?;
    Some(author.trim().to_string())
}

fn extract_dm_partner(result: &Value, channel: &str) -> Option<String> {
    let results = result.get("results")?.as_array()?;
    let dm = results
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(channel))?;
    let partner = dm.get("dm_user")?;
    let name = partner
        .get("username")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            partner
                .get("real_name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
        })?;
    Some(name.trim().to_string())
}

async fn resolve_google_file_label(sq: &PathBuf, href: &str) -> Option<String> {
    let result = run_sq_json(
        sq,
        &[
            "agent-tools",
            "google-drive",
            "get-file-metadata",
            "--file-id-or-url",
            href,
        ],
    )
    .await?;
    let name = result.get("file")?.get("name")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.chars().take(180).collect())
}

fn sq_binary() -> Option<PathBuf> {
    crate::managed_agents::resolve_command("sq")
}

async fn run_sq_json(sq: &PathBuf, args: &[&str]) -> Option<Value> {
    let mut command = tokio::process::Command::new(sq);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let output = tokio::time::timeout(SQ_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        classify_agent_link, extract_dm_partner, extract_message_author, parse_slack_message_ts,
        AgentLink,
    };
    use serde_json::json;
    use url::Url;

    fn classify(href: &str) -> Option<AgentLink> {
        classify_agent_link(&Url::parse(href).unwrap())
    }

    #[test]
    fn slack_message_permalinks_classify_with_api_ts() {
        assert_eq!(
            classify("https://block.slack.com/archives/C0AJ54K0KNY/p1785361516443929"),
            Some(AgentLink::SlackMessage {
                channel: "C0AJ54K0KNY".to_string(),
                ts: "1785361516.443929".to_string(),
            })
        );
        // Thread-reply permalinks carry a query string; the p-segment is
        // still the linked message.
        assert_eq!(
            classify(
                "https://block.slack.com/archives/D0BLPGME058/p1785367578361779?thread_ts=1785361516.443929&cid=D0BLPGME058"
            ),
            Some(AgentLink::SlackMessage {
                channel: "D0BLPGME058".to_string(),
                ts: "1785367578.361779".to_string(),
            })
        );
    }

    #[test]
    fn slack_channel_links_classify_without_ts() {
        assert_eq!(
            classify("https://block.slack.com/archives/C0AJ54K0KNY"),
            Some(AgentLink::SlackChannel {
                channel: "C0AJ54K0KNY".to_string(),
            })
        );
    }

    #[test]
    fn non_archive_and_non_slack_urls_do_not_classify_as_slack() {
        assert_eq!(classify("https://block.slack.com/team/U123456"), None);
        assert_eq!(
            classify("https://notslack.com/archives/C0AJ54K0KNY/p1785361516443929"),
            None
        );
        assert_eq!(
            classify("http://block.slack.com/archives/C0AJ54K0KNY"),
            None
        );
        // Malformed p-segments fall through entirely.
        assert_eq!(
            classify("https://block.slack.com/archives/C0AJ54K0KNY/p123"),
            None
        );
        assert_eq!(
            classify("https://block.slack.com/archives/general/p1785361516443929"),
            None
        );
    }

    #[test]
    fn google_file_links_classify() {
        assert_eq!(
            classify("https://docs.google.com/document/d/1WQchQDNfF6ZD/edit"),
            Some(AgentLink::GoogleFile)
        );
        assert_eq!(
            classify("https://drive.google.com/file/d/abc123/view"),
            Some(AgentLink::GoogleFile)
        );
        assert_eq!(classify("https://docs.google.com/"), None);
    }

    #[test]
    fn permalink_ts_requires_p_prefix_and_digits() {
        assert_eq!(
            parse_slack_message_ts("p1785361516443929").as_deref(),
            Some("1785361516.443929")
        );
        assert_eq!(parse_slack_message_ts("1785361516443929"), None);
        assert_eq!(parse_slack_message_ts("p1785361516"), None);
        assert_eq!(parse_slack_message_ts("p17853615164439x9"), None);
    }

    #[test]
    fn author_prefers_username_then_real_name() {
        let with_username = json!({
            "messages": [{ "user": { "username": "gpap", "real_name": "Gábor Pap" } }]
        });
        assert_eq!(
            extract_message_author(&with_username).as_deref(),
            Some("gpap")
        );

        let real_name_only = json!({
            "messages": [{ "user": { "username": "", "real_name": "Gábor Pap" } }]
        });
        assert_eq!(
            extract_message_author(&real_name_only).as_deref(),
            Some("Gábor Pap")
        );

        assert_eq!(extract_message_author(&json!({ "messages": [] })), None);
    }

    #[test]
    fn dm_partner_matches_conversation_id_exactly() {
        let result = json!({
            "results": [
                { "id": "D0OTHER", "dm_user": { "username": "wrong" } },
                { "id": "D0BLPGME058", "dm_user": { "username": "dreya", "real_name": "Dreya Griffin" } }
            ]
        });
        assert_eq!(
            extract_dm_partner(&result, "D0BLPGME058").as_deref(),
            Some("dreya")
        );
        assert_eq!(extract_dm_partner(&result, "D0MISSING"), None);
    }
}
