//! Property/fuzz-generated conformance traces.
//!
//! These tests widen the checker's exercised input space beyond the hand-
//! built fixtures in `tests/fixtures/`. The skill (skill-runtime-formal-
//! compliance) calls for "property/fuzz-generated action sequences where
//! feasible"; this is that lane.
//!
//! ## Design: invariant properties, NOT a parallel oracle
//!
//! `transitions::check_step` is small and direct. A "reference oracle" that
//! re-derived the verdict would just be a copy of the checker — testing the
//! code against itself, proving nothing. So these tests do NOT re-implement
//! the verdict. They assert **spec-derived facts** about `check_trace`'s
//! result, read off the *shape of the generated trace*:
//!
//! - any read carrying a foreign row label MUST be rejected (NonInterference)
//! - a fully clean trace MUST be accepted
//! - AuthCheck Allow + foreign claim MUST bite (IllegalTransition)
//! - ImplBug MUST bite (CoverageBreach)
//! - a mid-trace state flip MUST bite (StateMismatch)
//! - the checker never panics and is deterministic
//!
//! The only checker surface these tests touch is the public
//! [`buzz_conformance::checker::check_trace`]. They never call
//! `transitions::check_step`, and they never depend on a production crate.
//!
//! ## Fail-fast discipline
//!
//! `check_trace` returns the FIRST error it finds. So every property that
//! asserts a *specific* error variant must construct traces in which the
//! targeted violation is the first/only one — otherwise an earlier
//! `StateMismatch` / `IllegalTransition` / `CoverageBreach` would mask the
//! variant under test. Each generator below is built to honor that.

use buzz_conformance::checker::{check_trace, Scenario};
use buzz_conformance::transitions::TransitionError;
use buzz_conformance::{
    AbstractState, ActorLabel, ChannelLabel, CommunityLabel, HostLabel, OpaqueId, SanitizedReason,
    TraceAction, TraceStep, Verdict,
};
use proptest::prelude::*;
use uuid::Uuid;

// --- Small fixed pools -----------------------------------------------------
//
// Pools are intentionally tiny (3 each) so that "foreign vs resolved"
// collisions happen with meaningful frequency. With a 3-community pool a
// randomly chosen row label is foreign ~2/3 of the time, so P1 actually
// stresses the leak path instead of almost always generating clean traces.

const POOL: u128 = 3;

fn community(i: u128) -> CommunityLabel {
    CommunityLabel::from_uuid(Uuid::from_u128(
        0x0c00_0000_0000_0000_0000_0000_0000_0000 + i,
    ))
}

fn channel(i: u128) -> ChannelLabel {
    ChannelLabel(Uuid::from_u128(
        0x0ca0_0000_0000_0000_0000_0000_0000_0000 + i,
    ))
}

fn host(i: u128) -> HostLabel {
    HostLabel(format!("h_{i}"))
}

fn actor(i: u128) -> ActorLabel {
    ActorLabel(format!("a_{i}"))
}

fn arb_community() -> impl Strategy<Value = CommunityLabel> {
    (0..POOL).prop_map(community)
}

fn arb_channel() -> impl Strategy<Value = ChannelLabel> {
    (0..POOL).prop_map(channel)
}

fn arb_opaque() -> impl Strategy<Value = OpaqueId> {
    (0u32..16).prop_map(|i| OpaqueId(format!("m{i}")))
}

fn arb_verdict() -> impl Strategy<Value = Verdict> {
    prop_oneof![Just(Verdict::Allow), Just(Verdict::Deny)]
}

fn arb_reason() -> impl Strategy<Value = SanitizedReason> {
    prop_oneof![
        Just(SanitizedReason::Restricted),
        Just(SanitizedReason::Invalid),
        Just(SanitizedReason::ServerError),
    ]
}

/// The bootstrapped state for a request resolved to `resolved`. Host/actor
/// are fixed so that, when we reuse this state for every step, the only way
/// a `StateMismatch` can arise is if a property deliberately flips a field.
fn state_for(resolved: CommunityLabel) -> AbstractState {
    AbstractState {
        resolved_community: resolved,
        bound_host: host(0),
        actor: actor(0),
    }
}

// --- Action generators -----------------------------------------------------

/// A "clean" action: one whose presence in a trace bootstrapped to
/// `resolved` introduces NO violation on its own. Read labels are all
/// `resolved`; AuthCheck either Denies (any claim) or Allows with a claim
/// equal to `resolved` (or no claim). No ImplBug. This is the alphabet P2
/// draws from, and the benign filler P1/P3/P4/P5 use for prefixes.
fn arb_clean_action(resolved: CommunityLabel) -> impl Strategy<Value = TraceAction> {
    let res = resolved;
    prop_oneof![
        (arb_opaque(), arb_channel(), prop::option::of(Just(res))).prop_map(
            |(msg_id, channel, claimed_community)| TraceAction::WriteInsert {
                msg_id,
                channel,
                claimed_community,
            }
        ),
        (arb_opaque(), prop::option::of(Just(res))).prop_map(|(msg_id, claimed_community)| {
            TraceAction::WriteInsertGlobal {
                msg_id,
                claimed_community,
            }
        }),
        (arb_opaque(), arb_channel(), prop::option::of(Just(res))).prop_map(
            |(msg_id, channel, claimed_community)| TraceAction::WriteDuplicate {
                msg_id,
                channel,
                claimed_community,
            }
        ),
        arb_reason().prop_map(|reason| TraceAction::SanitizedError { reason }),
        // AuthCheck that cannot bite M2/M8: either Deny (any claim is in-spec)
        // or Allow with a claim that is None or equal to resolved.
        (
            arb_channel(),
            arb_verdict(),
            prop_oneof![Just(None), Just(Some(res))],
        )
            .prop_map(|(channel, verdict, claimed_community)| {
                // For Deny, the claim is unconstrained; for Allow it is
                // None-or-resolved by construction above, so it never bites.
                TraceAction::AuthCheck {
                    channel,
                    claimed_community,
                    verdict,
                }
            }),
        // Reads whose every row label equals resolved.
        (arb_channel(), 0usize..4).prop_map(move |(channel, n)| TraceAction::ReadMessageRows {
            channel: Some(channel),
            row_communities: vec![res; n],
        }),
        (0usize..4).prop_map(move |n| TraceAction::ReadByIdRows {
            channel: None,
            row_communities: vec![res; n],
        }),
        (0usize..4).prop_map(move |n| TraceAction::ReadHostFeedRows {
            row_communities: vec![res; n],
        }),
    ]
}

/// Wrap actions into steps that all share the bootstrapped state, so the
/// only violations possible are action-level (no incidental StateMismatch).
fn steps_with_state(actions: Vec<TraceAction>, resolved: CommunityLabel) -> Vec<TraceStep> {
    let st = state_for(resolved);
    actions
        .into_iter()
        .map(|a| TraceStep::new(a, st.clone()))
        .collect()
}

/// A clean trace: 1..=12 clean actions over one resolved community, all
/// sharing the bootstrap state. By construction this contains no foreign
/// label, no Allow+foreign claim, no ImplBug, no state flip, no schema
/// mismatch.
fn arb_clean_trace() -> impl Strategy<Value = (CommunityLabel, Vec<TraceStep>)> {
    arb_community().prop_flat_map(|resolved| {
        prop::collection::vec(arb_clean_action(resolved), 1..=12)
            .prop_map(move |actions| (resolved, steps_with_state(actions, resolved)))
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// P2 — completeness / no false reject.
    /// A fully clean, non-empty, current-schema, consistent-state trace with
    /// no coverage obligations MUST be accepted. `Scenario::unstructured`
    /// declares no required actions, so coverage breach cannot fire.
    #[test]
    fn clean_trace_is_accepted((_resolved, trace) in arb_clean_trace()) {
        let sc = Scenario::unstructured(trace);
        prop_assert!(
            check_trace(&sc).is_ok(),
            "clean trace was rejected: {:?}",
            check_trace(&sc)
        );
    }

    /// P1 — non-interference soundness / no false accept of a leak.
    /// A clean prefix followed by a single read whose row set contains a
    /// foreign label MUST be rejected with NonInterference. The foreign read
    /// is the only possible violation, so fail-fast surfaces exactly it.
    #[test]
    fn foreign_row_label_is_rejected(
        resolved in arb_community(),
        foreign_idx in 0u128..POOL,
        prefix in prop::collection::vec(arb_community().prop_map(|_| ()), 0..6),
        clean_before in any::<bool>(),
        which_read in 0u8..3,
    ) {
        // Pick a foreign community distinct from resolved.
        let foreign = {
            let mut f = community(foreign_idx);
            if f == resolved {
                f = community((foreign_idx + 1) % POOL);
            }
            f
        };
        // If POOL were 1 this could still collide; guard explicitly.
        prop_assume!(foreign != resolved);

        let mut actions: Vec<TraceAction> = Vec::new();
        // Optional benign clean prefix (reads of resolved-only rows) to prove
        // the violation still bites after valid steps.
        if clean_before {
            for _ in &prefix {
                actions.push(TraceAction::ReadMessageRows {
                    channel: Some(channel(0)),
                    row_communities: vec![resolved],
                });
            }
        }
        // The single violating read carries one foreign label. NI confinement
        // is enforced on ALL THREE read surfaces (they share `check_row_labels`),
        // so the property must bite regardless of which read leaked.
        let leaked = vec![resolved, foreign];
        let violating = match which_read {
            0 => TraceAction::ReadMessageRows {
                channel: Some(channel(0)),
                row_communities: leaked,
            },
            1 => TraceAction::ReadByIdRows {
                channel: None,
                row_communities: leaked,
            },
            _ => TraceAction::ReadHostFeedRows {
                row_communities: leaked,
            },
        };
        actions.push(violating);

        let trace = steps_with_state(actions, resolved);
        let err = check_trace(&Scenario::unstructured(trace)).unwrap_err();
        prop_assert!(
            matches!(err, TransitionError::NonInterference { .. }),
            "expected NonInterference, got {err:?}"
        );
    }

    /// P3a — AuthCheck Allow + foreign claim always bites IllegalTransition.
    /// One-step trace so the M2/M8 bite is the only candidate.
    #[test]
    fn auth_allow_foreign_claim_bites(
        resolved in arb_community(),
        foreign_idx in 0u128..POOL,
        chan in arb_channel(),
    ) {
        let foreign = {
            let mut f = community(foreign_idx);
            if f == resolved {
                f = community((foreign_idx + 1) % POOL);
            }
            f
        };
        prop_assume!(foreign != resolved);

        let trace = steps_with_state(
            vec![TraceAction::AuthCheck {
                channel: chan,
                claimed_community: Some(foreign),
                verdict: Verdict::Allow,
            }],
            resolved,
        );
        let err = check_trace(&Scenario::unstructured(trace)).unwrap_err();
        prop_assert!(
            matches!(err, TransitionError::IllegalTransition { .. }),
            "expected IllegalTransition for Allow+foreign claim, got {err:?}"
        );
    }

    /// P3b — AuthCheck Deny with any claim is in-spec (never bites on the
    /// claim axis). One-step clean-otherwise trace MUST be accepted.
    #[test]
    fn auth_deny_any_claim_is_ok(
        resolved in arb_community(),
        claim_idx in 0u128..POOL,
        chan in arb_channel(),
        has_claim in any::<bool>(),
    ) {
        let claimed = if has_claim { Some(community(claim_idx)) } else { None };
        let trace = steps_with_state(
            vec![TraceAction::AuthCheck {
                channel: chan,
                claimed_community: claimed,
                verdict: Verdict::Deny,
            }],
            resolved,
        );
        prop_assert!(
            check_trace(&Scenario::unstructured(trace)).is_ok(),
            "Deny with any claim should be in-spec"
        );
    }

    /// P4 — ImplBug always bites CoverageBreach. Clean prefix then ImplBug;
    /// since the prefix is clean, the ImplBug is the first/only violation.
    #[test]
    fn impl_bug_bites_coverage_breach(
        resolved in arb_community(),
        prefix_len in 0usize..4,
        kind in "[a-z_]{1,16}",
    ) {
        let mut actions: Vec<TraceAction> = (0..prefix_len)
            .map(|_| TraceAction::ReadMessageRows {
                channel: Some(channel(0)),
                row_communities: vec![resolved],
            })
            .collect();
        actions.push(TraceAction::ImplBug { kind });

        let trace = steps_with_state(actions, resolved);
        let err = check_trace(&Scenario::unstructured(trace)).unwrap_err();
        prop_assert!(
            matches!(err, TransitionError::CoverageBreach { .. }),
            "expected CoverageBreach from ImplBug, got {err:?}"
        );
    }

    /// P5 — a mid-trace state flip bites StateMismatch. One clean bootstrap
    /// step, then a benign action whose `state_after` flips exactly one of
    /// resolved_community / bound_host / actor. State is checked before any
    /// action-specific logic, so this is the only possible violation.
    #[test]
    fn state_flip_bites_state_mismatch(
        resolved in arb_community(),
        other_idx in 0u128..POOL,
        which in 0u8..3,
    ) {
        let boot = state_for(resolved);
        // A benign first step.
        let step0 = TraceStep::new(
            TraceAction::ReadMessageRows {
                channel: Some(channel(0)),
                row_communities: vec![resolved],
            },
            boot.clone(),
        );

        // Flip exactly one field for step 1.
        let mut flipped = boot.clone();
        match which {
            0 => {
                let mut other = community(other_idx);
                if other == resolved {
                    other = community((other_idx + 1) % POOL);
                }
                prop_assume!(other != resolved);
                flipped.resolved_community = other;
            }
            1 => flipped.bound_host = host(9),
            _ => flipped.actor = actor(9),
        }
        let step1 = TraceStep::new(
            TraceAction::ReadMessageRows {
                channel: Some(channel(0)),
                // Use the FLIPPED resolved so the read itself is clean
                // relative to its own state_after; the bite must come from
                // the state divergence, not row labels.
                row_communities: vec![flipped.resolved_community],
            },
            flipped,
        );

        let err = check_trace(&Scenario::unstructured(vec![step0, step1])).unwrap_err();
        prop_assert!(
            matches!(err, TransitionError::StateMismatch { .. }),
            "expected StateMismatch from a mid-trace field flip, got {err:?}"
        );
    }

    /// P6 — determinism and no-panic. Running `check_trace` twice on the same
    /// scenario yields the same result, and neither call panics. Draws from
    /// the clean alphabet plus occasional violations so the input space is
    /// broad; we assert nothing about the verdict, only its stability.
    #[test]
    fn check_trace_is_deterministic_and_total(
        resolved in arb_community(),
        actions in prop::collection::vec(
            prop_oneof![
                arb_clean_action(community(0)),
                // a few intentionally-violating shapes to widen coverage
                Just(TraceAction::ImplBug { kind: "fuzz".into() }),
                arb_community().prop_map(|c| TraceAction::ReadMessageRows {
                    channel: None,
                    row_communities: vec![c],
                }),
            ],
            1..=12,
        ),
    ) {
        let trace = steps_with_state(actions, resolved);
        let sc = Scenario::unstructured(trace);
        let r1 = check_trace(&sc);
        let r2 = check_trace(&sc);
        prop_assert_eq!(
            format!("{r1:?}"),
            format!("{r2:?}"),
            "check_trace was non-deterministic"
        );
    }
}
