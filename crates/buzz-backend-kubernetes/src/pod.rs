//! Pod and Secret construction (spec §Pod shape, §K8s Secrets).
//!
//! The builder is pure: it turns resolved inputs into API objects and performs
//! no I/O, so every normative field is a unit assertion.

use crate::config::{
    ProviderConfig, RESTART_POLICY, RUN_AS_GID, RUN_AS_UID, TERMINATION_GRACE_SECONDS,
    WORKSPACE_PATH,
};
use crate::intent::{Fingerprint, IntentTemplate};
use crate::naming::{
    AgentIdentity, ANNOTATION_CREATE_INTENT, ANNOTATION_IMAGE, ANNOTATION_PUBKEY_FULL,
};
use k8s_openapi::api::core::v1::{
    Capabilities, Container, EmptyDirVolumeSource, EnvFromSource, Pod, PodSecurityContext, PodSpec,
    ResourceRequirements, SeccompProfile, Secret, SecretEnvSource, SecurityContext, Volume,
    VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use std::collections::BTreeMap;

/// Volume name for the agent's writable workspace.
const WORKSPACE_VOLUME: &str = "workspace";

/// The container name. Fixed: log and exec tooling addresses it by name.
const CONTAINER_NAME: &str = "agent";

/// Build the per-attempt Secret holding the resolved environment.
///
/// `immutable: true` — the Secret is written once per attempt and never
/// updated, which is what lets the pod's `envFrom` reference be treated as an
/// atomic binding to this exact payload (§K8s Secrets).
pub fn build_secret(
    identity: &AgentIdentity,
    namespace: &str,
    generation: &str,
    env: BTreeMap<String, String>,
) -> Secret {
    Secret {
        metadata: ObjectMeta {
            name: Some(identity.secret_name(generation)),
            namespace: Some(namespace.to_string()),
            labels: Some(identity.labels()),
            annotations: Some(
                [(
                    ANNOTATION_PUBKEY_FULL.to_string(),
                    identity.pubkey_hex().to_string(),
                )]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        },
        string_data: Some(env),
        immutable: Some(true),
        ..Default::default()
    }
}

/// Build the pod for one create attempt.
///
/// The `fingerprint` is computed from [`intent_template`] over a type that
/// cannot contain the generation or any Secret value, so it is stable across
/// attempts of the same configuration.
pub fn build_pod(
    identity: &AgentIdentity,
    cfg: &ProviderConfig,
    generation: &str,
    fingerprint: &Fingerprint,
) -> Pod {
    let annotations: BTreeMap<String, String> = [
        (
            ANNOTATION_PUBKEY_FULL.to_string(),
            identity.pubkey_hex().to_string(),
        ),
        (
            ANNOTATION_CREATE_INTENT.to_string(),
            fingerprint.as_str().to_string(),
        ),
        (ANNOTATION_IMAGE.to_string(), cfg.image.as_str().to_string()),
    ]
    .into_iter()
    .collect();

    let requests: BTreeMap<String, Quantity> = [
        (
            "cpu".to_string(),
            Quantity(cfg.resources.cpu_request.clone()),
        ),
        (
            "memory".to_string(),
            Quantity(cfg.resources.memory_request.clone()),
        ),
    ]
    .into_iter()
    .collect();
    let limits: BTreeMap<String, Quantity> = [
        ("cpu".to_string(), Quantity(cfg.resources.cpu_limit.clone())),
        (
            "memory".to_string(),
            Quantity(cfg.resources.memory_limit.clone()),
        ),
    ]
    .into_iter()
    .collect();

    let container = Container {
        name: CONTAINER_NAME.to_string(),
        image: Some(cfg.image.as_str().to_string()),
        // No `command`/`args`: the image's entrypoint execs the harness as
        // PID 1 (§Entrypoint). Overriding it here would be how a provider
        // accidentally puts a shell in front of the signal receiver.
        env_from: Some(vec![EnvFromSource {
            secret_ref: Some(SecretEnvSource {
                name: identity.secret_name(generation),
                optional: Some(false),
            }),
            ..Default::default()
        }]),
        resources: Some(ResourceRequirements {
            requests: Some(requests),
            limits: Some(limits),
            ..Default::default()
        }),
        volume_mounts: Some(vec![VolumeMount {
            name: WORKSPACE_VOLUME.to_string(),
            mount_path: WORKSPACE_PATH.to_string(),
            ..Default::default()
        }]),
        security_context: Some(SecurityContext {
            allow_privilege_escalation: Some(false),
            capabilities: Some(Capabilities {
                drop: Some(vec!["ALL".to_string()]),
                ..Default::default()
            }),
            // `readOnlyRootFilesystem` is deliberately unset: the sprig
            // toolchain writes outside the workspace mount (§Pod shape).
            ..Default::default()
        }),
        ..Default::default()
    };

    Pod {
        metadata: ObjectMeta {
            name: Some(identity.pod_name()),
            namespace: Some(cfg.namespace.clone()),
            labels: Some(identity.labels()),
            annotations: Some(annotations),
            ..Default::default()
        },
        spec: Some(PodSpec {
            containers: vec![container],
            restart_policy: Some(RESTART_POLICY.to_string()),
            termination_grace_period_seconds: Some(TERMINATION_GRACE_SECONDS),
            // The agent runs prompted, untrusted code while holding an nsec;
            // an ambient ServiceAccount token would be an API-stealable
            // credential it never needs (§Pod shape hardening). Naming a
            // service account selects a scheduling/RBAC identity and MUST NOT
            // re-enable token mounting.
            automount_service_account_token: Some(false),
            service_account_name: cfg.service_account.clone(),
            security_context: Some(PodSecurityContext {
                run_as_non_root: Some(true),
                run_as_user: Some(RUN_AS_UID),
                run_as_group: Some(RUN_AS_GID),
                fs_group: Some(RUN_AS_GID),
                seccomp_profile: Some(SeccompProfile {
                    type_: "RuntimeDefault".to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            volumes: Some(vec![Volume {
                name: WORKSPACE_VOLUME.to_string(),
                empty_dir: Some(EmptyDirVolumeSource::default()),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The create-intent template for this configuration (§Deploy State Machine).
pub fn intent_template(
    cfg: &ProviderConfig,
    env_keys: impl IntoIterator<Item = String>,
) -> IntentTemplate {
    IntentTemplate::new(
        &cfg.namespace,
        &cfg.image,
        &cfg.resources,
        cfg.service_account.as_deref(),
        env_keys,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn identity() -> AgentIdentity {
        use nostr::nips::nip19::ToBech32;
        let keys = nostr::Keys::generate();
        AgentIdentity::from_nsec(&keys.secret_key().to_bech32().unwrap()).unwrap()
    }

    fn provider_config() -> ProviderConfig {
        config::parse(&serde_json::json!({
            "namespace": "buzz-agents-test",
            "image": format!("ghcr.io/block/buzz-sprig@sha256:{}", "a".repeat(64)),
        }))
        .unwrap()
    }

    fn pod() -> Pod {
        let cfg = provider_config();
        build_pod(
            &identity(),
            &cfg,
            "gen00001",
            &intent_template(&cfg, ["BUZZ_RELAY_URL".to_string()]).fingerprint(),
        )
    }

    fn spec(pod: &Pod) -> &PodSpec {
        pod.spec.as_ref().unwrap()
    }

    /// Every hardening default from §Pod shape, asserted individually so a
    /// dropped one names itself.
    #[test]
    fn hardening_defaults_are_all_present() {
        let pod = pod();
        let spec = spec(&pod);
        assert_eq!(spec.automount_service_account_token, Some(false));

        let sc = spec
            .security_context
            .as_ref()
            .expect("pod security context");
        assert_eq!(sc.run_as_non_root, Some(true));
        assert_eq!(sc.run_as_user, Some(RUN_AS_UID));
        assert_ne!(sc.run_as_user, Some(0), "root UID");
        assert_eq!(sc.run_as_group, Some(RUN_AS_GID));
        assert_eq!(
            sc.seccomp_profile.as_ref().map(|p| p.type_.as_str()),
            Some("RuntimeDefault")
        );

        let csc = spec.containers[0]
            .security_context
            .as_ref()
            .expect("container sc");
        assert_eq!(csc.allow_privilege_escalation, Some(false));
        assert_eq!(
            csc.capabilities.as_ref().and_then(|c| c.drop.clone()),
            Some(vec!["ALL".to_string()])
        );
        assert_ne!(csc.privileged, Some(true));
    }

    /// The forbidden host-namespace and hostPath escapes, asserted as absence.
    #[test]
    fn never_uses_host_namespaces_or_host_paths() {
        let pod = pod();
        let spec = spec(&pod);
        assert!(spec.host_pid.is_none() || spec.host_pid == Some(false));
        assert!(spec.host_network.is_none() || spec.host_network == Some(false));
        assert!(spec.host_ipc.is_none() || spec.host_ipc == Some(false));
        for volume in spec.volumes.as_ref().unwrap() {
            assert!(
                volume.host_path.is_none(),
                "hostPath volume {}",
                volume.name
            );
        }
    }

    /// `Never` only. `OnFailure` is gated on the harness exit-code contract
    /// *and* a crash-loop classification row (`:1121-1139`); the config layer
    /// refuses `inactivity_seconds: 0` so this arm is unreachable, and the
    /// assertion keeps it that way.
    #[test]
    fn restart_policy_is_never() {
        assert_eq!(spec(&pod()).restart_policy.as_deref(), Some("Never"));
    }

    /// 60s, not Kubernetes' default 30s — which would SIGKILL the harness
    /// mid-drain and leave presence stale-online (§Pod shape).
    #[test]
    fn declares_the_sixty_second_grace_budget() {
        assert_eq!(spec(&pod()).termination_grace_period_seconds, Some(60));
    }

    /// The pod must not override the image's entrypoint: the image execs the
    /// harness as PID 1, and a `command` here is how a shell ends up in front
    /// of the signal receiver (§Entrypoint).
    #[test]
    fn does_not_override_the_image_entrypoint() {
        let pod = pod();
        let container = &spec(&pod).containers[0];
        assert!(container.command.is_none(), "overrode the entrypoint");
        assert!(container.args.is_none());
    }

    #[test]
    fn workspace_is_an_emptydir_mounted_at_home() {
        let pod = pod();
        let spec = spec(&pod);
        let volume = &spec.volumes.as_ref().unwrap()[0];
        assert!(volume.empty_dir.is_some());
        assert!(volume.persistent_volume_claim.is_none());
        let mount = &spec.containers[0].volume_mounts.as_ref().unwrap()[0];
        assert_eq!(mount.name, volume.name);
        assert_eq!(mount.mount_path, WORKSPACE_PATH);
    }

    #[test]
    fn resources_carry_the_configured_requests_and_limits() {
        let mut cfg = provider_config();
        cfg.resources.cpu_limit = "4".into();
        let pod = build_pod(&identity(), &cfg, "g", &Fingerprint::from_annotation("f"));
        let r = spec(&pod).containers[0].resources.as_ref().unwrap();
        assert_eq!(r.requests.as_ref().unwrap()["cpu"], Quantity("1".into()));
        assert_eq!(
            r.requests.as_ref().unwrap()["memory"],
            Quantity("2Gi".into())
        );
        assert_eq!(r.limits.as_ref().unwrap()["cpu"], Quantity("4".into()));
        assert_eq!(r.limits.as_ref().unwrap()["memory"], Quantity("4Gi".into()));
    }

    /// `envFrom` must point at this attempt's Secret and must NOT be optional:
    /// an optional reference starts the container with no identity at all,
    /// turning a missing-Secret bug into an agent that silently cannot
    /// authenticate.
    #[test]
    fn env_from_references_this_attempts_secret_and_is_required() {
        let id = identity();
        let cfg = provider_config();
        let pod = build_pod(&id, &cfg, "gen00042", &Fingerprint::from_annotation("f"));
        let source = &spec(&pod).containers[0].env_from.as_ref().unwrap()[0];
        let secret_ref = source.secret_ref.as_ref().unwrap();
        assert_eq!(secret_ref.name, id.secret_name("gen00042"));
        assert_eq!(secret_ref.optional, Some(false));
        assert!(source.config_map_ref.is_none());
    }

    /// Identity, ownership marker, and the recorded intent all travel on the
    /// pod — the GC and reconciliation fences read exactly these.
    #[test]
    fn pod_carries_identity_marker_and_recorded_intent() {
        let id = identity();
        let cfg = provider_config();
        let fp = intent_template(&cfg, ["A".to_string()]).fingerprint();
        let pod = build_pod(&id, &cfg, "g", &fp);
        let meta = &pod.metadata;
        assert_eq!(meta.name.as_deref(), Some(id.pod_name().as_str()));
        assert_eq!(meta.namespace.as_deref(), Some("buzz-agents-test"));
        assert_eq!(meta.labels.as_ref().unwrap(), &id.labels());
        let ann = meta.annotations.as_ref().unwrap();
        assert_eq!(ann[ANNOTATION_PUBKEY_FULL], id.pubkey_hex());
        assert_eq!(ann[ANNOTATION_CREATE_INTENT], fp.as_str());
        assert_eq!(ann[ANNOTATION_IMAGE], cfg.image.as_str());
    }

    /// The Secret is immutable and marker-bearing: immutability is what makes
    /// the pod's `envFrom` an atomic binding, and the marker is what GC
    /// requires before it will delete anything.
    #[test]
    fn secret_is_immutable_marked_and_holds_the_env() {
        let id = identity();
        let env: BTreeMap<String, String> =
            [("BUZZ_RELAY_URL".to_string(), "wss://r".to_string())].into();
        let secret = build_secret(&id, "ns", "gen1", env.clone());
        assert_eq!(secret.immutable, Some(true));
        assert_eq!(secret.string_data.as_ref().unwrap(), &env);
        assert_eq!(
            secret.metadata.name.as_deref(),
            Some(id.secret_name("gen1").as_str())
        );
        assert_eq!(secret.metadata.labels.as_ref().unwrap(), &id.labels());
        assert_eq!(
            secret.metadata.annotations.as_ref().unwrap()[ANNOTATION_PUBKEY_FULL],
            id.pubkey_hex()
        );
        // `data` must stay unset — setting both is an apiserver rejection.
        assert!(secret.data.is_none());
    }

    /// Naming a service account selects a scheduling identity; it must not
    /// re-enable token mounting (§Pod shape hardening, `:1221-1225`).
    #[test]
    fn service_account_does_not_re_enable_token_mounting() {
        let mut cfg = provider_config();
        cfg.service_account = Some("agent-sa".into());
        let pod = build_pod(&identity(), &cfg, "g", &Fingerprint::from_annotation("f"));
        assert_eq!(spec(&pod).service_account_name.as_deref(), Some("agent-sa"));
        assert_eq!(spec(&pod).automount_service_account_token, Some(false));
    }

    /// The fingerprint recorded on the pod is the one the classifier will
    /// recompute — pinned end-to-end so a builder change that forgets to feed
    /// the template a field cannot pass silently.
    #[test]
    fn recorded_fingerprint_matches_a_fresh_computation() {
        let cfg = provider_config();
        let keys = ["BUZZ_RELAY_URL".to_string(), "GOOSE_MODE".to_string()];
        let fp = intent_template(&cfg, keys.clone()).fingerprint();
        let pod = build_pod(&identity(), &cfg, "gen-a", &fp);
        let recorded = Fingerprint::from_annotation(
            &pod.metadata.annotations.as_ref().unwrap()[ANNOTATION_CREATE_INTENT],
        );
        assert_eq!(recorded, intent_template(&cfg, keys).fingerprint());
    }

    /// Two attempts differing only in generation must record the *same*
    /// fingerprint, or the divergence discriminator fires on every deploy and
    /// the never-started row deletes healthy pending pods.
    #[test]
    fn generation_does_not_change_the_recorded_fingerprint() {
        let cfg = provider_config();
        let keys = ["BUZZ_RELAY_URL".to_string()];
        let a = intent_template(&cfg, keys.clone()).fingerprint();
        let b = intent_template(&cfg, keys).fingerprint();
        let id = identity();
        let pod_a = build_pod(&id, &cfg, "gen-1", &a);
        let pod_b = build_pod(&id, &cfg, "gen-2", &b);
        let read =
            |p: &Pod| p.metadata.annotations.as_ref().unwrap()[ANNOTATION_CREATE_INTENT].clone();
        assert_eq!(read(&pod_a), read(&pod_b));
        // ...while the Secret they reference differs.
        let secret_of = |p: &Pod| {
            spec(p).containers[0].env_from.as_ref().unwrap()[0]
                .secret_ref
                .as_ref()
                .unwrap()
                .name
                .clone()
        };
        assert_ne!(secret_of(&pod_a), secret_of(&pod_b));
    }
}
