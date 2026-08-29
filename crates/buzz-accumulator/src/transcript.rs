//! Transcript rendering: whole recent signals under a hard character budget.
//!
//! The budget is filled newest-first so the freshest evidence always makes the
//! cut, then re-emitted in time order. The render reports the *exact* signals
//! shown — that list is what coverage may seal. A signal that did not fit is
//! simply not shown and stays pending; it is never summarized invisibly.

use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::signal::Signal;

/// Identity and timestamp of one signal actually included in a transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShownSignal {
    /// Event id.
    pub id: String,
    /// Unix seconds, used for the coverage window.
    pub created_at: i64,
}

/// A rendered transcript plus the honesty bookkeeping around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptRender {
    /// Time-ordered transcript body (possibly truncated, with a visible marker).
    pub body: String,
    /// True when any candidate signal was dropped or tail-trimmed to fit.
    pub truncated: bool,
    /// Exactly the signals whose text appears in `body`, in body order.
    pub shown: Vec<ShownSignal>,
}

/// Human label for an event kind in transcript lines.
fn kind_label(kind: u32) -> String {
    if kind == 9 {
        "message".to_string()
    } else {
        format!("kind-{kind}")
    }
}

/// Render one signal as a single transcript line:
/// `[YYYY-MM-DD HH:MM] {who} <{kind}>: {whitespace-normalized content}`.
fn render_line(signal: &Signal, names: &BTreeMap<String, String>) -> String {
    let iso = match Utc.timestamp_opt(signal.created_at, 0).single() {
        Some(ts) => ts.format("%Y-%m-%d %H:%M").to_string(),
        None => format!("ts:{}", signal.created_at),
    };
    let who = names
        .get(&signal.pubkey)
        .cloned()
        .unwrap_or_else(|| signal.pubkey.chars().take(8).collect());
    let text = signal
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!("[{iso}] {who} <{}>: {text}", kind_label(signal.kind))
}

/// Smallest tail of an oversized line worth showing the model. A shorter
/// sliver (a few characters of a long message) is not honest evidence — the
/// line is dropped and stays pending rather than being sealed as covered.
pub const MIN_TAIL_CHARS: usize = 256;

/// Render `signals` (already materialized: time-ordered, deduped) into a
/// transcript of at most `max_chars` characters and at most `max_items`
/// lines, newest-first fill.
///
/// If even the single newest line exceeds the budget, its identity is kept
/// with a `…`-prefixed tail of at least [`MIN_TAIL_CHARS`] — when the budget
/// cannot afford even that, the line is dropped (stays pending) rather than
/// sealed on a sliver.
pub fn render_transcript(
    signals: &[Signal],
    names: &BTreeMap<String, String>,
    max_chars: usize,
    max_items: usize,
) -> TranscriptRender {
    let rendered: Vec<(ShownSignal, String)> = signals
        .iter()
        .map(|s| {
            (
                ShownSignal {
                    id: s.id.clone(),
                    created_at: s.created_at,
                },
                render_line(s, names),
            )
        })
        .collect();
    let mut kept: Vec<(ShownSignal, String)> = Vec::new();
    let mut used = 0usize;
    let mut trimmed = false;
    for (shown, line) in rendered.iter().rev() {
        if kept.len() >= max_items {
            break;
        }
        let mut line = line.clone();
        let mut cost = line.chars().count() + usize::from(!kept.is_empty());
        if !kept.is_empty() && used + cost > max_chars {
            break;
        }
        if kept.is_empty() && cost > max_chars {
            if max_chars < MIN_TAIL_CHARS {
                break;
            }
            let tail_start = line.chars().count() - (max_chars - 1);
            line = format!("…{}", line.chars().skip(tail_start).collect::<String>());
            cost = line.chars().count();
            trimmed = true;
        }
        kept.push((shown.clone(), line));
        used += cost;
    }
    kept.reverse();
    let truncated = trimmed || kept.len() != rendered.len();
    let mut body = kept
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if truncated {
        body = if body.is_empty() {
            "[…older events truncated to fit context budget…]".to_string()
        } else {
            format!("[…older events truncated to fit context budget…]\n{body}")
        };
    }
    TranscriptRender {
        body,
        truncated,
        shown: kept.into_iter().map(|(shown, _)| shown).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(id: &str, ts: i64, content: &str) -> Signal {
        Signal {
            id: id.to_string(),
            pubkey: "aabbccddeeff0011".to_string(),
            kind: 9,
            created_at: ts,
            content: content.to_string(),
            channel: None,
        }
    }

    #[test]
    fn renders_all_when_budget_allows() {
        let names = BTreeMap::from([("aabbccddeeff0011".to_string(), "riley".to_string())]);
        let signals = vec![
            sig("e1", 1_700_000_000, "hello  there"),
            sig("e2", 1_700_000_060, "hi"),
        ];
        let r = render_transcript(&signals, &names, 10_000, usize::MAX);
        assert!(!r.truncated);
        assert_eq!(r.shown.len(), 2);
        assert!(r.body.contains("riley <message>: hello there"));
        assert!(r.body.lines().count() == 2);
    }

    #[test]
    fn drops_oldest_first_and_marks_truncation() {
        let names = BTreeMap::new();
        let signals = vec![sig("e1", 100, "old old old"), sig("e2", 200, "new")];
        let newest_len = render_transcript(&signals[1..], &names, 10_000, usize::MAX)
            .body
            .chars()
            .count();
        let r = render_transcript(&signals, &names, newest_len, usize::MAX);
        assert!(r.truncated);
        assert_eq!(
            r.shown.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["e2"]
        );
        assert!(r.body.starts_with("[…older events truncated"));
        assert!(r.body.contains("new"));
        assert!(!r.body.contains("old old old"));
    }

    #[test]
    fn oversized_single_line_keeps_identity_with_bounded_tail() {
        let names = BTreeMap::new();
        let signals = vec![sig("e1", 100, &"x".repeat(5_000))];
        let r = render_transcript(&signals, &names, 400, usize::MAX);
        assert!(r.truncated);
        assert_eq!(r.shown.len(), 1);
        let last_line = r.body.lines().last().unwrap_or("");
        assert!(last_line.starts_with('…'));
        assert_eq!(last_line.chars().count(), 400);
    }

    #[test]
    fn sliver_budget_drops_the_line_instead_of_sealing_it() {
        // A tail below MIN_TAIL_CHARS is not honest evidence: show nothing,
        // seal nothing — the event stays pending.
        let names = BTreeMap::new();
        let signals = vec![sig("e1", 100, &"x".repeat(5_000))];
        let r = render_transcript(&signals, &names, MIN_TAIL_CHARS - 1, usize::MAX);
        assert!(r.truncated);
        assert!(r.shown.is_empty());
        assert_eq!(r.body, "[…older events truncated to fit context budget…]");
    }

    #[test]
    fn item_cap_keeps_newest_and_marks_truncation() {
        let names = BTreeMap::new();
        let signals: Vec<Signal> = (0..10)
            .map(|i| sig(&format!("e{i}"), 100 + i, "m"))
            .collect();
        let r = render_transcript(&signals, &names, 10_000, 3);
        assert!(r.truncated);
        assert_eq!(
            r.shown.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["e7", "e8", "e9"]
        );
    }

    #[test]
    fn zero_budget_shows_nothing_and_says_so() {
        let names = BTreeMap::new();
        let signals = vec![sig("e1", 100, "hello")];
        let r = render_transcript(&signals, &names, 0, usize::MAX);
        assert!(r.truncated);
        assert!(r.shown.is_empty());
        assert_eq!(r.body, "[…older events truncated to fit context budget…]");
    }

    #[test]
    fn unknown_author_falls_back_to_pubkey_prefix_and_kind_labelled() {
        let names = BTreeMap::new();
        let mut s = sig("e1", 1_700_000_000, "deployed");
        s.kind = 40002;
        let r = render_transcript(&[s], &names, 10_000, usize::MAX);
        assert!(r.body.contains("aabbccdd <kind-40002>:"));
    }

    #[test]
    fn empty_input_is_empty_and_honest() {
        let r = render_transcript(&[], &BTreeMap::new(), 100, usize::MAX);
        assert_eq!(r.body, "");
        assert!(!r.truncated);
        assert!(r.shown.is_empty());
    }
}
