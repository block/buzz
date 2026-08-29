//! Planning and completing one fold run.
//!
//! A run is two pure halves around one model call:
//!
//! 1. [`plan_run`] — decide cached/stalled/ready, render the transcript under
//!    budget, and price the *exact* string the model would receive. No model
//!    call, no I/O.
//! 2. The caller shows the estimate (priced before spend), invokes a
//!    [`crate::runner::FoldRunner`] with `RunPlan::model_input`, then
//! 3. [`complete_run`] — validate the output against the artifact contract,
//!    splice history, and produce the next [`ArtifactPayload`]. Nonconforming
//!    output returns an error and nothing persists.

use std::collections::{BTreeMap, BTreeSet};

use crate::artifact::ArtifactPayload;
use crate::error::Error;
use crate::estimate::{self, Estimate};
use crate::schema;
use crate::selection::materialize;
use crate::signal::Signal;
use crate::spec::FoldSpec;
use crate::transcript::{render_transcript, ShownSignal};
use crate::validate::{splice_append_sections, validate_output};

/// Hard character budget for one run's model input (prior digest + transcript).
pub const MAX_CONTEXT_CHARS: usize = 120_000;

/// Most signals one run may show (and therefore seal). Bounds `shown_ids` so
/// an artifact payload cannot outgrow its encrypted relay event; the rest of
/// an oversized backlog stays pending for the next run.
pub const MAX_SHOWN_PER_RUN: usize = 250;

/// Largest prior artifact document a run will build on. The artifact JSON
/// must fit one NIP-44-encrypted relay event (65,535-byte plaintext cap);
/// stalling here keeps that failure priced-before-spend instead of a model
/// call whose output can never be published.
pub const MAX_PRIOR_OUTPUT_BYTES: usize = 40_000;

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
    /// Zero-spend price of `model_input` for the spec's model.
    pub estimate: Estimate,
    /// Total uncovered signals this plan drew from (`shown.len() <=` this).
    pub pending: usize,
}

/// Plan the next run of `spec`.
///
/// `prior` is the latest artifact version (if any), `covered` the union of
/// `shown_ids` across all prior versions, `fetched` the raw signals the
/// caller fetched for the selection's window, and `names` an optional
/// pubkey→display-name map for transcript lines.
pub fn plan_run(
    spec: &FoldSpec,
    prior: Option<&ArtifactPayload>,
    covered: &BTreeSet<String>,
    fetched: Vec<Signal>,
    names: &BTreeMap<String, String>,
) -> Result<Plan, Error> {
    if schema::builtin(&spec.schema).is_none() {
        return Err(Error::InvalidSpec(format!(
            "unknown schema {:?}",
            spec.schema
        )));
    }
    let signals = materialize(fetched);
    let new: Vec<Signal> = signals
        .into_iter()
        .filter(|s| !covered.contains(&s.id))
        .collect();
    let config_matches = prior.is_some_and(|p| {
        p.model == spec.model
            && p.schema == spec.schema
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
                "standing artifact is over the {MAX_PRIOR_OUTPUT_BYTES}-byte publish ceiling; \
                 compact it (tighter instructions, or start a fresh fold) before retrying"
            ),
            pending: new.len(),
        });
    }
    let parent = match prior {
        Some(p) => format!(
            "--- PRIOR DIGEST ({} v{}) ---\n{}",
            spec.name, p.version, p.output
        ),
        None => String::new(),
    };
    // A prior under the publish ceiling always leaves ample transcript budget
    // (40k bytes of parent against a 120k-char context).
    let raw_budget = MAX_CONTEXT_CHARS.saturating_sub(parent.chars().count() + 2);
    let render = render_transcript(&new, names, raw_budget, MAX_SHOWN_PER_RUN);
    if !new.is_empty() && render.shown.is_empty() {
        return Ok(Plan::Stalled {
            reason: "no pending event fits the remaining context budget".to_string(),
            pending: new.len(),
        });
    }
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
    let model_input = format!(
        "{}\n\n--- CONTEXT (time-ordered events) ---\n{}\n",
        spec.instructions, transcript
    );
    let est = estimate::estimate(&spec.model, model_input.chars().count());
    Ok(Plan::Ready(RunPlan {
        model_input,
        shown: render.shown,
        truncated: render.truncated,
        estimate: est,
        pending: new.len(),
    }))
}

/// Validate model `output` for `plan` and produce the next artifact version.
///
/// On any contract violation this returns [`Error::Nonconforming`] and the
/// caller must persist nothing. Coverage is computed from exactly the signals
/// the plan showed; append-section history from `prior` is spliced back
/// mechanically, so it survives regardless of what the model emitted.
pub fn complete_run(
    spec: &FoldSpec,
    prior: Option<&ArtifactPayload>,
    plan: &RunPlan,
    output: &str,
    created_at: i64,
) -> Result<ArtifactPayload, Error> {
    let sch = schema::builtin(&spec.schema)
        .ok_or_else(|| Error::InvalidSpec(format!("unknown schema {:?}", spec.schema)))?;
    let shown_ids: Vec<String> = plan.shown.iter().map(|s| s.id.clone()).collect();
    validate_output(sch, output, prior.map(|p| p.output.as_str()), &shown_ids)?;
    let spliced = splice_append_sections(sch, prior.map(|p| p.output.as_str()), output);
    // Taint travels with the chain: once a channel's events fold in, every
    // later version keeps carrying that channel, even after a selection edit.
    let channels: BTreeSet<String> = prior
        .map(|p| p.channels.iter().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default()
        .union(&spec.selection.channels.iter().cloned().collect())
        .cloned()
        .collect();
    Ok(ArtifactPayload {
        fold: spec.name.clone(),
        version: prior.map_or(1, |p| p.version + 1),
        output: spliced,
        coverage_since: plan.shown.iter().map(|s| s.created_at).min(),
        coverage_until: plan.shown.iter().map(|s| s.created_at).max().map(|t| t + 1),
        selection: spec.selection.clone(),
        channels: channels.into_iter().collect(),
        shown_ids,
        model: spec.model.clone(),
        schema: spec.schema.clone(),
        prompt_sha256: spec.prompt_sha256(),
        truncated: plan.truncated,
        created_at,
    })
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
                authors: vec![],
                kinds: vec![],
            },
            schema: "channel-digest@v1".to_string(),
            model: "haiku".to_string(),
            instructions: "Maintain the digest.".to_string(),
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
            schema: spec.schema.clone(),
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
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready, got {plan:?}");
        };
        assert!(run.shown.is_empty());
        assert!(!run.model_input.contains("--- SOURCE EVENT IDS ---"));
        assert!(run
            .model_input
            .contains("--- PRIOR DIGEST (team-digest v1) ---"));
        // No shown signals → citation contract not demanded, coverage stays empty.
        let v2 = complete_run(&spec, Some(&prior), &run, &digest(""), 1_700_000_200).expect("v2");
        assert_eq!(v2.version, 2);
        assert!(v2.shown_ids.is_empty());
        assert_eq!(v2.coverage_since, None);
        assert_eq!(v2.coverage_until, None);
        // The engine splice still carried the prior Log forward.
        assert!(v2.output.contains("- prior entry"));
    }

    #[test]
    fn stalled_when_standing_artifact_exceeds_publish_ceiling() {
        let spec = spec();
        let mut prior = artifact_v1(&spec, &[]);
        prior.output = "x".repeat(MAX_PRIOR_OUTPUT_BYTES + 1);
        let plan = plan_run(
            &spec,
            Some(&prior),
            &BTreeSet::new(),
            vec![sig(&hex_id('b'), 200, "pending")],
            &BTreeMap::new(),
        )
        .expect("plan");
        let Plan::Stalled { reason, pending } = plan else {
            panic!("expected Stalled, got Ready/Cached");
        };
        assert!(reason.contains("publish ceiling"));
        assert_eq!(pending, 1);
    }

    #[test]
    fn selection_change_is_not_cached() {
        let mut spec = spec();
        let a = hex_id('a');
        let prior = artifact_v1(&spec, &[(&a, 100)]);
        spec.selection.channels.push("ch2".to_string());
        let covered = BTreeSet::from([a]);
        let plan = plan_run(&spec, Some(&prior), &covered, vec![], &BTreeMap::new()).expect("plan");
        assert!(matches!(plan, Plan::Ready(_)), "got {plan:?}");
    }

    #[test]
    fn no_prior_and_no_signals_stalls_instead_of_folding_nothing() {
        let spec = spec();
        let plan = plan_run(&spec, None, &BTreeSet::new(), vec![], &BTreeMap::new()).expect("plan");
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
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        let out = digest(&format!("- new entry [event:{b}]"));
        let v2 = complete_run(&spec, Some(&prior), &run, &out, 1_700_000_500).expect("v2");
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
    fn oversized_backlog_chunks_honestly_and_never_seals_unread() {
        let spec = spec();
        // Enough large signals that they cannot all fit one run's budget.
        let fetched: Vec<Signal> = (0..40)
            .map(|i| sig(&format!("{i:064}"), 1_000 + i as i64, &"m".repeat(5_000)))
            .collect();
        let plan =
            plan_run(&spec, None, &BTreeSet::new(), fetched, &BTreeMap::new()).expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        assert!(run.truncated);
        assert!(run.shown.len() < 40, "must not claim the whole backlog");
        assert_eq!(run.pending, 40);
        // Coverage seals only what was shown; the rest stays pending.
        let cited = run.shown[0].id.clone();
        let out = digest(&format!("- chunk one [event:{cited}]"));
        let v1 = complete_run(&spec, None, &run, &out, 1_700_000_300).expect("v1");
        assert_eq!(v1.shown_ids.len(), run.shown.len());
        assert!(v1.truncated);
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
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        let out = digest(&format!("- said hello [event:{b}]"));
        let v1 = complete_run(&spec, None, &run, &out, 1_700_000_300).expect("v1");
        assert_eq!(v1.version, 1);
        assert_eq!(v1.shown_ids, vec![b]);
        assert_eq!(v1.coverage_since, Some(200));
        assert_eq!(v1.coverage_until, Some(201));
        assert_eq!(v1.prompt_sha256, spec.prompt_sha256());
    }

    #[test]
    fn complete_run_splices_prior_history_and_increments_version() {
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
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        // Model returns ONLY the new entry; engine must carry the prior Log.
        let out = digest(&format!("- new entry [event:{b}]"));
        let v2 = complete_run(&spec, Some(&prior), &run, &out, 1_700_000_400).expect("v2");
        assert_eq!(v2.version, 2);
        assert!(v2.output.contains("- prior entry"));
        assert!(v2.output.contains("- new entry"));
    }

    #[test]
    fn nonconforming_output_is_refused_and_nothing_persists() {
        let spec = spec();
        let b = hex_id('b');
        let plan = plan_run(
            &spec,
            None,
            &BTreeSet::new(),
            vec![sig(&b, 200, "hello")],
            &BTreeMap::new(),
        )
        .expect("plan");
        let Plan::Ready(run) = plan else {
            panic!("expected Ready");
        };
        // Fabricated citation.
        let fabricated = digest(&format!("- invented [event:{}]", hex_id('c')));
        assert!(matches!(
            complete_run(&spec, None, &run, &fabricated, 0),
            Err(Error::Nonconforming(_))
        ));
        // Wrong sections.
        assert!(matches!(
            complete_run(&spec, None, &run, "# Nope\n\nbody\n", 0),
            Err(Error::Nonconforming(_))
        ));
    }
}
