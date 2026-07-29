//! Slack attachment, Block Kit, and reaction rendering helpers.

use std::collections::HashMap;

use super::export::SlackAttachment;
use super::mrkdwn;

/// Render a classic Slack attachment to readable Markdown. Interactive
/// buttons/actions are intentionally omitted; their visible label and context
/// are retained through the attachment fallback or fields.
pub(super) fn render_attachment(
    attachment: &SlackAttachment,
    names: &HashMap<String, String>,
) -> String {
    let mut parts = Vec::new();
    push_unique(
        &mut parts,
        mrkdwn::convert(attachment.pretext.trim(), names),
    );

    if !attachment.author_name.trim().is_empty() {
        let author = if attachment.author_link.trim().is_empty() {
            attachment.author_name.trim().to_string()
        } else {
            format!(
                "[{}]({})",
                attachment.author_name.trim(),
                attachment.author_link.trim()
            )
        };
        push_unique(&mut parts, author);
    }

    if !attachment.title.trim().is_empty() {
        let title = if attachment.title_link.trim().is_empty() {
            format!("**{}**", attachment.title.trim())
        } else {
            format!(
                "**[{}]({})**",
                attachment.title.trim(),
                attachment.title_link.trim()
            )
        };
        push_unique(&mut parts, title);
    }
    push_unique(&mut parts, mrkdwn::convert(attachment.text.trim(), names));

    for field in &attachment.fields {
        let title = field.title.trim();
        let value = mrkdwn::convert(field.value.trim(), names);
        let rendered = match (title.is_empty(), value.is_empty()) {
            (false, false) => format!("**{title}:** {value}"),
            (false, true) => format!("**{title}**"),
            (true, false) => value,
            (true, true) => continue,
        };
        push_unique(&mut parts, rendered);
    }

    if parts.is_empty() {
        push_unique(
            &mut parts,
            mrkdwn::convert(attachment.fallback.trim(), names),
        );
    }
    if parts.is_empty() {
        push_unique(&mut parts, render_slack_blocks(&attachment.blocks, names));
    }

    let media_url = [
        attachment.image_url.trim(),
        attachment.original_url.trim(),
        attachment.from_url.trim(),
    ]
    .into_iter()
    .find(|value| !value.is_empty());
    if let Some(url) = media_url {
        let label = if attachment.title.trim().is_empty() {
            "Slack attachment"
        } else {
            attachment.title.trim()
        };
        let link = format!("[{label}]({url})");
        if !parts.iter().any(|part| part.contains(url)) {
            parts.push(link);
        }
    }

    parts.join("\n")
}

fn push_unique(parts: &mut Vec<String>, value: String) {
    let value = value.trim();
    if !value.is_empty() && !parts.iter().any(|existing| existing == value) {
        parts.push(value.to_string());
    }
}

/// Tolerant renderer for the non-interactive subset of Slack rich-text and
/// Block Kit structures. Unknown container blocks still recurse through their
/// `elements`, so newer Slack block wrappers do not silently erase their text.
pub(super) fn render_slack_blocks(
    blocks: &[serde_json::Value],
    names: &HashMap<String, String>,
) -> String {
    blocks
        .iter()
        .map(|block| render_slack_block(block, names))
        .filter(|rendered| !rendered.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_slack_block(value: &serde_json::Value, names: &HashMap<String, String>) -> String {
    let Some(object) = value.as_object() else {
        return String::new();
    };
    let block_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    let rendered = match block_type {
        "text" | "plain_text" | "mrkdwn" => object
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_default(),
        "link" => {
            let url = object
                .get("url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let label = object
                .get("text")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty())
                .unwrap_or(url);
            if url.is_empty() {
                label.to_string()
            } else {
                format!("[{label}]({url})")
            }
        }
        "user" => {
            let id = object
                .get("user_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            format!("@{}", names.get(id).map(String::as_str).unwrap_or(id))
        }
        "channel" => object
            .get("channel_id")
            .and_then(serde_json::Value::as_str)
            .map(|id| format!("#{id}"))
            .unwrap_or_default(),
        "emoji" => object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|name| format!(":{name}:"))
            .unwrap_or_default(),
        "broadcast" => object
            .get("range")
            .and_then(serde_json::Value::as_str)
            .map(|range| format!("@{range}"))
            .unwrap_or_default(),
        "rich_text_list" => {
            let ordered =
                object.get("style").and_then(serde_json::Value::as_str) == Some("ordered");
            child_elements(object)
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    let body = render_slack_block(child, names);
                    if ordered {
                        format!("{}. {body}", index + 1)
                    } else {
                        format!("- {body}")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "rich_text_quote" => child_elements(object)
            .iter()
            .map(|child| render_slack_block(child, names))
            .collect::<String>()
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "rich_text_preformatted" => {
            let body = child_elements(object)
                .iter()
                .map(|child| render_slack_block(child, names))
                .collect::<String>();
            format!("```\n{body}\n```")
        }
        "divider" => "---".to_string(),
        _ => {
            if let Some(text) = object.get("text") {
                text.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| render_slack_block(text, names))
            } else {
                let children = child_elements(object);
                let separator = if matches!(block_type, "rich_text" | "section") {
                    "\n"
                } else {
                    ""
                };
                children
                    .iter()
                    .map(|child| render_slack_block(child, names))
                    .collect::<Vec<_>>()
                    .join(separator)
            }
        }
    };

    apply_block_style(rendered, object.get("style"))
}

fn child_elements(object: &serde_json::Map<String, serde_json::Value>) -> &[serde_json::Value] {
    object
        .get("elements")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn apply_block_style(mut text: String, style: Option<&serde_json::Value>) -> String {
    let Some(style) = style.and_then(serde_json::Value::as_object) else {
        return text;
    };
    if style
        .get("code")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        text = format!("`{text}`");
    }
    if style
        .get("bold")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        text = format!("**{text}**");
    }
    if style
        .get("italic")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        text = format!("_{text}_");
    }
    if style
        .get("strike")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        text = format!("~~{text}~~");
    }
    text
}

/// Map common Slack reaction shortcodes to Unicode; anything unknown keeps
/// the `:shortcode:` form (rendered when the custom emoji is registered).
/// Skin-tone suffixes (`::skin-tone-N`) are dropped.
pub(super) fn emoji_for_shortcode(name: &str) -> String {
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
