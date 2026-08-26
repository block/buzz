//! Preflight garbage collection (spec §K8s GC, `docs/remote-agents.md:1282-1335`).
//!
//! GC runs on every deploy, after identity derivation and before the state
//! transition. It deletes terminated pods (and their referenced Secrets) and
//! age-eligible orphan Secrets — every one of which must pass the full-pubkey
//! annotation check *and* carry the management marker. An unmarked object is
//! never GC'd regardless of its labels.
//!
//! The decision layer here is pure. The effectful caller supplies the observed
//! objects and the apiserver's clock; this module decides what may be deleted.

use crate::naming::AgentIdentity;
use crate::observe::{referenced_secret, secret_is_ours};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Pod, Secret};

/// The deploy operation deadline (spec §Deploy: `timeout: 600s`).
pub const OPERATION_DEADLINE_SECS: i64 = 600;

/// An unreferenced Secret is GC-eligible only once it is older than **twice**
/// the deploy deadline. Rationale: Secret-create → pod-create is not atomic
/// against an independent GC pass, so without the gate a concurrent attempt's
/// preflight GC can delete a Secret whose pod has not been created yet and
/// strand that deploy. The age bound makes "unreferenced" mean "provably
/// abandoned" — any attempt that could still reference it has exceeded its own
/// deadline (`:1301-1319`).
pub const ORPHAN_SECRET_MIN_AGE_SECS: i64 = 2 * OPERATION_DEADLINE_SECS;

/// What a GC pass decided to delete. Names only: the caller re-reads each
/// object's own fence at delete time.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcPlan {
    /// Terminated, verified, marker-bearing pods.
    pub pods: Vec<String>,
    /// Age-eligible, verified, marker-bearing orphan Secrets.
    pub secrets: Vec<String>,
}

/// Plan a GC pass.
///
/// `now` is the apiserver's clock — the HTTP `Date` header from the very list
/// call that produced `secrets`. `None` means the header was absent or
/// unparseable, in which case **orphan-Secret GC is skipped entirely** rather
/// than falling back to local time: this provider runs on a user's desktop,
/// and a local clock fast by more than the margin does not race — it
/// deterministically computes every in-flight Secret as expired, on every
/// pass, reopening exactly the interleaving the gate exists to close
/// (`:1321-1335`). A deferred cleanup is free; a wrong deletion is not.
///
/// Terminated-pod GC does not use the clock and is unaffected.
pub fn plan(
    identity: &AgentIdentity,
    pods: &[Pod],
    secrets: &[Secret],
    terminated: impl Fn(&Pod) -> bool,
    now: Option<DateTime<Utc>>,
) -> GcPlan {
    // Only pods that pass the full fence participate — in either direction.
    // An unverified pod is neither deleted nor allowed to protect a Secret:
    // it cannot be ours, so its `envFrom` cannot reference our generation.
    let ours: Vec<&Pod> = pods
        .iter()
        .filter(|p| {
            crate::observe::verify(p, identity, crate::classify::Startup::Started).is_some()
        })
        .collect();

    let doomed_pods: Vec<&&Pod> = ours.iter().filter(|p| terminated(p)).collect();

    // A Secret referenced by ANY existing pod is protected — deliberately
    // including not-yet-started pods, whose `envFrom` is exactly as
    // load-bearing as a running pod's (`:1262-1264`). Pods being GC'd in this
    // same pass are excluded, so their Secrets go with them.
    let doomed_names: Vec<&str> = doomed_pods
        .iter()
        .filter_map(|p| p.metadata.name.as_deref())
        .collect();
    let protected: Vec<String> = ours
        .iter()
        .filter(|p| !doomed_names.contains(&p.metadata.name.as_deref().unwrap_or_default()))
        .filter_map(|p| referenced_secret(p))
        .collect();

    let mut plan = GcPlan {
        pods: doomed_names.iter().map(|n| n.to_string()).collect(),
        secrets: doomed_pods
            .iter()
            .filter_map(|p| referenced_secret(p))
            .collect(),
    };

    // Orphan sweep: only with a server clock.
    if let Some(now) = now {
        for secret in secrets {
            if !secret_is_ours(secret, identity) {
                continue;
            }
            let Some(name) = secret.metadata.name.as_deref() else {
                continue;
            };
            if protected.contains(&name.to_string()) || plan.secrets.iter().any(|s| s == name) {
                continue;
            }
            let Some(created) = secret.metadata.creation_timestamp.as_ref() else {
                // No server-assigned timestamp means no age proof. Skip.
                continue;
            };
            if (now - created.0).num_seconds() >= ORPHAN_SECRET_MIN_AGE_SECS {
                plan.secrets.push(name.to_string());
            }
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::naming::{ANNOTATION_PUBKEY_FULL, LABEL_MANAGED_BY};
    use k8s_openapi::api::core::v1::{Container, EnvFromSource, PodSpec, SecretEnvSource};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
    use std::collections::BTreeMap;

    fn identity() -> AgentIdentity {
        use nostr::nips::nip19::ToBech32;
        let keys = nostr::Keys::generate();
        AgentIdentity::from_nsec(&keys.secret_key().to_bech32().unwrap()).unwrap()
    }

    fn pod_named(id: &AgentIdentity, name: &str, secret: Option<&str>) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(name.into()),
                uid: Some(format!("uid-{name}")),
                resource_version: Some("1".into()),
                labels: Some(id.labels()),
                annotations: Some(
                    [(
                        ANNOTATION_PUBKEY_FULL.to_string(),
                        id.pubkey_hex().to_string(),
                    )]
                    .into_iter()
                    .collect::<BTreeMap<_, _>>(),
                ),
                ..Default::default()
            },
            spec: secret.map(|s| PodSpec {
                containers: vec![Container {
                    name: "agent".into(),
                    env_from: Some(vec![EnvFromSource {
                        secret_ref: Some(SecretEnvSource {
                            name: s.into(),
                            optional: Some(false),
                        }),
                        ..Default::default()
                    }]),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn secret_named(id: &AgentIdentity, name: &str, age_secs: i64, now: DateTime<Utc>) -> Secret {
        Secret {
            metadata: ObjectMeta {
                name: Some(name.into()),
                labels: Some(id.labels()),
                annotations: Some(
                    [(
                        ANNOTATION_PUBKEY_FULL.to_string(),
                        id.pubkey_hex().to_string(),
                    )]
                    .into_iter()
                    .collect::<BTreeMap<_, _>>(),
                ),
                creation_timestamp: Some(Time(now - chrono::Duration::seconds(age_secs))),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn never(_: &Pod) -> bool {
        false
    }
    fn always(_: &Pod) -> bool {
        true
    }

    #[test]
    fn terminated_pods_and_their_secrets_are_collected_together() {
        let id = identity();
        let now = Utc::now();
        let pod = pod_named(&id, "buzz-agent-dead", Some("buzz-agent-dead-gen1"));
        let plan = plan(&id, &[pod], &[], always, Some(now));
        assert_eq!(plan.pods, ["buzz-agent-dead"]);
        assert_eq!(plan.secrets, ["buzz-agent-dead-gen1"]);
    }

    #[test]
    fn live_pods_are_never_collected() {
        let id = identity();
        let pod = pod_named(&id, "buzz-agent-live", Some("buzz-agent-live-gen1"));
        let plan = plan(&id, &[pod], &[], never, Some(Utc::now()));
        assert_eq!(plan, GcPlan::default());
    }

    /// The auto-repair fence applies to GC identically: an object that lacks
    /// the marker, or carries a different pubkey, is never touched — however
    /// well its labels match.
    #[test]
    fn unmarked_and_mismatched_objects_are_never_collected() {
        let id = identity();
        let other = identity();
        let now = Utc::now();

        let mut unmarked = pod_named(&id, "look-alike", Some("look-alike-gen1"));
        let mut labels = id.labels();
        labels.remove(LABEL_MANAGED_BY);
        unmarked.metadata.labels = Some(labels);

        let mut foreign = pod_named(&id, "someone-elses", Some("someone-elses-gen1"));
        foreign.metadata.annotations = Some(
            [(
                ANNOTATION_PUBKEY_FULL.to_string(),
                other.pubkey_hex().to_string(),
            )]
            .into_iter()
            .collect(),
        );

        let mut unmarked_secret = secret_named(&id, "orphan-unmarked", 100_000, now);
        unmarked_secret.metadata.labels = Some(BTreeMap::new());
        let mut foreign_secret = secret_named(&id, "orphan-foreign", 100_000, now);
        foreign_secret.metadata.annotations = Some(
            [(
                ANNOTATION_PUBKEY_FULL.to_string(),
                other.pubkey_hex().to_string(),
            )]
            .into_iter()
            .collect(),
        );

        let plan = plan(
            &id,
            &[unmarked, foreign],
            &[unmarked_secret, foreign_secret],
            always,
            Some(now),
        );
        assert_eq!(
            plan,
            GcPlan::default(),
            "GC touched an object it does not own"
        );
    }

    /// The interleaving the age gate exists to close: attempt A creates its
    /// Secret; concurrent attempt B's preflight GC runs before A creates its
    /// pod. Without the gate B deletes A's Secret and strands A.
    #[test]
    fn young_unreferenced_secrets_are_protected() {
        let id = identity();
        let now = Utc::now();
        let fresh = secret_named(&id, "buzz-agent-x-gen-inflight", 5, now);
        assert_eq!(
            plan(&id, &[], &[fresh], never, Some(now)),
            GcPlan::default()
        );
    }

    /// Past twice the deadline, any attempt that could still reference the
    /// Secret has exceeded its own deadline — so it is provably abandoned.
    #[test]
    fn secrets_older_than_twice_the_deadline_are_collected() {
        let id = identity();
        let now = Utc::now();
        let old = secret_named(
            &id,
            "buzz-agent-x-gen-abandoned",
            ORPHAN_SECRET_MIN_AGE_SECS + 1,
            now,
        );
        let plan = plan(&id, &[], &[old], never, Some(now));
        assert_eq!(plan.secrets, ["buzz-agent-x-gen-abandoned"]);
    }

    /// The boundary itself, both sides. `>= 1200s` is eligible.
    #[test]
    fn age_gate_boundary_is_exact() {
        let id = identity();
        let now = Utc::now();
        let just_under = secret_named(&id, "under", ORPHAN_SECRET_MIN_AGE_SECS - 1, now);
        let exactly = secret_named(&id, "exact", ORPHAN_SECRET_MIN_AGE_SECS, now);
        assert!(plan(&id, &[], &[just_under], never, Some(now))
            .secrets
            .is_empty());
        assert_eq!(
            plan(&id, &[], &[exactly], never, Some(now)).secrets,
            ["exact"]
        );
    }

    /// The same-clock rule. No apiserver `Date` header → skip the orphan
    /// sweep entirely. A local clock fast by more than the margin would
    /// silently delete every in-flight Secret on every pass.
    #[test]
    fn without_a_server_clock_the_orphan_sweep_is_skipped() {
        let id = identity();
        let now = Utc::now();
        let ancient = secret_named(&id, "buzz-agent-x-gen-ancient", 10_000_000, now);
        let plan = plan(&id, &[], &[ancient], never, None);
        assert!(
            plan.secrets.is_empty(),
            "orphan swept without a server clock — a fast local clock would delete live Secrets"
        );
    }

    /// ...but terminated-pod GC does not consult the clock, so it still runs.
    #[test]
    fn terminated_pod_gc_runs_without_a_server_clock() {
        let id = identity();
        let pod = pod_named(&id, "buzz-agent-dead", Some("buzz-agent-dead-gen1"));
        let plan = plan(&id, &[pod], &[], always, None);
        assert_eq!(plan.pods, ["buzz-agent-dead"]);
        assert_eq!(plan.secrets, ["buzz-agent-dead-gen1"]);
    }

    /// "Existing" includes not-yet-started pods: a Secret referenced by a pod
    /// still pulling its image must not be swept, however old it is.
    #[test]
    fn secrets_referenced_by_a_pending_pod_are_protected() {
        let id = identity();
        let now = Utc::now();
        let pending = pod_named(&id, "buzz-agent-pending", Some("buzz-agent-pending-gen1"));
        let old = secret_named(&id, "buzz-agent-pending-gen1", 10_000_000, now);
        let plan = plan(&id, &[pending], &[old], never, Some(now));
        assert!(plan.secrets.is_empty(), "swept a referenced Secret");
    }

    /// A Secret with no server-assigned creationTimestamp has no age proof,
    /// so it is skipped rather than assumed old.
    #[test]
    fn secrets_without_a_creation_timestamp_are_skipped() {
        let id = identity();
        let now = Utc::now();
        let mut no_timestamp = secret_named(&id, "buzz-agent-x-gen-unknown", 10_000_000, now);
        no_timestamp.metadata.creation_timestamp = None;
        assert!(plan(&id, &[], &[no_timestamp], never, Some(now))
            .secrets
            .is_empty());
    }

    /// A Secret belonging to a pod being collected in this same pass goes with
    /// it, and must not be listed twice.
    #[test]
    fn a_collected_pods_secret_is_listed_once() {
        let id = identity();
        let now = Utc::now();
        let dead = pod_named(&id, "buzz-agent-dead", Some("buzz-agent-dead-gen1"));
        let its_secret = secret_named(&id, "buzz-agent-dead-gen1", 10_000_000, now);
        let plan = plan(&id, &[dead], &[its_secret], always, Some(now));
        assert_eq!(plan.secrets, ["buzz-agent-dead-gen1"]);
    }
}
