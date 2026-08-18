//! Independent translation of `docs/spec/MultiTenantRelay.tla`'s `Next`
//! transition relation into Rust.
//!
//! This module is the heart of the conformance gate. It is deliberately
//! **independent** of the production reducer: it reads only the trace
//! schema in [`crate`] and the spec text in `docs/spec/MultiTenantRelay.tla`.
//! It does not import `buzz-relay`, `buzz-db`, `buzz-auth`, or any other
//! production crate that could share a normalization bug with the emitter.
//!
//! ## What an "abstract state" means here
//!
//! The TLA+ spec models the relay as a multi-worker system whose state is
//! the set of accepted rows, projection rows, observations, etc. A runtime
//! trace covers ONE worker handling ONE request — so the model state we
//! carry is much smaller:
//!
//! - `resolved_community` — the server-resolved `TenantContext::community()`
//!   for this request. `Inv_NonInterference` requires every row label
//!   observed in this request be a subset of `{resolved_community}`.
//! - `bound_host` — the host label `TenantContext::host()` was bound from.
//!   `AuthCheck` / channel-less reads require `HostCommunity[host]` agree
//!   with the resolved community.
//!
//! The checker rebuilds this state independently from the FIRST trace step
//! it sees and then validates every subsequent step against it.
//!
//! ## Per-action obligations
//!
//! Each action has a triple of obligations distilled from the spec:
//!
//! 1. **State match.** `step.state_after.resolved_community` and
//!    `bound_host` agree with the checker's running model (no mid-request
//!    tenant flip).
//! 2. **Row-label confinement** (`Inv_NonInterference` line ~983,
//!    `Inv_ReadConfinement` line ~1003). Every `row_communities` entry,
//!    every accept label, must equal `resolved_community`. A single foreign
//!    label fails the trace.
//! 3. **Action-specific guards.** AuthCheck `Allow` requires host/channel
//!    agreement; channel-less reads require `HostCommunity[host] = c`;
//!    `WriteInsert` claim-vs-resolved is recorded but a mismatch is
//!    allowed at the abstract level — the spec ignores it ("host wins"),
//!    so the gate that bites mismatches is the row-label confinement on
//!    the *next* read.

use crate::{
    AbstractState, ChannelLabel, CommunityLabel, SanitizedReason, TraceAction, TraceStep, Verdict,
};

/// A judgment about a single trace step. The checker walks the trace and
/// returns the first failure verdict (fail-fast); per the skill's "fail
/// closed on the first violation" guidance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict_ {
    /// Reserved — internal placeholder.
    Ok,
}

/// Failure reasons returned by [`check_step`]. The string payload is
/// human-readable; mechanical consumers should match on the variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransitionError {
    /// The traced action is not allowed from the checker's current model
    /// state — e.g. an `AuthCheck { verdict: Allow }` with `claimed != real`.
    #[error("illegal transition at step {step_index}: {detail}")]
    IllegalTransition {
        /// 0-based index of the offending step.
        step_index: usize,
        /// Human-readable detail.
        detail: String,
    },

    /// The trace's `state_after` does not match the model state the checker
    /// computed independently. Indicates the relay either reassigned the
    /// tenant context mid-request, or emitted a step from a context other
    /// than `TenantContext`.
    #[error("state mismatch at step {step_index}: {detail}")]
    StateMismatch {
        /// 0-based index of the offending step.
        step_index: usize,
        /// Human-readable detail.
        detail: String,
    },

    /// Row labels include a community other than the resolved tenant —
    /// the master `Inv_NonInterference` failure.
    #[error("non-interference breach at step {step_index}: {detail}")]
    NonInterference {
        /// 0-based index of the offending step.
        step_index: usize,
        /// Human-readable detail.
        detail: String,
    },

    /// A coverage breach: ImplBug action, or a fixture-declared
    /// `required_critical_actions` entry never appeared.
    #[error("coverage breach: {detail}")]
    CoverageBreach {
        /// Human-readable detail naming the missing or broken coverage rule.
        detail: String,
    },
}

/// The model state the checker carries between steps.
#[derive(Debug, Clone)]
pub struct ModelState {
    /// The community the FIRST step's `state_after` told us was resolved.
    /// Subsequent steps must agree.
    pub resolved_community: CommunityLabel,
    /// The host the FIRST step's `state_after` told us was bound. Channel-
    /// bearing AuthCheck and channel-less reads enforce
    /// `host_community(host) == resolved_community`. The checker does NOT
    /// know `HostCommunity[_]` at large; it only knows the spec guarantees
    /// `HostCommunity[bound_host] = resolved_community` whenever the relay
    /// took the success branch.
    pub bound_host: crate::HostLabel,
    /// The actor for this request — opaque, equality-checked only.
    pub actor: crate::ActorLabel,
}

impl ModelState {
    /// Bootstrap the model from the very first step. Subsequent calls to
    /// [`check_step`] return a `StateMismatch` if `state_after` disagrees.
    pub fn bootstrap(first: &AbstractState) -> Self {
        Self {
            resolved_community: first.resolved_community,
            bound_host: first.bound_host.clone(),
            actor: first.actor.clone(),
        }
    }
}

/// Validate one step against the model. Updates nothing (the model is
/// immutable for the lifetime of a single trace); a violation returns the
/// matching [`TransitionError`].
///
/// Spec line numbers below refer to `docs/spec/MultiTenantRelay.tla` at the
/// snapshot pinned in this PR's `docs/spec/`.
pub fn check_step(
    step_index: usize,
    model: &ModelState,
    step: &TraceStep,
) -> Result<(), TransitionError> {
    // Universal obligation 1: state_after agrees with the bootstrapped model.
    if step.state_after.resolved_community != model.resolved_community {
        return Err(TransitionError::StateMismatch {
            step_index,
            detail: format!(
                "resolved_community changed mid-request: bootstrap={:?}, step={:?}",
                model.resolved_community, step.state_after.resolved_community
            ),
        });
    }
    if step.state_after.bound_host != model.bound_host {
        return Err(TransitionError::StateMismatch {
            step_index,
            detail: format!(
                "bound_host changed mid-request: bootstrap={:?}, step={:?}",
                model.bound_host, step.state_after.bound_host
            ),
        });
    }
    if step.state_after.actor != model.actor {
        return Err(TransitionError::StateMismatch {
            step_index,
            detail: format!(
                "actor changed mid-request: bootstrap={:?}, step={:?}",
                model.actor, step.state_after.actor
            ),
        });
    }

    // Action-specific obligations.
    match &step.action {
        // --- Spec WriteInsert (lines 514-550) ---
        // Resolution: real == ChannelCommunity(ch).
        // Success branch requires HostCommunity[host] = real, which the
        // emitter guarantees by emitting from inside the success path with
        // state_after.resolved_community = real and state_after.bound_host
        // = the bound host. The trace records claimed_community separately
        // so M2/M8 (host/channel disagreement, claim≠resolved) surface here
        // as a state mismatch on the *resolved* side.
        //
        // What we check at this step: nothing beyond the universal state
        // match. The spec ignores claimed_community ("host wins"), so a
        // mismatch is allowed at this exact action — the gate that bites
        // it is the next read's row labels.
        TraceAction::WriteInsert { .. } => Ok(()),

        // --- Spec WriteInsertGlobal (lines 559-595) ---
        // resolved == HostCommunity[host]. Same shape as WriteInsert.
        TraceAction::WriteInsertGlobal { .. } => Ok(()),

        // --- Spec WriteDuplicate (lines 606-637) ---
        // Carries the same host-axis obligation as WriteInsert: an A-host
        // presenting a B-channel id must not learn whether the id exists.
        // Same observable: state_after.resolved_community must be the
        // real ChannelCommunity(ch), enforced by the universal check.
        TraceAction::WriteDuplicate { .. } => Ok(()),

        // --- Spec SanitizedError (line 778) ---
        // Closed-alphabet reason; labels = {}; carries no row data. The
        // emitter must collapse every reject path into one of the three
        // SanitizedReason variants. The schema-level type system already
        // enforces that — we just check the variant is among the spec's
        // closed set (trivially true by construction).
        TraceAction::SanitizedError { reason } => match reason {
            SanitizedReason::Restricted
            | SanitizedReason::Invalid
            | SanitizedReason::ServerError => Ok(()),
        },

        // --- Spec AuthCheck (lines 794-810) ---
        // real == ChannelCommunity(ch).
        // hostAgrees == real ∈ Communities ∧ HostCommunity[host] = real.
        // allowed   == hostAgrees ∧ ch ∈ ScopedAccessible(real, a).
        // verdict   == IF allowed THEN Allow ELSE Deny.
        //
        // The runtime checker cannot recompute ScopedAccessible (that's
        // production state). What it CAN check: when verdict = Allow, the
        // claimed_community MUST equal resolved_community. This is the
        // M2 bite ("auth verdict driven by claimed instead of resolved")
        // and the M8 bite ("A-host driving a B-channel verdict") — both
        // collapse to "Allow with a foreign label leak".
        //
        // We deliberately do NOT bite Deny on claim mismatch (Deny with
        // any claim is in-spec — the spec models Deny as the catch-all
        // for hostAgrees=false or accessibility=false).
        TraceAction::AuthCheck {
            channel: _,
            claimed_community,
            verdict,
        } => match (verdict, claimed_community) {
            (Verdict::Allow, Some(c)) if c != &model.resolved_community => {
                Err(TransitionError::IllegalTransition {
                    step_index,
                    detail: format!(
                        "AuthCheck verdict=Allow with claimed_community={:?} != resolved={:?} \
                         — M2/M8 (claim or host driving verdict) bite",
                        c, model.resolved_community
                    ),
                })
            }
            _ => Ok(()),
        },

        // --- Spec ReadMessageRows (line 643) / ReadByIdRows (line 681) ---
        // The action emits rows; `RowLabels(rows)` is the observation's
        // labels and Inv_NonInterference requires labels ⊆ {community}.
        // For channel-less (ch = NoChannel) the spec ADDS:
        //   HostCommunity[host] = c ∧ IsAdmitted(c, a).
        // The host-agreement piece is enforced at the universal check
        // (state_after.resolved_community is host-derived); IsAdmitted is
        // production state we cannot recompute, so it lives in fixture
        // assertions rather than this generic checker.
        //
        // What this checker bites: every row label must equal the
        // resolved community. ONE foreign label fails NI.
        TraceAction::ReadMessageRows {
            channel: _,
            row_communities,
        }
        | TraceAction::ReadByIdRows {
            channel: _,
            row_communities,
        } => check_row_labels(step_index, model, row_communities),

        // --- Spec ReadHostFeedRows (line ~720) ---
        // Community is host-derived; same row-label confinement.
        TraceAction::ReadHostFeedRows { row_communities } => {
            check_row_labels(step_index, model, row_communities)
        }

        // --- Coverage breach via the Drop guard ---
        // The seam exited without emitting any recognized action.
        // Per the skill: this is the load-bearing coverage mode — without
        // it, trace conformance is decorative logging.
        TraceAction::ImplBug { kind } => Err(TransitionError::CoverageBreach {
            detail: format!("ImplBug action emitted by Drop guard: kind={kind:?}"),
        }),
    }
}

/// Row-label confinement check shared by all three read actions.
///
/// `Inv_NonInterference` (spec line ~983):
///   `\A o \in observations : o.labels \subseteq {o.community}`.
///
/// Translated: every `row_communities` entry must equal `model
/// .resolved_community`. The check is on a `Vec`, not a `Set`, deliberately
/// — if a buggy relay returned the same foreign row twice the checker still
/// bites, and if a buggy emitter de-duped foreign labels to one occurrence
/// the checker still bites. Foreign-label count is unimportant; foreign-
/// label presence is the entire bar.
fn check_row_labels(
    step_index: usize,
    model: &ModelState,
    row_communities: &[CommunityLabel],
) -> Result<(), TransitionError> {
    if let Some(foreign) = row_communities
        .iter()
        .find(|c| **c != model.resolved_community)
    {
        return Err(TransitionError::NonInterference {
            step_index,
            detail: format!(
                "row labeled {:?} returned in observation scoped to {:?} \
                 — Inv_NonInterference breach (foreign row leaked through tenant fence)",
                foreign, model.resolved_community
            ),
        });
    }
    Ok(())
}

/// Helper: which channel (if any) does the action target? Used by the
/// checker to bind cross-step claims to a stable channel — and by fixtures
/// asserting that a particular channel surfaced at this seam.
pub fn action_channel(action: &TraceAction) -> Option<&ChannelLabel> {
    match action {
        TraceAction::WriteInsert { channel, .. } => Some(channel),
        TraceAction::WriteDuplicate { channel, .. } => Some(channel),
        TraceAction::AuthCheck { channel, .. } => Some(channel),
        TraceAction::ReadMessageRows { channel, .. } => channel.as_ref(),
        TraceAction::ReadByIdRows { channel, .. } => channel.as_ref(),
        TraceAction::WriteInsertGlobal { .. }
        | TraceAction::ReadHostFeedRows { .. }
        | TraceAction::SanitizedError { .. }
        | TraceAction::ImplBug { .. } => None,
    }
}
