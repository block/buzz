//! Runtime trace schema and independent replay checker for the Git land seam.
//!
//! A push is decided once and published once. The relay reads the authority
//! state (rules, roles, grants, evidence), decides each ref update, then
//! moves the repository pointer with a compare-and-swap. If authority
//! changes between the read and the swap, the land happened under authority
//! that no longer held.
//!
//! The trace makes that gap observable. Every decision records the
//! `read_version` of the per-repo authority row it read under a shared lock,
//! and the `valid_until` of the authority it relied on; every pointer swap
//! records the `cas_version`, re-read from the database after the swap, and
//! `swapped_at`, the swap statement's own clock. Authority is judged at that
//! statement, which takes the pointer row; a revoke cannot fall between it
//! and the commit, because it waits on the shared lock. The checker fails a
//! won land whose two versions differ or whose swap is not before
//! `valid_until`. `swapped_at` is read after the statement's own expiry
//! check, so near the boundary the rule can fail an honest land but never
//! pass a late one.
//!
//! `PointerObserved` is the stored pointer read from the database. The
//! checker fails a trace in which it moved with no won land to move it, and
//! every decision must end in a receipt that records the evidence and checks
//! it counted.
//!
//! The same independence rule as the rest of this crate applies: nothing
//! here calls production decision code. The land protocol these rules check
//! is the row protocol in `docs/spec/GitGovernanceLand.tla`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::transitions::TransitionError;
use crate::{ActorLabel, OpaqueId};

/// Land-seam schema version. Bump on any backwards-incompatible change.
pub const LAND_SCHEMA_VERSION: u32 = 1;

/// The decision for one ref update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LandVerdict {
    /// The update may land.
    Allow,
    /// The update is refused.
    Deny,
}

/// Outcome of the pointer compare-and-swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CasOutcome {
    /// The pointer moved to the new state.
    Won,
    /// Another writer moved the pointer first; nothing was published.
    Lost,
}

/// One action at the land seam.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LandAction {
    /// The relay decided one ref update of a push.
    Decide {
        /// Opaque id of the push, shared by every action it causes.
        push: OpaqueId,
        /// Opaque repository label.
        repo: OpaqueId,
        /// The ref being updated.
        git_ref: String,
        /// The tip the update would publish, opaque.
        new_tip: OpaqueId,
        /// The key that pushed.
        actor: ActorLabel,
        /// Version of the authority state the decision's inputs were read
        /// from. Taken from the same snapshot as the inputs, never re-read
        /// at emit time.
        read_version: u64,
        /// The decision.
        verdict: LandVerdict,
        /// The earliest moment, in database milliseconds, at which any
        /// authority the decision relied on lapses with no write (a grant's
        /// expiry), or `null` if none does. Required on the wire: an emitter
        /// that forgets it must not read as "nothing expires".
        #[serde(deserialize_with = "Option::deserialize")]
        valid_until: Option<u64>,
        /// The approvals the decision counted, opaque.
        evidence: BTreeSet<OpaqueId>,
        /// The newest trusted passing result it relied on for each required
        /// check, opaque.
        checks: BTreeSet<OpaqueId>,
    },
    /// The relay attempted to publish a push by swapping the pointer.
    Land {
        /// The push being published.
        push: OpaqueId,
        /// Every ref the swap publishes, with the tip it publishes.
        refs: BTreeMap<String, OpaqueId>,
        /// Version of the authority state in force when the swap took effect.
        cas_version: u64,
        /// Database time, in milliseconds, returned by the pointer swap
        /// statement itself (`RETURNING clock_timestamp()`). It is read
        /// after the statement's own expiry check, so a land judged against
        /// it can be refused wrongly near an expiry but never accepted
        /// wrongly.
        swapped_at: u64,
        /// Whether the swap took effect.
        cas: CasOutcome,
    },
    /// What the repository's pointer names, read from storage rather than
    /// reported by the process that moved it. A pointer that moved with no
    /// won land in between was moved by something the seam never saw, such
    /// as a swap that completed after its sender died.
    PointerObserved {
        /// Opaque repository label.
        repo: OpaqueId,
        /// Every ref the pointer names, with its tip.
        refs: BTreeMap<String, OpaqueId>,
    },
    /// The durable record of one decision.
    Receipt {
        /// The push the decision belongs to.
        push: OpaqueId,
        /// The ref the decision was about.
        git_ref: String,
        /// The verdict recorded; must equal the decision's.
        verdict: LandVerdict,
        /// The approvals recorded; must equal the ones the decision counted.
        evidence: BTreeSet<OpaqueId>,
        /// The check results recorded; must equal the ones it relied on.
        checks: BTreeSet<OpaqueId>,
    },
    /// The seam exited without recording anything. Emitted by [`LandGuard`].
    ImplBug {
        /// The seam that exited silently.
        seam: String,
    },
}

impl LandAction {
    /// A short stable name for the action kind.
    pub fn kind(&self) -> &'static str {
        match self {
            LandAction::Decide { .. } => "decide",
            LandAction::Land { .. } => "land",
            LandAction::Receipt { .. } => "receipt",
            LandAction::PointerObserved { .. } => "pointer_observed",
            LandAction::ImplBug { .. } => "impl_bug",
        }
    }
}

/// One step in a land-seam trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandStep {
    /// Schema version; must equal [`LAND_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The action at the seam.
    pub action: LandAction,
}

impl LandStep {
    /// A step at the current schema version.
    pub fn new(action: LandAction) -> Self {
        Self {
            schema_version: LAND_SCHEMA_VERSION,
            action,
        }
    }
}

/// The emit trait the relay calls at the land seam.
pub trait LandTracer: Send + Sync {
    /// Record one step.
    fn record(&self, step: LandStep);

    /// Whether recorded steps are observed. Callers may skip building emit
    /// inputs when this is false, never skip an emit when it is true.
    fn enabled(&self) -> bool {
        true
    }
}

/// The production tracer: records nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopLandTracer;

impl LandTracer for NoopLandTracer {
    fn record(&self, _step: LandStep) {}

    fn enabled(&self) -> bool {
        false
    }
}

/// Coverage guard for one entry into the land seam.
///
/// Arm it where the seam is entered and pass the returned tracer down the
/// request path. If the request records nothing before the guard drops, the
/// guard records [`LandAction::ImplBug`], which the checker fails.
pub struct LandGuard {
    inner: Arc<dyn LandTracer>,
    count: Arc<AtomicUsize>,
    seam: &'static str,
}

struct CountingLandTracer {
    inner: Arc<dyn LandTracer>,
    count: Arc<AtomicUsize>,
}

impl LandTracer for CountingLandTracer {
    fn record(&self, step: LandStep) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.inner.record(step);
    }

    // Delegate: whether steps are observed belongs to the tracer underneath.
    fn enabled(&self) -> bool {
        self.inner.enabled()
    }
}

impl LandGuard {
    /// Arm a guard for `seam`. Returns the guard and the counting tracer the
    /// request must record through.
    pub fn arm(tracer: Arc<dyn LandTracer>, seam: &'static str) -> (Self, Arc<dyn LandTracer>) {
        let count = Arc::new(AtomicUsize::new(0));
        let counting: Arc<dyn LandTracer> = Arc::new(CountingLandTracer {
            inner: tracer.clone(),
            count: count.clone(),
        });
        let guard = Self {
            inner: tracer,
            count,
            seam,
        };
        (guard, counting)
    }
}

impl Drop for LandGuard {
    fn drop(&mut self) {
        if self.count.load(Ordering::Relaxed) == 0 {
            self.inner.record(LandStep::new(LandAction::ImplBug {
                seam: self.seam.to_string(),
            }));
        }
    }
}

/// Check a land-seam trace.
///
/// `required` names action kinds the scenario must contain; a missing one is
/// a coverage breach, so a seam that was never armed cannot pass silently.
pub fn check_land_trace(trace: &[LandStep], required: &[&str]) -> Result<(), TransitionError> {
    if trace.is_empty() {
        return Err(breach("empty land trace"));
    }
    // (push, ref) -> the decision seen for it.
    let mut decided: BTreeMap<(&OpaqueId, &str), Decided<'_>> = BTreeMap::new();
    // (push, ref) -> recorded verdict.
    let mut receipts: BTreeMap<(&OpaqueId, &str), LandVerdict> = BTreeMap::new();
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    // push -> repo, from its decisions.
    let mut repo_of: BTreeMap<&OpaqueId, &OpaqueId> = BTreeMap::new();
    // repo -> what its pointer must name, once observed.
    let mut pointer: BTreeMap<&OpaqueId, BTreeMap<String, OpaqueId>> = BTreeMap::new();

    for (i, step) in trace.iter().enumerate() {
        if step.schema_version != LAND_SCHEMA_VERSION {
            return Err(illegal(
                i,
                format!(
                    "schema version {} is not {LAND_SCHEMA_VERSION}",
                    step.schema_version
                ),
            ));
        }
        seen.insert(step.action.kind());
        match &step.action {
            LandAction::Decide {
                push,
                repo,
                git_ref,
                new_tip,
                read_version,
                verdict,
                valid_until,
                evidence,
                checks,
                ..
            } => {
                if *repo_of.entry(push).or_insert(repo) != repo {
                    return Err(illegal(
                        i,
                        format!("push decided for two repos at {git_ref}"),
                    ));
                }
                let d = Decided {
                    tip: new_tip,
                    read_version: *read_version,
                    valid_until: *valid_until,
                    verdict: *verdict,
                    evidence,
                    checks,
                };
                if decided.insert((push, git_ref.as_str()), d).is_some() {
                    return Err(illegal(i, format!("{git_ref} decided twice in one push")));
                }
            }
            LandAction::Land {
                push,
                refs,
                cas_version,
                swapped_at,
                cas,
            } => {
                if *cas == CasOutcome::Lost {
                    continue;
                }
                if refs.is_empty() {
                    return Err(illegal(i, "won land publishes no ref".into()));
                }
                for (git_ref, tip) in refs {
                    let Some(d) = decided.get(&(push, git_ref.as_str())) else {
                        return Err(illegal(i, format!("{git_ref} landed without a decision")));
                    };
                    if d.verdict != LandVerdict::Allow {
                        return Err(illegal(i, format!("{git_ref} landed after a deny")));
                    }
                    if d.tip != tip {
                        return Err(illegal(
                            i,
                            format!("{git_ref} landed a tip it did not decide"),
                        ));
                    }
                    if d.read_version != *cas_version {
                        return Err(TransitionError::StateMismatch {
                            step_index: i,
                            detail: format!(
                                "{git_ref} decided at authority version {} \
                                 but landed at {cas_version}",
                                d.read_version
                            ),
                        });
                    }
                    if let Some(until) = d.valid_until {
                        if *swapped_at >= until {
                            return Err(TransitionError::StateMismatch {
                                step_index: i,
                                detail: format!(
                                    "{git_ref} landed at {swapped_at} on authority that \
                                     lapsed at {until}"
                                ),
                            });
                        }
                    }
                }
                if let Some(state) = repo_of.get(push).and_then(|r| pointer.get_mut(r)) {
                    state.extend(refs.iter().map(|(r, t)| (r.clone(), t.clone())));
                }
            }
            LandAction::PointerObserved { repo, refs } => match pointer.get(repo) {
                None => {
                    pointer.insert(repo, refs.clone());
                }
                Some(expected) if expected != refs => {
                    return Err(TransitionError::StateMismatch {
                        step_index: i,
                        detail: "the pointer moved with no won land to move it".into(),
                    });
                }
                Some(_) => {}
            },
            LandAction::Receipt {
                push,
                git_ref,
                verdict,
                evidence,
                checks,
            } => {
                let Some(d) = decided.get(&(push, git_ref.as_str())) else {
                    return Err(illegal(
                        i,
                        format!("receipt for {git_ref} with no decision"),
                    ));
                };
                if d.verdict != *verdict {
                    return Err(illegal(
                        i,
                        format!("receipt for {git_ref} records the wrong verdict"),
                    ));
                }
                if d.evidence != evidence {
                    return Err(illegal(
                        i,
                        format!("receipt for {git_ref} does not record the approvals counted"),
                    ));
                }
                if d.checks != checks {
                    return Err(illegal(
                        i,
                        format!("receipt for {git_ref} does not record the checks relied on"),
                    ));
                }
                if receipts
                    .insert((push, git_ref.as_str()), *verdict)
                    .is_some()
                {
                    return Err(illegal(i, format!("{git_ref} has two receipts")));
                }
            }
            LandAction::ImplBug { seam } => {
                return Err(breach(&format!("{seam} exited without recording a step")));
            }
        }
    }

    if let Some(((_, git_ref), _)) = decided.iter().find(|(k, _)| !receipts.contains_key(*k)) {
        return Err(breach(&format!("decision for {git_ref} has no receipt")));
    }
    if let Some(missing) = required.iter().find(|k| !seen.contains(**k)) {
        return Err(breach(&format!("required action {missing} never appeared")));
    }
    Ok(())
}

struct Decided<'a> {
    tip: &'a OpaqueId,
    read_version: u64,
    valid_until: Option<u64>,
    verdict: LandVerdict,
    evidence: &'a BTreeSet<OpaqueId>,
    checks: &'a BTreeSet<OpaqueId>,
}

fn illegal(step_index: usize, detail: String) -> TransitionError {
    TransitionError::IllegalTransition { step_index, detail }
}

fn breach(detail: &str) -> TransitionError {
    TransitionError::CoverageBreach {
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests;
