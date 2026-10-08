//! Optional owner-approved identity policy, checked before cluster access.

use crate::{naming::AgentIdentity, wire::DeployRequest};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityPolicy {
    agent_pubkey: String,
    owner_pubkey: String,
}

/// Validate an optional public identity policy without exposing key material.
/// A local preflight requires the policy; ordinary deploys retain compatibility.
pub fn check(request: &DeployRequest, required: bool) -> Result<AgentIdentity, String> {
    let identity = AgentIdentity::from_nsec(&request.agent.private_key_nsec)?;
    let Some(value) = request.provider_config.get("identity_policy") else {
        return if required {
            Err("identity preflight requires an approved identity_policy".into())
        } else {
            Ok(identity)
        };
    };
    let policy: IdentityPolicy =
        serde_json::from_value(value.clone()).map_err(|_| "invalid identity_policy".to_string())?;
    let approved_agent = nostr::PublicKey::from_hex(&policy.agent_pubkey)
        .map_err(|_| "invalid approved agent public key".to_string())?;
    let approved_owner = nostr::PublicKey::from_hex(&policy.owner_pubkey)
        .map_err(|_| "invalid approved owner public key".to_string())?;
    if identity.pubkey_hex() == approved_owner.to_hex() {
        return Err("owner identity cannot be deployed as the agent".into());
    }
    if identity.pubkey_hex() != approved_agent.to_hex() {
        return Err("agent key does not match the approved pilot identity".into());
    }
    let tag = request
        .agent
        .auth_tag
        .as_deref()
        .ok_or_else(|| "owner attestation is required by identity_policy".to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "could not establish identity verification time".to_string())?
        .as_secs();
    let owner = buzz_sdk::nip_oa::verify_auth_tag_for_auth_event(tag, &approved_agent, now)
        .map_err(|_| "owner attestation verification failed".to_string())?;
    if owner != approved_owner {
        return Err("attestation owner does not match the approved owner".into());
    }
    let descriptor_owner = request
        .agent
        .launch
        .as_ref()
        .and_then(|launch| launch.owner_pubkey.as_deref());
    if descriptor_owner != Some(approved_owner.to_hex().as_str()) {
        return Err("launch owner does not match the verified owner".into());
    }
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::nips::nip19::ToBech32;

    fn request(agent: &nostr::Keys, owner: &nostr::Keys) -> DeployRequest {
        serde_json::from_value(serde_json::json!({
            "agent": {
                "relay_url": "wss://example.invalid",
                "private_key_nsec": agent.secret_key().to_bech32().unwrap(),
                "auth_tag": buzz_sdk::nip_oa::compute_auth_tag(owner, &agent.public_key(), "").unwrap(),
                "launch": {"owner_pubkey": owner.public_key().to_hex()}
            },
            "provider_config": {"identity_policy": {
                "agent_pubkey": agent.public_key().to_hex(),
                "owner_pubkey": owner.public_key().to_hex()
            }}
        })).unwrap()
    }

    #[test]
    fn binds_agent_signature_owner_and_descriptor() {
        let agent = nostr::Keys::generate();
        let owner = nostr::Keys::generate();
        let mut request = request(&agent, &owner);
        assert_eq!(
            check(&request, true).unwrap().pubkey_hex(),
            agent.public_key().to_hex()
        );
        request.agent.launch.as_mut().unwrap().owner_pubkey = Some(agent.public_key().to_hex());
        assert!(check(&request, true).unwrap_err().contains("launch owner"));
    }

    #[test]
    fn rejects_wrong_key_human_key_and_forged_or_wrong_owner_tag() {
        let agent = nostr::Keys::generate();
        let owner = nostr::Keys::generate();
        let stranger = nostr::Keys::generate();
        let mut wrong = request(&agent, &owner);
        wrong.agent.private_key_nsec = stranger.secret_key().to_bech32().unwrap();
        assert!(check(&wrong, true).unwrap_err().contains("approved pilot"));
        wrong.agent.private_key_nsec = owner.secret_key().to_bech32().unwrap();
        assert!(check(&wrong, true).unwrap_err().contains("owner identity"));
        let mut forged = request(&agent, &owner);
        forged.agent.auth_tag = Some("synthetic-secret-not-a-tag".into());
        let error = check(&forged, true).unwrap_err();
        assert!(error.contains("verification failed"));
        assert!(!error.contains("synthetic-secret"));
        forged.agent.auth_tag =
            Some(buzz_sdk::nip_oa::compute_auth_tag(&stranger, &agent.public_key(), "").unwrap());
        assert!(check(&forged, true).unwrap_err().contains("approved owner"));
        forged.agent.auth_tag = Some(
            buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "created_at<1")
                .unwrap(),
        );
        assert!(check(&forged, true)
            .unwrap_err()
            .contains("verification failed"));
    }

    #[test]
    fn requires_policy_only_for_preflight_and_refuses_unknown_policy_fields() {
        let mut request = request(&nostr::Keys::generate(), &nostr::Keys::generate());
        request.provider_config["identity_policy"]["extra"] = serde_json::json!(true);
        assert!(check(&request, true).is_err());
        request.provider_config = serde_json::json!({});
        assert!(check(&request, true).is_err());
        assert!(check(&request, false).is_ok());
    }
}
