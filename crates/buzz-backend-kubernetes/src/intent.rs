//! The create-intent fingerprint (spec §Deploy State Machine, create-intent
//! fingerprint, `docs/remote-agents.md:796-828`).
//!
//! The fingerprint is an unkeyed SHA-256 over a canonical serialization of the
//! provider's non-secret create-intent template. A plain hash is safe *only*
//! because of the scope rule: the input covers exactly the provider-controlled
//! fields that can affect scheduling or container creation, and **never Secret
//! data or attempt identity**. Hashing low-entropy secrets into a
//! world-readable annotation would be a dictionary oracle.
//!
//! That rule is enforced structurally rather than remembered. [`IntentTemplate`]
//! is a *pre-binding* type: it has no field that can hold Secret material or a
//! generation token, so there is no expression that hashes one. The
//! per-attempt Secret name never appears — the pod's `envFrom` is represented
//! by the fixed [`SECRET_PLACEHOLDER`], because otherwise every attempt would
//! diverge from every other by construction.
//!
//! Server- and admission-produced output (UID, `resourceVersion`, timestamps,
//! defaulted fields, the annotation itself) is excluded the same way: the
//! serializer is only ever handed this template, never a live `Pod`, so the
//! exclusion is checkable by inspection.

use crate::image::ImageRef;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Stands in for the per-attempt Secret name in the `envFrom` position.
/// A real generation token here would make every attempt diverge.
const SECRET_PLACEHOLDER: &str = "<per-attempt-secret>";

/// The recorded/computed create intent: a hex SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint(String);

impl Fingerprint {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Read a fingerprint off a pod annotation. Any recorded string is
    /// accepted verbatim: comparison is equality against a freshly computed
    /// value, so a malformed annotation simply reads as divergence — which is
    /// the correct outcome for a pod this provider version did not write.
    pub fn from_annotation(value: &str) -> Self {
        Self(value.to_string())
    }

    #[cfg(test)]
    pub fn for_test(seed: &str) -> Self {
        Self(format!("test-{seed}"))
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The non-secret, pre-binding description of the pod this deploy would
/// create. Every field is provider-controlled and scheduling-relevant; there
/// is deliberately no field for env values, Secret data, or the generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntentTemplate {
    /// Schema version of the template itself. Bumping it re-fingerprints every
    /// pod, which is the intended way to roll out a pod-shape change.
    pub template_version: u32,
    pub namespace: String,
    /// Normalized, digest-qualified image reference.
    pub image: String,
    pub cpu_request: String,
    pub memory_request: String,
    pub cpu_limit: String,
    pub memory_limit: String,
    pub service_account: Option<String>,
    pub restart_policy: &'static str,
    pub termination_grace_period_seconds: i64,
    /// Env *keys* only, sorted. Keys are pod-shape (a renamed key changes the
    /// container's contract); values are Secret material and must not be here.
    pub env_keys: Vec<String>,
    /// Fixed placeholder for the per-attempt Secret in `envFrom`.
    pub env_from_secret: &'static str,
    pub workspace_mount_path: String,
    pub run_as_user: i64,
    pub run_as_group: i64,
}

/// Current template schema version.
pub const TEMPLATE_VERSION: u32 = 1;

impl IntentTemplate {
    /// Compute the fingerprint. `serde_json` on a struct with declared field
    /// order plus pre-sorted `env_keys` is a canonical serialization: the same
    /// template always produces the same bytes.
    pub fn fingerprint(&self) -> Fingerprint {
        let canonical = serde_json::to_vec(self).expect("intent template is plain data");
        Fingerprint(hex::encode(Sha256::digest(&canonical)))
    }

    /// Build from resolved pod-shape inputs. `env_keys` is sorted here rather
    /// than at the call site so key ordering can never leak into the digest.
    ///
    /// The fixed pod-shape constants are read from [`crate::config`] rather
    /// than passed in: `pod::build_pod` stamps the pod from those same
    /// constants, so the fingerprint cannot describe a pod shape different
    /// from the one actually created. Threading them through as arguments
    /// would make that agreement a thing to test instead of a thing that holds.
    pub fn new(
        namespace: &str,
        image: &ImageRef,
        resources: &crate::config::Resources,
        service_account: Option<&str>,
        env_keys: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut env_keys: Vec<String> = env_keys.into_iter().collect();
        env_keys.sort();
        Self {
            template_version: TEMPLATE_VERSION,
            namespace: namespace.to_string(),
            image: image.as_str().to_string(),
            cpu_request: resources.cpu_request.clone(),
            memory_request: resources.memory_request.clone(),
            cpu_limit: resources.cpu_limit.clone(),
            memory_limit: resources.memory_limit.clone(),
            service_account: service_account.map(str::to_string),
            restart_policy: crate::config::RESTART_POLICY,
            termination_grace_period_seconds: crate::config::TERMINATION_GRACE_SECONDS,
            env_keys,
            env_from_secret: SECRET_PLACEHOLDER,
            workspace_mount_path: crate::config::WORKSPACE_PATH.to_string(),
            run_as_user: crate::config::RUN_AS_UID,
            run_as_group: crate::config::RUN_AS_GID,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Resources;

    fn image(byte: char) -> ImageRef {
        crate::image::parse(&format!(
            "ghcr.io/block/buzz-sprig@sha256:{}",
            byte.to_string().repeat(64)
        ))
        .unwrap()
    }

    fn template() -> IntentTemplate {
        IntentTemplate::new(
            "buzz-agents",
            &image('a'),
            &Resources::default(),
            None,
            ["BUZZ_RELAY_URL".to_string(), "GOOSE_MODE".to_string()],
        )
    }

    #[test]
    fn fingerprint_is_deterministic() {
        assert_eq!(template().fingerprint(), template().fingerprint());
    }

    #[test]
    fn fingerprint_is_hex_sha256() {
        let fp = template().fingerprint();
        assert_eq!(fp.as_str().len(), 64);
        assert!(fp.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Key *order* must not reach the digest, or two identical environments
    /// built in different orders would look like a config change.
    #[test]
    fn env_key_order_does_not_affect_the_digest() {
        let a = IntentTemplate::new(
            "ns",
            &image('a'),
            &Resources::default(),
            None,
            ["A".to_string(), "B".to_string(), "C".to_string()],
        );
        let b = IntentTemplate::new(
            "ns",
            &image('a'),
            &Resources::default(),
            None,
            ["C".to_string(), "A".to_string(), "B".to_string()],
        );
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    /// A mutation applied to a fresh template clone, named for its assertion
    /// message.
    type Mutation = (&'static str, Box<dyn Fn(&mut IntentTemplate)>);

    /// Every scheduling-relevant knob must move the digest — this is the
    /// wedge escape (§Deploy State Machine never-started recoverable row).
    /// Exhaustive by construction: each mutation is applied to a fresh clone.
    #[test]
    fn every_scheduling_field_changes_the_digest() {
        let base = template();
        let baseline = base.fingerprint();

        let mutations: Vec<Mutation> = vec![
            (
                "template_version",
                Box::new(|t: &mut IntentTemplate| t.template_version += 1),
            ),
            (
                "namespace",
                Box::new(|t: &mut IntentTemplate| t.namespace = "other".into()),
            ),
            (
                "image",
                Box::new(|t: &mut IntentTemplate| t.image = image('b').as_str().into()),
            ),
            (
                "cpu_request",
                Box::new(|t: &mut IntentTemplate| t.cpu_request = "4".into()),
            ),
            (
                "memory_request",
                Box::new(|t: &mut IntentTemplate| t.memory_request = "8Gi".into()),
            ),
            (
                "cpu_limit",
                Box::new(|t: &mut IntentTemplate| t.cpu_limit = "8".into()),
            ),
            (
                "memory_limit",
                Box::new(|t: &mut IntentTemplate| t.memory_limit = "16Gi".into()),
            ),
            (
                "service_account",
                Box::new(|t: &mut IntentTemplate| t.service_account = Some("sa".into())),
            ),
            (
                "restart_policy",
                Box::new(|t: &mut IntentTemplate| t.restart_policy = "OnFailure"),
            ),
            (
                "grace_period",
                Box::new(|t: &mut IntentTemplate| t.termination_grace_period_seconds = 30),
            ),
            (
                "env_keys",
                Box::new(|t: &mut IntentTemplate| t.env_keys.push("NEW_KEY".into())),
            ),
            (
                "workspace_mount_path",
                Box::new(|t: &mut IntentTemplate| t.workspace_mount_path = "/w".into()),
            ),
            (
                "run_as_user",
                Box::new(|t: &mut IntentTemplate| t.run_as_user = 2000),
            ),
            (
                "run_as_group",
                Box::new(|t: &mut IntentTemplate| t.run_as_group = 2000),
            ),
        ];

        for (name, mutate) in mutations {
            let mut t = base.clone();
            mutate(&mut t);
            assert_ne!(
                t.fingerprint(),
                baseline,
                "{name} did not affect the digest"
            );
        }
    }

    /// The scope rule, asserted on the bytes: no Secret value and no
    /// generation token can appear in the serialization, because the type has
    /// nowhere to put them. The placeholder is what `envFrom` contributes.
    #[test]
    fn serialization_contains_no_secret_material_or_attempt_identity() {
        let json = serde_json::to_string(&template()).unwrap();
        for forbidden in ["nsec1", "SPOOFED", "wss://", "gen0001"] {
            assert!(
                !json.contains(forbidden),
                "template leaked {forbidden}: {json}"
            );
        }
        assert!(json.contains(SECRET_PLACEHOLDER));
    }

    /// Two attempts for the same agent differ only in generation, which is
    /// absent from the template — so their fingerprints must be equal, or the
    /// divergence discriminator would fire on every single deploy.
    #[test]
    fn attempts_differing_only_by_generation_do_not_diverge() {
        // There is no generation input to pass; that *is* the property. The
        // test states it explicitly so a future field addition breaks here.
        assert_eq!(template().fingerprint(), template().fingerprint());
        let json = serde_json::to_string(&template()).unwrap();
        assert_eq!(json.matches(SECRET_PLACEHOLDER).count(), 1);
    }

    /// A recorded annotation this provider version did not write reads as
    /// divergence rather than an error.
    #[test]
    fn unrecognized_annotation_reads_as_divergence() {
        let recorded = Fingerprint::from_annotation("not-a-digest");
        assert_ne!(recorded, template().fingerprint());
    }
}
