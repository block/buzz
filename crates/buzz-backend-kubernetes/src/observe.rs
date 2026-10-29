//! Decoding API objects into verified observations (spec §Deploy State
//! Machine step 1).
//!
//! Pure: `Pod` in, [`VerifiedPod`] out. Keeping the decode here means the
//! conformance tests drive the *shipped* decoder with real API types rather
//! than a test-only stand-in, and it keeps `classify.rs` free of API types.
//!
//! Verification is the gate, not a filter: [`verify`] returns `None` for any
//! object whose full-pubkey annotation does not equal the derived pubkey or
//! that lacks the management marker, so an unverified object cannot reach
//! classification, deletion, or the returned `agent_id`.

use crate::classify::{Fence, PullFailure, Startup, VerifiedPod};
use crate::intent::Fingerprint;
use crate::naming::{
    AgentIdentity, ANNOTATION_CREATE_INTENT, ANNOTATION_PUBKEY_FULL, BINDING_VERSION,
    LABEL_BINDING_VERSION, LABEL_MANAGED_BY, MANAGED_BY,
};
use k8s_openapi::api::core::v1::{Pod, Secret};

/// Container name the provider creates; status is read from this container.
pub const CONTAINER_NAME: &str = "agent";

/// The startup state, or a state that cannot be settled without one more read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupObservation {
    Resolved(Startup),
    /// `CreateContainerConfigError` — recoverable *unless* the referenced
    /// Secret is confirmed absent by a most-recent read. The kubelet's reason
    /// string is a hint; the provider verifies before treating it as fatal
    /// (§Deploy State Machine: "provably" means a verified absence, never a
    /// reason string).
    ConfigErrorPendingSecretCheck {
        secret_name: String,
    },
}

/// Does this object carry the management marker (§Pod shape)?
///
/// Identity labels prove identity; the marker asserts protocol ownership.
/// Without it an object that merely matches our schema fails closed.
fn has_marker(labels: Option<&std::collections::BTreeMap<String, String>>) -> bool {
    let Some(labels) = labels else { return false };
    labels.get(LABEL_MANAGED_BY).map(String::as_str) == Some(MANAGED_BY)
        && labels.get(LABEL_BINDING_VERSION).map(String::as_str) == Some(BINDING_VERSION)
}

/// Does the full-pubkey annotation equal the derived pubkey?
///
/// The 32-hex label is collision-*resistant*, not collision-free, which is
/// why this check is normative rather than decorative (`:1152-1166`).
fn annotation_matches(
    annotations: Option<&std::collections::BTreeMap<String, String>>,
    identity: &AgentIdentity,
) -> bool {
    annotations
        .and_then(|a| a.get(ANNOTATION_PUBKEY_FULL))
        .map(|v| v == identity.pubkey_hex())
        .unwrap_or(false)
}

/// Is this Secret ours and this identity's? The same fence GC applies before
/// deleting anything.
pub fn secret_is_ours(secret: &Secret, identity: &AgentIdentity) -> bool {
    has_marker(secret.metadata.labels.as_ref())
        && annotation_matches(secret.metadata.annotations.as_ref(), identity)
}

/// Decode a pod's startup state from its status.
///
/// "Started" means `state.running` on our container — not pod phase. A pod can
/// sit in phase `Running` with a container that never started, and a pod being
/// gracefully deleted stays in phase `Running` for its whole grace period.
pub fn decode_startup(pod: &Pod) -> StartupObservation {
    use StartupObservation::Resolved;

    let status = pod.status.as_ref();
    let phase = status.and_then(|s| s.phase.as_deref());

    let container = status
        .and_then(|s| s.container_statuses.as_ref())
        .and_then(|cs| cs.iter().find(|c| c.name == CONTAINER_NAME));

    if let Some(state) = container.and_then(|c| c.state.as_ref()) {
        if state.running.is_some() {
            return Resolved(Startup::Started);
        }
        if state.terminated.is_some() {
            return Resolved(Startup::Terminated);
        }
        if let Some(waiting) = state.waiting.as_ref() {
            let reason = waiting.reason.as_deref().unwrap_or_default();
            let message = waiting.message.as_deref().unwrap_or_default();
            return match reason {
                // Structurally invalid reference: no retry can fix it.
                "InvalidImageName" => Resolved(Startup::NeverStartedProvablyBroken),
                "ErrImagePull" | "ImagePullBackOff" => match classify_pull_failure(message) {
                    Some(failure) => Resolved(Startup::NeverStartedPullFailing(failure)),
                    None => Resolved(Startup::NeverStartedRecoverable),
                },
                "CreateContainerConfigError" => match referenced_secret(pod) {
                    Some(secret_name) => {
                        StartupObservation::ConfigErrorPendingSecretCheck { secret_name }
                    }
                    None => Resolved(Startup::NeverStartedRecoverable),
                },
                _ => Resolved(Startup::NeverStartedRecoverable),
            };
        }
    }

    // No container status yet (unscheduled, image pulling before the kubelet
    // reports, quota-blocked). A terminal phase without container status still
    // means the pod is done.
    match phase {
        Some("Succeeded") | Some("Failed") => Resolved(Startup::Terminated),
        _ => Resolved(Startup::NeverStartedRecoverable),
    }
}

/// The Secret name this pod's `envFrom` references, if any.
pub fn referenced_secret(pod: &Pod) -> Option<String> {
    pod.spec
        .as_ref()?
        .containers
        .iter()
        .flat_map(|c| c.env_from.iter().flatten())
        .find_map(|source| source.secret_ref.as_ref().map(|r| r.name.clone()))
}

/// Classify a pull failure from the kubelet's message.
///
/// Reporting only — [`PullFailure`] is structurally excluded from
/// `Action::Delete`, so a wrong guess here can delay a report but can never
/// destroy anything. `None` means "no permanent cause recognized", which
/// leaves the pod on the ordinary observational path.
fn classify_pull_failure(message: &str) -> Option<PullFailure> {
    let m = message.to_ascii_lowercase();
    if m.contains("401")
        || m.contains("unauthorized")
        || m.contains("403")
        || m.contains("denied")
        || m.contains("authentication required")
    {
        return Some(PullFailure::Unauthorized);
    }
    if m.contains("manifest unknown")
        || m.contains("not found")
        || m.contains("manifest_unknown")
        || m.contains("repository does not exist")
    {
        return Some(PullFailure::ManifestUnknown);
    }
    if m.contains("no match for platform") || m.contains("no matching manifest") {
        return Some(PullFailure::ArchMismatch);
    }
    None
}

/// The redacted, actionable condition text for a pull failure.
///
/// Names the registry and the immutable reference — never credentials, and
/// never the kubelet's raw message, which can echo a registry token.
pub fn pull_failure_message(failure: PullFailure, image: &str) -> String {
    let registry = image.split('/').next().unwrap_or(image);
    match failure {
        PullFailure::Unauthorized => format!(
            "the cluster is not authorized to pull {image} from {registry}. \
             This pull retries indefinitely and will not succeed on its own: \
             grant the cluster's nodes access to that registry."
        ),
        PullFailure::ManifestUnknown => {
            format!("{registry} has no image at {image}. Check the digest and repository.")
        }
        PullFailure::ArchMismatch => format!(
            "{image} has no variant for the architecture of the nodes it was \
             scheduled on."
        ),
    }
}

/// The latest actionable condition for a pod that has not started, redacted.
///
/// Two sources, deliberately treated differently:
///
/// * The container's waiting **reason** is included; its **message** is not.
///   Waiting messages are kubelet-composed and echo the thing that failed —
///   for a pull that is the registry request, which can carry credential
///   material. The reason token alone (`ImagePullBackOff`,
///   `CreateContainerConfigError`) is the diagnostic; the message adds
///   exposure, not information the user can act on.
/// * Pod-condition messages **are** included. They are scheduler- and
///   kubelet-composed from the pod's own spec and cluster capacity
///   ("0/3 nodes are available: Insufficient memory"), which is precisely the
///   actionable half and contains nothing derived from Secret data.
pub fn condition(pod: &Pod) -> Option<String> {
    let status = pod.status.as_ref()?;

    if let Some(state) = status
        .container_statuses
        .as_ref()
        .and_then(|cs| cs.iter().find(|c| c.name == CONTAINER_NAME))
        .and_then(|c| c.state.as_ref())
    {
        if let Some(waiting) = state.waiting.as_ref() {
            if let Some(reason) = waiting.reason.as_deref() {
                return Some(format!("the container is waiting, reason {reason}"));
            }
        }
        // Exit code and reason only — the terminated `message` is
        // process-composed output and falls under the same redaction rule as
        // waiting messages.
        if let Some(terminated) = state.terminated.as_ref() {
            return Some(match terminated.reason.as_deref() {
                Some(reason) => format!(
                    "the container exited with code {} ({reason})",
                    terminated.exit_code
                ),
                None => format!("the container exited with code {}", terminated.exit_code),
            });
        }
    }

    if let Some((type_, reason, message)) = status.conditions.as_ref().and_then(|cs| {
        cs.iter().find(|c| c.status == "False").map(|c| {
            (
                c.type_.clone(),
                c.reason.clone().unwrap_or_default(),
                c.message.clone().unwrap_or_default(),
            )
        })
    }) {
        let detail = [reason, message]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(": ");
        return Some(if detail.is_empty() {
            format!("pod condition {type_} is false")
        } else {
            format!("pod condition {type_} is false: {detail}")
        });
    }

    status.phase.as_deref().map(|p| format!("the pod is {p}"))
}

/// Verify a label-selected pod and decode it, or reject it.
///
/// `startup` is supplied by the caller because settling
/// `CreateContainerConfigError` needs a most-recent Secret read the pure layer
/// must not perform.
pub fn verify(pod: &Pod, identity: &AgentIdentity, startup: Startup) -> Option<VerifiedPod> {
    if !has_marker(pod.metadata.labels.as_ref()) {
        return None;
    }
    if !annotation_matches(pod.metadata.annotations.as_ref(), identity) {
        return None;
    }
    Some(VerifiedPod {
        name: pod.metadata.name.clone()?,
        fence: Fence {
            uid: pod.metadata.uid.clone()?,
            resource_version: pod.metadata.resource_version.clone()?,
        },
        deletion_marked: pod.metadata.deletion_timestamp.is_some(),
        startup,
        recorded_intent: pod
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(ANNOTATION_CREATE_INTENT))
            .map(|v| Fingerprint::from_annotation(v)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        ContainerState, ContainerStateRunning, ContainerStateTerminated, ContainerStateWaiting,
        ContainerStatus, PodStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
    use std::collections::BTreeMap;

    fn identity() -> AgentIdentity {
        use nostr::nips::nip19::ToBech32;
        let keys = nostr::Keys::generate();
        AgentIdentity::from_nsec(&keys.secret_key().to_bech32().unwrap()).unwrap()
    }

    fn base_pod(id: &AgentIdentity) -> Pod {
        Pod {
            metadata: ObjectMeta {
                name: Some(id.pod_name()),
                uid: Some("uid-1".into()),
                resource_version: Some("100".into()),
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
            ..Default::default()
        }
    }

    fn with_container_state(mut pod: Pod, state: ContainerState) -> Pod {
        pod.status = Some(PodStatus {
            phase: Some("Running".into()),
            container_statuses: Some(vec![ContainerStatus {
                name: CONTAINER_NAME.into(),
                state: Some(state),
                ..Default::default()
            }]),
            ..Default::default()
        });
        pod
    }

    fn waiting(reason: &str, message: &str) -> ContainerState {
        ContainerState {
            waiting: Some(ContainerStateWaiting {
                reason: Some(reason.into()),
                message: Some(message.into()),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn running_container_is_started() {
        let id = identity();
        let pod = with_container_state(
            base_pod(&id),
            ContainerState {
                running: Some(ContainerStateRunning::default()),
                ..Default::default()
            },
        );
        assert_eq!(
            decode_startup(&pod),
            StartupObservation::Resolved(Startup::Started)
        );
    }

    /// Pod phase is not the criterion. A pod in phase `Running` whose
    /// container never started must NOT read as started, or the reconciler
    /// no-ops on a pod that will never serve.
    #[test]
    fn phase_running_with_waiting_container_is_not_started() {
        let id = identity();
        let pod = with_container_state(base_pod(&id), waiting("ContainerCreating", ""));
        assert_eq!(
            decode_startup(&pod),
            StartupObservation::Resolved(Startup::NeverStartedRecoverable)
        );
    }

    #[test]
    fn terminated_container_is_terminated() {
        let id = identity();
        let pod = with_container_state(
            base_pod(&id),
            ContainerState {
                terminated: Some(ContainerStateTerminated {
                    exit_code: 0,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        assert_eq!(
            decode_startup(&pod),
            StartupObservation::Resolved(Startup::Terminated)
        );
    }

    /// A terminal phase with no container status (evicted before the kubelet
    /// reported) is still terminated — otherwise the residue is never GC'd.
    #[test]
    fn terminal_phase_without_container_status_is_terminated() {
        let id = identity();
        for phase in ["Succeeded", "Failed"] {
            let mut pod = base_pod(&id);
            pod.status = Some(PodStatus {
                phase: Some(phase.into()),
                ..Default::default()
            });
            assert_eq!(
                decode_startup(&pod),
                StartupObservation::Resolved(Startup::Terminated),
                "phase {phase}"
            );
        }
    }

    #[test]
    fn invalid_image_name_is_provably_broken() {
        let id = identity();
        let pod = with_container_state(base_pod(&id), waiting("InvalidImageName", "bad ref"));
        assert_eq!(
            decode_startup(&pod),
            StartupObservation::Resolved(Startup::NeverStartedProvablyBroken)
        );
    }

    /// Permanent pull failures are recognized from the message; anything
    /// unrecognized stays on the ordinary observational path rather than
    /// being guessed at.
    #[test]
    fn permanent_pull_failures_are_classified() {
        let id = identity();
        let cases = [
            ("401 Unauthorized", PullFailure::Unauthorized),
            ("pull access denied", PullFailure::Unauthorized),
            ("manifest unknown", PullFailure::ManifestUnknown),
            (
                "no match for platform in manifest",
                PullFailure::ArchMismatch,
            ),
        ];
        for (message, expected) in cases {
            let pod = with_container_state(base_pod(&id), waiting("ErrImagePull", message));
            assert_eq!(
                decode_startup(&pod),
                StartupObservation::Resolved(Startup::NeverStartedPullFailing(expected)),
                "message {message:?}"
            );
        }

        let pod = with_container_state(
            base_pod(&id),
            waiting("ImagePullBackOff", "dial tcp: i/o timeout"),
        );
        assert_eq!(
            decode_startup(&pod),
            StartupObservation::Resolved(Startup::NeverStartedRecoverable),
            "a transient network failure must not be reported as permanent"
        );
    }

    /// The kubelet's reason string is a hint, not proof: a config error defers
    /// to a most-recent Secret read before anything is called broken.
    #[test]
    fn config_error_defers_to_a_secret_read() {
        let id = identity();
        let mut pod =
            with_container_state(base_pod(&id), waiting("CreateContainerConfigError", ""));
        pod.spec = Some(k8s_openapi::api::core::v1::PodSpec {
            containers: vec![k8s_openapi::api::core::v1::Container {
                name: CONTAINER_NAME.into(),
                env_from: Some(vec![k8s_openapi::api::core::v1::EnvFromSource {
                    secret_ref: Some(k8s_openapi::api::core::v1::SecretEnvSource {
                        name: "buzz-agent-abc-gen1".into(),
                        optional: Some(false),
                    }),
                    ..Default::default()
                }]),
                ..Default::default()
            }],
            ..Default::default()
        });
        assert_eq!(
            decode_startup(&pod),
            StartupObservation::ConfigErrorPendingSecretCheck {
                secret_name: "buzz-agent-abc-gen1".into()
            }
        );
    }

    /// The auto-repair fence: an object that matches our schema but lacks the
    /// marker, or carries someone else's pubkey, is never verified — so it can
    /// never be no-op'd against, deleted, or returned as an `agent_id`.
    #[test]
    fn unmarked_or_mismatched_objects_fail_verification() {
        let id = identity();
        let other = identity();

        let mut unmarked = base_pod(&id);
        unmarked.metadata.labels = Some(BTreeMap::new());
        assert!(
            verify(&unmarked, &id, Startup::Started).is_none(),
            "unmarked pod verified"
        );

        let mut wrong_version = base_pod(&id);
        let mut labels = id.labels();
        labels.insert(LABEL_BINDING_VERSION.to_string(), "999".to_string());
        wrong_version.metadata.labels = Some(labels);
        assert!(verify(&wrong_version, &id, Startup::Started).is_none());

        let mut mismatched = base_pod(&id);
        mismatched.metadata.annotations = Some(
            [(
                ANNOTATION_PUBKEY_FULL.to_string(),
                other.pubkey_hex().to_string(),
            )]
            .into_iter()
            .collect(),
        );
        assert!(
            verify(&mismatched, &id, Startup::Started).is_none(),
            "collision verified"
        );

        let mut missing = base_pod(&id);
        missing.metadata.annotations = Some(BTreeMap::new());
        assert!(verify(&missing, &id, Startup::Started).is_none());

        assert!(
            verify(&base_pod(&id), &id, Startup::Started).is_some(),
            "own pod rejected"
        );
    }

    /// The fence must come from the observed object, and the deletion mark
    /// must be read even though the phase says `Running`.
    #[test]
    fn verified_pod_carries_the_fence_and_deletion_mark() {
        let id = identity();
        let mut pod = base_pod(&id);
        pod.metadata.deletion_timestamp = Some(Time(chrono::Utc::now()));
        let verified = verify(&pod, &id, Startup::Started).unwrap();
        assert_eq!(verified.fence.uid, "uid-1");
        assert_eq!(verified.fence.resource_version, "100");
        assert!(verified.deletion_marked);
    }

    /// A pod with no recorded intent reads as `None`, which the classifier
    /// groups with divergence.
    #[test]
    fn missing_intent_annotation_decodes_as_none() {
        let id = identity();
        assert!(verify(&base_pod(&id), &id, Startup::Started)
            .unwrap()
            .recorded_intent
            .is_none());
    }

    /// A pull-failure report must name the registry and the immutable
    /// reference and nothing else — never the kubelet's raw message, which
    /// can echo a registry token.
    #[test]
    fn pull_failure_messages_are_actionable_and_redacted() {
        let image = format!("ghcr.io/block/buzz-sprig@sha256:{}", "a".repeat(64));
        for failure in [
            PullFailure::Unauthorized,
            PullFailure::ManifestUnknown,
            PullFailure::ArchMismatch,
        ] {
            let msg = pull_failure_message(failure, &image);
            assert!(msg.contains("ghcr.io"), "{msg}");
            assert!(msg.contains(&image), "{msg}");
            for secret in ["Bearer", "password", "nsec1", "token"] {
                assert!(!msg.contains(secret), "leaked {secret}: {msg}");
            }
        }
    }

    #[test]
    fn secret_ownership_requires_marker_and_annotation() {
        let id = identity();
        let other = identity();
        let ours = Secret {
            metadata: ObjectMeta {
                labels: Some(id.labels()),
                annotations: Some(
                    [(
                        ANNOTATION_PUBKEY_FULL.to_string(),
                        id.pubkey_hex().to_string(),
                    )]
                    .into(),
                ),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(secret_is_ours(&ours, &id));
        assert!(!secret_is_ours(&ours, &other));

        let mut unmarked = ours.clone();
        unmarked.metadata.labels = Some(BTreeMap::new());
        assert!(!secret_is_ours(&unmarked, &id));
    }
}
