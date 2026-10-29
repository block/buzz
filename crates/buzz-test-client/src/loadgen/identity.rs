//! Deterministic identities and fixture ids derived from the run id.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use nostr::secp256k1::Message;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, SecretKey, Tag};
use sha2::{Digest, Sha256};

/// Role of a derived identity; part of the derivation so roles never collide.
#[derive(Clone, Copy, Debug)]
pub enum Role {
    Owner,
    Human,
    Agent,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Human => "human",
            Role::Agent => "agent",
        }
    }
}

fn digest(parts: &[&str]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    hasher.finalize().into()
}

/// Keys for identity `index` of `role` in run `run_id`.
pub fn keys(run_id: &str, role: Role, index: usize) -> Keys {
    let mut counter = 0u32;
    loop {
        let seed = digest(&[
            "buzz-loadgen",
            run_id,
            role.label(),
            &index.to_string(),
            &counter.to_string(),
        ]);
        // A digest outside the curve order is astronomically unlikely; retry anyway.
        if let Ok(secret) = SecretKey::from_slice(&seed) {
            return Keys::new(secret);
        }
        counter += 1;
    }
}

/// Channel UUID for channel `index` of community `community`.
pub fn channel_id(run_id: &str, community: usize, index: usize) -> String {
    let seed = digest(&[
        "buzz-loadgen-channel",
        run_id,
        &community.to_string(),
        &index.to_string(),
    ]);
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&seed[..16]);
    uuid::Builder::from_random_bytes(bytes)
        .into_uuid()
        .to_string()
}

/// NIP-OA `auth` tag in which `owner` authorizes `agent`.
pub fn nip_oa_tag(owner: &Keys, agent: &Keys) -> Tag {
    let preimage = format!("nostr:agent-auth:{}:", agent.public_key().to_hex());
    let digest: [u8; 32] = Sha256::digest(preimage.as_bytes()).into();
    let signature = owner.sign_schnorr(&Message::from_digest(digest));
    Tag::parse([
        "auth".to_string(),
        owner.public_key().to_hex(),
        String::new(),
        signature.to_string(),
    ])
    .expect("auth tag is well-formed")
}

/// NIP-98 `Authorization` header value for one request.
pub fn nip98_header(keys: &Keys, method: &str, url: &str, body: &[u8]) -> String {
    let tags = [
        Tag::parse(["u", url]).expect("u tag"),
        Tag::parse(["method", method]).expect("method tag"),
        Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).expect("nonce tag"),
        Tag::parse(["payload", &hex::encode(Sha256::digest(body))]).expect("payload tag"),
    ];
    let event = EventBuilder::new(Kind::Custom(27235), "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign NIP-98 event");
    format!("Nostr {}", B64.encode(event.as_json()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_is_deterministic_and_role_scoped() {
        let a = keys("run", Role::Human, 7);
        assert_eq!(a.public_key(), keys("run", Role::Human, 7).public_key());
        assert_ne!(a.public_key(), keys("run", Role::Agent, 7).public_key());
        assert_ne!(a.public_key(), keys("other", Role::Human, 7).public_key());
        assert_eq!(channel_id("run", 0, 3), channel_id("run", 0, 3));
        assert_ne!(channel_id("run", 0, 3), channel_id("run", 1, 3));
        assert!(uuid::Uuid::parse_str(&channel_id("run", 0, 0)).is_ok());
    }
}
