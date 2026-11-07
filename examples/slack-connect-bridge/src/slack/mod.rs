//! Minimal Slack Events/Web API client for the bridge.

mod api;
mod webhook;

pub(crate) use api::SlackClient;
pub(crate) use webhook::{
    run_webhook_server, SlackDelivery, SlackEvent, WebhookControl, WebhookServerState,
};

/// Convert the subset of Slack mrkdwn that would otherwise be unreadable in
/// Buzz. Unknown control tokens stay visible instead of being discarded.
pub(crate) fn slack_mrkdwn_to_markdown(input: &str) -> String {
    let decoded = input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    let mut output = String::with_capacity(decoded.len());
    let mut rest = decoded.as_str();

    while let Some(open) = rest.find('<') {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('>') else {
            output.push_str(&rest[open..]);
            return output;
        };
        let token = &after_open[..close];
        output.push_str(&convert_control_token(token));
        rest = &after_open[close + 1..];
    }
    output.push_str(rest);
    output
}

fn convert_control_token(token: &str) -> String {
    if let Some(user_id) = token.strip_prefix('@') {
        return format!("@{user_id}");
    }
    if let Some(channel) = token.strip_prefix('#') {
        let label = channel.split_once('|').map_or(channel, |(_, label)| label);
        return format!("#{label}");
    }
    if let Some(command) = token.strip_prefix('!') {
        let label = command
            .split_once('|')
            .map_or(command, |(_, label)| label.trim_start_matches('@'));
        return format!("@{label}");
    }
    if let Some((url, label)) = token.split_once('|') {
        if url.starts_with("http://") || url.starts_with("https://") {
            return format!("[{label}]({url})");
        }
        if let Some(address) = url.strip_prefix("mailto:") {
            return format!("[{label}](mailto:{address})");
        }
    }
    if token.starts_with("http://") || token.starts_with("https://") {
        return token.to_owned();
    }
    if let Some(address) = token.strip_prefix("mailto:") {
        return address.to_owned();
    }
    format!("<{token}>")
}

/// Make an untrusted display name safe in a Buzz Markdown bold span.
pub(crate) fn escape_markdown_label(input: &str) -> String {
    input
        .chars()
        .flat_map(|character| match character {
            '\\' | '*' | '_' | '[' | ']' | '`' => vec!['\\', character],
            '\r' | '\n' => vec![' '],
            other => vec![other],
        })
        .take(160)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_slack_links_channels_and_mentions() {
        assert_eq!(
            slack_mrkdwn_to_markdown(
                "See <https://example.com|the docs> in <#C12345678|project-x> with <@U12345678> &amp; <!here>"
            ),
            "See [the docs](https://example.com) in #project-x with @U12345678 & @here"
        );
    }

    #[test]
    fn malformed_control_token_stays_visible() {
        assert_eq!(
            slack_mrkdwn_to_markdown("before <not-closed"),
            "before <not-closed"
        );
        assert_eq!(
            slack_mrkdwn_to_markdown("before <unknown> after"),
            "before <unknown> after"
        );
    }

    #[test]
    fn escapes_untrusted_display_names() {
        assert_eq!(
            escape_markdown_label("*Mallory*\n`admin`"),
            "\\*Mallory\\* \\`admin\\`"
        );
    }
}
