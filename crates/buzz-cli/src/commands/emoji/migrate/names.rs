//! Input parsing, shortcode normalization and alias resolution for
//! `emoji migrate plan`. Pure: no network, no filesystem writes.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

/// Longest alias chain followed before the name is rejected.
pub const MAX_ALIAS_DEPTH: usize = 5;

/// One name-to-value pair from the input before any validation. `value` is an
/// image source, or `alias:<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub name: String,
    pub value: String,
}

/// Where image sources may come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePolicy {
    /// `http://` and `https://` only (a Slack export is not trusted to name local files).
    Remote,
    /// Remote sources and `file://` paths (an operator-chosen directory).
    RemoteOrLocal,
}

/// A name that will be migrated, with the image source its alias chain ends at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub name: String,
    pub source: String,
    /// The name this one aliases, when it is an alias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_of: Option<String>,
}

/// Why a name is not migrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    InvalidName,
    DuplicateName,
    InvalidSource,
    AliasCycle,
    AliasTooDeep,
    AliasDangling,
    AliasToUnicode,
    ConflictSkipped,
    FetchFailed,
    InvalidImage,
    UploadFailed,
    PublishFailed,
    RetriesExhausted,
}

/// One line of `rejects.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reject {
    pub name: String,
    pub reason: RejectReason,
    pub detail: String,
}

/// Parse Slack `emoji.list` JSON: `{"emoji": {"name": "<url>" | "alias:<name>"}}`.
/// A top-level object without an `emoji` key is read as the map itself.
pub fn parse_slack_emoji_list(json: &str) -> Result<Vec<RawEntry>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("input is not valid JSON: {e}"))?;
    let map = match value.get("emoji") {
        Some(inner) => inner,
        None => &value,
    };
    let map = map
        .as_object()
        .ok_or_else(|| "expected a JSON object of emoji names".to_string())?;
    map.iter()
        .map(|(name, v)| {
            let value = v
                .as_str()
                .ok_or_else(|| format!("value for '{name}' is not a string"))?;
            Ok(RawEntry {
                name: name.clone(),
                value: value.to_string(),
            })
        })
        .collect()
}

enum Target {
    Direct(String),
    Alias(String),
}

fn source_allowed(source: &str, policy: SourcePolicy) -> bool {
    let remote = source.starts_with("http://") || source.starts_with("https://");
    let local = source.starts_with("file://") && source.len() > "file://".len();
    remote || (policy == SourcePolicy::RemoteOrLocal && local)
}

/// Normalize, de-duplicate and resolve aliases. Items are sorted by name.
/// `unicode_names` classifies alias targets that are standard emoji; without
/// it those targets are reported as dangling.
pub fn resolve_entries(
    raw: &[RawEntry],
    policy: SourcePolicy,
    unicode_names: &HashSet<String>,
) -> (Vec<PlanItem>, Vec<Reject>) {
    let mut rejects = Vec::new();
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    for entry in raw {
        let name = match buzz_sdk::normalize_custom_emoji_shortcode(&entry.name) {
            Ok(n) => n,
            Err(e) => {
                rejects.push(Reject {
                    name: entry.name.clone(),
                    reason: RejectReason::InvalidName,
                    detail: e.to_string(),
                });
                continue;
            }
        };
        if targets.contains_key(&name) {
            rejects.push(Reject {
                name: entry.name.clone(),
                reason: RejectReason::DuplicateName,
                detail: format!("normalizes to '{name}', which an earlier entry already uses"),
            });
            continue;
        }
        let target = match entry.value.strip_prefix("alias:") {
            Some(t) => Target::Alias(t.trim().to_string()),
            None if source_allowed(&entry.value, policy) => Target::Direct(entry.value.clone()),
            None => {
                rejects.push(Reject {
                    name: entry.name.clone(),
                    reason: RejectReason::InvalidSource,
                    detail: format!("unsupported image source '{}'", entry.value),
                });
                continue;
            }
        };
        targets.insert(name, target);
    }

    let mut items = Vec::new();
    for (name, target) in &targets {
        match follow(name, target, &targets, unicode_names) {
            Ok(item) => items.push(item),
            Err((reason, detail)) => rejects.push(Reject {
                name: name.clone(),
                reason,
                detail,
            }),
        }
    }
    (items, rejects)
}

fn follow(
    name: &str,
    first: &Target,
    targets: &BTreeMap<String, Target>,
    unicode_names: &HashSet<String>,
) -> Result<PlanItem, (RejectReason, String)> {
    let mut seen = HashSet::from([name.to_string()]);
    let mut alias_of: Option<String> = None;
    let mut hops = 0usize;
    let mut current = first;
    loop {
        match current {
            Target::Direct(source) => {
                return Ok(PlanItem {
                    name: name.to_string(),
                    source: source.clone(),
                    alias_of,
                })
            }
            Target::Alias(raw_target) => {
                hops += 1;
                if hops > MAX_ALIAS_DEPTH {
                    return Err((
                        RejectReason::AliasTooDeep,
                        format!("alias chain is longer than {MAX_ALIAS_DEPTH}"),
                    ));
                }
                let normalized = buzz_sdk::normalize_custom_emoji_shortcode(raw_target).ok();
                let next = normalized.as_deref().and_then(|t| targets.get(t));
                let (Some(target_name), Some(next)) = (normalized.as_deref(), next) else {
                    return Err(if unicode_names.contains(raw_target) {
                        (
                            RejectReason::AliasToUnicode,
                            format!("alias target '{raw_target}' is a standard Unicode emoji"),
                        )
                    } else {
                        (
                            RejectReason::AliasDangling,
                            format!(
                                "alias target '{raw_target}' is not a custom emoji in this input (dangling, or a standard Unicode name)"
                            ),
                        )
                    });
                };
                if !seen.insert(target_name.to_string()) {
                    return Err((
                        RejectReason::AliasCycle,
                        format!("alias chain returns to '{target_name}'"),
                    ));
                }
                alias_of.get_or_insert_with(|| target_name.to_string());
                current = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, value: &str) -> RawEntry {
        RawEntry {
            name: name.into(),
            value: value.into(),
        }
    }

    fn resolve(raw: &[RawEntry]) -> (Vec<PlanItem>, Vec<Reject>) {
        resolve_entries(raw, SourcePolicy::Remote, &HashSet::new())
    }

    fn reasons(rejects: &[Reject]) -> Vec<(&str, RejectReason)> {
        rejects
            .iter()
            .map(|r| (r.name.as_str(), r.reason))
            .collect()
    }

    #[test]
    fn parses_slack_wrapper_and_bare_map() {
        let wrapped = r#"{"ok":true,"emoji":{"a":"https://x/a.png","b":"alias:a"}}"#;
        let bare = r#"{"a":"https://x/a.png","b":"alias:a"}"#;
        assert_eq!(
            parse_slack_emoji_list(wrapped).unwrap(),
            parse_slack_emoji_list(bare).unwrap()
        );
        assert!(parse_slack_emoji_list("[1]").is_err());
        assert!(parse_slack_emoji_list(r#"{"emoji":{"a":5}}"#).is_err());
        assert!(parse_slack_emoji_list("nope").is_err());
    }

    #[test]
    fn normalization_table() {
        let long = "a".repeat(65);
        let ok64 = "a".repeat(64);
        let cases: Vec<(&str, Option<&str>)> = vec![
            ("party_parrot", Some("party_parrot")),
            ("Party-Parrot", Some("party-parrot")),
            (":wave:", Some("wave")),
            ("  wave  ", Some("wave")),
            ("a1_b-2", Some("a1_b-2")),
            (ok64.as_str(), Some(ok64.as_str())),
            (long.as_str(), None),
            ("", None),
            ("+1", None),
            ("don't", None),
            ("two words", None),
            ("café", None),
            ("😀", None),
        ];
        for (input, expected) in cases {
            let (items, rejects) = resolve(&[entry(input, "https://x/i.png")]);
            match expected {
                Some(name) => {
                    assert_eq!(items.len(), 1, "{input:?} should be accepted");
                    assert_eq!(items[0].name, name, "{input:?}");
                }
                None => {
                    assert!(items.is_empty(), "{input:?} should be rejected");
                    assert_eq!(rejects[0].reason, RejectReason::InvalidName, "{input:?}");
                }
            }
        }
    }

    #[test]
    fn names_that_collide_after_normalization_keep_the_first() {
        let (items, rejects) = resolve(&[
            entry("Wave", "https://x/1.png"),
            entry("wave", "https://x/2.png"),
        ]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, "https://x/1.png");
        assert_eq!(
            reasons(&rejects),
            vec![("wave", RejectReason::DuplicateName)]
        );
    }

    #[test]
    fn aliases_resolve_to_a_second_entry_with_the_same_source() {
        let (items, rejects) = resolve(&[
            entry("c", "alias:b"),
            entry("b", "alias:a"),
            entry("a", "https://x/a.png"),
        ]);
        assert!(rejects.is_empty());
        let by_name: Vec<_> = items
            .iter()
            .map(|i| (i.name.as_str(), i.source.as_str(), i.alias_of.as_deref()))
            .collect();
        assert_eq!(
            by_name,
            vec![
                ("a", "https://x/a.png", None),
                ("b", "https://x/a.png", Some("a")),
                ("c", "https://x/a.png", Some("b")),
            ]
        );
    }

    #[test]
    fn alias_depth_limit_is_five_hops() {
        let mut raw = vec![entry("n0", "https://x/0.png")];
        for i in 1..=6 {
            raw.push(entry(&format!("n{i}"), &format!("alias:n{}", i - 1)));
        }
        let (items, rejects) = resolve(&raw);
        let ok: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(ok, vec!["n0", "n1", "n2", "n3", "n4", "n5"]);
        assert_eq!(reasons(&rejects), vec![("n6", RejectReason::AliasTooDeep)]);
    }

    #[test]
    fn alias_cycles_are_rejected_for_every_member() {
        let (items, rejects) = resolve(&[
            entry("a", "alias:b"),
            entry("b", "alias:c"),
            entry("c", "alias:a"),
            entry("self", "alias:self"),
            entry("tail", "alias:a"),
        ]);
        assert!(items.is_empty());
        assert_eq!(
            reasons(&rejects),
            vec![
                ("a", RejectReason::AliasCycle),
                ("b", RejectReason::AliasCycle),
                ("c", RejectReason::AliasCycle),
                ("self", RejectReason::AliasCycle),
                ("tail", RejectReason::AliasCycle),
            ]
        );
    }

    #[test]
    fn dangling_and_unicode_aliases_are_reported_separately() {
        let raw = [
            entry("ghost", "alias:missing"),
            entry("thumb", "alias:thumbsup"),
            entry("bad", "alias:not valid!"),
        ];
        let unicode = HashSet::from(["thumbsup".to_string()]);
        let (_, rejects) = resolve_entries(&raw, SourcePolicy::Remote, &unicode);
        assert_eq!(
            reasons(&rejects),
            vec![
                ("bad", RejectReason::AliasDangling),
                ("ghost", RejectReason::AliasDangling),
                ("thumb", RejectReason::AliasToUnicode),
            ]
        );
        let (_, without_list) = resolve(&raw);
        assert!(without_list
            .iter()
            .all(|r| r.reason == RejectReason::AliasDangling));
    }

    #[test]
    fn alias_to_a_rejected_name_is_dangling() {
        let (items, rejects) = resolve(&[entry("a b", "https://x/a.png"), entry("c", "alias:a b")]);
        assert!(items.is_empty());
        assert_eq!(
            reasons(&rejects),
            vec![
                ("a b", RejectReason::InvalidName),
                ("c", RejectReason::AliasDangling)
            ]
        );
    }

    #[test]
    fn source_policy_blocks_local_files_from_remote_inputs() {
        let raw = [
            entry("local", "file:///tmp/a.png"),
            entry("ftp", "ftp://x/a.png"),
            entry("web", "https://x/a.png"),
            entry("empty", "file://"),
        ];
        let (items, rejects) = resolve_entries(&raw, SourcePolicy::Remote, &HashSet::new());
        assert_eq!(
            items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["web"]
        );
        assert!(rejects
            .iter()
            .all(|r| r.reason == RejectReason::InvalidSource));
        let (items, _) = resolve_entries(&raw, SourcePolicy::RemoteOrLocal, &HashSet::new());
        assert_eq!(
            items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["local", "web"]
        );
    }
}
