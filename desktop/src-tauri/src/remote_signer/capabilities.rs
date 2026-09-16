//! Purpose-specific adapters on the event signer's captured transport and lifetime.
use super::{RemoteSigner, RemoteSignerError};
use base64::{engine::general_purpose::STANDARD, Engine};
use nostr::PublicKey;
use serde::{Deserialize, Serialize};
use url::Url;

const MAX_PLAINTEXT: usize = 65_535;
const MAX_ENVELOPE: usize = 65_603;
const MAX_BASE64: usize = 87_472;

fn plaintext_len(text: &str) -> Result<usize, RemoteSignerError> {
    if !(1..=MAX_PLAINTEXT).contains(&text.len()) {
        return Err(RemoteSignerError::InvalidInput);
    }
    Ok(text.len())
}

// NIP-44 v2 public padding/length rule, not a cipher implementation. The largest
// valid plaintext pads to 65536; do not inherit rust-nostr's local encrypt limit.
fn padded_len(len: usize) -> usize {
    if len <= 32 {
        return 32;
    }
    let next = len.next_power_of_two();
    let chunk = if next <= 256 { 32 } else { next / 8 };
    chunk * len.div_ceil(chunk)
}

fn envelope_len(text: &str) -> Result<usize, RemoteSignerError> {
    if !(132..=MAX_BASE64).contains(&text.len()) {
        return Err(RemoteSignerError::InvalidInput);
    }
    let bytes = STANDARD
        .decode(text)
        .map_err(|_| RemoteSignerError::InvalidInput)?;
    if !(99..=MAX_ENVELOPE).contains(&bytes.len())
        || bytes[0] != 2
        || STANDARD.encode(&bytes) != text
    {
        return Err(RemoteSignerError::InvalidInput);
    }
    let padded = bytes.len() - 67; // version + nonce + length prefix + MAC
    if padded_len(padded) != padded {
        return Err(RemoteSignerError::InvalidInput);
    }
    Ok(padded)
}

#[derive(Serialize)]
struct EncryptRequest<'a> {
    peer_pubkey: String,
    plaintext: &'a str,
}
#[derive(Serialize)]
struct DecryptRequest<'a> {
    peer_pubkey: String,
    ciphertext: &'a str,
}
#[derive(Serialize)]
struct AuthorizeRequest<'a> {
    agent_pubkey: String,
    conditions: &'a str,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EncryptResponse {
    ciphertext: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecryptResponse {
    plaintext: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthorizeResponse {
    auth_tag: [String; 4],
}

impl RemoteSigner {
    fn capability_endpoint(&self, name: &str) -> Url {
        let mut endpoint = self.endpoint.clone();
        // endpoint is constructed internally with the fixed /sign suffix.
        endpoint.set_path(&format!(
            "{}/{name}",
            self.endpoint.path().trim_end_matches("/sign")
        ));
        endpoint
    }

    pub(super) async fn encrypt(
        &self,
        peer: &PublicKey,
        plaintext: &str,
    ) -> Result<String, RemoteSignerError> {
        self.run(async {
            let len = plaintext_len(plaintext)?;
            let body = serde_json::to_vec(&EncryptRequest {
                peer_pubkey: peer.to_hex(),
                plaintext,
            })
            .map_err(|_| RemoteSignerError::InvalidInput)?;
            let bytes = self
                .post(
                    self.capability_endpoint("encrypt"),
                    body,
                    MAX_BASE64 * 6 + 1024,
                )
                .await?;
            let response: EncryptResponse =
                serde_json::from_slice(&bytes).map_err(|_| RemoteSignerError::MalformedResponse)?;
            if envelope_len(&response.ciphertext)
                .map_err(|_| RemoteSignerError::MalformedResponse)?
                != padded_len(len)
            {
                return Err(RemoteSignerError::CapabilityMismatch);
            }
            // Structural validation only: without the private key the client cannot
            // authenticate this ciphertext to the peer or prove the server's plaintext.
            Ok(response.ciphertext)
        })
        .await
    }

    pub(super) async fn decrypt(
        &self,
        peer: &PublicKey,
        ciphertext: &str,
    ) -> Result<String, RemoteSignerError> {
        self.run(async {
            let padded = envelope_len(ciphertext)?;
            let body = serde_json::to_vec(&DecryptRequest {
                peer_pubkey: peer.to_hex(),
                ciphertext,
            })
            .map_err(|_| RemoteSignerError::InvalidInput)?;
            let bytes = self
                .post(
                    self.capability_endpoint("decrypt"),
                    body,
                    MAX_PLAINTEXT * 6 + 1024,
                )
                .await?;
            // serde rejects invalid UTF-8 and unpaired surrogates, but preserves a
            // legitimate U+FFFD and all whitespace/normalization forms exactly.
            let response: DecryptResponse =
                serde_json::from_slice(&bytes).map_err(|_| RemoteSignerError::MalformedResponse)?;
            let len = plaintext_len(&response.plaintext)
                .map_err(|_| RemoteSignerError::MalformedResponse)?;
            if padded_len(len) != padded {
                return Err(RemoteSignerError::CapabilityMismatch);
            }
            Ok(response.plaintext)
        })
        .await
    }

    /// Obtain one owner attestation for this agent and the exact conditions.
    /// The SDK checks canonical grammar and the BIP340 signature, not whether
    /// any particular event satisfies those conditions. No raw digest signing.
    pub async fn authorize_agent(
        &self,
        agent: &PublicKey,
        conditions: &str,
    ) -> Result<[String; 4], RemoteSignerError> {
        self.run(async {
            let body = serde_json::to_vec(&AuthorizeRequest {
                agent_pubkey: agent.to_hex(),
                conditions,
            })
            .map_err(|_| RemoteSignerError::InvalidInput)?;
            let limit = body.len().saturating_mul(6).saturating_add(4096);
            let bytes = self
                .post(self.capability_endpoint("authorize-agent"), body, limit)
                .await?;
            let response: AuthorizeResponse =
                serde_json::from_slice(&bytes).map_err(|_| RemoteSignerError::MalformedResponse)?;
            if response.auth_tag[1] != self.session.public_key.to_hex()
                || response.auth_tag[2] != conditions
            {
                return Err(RemoteSignerError::CapabilityMismatch);
            }
            let serialized = serde_json::to_string(&response.auth_tag)
                .map_err(|_| RemoteSignerError::MalformedResponse)?;
            let owner = buzz_sdk_pkg::nip_oa::verify_auth_tag(&serialized, agent)
                .map_err(|_| RemoteSignerError::InvalidSignature)?;
            if owner != self.session.public_key {
                return Err(RemoteSignerError::CapabilityMismatch);
            }
            Ok(response.auth_tag)
        })
        .await
    }
}

// Keep auxiliary owner operations on the same captured remote backend as
// event signing. In particular, memory cannot use ordinary NIP-44 decryption:
// authenticating its keyed address requires a capability the API does not expose.
impl crate::active_user_signer::AgentCapabilities for RemoteSigner {
    fn public_key(&self) -> PublicKey {
        self.session.public_key
    }

    fn authorize_agent<'a>(
        &'a self,
        agent: &'a PublicKey,
        conditions: &'a str,
    ) -> nostr::util::BoxedFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let tag = RemoteSigner::authorize_agent(self, agent, conditions)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&tag).map_err(|e| e.to_string())
        })
    }

    fn read_agent_memory<'a>(
        &'a self,
        _event: &'a nostr::Event,
        _agent: &'a PublicKey,
    ) -> nostr::util::BoxedFuture<'a, Result<Option<buzz_core_pkg::engram::Body>, String>> {
        Box::pin(async { Err(RemoteSignerError::UnsupportedCapability.to_string()) })
    }
}
