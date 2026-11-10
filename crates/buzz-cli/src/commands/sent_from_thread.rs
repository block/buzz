//! `buzz:sent-from-thread` provenance tag, mirroring desktop's
//! `features/messages/lib/sentFromThread.ts` and the validation in
//! `desktop/src-tauri/src/events/message_tags.rs`.

use nostr::{Tag, TagKind};

const TAG_NAME: &str = "buzz:sent-from-thread";
const EXCERPT_MAX_CHARS: usize = 64;
const MIN_WORD_BREAK_UTF16: usize = 32;

/// JavaScript `\s` / `String.prototype.trim` whitespace.
fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn starts_with_at(chars: &[char], at: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(i, p)| chars.get(at + i) == Some(&p))
}

fn find_from(chars: &[char], from: usize, pat: &str) -> Option<usize> {
    (from..chars.len()).find(|&i| starts_with_at(chars, i, pat))
}

/// Match `||…||` (no `||` inside) at `at`; returns the end index.
fn spoiler_end(chars: &[char], at: usize) -> Option<usize> {
    if !starts_with_at(chars, at, "||") {
        return None;
    }
    find_from(chars, at + 2, "||").map(|close| close + 2)
}

/// Match `[label](target)` at `at`; returns `(end, label_end)` where the label
/// is `at + 1..label_end`. `min_label` is the minimum label length.
fn link_end(chars: &[char], at: usize, min_label: usize) -> Option<(usize, usize)> {
    if chars.get(at) != Some(&'[') {
        return None;
    }
    let close = (at + 1..chars.len()).find(|&i| chars[i] == ']')?;
    if close - (at + 1) < min_label || chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let paren = (close + 2..chars.len()).find(|&i| chars[i] == ')')?;
    Some((paren + 1, close))
}

/// Match `<?https?://\S+>?` at `at`; returns the end index.
fn url_end(chars: &[char], at: usize) -> Option<usize> {
    let start = if chars.get(at) == Some(&'<') {
        at + 1
    } else {
        at
    };
    let after_scheme = if starts_with_at(chars, start, "https://") {
        start + 8
    } else if starts_with_at(chars, start, "http://") {
        start + 7
    } else {
        return None;
    };
    let end = (after_scheme..chars.len())
        .find(|&i| is_js_space(chars[i]))
        .unwrap_or(chars.len());
    (end > after_scheme).then_some(end)
}

/// Left-to-right `String.prototype.replace` with a global pattern: `matcher`
/// returns the match end and replacement for a match starting at the index.
fn replace_all(
    chars: &[char],
    matcher: impl Fn(&[char], usize) -> Option<(usize, Vec<char>)>,
) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        match matcher(chars, i) {
            Some((end, replacement)) => {
                out.extend(replacement);
                i = end;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn strip_markdown(content: &str) -> String {
    let controls_removed: Vec<char> = content
        .chars()
        .map(|c| {
            let cp = c as u32;
            if cp <= 0x1f || (0x7f..=0x9f).contains(&cp) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let spoilers = replace_all(&controls_removed, |chars, at| {
        spoiler_end(chars, at).map(|end| (end, vec![' ']))
    });
    let images = replace_all(&spoilers, |chars, at| {
        if chars[at] != '!' {
            return None;
        }
        link_end(chars, at + 1, 0).map(|(end, _)| (end, vec![' ']))
    });
    let links = replace_all(&images, |chars, at| {
        link_end(chars, at, 1).map(|(end, label_end)| (end, chars[at + 1..label_end].to_vec()))
    });
    let urls = replace_all(&links, |chars, at| {
        url_end(chars, at).map(|end| (end, vec![' ']))
    });
    urls.into_iter()
        .map(|c| if "`*_~>#|".contains(c) { ' ' } else { c })
        .collect()
}

fn collapse_whitespace(text: &str) -> String {
    text.split(is_js_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Summarize a thread root for the tag excerpt exactly as desktop's
/// `summarizeThreadRoot` does. Returns `None` when nothing readable remains.
pub(crate) fn summarize_thread_root(content: &str) -> Option<String> {
    let normalized = collapse_whitespace(&strip_markdown(content));
    if normalized.is_empty() {
        return None;
    }
    let chars: Vec<char> = normalized.chars().collect();
    if chars.len() <= EXCERPT_MAX_CHARS {
        return Some(normalized);
    }
    let clipped: String = chars[..EXCERPT_MAX_CHARS - 1].iter().collect();
    let units: Vec<u16> = clipped.encode_utf16().collect();
    let excerpt = match units.iter().rposition(|&u| u == u16::from(b' ')) {
        Some(space) if space > MIN_WORD_BREAK_UTF16 => String::from_utf16_lossy(&units[..space]),
        _ => clipped,
    };
    Some(format!("{}…", excerpt.trim_end_matches(is_js_space)))
}

/// Build the `["buzz:sent-from-thread", <root-id>, <excerpt?>]` tag.
pub(crate) fn build_tag(root_event_id: &str, excerpt: Option<&str>) -> Tag {
    let mut parts = vec![root_event_id.to_string()];
    parts.extend(excerpt.map(str::to_string));
    Tag::custom(TagKind::custom(TAG_NAME), parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(content: &str) -> Option<String> {
        summarize_thread_root(content)
    }

    #[test]
    fn strips_markdown_like_desktop() {
        assert_eq!(s("  hello\n\tworld  ").as_deref(), Some("hello world"));
        assert_eq!(
            s("**bold** _it_ `code` ~~x~~ > q # h").as_deref(),
            Some("bold it code x q h")
        );
        assert_eq!(
            s("see [the doc](https://x.io/a) now").as_deref(),
            Some("see the doc now")
        );
        assert_eq!(s("a ![alt](https://x.io/i.png) b").as_deref(), Some("a b"));
        assert_eq!(s("a ||secret | x|| b").as_deref(), Some("a b"));
        assert_eq!(s("a ||unclosed b").as_deref(), Some("a unclosed b"));
        assert_eq!(
            s("go <https://x.io/a?b=1> and http://y.io z").as_deref(),
            Some("go and z")
        );
        assert_eq!(s("[](x) [a]b").as_deref(), Some("[](x) [a]b"));
        assert_eq!(s("a\u{0}b\u{7f}c\u{85}d").as_deref(), Some("a b c d"));
        assert_eq!(s("a\u{feff}b\u{a0}c").as_deref(), Some("a b c"));
        assert_eq!(s("***  ||x|| ![i](u)"), None);
        assert_eq!(s(""), None);
    }

    #[test]
    fn clips_at_the_desktop_boundaries() {
        let exact = "a".repeat(64);
        assert_eq!(s(&exact).as_deref(), Some(exact.as_str()));

        let unbroken = "a".repeat(65);
        assert_eq!(s(&unbroken), Some(format!("{}…", "a".repeat(63))));

        let words = format!("{} {}", "a".repeat(40), "b".repeat(40));
        assert_eq!(s(&words), Some(format!("{}…", "a".repeat(40))));

        let early_space = format!("{} {}", "a".repeat(32), "b".repeat(40));
        assert_eq!(
            s(&early_space),
            Some(format!("{} {}…", "a".repeat(32), "b".repeat(30)))
        );

        let late_space = format!("{} {}", "a".repeat(33), "b".repeat(40));
        assert_eq!(s(&late_space), Some(format!("{}…", "a".repeat(33))));
    }

    #[test]
    fn clips_by_code_point_and_breaks_by_utf16_unit() {
        let wide = "é".repeat(65);
        assert_eq!(s(&wide), Some(format!("{}…", "é".repeat(63))));
        let emoji = "😀".repeat(65);
        assert_eq!(s(&emoji), Some(format!("{}…", "😀".repeat(63))));
        let kept = "😀".repeat(64);
        assert_eq!(s(&kept).as_deref(), Some(kept.as_str()));

        // 16 emoji take 32 UTF-16 units, so the space sits at unit 32: kept whole.
        let at_32 = format!("{} {}", "😀".repeat(16), "b".repeat(60));
        assert_eq!(
            s(&at_32),
            Some(format!("{} {}…", "😀".repeat(16), "b".repeat(46)))
        );
        // 17 emoji put the space at unit 34 (> 32), though only 17 code points in: break there.
        let at_34 = format!("{} {}", "😀".repeat(17), "b".repeat(60));
        assert_eq!(s(&at_34), Some(format!("{}…", "😀".repeat(17))));
    }

    #[test]
    fn matches_desktop_summarize_thread_root_on_generated_vectors() {
        let vectors: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("sent_from_thread_vectors.json")).unwrap();
        assert!(vectors.len() > 600);
        for vector in vectors {
            let input = vector["input"].as_str().unwrap();
            assert_eq!(
                s(input).as_deref(),
                vector["expected"].as_str(),
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn tag_has_desktop_shape() {
        let id = "a".repeat(64);
        assert_eq!(build_tag(&id, None).as_slice(), [TAG_NAME, id.as_str()]);
        assert_eq!(
            build_tag(&id, Some("hi")).as_slice(),
            [TAG_NAME, id.as_str(), "hi"]
        );
    }
}
