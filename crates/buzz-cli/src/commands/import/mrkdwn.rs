//! Minimal Slack mrkdwn → markdown conversion.
//!
//! Code blocks (``` fenced) and inline code (single backtick) are preserved
//! verbatim. Outside code, Slack angle-bracket tokens are rewritten to
//! plain-text/markdown equivalents and HTML entities are unescaped.
//!
//! Mentions become plain `@Name` text on purpose — no `p` tags are emitted
//! anywhere in the importer, so backdated history cannot flood mention
//! feeds (see docs/slack-import.md).

use std::collections::HashMap;

/// Convert one Slack message body to markdown.
///
/// `user_names` maps Slack user IDs to display names for `<@U...>` tokens.
pub fn convert(text: &str, user_names: &HashMap<String, String>) -> String {
    map_outside_code_blocks(text, |segment| {
        map_outside_inline_code(segment, |plain| {
            let replaced = convert_tokens(plain, user_names);
            let unescaped = unescape_entities(&replaced);
            convert_bold(&unescaped)
        })
    })
}

/// Split on ``` fences; apply `f` to segments outside fences, keep fenced
/// segments (and the fences themselves) verbatim.
fn map_outside_code_blocks(text: &str, f: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, segment) in text.split("```").enumerate() {
        if i > 0 {
            out.push_str("```");
        }
        if i % 2 == 0 {
            out.push_str(&f(segment));
        } else {
            out.push_str(segment);
        }
    }
    out
}

/// Split on single backticks; apply `f` outside inline code spans.
fn map_outside_inline_code(text: &str, f: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, segment) in text.split('`').enumerate() {
        if i > 0 {
            out.push('`');
        }
        if i % 2 == 0 {
            out.push_str(&f(segment));
        } else {
            out.push_str(segment);
        }
    }
    out
}

/// Rewrite `<...>` tokens: user/channel mentions, specials, and links.
fn convert_tokens(text: &str, user_names: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('>') {
            Some(end) => {
                out.push_str(&convert_one_token(&after[..end], user_names));
                rest = &after[end + 1..];
            }
            None => {
                // Unclosed '<' — keep the rest verbatim.
                out.push('<');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn convert_one_token(inner: &str, user_names: &HashMap<String, String>) -> String {
    // `<@U123>` or `<@U123|fallback>`
    if let Some(body) = inner.strip_prefix('@') {
        let (id, fallback) = split_pipe(body);
        let name = user_names
            .get(id)
            .map(String::as_str)
            .or(fallback)
            .unwrap_or(id);
        return format!("@{name}");
    }
    // `<#C123|name>` or `<#C123>`
    if let Some(body) = inner.strip_prefix('#') {
        let (id, label) = split_pipe(body);
        return format!("#{}", label.unwrap_or(id));
    }
    // `<!here>`, `<!channel>`, `<!everyone>`, `<!subteam^ID|@handle>`
    if let Some(body) = inner.strip_prefix('!') {
        let (id, label) = split_pipe(body);
        return match id {
            "here" | "channel" | "everyone" => format!("@{id}"),
            _ => label
                .map(str::to_string)
                .unwrap_or_else(|| format!("@{id}")),
        };
    }
    // `<url|label>` or `<url>`
    let (url, label) = split_pipe(inner);
    match label {
        Some(label) if !label.is_empty() => format!("[{label}]({url})"),
        _ => url.to_string(),
    }
}

fn split_pipe(s: &str) -> (&str, Option<&str>) {
    match s.split_once('|') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    }
}

/// Unescape the three entities Slack always escapes in message text.
fn unescape_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Convert Slack `*bold*` to markdown `**bold**`, conservatively: the pair
/// must sit on one line, open must be followed by non-space, close must be
/// preceded by non-space. Anything else is left untouched.
fn convert_bold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&convert_bold_line(line));
    }
    out
}

fn convert_bold_line(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut doubled: Vec<bool> = vec![false; chars.len()];
    let mut open: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        if c != '*' {
            continue;
        }
        match open {
            None => {
                let can_open = chars.get(i + 1).is_some_and(|&n| n != ' ' && n != '*')
                    && (i == 0 || chars[i - 1] != '*');
                if can_open {
                    open = Some(i);
                }
            }
            Some(start) => {
                let can_close = i > 0 && chars[i - 1] != ' ' && chars[i - 1] != '*';
                if can_close {
                    doubled[start] = true;
                    doubled[i] = true;
                    open = None;
                }
            }
        }
    }
    let mut out = String::with_capacity(line.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        out.push(c);
        if doubled[i] {
            out.push('*');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("U123".to_string(), "alice".to_string());
        m
    }

    #[test]
    fn converts_user_mentions() {
        assert_eq!(convert("hi <@U123>!", &names()), "hi @alice!");
        // Unknown user falls back to the fallback label, then the raw id.
        assert_eq!(convert("<@U999|bob>", &names()), "@bob");
        assert_eq!(convert("<@U999>", &names()), "@U999");
    }

    #[test]
    fn converts_channels_and_specials() {
        assert_eq!(convert("see <#C1|general>", &names()), "see #general");
        assert_eq!(convert("<!here> heads up", &names()), "@here heads up");
        assert_eq!(convert("<!channel>", &names()), "@channel");
    }

    #[test]
    fn converts_links() {
        assert_eq!(
            convert("see <https://a.io|the docs>", &names()),
            "see [the docs](https://a.io)"
        );
        assert_eq!(convert("<https://a.io>", &names()), "https://a.io");
    }

    #[test]
    fn unescapes_entities() {
        assert_eq!(
            convert("a &lt; b &amp;&amp; c &gt; d", &names()),
            "a < b && c > d"
        );
    }

    #[test]
    fn converts_bold_conservatively() {
        assert_eq!(convert("*bold* text", &names()), "**bold** text");
        assert_eq!(convert("2 * 3 * 4", &names()), "2 * 3 * 4");
        assert_eq!(convert("a *b\nc* d", &names()), "a *b\nc* d");
    }

    #[test]
    fn preserves_code() {
        assert_eq!(
            convert("look ```<@U123> *x*``` done *y*", &names()),
            "look ```<@U123> *x*``` done **y**"
        );
        assert_eq!(
            convert("run `cmd <@U123>` now", &names()),
            "run `cmd <@U123>` now"
        );
    }

    #[test]
    fn keeps_unclosed_angle_verbatim() {
        assert_eq!(convert("a < b", &names()), "a < b");
    }
}
