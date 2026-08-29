//! Artifact versions: the immutable, provenance-carrying output of one run.

use serde::{Deserialize, Serialize};

use crate::selection::Selection;

/// One artifact version — the JSON a caller NIP-44-encrypts into the content
/// of an immutable relay event.
///
/// Versions form a chain per fold (v1, v2, …), each produced by
/// [`crate::run::complete_run`]. The chain is the coverage ledger: the set of
/// signals a fold has ever folded is exactly the union of `shown_ids` over
/// its versions — computed from what the model was actually shown, so unread
/// signals are never sealed as covered. Append-only is physics here: relay
/// events cannot be rewritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPayload {
    /// Fold name this version belongs to.
    pub fold: String,
    /// 1-based version number; each run of the fold increments it.
    pub version: u32,
    /// Full artifact document (after the engine splice).
    pub output: String,
    /// Exactly the signal ids the model was shown this run.
    pub shown_ids: Vec<String>,
    /// Earliest shown signal timestamp (unix seconds); `None` when the run
    /// showed no signals (config-only rerun).
    pub coverage_since: Option<i64>,
    /// Half-open end of the covered window: latest shown timestamp + 1;
    /// `None` when the run showed no signals.
    ///
    /// The window is advisory provenance: coverage *truth* is `shown_ids`.
    /// Signals timestamped inside the window that were dropped to fit the
    /// budget stay pending — the next plan filters by id, not by window.
    pub coverage_until: Option<i64>,
    /// The exact selection this version was planned from. A later selection
    /// change makes the cached-run shortcut miss, so edits always re-fold.
    #[serde(default)]
    pub selection: Selection,
    /// Every channel any version in this chain has ever read (sorted union
    /// over the chain). Sharing is taint-checked against this, not the live
    /// spec: an artifact that ever folded another channel's events never
    /// leaks them into a single-channel share.
    #[serde(default)]
    pub channels: Vec<String>,
    /// Model that produced this version.
    pub model: String,
    /// Schema the output conforms to.
    pub schema: String,
    /// SHA-256 of the instructions used (provenance + cache key).
    pub prompt_sha256: String,
    /// True when the context budget dropped or trimmed pending signals — the
    /// remainder stays pending for the next run.
    pub truncated: bool,
    /// Unix seconds this version was produced (caller-supplied).
    pub created_at: i64,
}
