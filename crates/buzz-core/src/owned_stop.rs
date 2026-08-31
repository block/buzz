//! Local supported-runtime completion evidence, not a relay event or a second
//! run identity. The host still signs the public execution Receipt. This proof
//! authenticates the retained harness's explicit child-work result to that host.
use nostr::{
    hashes::{sha256::Hash as Sha256Hash, Hash},
    secp256k1::{schnorr::Signature, Message},
    Keys, PublicKey, SECP256K1,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Domain-separated, agent-signed assertion for one existing launcher generation.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    /// Canonical community URL.
    pub relay: String,
    /// Existing launcher nonce (never a new supervisor-generated run ID).
    pub run: String,
    /// BIP-340 signature; the expected agent key is supplied by the host.
    pub signature: String,
}
fn message(agent: &PublicKey, relay: &str, run: &str) -> Result<Message, String> {
    if !crate::host_execution::hex_id(run, 32)
        || crate::relay::normalize_relay_url(relay).ok().as_deref() != Some(relay)
    {
        return Err("invalid owned-work proof scope".into());
    }
    // JSON array is unambiguous; fixed domain commits to v1's supported boundary.
    let preimage = serde_json::to_vec(&["buzz.owned-work.stopped.v1", &agent.to_hex(), relay, run])
        .map_err(|_| "cannot encode owned-work proof")?;
    Ok(Message::from_digest(
        Sha256Hash::hash(&preimage).to_byte_array(),
    ))
}
/// Sign only after every owned child supplied supported completion evidence and
/// was reaped successfully. Never infer this assertion from root exit alone.
pub fn sign(keys: &Keys, relay: &str, run: &str) -> Result<Proof, String> {
    let message = message(&keys.public_key(), relay, run)?;
    Ok(Proof {
        relay: relay.into(),
        run: run.into(),
        signature: keys.sign_schnorr(&message).to_string(),
    })
}
/// Validate a bounded local proof against the selected placement, identity and
/// generation. The caller must separately reap its retained root before using it.
pub fn verify(proof: &Proof, agent: &str, relay: &str, run: &str) -> Result<(), String> {
    if proof.relay != relay || proof.run != run {
        return Err("owned-work proof scope mismatch".into());
    }
    let key = PublicKey::from_hex(agent).map_err(|_| "invalid agent key")?;
    let sig = Signature::from_str(&proof.signature).map_err(|_| "invalid proof signature")?;
    SECP256K1
        .verify_schnorr(
            &sig,
            &message(&key, relay, run)?,
            &key.xonly().map_err(|_| "invalid agent key")?,
        )
        .map_err(|_| "invalid owned-work proof".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proof_binds_identity_community_generation_and_result_domain() {
        let keys = Keys::generate();
        let relay = "wss://example.com";
        let run = "aa".repeat(16);
        let proof = sign(&keys, relay, &run).unwrap();
        assert!(verify(&proof, &keys.public_key().to_hex(), relay, &run).is_ok());
        assert!(verify(&proof, &Keys::generate().public_key().to_hex(), relay, &run).is_err());
        assert!(verify(
            &proof,
            &keys.public_key().to_hex(),
            "wss://peer.example.com",
            &run
        )
        .is_err());
        assert!(verify(&proof, &keys.public_key().to_hex(), relay, &"bb".repeat(16)).is_err());
    }
}
