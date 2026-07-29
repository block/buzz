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

use buzz_core::channel_mentions::is_reserved_mention_token;
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
///
/// The reserved channel-wide mention tokens (`@channel`, `@here`) are never
/// returned — see [`extract_reserved_mention_tokens`].
pub fn extract_at_names(content: &str) -> Vec<String> {
    scan_at_tokens(content)
        .into_iter()
        .filter(|name| !is_reserved_mention_token(name))
        .collect()
}

/// Extract the reserved channel-wide mention tokens (`channel`, `here`) that
/// appear as `@tokens` in `content`.
///
/// Returned lowercased, deduplicated, in first-seen order. Matching is
/// case-insensitive, so `@Here` is reported as `here`. Callers that care about
/// code blocks should pass content through [`strip_code_regions`] first.
pub fn extract_reserved_mention_tokens(content: &str) -> Vec<String> {
    scan_at_tokens(content)
        .into_iter()
        .filter(|name| is_reserved_mention_token(name))
        .collect()
}

/// Scan single-word `@tokens`, lowercased, deduplicated, first-seen order.
fn scan_at_tokens(content: &str) -> Vec<String> {
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
///
/// The reserved channel-wide mention tokens (`channel`, `here`) are never
/// returned, even when a member is literally named one of them.
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

        if is_reserved_mention_token(&lower) {
            continue;
        }
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
/// (and `name`) are absent or non-string, are silently skipped. So are
/// profiles named after a reserved channel-wide mention token (`channel`,
/// `here`) — those tokens never resolve to an identity.
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
        if name.is_empty() || is_reserved_mention_token(name) {
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

/// Length of the run of `marker` characters starting at byte index `i`.
fn marker_run_len(content: &str, i: usize, marker: char) -> usize {
    content[i..].chars().take_while(|&c| c == marker).count()
}

/// Whether byte index `i` is preceded only by whitespace on its line.
fn at_line_start(content: &str, i: usize) -> bool {
    let before = &content[..i];
    match before.rsplit_once('\n') {
        Some((_, after_nl)) => after_nl.chars().all(|c| c.is_ascii_whitespace()),
        None => before.chars().all(|c| c.is_ascii_whitespace()),
    }
}

/// If a fenced code block opens at `i`, return the byte index just past it.
///
/// A fence is a run of three or more `marker` characters (`` ` `` or `~`) at
/// the start of a line. It is closed by a run of at least the same length at
/// the start of a later line, per CommonMark, or by the end of the content.
fn fence_close_end(content: &str, i: usize, marker: char) -> Option<usize> {
    let open_len = marker_run_len(content, i, marker);
    if open_len < 3 || !at_line_start(content, i) {
        return None;
    }

    // Skip the rest of the opening fence line (the info string).
    let after_fence = i + open_len;
    let mut search_from = content[after_fence..]
        .find('\n')
        .map_or(content.len(), |p| after_fence + p + 1);

    loop {
        let Some(pos) = content[search_from..].find(marker) else {
            return Some(content.len());
        };
        let abs = search_from + pos;
        let run = marker_run_len(content, abs, marker);
        if run >= open_len && at_line_start(content, abs) {
            // Consume the rest of the closing fence line.
            let after_close = abs + run;
            return Some(
                content[after_close..]
                    .find('\n')
                    .map_or(content.len(), |p| after_close + p + 1),
            );
        }
        search_from = abs + run;
    }
}

/// If an inline code span opens at `i`, return the byte index just past it.
///
/// A span opens with a run of `n` backticks and closes with the next run of
/// *exactly* `n` backticks on the same line (CommonMark's equal-length rule),
/// so ``` ``@here`` ``` masks its contents just as `` `@here` `` does.
fn code_span_close_end(content: &str, i: usize) -> Option<usize> {
    let open_len = marker_run_len(content, i, '`');
    let after_open = i + open_len;
    let mut search_from = after_open;

    while let Some(pos) = content[search_from..].find('`') {
        let abs = search_from + pos;
        if content[after_open..abs].contains('\n') {
            return None;
        }
        let run = marker_run_len(content, abs, '`');
        if run == open_len {
            return Some(abs + run);
        }
        search_from = abs + run;
    }
    None
}

/// Remove fenced code blocks and inline code spans from content.
///
/// Returns a copy of `content` with fenced blocks (three or more backticks
/// **or** tildes at line start) and backtick code spans of any delimiter
/// length replaced by spaces. Used only for mention scanning — the original
/// content is stored verbatim. Preserves valid UTF-8 throughout.
pub fn strip_code_regions(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut chars = content.char_indices().peekable();

    while let Some(&(i, ch)) = chars.peek() {
        // Fenced code block: ``` or ~~~ (3+) at line start.
        if ch == '`' || ch == '~' {
            if let Some(close_end) = fence_close_end(content, i, ch) {
                out.push(' ');
                while let Some(&(ci, _)) = chars.peek() {
                    if ci >= close_end {
                        break;
                    }
                    chars.next();
                }
                continue;
            }
        }

        // Inline code span: `…`, ``…``, and longer.
        if ch == '`' {
            if let Some(close_end) = code_span_close_end(content, i) {
                out.push(' ');
                while let Some(&(ci, _)) = chars.peek() {
                    if ci >= close_end {
                        break;
                    }
                    chars.next();
                }
                continue;
            }
        }

        out.push(ch);
        chars.next();
    }

    out
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
    fn reserved_tokens_are_never_extracted_as_names() {
        assert!(extract_at_names("@channel ship it").is_empty());
        assert!(extract_at_names("heads up @Here").is_empty());
        assert_eq!(
            extract_at_names("@channel and @alice"),
            vec!["alice"],
            "regular names alongside a reserved token still resolve"
        );
    }

    #[test]
    fn reserved_tokens_lose_to_no_one_even_a_member_named_here() {
        // A member whose display name is literally "here" must not be pulled in
        // by @here — the reserved token wins in every parser.
        let names = extract_at_mentions_with_known("ping @here now", &["here", "Alice"]);
        assert!(names.is_empty(), "got {names:?}");

        let profiles = [MentionProfile {
            pubkey: "aa",
            content_json: r#"{"display_name":"here"}"#,
        }];
        assert!(match_names_to_profiles(&["here".to_string()], &profiles).is_empty());
    }

    #[test]
    fn reserved_token_prefixes_are_still_ordinary_names() {
        assert_eq!(
            extract_at_names("@channels @herer"),
            vec!["channels", "herer"]
        );
    }

    #[test]
    fn extract_reserved_mention_tokens_reports_lowercased_tokens() {
        assert_eq!(
            extract_reserved_mention_tokens("hey @Channel and @here"),
            vec!["channel", "here"]
        );
        assert!(extract_reserved_mention_tokens("hey @alice").is_empty());
        assert!(extract_reserved_mention_tokens("user@channel.com").is_empty());
        assert_eq!(
            extract_reserved_mention_tokens("@here @HERE"),
            vec!["here"],
            "deduplicated case-insensitively"
        );
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

    #[test]
    fn strip_code_regions_removes_tilde_fenced_block() {
        let stripped = strip_code_regions("before\n~~~\n@here\n~~~\nafter");
        assert!(!stripped.contains("@here"), "{stripped:?}");
        assert!(stripped.contains("before") && stripped.contains("after"));

        // A longer closer still closes the fence; a shorter one does not.
        let longer = strip_code_regions("~~~\n@channel\n~~~~\nafter");
        assert!(!longer.contains("@channel"), "{longer:?}");
        assert!(longer.contains("after"));
        let shorter = strip_code_regions("~~~~\n@channel\n~~~\nstill code");
        assert!(!shorter.contains("@channel"), "{shorter:?}");
        assert!(!shorter.contains("still code"), "{shorter:?}");

        // An unclosed fence masks to end of content.
        let unclosed = strip_code_regions("~~~\n@here never closed");
        assert!(!unclosed.contains("@here"), "{unclosed:?}");

        // Strikethrough is two tildes — not a fence.
        let strike = strip_code_regions("~~@here~~ shipped");
        assert!(strike.contains("@here"), "{strike:?}");
    }

    #[test]
    fn strip_code_regions_removes_multi_backtick_span() {
        let stripped = strip_code_regions("see ``@here`` there");
        assert!(!stripped.contains("@here"), "{stripped:?}");
        assert!(stripped.contains("see") && stripped.contains("there"));

        // A shorter run inside the span does not close it.
        let nested = strip_code_regions("``a `@channel` c`` tail");
        assert!(!nested.contains("@channel"), "{nested:?}");
        assert!(nested.contains("tail"));

        // Single-backtick spans keep their existing behavior.
        let single = strip_code_regions("a `@here` b");
        assert!(!single.contains("@here"), "{single:?}");
        assert!(single.contains('a') && single.contains('b'));
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
