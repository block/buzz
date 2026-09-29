//! STT wake gate: only p-tag / wake huddle agents when speech uses
//! the explicit phrase **"{activation} {agent name}"** (case-insensitive).
//!
//! Ordinary STT lines still post as kind:9 transcript messages, but with
//! **no** agent p-tags. Bare name address (`Fable, what do you think?`) does
//! **not** wake. Explicit chat @mentions continue to wake via the normal
//! composer send path (unchanged here).
//!
//! Default activation keyword is **"hey"** (friendlier than the legacy
//! hard-coded "at"). The live keyword is user-selectable in the huddle UI.

use std::collections::HashSet;

/// Default spoken activation keyword when the user has not chosen one.
pub const DEFAULT_ACTIVATION_KEYWORD: &str = "hey";

/// Short preset list shown in the huddle activation dropdown.
pub const ACTIVATION_KEYWORD_PRESETS: &[&str] =
    &["hey", "at", "agent", "bot", "robo", "ok", "yo", "okay"];

/// One huddle agent and the display names / aliases that can follow the
/// activation keyword.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNameAlias {
    pub pubkey: String,
    /// Non-empty display names / aliases (trimmed). Matched longest-first.
    pub aliases: Vec<String>,
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

/// Return pubkeys of huddle agents addressed by spoken
/// `{keyword} <Name>` in `text`.
///
/// Matching rules:
/// - Case-insensitive activation keyword at start-of-string or after whitespace.
/// - Immediately followed by whitespace, then an agent alias (longest first).
/// - Alias must end at a word boundary (end of string, whitespace, or
///   comma / punctuation like `,.;:!?)]}`).
/// - Bare agent name without leading keyword does **not** match.
/// - Keyword with no following huddle agent name does **not** match.
pub fn addressed_agent_pubkeys(
    text: &str,
    agents: &[AgentNameAlias],
    activation_keyword: &str,
) -> Vec<String> {
    if text.is_empty() || agents.is_empty() {
        return Vec::new();
    }
    let keyword = normalize_activation_keyword(activation_keyword)
        .unwrap_or_else(|| DEFAULT_ACTIVATION_KEYWORD.to_string());
    let keyword_lower = keyword.to_ascii_lowercase();
    if keyword_lower.is_empty() {
        return Vec::new();
    }

    let mut aliases: Vec<(&str, &str)> = Vec::new(); // (alias, pubkey)
    for agent in agents {
        let pk = agent.pubkey.trim();
        if pk.is_empty() {
            continue;
        }
        for alias in &agent.aliases {
            let a = alias.trim();
            if !a.is_empty() {
                aliases.push((a, pk));
            }
        }
    }
    if aliases.is_empty() {
        return Vec::new();
    }
    // Longest alias first so "hey Claim Miner" wins over "hey Claim".
    aliases.sort_by_key(|(a, _)| std::cmp::Reverse(a.len()));

    let lower = text.to_ascii_lowercase();
    let mut hit_pubkeys = Vec::new();
    let mut seen = HashSet::new();

    for (idx, _) in lower.match_indices(&keyword_lower) {
        if !is_keyword_token_start(&lower, idx, keyword_lower.len()) {
            continue;
        }
        let after_kw = idx + keyword_lower.len();
        let rest = &lower[after_kw..];
        let Some(name_start_rel) = rest.find(|c: char| !c.is_ascii_whitespace()) else {
            continue;
        };
        // Require at least one whitespace between keyword and the name.
        if name_start_rel == 0 {
            continue;
        }
        let name_region = &rest[name_start_rel..];
        if let Some((_, pubkey)) = aliases.iter().find(|(alias, _)| {
            let al = alias.to_ascii_lowercase();
            name_region
                .get(..al.len())
                .is_some_and(|s| s == al && is_name_boundary(&name_region[al.len()..]))
        }) {
            let pk = (*pubkey).to_string();
            if seen.insert(pk.to_ascii_lowercase()) {
                hit_pubkeys.push(pk);
            }
        }
    }

    hit_pubkeys
}

/// True when `keyword` at `idx` is a standalone token (start or after
/// whitespace, and not part of a longer word like "heythere" / "that").
fn is_keyword_token_start(lower: &str, idx: usize, keyword_len: usize) -> bool {
    let bytes = lower.as_bytes();
    if idx + keyword_len > bytes.len() {
        return false;
    }
    // Preceded by start or whitespace.
    if idx > 0 && !bytes[idx - 1].is_ascii_whitespace() {
        return false;
    }
    true
}

fn is_name_boundary(s: &str) -> bool {
    s.chars().next().is_none_or(|c| {
        c.is_ascii_whitespace() || matches!(c, ',' | ';' | '.' | '!' | '?' | ':' | ')' | ']' | '}')
    })
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

    fn fable() -> AgentNameAlias {
        AgentNameAlias {
            pubkey: "pk-fable".into(),
            aliases: vec!["Fable".into()],
        }
    }

    fn claim_miner() -> AgentNameAlias {
        AgentNameAlias {
            pubkey: "pk-cm".into(),
            aliases: vec!["Claim Miner".into(), "ClaimMiner".into()],
        }
    }

    #[test]
    fn hey_fable_tags_fable_default() {
        let hits = addressed_agent_pubkeys("Hey Fable, what do you think?", &[fable()], "hey");
        assert_eq!(hits, vec!["pk-fable"]);
    }

    #[test]
    fn at_fable_tags_when_keyword_is_at() {
        let hits = addressed_agent_pubkeys("At Fable, what do you think?", &[fable()], "at");
        assert_eq!(hits, vec!["pk-fable"]);
    }

    #[test]
    fn bare_fable_does_not_tag() {
        let hits = addressed_agent_pubkeys("Fable, what do you think?", &[fable()], "hey");
        assert!(hits.is_empty());
    }

    #[test]
    fn wrong_keyword_does_not_tag() {
        let hits = addressed_agent_pubkeys("At Fable please", &[fable()], "hey");
        assert!(hits.is_empty());
        let hits = addressed_agent_pubkeys("Hey Fable please", &[fable()], "at");
        assert!(hits.is_empty());
    }

    #[test]
    fn keyword_case_insensitive() {
        let hits = addressed_agent_pubkeys("HEY fable can you help", &[fable()], "hey");
        assert_eq!(hits, vec!["pk-fable"]);
        let hits = addressed_agent_pubkeys("yo FABLE please", &[fable()], "yo");
        assert_eq!(hits, vec!["pk-fable"]);
    }

    #[test]
    fn multi_char_presets_match() {
        for kw in ["agent", "bot", "robo", "ok", "okay"] {
            let text = format!("{kw} Fable summarize");
            let hits = addressed_agent_pubkeys(&text, &[fable()], kw);
            assert_eq!(hits, vec!["pk-fable"], "kw={kw}");
        }
    }

    #[test]
    fn keyword_without_agent_name_does_not_tag() {
        let hits = addressed_agent_pubkeys("Hey the store later", &[fable()], "hey");
        assert!(hits.is_empty());
        let hits = addressed_agent_pubkeys("we meet at noon", &[fable()], "at");
        assert!(hits.is_empty());
        let hits = addressed_agent_pubkeys("Hey", &[fable()], "hey");
        assert!(hits.is_empty());
        let hits = addressed_agent_pubkeys("heythere rising", &[fable()], "hey");
        assert!(hits.is_empty());
    }

    #[test]
    fn stt_without_keyword_name_logs_no_tags() {
        let agents = vec![fable(), claim_miner()];
        assert!(addressed_agent_pubkeys("sounds good everyone", &agents, "hey").is_empty());
        assert!(addressed_agent_pubkeys("Fable and ClaimMiner agree", &agents, "hey").is_empty());
    }

    #[test]
    fn multiword_alias() {
        let hits =
            addressed_agent_pubkeys("Hey Claim Miner, summarize please", &[claim_miner()], "hey");
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
            let hits = addressed_agent_pubkeys(text, &[fable()], "hey");
            assert_eq!(hits, vec!["pk-fable"], "text={text}");
        }
    }

    #[test]
    fn only_addressed_agent_tagged() {
        let agents = vec![fable(), claim_miner()];
        let hits = addressed_agent_pubkeys("Hey Fable take the lead", &agents, "hey");
        assert_eq!(hits, vec!["pk-fable"]);
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
}
