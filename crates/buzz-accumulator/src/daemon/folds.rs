//! Fold machinery over the local mirror: preflight and run.
//!
//! Mirrors the discipline of the relay-backed CLI path with one structural
//! upgrade: artifacts live in local SQLite under a `(fold, version)` primary
//! key, so the version fence is atomic — a losing concurrent run hits the
//! constraint and its (already paid-for) output is salvaged in the response
//! instead of forking the chain.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use crate::run::Plan;
use crate::runner::FoldRunner;
use crate::{complete_run, plan_run, ArtifactPayload, FoldSpec};

use super::store::Store;

/// A fold-run outcome, HTTP-shaped.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunOutcome {
    /// Nothing new and configuration unchanged; no model call was made.
    Cached,
    /// The engine refused to run before any spend.
    Stalled {
        /// Why the run stalled.
        reason: String,
        /// New (uncovered) signals waiting in the window.
        pending: usize,
    },
    /// Model output broke the artifact contract; nothing persisted.
    /// The paid-for output is salvaged here rather than discarded.
    Refused {
        /// The validator's refusal.
        reason: String,
        /// Raw model output for inspection.
        model_output: String,
    },
    /// A concurrent run appended this version first; nothing persisted.
    Unpublished {
        /// What happened.
        reason: String,
        /// Raw model output for inspection.
        model_output: String,
    },
    /// A new artifact version was appended.
    Folded {
        /// The persisted artifact (boxed: much larger than the other
        /// variants).
        artifact: Box<ArtifactPayload>,
        /// Signals shown this run.
        shown: usize,
        /// New signals in the window (engine semantics: includes the shown
        /// ones; `pending - shown` remain for a follow-up run).
        pending: usize,
        /// Whether this run truncated (chunked) the window.
        truncated: bool,
    },
}

/// Preflight summary, HTTP-shaped.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "plan", rename_all = "snake_case")]
pub enum PreflightOutcome {
    /// Nothing to do.
    Cached,
    /// Would not run.
    Stalled {
        /// Why.
        reason: String,
        /// New (uncovered) signals waiting in the window.
        pending: usize,
    },
    /// Would run: what the model would see and what it would cost.
    Ready {
        /// Signals that would be shown.
        shown: usize,
        /// New signals in the window (includes the shown ones).
        pending: usize,
        /// Whether the window would be chunked.
        truncated: bool,
        /// Zero-spend estimate (tokens; cost only for curated models).
        estimate: crate::estimate::Estimate,
        /// Window actually queried, `[since, until_exclusive)`.
        window: (i64, i64),
    },
}

/// Errors from fold operations.
#[derive(Debug, thiserror::Error)]
pub enum FoldError {
    /// Unknown fold name.
    #[error("fold not found: {0}")]
    NotFound(String),
    /// Another run for this fold is currently in flight.
    #[error("a run for fold {0} is already in flight")]
    Busy(String),
    /// Engine error (validation, planning).
    #[error(transparent)]
    Engine(#[from] crate::Error),
    /// Storage error.
    #[error("storage: {0}")]
    Store(#[from] sqlx::Error),
}

/// Serializes runs per fold: preflight is concurrent-safe, runs are not.
#[derive(Clone, Default)]
pub struct RunGuard(Arc<Mutex<BTreeSet<String>>>);

impl RunGuard {
    fn acquire(&self, name: &str) -> Result<RunToken, FoldError> {
        let mut held = match self.0.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !held.insert(name.to_string()) {
            return Err(FoldError::Busy(name.to_string()));
        }
        Ok(RunToken {
            guard: self.clone(),
            name: name.to_string(),
        })
    }
}

struct RunToken {
    guard: RunGuard,
    name: String,
}

impl Drop for RunToken {
    fn drop(&mut self) {
        let mut held = match self.guard.0.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        held.remove(&self.name);
    }
}

/// Loads spec + chain and plans a run over `[since, until_exclusive)`.
async fn plan(
    store: &Store,
    name: &str,
    since: i64,
    until_exclusive: i64,
) -> Result<(FoldSpec, Vec<ArtifactPayload>, Plan), FoldError> {
    let spec = store
        .get_fold(name)
        .await?
        .ok_or_else(|| FoldError::NotFound(name.to_string()))?;
    let chain = store.artifacts(name).await?;
    let covered: BTreeSet<String> = chain
        .iter()
        .flat_map(|a| a.shown_ids.iter().cloned())
        .collect();
    let fetched = store
        .query_signals(&spec.selection, since, until_exclusive)
        .await?;
    let authors: BTreeSet<String> = fetched.iter().map(|s| s.pubkey.clone()).collect();
    let names = store.names(&authors).await?;
    let plan = plan_run(&spec, chain.last(), &covered, fetched, &names)?;
    Ok((spec, chain, plan))
}

/// Zero-spend preflight: what would a run over this window do, and at what
/// estimated cost?
pub async fn preflight(
    store: &Store,
    name: &str,
    since: i64,
    until_exclusive: i64,
) -> Result<PreflightOutcome, FoldError> {
    let (_, _, plan) = plan(store, name, since, until_exclusive).await?;
    Ok(match plan {
        Plan::Cached => PreflightOutcome::Cached,
        Plan::Stalled { reason, pending } => PreflightOutcome::Stalled { reason, pending },
        Plan::Ready(rp) => PreflightOutcome::Ready {
            shown: rp.shown.len(),
            pending: rp.pending,
            truncated: rp.truncated,
            estimate: rp.estimate,
            window: (since, until_exclusive),
        },
    })
}

/// Runs one fold turn over `[since, until_exclusive)`.
pub async fn run_fold(
    store: &Store,
    runner: Arc<dyn FoldRunner + Send + Sync>,
    guard: &RunGuard,
    name: &str,
    since: i64,
    until_exclusive: i64,
) -> Result<RunOutcome, FoldError> {
    let _token = guard.acquire(name)?;
    let (spec, chain, plan) = plan(store, name, since, until_exclusive).await?;
    let run_plan = match plan {
        Plan::Cached => return Ok(RunOutcome::Cached),
        Plan::Stalled { reason, pending } => return Ok(RunOutcome::Stalled { reason, pending }),
        Plan::Ready(rp) => rp,
    };

    // The model call is synchronous and long; keep it off the async runtime.
    let model_input = run_plan.model_input.clone();
    let model = spec.model.clone();
    let output = tokio::task::spawn_blocking(move || runner.run(&model_input, &model))
        .await
        .map_err(|e| crate::Error::Runner(format!("runner task panicked: {e}")))??;

    // Post-spend: every failure from here on salvages the paid-for output.
    let now = chrono::Utc::now().timestamp();
    let artifact = match complete_run(&spec, chain.last(), &run_plan, &output, now) {
        Ok(a) => a,
        Err(crate::Error::Nonconforming(reason)) => {
            return Ok(RunOutcome::Refused {
                reason,
                model_output: output,
            });
        }
        Err(e) => return Err(e.into()),
    };
    if let Err(e) = store.insert_artifact(&artifact).await {
        // The (fold, version) primary key is the fence; a violation means a
        // concurrent writer won. Nothing forked, nothing persisted here.
        return Ok(RunOutcome::Unpublished {
            reason: format!("version fence: {e}"),
            model_output: output,
        });
    }
    Ok(RunOutcome::Folded {
        shown: run_plan.shown.len(),
        pending: run_plan.pending,
        truncated: run_plan.truncated,
        artifact: Box::new(artifact),
    })
}

/// Default window for preflight/run when the caller does not narrow it:
/// everything up to now.
pub fn default_window() -> (i64, i64) {
    (0, chrono::Utc::now().timestamp() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Selection;

    const CHANNEL: &str = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";

    struct FakeRunner(String);
    impl FoldRunner for FakeRunner {
        fn run(&self, _input: &str, _model: &str) -> Result<String, crate::Error> {
            Ok(self.0.clone())
        }
    }

    async fn store_with_fold_and_signal() -> Store {
        let store = Store::open(":memory:").await.expect("open");
        let mut spec = FoldSpec {
            name: "weekly".into(),
            selection: Selection {
                channels: vec![CHANNEL.into()],
                authors: vec![],
                kinds: vec![],
            },
            schema: "channel-digest@v1".into(),
            model: "test-model".into(),
            instructions: "digest the channel".into(),
        };
        spec.validate().expect("valid");
        store.put_fold(&spec, 1).await.expect("put fold");
        store
            .upsert_events(&[super::super::store::StoredEvent {
                id: "e".repeat(64),
                channel: Some(CHANNEL.into()),
                pubkey: "a".repeat(64),
                kind: 9,
                created_at: 100,
                content: "hello world".into(),
                raw: "{}".into(),
            }])
            .await
            .expect("seed signal");
        store
    }

    #[tokio::test]
    async fn preflight_prices_before_any_spend() {
        let store = store_with_fold_and_signal().await;
        let out = preflight(&store, "weekly", 0, 1_000)
            .await
            .expect("preflight");
        match out {
            PreflightOutcome::Ready { shown, pending, .. } => {
                assert_eq!(shown, 1);
                // Engine semantics: pending counts every new signal in the
                // window, including the ones that would be shown.
                assert_eq!(pending, 1);
            }
            other => panic!("expected ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_persists_conforming_output_and_caches_next() {
        let store = store_with_fold_and_signal().await;
        let id = "e".repeat(64);
        let output = format!("# Working Context\nsummary\n\n# Log\n- hello [event:{id}]\n");
        let runner = Arc::new(FakeRunner(output));
        let guard = RunGuard::default();
        let out = run_fold(&store, runner.clone(), &guard, "weekly", 0, 1_000)
            .await
            .expect("run");
        match out {
            RunOutcome::Folded {
                artifact, shown, ..
            } => {
                assert_eq!(artifact.version, 1);
                assert_eq!(shown, 1);
                assert_eq!(artifact.shown_ids, vec![id.clone()]);
            }
            other => panic!("expected folded, got {other:?}"),
        }
        // Same window again: engine reports cached, no new version.
        let again = run_fold(&store, runner, &guard, "weekly", 0, 1_000)
            .await
            .expect("rerun");
        assert!(matches!(again, RunOutcome::Cached), "got {again:?}");
        assert_eq!(store.artifacts("weekly").await.expect("chain").len(), 1);
    }

    #[tokio::test]
    async fn nonconforming_output_is_refused_and_salvaged() {
        let store = store_with_fold_and_signal().await;
        let runner = Arc::new(FakeRunner("no headings at all".into()));
        let out = run_fold(&store, runner, &RunGuard::default(), "weekly", 0, 1_000)
            .await
            .expect("run");
        match out {
            RunOutcome::Refused { model_output, .. } => {
                assert_eq!(model_output, "no headings at all");
            }
            other => panic!("expected refused, got {other:?}"),
        }
        assert!(store.artifacts("weekly").await.expect("chain").is_empty());
    }

    #[tokio::test]
    async fn unknown_fold_is_not_found() {
        let store = Store::open(":memory:").await.expect("open");
        let err = preflight(&store, "nope", 0, 10)
            .await
            .expect_err("missing fold");
        assert!(matches!(err, FoldError::NotFound(_)));
    }
}
