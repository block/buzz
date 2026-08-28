//! Selections: saveable queries over signals, compiled to relay filters.

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::signal::Signal;

/// Kinds a selection reads when none are specified: chat messages.
pub const DEFAULT_SIGNAL_KINDS: &[u32] = &[9];

/// A named-able query over signals.
///
/// v1 supports the shapes the relay can answer directly: channels (`#h`),
/// authors, and kinds. DMs are deliberately excluded — a selection never
/// reads them. Time is not part of the selection; each run supplies its own
/// half-open window so the same selection replays deterministically.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// Channel ids (NIP-29 `h` tags). One relay filter is emitted per channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<String>,
    /// Author pubkeys (hex).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    /// Event kinds; empty means [`DEFAULT_SIGNAL_KINDS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<u32>,
}

impl Selection {
    /// Validate and canonicalize (sort + dedupe) in place, so equal selections
    /// serialize identically.
    pub fn canonicalize(&mut self) -> Result<(), Error> {
        self.channels.sort();
        self.channels.dedup();
        self.authors.sort();
        self.authors.dedup();
        self.kinds.sort_unstable();
        self.kinds.dedup();
        if self.channels.is_empty() && self.authors.is_empty() {
            return Err(Error::InvalidSpec(
                "selection must name at least one channel or author".into(),
            ));
        }
        if self.channels.iter().any(|c| c.trim().is_empty()) {
            return Err(Error::InvalidSpec(
                "selection has an empty channel id".into(),
            ));
        }
        if self.authors.iter().any(|a| a.trim().is_empty()) {
            return Err(Error::InvalidSpec(
                "selection has an empty author pubkey".into(),
            ));
        }
        Ok(())
    }

    /// Effective kinds for relay filters (relay queries must always name kinds).
    pub fn effective_kinds(&self) -> Vec<u32> {
        if self.kinds.is_empty() {
            DEFAULT_SIGNAL_KINDS.to_vec()
        } else {
            self.kinds.clone()
        }
    }

    /// Compile to NIP-01 relay filters over the half-open window
    /// `[since, until_exclusive)`.
    ///
    /// One filter is emitted **per channel** rather than a single multi-`#h`
    /// filter: per-channel requests match the relay's fan-out expectations and
    /// avoid union-filter edge cases. Nostr `since`/`until` are inclusive, so
    /// the exclusive end is mapped to `until_exclusive - 1`.
    pub fn compile_filters(
        &self,
        since: i64,
        until_exclusive: i64,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        let kinds = self.effective_kinds();
        let until = until_exclusive.saturating_sub(1);
        let base = |extra: &mut serde_json::Map<String, serde_json::Value>| {
            extra.insert("kinds".into(), serde_json::json!(kinds));
            extra.insert("since".into(), serde_json::json!(since));
            extra.insert("until".into(), serde_json::json!(until));
            extra.insert("limit".into(), serde_json::json!(limit));
            if !self.authors.is_empty() {
                extra.insert("authors".into(), serde_json::json!(self.authors));
            }
        };
        if self.channels.is_empty() {
            let mut m = serde_json::Map::new();
            base(&mut m);
            return vec![serde_json::Value::Object(m)];
        }
        self.channels
            .iter()
            .map(|ch| {
                let mut m = serde_json::Map::new();
                base(&mut m);
                m.insert("#h".into(), serde_json::json!([ch]));
                serde_json::Value::Object(m)
            })
            .collect()
    }

    /// Human-readable one-line description.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.channels.is_empty() {
            parts.push(format!("{} channel(s)", self.channels.len()));
        }
        if !self.authors.is_empty() {
            parts.push(format!("{} author(s)", self.authors.len()));
        }
        parts.push(format!("kinds {:?}", self.effective_kinds()));
        parts.join(" · ")
    }
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

    #[test]
    fn empty_selection_is_rejected() {
        let mut s = Selection::default();
        assert!(s.canonicalize().is_err());
    }

    #[test]
    fn canonicalize_sorts_and_dedupes() {
        let mut s = Selection {
            channels: vec!["b".into(), "a".into(), "b".into()],
            authors: vec![],
            kinds: vec![40002, 9, 9],
        };
        s.canonicalize().expect("valid");
        assert_eq!(s.channels, vec!["a", "b"]);
        assert_eq!(s.kinds, vec![9, 40002]);
    }

    #[test]
    fn one_filter_per_channel_with_half_open_window() {
        let s = Selection {
            channels: vec!["ch1".into(), "ch2".into()],
            authors: vec!["p1".into()],
            kinds: vec![],
        };
        let filters = s.compile_filters(100, 200, 500);
        assert_eq!(filters.len(), 2);
        assert_eq!(filters[0]["#h"], serde_json::json!(["ch1"]));
        assert_eq!(filters[0]["kinds"], serde_json::json!([9]));
        assert_eq!(filters[0]["since"], serde_json::json!(100));
        assert_eq!(filters[0]["until"], serde_json::json!(199));
        assert_eq!(filters[0]["authors"], serde_json::json!(["p1"]));
    }

    #[test]
    fn author_only_selection_compiles_one_filter() {
        let s = Selection {
            channels: vec![],
            authors: vec!["p1".into()],
            kinds: vec![9],
        };
        let filters = s.compile_filters(0, 10, 5);
        assert_eq!(filters.len(), 1);
        assert!(filters[0].get("#h").is_none());
    }
}
