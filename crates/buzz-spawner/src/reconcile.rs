//! Desired state (specs) versus actual state (containers).
//!
//! The diff is a pure function so it can be tested without Docker or a relay.
//! Everything that performs I/O lives in [`crate::daemon`]; this module only
//! decides *what* should happen.

use std::collections::{HashMap, HashSet};

use buzz_sdk::spawner::SpawnerAgentSpec;
use sha2::{Digest, Sha256};

use crate::{container::ManagedContainer, store::AgentRecord};

/// A spec the spawner is responsible for, paired with its author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredAgent {
    /// Spec slug (`d` tag).
    pub slug: String,
    /// Spec author pubkey, hex.
    pub owner_pubkey: String,
    /// The spec body.
    pub spec: SpawnerAgentSpec,
}

impl DesiredAgent {
    /// Stable content hash of the spec, used to detect drift.
    ///
    /// Hashes the serialized spec rather than the event id: an owner can
    /// republish an identical spec (a NIP-33 replacement with a new timestamp
    /// and id), and that must not restart a healthy agent.
    pub fn spec_hash(&self) -> String {
        let json = serde_json::to_string(&self.spec).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// One unit of work the reconciler should perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Mint a keypair and open an attestation handshake.
    Provision {
        /// The spec to provision for.
        desired: DesiredAgent,
    },
    /// Re-send an attestation request whose previous round timed out.
    ReRequestAttestation {
        /// Owner pubkey.
        owner_pubkey: String,
        /// Spec slug.
        slug: String,
    },
    /// Create the container for an attested agent.
    Start {
        /// The spec to start.
        desired: DesiredAgent,
    },
    /// Replace a container.
    Restart {
        /// The spec to restart against.
        desired: DesiredAgent,
        /// Container to remove first.
        container_id: String,
        /// True when the container died on its own rather than being replaced
        /// because its spec changed.
        ///
        /// The two look identical at the Docker layer but mean opposite things.
        /// A drifted container was healthy and is being deliberately replaced;
        /// a crashed one failed and must count toward backoff. Without the
        /// distinction a container that starts and immediately exits is
        /// recreated on every reconcile pass forever, because creating it
        /// always "succeeds".
        crashed: bool,
    },
    /// Stop a container the spec disabled, keeping identity and state.
    Stop {
        /// Owner pubkey.
        owner_pubkey: String,
        /// Spec slug.
        slug: String,
        /// Container to remove.
        container_id: String,
    },
    /// Tear an agent down entirely — its spec is gone.
    Delete {
        /// Owner pubkey.
        owner_pubkey: String,
        /// Spec slug.
        slug: String,
        /// Container to remove, if one exists.
        container_id: Option<String>,
    },
    /// Remove a container with no corresponding record at all.
    RemoveOrphan {
        /// Container to remove.
        container_id: String,
    },
}

/// Inputs to a reconciliation pass.
pub struct ReconcileInput<'a> {
    /// Specs addressed to this spawner, from the relay.
    pub desired: &'a [DesiredAgent],
    /// What the spawner has minted so far.
    pub records: &'a [AgentRecord],
    /// Containers currently on the host carrying this spawner's label.
    pub containers: &'a [ManagedContainer],
    /// Current unix time.
    pub now: i64,
    /// Attestation timeout, seconds.
    pub attestation_timeout_secs: i64,
    /// Cap on concurrently running agents.
    pub max_agents: usize,
    /// Whether `desired` reflects the relay's full set of specs yet.
    ///
    /// False between startup and the specs subscription reaching EOSE. In that
    /// window an absent spec means "not delivered yet", not "deleted", and the
    /// two are indistinguishable from the desired-state map alone. Acting on
    /// the wrong reading destroys an agent's container, volume, and secret key
    /// on every restart — the agent comes back with a new identity and loses
    /// its channel membership and attestation.
    pub desired_hydrated: bool,
}

/// Backoff before retrying a failed start, capped so a permanently broken agent
/// still gets an occasional retry without hammering the Docker daemon.
pub fn backoff_secs(restart_count: u32) -> i64 {
    const CAP: i64 = 600;
    let exp = 15i64.saturating_mul(1 << restart_count.min(6));
    exp.min(CAP)
}

/// Compute the actions needed to bring actual state in line with desired state.
///
/// Ordering matters: removals are emitted before creations so a pass that both
/// deletes and provisions frees its agent-cap slot first.
pub fn plan(input: ReconcileInput<'_>) -> Vec<Action> {
    let records: HashMap<(&str, &str), &AgentRecord> = input
        .records
        .iter()
        .map(|r| ((r.owner_pubkey.as_str(), r.slug.as_str()), r))
        .collect();

    let containers_by_agent: HashMap<&str, &ManagedContainer> = input
        .containers
        .iter()
        .map(|c| (c.agent_pubkey.as_str(), c))
        .collect();

    let desired_keys: HashSet<(&str, &str)> = input
        .desired
        .iter()
        .map(|d| (d.owner_pubkey.as_str(), d.slug.as_str()))
        .collect();

    let mut actions = Vec::new();

    // Destructive actions are gated on hydration. Everything below this block
    // is additive or idempotent and is safe to run against partial desired
    // state; deletion is neither, and is irreversible.
    if input.desired_hydrated {
        // 1. Records whose spec disappeared — the owner deleted it.
        for record in input.records {
            if desired_keys.contains(&(record.owner_pubkey.as_str(), record.slug.as_str())) {
                continue;
            }
            actions.push(Action::Delete {
                owner_pubkey: record.owner_pubkey.clone(),
                slug: record.slug.clone(),
                container_id: containers_by_agent
                    .get(record.agent_pubkey.as_str())
                    .map(|c| c.id.clone()),
            });
        }

        // 2. Containers with no record at all. These are unreachable: the
        // spawner has no key for them, so it can neither manage nor speak for
        // them.
        let known_agents: HashSet<&str> = input
            .records
            .iter()
            .map(|r| r.agent_pubkey.as_str())
            .collect();
        for container in input.containers {
            if !known_agents.contains(container.agent_pubkey.as_str()) {
                actions.push(Action::RemoveOrphan {
                    container_id: container.id.clone(),
                });
            }
        }
    }

    // A pass that deletes frees capacity, so count only the agents that survive.
    let mut running_budget = input.max_agents.saturating_sub(
        input
            .records
            .iter()
            .filter(|r| {
                desired_keys.contains(&(r.owner_pubkey.as_str(), r.slug.as_str()))
                    && containers_by_agent.contains_key(r.agent_pubkey.as_str())
            })
            .count(),
    );

    for desired in input.desired {
        let key = (desired.owner_pubkey.as_str(), desired.slug.as_str());
        let Some(record) = records.get(&key) else {
            // 3. Never seen — mint a key and open the handshake.
            actions.push(Action::Provision {
                desired: desired.clone(),
            });
            continue;
        };

        // A spec that names an identity different from the one on record means
        // the agent was relocated here after this spawner had already minted
        // one for the same slug. Spec drift alone would restart the *wrong*
        // identity forever, so the minted stand-in has to be torn down and the
        // named agent adopted in its place.
        if desired
            .spec
            .agent_pubkey
            .as_deref()
            .is_some_and(|wanted| !wanted.eq_ignore_ascii_case(&record.agent_pubkey))
        {
            actions.push(Action::Delete {
                owner_pubkey: desired.owner_pubkey.clone(),
                slug: desired.slug.clone(),
                container_id: containers_by_agent
                    .get(record.agent_pubkey.as_str())
                    .map(|c| c.id.clone()),
            });
            actions.push(Action::Provision {
                desired: desired.clone(),
            });
            continue;
        }

        let container = containers_by_agent.get(record.agent_pubkey.as_str());

        // 4. Disabled: stop the container but keep the identity, so re-enabling
        // resumes the same agent rather than creating a stranger.
        if !desired.spec.enabled {
            if let Some(container) = container {
                actions.push(Action::Stop {
                    owner_pubkey: desired.owner_pubkey.clone(),
                    slug: desired.slug.clone(),
                    container_id: container.id.clone(),
                });
            }
            continue;
        }

        // 5. Not yet attested — chase the handshake rather than starting.
        if !record.is_attested() {
            if crate::attestation::is_attestation_expired(
                record,
                input.now,
                input.attestation_timeout_secs,
            ) {
                actions.push(Action::ReRequestAttestation {
                    owner_pubkey: desired.owner_pubkey.clone(),
                    slug: desired.slug.clone(),
                });
            }
            continue;
        }

        let hash = desired.spec_hash();

        match container {
            // 6. Running with a stale spec — replace it.
            Some(container) if record.spec_hash.as_deref() != Some(hash.as_str()) => {
                actions.push(Action::Restart {
                    desired: desired.clone(),
                    container_id: container.id.clone(),
                    crashed: false,
                });
            }
            // 7. Present and current, but the container died out of band.
            Some(container) if !container.running => {
                if !in_backoff(record, input.now) {
                    actions.push(Action::Restart {
                        desired: desired.clone(),
                        container_id: container.id.clone(),
                        crashed: true,
                    });
                }
            }
            // 8. Healthy. Nothing to do.
            Some(_) => {}
            // 9. Attested but not running.
            None => {
                if in_backoff(record, input.now) {
                    continue;
                }
                if running_budget == 0 {
                    continue;
                }
                running_budget -= 1;
                actions.push(Action::Start {
                    desired: desired.clone(),
                });
            }
        }
    }

    actions
}

fn in_backoff(record: &AgentRecord, now: i64) -> bool {
    match record.last_failure_at {
        Some(failed_at) => now.saturating_sub(failed_at) < backoff_secs(record.restart_count),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_sdk::spawner::RespondTo;

    fn spec(enabled: bool, parallelism: u32) -> SpawnerAgentSpec {
        SpawnerAgentSpec {
            name: "Fizz".into(),
            agent_pubkey: None,
            persona_id: Some("builtin:fizz".into()),
            system_prompt: None,
            model: None,
            provider: None,
            parallelism,
            respond_to: RespondTo::Anyone,
            respond_to_allowlist: vec![],
            resources: None,
            enabled,
        }
    }

    fn desired(slug: &str, enabled: bool, parallelism: u32) -> DesiredAgent {
        DesiredAgent {
            slug: slug.into(),
            owner_pubkey: "b".repeat(64),
            spec: spec(enabled, parallelism),
        }
    }

    fn record(slug: &str, agent: &str, attested: bool, hash: Option<&str>) -> AgentRecord {
        AgentRecord {
            slug: slug.into(),
            owner_pubkey: "b".repeat(64),
            agent_pubkey: agent.into(),
            private_key_nsec: "nsec1x".into(),
            auth_tag: attested.then(|| r#"["auth","o","","s"]"#.to_string()),
            pending_nonce: (!attested).then(|| "n".repeat(64)),
            attestation_sent_at: (!attested).then_some(1_000),
            spec_hash: hash.map(str::to_string),
            prompt: None,
            restart_count: 0,
            last_failure_at: None,
            carried_team_instructions: None,
        }
    }

    fn container(agent: &str, running: bool) -> ManagedContainer {
        ManagedContainer {
            id: format!("ctr-{agent}"),
            name: format!("buzz-agent-{agent}"),
            agent_pubkey: agent.into(),
            slug: "fizz".into(),
            running,
        }
    }

    fn input<'a>(
        desired: &'a [DesiredAgent],
        records: &'a [AgentRecord],
        containers: &'a [ManagedContainer],
    ) -> ReconcileInput<'a> {
        ReconcileInput {
            desired,
            records,
            containers,
            now: 1_100,
            attestation_timeout_secs: 600,
            max_agents: 16,
            desired_hydrated: true,
        }
    }

    #[test]
    fn provisions_a_spec_it_has_never_seen() {
        let d = vec![desired("fizz", true, 1)];
        let actions = plan(input(&d, &[], &[]));
        assert!(matches!(actions.as_slice(), [Action::Provision { .. }]));
    }

    #[test]
    fn starts_an_attested_agent_with_no_container() {
        let d = vec![desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let actions = plan(input(&d, &r, &[]));
        assert!(matches!(actions.as_slice(), [Action::Start { .. }]));
    }

    #[test]
    fn does_nothing_when_running_and_current() {
        let d = vec![desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let c = vec![container("agent1", true)];
        assert!(plan(input(&d, &r, &c)).is_empty());
    }

    #[test]
    fn republishing_an_identical_spec_does_not_restart() {
        // NIP-33 replacement gives a new event id and timestamp for identical
        // content. Hashing content rather than the event keeps the agent up.
        let d = [desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let d2 = vec![desired("fizz", true, 1)];
        assert_eq!(hash, d2[0].spec_hash());

        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let c = vec![container("agent1", true)];
        assert!(plan(input(&d2, &r, &c)).is_empty());
    }

    #[test]
    fn restarts_on_spec_drift() {
        let old = desired("fizz", true, 1);
        let r = vec![record("fizz", "agent1", true, Some(&old.spec_hash()))];
        let c = vec![container("agent1", true)];
        let d = vec![desired("fizz", true, 4)];
        assert!(matches!(
            plan(input(&d, &r, &c)).as_slice(),
            [Action::Restart { .. }]
        ));
    }

    #[test]
    fn restarts_a_container_that_died_out_of_band_and_marks_it_crashed() {
        let d = vec![desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let c = vec![container("agent1", false)];
        assert!(matches!(
            plan(input(&d, &r, &c)).as_slice(),
            [Action::Restart { crashed: true, .. }]
        ));
    }

    #[test]
    fn a_drift_restart_is_not_a_crash() {
        // A healthy container replaced because its spec changed must not count
        // toward backoff — otherwise editing an agent repeatedly would throttle
        // a perfectly working one.
        let old = desired("fizz", true, 1);
        let r = vec![record("fizz", "agent1", true, Some(&old.spec_hash()))];
        let c = vec![container("agent1", true)];
        let d = vec![desired("fizz", true, 4)];
        assert!(matches!(
            plan(input(&d, &r, &c)).as_slice(),
            [Action::Restart { crashed: false, .. }]
        ));
    }

    #[test]
    fn a_crash_looping_container_is_throttled() {
        // The bug this guards: a container that starts then immediately exits
        // is recreated on every pass, because container *creation* keeps
        // succeeding. Only counting crashes makes the backoff bite.
        let d = vec![desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let mut rec = record("fizz", "agent1", true, Some(&hash));
        rec.restart_count = 4;
        rec.last_failure_at = Some(1_090);
        let r = vec![rec];
        let c = vec![container("agent1", false)];

        // 15 * 2^4 = 240s of backoff; only 10s have elapsed.
        assert!(plan(input(&d, &r, &c)).is_empty());
    }

    #[test]
    fn respects_backoff_after_repeated_failures() {
        let d = vec![desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let mut rec = record("fizz", "agent1", true, Some(&hash));
        rec.restart_count = 3;
        rec.last_failure_at = Some(1_090);
        let r = vec![rec];

        // 15 * 2^3 = 120s of backoff; only 10s have elapsed.
        assert!(plan(input(&d, &r, &[])).is_empty());

        let mut later = input(&d, &r, &[]);
        later.now = 1_090 + 121;
        assert!(matches!(plan(later).as_slice(), [Action::Start { .. }]));
    }

    #[test]
    fn backoff_is_capped() {
        assert_eq!(backoff_secs(0), 15);
        assert_eq!(backoff_secs(3), 120);
        assert_eq!(backoff_secs(20), 600, "must not grow without bound");
    }

    #[test]
    fn replaces_a_minted_identity_when_the_spec_names_a_different_agent() {
        // The relocation case: the spawner already minted an identity for this
        // slug, then the owner published a spec naming an existing agent to
        // move here. Treating that as ordinary drift would restart the minted
        // stand-in forever and the real agent would never arrive.
        let mut d = desired("fizz", true, 1);
        d.spec.agent_pubkey = Some("f".repeat(64));
        let r = vec![record("fizz", "agent-minted", true, Some(&d.spec_hash()))];
        let c = vec![container("agent-minted", true)];

        let actions = plan(input(&[d], &r, &c));
        assert!(actions.iter().any(|a| matches!(a, Action::Delete { .. })));
        assert!(actions
            .iter()
            .any(|a| matches!(a, Action::Provision { .. })));
        // The wrong identity must not simply be restarted.
        assert!(!actions.iter().any(|a| matches!(a, Action::Restart { .. })));
    }

    #[test]
    fn relocation_converges_instead_of_looping() {
        // Regression: the identity-mismatch rule tears down a minted stand-in
        // and re-provisions. If provisioning mints AGAIN rather than adopting
        // the named pubkey, the next pass sees a mismatch again and the spawner
        // churns containers forever — 29 mints in one run before this was
        // caught. Model the adopt step and assert the second pass is quiet.
        let mut d = desired("fizz", true, 1);
        let wanted = "f".repeat(64);
        d.spec.agent_pubkey = Some(wanted.clone());

        // Pass 1: a minted identity is on record, so it must be replaced.
        let minted = vec![record("fizz", "agent-minted", true, Some(&d.spec_hash()))];
        let first = plan(input(std::slice::from_ref(&d), &minted, &[]));
        assert!(first.iter().any(|a| matches!(a, Action::Delete { .. })));
        assert!(first.iter().any(|a| matches!(a, Action::Provision { .. })));

        // Pass 2: provisioning adopted the named pubkey, as `provision` does
        // for a spec carrying `agent_pubkey`. Nothing further should happen
        // beyond starting it — crucially, no second Delete/Provision cycle.
        let adopted = vec![record("fizz", &wanted, true, Some(&d.spec_hash()))];
        let second = plan(input(&[d], &adopted, &[]));
        assert!(
            !second.iter().any(|a| matches!(a, Action::Delete { .. })),
            "an adopted identity must not be torn down again"
        );
        assert!(
            !second.iter().any(|a| matches!(a, Action::Provision { .. })),
            "an adopted identity must not be re-provisioned"
        );
        assert!(matches!(second.as_slice(), [Action::Start { .. }]));
    }

    #[test]
    fn relocation_matching_is_case_insensitive() {
        // A pubkey that differs only in case is the same identity; treating it
        // as a mismatch would restart the mint/delete cycle.
        let mut d = desired("fizz", true, 1);
        d.spec.agent_pubkey = Some("A".repeat(64));
        let hash = d.spec_hash();
        let r = vec![record("fizz", &"a".repeat(64), true, Some(&hash))];
        let c = vec![container(&"a".repeat(64), true)];
        assert!(plan(input(&[d], &r, &c)).is_empty());
    }

    #[test]
    fn leaves_a_matching_relocated_identity_alone() {
        // Same pubkey on spec and record: this is the steady state after a
        // successful relocation, and must not churn.
        let mut d = desired("fizz", true, 1);
        d.spec.agent_pubkey = Some("a".repeat(64));
        let hash = d.spec_hash();
        let r = vec![record("fizz", &"a".repeat(64), true, Some(&hash))];
        let c = vec![container(&"a".repeat(64), true)];
        assert!(plan(input(&[d], &r, &c)).is_empty());
    }

    #[test]
    fn stops_a_disabled_agent_without_deleting_its_identity() {
        let d = vec![desired("fizz", false, 1)];
        let hash = d[0].spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let c = vec![container("agent1", true)];
        let actions = plan(input(&d, &r, &c));
        assert!(matches!(actions.as_slice(), [Action::Stop { .. }]));
        // Crucially not a Delete — re-enabling must resume the same agent.
        assert!(!actions.iter().any(|a| matches!(a, Action::Delete { .. })));
    }

    #[test]
    fn deletes_when_the_spec_disappears() {
        let hash = desired("fizz", true, 1).spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let c = vec![container("agent1", true)];
        let actions = plan(input(&[], &r, &c));
        assert_eq!(
            actions,
            [Action::Delete {
                owner_pubkey: "b".repeat(64),
                slug: "fizz".into(),
                container_id: Some("ctr-agent1".into()),
            }]
        );
    }

    #[test]
    fn never_deletes_before_desired_state_is_hydrated() {
        // The restart bug: at boot the relay has not replayed specs yet, so
        // `desired` is empty. Reading that as "every spec was deleted" destroys
        // each agent's container, volume, and secret key, and the agent comes
        // back as a brand-new pubkey that has lost its attestation and channel
        // membership. An absent spec before EOSE means "unknown", not "gone".
        let hash = desired("fizz", true, 1).spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];
        let c = vec![container("agent1", true)];

        let mut booting = input(&[], &r, &c);
        booting.desired_hydrated = false;
        assert!(
            plan(booting).is_empty(),
            "a pre-hydration pass must not delete anything"
        );

        // Once the relay confirms the spec really is gone, deletion proceeds.
        let mut hydrated = input(&[], &r, &c);
        hydrated.desired_hydrated = true;
        assert!(plan(hydrated)
            .iter()
            .any(|a| matches!(a, Action::Delete { .. })));
    }

    #[test]
    fn still_starts_known_agents_before_hydration() {
        // Gating deletion must not stall the additive path: an agent whose spec
        // already arrived should start without waiting for EOSE.
        let d = vec![desired("fizz", true, 1)];
        let hash = d[0].spec_hash();
        let r = vec![record("fizz", "agent1", true, Some(&hash))];

        let mut booting = input(&d, &r, &[]);
        booting.desired_hydrated = false;
        assert!(matches!(plan(booting).as_slice(), [Action::Start { .. }]));
    }

    #[test]
    fn never_reaps_orphan_containers_before_hydration() {
        // Same reasoning for containers: at boot the store may still be loading
        // and a running agent would look parentless.
        let c = vec![container("stranger", true)];
        let mut booting = input(&[], &[], &c);
        booting.desired_hydrated = false;
        assert!(plan(booting).is_empty());
    }

    #[test]
    fn removes_containers_with_no_record() {
        // No record means no key: the spawner cannot manage or speak for it.
        let c = vec![container("stranger", true)];
        assert!(matches!(
            plan(input(&[], &[], &c)).as_slice(),
            [Action::RemoveOrphan { .. }]
        ));
    }

    #[test]
    fn chases_a_timed_out_attestation_instead_of_starting() {
        let d = vec![desired("fizz", true, 1)];
        let r = vec![record("fizz", "agent1", false, None)];

        // Still within the window: wait quietly.
        assert!(plan(input(&d, &r, &[])).is_empty());

        let mut late = input(&d, &r, &[]);
        late.now = 1_000 + 601;
        assert!(matches!(
            plan(late).as_slice(),
            [Action::ReRequestAttestation { .. }]
        ));
    }

    #[test]
    fn honors_the_agent_cap() {
        let d: Vec<_> = (0..5)
            .map(|i| desired(&format!("fizz{i}"), true, 1))
            .collect();
        let r: Vec<_> = d
            .iter()
            .enumerate()
            .map(|(i, x)| record(&x.slug, &format!("agent{i}"), true, Some(&x.spec_hash())))
            .collect();

        let mut capped = input(&d, &r, &[]);
        capped.max_agents = 2;
        let starts = plan(capped)
            .into_iter()
            .filter(|a| matches!(a, Action::Start { .. }))
            .count();
        assert_eq!(starts, 2);
    }

    #[test]
    fn deletions_free_capacity_in_the_same_pass() {
        // One agent is going away and one is waiting to start, with room for
        // exactly one. The pass must not stall on the departing agent's slot.
        let d = vec![desired("new", true, 1)];
        let r = vec![
            record("old", "agent-old", true, Some("stale")),
            record("new", "agent-new", true, Some(&d[0].spec_hash())),
        ];
        let c = vec![container("agent-old", true)];

        let mut capped = input(&d, &r, &c);
        capped.max_agents = 1;
        let actions = plan(capped);

        assert!(actions.iter().any(|a| matches!(a, Action::Delete { .. })));
        assert!(actions.iter().any(|a| matches!(a, Action::Start { .. })));
    }
}
