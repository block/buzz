//! The deploy loop: executes [`crate::classify`]'s actions against a substrate
//! and re-enters (spec §Deploy State Machine).
//!
//! The substrate is a trait so the conformance tests drive this exact
//! reconciler — the shipped code path, not a test-only reimplementation — with
//! a fake cluster and a fake clock.
//!
//! Two shapes are worth naming up front, because they are what keep the loop
//! terminating:
//!
//! * **Success means the harness container started** (`:696-699`). There is no
//!   "deployed but not confirmed" success: `deploy` returns an `agent_id` or an
//!   in-band error carrying the latest condition. The wire has no third form.
//! * **A create-conflict loser never repairs.** It verifies the winner, drops
//!   its own Secret, and *observes* until the winner starts. Applying the
//!   divergence row to the pod that just beat it is exactly the ping-pong the
//!   spec forbids (`:845-850`), and the escape it names is the *next* deploy,
//!   not this one.

use crate::classify::{self, Action, Fence, Startup, VerifiedPod};
use crate::config::ProviderConfig;
use crate::gc;
use crate::naming::AgentIdentity;
use crate::observe::{self, StartupObservation};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Pod, Secret};
use std::collections::BTreeMap;
use std::time::Duration;

/// Outcome of a create against the deterministic pod name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateOutcome {
    Created,
    /// 409 with `Status.reason: AlreadyExists` — a concurrent attempt won.
    /// Discriminated on the typed reason, never on the HTTP code alone: 409 is
    /// also `Conflict`, which means a failed precondition (`:780-794`).
    AlreadyExists,
}

/// Outcome of a fenced delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Accepted,
    /// Already gone. Delete-not-found is success (`:842`).
    NotFound,
    /// 409 with `Status.reason: Conflict` — the object changed since the
    /// observation that authorized this delete. Neither an error nor
    /// permission to retry: re-enter and classify what exists now
    /// (`:775-777`).
    PreconditionFailed,
}

/// The cluster operations the reconciler needs. Everything here is I/O;
/// everything that decides is pure and lives in `classify`/`gc`.
#[allow(async_fn_in_trait)]
pub trait Substrate {
    /// Create the namespace if absent. On RBAC denial the error MUST name the
    /// literal `kubectl create namespace <name>` command and MUST NOT fall
    /// back to `default` (`:1002-1005`).
    async fn ensure_namespace(&self, namespace: &str) -> Result<(), String>;

    /// Most-recent read of the pods matching this identity's selector, plus
    /// the apiserver's clock from the same call's HTTP `Date` header. `None`
    /// clock means the header was absent or unparseable, which makes the
    /// orphan-Secret sweep skip (`:1321-1335`).
    async fn list_pods(&self, selector: &str) -> Result<(Vec<Pod>, Option<DateTime<Utc>>), String>;

    async fn list_secrets(&self, selector: &str) -> Result<Vec<Secret>, String>;

    /// Most-recent existence check (`resourceVersion` explicitly unset, not
    /// `"0"`): the classifier treats a confirmed absence as proof, so a
    /// possibly-stale cache read would be proof of nothing (`:761-769`).
    async fn secret_exists(&self, name: &str) -> Result<bool, String>;

    async fn create_secret(&self, secret: &Secret) -> Result<(), String>;

    async fn create_pod(&self, pod: &Pod) -> Result<CreateOutcome, String>;

    /// Compare-and-delete against the fence from the authorizing observation.
    /// Uses the object's own grace period — never `grace_period_seconds: 0`,
    /// which is a force-kill that discards the declared 60s shutdown budget
    /// (`:1185-1189`).
    async fn delete_pod(&self, name: &str, fence: &Fence) -> Result<DeleteOutcome, String>;

    /// Best-effort: the Secret may already be gone, which is success.
    async fn delete_secret(&self, name: &str) -> Result<(), String>;

    /// Read one pod by name, most-recent. `None` is a confirmed absence.
    async fn get_pod(&self, name: &str) -> Result<Option<Pod>, String>;

    async fn sleep(&self, duration: Duration);

    /// Monotonic elapsed time since the operation began. Fake-clock driven in
    /// tests; the deadline must not depend on wall-clock adjustments.
    fn elapsed(&self) -> Duration;
}

/// Interval between reconciler polls. Short enough that a fast start is
/// reported promptly, long enough not to hammer the apiserver for 600s.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The deploy operation deadline (spec §Deploy: `timeout: 600s`).
const DEADLINE: Duration = Duration::from_secs(gc::OPERATION_DEADLINE_SECS as u64);

/// Settle a pod's startup state, performing the one most-recent Secret read
/// that `CreateContainerConfigError` requires.
async fn settle(substrate: &impl Substrate, pod: &Pod) -> Result<Startup, String> {
    Ok(match observe::decode_startup(pod) {
        StartupObservation::Resolved(startup) => startup,
        StartupObservation::ConfigErrorPendingSecretCheck { secret_name } => {
            // A confirmed absence is proof; anything else stays recoverable,
            // because the kubelet's reason string alone is not evidence.
            if substrate.secret_exists(&secret_name).await? {
                Startup::NeverStartedRecoverable
            } else {
                Startup::NeverStartedProvablyBroken
            }
        }
    })
}

/// Observe the single pod owned by this identity, verified.
///
/// `Ok(None)` conflates "absent" with "present but not ours" on purpose *here*
/// — both mean the classifier has nothing it may act on. The create path
/// separates them, because only there is the difference actionable.
async fn observe_pod(
    substrate: &impl Substrate,
    identity: &AgentIdentity,
) -> Result<Option<VerifiedPod>, String> {
    let Some(pod) = substrate.get_pod(&identity.pod_name()).await? else {
        return Ok(None);
    };
    let startup = settle(substrate, &pod).await?;
    Ok(observe::verify(&pod, identity, startup))
}

/// The latest condition to report, read fresh at the moment of reporting.
///
/// Reading it here rather than threading it through every loop iteration is
/// what "the *latest* redacted condition" (`:689`) asks for, and it costs a
/// read only on the paths that are already failing.
async fn latest_condition(substrate: &impl Substrate, identity: &AgentIdentity) -> String {
    match substrate.get_pod(&identity.pod_name()).await {
        Ok(Some(pod)) => observe::condition(&pod)
            .unwrap_or_else(|| "no condition reported by the cluster".to_string()),
        Ok(None) => "the pod no longer exists".to_string(),
        Err(e) => format!("the pod's condition could not be read: {e}"),
    }
}

/// Preflight GC (§K8s GC). Failures are logged and swallowed: GC is hygiene,
/// and a deploy must not fail because a stale object could not be listed or
/// removed. A denial that actually blocks this deploy resurfaces at create,
/// where the message names the operation the user was denied.
async fn preflight_gc(substrate: &impl Substrate, identity: &AgentIdentity) {
    if let Err(e) = try_preflight_gc(substrate, identity).await {
        eprintln!("gc: preflight pass skipped: {e}");
    }
}

async fn try_preflight_gc(
    substrate: &impl Substrate,
    identity: &AgentIdentity,
) -> Result<(), String> {
    let selector = identity.selector();
    let (pods, server_now) = substrate.list_pods(&selector).await?;
    let secrets = substrate.list_secrets(&selector).await?;

    let mut terminated: Vec<String> = Vec::new();
    for pod in &pods {
        if matches!(settle(substrate, pod).await?, Startup::Terminated) {
            if let Some(name) = pod.metadata.name.clone() {
                terminated.push(name);
            }
        }
    }

    let plan = gc::plan(
        identity,
        &pods,
        &secrets,
        |pod| {
            pod.metadata
                .name
                .as_deref()
                .map(|n| terminated.iter().any(|t| t == n))
                .unwrap_or(false)
        },
        server_now,
    );

    for name in &plan.pods {
        // Re-read to fence the delete against the object we just observed; a
        // pod that changed since the list is simply skipped this pass.
        let Some(pod) = substrate.get_pod(name).await? else {
            continue;
        };
        let (Some(uid), Some(rv)) = (
            pod.metadata.uid.clone(),
            pod.metadata.resource_version.clone(),
        ) else {
            continue;
        };
        if !matches!(settle(substrate, &pod).await?, Startup::Terminated) {
            continue;
        }
        let fence = Fence {
            uid,
            resource_version: rv,
        };
        if let Err(e) = substrate.delete_pod(name, &fence).await {
            eprintln!("gc: could not delete terminated pod {name}: {e}");
        }
    }
    for name in &plan.secrets {
        if let Err(e) = substrate.delete_secret(name).await {
            eprintln!("gc: could not delete secret {name}: {e}");
        }
    }
    Ok(())
}

/// Wait for a pod to actually disappear.
///
/// Mandatory before recreating: `DELETE` returns success while the object
/// still exists, and the deterministic name stays taken for the whole grace
/// period (`:1177-1195`).
async fn await_disappearance(substrate: &impl Substrate, name: &str) -> Result<(), String> {
    while substrate.elapsed() < DEADLINE {
        if substrate.get_pod(name).await?.is_none() {
            return Ok(());
        }
        substrate.sleep(POLL_INTERVAL).await;
    }
    Err(format!(
        "timed out after {}s waiting for {name} to finish terminating",
        DEADLINE.as_secs()
    ))
}

/// Is `secret` referenced by any pod that currently exists under this
/// identity's selector?
///
/// Protection deliberately spans *all* our pods, not just the winner: an
/// `envFrom` reference from a pod still pulling its image is exactly as
/// load-bearing as one from a running pod (`:1261-1264`).
async fn secret_is_referenced(
    substrate: &impl Substrate,
    identity: &AgentIdentity,
    secret: &str,
) -> Result<bool, String> {
    let (pods, _) = substrate.list_pods(&identity.selector()).await?;
    Ok(pods
        .iter()
        .filter_map(observe::referenced_secret)
        .any(|name| name == secret))
}

/// Drop this attempt's own Secret once nothing references it.
///
/// Only ever called with a name this process generated, and gated on the
/// reference check: "never the winner's, never any Secret referenced by an
/// existing pod" (`:1259-1264`). Failure is logged, not fatal — a leaked
/// Secret is collected by the age-gated sweep.
async fn drop_own_secret(substrate: &impl Substrate, identity: &AgentIdentity, secret: &str) {
    match secret_is_referenced(substrate, identity, secret).await {
        Ok(false) => {
            if let Err(e) = substrate.delete_secret(secret).await {
                eprintln!("could not clean up own unreferenced secret {secret}: {e}");
            }
        }
        Ok(true) => {}
        Err(e) => eprintln!("could not check whether {secret} is still referenced: {e}"),
    }
}

/// Lost the create race: adopt the elected winner.
///
/// Observe-only by construction — this function has no delete edge for the
/// pod. A winner that is terminated or provably broken is reported, not
/// repaired; the spec's escape is "a *subsequent* deploy that walks in and
/// observes that never-started divergent winner replaces it normally"
/// (`:849-850`).
async fn adopt_winner(
    substrate: &impl Substrate,
    identity: &AgentIdentity,
    own_secret: &str,
) -> Result<String, String> {
    let name = identity.pod_name();

    // Verify before adopting. A pod under our deterministic name that fails
    // the marker/annotation check is not ours to adopt, wait for, or touch —
    // and it will never become ours, so this is terminal rather than a retry.
    match substrate.get_pod(&name).await? {
        None => {}
        Some(pod) => {
            let startup = settle(substrate, &pod).await?;
            if observe::verify(&pod, identity, startup).is_none() {
                drop_own_secret(substrate, identity, own_secret).await;
                return Err(format!(
                    "a pod named {name} already exists in this namespace but is not \
                     managed by this provider for this agent (it lacks the management \
                     marker or carries a different agent identity). Remove it, or \
                     deploy this agent to a different namespace."
                ));
            }
        }
    }

    drop_own_secret(substrate, identity, own_secret).await;

    // Then wait for the winner exactly as we would wait for our own pod:
    // success still means the harness container started.
    loop {
        if substrate.elapsed() >= DEADLINE {
            return Err(format!(
                "startup not confirmed within {}s for {name} (another deploy of this \
                 agent created it): {}",
                DEADLINE.as_secs(),
                latest_condition(substrate, identity).await
            ));
        }
        match observe_pod(substrate, identity).await? {
            Some(pod) if matches!(pod.startup, Startup::Started) => return Ok(pod.name),
            Some(pod) if pod.deletion_marked => {
                return Err(format!(
                    "{name} was created by another deploy of this agent and is already \
                     being deleted; try again"
                ))
            }
            Some(pod)
                if matches!(
                    pod.startup,
                    Startup::Terminated | Startup::NeverStartedProvablyBroken
                ) =>
            {
                return Err(format!(
                    "{name} was created by another deploy of this agent and did not \
                     start: {}",
                    latest_condition(substrate, identity).await
                ))
            }
            // Gone again, or still coming up: keep observing under this
            // operation's deadline.
            _ => substrate.sleep(POLL_INTERVAL).await,
        }
    }
}

/// Run the deploy state machine to a terminal outcome: the started pod's name,
/// or an in-band error carrying the latest condition.
pub async fn deploy(
    substrate: &impl Substrate,
    identity: &AgentIdentity,
    cfg: &ProviderConfig,
    env: BTreeMap<String, String>,
) -> Result<String, String> {
    substrate.ensure_namespace(&cfg.namespace).await?;
    preflight_gc(substrate, identity).await;

    let desired = crate::pod::intent_template(cfg, env.keys().cloned()).fingerprint();

    // Has THIS call created a pod? Set once its create lands. The replacement
    // rows below are for residue from a previous life; once this call has made
    // its own attempt, a replace-classification means that attempt failed —
    // and startup verification is part of create, so the failure is reported
    // in-band rather than retried. Without this bound a deterministic startup
    // failure (the harness starts, rejects its configuration, exits) is
    // delete-recreated every poll for the whole deadline, minting an immutable
    // Secret per cycle — measured live at 107 Secrets in one 600s call, every
    // one younger than the orphan sweep's age gate.
    let mut created_this_call = false;

    loop {
        if substrate.elapsed() >= DEADLINE {
            return Err(format!(
                "startup not confirmed within {}s for {}: {}",
                DEADLINE.as_secs(),
                identity.pod_name(),
                latest_condition(substrate, identity).await
            ));
        }

        let observed = observe_pod(substrate, identity).await?;
        match classify::classify(observed.as_ref(), &desired) {
            // The only success edge: the harness container is running.
            Action::NoOp { agent_id } => return Ok(agent_id),

            // Self-healing states. Never delete, on this call or any later one
            // — what replaces a never-started pod is a config change, never a
            // deadline (`:717-729`).
            Action::Observe { .. } => substrate.sleep(POLL_INTERVAL).await,

            // A pull that will not self-heal: report now rather than spend the
            // remaining deadline on it. Still no delete authority.
            Action::Report { name, failure } => {
                return Err(format!(
                    "{name} did not start: {}",
                    observe::pull_failure_message(failure, cfg.image.as_str())
                ))
            }

            Action::AwaitDisappearance { name } => await_disappearance(substrate, &name).await?,

            Action::Delete { name, fence } => {
                // This call already made its own attempt, and that attempt is
                // what the classification wants replaced: it terminated (the
                // deterministic startup failure — the harness starts, rejects
                // its configuration, exits) or was proven broken. Replacing it
                // here retries the identical configuration against the same
                // cluster: a hot delete/mint/create cycle every poll for the
                // whole deadline, an immutable Secret per cycle — measured
                // live at 107 Secrets in one 600s call, all younger than the
                // orphan sweep's age gate. Report in-band instead. The residue
                // is deliberate: the next Start's preflight GC collects the
                // terminated pod and its referenced Secret together, so retry
                // is gated on fresh owner intent and litter stays bounded at
                // one pod + one Secret per press.
                if created_this_call {
                    return Err(format!(
                        "{name} was created by this deploy and did not stay \
                         running: {}. Not retrying in this call — an immediate \
                         exit recurs until its cause is fixed. Check the \
                         agent's configuration and press Start to try again.",
                        latest_condition(substrate, identity).await
                    ));
                }
                match substrate.delete_pod(&name, &fence).await? {
                    // Accepted or already gone: both need the disappearance
                    // poll before the name is free again.
                    DeleteOutcome::Accepted | DeleteOutcome::NotFound => {
                        await_disappearance(substrate, &name).await?
                    }
                    // The object changed since the observation that authorized
                    // this delete. Discard the action and re-classify — never
                    // retry with a fresher fence, which would delete something
                    // we never examined. Sleep before re-entering: the losing
                    // race is against another writer, and re-reading at full
                    // speed is a busy-retry with no better odds than a paced
                    // one.
                    DeleteOutcome::PreconditionFailed => substrate.sleep(POLL_INTERVAL).await,
                }
            }

            Action::Create => {
                let generation = crate::naming::new_generation();
                let secret_name = identity.secret_name(&generation);

                // The generation is minted *per attempt*, and it is two things
                // at once: the Secret's name suffix and the lifecycle
                // correlator the harness reports. `build_env` stamped the
                // caller's generation, so on any attempt after the first the
                // two would name different generations — pod logs correlating
                // to a Secret that is not the one mounted. Restamp so there is
                // exactly one generation per attempt (§K8s Secrets).
                let mut env = env.clone();
                env.insert(crate::env::START_NONCE_KEY.to_string(), generation.clone());

                // Secret first: the pod's spec references this exact name, so
                // payload and Secret are atomic at the pod-spec boundary.
                let secret = crate::pod::build_secret(identity, &cfg.namespace, &generation, env);
                substrate.create_secret(&secret).await?;

                let pod = crate::pod::build_pod(identity, cfg, &generation, &desired);
                match substrate.create_pod(&pod).await? {
                    // Re-enter rather than wait inline: the next iteration
                    // observes what we just created and runs the same rows
                    // every other state runs through. One loop, one table.
                    //
                    // Sleep first. A just-created pod cannot already be
                    // started, so the immediate observation has no outcome but
                    // "still coming up" — and if it ever came back
                    // unverifiable, re-entering without advancing the clock
                    // would hot-spin creates against the apiserver for the
                    // whole deadline.
                    CreateOutcome::Created => {
                        created_this_call = true;
                        substrate.sleep(POLL_INTERVAL).await
                    }
                    CreateOutcome::AlreadyExists => {
                        return adopt_winner(substrate, identity, &secret_name).await
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Resources;
    use crate::naming::{ANNOTATION_CREATE_INTENT, ANNOTATION_PUBKEY_FULL, LABEL_MANAGED_BY};
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateRunning, ContainerStateTerminated, ContainerStateWaiting,
        ContainerStatus, PodStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
    use std::cell::RefCell;
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    /// Mutates the pod map after a poll, so a test can script a pod that
    /// starts (or vanishes) partway through an observation loop.
    type PollHook = Box<dyn Fn(&mut BTreeMap<String, Pod>)>;

    /// A scripted cluster. Single-threaded on purpose: the reconciler is one
    /// process per operation, and `RefCell` keeps the assertions readable.
    ///
    /// This drives the *shipped* `deploy` — the point of the `Substrate` seam.
    /// Every fake here answers with real `k8s_openapi` objects, so the decode
    /// and verification layers under test are the ones that run in a cluster.
    #[derive(Default)]
    struct Fake {
        pods: RefCell<BTreeMap<String, Pod>>,
        secrets: RefCell<Vec<Secret>>,
        /// Server clock for the GC age gate; `None` models a missing `Date`.
        server_now: Option<DateTime<Utc>>,
        /// Elapsed time, advanced only by `sleep` — a fake clock, so a 600s
        /// deadline test runs instantly and cannot flake on a slow machine.
        elapsed: RefCell<Duration>,
        /// Queued create outcomes; the default is `Created`.
        create_outcomes: RefCell<Vec<CreateOutcome>>,
        /// Installed when a create loses the race. A winner must be *absent*
        /// at the observation that decides to create and *present* by the time
        /// the create lands — pre-seeding it instead makes the loop no-op
        /// before it ever reaches the create edge.
        winner: RefCell<Option<Pod>>,
        /// Every Secret ever created, retained across deletion.
        created_secrets: RefCell<Vec<Secret>>,
        /// Queued delete outcomes; the default is `Accepted`.
        delete_outcomes: RefCell<Vec<DeleteOutcome>>,
        /// Every mutating call, in order — the anti-mutation assertions read
        /// this rather than guessing from final state.
        calls: RefCell<Vec<String>>,
        /// Applied to the pod map after each poll, so a test can script a pod
        /// that starts (or vanishes) partway through an observation loop.
        on_poll: RefCell<Vec<PollHook>>,
        /// `ensure_namespace` fails with this, if set.
        namespace_error: Option<String>,
    }

    impl Fake {
        fn with_pod(self, pod: Pod) -> Self {
            self.pods
                .borrow_mut()
                .insert(pod.metadata.name.clone().unwrap(), pod);
            self
        }
        fn log(&self, entry: impl Into<String>) {
            self.calls.borrow_mut().push(entry.into());
        }
        fn mutations(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Substrate for Fake {
        async fn ensure_namespace(&self, namespace: &str) -> Result<(), String> {
            match &self.namespace_error {
                Some(e) => Err(e.clone()),
                None => {
                    self.log(format!("ensure_namespace {namespace}"));
                    Ok(())
                }
            }
        }

        async fn list_pods(
            &self,
            _selector: &str,
        ) -> Result<(Vec<Pod>, Option<DateTime<Utc>>), String> {
            Ok((
                self.pods.borrow().values().cloned().collect(),
                self.server_now,
            ))
        }

        async fn list_secrets(&self, _selector: &str) -> Result<Vec<Secret>, String> {
            Ok(self.secrets.borrow().clone())
        }

        async fn secret_exists(&self, name: &str) -> Result<bool, String> {
            Ok(self
                .secrets
                .borrow()
                .iter()
                .any(|s| s.metadata.name.as_deref() == Some(name)))
        }

        async fn create_secret(&self, secret: &Secret) -> Result<(), String> {
            let name = secret.metadata.name.clone().unwrap();
            self.log(format!("create_secret {name}"));
            self.secrets.borrow_mut().push(secret.clone());
            // Kept even after the Secret is deleted: assertions about what an
            // attempt *wrote* must not be silently vacuous once cleanup runs.
            self.created_secrets.borrow_mut().push(secret.clone());
            Ok(())
        }

        async fn create_pod(&self, pod: &Pod) -> Result<CreateOutcome, String> {
            let name = pod.metadata.name.clone().unwrap();
            self.log(format!("create_pod {name}"));
            let outcome = if self.create_outcomes.borrow().is_empty() {
                CreateOutcome::Created
            } else {
                self.create_outcomes.borrow_mut().remove(0)
            };
            if outcome == CreateOutcome::Created {
                // The apiserver stamps these on admission; a builder never
                // carries them. Without them the pod fails `verify`'s fence
                // extraction and the loop can never see what it just created.
                let mut pod = pod.clone();
                pod.metadata.uid = Some(format!("uid-created-{}", self.calls.borrow().len()));
                pod.metadata.resource_version = Some("1".into());
                self.pods.borrow_mut().insert(name, pod);
            } else if let Some(winner) = self.winner.borrow_mut().take() {
                // The concurrent attempt's pod becomes visible exactly when our
                // create is rejected — the ordering a real race produces.
                self.pods.borrow_mut().insert(name, winner);
            }
            Ok(outcome)
        }

        async fn delete_pod(&self, name: &str, fence: &Fence) -> Result<DeleteOutcome, String> {
            self.log(format!(
                "delete_pod {name} uid={} rv={}",
                fence.uid, fence.resource_version
            ));
            let outcome = if self.delete_outcomes.borrow().is_empty() {
                DeleteOutcome::Accepted
            } else {
                self.delete_outcomes.borrow_mut().remove(0)
            };
            if outcome == DeleteOutcome::Accepted {
                self.pods.borrow_mut().remove(name);
            }
            Ok(outcome)
        }

        async fn delete_secret(&self, name: &str) -> Result<(), String> {
            self.log(format!("delete_secret {name}"));
            self.secrets
                .borrow_mut()
                .retain(|s| s.metadata.name.as_deref() != Some(name));
            Ok(())
        }

        async fn get_pod(&self, name: &str) -> Result<Option<Pod>, String> {
            Ok(self.pods.borrow().get(name).cloned())
        }

        async fn sleep(&self, duration: Duration) {
            *self.elapsed.borrow_mut() += duration;
            let hooks = self.on_poll.borrow();
            let mut pods = self.pods.borrow_mut();
            for hook in hooks.iter() {
                hook(&mut pods);
            }
        }

        fn elapsed(&self) -> Duration {
            *self.elapsed.borrow()
        }
    }

    fn identity() -> AgentIdentity {
        use nostr::nips::nip19::ToBech32;
        let keys = nostr::Keys::generate();
        AgentIdentity::from_nsec(&keys.secret_key().to_bech32().unwrap()).unwrap()
    }

    fn config() -> ProviderConfig {
        ProviderConfig {
            context: None,
            namespace: "buzz-agents-test".into(),
            image: crate::image::parse(&format!(
                "ghcr.io/block/buzz-sprig@sha256:{}",
                "a".repeat(64)
            ))
            .unwrap(),
            resources: Resources::default(),
            inactivity_seconds: Some(7200),
            service_account: None,
        }
    }

    fn env() -> BTreeMap<String, String> {
        [("BUZZ_RELAY_URL".to_string(), "wss://r".to_string())]
            .into_iter()
            .collect()
    }

    /// A pod exactly as this provider would have created it — same builder the
    /// reconciler uses, so verification is exercised rather than bypassed.
    fn our_pod(id: &AgentIdentity, cfg: &ProviderConfig, state: Option<ContainerState>) -> Pod {
        let fp = crate::pod::intent_template(cfg, env().keys().cloned()).fingerprint();
        let mut pod = crate::pod::build_pod(id, cfg, "gen-existing", &fp);
        pod.metadata.uid = Some("uid-1".into());
        pod.metadata.resource_version = Some("100".into());
        pod.status = Some(PodStatus {
            phase: Some("Running".into()),
            container_statuses: state.map(|s| {
                vec![ContainerStatus {
                    name: crate::observe::CONTAINER_NAME.into(),
                    state: Some(s),
                    ..Default::default()
                }]
            }),
            ..Default::default()
        });
        pod
    }

    fn running() -> ContainerState {
        ContainerState {
            running: Some(ContainerStateRunning::default()),
            ..Default::default()
        }
    }

    fn terminated() -> ContainerState {
        ContainerState {
            terminated: Some(ContainerStateTerminated::default()),
            ..Default::default()
        }
    }

    fn waiting(reason: &str) -> ContainerState {
        ContainerState {
            waiting: Some(ContainerStateWaiting {
                reason: Some(reason.into()),
                message: None,
            }),
            ..Default::default()
        }
    }

    fn run(fake: &Fake, id: &AgentIdentity, cfg: &ProviderConfig) -> Result<String, String> {
        block_on(deploy(fake, id, cfg, env()))
    }

    /// Minimal executor: the fake never yields to a reactor (no timers, no
    /// I/O — `sleep` just advances a counter), so polling to completion is
    /// sufficient and avoids pulling a runtime into the unit job.
    fn block_on<T>(fut: impl Future<Output = T>) -> T {
        fn noop_waker() -> Waker {
            fn nop(_: *const ()) {}
            fn clone(_: *const ()) -> RawWaker {
                RawWaker::new(std::ptr::null(), &VTABLE)
            }
            static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, nop, nop, nop);
            unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
        }

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("fake substrate future parked — it has no reactor"),
        }
    }

    // ---- the state machine's rows, end to end -------------------------------

    /// First deploy: Secret before pod, and the returned id is the pod name.
    #[test]
    fn creates_secret_then_pod_and_returns_the_pod_name() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default();
        fake.on_poll.borrow_mut().push({
            let name = id.pod_name();
            Box::new(move |pods: &mut BTreeMap<String, Pod>| {
                if let Some(pod) = pods.get_mut(&name) {
                    pod.status = Some(PodStatus {
                        phase: Some("Running".into()),
                        container_statuses: Some(vec![ContainerStatus {
                            name: crate::observe::CONTAINER_NAME.into(),
                            state: Some(running()),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    });
                }
            })
        });

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());

        let calls = fake.mutations();
        let secret_at = calls
            .iter()
            .position(|c| c.starts_with("create_secret"))
            .unwrap();
        let pod_at = calls
            .iter()
            .position(|c| c.starts_with("create_pod"))
            .unwrap();
        assert!(
            secret_at < pod_at,
            "pod created before its Secret: {calls:?}"
        );
    }

    /// The strict no-op row: a started pod returns its id having mutated
    /// nothing at all. Asserted on the *call log*, not on final state — a
    /// delete-then-recreate would leave identical final state.
    #[test]
    fn started_pod_is_a_zero_mutation_no_op() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default().with_pod(our_pod(&id, &cfg, Some(running())));

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        assert_eq!(
            fake.mutations(),
            [format!("ensure_namespace {}", cfg.namespace)],
            "the no-op row mutated something"
        );
    }

    /// ...including when the desired intent has diverged. Edits reach a
    /// started pod only via the next generation (`:861-865`).
    #[test]
    fn started_pod_no_ops_under_divergent_intent() {
        let id = identity();
        let cfg = config();
        let mut pod = our_pod(&id, &cfg, Some(running()));
        pod.metadata.annotations.as_mut().unwrap().insert(
            ANNOTATION_CREATE_INTENT.to_string(),
            "stale-fingerprint".into(),
        );
        let fake = Fake::default().with_pod(pod);

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        assert!(
            !fake.mutations().iter().any(|c| c.starts_with("delete_pod")),
            "divergence deleted a started pod"
        );
    }

    /// Terminated → fenced delete → disappearance → recreate. The normal
    /// restart path, and the fence must carry the observed uid/rv.
    #[test]
    fn terminated_pod_is_replaced_with_a_fenced_delete() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default().with_pod(our_pod(&id, &cfg, Some(terminated())));
        fake.on_poll.borrow_mut().push({
            let name = id.pod_name();
            Box::new(move |pods: &mut BTreeMap<String, Pod>| {
                if let Some(pod) = pods.get_mut(&name) {
                    if pod
                        .status
                        .as_ref()
                        .and_then(|s| s.container_statuses.as_ref())
                        .is_none()
                    {
                        pod.status = Some(PodStatus {
                            phase: Some("Running".into()),
                            container_statuses: Some(vec![ContainerStatus {
                                name: crate::observe::CONTAINER_NAME.into(),
                                state: Some(running()),
                                ..Default::default()
                            }]),
                            ..Default::default()
                        });
                    }
                }
            })
        });

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        let calls = fake.mutations();
        assert!(
            calls
                .iter()
                .any(|c| c == &format!("delete_pod {} uid=uid-1 rv=100", id.pod_name())),
            "delete was not fenced to the observed uid+resourceVersion: {calls:?}"
        );
        assert!(calls.iter().any(|c| c.starts_with("create_pod")));
    }

    /// A failed precondition is neither an error nor permission to retry the
    /// delete: re-enter and classify what exists now (`:775-777`). Here the
    /// object has become a *started* pod, so the correct outcome is the no-op
    /// row — never a second delete with a fresher fence.
    #[test]
    fn precondition_failure_reclassifies_instead_of_retrying() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default().with_pod(our_pod(&id, &cfg, Some(terminated())));
        fake.delete_outcomes
            .borrow_mut()
            .push(DeleteOutcome::PreconditionFailed);
        // The writer we lost the race to: between our observation and our
        // delete, the pod under this name became a *started* one with a new
        // resourceVersion. Installed on the poll that follows the failed
        // precondition, so the sequence is observe → delete → lose → re-observe.
        fake.on_poll.borrow_mut().push({
            let name = id.pod_name();
            let started = {
                let mut p = our_pod(&id, &cfg, Some(running()));
                p.metadata.resource_version = Some("200".into());
                p
            };
            Box::new(move |pods: &mut BTreeMap<String, Pod>| {
                pods.insert(name.clone(), started.clone());
            })
        });

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        let deletes: Vec<_> = fake
            .mutations()
            .into_iter()
            .filter(|c| c.starts_with("delete_pod"))
            .collect();
        // Two call sites legitimately target a terminated pod: preflight GC's
        // sweep (which is the one that loses the precondition here) and the
        // state machine's terminated row. Both are fenced on their own
        // observation. What must never happen is a *third* — a retry of the
        // failed delete — so the invariant is the fence, not the count: every
        // delete carries rv=100, the version we observed. A retry would carry
        // the racing writer's rv=200.
        assert_eq!(deletes.len(), 2, "unexpected delete traffic: {deletes:?}");
        assert!(
            deletes.iter().all(|d| d.contains("rv=100")),
            "retried with a fresher fence: {deletes:?}"
        );
    }

    /// The anti-livelock rule, at the loop level: a recoverable pod whose
    /// intent matches is observed until the deadline and **never** deleted —
    /// the case that would otherwise reset the pod age Cluster Autoscaler
    /// keys on (`:689`).
    #[test]
    fn recoverable_pod_with_matching_intent_is_never_deleted() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default().with_pod(our_pod(&id, &cfg, Some(waiting("Unschedulable"))));

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(err.contains("startup not confirmed"), "got: {err}");
        assert!(
            !fake.mutations().iter().any(|c| c.starts_with("delete_pod")),
            "deleted a recoverable pod: {:?}",
            fake.mutations()
        );
        assert!(fake.elapsed() >= DEADLINE, "gave up before the deadline");
    }

    /// The same pod, provisioned late: the observation loop must *succeed*
    /// when the autoscaler eventually lands the node, not merely avoid
    /// deleting.
    #[test]
    fn recoverable_pod_that_starts_late_succeeds() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default().with_pod(our_pod(&id, &cfg, Some(waiting("Unschedulable"))));
        let polls = RefCell::new(0);
        fake.on_poll.borrow_mut().push({
            let name = id.pod_name();
            Box::new(move |pods: &mut BTreeMap<String, Pod>| {
                *polls.borrow_mut() += 1;
                if *polls.borrow() >= 5 {
                    if let Some(pod) = pods.get_mut(&name) {
                        pod.status.as_mut().unwrap().container_statuses =
                            Some(vec![ContainerStatus {
                                name: crate::observe::CONTAINER_NAME.into(),
                                state: Some(running()),
                                ..Default::default()
                            }]);
                    }
                }
            })
        });

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        assert!(fake.elapsed() < DEADLINE);
    }

    /// A permanent pull failure reports immediately rather than burning the
    /// deadline — and still never deletes.
    #[test]
    fn permanent_pull_failure_reports_without_deleting() {
        let id = identity();
        let cfg = config();
        let mut pod = our_pod(&id, &cfg, None);
        pod.status = Some(PodStatus {
            phase: Some("Pending".into()),
            container_statuses: Some(vec![ContainerStatus {
                name: crate::observe::CONTAINER_NAME.into(),
                state: Some(ContainerState {
                    waiting: Some(ContainerStateWaiting {
                        reason: Some("ImagePullBackOff".into()),
                        message: Some("manifest unknown".into()),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        });
        let fake = Fake::default().with_pod(pod);

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(err.contains("no image at"), "got: {err}");
        assert!(
            fake.elapsed() < DEADLINE,
            "burned the deadline on a permanent failure"
        );
        assert!(!fake.mutations().iter().any(|c| c.starts_with("delete_pod")));
    }

    /// The kubelet's pull *message* can echo a registry request; only the
    /// classified outcome and the image reference reach the user.
    #[test]
    fn pull_failure_message_is_not_echoed_verbatim() {
        let id = identity();
        let cfg = config();
        let mut pod = our_pod(&id, &cfg, None);
        pod.status = Some(PodStatus {
            phase: Some("Pending".into()),
            container_statuses: Some(vec![ContainerStatus {
                name: crate::observe::CONTAINER_NAME.into(),
                state: Some(ContainerState {
                    waiting: Some(ContainerStateWaiting {
                        reason: Some("ErrImagePull".into()),
                        message: Some(
                            "unauthorized: authentication required, token=SUPERSECRET".into(),
                        ),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ..Default::default()
        });
        let fake = Fake::default().with_pod(pod);

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(
            !err.contains("SUPERSECRET"),
            "kubelet message echoed: {err}"
        );
    }

    /// A pod being gracefully deleted stays in phase `Running` for its whole
    /// grace period. The reconciler must wait it out and recreate, not return
    /// an id that evaporates.
    #[test]
    fn deletion_marked_pod_is_awaited_then_recreated() {
        let id = identity();
        let cfg = config();
        let mut dying = our_pod(&id, &cfg, Some(running()));
        dying.metadata.deletion_timestamp = Some(Time(Utc::now()));
        let fake = Fake::default().with_pod(dying);
        fake.on_poll.borrow_mut().push({
            let name = id.pod_name();
            Box::new(move |pods: &mut BTreeMap<String, Pod>| {
                match pods
                    .get(&name)
                    .map(|p| p.metadata.deletion_timestamp.is_some())
                {
                    // The grace period elapses: the object disappears.
                    Some(true) => {
                        pods.remove(&name);
                    }
                    // The replacement we create then starts.
                    Some(false) => {
                        let pod = pods.get_mut(&name).unwrap();
                        pod.status = Some(PodStatus {
                            phase: Some("Running".into()),
                            container_statuses: Some(vec![ContainerStatus {
                                name: crate::observe::CONTAINER_NAME.into(),
                                state: Some(running()),
                                ..Default::default()
                            }]),
                            ..Default::default()
                        });
                    }
                    None => {}
                }
            })
        });

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        assert!(
            !fake.mutations().iter().any(|c| c.starts_with("delete_pod")),
            "deleted a pod that was already terminating"
        );
    }

    /// `CreateContainerConfigError` is settled by a most-recent Secret read,
    /// never by the reason string: Secret present → recoverable (observe).
    #[test]
    fn config_error_with_a_present_secret_is_recoverable() {
        let id = identity();
        let cfg = config();
        let pod = our_pod(&id, &cfg, Some(waiting("CreateContainerConfigError")));
        let referenced = crate::observe::referenced_secret(&pod).unwrap();
        let fake = Fake::default().with_pod(pod);
        fake.secrets.borrow_mut().push(crate::pod::build_secret(
            &id,
            &cfg.namespace,
            "gen-existing",
            env(),
        ));
        assert_eq!(referenced, id.secret_name("gen-existing"));

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(err.contains("startup not confirmed"), "got: {err}");
        assert!(
            !fake.mutations().iter().any(|c| c.starts_with("delete_pod")),
            "deleted a pod whose Secret exists"
        );
    }

    /// ...and Secret confirmed absent → provably broken → fenced replace.
    #[test]
    fn config_error_with_a_confirmed_absent_secret_is_replaced() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default().with_pod(our_pod(
            &id,
            &cfg,
            Some(waiting("CreateContainerConfigError")),
        ));
        fake.on_poll.borrow_mut().push({
            let name = id.pod_name();
            Box::new(move |pods: &mut BTreeMap<String, Pod>| {
                if let Some(pod) = pods.get_mut(&name) {
                    if pod
                        .status
                        .as_ref()
                        .and_then(|s| s.container_statuses.as_ref())
                        .is_none()
                    {
                        pod.status = Some(PodStatus {
                            phase: Some("Running".into()),
                            container_statuses: Some(vec![ContainerStatus {
                                name: crate::observe::CONTAINER_NAME.into(),
                                state: Some(running()),
                                ..Default::default()
                            }]),
                            ..Default::default()
                        });
                    }
                }
            })
        });

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        assert!(fake.mutations().iter().any(|c| c.starts_with("delete_pod")));
    }

    /// The generation is the Secret's name suffix *and* the lifecycle
    /// correlator inside it. They must name the same generation, or pod logs
    /// point at a Secret that was never mounted. The caller stamps one
    /// generation into the env once; each create attempt mints its own, so
    /// only a restamp keeps the pair together across a retry.
    #[test]
    fn each_attempt_stamps_its_own_generation_into_its_own_secret() {
        let id = identity();
        let cfg = config();
        // First attempt loses the create race and is cleaned up; the operation
        // then adopts. Two creates, two generations.
        let fake = Fake::default();
        *fake.winner.borrow_mut() = Some(our_pod(&id, &cfg, Some(running())));
        fake.create_outcomes
            .borrow_mut()
            .push(CreateOutcome::AlreadyExists);

        run(&fake, &id, &cfg).unwrap();

        let created = fake.created_secrets.borrow();
        // Guard the guard: this assertion is over a list that cleanup empties,
        // so an empty list would make every check below vacuously true.
        assert_eq!(created.len(), 1, "expected one create attempt");
        for secret in created.iter() {
            let name = secret.metadata.name.as_deref().unwrap();
            let nonce = secret
                .string_data
                .as_ref()
                .and_then(|d| d.get(crate::env::START_NONCE_KEY))
                .expect("no lifecycle correlator in the Secret");
            assert!(
                name.ends_with(nonce.as_str()),
                "secret {name} carries a correlator for a different generation ({nonce})"
            );
        }
    }

    // ---- bounded replacement ------------------------------------------------

    /// Installs a hook that makes every pod under `name` crash-exit before the
    /// next poll — a deterministic startup failure (the harness starts, rejects
    /// its configuration, and exits) as observed live: the container reaches
    /// `state.terminated` with a nonzero exit code.
    fn crash_exits_immediately(fake: &Fake, name: String) {
        fake.on_poll
            .borrow_mut()
            .push(Box::new(move |pods: &mut BTreeMap<String, Pod>| {
                if let Some(pod) = pods.get_mut(&name) {
                    if pod
                        .status
                        .as_ref()
                        .and_then(|s| s.container_statuses.as_ref())
                        .is_none()
                    {
                        pod.status = Some(PodStatus {
                            phase: Some("Failed".into()),
                            container_statuses: Some(vec![ContainerStatus {
                                name: crate::observe::CONTAINER_NAME.into(),
                                state: Some(ContainerState {
                                    terminated: Some(ContainerStateTerminated {
                                        exit_code: 1,
                                        reason: Some("Error".into()),
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            }]),
                            ..Default::default()
                        });
                    }
                }
            }));
    }

    /// A pod that this call created and that crash-exits deterministically is
    /// reported in-band after exactly ONE attempt — never hot-replaced until
    /// the deadline. The live failure this pins: one delete/create cycle every
    /// ~4s minted 107 immutable Secrets in a single 600s deploy call, all
    /// younger than the orphan sweep's age gate.
    #[test]
    fn a_deterministic_crash_exit_is_reported_not_hot_replaced() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default();
        crash_exits_immediately(&fake, id.pod_name());

        let err = run(&fake, &id, &cfg).unwrap_err();

        let creates = fake
            .mutations()
            .iter()
            .filter(|c| c.starts_with("create_secret"))
            .count();
        assert_eq!(
            creates, 1,
            "hot replacement loop: {creates} Secrets minted in one deploy call"
        );
        assert!(
            err.contains("exited with code 1"),
            "error does not carry the exit: {err}"
        );
        // The failed attempt is left in place as evidence — no cleanup on the
        // error path. Its Secret stays referenced by the terminated pod, so
        // the next deploy's preflight GC collects both together.
        assert!(
            !fake
                .mutations()
                .iter()
                .any(|c| c.starts_with("delete_pod") || c.starts_with("delete_secret")),
            "the failed attempt was cleaned up on the error path: {:?}",
            fake.mutations()
        );
        assert!(
            fake.elapsed() < DEADLINE,
            "burned the whole deadline on a deterministic failure"
        );
    }

    /// The revive path is untouched: terminated residue from a *previous*
    /// life is still replaced — the bound is on pods this call created, not
    /// on the row.
    #[test]
    fn pre_existing_terminated_residue_is_still_replaced_once() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default().with_pod(our_pod(&id, &cfg, Some(terminated())));
        crash_exits_immediately(&fake, id.pod_name());

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(err.contains("exited with code 1"), "got: {err}");

        let calls = fake.mutations();
        let deletes = calls.iter().filter(|c| c.starts_with("delete_pod")).count();
        let creates = calls
            .iter()
            .filter(|c| c.starts_with("create_secret"))
            .count();
        // Exactly one residue delete (the normal restart path) and one fresh
        // attempt — then report, not another cycle.
        assert_eq!(deletes, 1, "unexpected delete traffic: {calls:?}");
        assert_eq!(creates, 1, "unexpected create traffic: {calls:?}");
    }

    /// Retry is gated on fresh owner intent: the *next* deploy call clears
    /// the crashed attempt (preflight GC collects the terminated pod and its
    /// referenced Secret together) and makes exactly one new attempt — total
    /// litter stays one pod + one Secret however many times Start is pressed.
    #[test]
    fn the_next_deploy_collects_the_crashed_attempt_before_its_own_attempt() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default();
        crash_exits_immediately(&fake, id.pod_name());

        run(&fake, &id, &cfg).unwrap_err();
        // Second Start: fresh call, fresh clock.
        *fake.elapsed.borrow_mut() = Duration::ZERO;
        run(&fake, &id, &cfg).unwrap_err();

        assert_eq!(
            fake.secrets.borrow().len(),
            1,
            "crashed attempts accumulated Secrets across calls"
        );
        assert_eq!(fake.pods.borrow().len(), 1, "crashed pods accumulated");
    }

    // ---- the auto-repair fence ----------------------------------------------

    /// An object under our deterministic name that lacks the management
    /// marker is not ours. The reconciler must not adopt it, delete it, or
    /// return its name — it fails closed to the operator (`:1156-1162`).
    #[test]
    fn an_unmarked_look_alike_is_never_touched_or_adopted() {
        let id = identity();
        let cfg = config();
        let mut look_alike = our_pod(&id, &cfg, Some(running()));
        look_alike
            .metadata
            .labels
            .as_mut()
            .unwrap()
            .remove(LABEL_MANAGED_BY);
        let fake = Fake::default();
        // Our create loses to the object already sitting on the name.
        *fake.winner.borrow_mut() = Some(look_alike);
        fake.create_outcomes
            .borrow_mut()
            .push(CreateOutcome::AlreadyExists);

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(err.contains("not managed by this provider"), "got: {err}");
        assert!(
            !fake.mutations().iter().any(|c| c.starts_with("delete_pod")),
            "deleted an object we do not own"
        );
    }

    /// Same fence, other direction: the marker is present but the annotation
    /// carries a different agent's pubkey. The 32-hex label is
    /// collision-resistant, not collision-free (`:1152-1155`).
    #[test]
    fn a_pubkey_mismatch_is_never_adopted() {
        let id = identity();
        let other = identity();
        let cfg = config();
        let mut foreign = our_pod(&id, &cfg, Some(running()));
        foreign.metadata.annotations.as_mut().unwrap().insert(
            ANNOTATION_PUBKEY_FULL.to_string(),
            other.pubkey_hex().to_string(),
        );
        let fake = Fake::default();
        *fake.winner.borrow_mut() = Some(foreign);
        fake.create_outcomes
            .borrow_mut()
            .push(CreateOutcome::AlreadyExists);

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(err.contains("different agent identity"), "got: {err}");
        assert!(!fake.mutations().iter().any(|c| c.starts_with("delete_pod")));
    }

    // ---- create-conflict convergence ---------------------------------------

    /// The loser adopts the winner, returns the winner's id, and deletes
    /// **only its own** Secret — never the winner's (`:1259-1264`).
    #[test]
    fn create_loser_adopts_the_winner_and_drops_only_its_own_secret() {
        let id = identity();
        let cfg = config();
        let winner = our_pod(&id, &cfg, Some(running()));
        let winners_secret = crate::observe::referenced_secret(&winner).unwrap();
        let fake = Fake::default();
        *fake.winner.borrow_mut() = Some(winner);
        fake.secrets.borrow_mut().push(crate::pod::build_secret(
            &id,
            &cfg.namespace,
            "gen-existing",
            env(),
        ));
        fake.create_outcomes
            .borrow_mut()
            .push(CreateOutcome::AlreadyExists);

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());

        let deleted: Vec<_> = fake
            .mutations()
            .into_iter()
            .filter(|c| c.starts_with("delete_secret"))
            .collect();
        assert_eq!(
            deleted.len(),
            1,
            "expected exactly our own Secret dropped: {deleted:?}"
        );
        assert!(
            !deleted[0].contains(&winners_secret),
            "deleted the winner's Secret: {deleted:?}"
        );
        assert!(
            fake.secrets
                .borrow()
                .iter()
                .any(|s| s.metadata.name.as_deref() == Some(winners_secret.as_str())),
            "the winner's Secret is gone"
        );
    }

    /// The loser must not apply the divergence row to the pod that just beat
    /// it — that is the ping-pong the spec forbids (`:845-850`). A divergent
    /// *started* winner is adopted as-is.
    #[test]
    fn create_loser_does_not_replace_a_divergent_winner() {
        let id = identity();
        let cfg = config();
        let mut winner = our_pod(&id, &cfg, Some(running()));
        winner.metadata.annotations.as_mut().unwrap().insert(
            ANNOTATION_CREATE_INTENT.to_string(),
            "a-different-intent".into(),
        );
        let fake = Fake::default();
        *fake.winner.borrow_mut() = Some(winner);
        fake.create_outcomes
            .borrow_mut()
            .push(CreateOutcome::AlreadyExists);

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        assert!(
            !fake.mutations().iter().any(|c| c.starts_with("delete_pod")),
            "the create loser deleted the winner"
        );
    }

    /// The loser waits for the winner to *start* — adopting a not-yet-started
    /// winner as success would report an agent that is not running.
    #[test]
    fn create_loser_waits_for_the_winner_to_start() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default();
        *fake.winner.borrow_mut() = Some(our_pod(&id, &cfg, Some(waiting("ContainerCreating"))));
        fake.create_outcomes
            .borrow_mut()
            .push(CreateOutcome::AlreadyExists);
        let polls = RefCell::new(0);
        fake.on_poll.borrow_mut().push({
            let name = id.pod_name();
            Box::new(move |pods: &mut BTreeMap<String, Pod>| {
                *polls.borrow_mut() += 1;
                if *polls.borrow() >= 3 {
                    if let Some(pod) = pods.get_mut(&name) {
                        pod.status.as_mut().unwrap().container_statuses =
                            Some(vec![ContainerStatus {
                                name: crate::observe::CONTAINER_NAME.into(),
                                state: Some(running()),
                                ..Default::default()
                            }]);
                    }
                }
            })
        });

        assert_eq!(run(&fake, &id, &cfg).unwrap(), id.pod_name());
        assert!(fake.elapsed() > Duration::ZERO, "did not wait at all");
    }

    // ---- deadline and namespace --------------------------------------------

    /// Deadline expiry reports the *latest* condition, not a generic timeout
    /// (`:699-701`), and triggers no cleanup (`:720-724`).
    #[test]
    fn deadline_expiry_reports_the_condition_and_cleans_up_nothing() {
        let id = identity();
        let cfg = config();
        let fake = Fake::default().with_pod(our_pod(&id, &cfg, Some(waiting("ContainerCreating"))));

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(err.contains("ContainerCreating"), "generic timeout: {err}");
        let calls = fake.mutations();
        assert!(
            !calls.iter().any(|c| c.starts_with("delete_")),
            "deadline expiry triggered cleanup: {calls:?}"
        );
    }

    /// An RBAC denial on namespace create fails the deploy with the literal
    /// command to run, before any Secret is written (`:1002-1005`).
    #[test]
    fn namespace_denial_fails_before_writing_any_secret() {
        let id = identity();
        let cfg = config();
        let fake = Fake {
            namespace_error: Some(format!(
                "not authorized to create namespaces: run `kubectl create namespace {}`",
                cfg.namespace
            )),
            ..Default::default()
        };

        let err = run(&fake, &id, &cfg).unwrap_err();
        assert!(err.contains("kubectl create namespace"), "got: {err}");
        assert!(
            fake.mutations().is_empty(),
            "wrote something after a namespace denial: {:?}",
            fake.mutations()
        );
    }

    /// GC failures are hygiene, not deploy failures: a list the user cannot
    /// perform must not block a deploy they can.
    #[test]
    fn a_gc_failure_does_not_fail_the_deploy() {
        struct GcDenied(Fake);
        impl Substrate for GcDenied {
            async fn ensure_namespace(&self, ns: &str) -> Result<(), String> {
                self.0.ensure_namespace(ns).await
            }
            async fn list_pods(
                &self,
                _s: &str,
            ) -> Result<(Vec<Pod>, Option<DateTime<Utc>>), String> {
                Err("forbidden: cannot list pods".into())
            }
            async fn list_secrets(&self, _s: &str) -> Result<Vec<Secret>, String> {
                Err("forbidden: cannot list secrets".into())
            }
            async fn secret_exists(&self, n: &str) -> Result<bool, String> {
                self.0.secret_exists(n).await
            }
            async fn create_secret(&self, s: &Secret) -> Result<(), String> {
                self.0.create_secret(s).await
            }
            async fn create_pod(&self, p: &Pod) -> Result<CreateOutcome, String> {
                self.0.create_pod(p).await
            }
            async fn delete_pod(&self, n: &str, f: &Fence) -> Result<DeleteOutcome, String> {
                self.0.delete_pod(n, f).await
            }
            async fn delete_secret(&self, n: &str) -> Result<(), String> {
                self.0.delete_secret(n).await
            }
            async fn get_pod(&self, n: &str) -> Result<Option<Pod>, String> {
                self.0.get_pod(n).await
            }
            async fn sleep(&self, d: Duration) {
                self.0.sleep(d).await
            }
            fn elapsed(&self) -> Duration {
                self.0.elapsed()
            }
        }

        let id = identity();
        let cfg = config();
        let inner = Fake::default().with_pod(our_pod(&id, &cfg, Some(running())));
        let fake = GcDenied(inner);

        assert_eq!(
            block_on(deploy(&fake, &id, &cfg, env())).unwrap(),
            id.pod_name()
        );
    }
}
