use buzz_core_pkg::nostr_identity::{parse_public_key_compat, public_key_to_npub};

/// Serialize the app-defined setup payload with a canonical npub identity.
/// Records remain protocol hex internally; legacy npub records are accepted defensively.
pub(super) fn serialize_setup_payload(
    agent_name: &str,
    agent_pubkey: &str,
    requirements: Vec<serde_json::Value>,
) -> Result<String, String> {
    let (public_key, _) = parse_public_key_compat(agent_pubkey)
        .map_err(|_| "agent record has an invalid public key".to_string())?;
    let agent_npub =
        public_key_to_npub(&public_key).map_err(|_| "failed to encode agent npub".to_string())?;
    serde_json::to_string(&serde_json::json!({
        "agent_name": agent_name,
        "agent_pubkey": agent_npub,
        "requirements": requirements,
    }))
    .map_err(|error| format!("failed to serialize setup payload: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT_HEX: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    #[test]
    fn serialization_emits_npub_without_record_hex() {
        let json = serialize_setup_payload(
            "Fizz",
            AGENT_HEX,
            vec![serde_json::json!({"surface": "env_key", "key": "TOKEN"})],
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["agent_pubkey"].as_str().unwrap().starts_with("npub1"));
        assert!(!json.contains(AGENT_HEX));
    }

    #[test]
    fn serialization_rejects_invalid_record_key_without_echoing_it() {
        let error = serialize_setup_payload("Fizz", "not-a-public-key", vec![]).unwrap_err();
        assert!(error.contains("invalid public key"));
        assert!(!error.contains("not-a-public-key"));
    }
}
