//! `@name` and NIP-27 `nostr:npub1…` mention resolution helpers for Buzz chat messages.
//!
//! These helpers are **pure** — no network calls, no async. Callers query
//! channel membership (kind 39002) and profile (kind 0) events themselves,
//! then hand the profile JSON to [`match_names_to_profiles`].
//!
//! ## Pipeline
//!
//! ```text
//! body text ──► extract_at_names ──► names: Vec<String>
//!                                       │
//! members + profiles (queried by caller) │
//!                                       ▼
//!                            match_names_to_profiles ──► pubkeys
//!                                                          │
//! body text ──► strip_code_regions ──► extract_nostr_uris ─┤
//!                                                          ▼
//!                            explicit mentions ──► normalize ──► merge_mentions ──► p-tags
//! ```
//!
//! When the set of known member names is available upfront,
//! [`extract_at_mentions_with_known`] replaces the first step to correctly
//! handle multi-word display names.
//!
//! [`extract_nostr_uris`] handles NIP-27 inline `nostr:npub1…` references,
//! skipping those inside code blocks/spans via [`strip_code_regions`].
//!
//! See [`crate::mentions::MENTION_CAP`] for the hard upper bound on tags.

use std::collections::HashSet;

use nostr::{FromBech32, PublicKey};

/// Maximum number of mention p-tags allowed on a single message.
///
/// Matches the cap enforced by Buzz message builders and the legacy MCP
/// inline implementation.
pub const MENTION_CAP: usize = 50;

/// A channel-member profile, as needed for name matching.
///
/// `pubkey` is the lowercase hex public key. `content_json` is the raw
/// kind 0 event content (a JSON object). Borrowing the content avoids
/// cloning what can be a sizable string.
#[derive(Debug, Clone, Copy)]
pub struct MentionProfile<'a> {
    /// Lowercase hex public key.
    pub pubkey: &'a str,
    /// Raw kind 0 event `content` field (a JSON object).
    pub content_json: &'a str,
}

/// Extract single-word `@mention` names from message content.
///
/// Prefer [`extract_at_mentions_with_known`] when known member names are
/// available — it correctly handles multi-word display names.
///
/// Returns lowercased names found after `@` tokens. An `@name` only matches
/// when the `@` is at start-of-string or preceded by an ASCII whitespace
/// character — this excludes things like email addresses (`user@host`).
///
/// Allowed name characters: ASCII alphanumerics, `.`, `-`, `_`.
/// Duplicates are removed; first-seen order is preserved.
pub fn extract_at_names(content: &str) -> Vec<String> {
    if content.is_empty() || !content.contains('@') {
        return vec![];
    }
    let mut names: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i] == '@' {
            let preceded_by_ws = i == 0 || chars[i - 1].is_ascii_whitespace();
            if preceded_by_ws && i + 1 < len {
                let start = i + 1;
                let mut end = start;
                while end < len {
                    let c = chars[end];
                    if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                        end += 1;
                    } else {
                        break;
                    }
                }
                if end > start {
                    let name: String = chars[start..end].iter().collect();
                    let lower = name.to_ascii_lowercase();
                    if seen.insert(lower.clone()) {
                        names.push(lower);
                    }
                }
            }
        }
        i += 1;
    }
    names
}

/// Extract `@mention` names from message content using known member names.
///
/// At each `@` preceded by whitespace or start-of-string, tries known names
/// longest-first (case-insensitive, word-boundary-checked), then falls back
/// to single-word tokenization. Returns lowercased names in first-seen order,
/// deduplicated. Empty/whitespace-only entries in `known_names` are ignored.
pub fn extract_at_mentions_with_known(content: &str, known_names: &[&str]) -> Vec<String> {
    if content.is_empty() || !content.contains('@') {
        return vec![];
    }

    let mut sorted: Vec<&str> = known_names
        .iter()
        .copied()
        .filter(|n| !n.trim().is_empty())
        .collect();
    sorted.sort_by_key(|k| std::cmp::Reverse(k.len()));

    let mut names = Vec::new();
    let mut seen = HashSet::new();

    for (i, _) in content.match_indices('@') {
        let preceded = i == 0 || content.as_bytes()[i - 1].is_ascii_whitespace();
        if !preceded {
            continue;
        }
        let rest = &content[i + 1..];
        if rest.is_empty() {
            continue;
        }

        let lower = if let Some(&known) = sorted.iter().find(|&&k| {
            rest.get(..k.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(k) && is_word_boundary(&rest[k.len()..]))
        }) {
            known.to_ascii_lowercase()
        } else {
            let end = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '.' | '-' | '_'))
                .unwrap_or(rest.len());
            if end == 0 {
                continue;
            }
            rest[..end].to_ascii_lowercase()
        };

        if seen.insert(lower.clone()) {
            names.push(lower);
        }
    }
    names
}

fn is_word_boundary(s: &str) -> bool {
    s.chars().next().is_none_or(|c| {
        c.is_ascii_whitespace() || matches!(c, ',' | ';' | '.' | '!' | '?' | ':' | ')' | ']' | '}')
    })
}

/// Match extracted `@names` against channel-member profiles.
///
/// For each profile, parses its `content_json` and reads the
/// `display_name` field (falling back to `name` **only if `display_name`
/// is absent**, preserving the legacy MCP behavior). If the resulting
/// name matches any extracted `@name` case-insensitively, the profile's
/// pubkey is included.
///
/// Output order is **profile-input order**, not name-input order. When
/// the [`MENTION_CAP`] is later applied during merging, this means the
/// matched-pubkey set is stable with respect to query result ordering
/// rather than text-position ordering.
///
/// Profiles whose `content_json` does not parse, or whose `display_name`
/// (and `name`) are absent or non-string, are silently skipped.
///
/// Duplicate display names within a channel will produce multiple matches
/// for a single `@name` — this is by design; resolution is bounded to
/// channel members, so ambiguity is local to that channel.
pub fn match_names_to_profiles(names: &[String], profiles: &[MentionProfile<'_>]) -> Vec<String> {
    if names.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    for p in profiles {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(p.content_json) else {
            continue;
        };
        let name = value
            .get("display_name")
            .or_else(|| value.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        if names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            out.push(p.pubkey.to_string());
        }
    }
    out
}

/// Merge auto-resolved pubkeys into an explicit mention list, up to `cap`.
///
/// Explicit mentions have priority; auto-resolved entries are appended
/// only if not already present (case-sensitive contains check — callers
/// should normalize beforehand). Stops adding once `cap` is reached.
pub fn merge_mentions(explicit: &mut Vec<String>, auto_resolved: &[String], cap: usize) {
    let budget = cap.saturating_sub(explicit.len());
    let mut added = 0usize;
    for pk in auto_resolved {
        if added >= budget {
            break;
        }
        if !explicit.contains(pk) {
            explicit.push(pk.clone());
            added += 1;
        }
    }
}

/// Normalize a list of mention pubkeys.
///
/// - Lowercases every entry.
/// - Removes duplicates, preserving first-seen order.
/// - When `sender_pubkey` is `Some(pk)`, removes any case-insensitive match
///   against the sender's own pubkey (you don't @mention yourself).
pub fn normalize_mention_pubkeys(pubkeys: &[String], sender_pubkey: Option<&str>) -> Vec<String> {
    let sender = sender_pubkey.map(|s| s.to_ascii_lowercase());
    let mut seen = HashSet::new();
    pubkeys
        .iter()
        .map(|pk| pk.to_ascii_lowercase())
        .filter(|pk| sender.as_deref() != Some(pk.as_str()))
        .filter(|pk| seen.insert(pk.clone()))
        .collect()
}

/// Remove fenced code blocks and inline code spans from content.
///
/// Returns a copy of `content` with fenced blocks and inline spans replaced by
/// a single space. Used only for mention scanning — the original content is
/// stored verbatim. Preserves valid UTF-8 throughout.
///
/// Mirrors `maskMarkdownCode` in
/// `desktop/src/features/messages/lib/hasMention.ts`, which is what decides
/// whether Desktop renders and notifies a mention. Where the two disagree the
/// signed `p` tags disagree with what a reader sees: text that renders as code
/// still wakes an agent, or a visible mention carries no tag at all. So this is
/// a transcription of that algorithm, in the same two passes — all fenced
/// blocks first, then spans over what is left — with the same rules for fence
/// markers, delimiter-run lengths, escapes and line endings. Each is spelled
/// out at the pass that applies it, and the parity tests below check the
/// combined result against Desktop's own `hasMention` output.
///
/// Not mirrored: Desktop additionally masks any line indented by four spaces
/// or a tab. That is not CommonMark — an indented line continuing a list item
/// is ordinary text — and copying it here would drop `p` tags from list
/// continuations, which is the more harmful direction. See the PR discussion.
pub fn strip_code_regions(content: &str) -> String {
    let bytes = content.as_bytes();
    let mut masked = vec![false; bytes.len()];

    mask_fenced_blocks(content, &mut masked);
    mask_code_spans(bytes, &mut masked);

    // Each contiguous masked run collapses to one space. Line terminators are
    // never masked, so a fenced block keeps its line structure — harmless for
    // mention scanning, and it keeps offsets from drifting inside a line.
    let mut out = String::with_capacity(content.len());
    let mut in_run = false;
    for (index, ch) in content.char_indices() {
        if masked[index] {
            if !in_run {
                out.push(' ');
                in_run = true;
            }
        } else {
            in_run = false;
            out.push(ch);
        }
    }
    out
}

/// Mask every byte in `start..end`, leaving line terminators alone so line
/// structure survives — the same carve-out Desktop's `maskRange` makes.
fn mask_range(bytes: &[u8], masked: &mut [bool], start: usize, end: usize) {
    for index in start..end.min(bytes.len()) {
        if bytes[index] != b'\n' && bytes[index] != b'\r' {
            masked[index] = true;
        }
    }
}

/// Byte range of each line, excluding its terminator.
///
/// Recognizes all three conventions — LF, CRLF and CR — because Desktop does,
/// explicitly. A CR-only message whose fences went unrecognized here would be
/// masked by Desktop and not by us, so its `@name` would be tagged for text
/// the reader sees as code.
fn line_ranges(content: &str) -> Vec<(usize, usize)> {
    let bytes = content.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\n' => {
                lines.push((start, cursor));
                cursor += 1;
                start = cursor;
            }
            b'\r' => {
                lines.push((start, cursor));
                cursor += if bytes.get(cursor + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = cursor;
            }
            _ => cursor += 1,
        }
    }
    if start < bytes.len() {
        lines.push((start, bytes.len()));
    }
    lines
}

/// First pass: mask every fenced block, whole lines at a time.
///
/// Runs before spans, exactly as Desktop does, and that order is load-bearing:
/// scanning both in one pass lets a backtick inside a fenced body close an
/// outer span, after which the real closing fence reads as a fresh unterminated
/// opener and swallows the rest of the message — suppressing a mention that is
/// plainly visible below the block.
fn mask_fenced_blocks(content: &str, masked: &mut [bool]) {
    let bytes = content.as_bytes();
    let mut fence: Option<(u8, usize)> = None;

    for (start, end) in line_ranges(content) {
        let line = &content[start..end];
        let indent = line.len() - line.trim_start_matches(' ').len();

        if let Some((marker, length)) = fence {
            mask_range(bytes, masked, start, end);
            if indent <= 3 {
                let run = run_length(bytes, start + indent, marker);
                // A closing fence is at least as long as the opener and carries
                // nothing but trailing whitespace.
                if run >= length && line[indent + run..].chars().all(|c| c == ' ' || c == '\t') {
                    fence = None;
                }
            }
            continue;
        }

        if indent > 3 {
            continue;
        }
        let marker = match bytes.get(start + indent) {
            Some(&b @ (b'`' | b'~')) => b,
            _ => continue,
        };
        let length = run_length(bytes, start + indent, marker);
        if length < 3 {
            continue;
        }
        // A backtick fence's info string may not contain a backtick
        // (CommonMark), so ``` `x` ``` on one line is a span, not a block.
        if marker == b'`' && line[indent + length..].contains('`') {
            continue;
        }
        mask_range(bytes, masked, start, end);
        fence = Some((marker, length));
    }
}

/// Second pass: mask inline code spans across the unmasked text.
///
/// A transcription of Desktop's span loop, including the two rules that are
/// easy to get wrong in the other direction:
///
/// * a span may cross a line ending. CommonMark allows it and Desktop allows
///   it, so refusing here left `` `code\n@alice` `` tagged as a live mention;
/// * a backslash before a *closing* run does not escape it. Backslashes are
///   literal inside a code span, so Desktop pairs with that run — skipping it
///   pairs with a later delimiter instead and can swallow a visible mention.
///
/// An opener with no matching closer advances past the whole run, not one
/// backtick: reconsidering its remaining backticks as shorter openers makes the
/// two implementations pair later delimiters differently.
fn mask_code_spans(bytes: &[u8], masked: &mut [bool]) {
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'`' || masked[index] || is_escaped(bytes, index) {
            index += 1;
            continue;
        }

        let opener_end = index + unmasked_run_length(bytes, masked, index);
        let delimiter = opener_end - index;

        let mut closer = opener_end;
        let mut closed = None;
        while closer < bytes.len() {
            if bytes[closer] != b'`' || masked[closer] {
                closer += 1;
                continue;
            }
            let closer_end = closer + unmasked_run_length(bytes, masked, closer);
            if closer_end - closer == delimiter {
                closed = Some(closer_end);
                break;
            }
            closer = closer_end;
        }

        match closed {
            Some(closer_end) => {
                mask_range(bytes, masked, index, closer_end);
                index = closer_end;
            }
            None => index = opener_end,
        }
    }
}

/// How many consecutive `marker` bytes start at `index`.
fn run_length(bytes: &[u8], index: usize, marker: u8) -> usize {
    bytes[index..].iter().take_while(|&&b| b == marker).count()
}

/// Run of backticks at `index` that stops at the first already-masked one — a
/// fenced block's backticks are not part of a span's delimiter run.
fn unmasked_run_length(bytes: &[u8], masked: &[bool], index: usize) -> usize {
    let mut length = 0usize;
    while index + length < bytes.len() && bytes[index + length] == b'`' && !masked[index + length] {
        length += 1;
    }
    length
}

/// Whether the byte at `index` is preceded by an odd number of backslashes.
fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut backslashes = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }
    backslashes % 2 == 1
}

/// Bech32 alphabet used by NIP-19.
// NIP-19 allows uppercase; normalize before decode
fn is_bech32_char(c: char) -> bool {
    matches!(c, '0'..='9' | 'a'..='z' | 'A'..='Z')
}

/// Extract pubkeys from NIP-27 `nostr:npub1…` URIs in content.
///
/// Scans `content` (which should already have code regions stripped via
/// [`strip_code_regions`]) for `nostr:npub1` followed by 58 bech32 characters.
/// Decodes each to a 32-byte pubkey hex string. Invalid bech32 is silently
/// skipped. Returns deduplicated lowercase hex pubkeys.
pub fn extract_nostr_uris(content: &str) -> Vec<String> {
    const PREFIX: &str = "nostr:npub1";
    const BECH32_SUFFIX_LEN: usize = 58; // chars after "npub1"

    let mut pubkeys = Vec::new();
    let mut seen = HashSet::new();

    for (start, _) in content.match_indices(PREFIX) {
        let bech32_start = start + "nostr:".len();
        let bech32_end = bech32_start + 5 + BECH32_SUFFIX_LEN; // "npub1" + 58

        // The fixed-width window can land mid-character when multi-byte UTF-8
        // follows the prefix; slicing a non-boundary would panic. A real bech32
        // suffix is 58 ASCII bytes, so any non-boundary here is a non-match.
        if bech32_end > content.len() || !content.is_char_boundary(bech32_end) {
            continue;
        }

        let candidate = &content[bech32_start..bech32_end];
        if !candidate.chars().all(is_bech32_char) {
            continue;
        }

        // NIP-19 allows uppercase; normalize before decode
        let normalized = candidate.to_ascii_lowercase();
        if let Ok(pk) = PublicKey::from_bech32(&normalized) {
            let hex = pk.to_hex();
            if seen.insert(hex.clone()) {
                pubkeys.push(hex);
            }
        }
    }

    pubkeys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_at_names_matches_basic() {
        assert_eq!(extract_at_names("hello @alice"), vec!["alice"]);
        assert_eq!(extract_at_names("@bob hello"), vec!["bob"]);
    }

    #[test]
    fn extract_at_names_lowercases_and_dedups() {
        assert_eq!(
            extract_at_names("@Alice and @alice, meet @Bob"),
            vec!["alice", "bob"]
        );
    }

    #[test]
    fn extract_at_names_allows_newline_prefix() {
        assert_eq!(extract_at_names("line1\n@tyler line2"), vec!["tyler"]);
    }

    #[test]
    fn extract_at_names_allows_punctuation_in_names() {
        assert_eq!(
            extract_at_names("@john.doe @mary_jane @bob-smith"),
            vec!["john.doe", "mary_jane", "bob-smith"]
        );
    }

    #[test]
    fn extract_at_names_rejects_email_and_empty() {
        assert!(extract_at_names("").is_empty());
        assert!(extract_at_names("no mentions").is_empty());
        assert!(extract_at_names("user@example.com").is_empty());
        assert!(extract_at_names("hello @ world").is_empty());
        assert!(extract_at_names("hello @").is_empty());
    }

    #[test]
    fn known_multiword_name_matches_fully() {
        // "Will Pfleger" should match @Will Pfleger, not just @Will.
        let result = extract_at_mentions_with_known("hello @Will Pfleger!", &["Will Pfleger"]);
        assert_eq!(result, vec!["will pfleger"]);
    }

    #[test]
    fn partial_first_word_does_not_match_multiword_name() {
        // @Will alone must NOT match "Will Pfleger" — partial matches are rejected.
        let result = extract_at_mentions_with_known("hey @Will how are you", &["Will Pfleger"]);
        // No known name matches @Will (boundary check: 'Will' is followed by ' h'
        // which would match "Will Pfleger" only if the full name follows).
        // Falls back to single-word tokenizer → emits "will".
        assert_eq!(result, vec!["will"]);
    }

    #[test]
    fn longest_first_wins_over_prefix() {
        // With both "Will" and "Will Pfleger" known, "@Will Pfleger" should
        // match the longer name, not just "Will".
        let result = extract_at_mentions_with_known(
            "@Will Pfleger sent a message",
            &["Will", "Will Pfleger"],
        );
        assert_eq!(result, vec!["will pfleger"]);
    }

    #[test]
    fn single_word_known_name_matches() {
        let result = extract_at_mentions_with_known("ping @alice please", &["Alice"]);
        assert_eq!(result, vec!["alice"]);
    }

    #[test]
    fn unknown_name_falls_back_to_single_word() {
        // @alice is not in known_names but single-word fallback still emits it.
        let result = extract_at_mentions_with_known("hey @alice", &["Bob"]);
        assert_eq!(result, vec!["alice"]);
    }

    #[test]
    fn multiple_mentions_mixed_known_and_unknown() {
        let result = extract_at_mentions_with_known(
            "@Will Pfleger and @alice should review",
            &["Will Pfleger"],
        );
        assert_eq!(result, vec!["will pfleger", "alice"]);
    }

    #[test]
    fn deduplicates_case_insensitively() {
        let result = extract_at_mentions_with_known(
            "@Will Pfleger and @will pfleger again",
            &["Will Pfleger"],
        );
        assert_eq!(result, vec!["will pfleger"]);
    }

    #[test]
    fn multiword_name_at_end_of_string() {
        let result = extract_at_mentions_with_known("cc @Will Pfleger", &["Will Pfleger"]);
        assert_eq!(result, vec!["will pfleger"]);
    }

    #[test]
    fn multiword_name_followed_by_punctuation() {
        let result =
            extract_at_mentions_with_known("thanks @Will Pfleger, great work", &["Will Pfleger"]);
        assert_eq!(result, vec!["will pfleger"]);
    }

    #[test]
    fn email_address_not_matched() {
        let result = extract_at_mentions_with_known("user@example.com", &["example.com"]);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_content_returns_empty() {
        let result = extract_at_mentions_with_known("", &["Alice"]);
        assert!(result.is_empty());
    }

    #[test]
    fn empty_known_names_uses_single_word_fallback() {
        let result = extract_at_mentions_with_known("hey @alice", &[]);
        assert_eq!(result, vec!["alice"]);
    }

    #[test]
    fn unicode_content_does_not_panic() {
        // Known name byte-length may land mid-character in multi-byte content.
        // e.g. known "ab" (2 bytes) vs content starting with 日 (3 bytes) —
        // byte offset 2 is not a char boundary. Must not panic; gracefully
        // skips the candidate via get() returning None.
        let result = extract_at_mentions_with_known("@日本語 hello", &["ab"]);
        // "ab" doesn't match — falls through to single-word fallback which
        // stops at non-ASCII, so no match. The key assertion: no panic.
        assert!(result.is_empty());
    }

    #[test]
    fn unicode_known_name_matches_with_boundary() {
        // Multi-byte known name followed by a space (valid boundary).
        let result = extract_at_mentions_with_known("@日本 hello", &["日本"]);
        assert_eq!(result, vec!["日本"]);
    }

    #[test]
    fn unicode_known_name_with_ascii_content_no_panic() {
        // Reverse case: multi-byte known name against ASCII content.
        let result = extract_at_mentions_with_known("@alice hello", &["日本語"]);
        assert_eq!(result, vec!["alice"]);
    }

    fn profile<'a>(pk: &'a str, json: &'a str) -> MentionProfile<'a> {
        MentionProfile {
            pubkey: pk,
            content_json: json,
        }
    }

    #[test]
    fn match_uses_display_name_case_insensitive() {
        let names = vec!["alice".to_string()];
        let profiles = vec![profile("pk1", r#"{"display_name":"Alice"}"#)];
        assert_eq!(match_names_to_profiles(&names, &profiles), vec!["pk1"]);
    }

    #[test]
    fn match_falls_back_to_name_only_if_display_name_absent() {
        let names = vec!["bob".to_string()];
        // display_name present but empty → skipped (no fallback to `name`).
        let p1 = profile("pk1", r#"{"display_name":"","name":"Bob"}"#);
        // display_name absent → falls back to `name`.
        let p2 = profile("pk2", r#"{"name":"Bob"}"#);
        let out = match_names_to_profiles(&names, &[p1, p2]);
        assert_eq!(out, vec!["pk2"]);
    }

    #[test]
    fn match_preserves_profile_input_order() {
        let names = vec!["alice".to_string(), "bob".to_string()];
        let profiles = vec![
            profile("pkB", r#"{"display_name":"Bob"}"#),
            profile("pkA", r#"{"display_name":"Alice"}"#),
        ];
        // Output order tracks the profile slice, not the name slice.
        assert_eq!(
            match_names_to_profiles(&names, &profiles),
            vec!["pkB", "pkA"]
        );
    }

    #[test]
    fn match_returns_all_pubkeys_for_duplicate_display_names() {
        // Ambiguity is intentional and bounded to channel members.
        let names = vec!["alice".to_string()];
        let profiles = vec![
            profile("pk1", r#"{"display_name":"Alice"}"#),
            profile("pk2", r#"{"display_name":"alice"}"#),
        ];
        assert_eq!(
            match_names_to_profiles(&names, &profiles),
            vec!["pk1", "pk2"]
        );
    }

    #[test]
    fn match_skips_unparseable_and_missing_fields() {
        let names = vec!["alice".to_string()];
        let profiles = vec![
            profile("pk1", "not json"),
            profile("pk2", "{}"),
            profile("pk3", r#"{"display_name":42}"#),
            profile("pk4", r#"{"display_name":"Alice"}"#),
        ];
        assert_eq!(match_names_to_profiles(&names, &profiles), vec!["pk4"]);
    }

    #[test]
    fn match_empty_names_returns_empty() {
        let profiles = vec![profile("pk1", r#"{"display_name":"Alice"}"#)];
        assert!(match_names_to_profiles(&[], &profiles).is_empty());
    }

    #[test]
    fn merge_appends_new_and_skips_dupes() {
        let mut m = vec!["a".to_string()];
        merge_mentions(&mut m, &["a".into(), "b".into()], MENTION_CAP);
        assert_eq!(m, vec!["a", "b"]);
    }

    #[test]
    fn merge_respects_cap() {
        let mut m: Vec<String> = (0..49).map(|i| format!("pk{i}")).collect();
        merge_mentions(&mut m, &["x".into(), "y".into()], MENTION_CAP);
        assert_eq!(m.len(), MENTION_CAP);
        assert_eq!(m.last().unwrap(), "x");
    }

    #[test]
    fn merge_noop_when_explicit_at_cap() {
        let mut m: Vec<String> = (0..MENTION_CAP).map(|i| format!("pk{i}")).collect();
        merge_mentions(&mut m, &["extra".into()], MENTION_CAP);
        assert_eq!(m.len(), MENTION_CAP);
        assert!(!m.contains(&"extra".to_string()));
    }

    #[test]
    fn normalize_lowercases_and_dedups() {
        let pks = vec!["ABC".to_string(), "abc".to_string(), "DEF".to_string()];
        assert_eq!(normalize_mention_pubkeys(&pks, None), vec!["abc", "def"]);
    }

    #[test]
    fn normalize_removes_sender_case_insensitive() {
        let pks = vec!["ABC".to_string(), "DEF".to_string()];
        assert_eq!(normalize_mention_pubkeys(&pks, Some("abc")), vec!["def"]);
    }

    #[test]
    fn normalize_with_none_sender_keeps_everything() {
        let pks = vec!["abc".to_string()];
        assert_eq!(normalize_mention_pubkeys(&pks, None), vec!["abc"]);
    }

    #[test]
    fn normalize_empty_input() {
        assert!(normalize_mention_pubkeys(&[], Some("anything")).is_empty());
    }

    #[test]
    fn strip_code_regions_removes_fenced_block() {
        let input = "before\n```rust\nlet x = 1;\n```\nafter";
        let stripped = strip_code_regions(input);
        assert!(!stripped.contains("let x = 1"));
        assert!(stripped.contains("before"));
        assert!(stripped.contains("after"));
    }

    #[test]
    fn strip_code_regions_removes_inline_code() {
        let input =
            "see `nostr:npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjptg` here";
        let stripped = strip_code_regions(input);
        assert!(!stripped.contains("npub1"));
        assert!(stripped.contains("see"));
        assert!(stripped.contains("here"));
    }

    #[test]
    fn strip_code_regions_preserves_prose() {
        let input =
            "hello nostr:npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjptg world";
        let stripped = strip_code_regions(input);
        assert!(stripped.contains("nostr:npub1"));
    }

    #[test]
    fn strip_code_regions_handles_empty() {
        assert_eq!(strip_code_regions(""), "");
    }

    #[test]
    fn strip_code_regions_unclosed_backtick_preserved() {
        // A lone backtick without a closing one is not a code span
        let input = "hello `world";
        let stripped = strip_code_regions(input);
        assert!(stripped.contains("world"));
    }

    const TEST_NPUB1: &str = "npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjptg";
    const TEST_HEX1: &str = "7e7e9c42a91bfef19fa929e5fda1b72e0ebc1a4c1141673e2794234d86addf4e";
    const TEST_NPUB2: &str = "npub1fgdl5qqnh3k3f2xkqrvt7cujalhm623x4s7fdjdj5yrtp5fzjl9qrjpucw";
    const TEST_HEX2: &str = "4a1bfa0013bc6d14a8d600d8bf6392efefbd2a26ac3c96c9b2a106b0d12297ca";

    #[test]
    fn extract_nostr_uris_valid_in_prose() {
        let content = format!("hello nostr:{} world", TEST_NPUB1);
        let result = extract_nostr_uris(&content);
        assert_eq!(result, vec![TEST_HEX1]);
    }

    #[test]
    fn extract_nostr_uris_not_extracted_in_backticks() {
        let content = format!("see `nostr:{}` here", TEST_NPUB1);
        let stripped = strip_code_regions(&content);
        let result = extract_nostr_uris(&stripped);
        assert!(result.is_empty());
    }

    #[test]
    fn extract_nostr_uris_not_extracted_in_fenced_code() {
        let content = format!("before\n```\nnostr:{}\n```\nafter", TEST_NPUB1);
        let stripped = strip_code_regions(&content);
        let result = extract_nostr_uris(&stripped);
        assert!(result.is_empty());
    }

    #[test]
    fn extract_nostr_uris_invalid_bech32_skipped() {
        // Corrupt the last few chars to make invalid bech32
        let invalid = "npub10elfcs4fr0l0r8af98jlmgdh9c8tcxjvz9qkw038js35mp4dma8qzvjaaaa";
        let content = format!("nostr:{}", invalid);
        let result = extract_nostr_uris(&content);
        // Should not panic, just skip
        assert!(result.is_empty());
    }

    #[test]
    fn extract_nostr_uris_deduplicates() {
        let content = format!("nostr:{} and again nostr:{}", TEST_NPUB1, TEST_NPUB1);
        let result = extract_nostr_uris(&content);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], TEST_HEX1);
    }

    #[test]
    fn extract_nostr_uris_multiple_different() {
        let content = format!("nostr:{} and nostr:{}", TEST_NPUB1, TEST_NPUB2);
        let result = extract_nostr_uris(&content);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&TEST_HEX1.to_string()));
        assert!(result.contains(&TEST_HEX2.to_string()));
    }

    #[test]
    fn extract_nostr_uris_at_name_and_npub_dedup() {
        // Simulates the integration: @name resolves to same pubkey as nostr:npub
        // The dedup happens at the merge_mentions level, but extract_nostr_uris
        // itself deduplicates within its own output.
        let content = format!("nostr:{}", TEST_NPUB1);
        let uri_pubkeys = extract_nostr_uris(&content);
        let name_pubkeys = vec![TEST_HEX1.to_string()];

        // merge_mentions deduplicates
        let mut merged = name_pubkeys;
        merge_mentions(&mut merged, &uri_pubkeys, MENTION_CAP);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0], TEST_HEX1);
    }

    #[test]
    fn extract_nostr_uris_empty_content() {
        assert!(extract_nostr_uris("").is_empty());
    }

    #[test]
    fn extract_nostr_uris_no_prefix() {
        // npub without "nostr:" prefix should not match
        let content = format!("just {} in text", TEST_NPUB1);
        let result = extract_nostr_uris(&content);
        assert!(result.is_empty());
    }

    #[test]
    fn extract_nostr_uris_after_unicode_does_not_panic() {
        // Multi-byte UTF-8 before a nostr: URI must not cause panics
        let content = format!("こんにちは nostr:{}", TEST_NPUB1);
        let result = extract_nostr_uris(&content);
        assert_eq!(result, vec![TEST_HEX1]);
    }

    #[test]
    fn extract_nostr_uris_multibyte_inside_window_does_not_panic() {
        // Multi-byte UTF-8 within the fixed 58-char suffix window would make
        // bech32_end land mid-character; the boundary guard must skip it.
        let content = format!("nostr:npub1{}", "あ".repeat(20));
        assert!(extract_nostr_uris(&content).is_empty());
    }

    #[test]
    fn strip_code_regions_preserves_unicode() {
        let input = "こんにちは `code` 世界";
        let stripped = strip_code_regions(input);
        assert!(stripped.contains("こんにちは"));
        assert!(stripped.contains("世界"));
        assert!(!stripped.contains("code"));
    }

    #[test]
    fn extract_nostr_uris_uppercase_bech32_chars() {
        // NIP-19 allows uppercase bech32 characters in the suffix
        let upper_suffix = &TEST_NPUB1[5..].to_uppercase(); // uppercase the 58 chars after "npub1"
        let npub_mixed = format!("npub1{}", upper_suffix);
        let content = format!("nostr:{}", npub_mixed);
        let result = extract_nostr_uris(&content);
        assert_eq!(result, vec![TEST_HEX1]);
    }
}

#[cfg(test)]
mod code_region_parity_tests {
    use super::{extract_at_mentions_with_known, strip_code_regions};

    /// Whether the send path would emit a `p` tag for `alice`.
    fn tags_alice(content: &str) -> bool {
        !extract_at_mentions_with_known(&strip_code_regions(content), &["alice"]).is_empty()
    }

    /// Each of these renders as code in Desktop, so `hasMention` returns false
    /// for it — verified by running Desktop's own `hasMention` over the same
    /// inputs. Tagging them anyway wakes an agent for text nobody addressed.
    #[test]
    fn code_desktop_masks_never_produces_a_mention() {
        for content in [
            "~~~\n@alice\n~~~",
            "~~~~\n@alice\n~~~~",
            "```\n@alice\n```",
            "``@alice``",
            "``` @alice ```",
            "`@alice`",
            // A longer opener is not closed by a shorter run.
            "````\n@alice\n```\nstill code\n````",
            // Never closed: code to the end of the message, as Desktop treats it.
            "```\n@alice",
            "~~~\n@alice",
            // CommonMark allows up to three spaces of indent before a fence.
            "   ```\n@alice\n   ```",
        ] {
            assert!(
                !tags_alice(content),
                "must not tag a mention in {content:?}"
            );
        }
    }

    /// The other direction: a visible mention must keep its tag. A missed tag
    /// is the worse failure — the agent never wakes at all.
    #[test]
    fn a_visible_mention_still_tags() {
        for content in [
            "hi @alice",
            "```\ncode\n```\nhi @alice",
            "~~~\ncode\n~~~\nhi @alice",
            "``code`` and @alice",
            "`code` then @alice",
            // A fence marker mid-line opens nothing.
            "look at ``` @alice",
            // A backtick fence's info string cannot contain a backtick, so
            // this is a span followed by text, not an unterminated block.
            "``` `x` ``` @alice",
        ] {
            assert!(tags_alice(content), "must tag the mention in {content:?}");
        }
    }

    /// An escaped backtick does not open a code span — matching Desktop, which
    /// skips escaped delimiters when masking. The text survives stripping; it
    /// still resolves to no mention because a backtick is not a mention opener,
    /// which is exactly what Desktop concludes.
    #[test]
    fn an_escaped_backtick_does_not_open_a_span() {
        assert_eq!(strip_code_regions("\\`@alice\\`"), "\\`@alice\\`");
        assert!(!tags_alice("\\`@alice\\`"));
        // The mention outside the escaped pair is untouched.
        assert!(tags_alice("\\`x\\` @alice"));
    }

    /// A code span may cross a line ending — CommonMark allows it and Desktop
    /// allows it. Refusing here left the span unmasked, so `` `code\n@alice` ``
    /// emitted a `p` tag and could wake alice for code.
    #[test]
    fn a_code_span_may_cross_a_line_ending() {
        assert!(!tags_alice("`code\n@alice` after"));
        assert!(!tags_alice("`code\r\n@alice` after"));
    }

    /// Backslashes are literal inside a code span, so a backtick run preceded
    /// by one still closes it — Desktop pairs with that run. Treating it as
    /// escaped pairs the opener with a later delimiter instead, swallowing
    /// everything between, including a visible mention.
    #[test]
    fn a_backslash_does_not_escape_a_closing_run() {
        assert!(tags_alice("`x\\` @alice `y"));
        assert!(tags_alice("a `x\\` @alice ok"));
        // An escaped backtick still does not *open* a span, as before.
        assert!(tags_alice("\\`x\\` @alice"));
    }

    /// An opener with no matching closer advances past the whole run.
    /// Reconsidering its remaining backticks as shorter openers makes the two
    /// implementations pair later delimiters differently.
    #[test]
    fn an_unmatched_opener_advances_past_its_whole_run() {
        assert!(tags_alice("``a`b @alice"));
        assert!(tags_alice("```a`b @alice"));
    }

    /// Fenced blocks are masked before spans are scanned, as Desktop does.
    /// In one pass, the backtick inside the body closes an outer span, the real
    /// closing fence then reads as a fresh unterminated opener, and the visible
    /// mention below the block loses its tag.
    #[test]
    fn fences_are_masked_before_spans_are_scanned() {
        assert!(tags_alice("```\n`\n```\n@alice"));
        assert!(tags_alice("```\n``\n```\n@alice"));
    }

    /// Desktop recognizes LF, CRLF and CR line endings explicitly. A CR-only
    /// message whose fences went unrecognized here would be masked there and
    /// not here, tagging a mention the reader sees as code.
    #[test]
    fn fence_lines_are_found_under_all_three_line_endings() {
        for content in [
            "```\n@alice\n```",
            "```\r\n@alice\r\n```",
            "```\r@alice\r```",
        ] {
            assert!(!tags_alice(content), "must not tag inside {content:?}");
        }
        for content in [
            "```\ncode\n```\n@alice",
            "```\r\ncode\r\n```\r\n@alice",
            "```\rcode\r```\r@alice",
        ] {
            assert!(
                tags_alice(content),
                "must tag after the block in {content:?}"
            );
        }
    }

    /// Documented divergence: Desktop also masks any line indented four spaces
    /// or a tab. That is not CommonMark — the indented line here continues a
    /// list item and renders as ordinary text — so copying the rule would drop
    /// a `p` tag from a message whose mention is plainly visible.
    #[test]
    fn an_indented_list_continuation_keeps_its_mention() {
        assert!(tags_alice("- item\n    @alice look"));
    }
}
