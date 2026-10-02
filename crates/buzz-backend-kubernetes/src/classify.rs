//! The deploy state machine (spec §Deploy State Machine), as a pure function.
//!
//! `classify` maps a verified observation plus the desired create intent to
//! one [`Action`]. It performs no I/O, so every row of the spec's table is a
//! unit test with no cluster. `reconcile` executes actions and re-enters.
//!
//! Two invariants are structural rather than remembered:
//!
//! * [`Action::Delete`] carries the [`Fence`] from the exact observation that
//!   authorized it. There is no way to build a delete without one, so a later
//!   helper cannot re-read and silently substitute a fresher fence.
//! * The pull-failure classifier ([`PullFailure`]) reaches only
//!   [`Action::Report`] and [`Action::Observe`]. It is absent from
//!   `Action::Delete`'s type, so "reason strings are never deletion
//!   authority" is enforced by the compiler.

use crate::intent::Fingerprint;

/// The compare-and-delete fence: UID + resourceVersion from the observation
/// that authorized the deletion. A failed precondition means the object
/// changed since the read — re-enter, never retry the delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    pub uid: String,
    pub resource_version: String,
}

/// Why a pod that never started looks permanently broken. Reporting only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullFailure {
    /// Registry auth: a 403/401 `ImagePullBackOff` retries forever without
    /// ever succeeding, so "the pull retries" is false for this case.
    Unauthorized,
    /// The digest or repository does not exist at that registry.
    ManifestUnknown,
    /// The image has no variant for the node's architecture.
    ArchMismatch,
}

/// The container's startup state, already decoded from pod status. Decoding
/// happens at the edge so this module stays free of API types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    /// `state.running` — the harness process is up. This, not pod phase, is
    /// what "live" means.
    Started,
    /// Started once and reached a terminal phase (Succeeded/Failed).
    Terminated,
    /// Never started, and self-healing is plausible: unschedulable during
    /// scale-from-zero, an image pull in progress, a transient
    /// `CreateContainerConfigError` whose Secret exists.
    NeverStartedRecoverable,
    /// Never started, and the provider *verified* the cause — not a reason
    /// string. Either the referenced Secret is confirmed absent by a
    /// most-recent read, or the image reference is structurally invalid.
    NeverStartedProvablyBroken,
    /// Never started; the pull is failing in a way that will not self-heal.
    /// Still recoverable in the *never delete* sense — this only changes what
    /// we report and how long we wait.
    NeverStartedPullFailing(PullFailure),
}

/// A pod that passed identity and ownership verification: label-selected,
/// full-pubkey annotation equal to the derived pubkey, management marker
/// present. Constructing this type is the verification step's output, so an
/// unverified object cannot reach `classify` at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPod {
    pub name: String,
    pub fence: Fence,
    /// Set once the apiserver accepts a delete. In Kubernetes there is no
    /// `Terminating` phase — a pod being gracefully deleted stays in phase
    /// `Running` for its whole grace period — so this must be checked
    /// *before* startup state or the dying pod reads as the no-op row.
    pub deletion_marked: bool,
    pub startup: Startup,
    /// The `buzz.block.xyz/create-intent` annotation as recorded at create.
    /// `None` for a pod written before the annotation existed, which counts
    /// as divergence.
    pub recorded_intent: Option<Fingerprint>,
}

/// What the reconciler should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Create the pod, then wait for the harness container to start.
    Create,
    /// Compare-and-delete, poll for actual disappearance, then re-enter.
    Delete { name: String, fence: Fence },
    /// Wait for a deletion already in flight, then re-enter.
    AwaitDisappearance { name: String },
    /// Strict no-op: return this `agent_id`, mutate nothing.
    NoOp { agent_id: String },
    /// Keep observing until started or the operation deadline expires; on
    /// expiry report the latest condition. Never deletes, on this call or any
    /// later one.
    Observe { name: String },
    /// Surface an actionable condition immediately rather than burning the
    /// deadline on a failure that will not self-heal.
    Report { name: String, failure: PullFailure },
}

/// Apply the spec's ordered rules to one verified observation.
///
/// `desired` is the freshly computed create intent; comparison is always
/// recorded-annotation vs freshly-computed, never a diff against the live pod
/// (admission defaulting would make every pod look divergent).
pub fn classify(observed: Option<&VerifiedPod>, desired: &Fingerprint) -> Action {
    let Some(pod) = observed else {
        // Row: no instance → create. First deploy, or after GC.
        return Action::Create;
    };

    // Row: deletion-marked, ANY phase. Checked before startup state because
    // there is no `Terminating` phase to match on — a gracefully deleting pod
    // reports phase `Running` throughout its grace period, so testing startup
    // first would mistake it for the live no-op row and return an id that
    // evaporates.
    if pod.deletion_marked {
        return Action::AwaitDisappearance {
            name: pod.name.clone(),
        };
    }

    match &pod.startup {
        // Row: live and started → strict no-op. Start must never kill a live
        // agent mid-turn, whatever the fingerprint says.
        Startup::Started => Action::NoOp {
            agent_id: pod.name.clone(),
        },

        // Row: terminated → delete residue, then re-enter to create. This is
        // the normal restart path — how a user revives a reaped agent.
        Startup::Terminated => Action::Delete {
            name: pod.name.clone(),
            fence: pod.fence.clone(),
        },

        // Row: never started, provably non-recoverable → fenced replace.
        // "Provably" means a verified absence or a structural defect, never a
        // reason string.
        Startup::NeverStartedProvablyBroken => Action::Delete {
            name: pod.name.clone(),
            fence: pod.fence.clone(),
        },

        // Inside the recoverable row: a pull that will not self-heal is
        // reported immediately instead of consuming the 600s deadline. This
        // changes reporting and wait behavior only — no delete authority.
        Startup::NeverStartedPullFailing(failure) => Action::Report {
            name: pod.name.clone(),
            failure: *failure,
        },

        // Row: never started, recoverable — split on create-intent
        // divergence. Divergence is evidence of a config change the user is
        // waiting on, and it is the *only* thing that replaces a
        // never-started pod. Pod age triggers nothing: any finite age
        // threshold collides with Cluster Autoscaler's own pod-age delays,
        // and delete-recreate resets exactly the age it keys on.
        Startup::NeverStartedRecoverable => {
            if pod.recorded_intent.as_ref() == Some(desired) {
                Action::Observe {
                    name: pod.name.clone(),
                }
            } else {
                Action::Delete {
                    name: pod.name.clone(),
                    fence: pod.fence.clone(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(seed: &str) -> Fingerprint {
        Fingerprint::for_test(seed)
    }

    fn pod(startup: Startup, intent: Option<Fingerprint>) -> VerifiedPod {
        VerifiedPod {
            name: "buzz-agent-abc123def456".into(),
            fence: Fence {
                uid: "uid-1".into(),
                resource_version: "rv-1".into(),
            },
            deletion_marked: false,
            startup,
            recorded_intent: intent,
        }
    }

    #[test]
    fn no_instance_creates() {
        assert_eq!(classify(None, &fp("a")), Action::Create);
    }

    #[test]
    fn started_pod_is_strict_no_op() {
        let p = pod(Startup::Started, Some(fp("a")));
        assert_eq!(
            classify(Some(&p), &fp("a")),
            Action::NoOp {
                agent_id: p.name.clone()
            }
        );
    }

    /// The asymmetry the spec states plainly: an edit cannot reach a started
    /// pod until it exits, but it *can* reach a never-started one — the
    /// never-started pod is the one the user is editing because it did not
    /// start.
    #[test]
    fn started_pod_no_ops_even_when_intent_diverges() {
        let p = pod(Startup::Started, Some(fp("old")));
        assert_eq!(
            classify(Some(&p), &fp("new")),
            Action::NoOp {
                agent_id: p.name.clone()
            }
        );
    }

    /// In Kubernetes a gracefully deleting pod stays in phase `Running`. If
    /// the deletion mark were checked after startup state, this pod would
    /// take the no-op row and `deploy` would return an id that evaporates.
    #[test]
    fn deletion_mark_beats_every_startup_state() {
        for startup in [
            Startup::Started,
            Startup::Terminated,
            Startup::NeverStartedRecoverable,
            Startup::NeverStartedProvablyBroken,
            Startup::NeverStartedPullFailing(PullFailure::Unauthorized),
        ] {
            let mut p = pod(startup.clone(), Some(fp("a")));
            p.deletion_marked = true;
            assert_eq!(
                classify(Some(&p), &fp("a")),
                Action::AwaitDisappearance {
                    name: p.name.clone()
                },
                "deletion mark ignored for {startup:?}"
            );
        }
    }

    #[test]
    fn terminated_pod_is_replaced() {
        let p = pod(Startup::Terminated, Some(fp("a")));
        assert_eq!(
            classify(Some(&p), &fp("a")),
            Action::Delete {
                name: p.name.clone(),
                fence: p.fence.clone()
            }
        );
    }

    /// A never-started winner is repairable: pod exists, Secret confirmed
    /// absent, container never started. A later deploy must delete-recreate
    /// rather than no-op — the test that pins started-not-phase as the no-op
    /// criterion.
    #[test]
    fn provably_broken_never_started_pod_is_replaced() {
        let p = pod(Startup::NeverStartedProvablyBroken, Some(fp("a")));
        assert_eq!(
            classify(Some(&p), &fp("a")),
            Action::Delete {
                name: p.name.clone(),
                fence: p.fence.clone()
            }
        );
    }

    /// The anti-livelock rule: identical desired intent means *never* delete,
    /// however long the pod has been pending. Age is not an input to this
    /// function at all, which is the strongest way to say so.
    #[test]
    fn recoverable_with_matching_intent_only_observes() {
        let p = pod(Startup::NeverStartedRecoverable, Some(fp("same")));
        assert_eq!(
            classify(Some(&p), &fp("same")),
            Action::Observe {
                name: p.name.clone()
            }
        );
    }

    /// Repeated identical Starts can never delete anything — the same
    /// classification, arbitrarily many times.
    #[test]
    fn repeated_identical_starts_never_delete() {
        let p = pod(Startup::NeverStartedRecoverable, Some(fp("same")));
        for _ in 0..100 {
            assert!(!matches!(
                classify(Some(&p), &fp("same")),
                Action::Delete { .. }
            ));
        }
    }

    /// The wedge escape: the user corrected a resource request or image, so
    /// the never-started pod is built from configuration they have since
    /// changed. Without this row the edit could never materialize.
    #[test]
    fn recoverable_with_divergent_intent_is_replaced() {
        let p = pod(Startup::NeverStartedRecoverable, Some(fp("old")));
        assert_eq!(
            classify(Some(&p), &fp("new")),
            Action::Delete {
                name: p.name.clone(),
                fence: p.fence.clone()
            }
        );
    }

    /// A pod predating the annotation has no recorded intent — that is
    /// absence, which the spec groups with divergence.
    #[test]
    fn missing_recorded_intent_counts_as_divergence() {
        let p = pod(Startup::NeverStartedRecoverable, None);
        assert_eq!(
            classify(Some(&p), &fp("any")),
            Action::Delete {
                name: p.name.clone(),
                fence: p.fence.clone()
            }
        );
    }

    /// Permanent-looking pull failures report immediately instead of burning
    /// 600s — and, critically, never delete.
    #[test]
    fn pull_failures_report_and_never_delete() {
        for failure in [
            PullFailure::Unauthorized,
            PullFailure::ManifestUnknown,
            PullFailure::ArchMismatch,
        ] {
            let p = pod(Startup::NeverStartedPullFailing(failure), Some(fp("a")));
            // Divergent intent too — still no delete from this arm.
            for desired in [fp("a"), fp("different")] {
                assert_eq!(
                    classify(Some(&p), &desired),
                    Action::Report {
                        name: p.name.clone(),
                        failure
                    }
                );
            }
        }
    }

    /// Every delete carries the fence from the observation that authorized
    /// it. Exhaustive over the delete-producing states, so a future arm that
    /// forgets is caught here rather than in a cluster.
    #[test]
    fn every_delete_carries_the_authorizing_fence() {
        let states = [
            (Startup::Terminated, fp("a")),
            (Startup::NeverStartedProvablyBroken, fp("a")),
            (Startup::NeverStartedRecoverable, fp("divergent")),
        ];
        for (startup, desired) in states {
            let p = pod(startup, Some(fp("a")));
            match classify(Some(&p), &desired) {
                Action::Delete { fence, .. } => assert_eq!(fence, p.fence),
                other => panic!("expected Delete, got {other:?}"),
            }
        }
    }
}
