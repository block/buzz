//! Identity derivation and the object-naming contract (spec §Pod shape).
//!
//! Every name, label, and annotation below is derived from the pubkey the
//! provider decoded itself from `private_key_nsec` — never from a
//! caller-supplied pubkey (§Deploy State Machine step 0).

use nostr::nips::nip19::FromBech32;

/// `app.kubernetes.io/managed-by` value: the management marker's identity half.
pub const MANAGED_BY: &str = "buzz-backend-kubernetes";

/// Label key carrying [`MANAGED_BY`].
pub const LABEL_MANAGED_BY: &str = "app.kubernetes.io/managed-by";

/// Label key carrying [`BINDING_VERSION`] — the marker's schema half.
pub const LABEL_BINDING_VERSION: &str = "buzz.block.xyz/binding-version";

/// Schema version of the object layout this provider writes. Bumped when the
/// pod/Secret shape changes in a way a older provider would mis-handle.
pub const BINDING_VERSION: &str = "1";

/// Label key: truncated pubkey, the reconciliation and GC selector.
pub const LABEL_AGENT_PUBKEY: &str = "buzz.block.xyz/agent-pubkey";

/// Annotation key: full pubkey. Load-bearing — the truncated label is
/// collision-*resistant*, this is what makes it safe (§Deploy State Machine
/// step 1).
pub const ANNOTATION_PUBKEY_FULL: &str = "buzz.block.xyz/agent-pubkey-full";

/// Annotation key: the recorded create-intent fingerprint.
pub const ANNOTATION_CREATE_INTENT: &str = "buzz.block.xyz/create-intent";

/// Annotation key: the image reference this generation actually resolved to,
/// for post-hoc attribution (§Image).
pub const ANNOTATION_IMAGE: &str = "buzz.block.xyz/image";

/// An agent identity the provider derived itself, plus every name it implies.
///
/// Constructing this type is the *only* way to obtain the names — so a
/// caller-supplied pubkey cannot reach a selector by any path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentity {
    pubkey_hex: String,
}

impl AgentIdentity {
    /// Derive from the payload's `private_key_nsec`.
    ///
    /// Accepts bech32 `nsec1…`; a malformed or undecodable key is an
    /// immediate error, before any substrate read or mutation
    /// (§Deploy State Machine step 0).
    pub fn from_nsec(nsec: &str) -> Result<Self, String> {
        let secret = nostr::SecretKey::from_bech32(nsec.trim())
            .map_err(|_| "private_key_nsec is not a decodable nsec1 key".to_string())?;
        let keys = nostr::Keys::new(secret);
        Ok(Self {
            pubkey_hex: keys.public_key().to_hex(),
        })
    }

    /// Full 64-hex public key — the annotation value and the comparison
    /// operand for candidate authentication.
    pub fn pubkey_hex(&self) -> &str {
        &self.pubkey_hex
    }

    /// Selector label value: first 32 hex chars (128 bits). A full hex pubkey
    /// is 64 chars and label values cap at 63, which is why this is truncated
    /// and why the annotation check is normative rather than decorative.
    pub fn label_pubkey(&self) -> &str {
        &self.pubkey_hex[..32]
    }

    /// Deterministic pod name, also the returned `agent_id`.
    pub fn pod_name(&self) -> String {
        format!("buzz-agent-{}", &self.pubkey_hex[..12])
    }

    /// Per-attempt Secret name. `generation` is a fresh random token per
    /// create attempt — never reused — which is what makes payload and Secret
    /// atomic at the pod-spec boundary (§K8s Secrets).
    pub fn secret_name(&self, generation: &str) -> String {
        format!("buzz-agent-{}-{}", &self.pubkey_hex[..12], generation)
    }

    /// Label selector matching this identity's objects *and* our management
    /// marker. Selecting on the marker as well as the identity means an
    /// unmarked look-alike never even enters the candidate list.
    pub fn selector(&self) -> String {
        format!(
            "{LABEL_AGENT_PUBKEY}={},{LABEL_MANAGED_BY}={MANAGED_BY}",
            self.label_pubkey()
        )
    }

    /// The label set stamped on every object this provider creates.
    pub fn labels(&self) -> std::collections::BTreeMap<String, String> {
        [
            (
                LABEL_AGENT_PUBKEY.to_string(),
                self.label_pubkey().to_string(),
            ),
            (LABEL_MANAGED_BY.to_string(), MANAGED_BY.to_string()),
            (
                LABEL_BINDING_VERSION.to_string(),
                BINDING_VERSION.to_string(),
            ),
        ]
        .into_iter()
        .collect()
    }
}

/// A fresh generation token: 8 lowercase hex chars from the OS RNG.
///
/// Appears in the Secret name and as `BUZZ_MANAGED_AGENT_START_NONCE`, so the
/// Secret generation and the harness's lifecycle-frame correlator are one
/// identity (§Launch data tier 3).
pub fn new_generation() -> String {
    use rand::RngExt;
    let n: u32 = rand::rng().random();
    format!("{n:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed test key. Deriving the pubkey (rather than hardcoding both
    /// halves) is the point: the test exercises the same derivation the
    /// reconciler depends on.
    fn identity() -> AgentIdentity {
        let keys = nostr::Keys::generate();
        let nsec = {
            use nostr::nips::nip19::ToBech32;
            keys.secret_key().to_bech32().unwrap()
        };
        let id = AgentIdentity::from_nsec(&nsec).unwrap();
        assert_eq!(id.pubkey_hex(), keys.public_key().to_hex());
        id
    }

    #[test]
    fn rejects_malformed_nsec() {
        for bad in ["", "nsec1", "not-a-key", "npub1abc"] {
            assert!(
                AgentIdentity::from_nsec(bad).is_err(),
                "accepted malformed key {bad:?}"
            );
        }
    }

    #[test]
    fn tolerates_surrounding_whitespace() {
        let keys = nostr::Keys::generate();
        use nostr::nips::nip19::ToBech32;
        let nsec = keys.secret_key().to_bech32().unwrap();
        let padded = format!("  {nsec}\n");
        assert_eq!(
            AgentIdentity::from_nsec(&padded).unwrap().pubkey_hex(),
            keys.public_key().to_hex()
        );
    }

    /// Kubernetes label *values* cap at 63 chars; a full hex pubkey is 64,
    /// one over. That one-char overflow is the whole reason the selector is
    /// truncated, so it gets an explicit test.
    #[test]
    fn label_value_fits_kubernetes_limit() {
        let id = identity();
        assert_eq!(id.pubkey_hex().len(), 64);
        assert_eq!(id.label_pubkey().len(), 32);
        assert!(id.label_pubkey().len() <= 63);
    }

    #[test]
    fn pod_name_is_deterministic_and_dns_safe() {
        let id = identity();
        assert_eq!(id.pod_name(), id.pod_name());
        assert_eq!(
            id.pod_name(),
            format!("buzz-agent-{}", &id.pubkey_hex()[..12])
        );
        assert!(id.pod_name().len() <= 253);
        assert!(id
            .pod_name()
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    }

    /// Two attempts must never share a Secret name — that uniqueness is what
    /// stops a losing contender from overwriting the winner's identity.
    #[test]
    fn secret_names_are_per_attempt() {
        let id = identity();
        let a = id.secret_name(&new_generation());
        let b = id.secret_name(&new_generation());
        assert_ne!(a, b);
        assert!(a.starts_with(&id.pod_name()));
        assert!(a.len() <= 253);
    }

    #[test]
    fn selector_requires_the_management_marker() {
        let id = identity();
        let sel = id.selector();
        assert!(sel.contains(&format!("{LABEL_AGENT_PUBKEY}={}", id.label_pubkey())));
        assert!(sel.contains(&format!("{LABEL_MANAGED_BY}={MANAGED_BY}")));
    }

    #[test]
    fn every_created_object_carries_the_marker() {
        let labels = identity().labels();
        assert_eq!(
            labels.get(LABEL_MANAGED_BY).map(String::as_str),
            Some(MANAGED_BY)
        );
        assert_eq!(
            labels.get(LABEL_BINDING_VERSION).map(String::as_str),
            Some(BINDING_VERSION)
        );
    }
}
