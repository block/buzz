//! Selections: frozen-or-live descriptions of a set of signals.

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::signal::Signal;

/// Kinds a selection reads when none are specified: chat messages.
pub const DEFAULT_SIGNAL_KINDS: &[u32] = &[9];

/// A named-able description of a set of signals: who × what × when.
///
/// The *when* is part of the selection. A pinned `until_exclusive` makes the
/// selection **frozen** — it describes a fixed set that a fold works through
/// and is then done with forever. An open `until_exclusive` makes it **live**
/// — it keeps extending to "now", so a fold over it is never done.
///
/// v1 supports the shapes the mirror can answer directly: channels, authors,
/// and kinds. DMs are deliberately excluded — a selection never reads them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// Channel ids (NIP-29 `h` tags).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    /// Author pubkeys (hex).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// Thread anchors: 64-hex event ids, each selecting itself plus every
    /// descendant reply at any depth. Joins `channels` as a scope union — an
    /// event is in scope when it is in a listed channel OR a listed subtree.
    /// An anchor need not be a thread root: a mid-thread id selects that
    /// sub-subtree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<String>,
    /// Event kinds; empty means [`DEFAULT_SIGNAL_KINDS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<u32>,
    /// Tag predicates: exact-match `[name, value]` pairs against the event's
    /// own tag list, verbatim and case-sensitive. Pairs sharing a name OR
    /// together; distinct names AND together (NIP-01 filter semantics). Tags
    /// only NARROW — they never grant scope, so a selection still needs at
    /// least one channel, author, or thread.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<(String, String)>,
    /// Window start (unix seconds, inclusive). `None` = beginning of time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    /// Window end (unix seconds, exclusive). `None` = live: the selection
    /// keeps extending to "now". Pinned = frozen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_exclusive: Option<i64>,
}

impl Selection {
    /// Validate and canonicalize (lowercase + sort + dedupe) in place, so
    /// equal selections serialize identically and case variants of the same
    /// id cannot silently select nothing.
    pub fn canonicalize(&mut self) -> Result<(), Error> {
        for c in &mut self.channels {
            *c = c.trim().to_ascii_lowercase();
        }
        for a in &mut self.authors {
            *a = a.trim().to_ascii_lowercase();
        }
        for t in &mut self.threads {
            *t = t.trim().to_ascii_lowercase();
        }
        self.channels.sort();
        self.channels.dedup();
        self.authors.sort();
        self.authors.dedup();
        self.threads.sort();
        self.threads.dedup();
        self.kinds.sort_unstable();
        self.kinds.dedup();
        // Tag pairs stay verbatim (values are case-sensitive by contract);
        // sorting + deduping still makes equal predicates serialize identically.
        self.tags.sort();
        self.tags.dedup();
        if self.channels.is_empty() && self.authors.is_empty() && self.threads.is_empty() {
            return Err(Error::InvalidSpec(
                "selection must name at least one channel, author, or thread".into(),
            ));
        }
        if let Some(c) = self.channels.iter().find(|c| !is_uuid(c)) {
            return Err(Error::InvalidSpec(format!(
                "channel {c:?} is not a channel UUID"
            )));
        }
        if let Some(a) = self.authors.iter().find(|a| !is_hex_pubkey(a)) {
            return Err(Error::InvalidSpec(format!(
                "author {a:?} is not a 64-hex pubkey"
            )));
        }
        if let Some(t) = self.threads.iter().find(|t| !is_hex_id(t)) {
            return Err(Error::InvalidSpec(format!(
                "thread {t:?} is not a 64-hex event id"
            )));
        }
        if let Some(k) = self.kinds.iter().find(|k| **k > u16::MAX as u32) {
            return Err(Error::InvalidSpec(format!(
                "kind {k} is outside the event-kind range"
            )));
        }
        validate_tag_pairs(&self.tags)?;
        if let (Some(s), Some(u)) = (self.since, self.until_exclusive) {
            if s >= u {
                return Err(Error::InvalidSpec(format!(
                    "window is empty: since {s} must be before until_exclusive {u}"
                )));
            }
        }
        Ok(())
    }

    /// Whether the selection describes a fixed set (pinned end) rather than a
    /// live one that keeps extending to "now".
    pub fn is_frozen(&self) -> bool {
        self.until_exclusive.is_some()
    }

    /// Effective kinds for signal queries (queries must always name kinds).
    pub fn effective_kinds(&self) -> Vec<u32> {
        if self.kinds.is_empty() {
            DEFAULT_SIGNAL_KINDS.to_vec()
        } else {
            self.kinds.clone()
        }
    }

    /// The concrete half-open window `[since, until_exclusive)` a run over
    /// this selection reads, optionally narrowed by a caller clamp.
    ///
    /// The clamp is how a run stays pinned to its priced preflight window: a
    /// live selection resolves `until` to "now" at preflight time, and the run
    /// passes that resolved window back as the clamp. Clamps can only narrow —
    /// the intersection with the selection's own window is taken, so a frozen
    /// selection never reads outside its freeze.
    pub fn resolve_window(
        &self,
        clamp_since: Option<i64>,
        clamp_until_exclusive: Option<i64>,
        now: i64,
    ) -> (i64, i64) {
        let since = self.since.unwrap_or(0).max(clamp_since.unwrap_or(0));
        let until = self
            .until_exclusive
            .unwrap_or(now + 1)
            .min(clamp_until_exclusive.unwrap_or(i64::MAX));
        (since, until)
    }
}

/// Most tag pairs one predicate (or one publish) may carry.
pub const MAX_TAG_PAIRS: usize = 16;
/// Longest tag name accepted.
pub const MAX_TAG_NAME_LEN: usize = 64;
/// Longest tag value accepted.
pub const MAX_TAG_VALUE_LEN: usize = 256;

/// Shape rules shared by selection predicates and publish tags: bounded
/// count, non-empty name without whitespace, non-empty bounded value.
pub fn validate_tag_pairs(tags: &[(String, String)]) -> Result<(), Error> {
    if tags.len() > MAX_TAG_PAIRS {
        return Err(Error::InvalidSpec(format!(
            "at most {MAX_TAG_PAIRS} tag pairs are supported, got {}",
            tags.len()
        )));
    }
    for (name, value) in tags {
        if name.is_empty()
            || name.len() > MAX_TAG_NAME_LEN
            || name.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(Error::InvalidSpec(format!(
                "tag name {name:?} must be 1..={MAX_TAG_NAME_LEN} chars with no whitespace"
            )));
        }
        if value.is_empty() || value.len() > MAX_TAG_VALUE_LEN {
            return Err(Error::InvalidSpec(format!(
                "tag value for {name:?} must be 1..={MAX_TAG_VALUE_LEN} chars"
            )));
        }
    }
    Ok(())
}

/// Lowercase hyphenated UUID: 8-4-4-4-12 hex digits.
fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit() && !c.is_ascii_uppercase(),
        })
}

/// Lowercase 64-hex pubkey.
fn is_hex_pubkey(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

/// Lowercase 64-hex event id (same alphabet as a pubkey).
fn is_hex_id(s: &str) -> bool {
    is_hex_pubkey(s)
}

/// Deterministically materialize fetched signals: `created_at ASC, id ASC`,
/// duplicates (same id) removed.
///
/// The same fetched set always materializes to the identical ordered list, so
/// (selection, window) pins exactly which signals a run saw.
pub fn materialize(mut signals: Vec<Signal>) -> Vec<Signal> {
    signals.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    signals.dedup_by(|a, b| a.id == b.id);
    signals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(id: &str, ts: i64) -> Signal {
        Signal {
            id: id.into(),
            pubkey: "pk".into(),
            kind: 9,
            created_at: ts,
            content: "x".into(),
            channel: None,
        }
    }

    #[test]
    fn materialize_orders_created_at_then_id_and_dedupes() {
        let out = materialize(vec![sig("bb", 5), sig("aa", 5), sig("cc", 1), sig("aa", 5)]);
        let ids: Vec<&str> = out.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["cc", "aa", "bb"]);
    }

    const CH_A: &str = "59ca5528-71ea-4a53-a7f5-90c9fb2b1729";
    const CH_B: &str = "a2228687-0000-4000-8000-000000000000";

    fn channels(ids: &[&str]) -> Selection {
        Selection {
            channels: ids.iter().map(|s| s.to_string()).collect(),
            ..Selection::default()
        }
    }

    #[test]
    fn empty_selection_is_rejected() {
        let mut s = Selection::default();
        assert!(s.canonicalize().is_err());
    }

    #[test]
    fn canonicalize_lowercases_sorts_and_dedupes() {
        let mut s = Selection {
            kinds: vec![40002, 9, 9],
            ..channels(&[CH_B, &CH_A.to_uppercase(), CH_B])
        };
        s.canonicalize().expect("valid");
        assert_eq!(s.channels, vec![CH_A.to_string(), CH_B.to_string()]);
        assert_eq!(s.kinds, vec![9, 40002]);
    }

    #[test]
    fn malformed_ids_and_out_of_range_kinds_are_rejected() {
        assert!(channels(&["not-a-uuid"]).canonicalize().is_err());
        let mut s = Selection {
            authors: vec!["abc123".into()],
            ..Selection::default()
        };
        assert!(s.canonicalize().is_err());
        let mut s = Selection {
            kinds: vec![70_000],
            ..channels(&[CH_A])
        };
        assert!(s.canonicalize().is_err());
    }

    #[test]
    fn empty_window_is_rejected_but_open_ends_pass() {
        let mut s = Selection {
            since: Some(200),
            until_exclusive: Some(200),
            ..channels(&[CH_A])
        };
        assert!(s.canonicalize().is_err());
        let mut s = Selection {
            since: Some(100),
            until_exclusive: Some(200),
            ..channels(&[CH_A])
        };
        assert!(s.canonicalize().is_ok());
        assert!(s.is_frozen());
        let mut s = Selection {
            since: Some(100),
            ..channels(&[CH_A])
        };
        assert!(s.canonicalize().is_ok());
        assert!(!s.is_frozen());
    }

    #[test]
    fn live_selection_resolves_to_now_and_clamps_narrow() {
        let s = channels(&[CH_A]);
        assert_eq!(s.resolve_window(None, None, 1000), (0, 1001));
        // The run-pins-to-priced-window clamp.
        assert_eq!(s.resolve_window(Some(10), Some(900), 1000), (10, 900));
    }

    #[test]
    fn frozen_selection_never_reads_outside_its_freeze() {
        let s = Selection {
            since: Some(100),
            until_exclusive: Some(200),
            ..channels(&[CH_A])
        };
        // No clamp: the freeze is the window, even as "now" moves on.
        assert_eq!(s.resolve_window(None, None, 10_000), (100, 200));
        // A wider clamp cannot widen; a narrower one narrows.
        assert_eq!(s.resolve_window(Some(0), Some(10_000), 10_000), (100, 200));
        assert_eq!(s.resolve_window(Some(150), Some(180), 10_000), (150, 180));
    }

    #[test]
    fn threads_clause_canonicalizes_and_satisfies_the_scope_rule() {
        let root = "F".repeat(64);
        let mut s = Selection {
            threads: vec![root.clone(), root.to_ascii_lowercase()],
            ..Selection::default()
        };
        s.canonicalize().expect("threads alone are a valid scope");
        assert_eq!(s.threads, vec![root.to_ascii_lowercase()]);

        let mut bad = Selection {
            threads: vec!["not-an-id".into()],
            ..Selection::default()
        };
        assert!(bad.canonicalize().is_err());
    }

    #[test]
    fn tag_predicates_canonicalize_but_stay_case_sensitive() {
        let mut s = Selection {
            tags: vec![
                ("acc.recipe".into(), "Weekly".into()),
                ("acc.recipe".into(), "Weekly".into()),
                ("acc.interval".into(), "2026-W36".into()),
            ],
            ..channels(&[CH_A])
        };
        s.canonicalize().expect("valid");
        // Sorted + deduped, values verbatim — "Weekly" is NOT lowercased.
        assert_eq!(
            s.tags,
            vec![
                ("acc.interval".to_string(), "2026-W36".to_string()),
                ("acc.recipe".to_string(), "Weekly".to_string()),
            ]
        );
    }

    #[test]
    fn tags_never_grant_scope_and_bad_shapes_are_rejected() {
        // Tags alone are not a scope.
        let mut s = Selection {
            tags: vec![("t".into(), "x".into())],
            ..Selection::default()
        };
        assert!(s.canonicalize().is_err());
        // Empty / whitespace names, empty values, and oversize are rejected.
        for tags in [
            vec![(String::new(), "x".into())],
            vec![("a b".into(), "x".into())],
            vec![("t".into(), String::new())],
            vec![("t".into(), "v".repeat(MAX_TAG_VALUE_LEN + 1))],
        ] {
            let mut s = Selection {
                tags,
                ..channels(&[CH_A])
            };
            assert!(s.canonicalize().is_err());
        }
        let mut s = Selection {
            tags: (0..=MAX_TAG_PAIRS)
                .map(|i| ("t".to_string(), format!("v{i}")))
                .collect(),
            ..channels(&[CH_A])
        };
        assert!(s.canonicalize().is_err(), "pair cap enforced");
    }

    #[test]
    fn selection_json_without_tags_still_loads_and_compares_equal() {
        // Specs saved before the tags clause must keep loading and stay
        // cache-compatible with a tag-less selection.
        let legacy = format!(r#"{{"channels":["{CH_A}"]}}"#);
        let loaded: Selection = serde_json::from_str(&legacy).expect("deserialize");
        assert_eq!(loaded, channels(&[CH_A]));
        let out = serde_json::to_string(&loaded).expect("serialize");
        assert!(!out.contains("tags"), "empty tags must not serialize");
        // The wire shape round-trips as Nostr-style pairs.
        let tagged: Selection = serde_json::from_str(&format!(
            r#"{{"channels":["{CH_A}"],"tags":[["acc.recipe","weekly"]]}}"#
        ))
        .expect("deserialize");
        assert_eq!(tagged.tags, vec![("acc.recipe".into(), "weekly".into())]);
    }

    #[test]
    fn selection_json_without_threads_still_loads_and_compares_equal() {
        // Specs saved before the threads clause must keep loading and stay
        // cache-compatible with a threads-less selection.
        let legacy = format!(r#"{{"channels":["{CH_A}"]}}"#);
        let loaded: Selection = serde_json::from_str(&legacy).expect("deserialize");
        assert_eq!(loaded, channels(&[CH_A]));
        let out = serde_json::to_string(&loaded).expect("serialize");
        assert!(!out.contains("threads"), "empty threads must not serialize");
    }

    #[test]
    fn selection_json_without_window_still_loads_as_live() {
        // Specs saved before the when moved into the selection must keep
        // loading — and must compare equal to a live selection, so existing
        // chains stay cached.
        let legacy = format!(r#"{{"channels":["{CH_A}"]}}"#);
        let loaded: Selection = serde_json::from_str(&legacy).expect("deserialize");
        assert_eq!(loaded, channels(&[CH_A]));
        assert!(!loaded.is_frozen());
        let out = serde_json::to_string(&loaded).expect("serialize");
        assert!(!out.contains("until"), "open ends must not serialize");
    }
}
