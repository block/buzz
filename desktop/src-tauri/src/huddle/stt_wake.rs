//! STT wake gate: only p-tag / wake huddle agents when speech uses
//! the explicit phrase **"{activation} {agent name}"** (case-insensitive),
//! with light fuzzy tolerance for STT mis-hears (e.g. "Ok Faybell" ≈ "Okay Fable").
//!
//! Ordinary STT lines still post as kind:9 transcript messages, but with
//! **no** agent p-tags. Bare name address (`Fable, what do you think?`) does
//! **not** wake. Explicit chat @mentions continue to wake via the normal
//! composer send path (unchanged here).
//!
//! Each huddle agent has its own activation keyword (default **"hey"**).
//! Addressable (per-agent) still gates whether a spoken wake applies.

use std::collections::{HashMap, HashSet};

/// Default spoken activation keyword when the user has not chosen one.
pub const DEFAULT_ACTIVATION_KEYWORD: &str = "hey";

/// Short preset list shown in the per-agent activation dropdown.
pub const ACTIVATION_KEYWORD_PRESETS: &[&str] =
    &["hey", "at", "agent", "bot", "robo", "ok", "yo", "okay"];

/// One huddle agent and the display names / aliases that can follow the
/// activation keyword.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNameAlias {
    pub pubkey: String,
    /// Non-empty display names / aliases (trimmed). Matched longest-first.
    pub aliases: Vec<String>,
    /// Per-agent spoken activation keyword (preset allow-list).
    pub activation_keyword: String,
}

/// Normalize a user-facing activation keyword for matching / persistence.
///
/// Returns `None` when the value is empty or not in the preset allow-list
/// (case-insensitive). Presets keep the matcher predictable for STT noise.
pub fn normalize_activation_keyword(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    ACTIVATION_KEYWORD_PRESETS
        .iter()
        .find(|p| p.eq_ignore_ascii_case(&lower))
        .map(|p| (*p).to_string())
}

/// Keyword synonym groups for common STT confusions (ok ↔ okay).
fn keyword_synonym_group(normalized: &str) -> Option<&'static [&'static str]> {
    match normalized {
        "ok" | "okay" => Some(&["ok", "okay"]),
        _ => None,
    }
}

fn collapse_duplicate_letters(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if prev == Some(c) {
            continue;
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// Damerau–Levenshtein (optimal string alignment): insert/delete/substitute
/// plus adjacent transposition, so STT swaps like "Faybel"≈"Fable" stay close.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m {
        dp[i][0] = i;
    }
    for j in 0..=n {
        dp[0][j] = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let mut best = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(dp[i - 2][j - 2] + 1);
            }
            dp[i][j] = best;
        }
    }
    dp[m][n]
}

/// Whether a spoken token matches the agent's configured activation keyword.
///
/// Exact and synonym matches (ok/okay) always pass. Longer keywords also
/// allow a single edit so STT noise like "agents" ≈ "agent" still wakes.
pub fn keywords_fuzzy_match(spoken: &str, expected: &str) -> bool {
    let expected_n = normalize_activation_keyword(expected)
        .unwrap_or_else(|| expected.trim().to_ascii_lowercase());
    if expected_n.is_empty() {
        return false;
    }
    let spoken_raw = spoken.trim().to_ascii_lowercase();
    if spoken_raw.is_empty() {
        return false;
    }
    if let Some(spoken_n) = normalize_activation_keyword(&spoken_raw) {
        if spoken_n == expected_n {
            return true;
        }
        if let (Some(g1), Some(g2)) = (
            keyword_synonym_group(&spoken_n),
            keyword_synonym_group(&expected_n),
        ) {
            if g1.iter().any(|k| g2.contains(k)) {
                return true;
            }
        }
    }
    if let Some(group) = keyword_synonym_group(&expected_n) {
        if group.iter().any(|k| *k == spoken_raw) {
            return true;
        }
        let collapsed = collapse_duplicate_letters(&spoken_raw);
        if group.iter().any(|k| *k == collapsed.as_str()) {
            return true;
        }
    }
    // Single-edit tolerance for keywords of length >= 3 (not "at"/"ok"/"yo").
    if expected_n.len() >= 3 && spoken_raw.len() >= 3 {
        let a = collapse_duplicate_letters(&spoken_raw);
        let b = collapse_duplicate_letters(&expected_n);
        if levenshtein(&a, &b) <= 1 {
            return true;
        }
    }
    false
}

/// Max edit distance allowed for a fuzzy agent-name match.
fn name_fuzzy_allowed_distance(alias_len: usize, heard_len: usize) -> usize {
    let max_len = alias_len.max(heard_len);
    let min_len = alias_len.min(heard_len);
    if min_len < 3 {
        return 0; // very short names: exact only
    }
    // After duplicate-letter collapse + Damerau, allow up to 2 edits for
    // typical STT mangling ("Fable"↔"Faybell") while keeping short typos tight.
    if max_len >= 5 {
        2
    } else {
        1
    }
}

/// Fuzzy compare heard name token(s) to an alias. Returns edit distance when
/// it matches (0 = exact), or None.
pub fn names_fuzzy_match(heard: &str, alias: &str) -> Option<usize> {
    let heard_raw = heard.trim().to_ascii_lowercase();
    let alias_raw = alias.trim().to_ascii_lowercase();
    if heard_raw.is_empty() || alias_raw.is_empty() {
        return None;
    }
    if heard_raw == alias_raw {
        return Some(0);
    }
    let heard_n = collapse_duplicate_letters(&heard_raw);
    let alias_n = collapse_duplicate_letters(&alias_raw);
    if heard_n == alias_n {
        return Some(0);
    }
    // First letter must agree — blocks cross-agent false wakes (Fable vs Maple).
    if heard_n.chars().next() != alias_n.chars().next() {
        return None;
    }
    let dist = levenshtein(&heard_n, &alias_n);
    let allowed = name_fuzzy_allowed_distance(alias_n.len(), heard_n.len());
    if dist <= allowed {
        Some(dist)
    } else {
        None
    }
}

fn is_name_boundary(s: &str) -> bool {
    s.chars().next().is_none_or(|c| {
        c.is_ascii_whitespace() || matches!(c, ',' | ';' | '.' | '!' | '?' | ':' | ')' | ']' | '}')
    })
}

/// Split `text` into whitespace-separated tokens with byte offsets.
fn tokenize(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_whitespace() {
            if let Some(s) = start.take() {
                out.push((s, text[s..idx].to_string()));
            }
        } else if start.is_none() {
            start = Some(idx);
        }
    }
    if let Some(s) = start {
        out.push((s, text[s..].to_string()));
    }
    out
}

/// Strip trailing punctuation from a spoken name token for matching.
fn strip_trailing_punct(token: &str) -> &str {
    token.trim_end_matches(|c: char| {
        matches!(c, ',' | ';' | '.' | '!' | '?' | ':' | ')' | ']' | '}')
    })
}

/// Return pubkeys of huddle agents addressed by spoken
/// `{keyword} <Name>` in `text`, using each agent's own activation keyword
/// and fuzzy tolerance for STT mistakes.
///
/// Matching rules:
/// - Keyword is a standalone token matching the agent's keyword (exact,
///   synonym, or light fuzzy).
/// - Immediately followed by an agent alias (longest first; fuzzy OK).
/// - Alias must end at a word boundary (end of string, whitespace, or
///   comma / punctuation like `,.;:!?)]}`).
/// - Bare agent name without leading keyword does **not** match.
/// - Keyword with no following huddle agent name does **not** match.
/// - When two agents fuzzy-compete for the same span, the closer (then
///   longer alias) wins; an exact tie across different agents wakes none
///   for that span (avoids easy false wakes).
pub fn addressed_agent_pubkeys(text: &str, agents: &[AgentNameAlias]) -> Vec<String> {
    if text.is_empty() || agents.is_empty() {
        return Vec::new();
    }

    let tokens = tokenize(text);
    if tokens.len() < 2 {
        return Vec::new();
    }

    // Candidates: (token_index_of_keyword, pubkey, name_dist, alias_len)
    let mut candidates: Vec<(usize, String, usize, usize)> = Vec::new();

    for (ti, (_off, raw_kw)) in tokens.iter().enumerate() {
        if ti + 1 >= tokens.len() {
            break;
        }
        let kw_token = strip_trailing_punct(raw_kw);
        if kw_token.is_empty() {
            continue;
        }

        for agent in agents {
            let pk = agent.pubkey.trim();
            if pk.is_empty() {
                continue;
            }
            let expected = normalize_activation_keyword(&agent.activation_keyword)
                .unwrap_or_else(|| DEFAULT_ACTIVATION_KEYWORD.to_string());
            if !keywords_fuzzy_match(kw_token, &expected) {
                continue;
            }

            let mut aliases: Vec<&str> = agent
                .aliases
                .iter()
                .map(|a| a.trim())
                .filter(|a| !a.is_empty())
                .collect();
            aliases.sort_by_key(|a| std::cmp::Reverse(a.len()));

            for alias in aliases {
                let alias_words: Vec<&str> = alias.split_whitespace().collect();
                if alias_words.is_empty() {
                    continue;
                }
                let follow = &tokens[ti + 1..];
                if follow.len() < alias_words.len() {
                    continue;
                }

                let mut heard_parts: Vec<String> = Vec::with_capacity(alias_words.len());
                let mut ok = true;
                for (wi, _) in alias_words.iter().enumerate() {
                    let part = strip_trailing_punct(&follow[wi].1);
                    if part.is_empty() {
                        ok = false;
                        break;
                    }
                    heard_parts.push(part.to_ascii_lowercase());
                }
                if !ok {
                    continue;
                }

                let heard_joined = heard_parts.join(" ");
                let alias_joined = alias_words.join(" ").to_ascii_lowercase();

                if let Some(dist) = names_fuzzy_match(&heard_joined, &alias_joined) {
                    candidates.push((ti, pk.to_string(), dist, alias.len()));
                    break; // best (longest) alias for this agent at this keyword
                }
            }
        }
    }

    if candidates.is_empty() {
        return Vec::new();
    }

    // Per keyword token index: keep the best candidate. Prefer lower name
    // distance, then longer alias. If two different pubkeys tie, drop the span.
    let mut best_by_span: HashMap<usize, (String, usize, usize)> = HashMap::new();
    let mut tied_spans: HashSet<usize> = HashSet::new();

    for (ti, pk, dist, alias_len) in candidates {
        if tied_spans.contains(&ti) {
            continue;
        }
        match best_by_span.get(&ti) {
            None => {
                best_by_span.insert(ti, (pk, dist, alias_len));
            }
            Some((best_pk, best_dist, best_len)) => {
                if pk.eq_ignore_ascii_case(best_pk) {
                    if dist < *best_dist || (dist == *best_dist && alias_len > *best_len) {
                        best_by_span.insert(ti, (pk, dist, alias_len));
                    }
                } else if dist < *best_dist || (dist == *best_dist && alias_len > *best_len) {
                    best_by_span.insert(ti, (pk, dist, alias_len));
                } else if dist == *best_dist && alias_len == *best_len {
                    best_by_span.remove(&ti);
                    tied_spans.insert(ti);
                }
            }
        }
    }

    let mut hit_pubkeys = Vec::new();
    let mut seen = HashSet::new();
    let mut spans: Vec<_> = best_by_span.into_iter().collect();
    spans.sort_by_key(|(ti, _)| *ti);
    for (_ti, (pk, _, _)) in spans {
        if seen.insert(pk.to_ascii_lowercase()) {
            hit_pubkeys.push(pk);
        }
    }
    hit_pubkeys
}

/// Build the kind:9 content posted when STT wakes an agent.
///
/// Includes the full prior huddle transcript (all earlier lines) so the agent
/// receives meeting context, not only the tagged wake sentence. When there is
/// no prior context, returns `wake_text` unchanged.
pub fn compose_wake_content(prior_transcript_lines: &[String], wake_text: &str) -> String {
    let wake = wake_text.trim();
    if prior_transcript_lines.is_empty() {
        return wake.to_string();
    }
    let mut body = String::from("[Huddle transcript — full meeting context so far]\n");
    for line in prior_transcript_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        body.push_str(trimmed);
        body.push('\n');
    }
    body.push_str("\n[Addressed]\n");
    body.push_str(wake);
    body
}

/// Chronologically merge channel history lines with the local STT session
/// buffer, dropping exact duplicate consecutive lines and omitting the wake
/// text itself when it already appears as the last history line.
pub fn merge_transcript_context(
    channel_lines: &[String],
    session_lines: &[String],
    wake_text: &str,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let push_unique = |out: &mut Vec<String>, line: &str| {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        if out
            .last()
            .is_some_and(|prev| prev.eq_ignore_ascii_case(trimmed))
        {
            return;
        }
        out.push(trimmed.to_string());
    };
    for line in channel_lines {
        push_unique(&mut out, line);
    }
    for line in session_lines {
        push_unique(&mut out, line);
    }
    let wake = wake_text.trim();
    if !wake.is_empty()
        && out
            .last()
            .is_some_and(|prev| prev.eq_ignore_ascii_case(wake))
    {
        out.pop();
    }
    out
}

/// Build alias lists for huddle agent pubkeys from managed-agent records.
pub fn aliases_from_managed_records(
    agent_pubkeys: &[String],
    records: &[(String, Vec<String>)],
) -> Vec<AgentNameAlias> {
    let mut out = Vec::new();
    for pk in agent_pubkeys {
        let pk_norm = pk.trim();
        if pk_norm.is_empty() {
            continue;
        }
        let mut aliases = Vec::new();
        let mut seen = HashSet::new();
        for (rpk, names) in records {
            if !rpk.eq_ignore_ascii_case(pk_norm) {
                continue;
            }
            for name in names {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let key = trimmed.to_ascii_lowercase();
                if seen.insert(key) {
                    aliases.push(trimmed.to_string());
                }
            }
        }
        if !aliases.is_empty() {
            out.push(AgentNameAlias {
                pubkey: pk_norm.to_string(),
                aliases,
                activation_keyword: DEFAULT_ACTIVATION_KEYWORD.to_string(),
            });
        }
    }
    out
}

/// Collect display-name aliases from a managed agent name + optional display_name.
pub fn collect_record_aliases(name: &str, display_name: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in [Some(name), display_name].into_iter().flatten() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(trimmed.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(pubkey: &str, aliases: &[&str], keyword: &str) -> AgentNameAlias {
        AgentNameAlias {
            pubkey: pubkey.into(),
            aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
            activation_keyword: keyword.into(),
        }
    }

    fn fable() -> AgentNameAlias {
        agent("pk-fable", &["Fable"], "hey")
    }

    fn claim_miner() -> AgentNameAlias {
        agent("pk-cm", &["Claim Miner", "ClaimMiner"], "hey")
    }

    #[test]
    fn hey_fable_tags_fable_default() {
        let hits = addressed_agent_pubkeys("Hey Fable, what do you think?", &[fable()]);
        assert_eq!(hits, vec!["pk-fable"]);
    }

    #[test]
    fn at_fable_tags_when_keyword_is_at() {
        let hits = addressed_agent_pubkeys(
            "At Fable, what do you think?",
            &[agent("pk-fable", &["Fable"], "at")],
        );
        assert_eq!(hits, vec!["pk-fable"]);
    }

    #[test]
    fn bare_fable_does_not_tag() {
        let hits = addressed_agent_pubkeys("Fable, what do you think?", &[fable()]);
        assert!(hits.is_empty());
    }

    #[test]
    fn wrong_keyword_does_not_tag() {
        let hits = addressed_agent_pubkeys("At Fable please", &[fable()]);
        assert!(hits.is_empty());
        let hits =
            addressed_agent_pubkeys("Hey Fable please", &[agent("pk-fable", &["Fable"], "at")]);
        assert!(hits.is_empty());
    }

    #[test]
    fn keyword_case_insensitive() {
        let hits = addressed_agent_pubkeys("HEY fable can you help", &[fable()]);
        assert_eq!(hits, vec!["pk-fable"]);
        let hits =
            addressed_agent_pubkeys("yo FABLE please", &[agent("pk-fable", &["Fable"], "yo")]);
        assert_eq!(hits, vec!["pk-fable"]);
    }

    #[test]
    fn multi_char_presets_match() {
        for kw in ["agent", "bot", "robo", "ok", "okay"] {
            let text = format!("{kw} Fable summarize");
            let hits = addressed_agent_pubkeys(&text, &[agent("pk-fable", &["Fable"], kw)]);
            assert_eq!(hits, vec!["pk-fable"], "kw={kw}");
        }
    }

    #[test]
    fn keyword_without_agent_name_does_not_tag() {
        let hits = addressed_agent_pubkeys("Hey the store later", &[fable()]);
        assert!(hits.is_empty());
        let hits =
            addressed_agent_pubkeys("we meet at noon", &[agent("pk-fable", &["Fable"], "at")]);
        assert!(hits.is_empty());
        let hits = addressed_agent_pubkeys("Hey", &[fable()]);
        assert!(hits.is_empty());
        let hits = addressed_agent_pubkeys("heythere rising", &[fable()]);
        assert!(hits.is_empty());
    }

    #[test]
    fn stt_without_keyword_name_logs_no_tags() {
        let agents = vec![fable(), claim_miner()];
        assert!(addressed_agent_pubkeys("sounds good everyone", &agents).is_empty());
        assert!(addressed_agent_pubkeys("Fable and ClaimMiner agree", &agents).is_empty());
    }

    #[test]
    fn multiword_alias() {
        let hits = addressed_agent_pubkeys("Hey Claim Miner, summarize please", &[claim_miner()]);
        assert_eq!(hits, vec!["pk-cm"]);
    }

    #[test]
    fn name_allows_trailing_punctuation() {
        for text in [
            "Hey Fable.",
            "Hey Fable!",
            "Hey Fable?",
            "Hey Fable:",
            "Hey Fable)",
        ] {
            let hits = addressed_agent_pubkeys(text, &[fable()]);
            assert_eq!(hits, vec!["pk-fable"], "text={text}");
        }
    }

    #[test]
    fn only_addressed_agent_tagged() {
        let agents = vec![fable(), claim_miner()];
        let hits = addressed_agent_pubkeys("Hey Fable take the lead", &agents);
        assert_eq!(hits, vec!["pk-fable"]);
    }

    #[test]
    fn per_agent_keywords_independent() {
        let agents = vec![
            agent("pk-fable", &["Fable"], "hey"),
            agent("pk-cm", &["Claim Miner", "ClaimMiner"], "yo"),
        ];
        assert_eq!(
            addressed_agent_pubkeys("Hey Fable go", &agents),
            vec!["pk-fable"]
        );
        assert_eq!(
            addressed_agent_pubkeys("Yo ClaimMiner go", &agents),
            vec!["pk-cm"]
        );
        assert!(addressed_agent_pubkeys("Hey ClaimMiner go", &agents).is_empty());
        assert!(addressed_agent_pubkeys("Yo Fable go", &agents).is_empty());
    }

    #[test]
    fn okay_fable_matches_ok_faybell_fuzzy() {
        // Agent configured as "okay" + "Fable"; STT heard "Ok Faybell".
        let agents = vec![agent("pk-fable", &["Fable"], "okay")];
        let hits = addressed_agent_pubkeys("Ok Faybell, what do you think?", &agents);
        assert_eq!(hits, vec!["pk-fable"]);
        // Reverse direction: agent "ok", STT "Okay Fable".
        let agents = vec![agent("pk-fable", &["Fable"], "ok")];
        let hits = addressed_agent_pubkeys("Okay Fable summarize", &agents);
        assert_eq!(hits, vec!["pk-fable"]);
    }

    #[test]
    fn fuzzy_name_requires_first_letter() {
        let agents = vec![agent("pk-fable", &["Fable"], "hey")];
        assert!(addressed_agent_pubkeys("Hey Maple please", &agents).is_empty());
    }

    #[test]
    fn fuzzy_does_not_easily_cross_wake_other_agent() {
        let agents = vec![
            agent("pk-fable", &["Fable"], "hey"),
            agent("pk-maple", &["Maple"], "hey"),
        ];
        assert_eq!(
            addressed_agent_pubkeys("Hey Fayble go", &agents),
            vec!["pk-fable"]
        );
        assert_eq!(
            addressed_agent_pubkeys("Hey Mayple go", &agents),
            vec!["pk-maple"]
        );
    }

    #[test]
    fn short_names_stay_exact() {
        let agents = vec![agent("pk-ab", &["Ab"], "hey")];
        assert_eq!(addressed_agent_pubkeys("Hey Ab go", &agents), vec!["pk-ab"]);
        assert!(addressed_agent_pubkeys("Hey Ac go", &agents).is_empty());
    }

    #[test]
    fn collect_record_aliases_dedups() {
        let a = collect_record_aliases("Fable", Some("fable"));
        assert_eq!(a, vec!["Fable"]);
        let b = collect_record_aliases("Fable", Some("Ship"));
        assert_eq!(b, vec!["Fable", "Ship"]);
    }

    #[test]
    fn aliases_from_managed_records_filters_roster() {
        let records = vec![
            ("pk-fable".into(), vec!["Fable".into()]),
            ("pk-other".into(), vec!["Other".into()]),
        ];
        let out = aliases_from_managed_records(&["pk-fable".into()], &records);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pubkey, "pk-fable");
        assert_eq!(out[0].aliases, vec!["Fable"]);
        assert_eq!(out[0].activation_keyword, "hey");
    }

    #[test]
    fn normalize_activation_keyword_allow_list() {
        assert_eq!(normalize_activation_keyword(" Hey "), Some("hey".into()));
        assert_eq!(normalize_activation_keyword("AT"), Some("at".into()));
        assert_eq!(normalize_activation_keyword("nope"), None);
        assert_eq!(normalize_activation_keyword(""), None);
    }

    #[test]
    fn compose_wake_content_includes_prior_lines() {
        let prior = vec![
            "we discussed the roadmap".into(),
            "then covered PRs from yesterday".into(),
        ];
        let out = compose_wake_content(&prior, "Hey Fable, summarize that");
        assert!(out.contains("[Huddle transcript — full meeting context so far]"));
        assert!(out.contains("we discussed the roadmap"));
        assert!(out.contains("then covered PRs from yesterday"));
        assert!(out.contains("[Addressed]"));
        assert!(out.contains("Hey Fable, summarize that"));
    }

    #[test]
    fn compose_wake_content_without_prior_is_plain() {
        assert_eq!(compose_wake_content(&[], "Hey Fable hi"), "Hey Fable hi");
    }

    #[test]
    fn merge_transcript_context_dedups_and_strips_wake() {
        let channel = vec!["hello everyone".into(), "roadmap next".into()];
        let session = vec!["roadmap next".into(), "one more thing".into()];
        let merged = merge_transcript_context(&channel, &session, "Hey Fable go");
        assert_eq!(
            merged,
            vec![
                "hello everyone".to_string(),
                "roadmap next".to_string(),
                "one more thing".to_string(),
            ]
        );
        let with_wake_dup = merge_transcript_context(
            &["earlier".into(), "Hey Fable go".into()],
            &[],
            "Hey Fable go",
        );
        assert_eq!(with_wake_dup, vec!["earlier".to_string()]);
    }

    #[test]
    fn unused_is_name_boundary_helper_still_covers_punct() {
        assert!(is_name_boundary(""));
        assert!(is_name_boundary(", more"));
        assert!(!is_name_boundary("more"));
    }
}
