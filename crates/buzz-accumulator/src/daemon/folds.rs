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
use crate::{complete_run, plan_run, ArtifactPayload, FoldSpec, Order};

use super::store::Store;

/// Where a fold's coverage stands over its (clamped) window — the explicit
/// completion state for a multi-pass baseline.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Coverage {
    /// Matching signals already covered by the chain.
    pub processed: usize,
    /// Matching signals not yet covered.
    pub pending: usize,
    /// True when a frozen selection is fully covered: the fold is done
    /// forever. A live fold is never complete — at best it is caught up
    /// (`pending == 0`).
    pub complete: bool,
}

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

/// Preflight summary, HTTP-shaped. Every variant carries the fold's coverage
/// state so a multi-pass baseline always knows where it stands.
#[derive(Debug, serde::Serialize)]
#[serde(tag = "plan", rename_all = "snake_case")]
pub enum PreflightOutcome {
    /// Nothing to do.
    Cached {
        /// Where coverage stands.
        coverage: Coverage,
    },
    /// Would not run.
    Stalled {
        /// Why.
        reason: String,
        /// Where coverage stands (`pending` counts the waiting signals).
        coverage: Coverage,
    },
    /// Would run: what the model would see and what it would cost.
    Ready {
        /// Signals that would be shown ("selected").
        shown: usize,
        /// Where coverage stands (`pending` includes the shown ones).
        coverage: Coverage,
        /// Whether the window would be chunked.
        truncated: bool,
        /// Zero-spend input-size estimate (tokens + window fit + headroom).
        estimate: crate::estimate::Estimate,
        /// The model-aware budget the plan was sized against, term by term.
        budget: crate::estimate::ContextBudget,
        /// The constraint that actually bounded this plan — the chunk
        /// rationale, stated instead of implied.
        limit: crate::RunLimit,
        /// Window actually queried, `[since, until_exclusive)`.
        window: (i64, i64),
        /// The exact string the model would receive — the transparency seam
        /// ("what's happening behind the curtain"). Present only when the
        /// caller asked for it.
        #[serde(skip_serializing_if = "Option::is_none")]
        model_input: Option<String>,
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

/// An optional narrowing of the selection's own window, `[since,
/// until_exclusive)`. This is how a run stays pinned to the exact window its
/// preflight priced; it can only narrow, never widen (the selection's window
/// wins by intersection).
pub type WindowClamp = (Option<i64>, Option<i64>);

/// Loads spec + chain and plans a run over the selection's window (narrowed
/// by `clamp`), with an optional one-run `order` override. Returns the
/// resolved window and coverage state alongside the plan.
async fn plan(
    store: &Store,
    name: &str,
    clamp: WindowClamp,
    order: Option<Order>,
) -> Result<(FoldSpec, Vec<ArtifactPayload>, Plan, (i64, i64), Coverage), FoldError> {
    let spec = store
        .get_fold(name)
        .await?
        .ok_or_else(|| FoldError::NotFound(name.to_string()))?;
    let now = chrono::Utc::now().timestamp();
    let (since, until_exclusive) = spec.selection.resolve_window(clamp.0, clamp.1, now);
    let chain = store.artifacts(name).await?;
    let covered: BTreeSet<String> = chain
        .iter()
        .flat_map(|a| a.shown_ids.iter().cloned())
        .collect();
    let fetched = store
        .query_signals(&spec.selection, since, until_exclusive)
        .await?;
    let processed = fetched.iter().filter(|s| covered.contains(&s.id)).count();
    let pending = fetched.len() - processed;
    let coverage = Coverage {
        processed,
        pending,
        complete: spec.selection.is_frozen() && pending == 0,
    };
    let authors: BTreeSet<String> = fetched.iter().map(|s| s.pubkey.clone()).collect();
    let names = store.names(&authors).await?;
    let plan = plan_run(&spec, chain.last(), &covered, fetched, &names, order)?;
    Ok((spec, chain, plan, (since, until_exclusive), coverage))
}

/// Where a fold's coverage stands over its own window — the cheap state
/// lookup behind `GET /folds/{name}`.
pub async fn coverage(store: &Store, name: &str) -> Result<Coverage, FoldError> {
    let spec = store
        .get_fold(name)
        .await?
        .ok_or_else(|| FoldError::NotFound(name.to_string()))?;
    let now = chrono::Utc::now().timestamp();
    let (since, until_exclusive) = spec.selection.resolve_window(None, None, now);
    let covered: BTreeSet<String> = store
        .artifacts(name)
        .await?
        .iter()
        .flat_map(|a| a.shown_ids.iter().cloned())
        .collect();
    let fetched = store
        .query_signals(&spec.selection, since, until_exclusive)
        .await?;
    let processed = fetched.iter().filter(|s| covered.contains(&s.id)).count();
    let pending = fetched.len() - processed;
    Ok(Coverage {
        processed,
        pending,
        complete: spec.selection.is_frozen() && pending == 0,
    })
}

/// Zero-spend preflight: what would a run do, and at what estimated cost?
/// With `include_input`, the Ready outcome carries the exact string the model
/// would receive.
pub async fn preflight(
    store: &Store,
    name: &str,
    clamp: WindowClamp,
    include_input: bool,
    order: Option<Order>,
) -> Result<PreflightOutcome, FoldError> {
    let (_, _, plan, window, coverage) = plan(store, name, clamp, order).await?;
    Ok(match plan {
        Plan::Cached => PreflightOutcome::Cached { coverage },
        Plan::Stalled { reason, .. } => PreflightOutcome::Stalled { reason, coverage },
        Plan::Ready(rp) => PreflightOutcome::Ready {
            shown: rp.shown.len(),
            coverage,
            truncated: rp.truncated,
            estimate: rp.estimate,
            budget: rp.budget,
            limit: rp.limit,
            window,
            model_input: include_input.then_some(rp.model_input),
        },
    })
}

/// Runs one fold turn over the selection's window (narrowed by `clamp`),
/// with an optional one-run `order` override.
pub async fn run_fold(
    store: &Store,
    runner: Arc<dyn FoldRunner + Send + Sync>,
    guard: &RunGuard,
    name: &str,
    clamp: WindowClamp,
    order: Option<Order>,
) -> Result<RunOutcome, FoldError> {
    let _token = guard.acquire(name)?;
    let (spec, chain, plan, _, _) = plan(store, name, clamp, order).await?;
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

    // Post-spend: the response IS the artifact; only the version fence can
    // still reject it, and that path salvages the paid-for output.
    let now = chrono::Utc::now().timestamp();
    let artifact = complete_run(&spec, chain.last(), &run_plan, &output, now);
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
                ..Selection::default()
            },
            model: "test-model".into(),
            instructions: "digest the channel".into(),
            order: Order::default(),
            meta: None,
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
                parent: None,
            }])
            .await
            .expect("seed signal");
        store
    }

    #[tokio::test]
    async fn preflight_prices_before_any_spend() {
        let store = store_with_fold_and_signal().await;
        let out = preflight(&store, "weekly", (None, None), false, None)
            .await
            .expect("preflight");
        match out {
            PreflightOutcome::Ready {
                shown,
                coverage,
                model_input,
                ..
            } => {
                assert_eq!(shown, 1);
                // Engine semantics: pending counts every new signal in the
                // window, including the ones that would be shown.
                assert_eq!(coverage.pending, 1);
                assert_eq!(coverage.processed, 0);
                assert!(!coverage.complete, "live fold is never complete");
                assert!(model_input.is_none(), "input only rides when asked for");
            }
            other => panic!("expected ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn preflight_can_expose_the_exact_model_input() {
        let store = store_with_fold_and_signal().await;
        let out = preflight(&store, "weekly", (None, None), true, None)
            .await
            .expect("preflight");
        match out {
            PreflightOutcome::Ready { model_input, .. } => {
                let input = model_input.expect("requested input");
                assert!(input.contains("digest the channel"), "task rides in input");
                assert!(input.contains("hello world"), "events ride in input");
            }
            other => panic!("expected ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_persists_output_verbatim_and_caches_next() {
        let store = store_with_fold_and_signal().await;
        let id = "e".repeat(64);
        // Free-form output — any shape the task asked for persists verbatim.
        let output = format!("Just a paragraph citing [event:{id}], no headings.");
        let runner = Arc::new(FakeRunner(output.clone()));
        let guard = RunGuard::default();
        let out = run_fold(&store, runner.clone(), &guard, "weekly", (None, None), None)
            .await
            .expect("run");
        match out {
            RunOutcome::Folded {
                artifact, shown, ..
            } => {
                assert_eq!(artifact.version, 1);
                assert_eq!(shown, 1);
                assert_eq!(artifact.shown_ids, vec![id.clone()]);
                assert_eq!(artifact.output, output, "response persists verbatim");
            }
            other => panic!("expected folded, got {other:?}"),
        }
        // Same window again: engine reports cached, no new version.
        let again = run_fold(&store, runner, &guard, "weekly", (None, None), None)
            .await
            .expect("rerun");
        assert!(matches!(again, RunOutcome::Cached), "got {again:?}");
        assert_eq!(store.artifacts("weekly").await.expect("chain").len(), 1);
    }

    #[tokio::test]
    async fn frozen_fold_completes_then_stays_cached_forever() {
        let store = store_with_fold_and_signal().await;
        // Freeze the selection around the seeded signal (created_at = 100).
        let mut spec = store
            .get_fold("weekly")
            .await
            .expect("get")
            .expect("present");
        spec.selection.since = Some(0);
        spec.selection.until_exclusive = Some(1_000);
        spec.validate().expect("valid");
        store.put_fold(&spec, 2).await.expect("update");

        let runner = Arc::new(FakeRunner("the day, summarized".into()));
        let guard = RunGuard::default();
        let out = run_fold(&store, runner.clone(), &guard, "weekly", (None, None), None)
            .await
            .expect("run");
        assert!(matches!(out, RunOutcome::Folded { .. }), "got {out:?}");

        // Fully covered: the frozen fold is done forever — no clamp needed,
        // and later events outside the freeze must never wake it.
        store
            .upsert_events(&[super::super::store::StoredEvent {
                id: "f".repeat(64),
                channel: Some(CHANNEL.into()),
                pubkey: "a".repeat(64),
                kind: 9,
                created_at: 5_000,
                content: "after the freeze".into(),
                raw: "{}".into(),
                parent: None,
            }])
            .await
            .expect("late event");
        let pf = preflight(&store, "weekly", (None, None), false, None)
            .await
            .expect("preflight");
        assert!(matches!(pf, PreflightOutcome::Cached { .. }), "got {pf:?}");
        // A clamp can only narrow, never widen past the freeze.
        let pf = preflight(&store, "weekly", (Some(0), Some(10_000)), false, None)
            .await
            .expect("preflight");
        assert!(matches!(pf, PreflightOutcome::Cached { .. }), "got {pf:?}");
    }

    #[tokio::test]
    async fn unknown_fold_is_not_found() {
        let store = Store::open(":memory:").await.expect("open");
        let err = preflight(&store, "nope", (None, None), false, None)
            .await
            .expect_err("missing fold");
        assert!(matches!(err, FoldError::NotFound(_)));
    }

    /// Riley's acceptance test: a backlog larger than one call bootstraps
    /// oldest → newest across repeated runs with no skipped or duplicate
    /// `shown_ids`, each run sized by the model-aware budget with the
    /// limiting constraint reported; the finished baseline then absorbs one
    /// newly arrived event without replaying covered history.
    #[tokio::test]
    async fn bootstrap_walks_the_backlog_then_increments_without_replay() {
        let store = store_with_fold_and_signal().await;
        // Replace the seed with a 60-event backlog too big for one run under
        // the unknown-model fallback budget (~120k chars vs 60 × 5k-char events).
        let backlog: Vec<super::super::store::StoredEvent> = (0..60)
            .map(|i| super::super::store::StoredEvent {
                id: format!("{i:064}"),
                channel: Some(CHANNEL.into()),
                pubkey: "a".repeat(64),
                kind: 9,
                created_at: 1_000 + i as i64,
                content: "m".repeat(5_000),
                raw: "{}".into(),
                parent: None,
            })
            .collect();
        store.upsert_events(&backlog).await.expect("seed backlog");
        let all_ids: Vec<String> = std::iter::once("e".repeat(64))
            .chain((0..60).map(|i| format!("{i:064}")))
            .collect();

        let runner = Arc::new(FakeRunner("baseline chunk".into()));
        let guard = RunGuard::default();
        let mut covered_in_order: Vec<String> = Vec::new();
        let mut runs = 0;
        loop {
            let pf = preflight(&store, "weekly", (None, None), false, None)
                .await
                .expect("preflight");
            match pf {
                PreflightOutcome::Ready {
                    shown,
                    coverage,
                    truncated,
                    limit,
                    ..
                } => {
                    // Preflight explains the boundary truthfully: a chunked
                    // run names its constraint, a final run reports none.
                    assert_eq!(
                        coverage.pending,
                        all_ids.len() - covered_in_order.len(),
                        "pending must track the uncovered remainder"
                    );
                    assert_eq!(coverage.processed, covered_in_order.len());
                    if truncated {
                        assert!(shown < coverage.pending);
                        assert_eq!(limit, crate::RunLimit::TokenBudget);
                    } else {
                        assert_eq!(shown, coverage.pending);
                        assert_eq!(limit, crate::RunLimit::None);
                    }
                }
                other => panic!("expected ready mid-bootstrap, got {other:?}"),
            }
            let out = run_fold(&store, runner.clone(), &guard, "weekly", (None, None), None)
                .await
                .expect("run");
            let RunOutcome::Folded { artifact, .. } = out else {
                panic!("expected folded, got {out:?}");
            };
            covered_in_order.extend(artifact.shown_ids.iter().cloned());
            runs += 1;
            assert!(runs < 20, "bootstrap must converge");
            if covered_in_order.len() == all_ids.len() {
                break;
            }
        }
        assert!(runs > 1, "the backlog must not fit one call");
        // Earliest → latest, no skips, no duplicates: the runs' shown_ids
        // concatenate to exactly the chronological id list.
        assert_eq!(covered_in_order, all_ids);

        // Baseline done: the live fold is caught up (never "complete").
        let pf = preflight(&store, "weekly", (None, None), false, None)
            .await
            .expect("preflight");
        let PreflightOutcome::Cached { coverage } = pf else {
            panic!("expected cached after bootstrap, got {pf:?}");
        };
        assert_eq!(coverage.processed, all_ids.len());
        assert_eq!(coverage.pending, 0);
        assert!(!coverage.complete, "live selection is never complete");

        // One newly arrived event folds without replaying covered history.
        let late = "f".repeat(64);
        store
            .upsert_events(&[super::super::store::StoredEvent {
                id: late.clone(),
                channel: Some(CHANNEL.into()),
                pubkey: "a".repeat(64),
                kind: 9,
                created_at: 9_000,
                content: "the new arrival".into(),
                raw: "{}".into(),
                parent: None,
            }])
            .await
            .expect("late event");
        let out = run_fold(&store, runner, &guard, "weekly", (None, None), None)
            .await
            .expect("incremental run");
        let RunOutcome::Folded { artifact, .. } = out else {
            panic!("expected folded, got {out:?}");
        };
        assert_eq!(
            artifact.shown_ids,
            vec![late],
            "incremental run folds exactly the frontier, no replay"
        );
    }
}
