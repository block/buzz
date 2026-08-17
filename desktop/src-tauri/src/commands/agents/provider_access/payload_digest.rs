//! A stable digest over everything a provider deploy sends.
//!
//! A deploy's inputs are wider than the agent row it is keyed on: the payload
//! is re-resolved from the live persona, the team, the global agent config,
//! the effective harness descriptor, the workspace relay URL and the owner
//! pubkey (see [`crate::commands::agents::build_deploy_payload`]), and the
//! provider config travels beside it. Any of those can move without touching
//! `record.updated_at`, so a completion bound to `updated_at` alone can stamp
//! a stale deploy as current. Binding to a digest of the *fully resolved*
//! inputs closes that: re-resolve on completion and compare.
//!
//! The digest covers the provider id, the provider config, and the agent
//! payload — the whole of what [`crate::managed_agents::provider_deploy`]
//! puts on the wire.
//!
//! Canonicalization: object keys are sorted recursively before serialization,
//! so the digest does not depend on `serde_json`'s map implementation (the
//! `preserve_order` feature swaps `BTreeMap` for insertion order) nor on the
//! order the payload builder happens to insert keys in. Arrays keep their
//! order — it is meaningful (`agent_args`, allowlists).
//!
//! The payload carries secrets (the agent nsec, env vars). The digest is a
//! SHA-256 over a document containing a 256-bit private key, so it is not a
//! guessing oracle, but it is still never logged or published — it lives in
//! memory for the length of a deploy and, on the ambiguous-success path only,
//! in a `0o600` receipt beside the store that holds those secrets in the
//! clear anyway.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::managed_agents::{BackendKind, ManagedAgentRecord};

/// Digest the complete deploy input for `record` with its resolved `payload`.
///
/// Errors when the record is not provider-backed: there is no provider call to
/// digest, and silently digesting a local record would let a completion
/// compare two things that were never sent.
pub(in crate::commands::agents) fn deploy_input_digest(
    record: &ManagedAgentRecord,
    payload: &Value,
) -> Result<String, String> {
    let BackendKind::Provider { id, config } = &record.backend else {
        return Err(format!(
            "agent {} is not provider-backed, so it has no deploy input to digest",
            record.pubkey
        ));
    };
    digest_parts(id, config, payload)
}

/// The digest of one deploy's three wire inputs. Split out so the snapshot can
/// digest exactly the provider id and config it captured, rather than
/// re-reading them from a record that may have moved on.
pub(in crate::commands::agents) fn digest_parts(
    provider_id: &str,
    provider_config: &Value,
    payload: &Value,
) -> Result<String, String> {
    let document = serde_json::json!({
        "provider": provider_id,
        "provider_config": provider_config,
        "agent": payload,
    });
    let bytes = serde_json::to_vec(&canonical(&document))
        .map_err(|error| format!("failed to serialize the deploy payload for hashing: {error}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Rebuild `value` with every object's keys in sorted order.
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<(&String, &Value)> = map.iter().collect();
            sorted.sort_unstable_by(|left, right| left.0.cmp(right.0));
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, entry)| (key.clone(), canonical(entry)))
                    .collect::<Map<String, Value>>(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(model: &str) -> Value {
        serde_json::json!({
            "name": "agent",
            "model": model,
            "env_vars": {"B": "2", "A": "1"},
            "launch": {"args": ["acp", "--x"], "policy_env": {"Z": "z", "Y": "y"}},
        })
    }

    #[test]
    fn key_order_does_not_change_the_digest() {
        // The same document with every object's keys inserted in the opposite
        // order. Under `preserve_order` these serialize differently; the
        // digest must not notice.
        let reordered = serde_json::json!({
            "launch": {"policy_env": {"Y": "y", "Z": "z"}, "args": ["acp", "--x"]},
            "env_vars": {"A": "1", "B": "2"},
            "model": "m",
            "name": "agent",
        });

        assert_eq!(
            digest_parts(
                "provider",
                &serde_json::json!({"region": "a"}),
                &payload("m")
            )
            .unwrap(),
            digest_parts("provider", &serde_json::json!({"region": "a"}), &reordered).unwrap(),
        );
    }

    #[test]
    fn array_order_does_change_the_digest() {
        let swapped = serde_json::json!({
            "name": "agent",
            "model": "m",
            "env_vars": {"B": "2", "A": "1"},
            "launch": {"args": ["--x", "acp"], "policy_env": {"Z": "z", "Y": "y"}},
        });

        assert_ne!(
            digest_parts("provider", &Value::Null, &payload("m")).unwrap(),
            digest_parts("provider", &Value::Null, &swapped).unwrap(),
            "argv order is meaningful and must not be canonicalized away"
        );
    }

    /// Each of the three wire inputs moves the digest on its own — a digest
    /// blind to the provider config would let a namespace or image change be
    /// stamped as applied.
    #[test]
    fn every_wire_input_moves_the_digest() {
        let base = digest_parts(
            "provider",
            &serde_json::json!({"region": "a"}),
            &payload("m"),
        )
        .expect("digest");

        for (provider, config, resolved) in [
            ("other", serde_json::json!({"region": "a"}), payload("m")),
            ("provider", serde_json::json!({"region": "b"}), payload("m")),
            (
                "provider",
                serde_json::json!({"region": "a"}),
                payload("m2"),
            ),
        ] {
            assert_ne!(
                base,
                digest_parts(provider, &config, &resolved).expect("digest"),
                "a changed wire input left the digest unmoved"
            );
        }
    }

    #[test]
    fn digest_is_stable_across_calls() {
        let once = digest_parts("provider", &serde_json::json!({}), &payload("m")).unwrap();
        let twice = digest_parts("provider", &serde_json::json!({}), &payload("m")).unwrap();
        assert_eq!(once, twice);
        assert_eq!(once.len(), 64, "expected a hex sha256");
    }

    #[test]
    fn a_local_record_has_no_deploy_input() {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "agent", "name": "Agent", "relay_url": "", "acp_command": "",
            "agent_command": "", "agent_args": [], "mcp_command": "",
            "turn_timeout_seconds": 0, "created_at": "", "updated_at": ""
        }))
        .expect("record");
        record.backend = BackendKind::Local;

        assert!(deploy_input_digest(&record, &payload("m")).is_err());
    }
}
