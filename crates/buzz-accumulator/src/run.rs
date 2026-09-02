//! Planning and completing one fold run.
//!
//! A run is two pure halves around one model call:
//!
//! 1. [`plan_run`] — decide cached/stalled/ready, render the transcript under
//!    budget, and price the *exact* string the model would receive. No model
//!    call, no I/O.
//! 2. The caller shows the estimate (priced before spend), invokes a
//!    [`crate::runner::FoldRunner`] with `RunPlan::model_input`, then
//! 3. [`complete_run`] — record the output verbatim with engine-computed
//!    provenance (exactly the shown signals and their window) as the next
//!    [`ArtifactPayload`]. The output's shape is the task's business, not the
//!    engine's.

use std::collections::{BTreeMap, BTreeSet};

use crate::artifact::ArtifactPayload;
use crate::error::Error;
use crate::estimate::{self, ContextBudget, Estimate};
use crate::selection::materialize;
use crate::signal::Signal;
use crate::spec::{FoldSpec, Order};
use crate::transcript::{render_transcript, ShownSignal};

/// Emergency guard: most signals one run may show (and therefore seal).
/// Bounds `shown_ids` so one version's provenance stays storable; the normal
/// limit is the model-aware token budget, not this.
pub const MAX_SHOWN_PER_RUN: usize = 1_000;

/// Chars one SOURCE EVENT IDS line costs (64-hex id + newline), reserved out
/// of the budget before the transcript is rendered.
const ID_LINE_CHARS: usize = 65;

/// Largest prior artifact document a run will build on: a model-input budget.
/// The prior digest is re-fed to the model every run, so an unbounded one
/// erodes the transcript budget until runs fold nothing; stalling here keeps
/// that failure priced-before-spend instead of a wasted model call.
pub const MAX_PRIOR_OUTPUT_BYTES: usize = 40_000;

/// Engine note composed into every model input between the task and the
/// context: what the context is, and how citations work. The output shape is
/// deliberately unconstrained — provenance is engine-owned, so citations are
/// reader-verifiable links, not a contract.
const ENGINE_GUIDANCE: &str = "The context below is your evidence: the prior version of this \
document (when one exists) followed by new time-ordered events. Produce the next version of \
the document — whatever shape the task asks for. Where useful, cite the events behind a claim \
as [event:<id>], one id per bracket, copied in full from the SOURCE EVENT IDS list; citations \
render as clickable links to the source, so a wrong or invented id shows readers a dead link. \
Do not invent facts. Output only the document.";

/// The outcome of planning a run.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// Nothing new and the config is unchanged — the latest artifact already
    /// answers; spend nothing.
    Cached,
    /// The run cannot honestly proceed; nothing is spent and nothing is
    /// sealed. `pending` counts the uncovered signals still waiting.
    Stalled { reason: String, pending: usize },
    /// Ready to run: the exact model input and its price.
    Ready(RunPlan),
}

/// Which constraint actually bounded a run plan — preflight reports this so
/// every boundary is explainable, never mysterious.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunLimit {
    /// Everything pending fits: nothing was left behind.
    None,
    /// The model-aware token budget bound the run; the rest stays pending.
    TokenBudget,
    /// The [`MAX_SHOWN_PER_RUN`] emergency guard bound the run.
    EventCap,
}

/// A priced, ready-to-execute run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunPlan {
    /// The exact string to hand the runner — previews price this same string.
    pub model_input: String,
    /// Exactly the signals rendered into the input, in transcript order.
    pub shown: Vec<ShownSignal>,
    /// True when pending signals were dropped/trimmed to fit the budget; the
    /// remainder stays pending for the next run.
    pub truncated: bool,
    /// Zero-spend input-size estimate of `model_input` for the spec's model.
    pub estimate: Estimate,
    /// Total uncovered signals this plan drew from (`shown.len() <=` this).
    pub pending: usize,
    /// The model-aware budget the plan was sized against, term by term.
    pub budget: ContextBudget,
    /// The constraint that actually bounded this plan.
    pub limit: RunLimit,
}

/// Plan the next run of `spec`.
///
/// `prior` is the latest artifact version (if any), `covered` the union of
/// `shown_ids` across all prior versions, `fetched` the raw signals the
/// caller fetched for the selection's window, `names` an optional
/// pubkey→display-name map for transcript lines, and `order` an optional
/// one-run override of the spec's traversal policy.
pub fn plan_run(
    spec: &FoldSpec,
    prior: Option<&ArtifactPayload>,
    covered: &BTreeSet<String>,
    fetched: Vec<Signal>,
    names: &BTreeMap<String, String>,
    order: Option<Order>,
) -> Result<Plan, Error> {
    let signals = materialize(fetched);
    let new: Vec<Signal> = signals
        .into_iter()
        .filter(|s| !covered.contains(&s.id))
        .collect();
    let config_matches = prior.is_some_and(|p| {
        p.model == spec.model
            && p.prompt_sha256 == spec.prompt_sha256()
            && p.selection == spec.selection
    });
    if config_matches && new.is_empty() {
        return Ok(Plan::Cached);
    }
    if prior.is_none() && new.is_empty() {
        return Ok(Plan::Stalled {
            reason: "selection matched no signals in the window; nothing to fold yet".to_string(),
            pending: 0,
        });
    }
    if prior.is_some_and(|p| p.output.len() > MAX_PRIOR_OUTPUT_BYTES) {
        return Ok(Plan::Stalled {
            reason: format!(
                "standing artifact is over the {MAX_PRIOR_OUTPUT_BYTES}-byte prior-digest budget; \
                 compact it (tighter instructions, or start a fresh fold) before retrying"
            ),
            pending: new.len(),
        });
    }
    let parent = match prior {
        Some(p) => format!(
            "--- PRIOR VERSION ({} v{}) ---\n{}",
            spec.name, p.version, p.output
        ),
        None => String::new(),
    };
    // Model-aware budget: window − reserved output − safety margin, then
    // every fixed part of the input (task, guidance, prior, id list) is
    // charged before the transcript gets the remainder.
    let budget = estimate::context_budget(&spec.model);
    let order = order.unwrap_or(spec.order);
    let fixed_overhead = spec.instructions.chars().count()
        + ENGINE_GUIDANCE.chars().count()
        + parent.chars().count()
        + new.len().min(MAX_SHOWN_PER_RUN) * ID_LINE_CHARS
        + 128; // section headers + separators
    let raw_budget = budget.input_budget_chars.saturating_sub(fixed_overhead);
    let render = render_transcript(&new, names, raw_budget, MAX_SHOWN_PER_RUN, order);
    if !new.is_empty() && render.shown.is_empty() {
        return Ok(Plan::Stalled {
            reason: "no pending event fits the remaining context budget".to_string(),
            pending: new.len(),
        });
    }
    let limit = if !render.truncated {
        RunLimit::None
    } else if render.shown.len() >= MAX_SHOWN_PER_RUN {
        RunLimit::EventCap
    } else {
        RunLimit::TokenBudget
    };
    let mut transcript = [parent.as_str(), render.body.as_str()]
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n\n");
    if !render.shown.is_empty() {
        let ids: Vec<&str> = render.shown.iter().map(|s| s.id.as_str()).collect();
        transcript.push_str("\n\n--- SOURCE EVENT IDS ---\n");
        transcript.push_str(&ids.join("\n"));
    }
    // Instructions are the task and the output is free-form; the engine only
    // explains what the context is and how citations render.
    let model_input = format!(
        "--- TASK ---\n{}\n\n{}\n\n--- CONTEXT (time-ordered events) ---\n{}\n",
        spec.instructions, ENGINE_GUIDANCE, transcript
    );
    let est = estimate::estimate(&spec.model, model_input.chars().count());
    Ok(Plan::Ready(RunPlan {
        model_input,
        shown: render.shown,
        truncated: render.truncated,
        estimate: est,
        pending: new.len(),
        budget,
        limit,
    }))
}

/// Record model `output` for `plan` verbatim as the next artifact version.
///
/// The output's shape is the task's business — nothing about the text is
/// judged. Provenance is engine-owned: coverage is computed from exactly the
/// signals the plan showed, never from what the output claims.
pub fn complete_run(
    spec: &FoldSpec,
    prior: Option<&ArtifactPayload>,
    plan: &RunPlan,
    output: &str,
    created_at: i64,
) -> ArtifactPayload {
    let shown_ids: Vec<String> = plan.shown.iter().map(|s| s.id.clone()).collect();
    // Taint travels with the chain: once a channel's events fold in, every
    // later version keeps carrying that channel, even after a selection edit.
    let channels: BTreeSet<String> = prior
        .map(|p| p.channels.iter().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default()
        .union(&spec.selection.channels.iter().cloned().collect())
        .cloned()
        .collect();
    ArtifactPayload {
        fold: spec.name.clone(),
        version: prior.map_or(1, |p| p.version + 1),
        output: output.to_string(),
        coverage_since: plan.shown.iter().map(|s| s.created_at).min(),
        coverage_until: plan.shown.iter().map(|s| s.created_at).max().map(|t| t + 1),
        selection: spec.selection.clone(),
        channels: channels.into_iter().collect(),
        shown_ids,
        model: spec.model.clone(),
        prompt_sha256: spec.prompt_sha256(),
        truncated: plan.truncated,
        created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Selection;

    fn hex_id(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn spec() -> FoldSpec {
        FoldSpec {
            name: "team-digest".to_string(),
            selection: Selection {
                channels: vec!["ch1".to_string()],
                ..Selection::default()
            },
            model: "haiku".to_string(),
            instructions: "Maintain the digest.".to_string(),
            order: Order::default(),
            meta: None,
        }
    }

    fn sig(id: &str, ts: i64, content: &str) -> Signal {
        Signal {
            id: id.to_string(),
            pubkey: "pk".to_string(),
            kind: 9,
            created_at: ts,
            content: content.to_string(),
            channel: Some("ch1".to_string()),
        }
    }

    fn digest(log_body: &str) -> String {
        format!("# Working Context\n\nSummary.\n\n# Log\n\n{log_body}\n")
    }

    fn artifact_v1(spec: &FoldSpec, shown: &[(&str, i64)]) -> ArtifactPayload {
        ArtifactPayload {
            fold: spec.name.clone(),
            version: 1,
            output: digest(&format!("- prior entry [event:{}]", hex_id('a'))),
            shown_ids: shown.iter().map(|(id, _)| id.to_string()).collect(),
            coverage_since: shown.iter().map(|(_, ts)| *ts).min(),
            coverage_until: shown.iter().map(|(_, ts)| *ts).max().map(|t| t + 1),
            selection: spec.selection.clone(),
            channels: spec.selection.channels.clone(),
            model: spec.model.clone(),
            prompt_sha256: spec.prompt_sha256(),
            truncated: false,
            created_at: 1_700_000_100,
        }
    }

    #[test]
    fn cached_when_config_matches_and_nothing_new() {
        let spec = spec();
        let a = hex_id('a');
        let prior = artifact_v1(&spec, &[(&a, 100)]);
        let covered = BTreeSet::from([a.clone()]);
        let plan = plan_run(
            &spec,
            Some(&prior),
            &covered,
            vec![sig(&a, 100, "seen")],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        assert_eq!(plan, Plan::Cached);
    }

    #[test]
    fn config_change_reruns_without_new_signals() {
        let mut spec = spec();
        let a = hex_id('a');
        let prior = artifact_v1(&spec, &[(&a, 100)]);
        spec.instructions.push_str(" Now stricter.");
        let covered = BTreeSet::from([a.clone()]);
        let plan = plan_run(
            &spec,
            Some(&prior),
            &covered,
            vec![sig(&a, 100, "seen")],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready, got {plan:?}");
        };
        assert!(run.shown.is_empty());
        assert!(!run.model_input.contains("--- SOURCE EVENT IDS ---"));
        assert!(run
            .model_input
            .contains("--- PRIOR VERSION (team-digest v1) ---"));
        // No shown signals → nothing new to cover, coverage stays empty.
        let v2 = complete_run(
            &spec,
            Some(&prior),
            &run,
            "rewritten under new task",
            1_700_000_200,
        );
        assert_eq!(v2.version, 2);
        assert!(v2.shown_ids.is_empty());
        assert_eq!(v2.coverage_since, None);
        assert_eq!(v2.coverage_until, None);
        // The response persists verbatim — retention is the model's job now.
        assert_eq!(v2.output, "rewritten under new task");
    }

    #[test]
    fn stalled_when_standing_artifact_exceeds_prior_digest_budget() {
        let spec = spec();
        let mut prior = artifact_v1(&spec, &[]);
        prior.output = "x".repeat(MAX_PRIOR_OUTPUT_BYTES + 1);
        let plan = plan_run(
            &spec,
            Some(&prior),
            &BTreeSet::new(),
            vec![sig(&hex_id('b'), 200, "pending")],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Stalled { reason, pending } = plan else {
            panic!("expected Stalled, got Ready/Cached");
        };
        assert!(reason.contains("prior-digest budget"));
        assert_eq!(pending, 1);
    }

    #[test]
    fn selection_change_is_not_cached() {
        let mut spec = spec();
        let a = hex_id('a');
        let prior = artifact_v1(&spec, &[(&a, 100)]);
        spec.selection.channels.push("ch2".to_string());
        let covered = BTreeSet::from([a]);
        let plan = plan_run(
            &spec,
            Some(&prior),
            &covered,
            vec![],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        assert!(matches!(plan, Plan::Ready(_)), "got {plan:?}");
    }

    #[test]
    fn no_prior_and_no_signals_stalls_instead_of_folding_nothing() {
        let spec = spec();
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            vec![],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Stalled { reason, pending } = plan else {
            panic!("expected Stalled, got {plan:?}");
        };
        assert!(reason.contains("nothing to fold"));
        assert_eq!(pending, 0);
    }

    #[test]
    fn channels_union_survives_a_selection_change() {
        let mut spec = spec();
        let a = hex_id('a');
        let b = hex_id('b');
        let prior = artifact_v1(&spec, &[(&a, 100)]);
        // The fold once read ch1; the selection now reads only ch2.
        spec.selection.channels = vec!["ch2".to_string()];
        let covered = BTreeSet::from([a]);
        let mut new_sig = sig(&b, 200, "from ch2");
        new_sig.channel = Some("ch2".to_string());
        let plan = plan_run(
            &spec,
            Some(&prior),
            &covered,
            vec![new_sig],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        let out = digest(&format!("- new entry [event:{b}]"));
        let v2 = complete_run(&spec, Some(&prior), &run, &out, 1_700_000_500);
        assert_eq!(v2.channels, vec!["ch1".to_string(), "ch2".to_string()]);
        assert_eq!(v2.selection, spec.selection);
    }

    #[test]
    fn ready_plan_prices_the_exact_model_input() {
        let spec = spec();
        let b = hex_id('b');
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            vec![sig(&b, 200, "hello")],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        assert_eq!(run.pending, 1);
        assert_eq!(run.shown.len(), 1);
        assert!(run.model_input.contains("--- SOURCE EVENT IDS ---"));
        assert!(run.model_input.contains(&b));
        assert_eq!(
            run.estimate,
            estimate::estimate("haiku", run.model_input.chars().count())
        );
        assert_eq!(run.estimate.window_fit.fits, Some(true));
    }

    #[test]
    fn every_run_carries_the_task_and_the_engine_guidance() {
        // The engine explains what the context is and how citations render;
        // the task is the caller's, verbatim.
        let mut spec = spec();
        spec.instructions = "what is this project trying to accomplish".to_string();
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            vec![sig(&hex_id('b'), 200, "hello")],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        assert!(run
            .model_input
            .contains("what is this project trying to accomplish"));
        assert!(run.model_input.contains("whatever shape the task asks for"));
        assert!(run.model_input.contains("SOURCE EVENT IDS"));
    }

    #[test]
    fn oversized_backlog_chunks_honestly_and_never_seals_unread() {
        let spec = spec();
        // Enough large signals to overflow even the model-aware haiku budget
        // (300 × 5k chars ≈ 1.5M chars against a ~700k-char budget).
        let fetched: Vec<Signal> = (0..300)
            .map(|i| sig(&format!("{i:064}"), 1_000 + i as i64, &"m".repeat(5_000)))
            .collect();
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            fetched,
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        assert!(run.truncated);
        assert!(run.shown.len() < 300, "must not claim the whole backlog");
        assert_eq!(run.pending, 300);
        assert_eq!(run.limit, RunLimit::TokenBudget);
        // Default order walks the backlog forward: the OLDEST chunk first.
        assert_eq!(run.shown[0].id, format!("{:064}", 0));
        let n = run.shown.len();
        assert_eq!(run.shown[n - 1].id, format!("{:064}", n - 1));
        // Coverage seals only what was shown; the rest stays pending.
        let cited = run.shown[0].id.clone();
        let out = digest(&format!("- chunk one [event:{cited}]"));
        let v1 = complete_run(&spec, None, &run, &out, 1_700_000_300);
        assert_eq!(v1.shown_ids.len(), run.shown.len());
        assert!(v1.truncated);
    }

    #[test]
    fn newest_first_override_keeps_the_freshest_chunk() {
        let spec = spec();
        let fetched: Vec<Signal> = (0..300)
            .map(|i| sig(&format!("{i:064}"), 1_000 + i as i64, &"m".repeat(5_000)))
            .collect();
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            fetched,
            &BTreeMap::new(),
            Some(Order::NewestFirst),
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        assert!(run.truncated);
        // The freshest event makes the cut; older ones stay pending.
        assert_eq!(run.shown.last().expect("shown").id, format!("{:064}", 299));
        assert_ne!(run.shown[0].id, format!("{:064}", 0));
    }

    #[test]
    fn known_model_budget_folds_far_past_the_old_flat_ceiling() {
        // 40 × 5k chars ≈ 200k chars ≈ 50k tokens: over the old 120k-char
        // ceiling, comfortably inside haiku's model-aware budget — one run,
        // nothing left behind, and the plan says so.
        let spec = spec();
        let fetched: Vec<Signal> = (0..40)
            .map(|i| sig(&format!("{i:064}"), 1_000 + i as i64, &"m".repeat(5_000)))
            .collect();
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            fetched,
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        assert!(!run.truncated);
        assert_eq!(run.shown.len(), 40);
        assert_eq!(run.limit, RunLimit::None);
        assert!(
            run.estimate.est_input_tokens > 30_000,
            "the old ceiling stopped near 30k tokens; got {}",
            run.estimate.est_input_tokens
        );
        assert_eq!(run.estimate.window_fit.fits, Some(true));
        assert_eq!(run.budget.model_window, Some(200_000));
    }

    #[test]
    fn event_cap_is_the_emergency_guard_and_is_reported() {
        let spec = spec();
        // Tiny signals: the token budget would hold thousands, so the
        // MAX_SHOWN_PER_RUN guard binds — and the plan must say that.
        let fetched: Vec<Signal> = (0..MAX_SHOWN_PER_RUN + 500)
            .map(|i| sig(&format!("{i:064}"), 1_000 + i as i64, "m"))
            .collect();
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            fetched,
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        assert!(run.truncated);
        assert_eq!(run.shown.len(), MAX_SHOWN_PER_RUN);
        assert_eq!(run.limit, RunLimit::EventCap);
        assert_eq!(run.pending, MAX_SHOWN_PER_RUN + 500);
    }

    #[test]
    fn complete_run_produces_v1_with_coverage_window() {
        let spec = spec();
        let b = hex_id('b');
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            vec![sig(&b, 200, "hello")],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        let out = digest(&format!("- said hello [event:{b}]"));
        let v1 = complete_run(&spec, None, &run, &out, 1_700_000_300);
        assert_eq!(v1.version, 1);
        assert_eq!(v1.shown_ids, vec![b]);
        assert_eq!(v1.coverage_since, Some(200));
        assert_eq!(v1.coverage_until, Some(201));
        assert_eq!(v1.prompt_sha256, spec.prompt_sha256());
    }

    #[test]
    fn output_is_verbatim_and_the_prior_rides_in_the_input() {
        let spec = spec();
        let a = hex_id('a');
        let b = hex_id('b');
        let prior = artifact_v1(&spec, &[(&a, 100)]);
        let covered = BTreeSet::from([a]);
        let plan = plan_run(
            &spec,
            Some(&prior),
            &covered,
            vec![sig(&b, 200, "next")],
            &BTreeMap::new(),
            None,
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        // The prior version is the model's context, labeled as such…
        assert!(run
            .model_input
            .contains("--- PRIOR VERSION (team-digest v1) ---"));
        // …and the model's response IS the artifact: any shape, verbatim, no
        // splice. Provenance (version, coverage) is engine bookkeeping.
        let out = "A totally free-form rewrite that dropped the prior entry.";
        let v2 = complete_run(&spec, Some(&prior), &run, out, 1_700_000_400);
        assert_eq!(v2.version, 2);
        assert_eq!(v2.output, out);
        assert_eq!(v2.shown_ids, vec![b]);
    }
}
