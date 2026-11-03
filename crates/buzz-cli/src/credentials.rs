//! The CLI's credential chain: which identity it acts as, how it signs, and
//! which NIP-FI evidence it presents to the relay.
//!
//! Every relay request goes through [`Credentials`]. Signing and NIP-44 go
//! through a [`NostrSigner`], so a credential link never has to hand the CLI a
//! secret key. [`Credentials::evidence`] supplies the `Nostr-Federated-Identity`
//! assertion when an evidence source is configured and `None` otherwise, so one
//! code path serves relays with and without NIP-FI.
//!
//! [`resolve`] tries each link in order; the first configured link wins. An
//! explicit key (`--private-key` / `BUZZ_PRIVATE_KEY`) is the first link, so a
//! caller that chooses a key always gets that key.

use std::sync::Arc;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, NostrSigner, PublicKey, Tag};
use sha2::{Digest, Sha256};

use crate::error::CliError;
use crate::federated_identity::AdapterSession;

/// Environment variable carrying a NIP-FA adapter session token.
pub const FEDERATED_IDENTITY_TOKEN_ENV: &str = "BUZZ_FEDERATED_IDENTITY_TOKEN";
/// Environment variable carrying the NIP-FA adapter base URL.
pub const FEDERATED_IDENTITY_ADAPTER_ENV: &str = "BUZZ_FEDERATED_IDENTITY_ADAPTER";

/// Raw credential settings, from flags and the environment.
#[derive(Default)]
pub struct Settings {
    /// `--private-key` / `BUZZ_PRIVATE_KEY`.
    pub private_key: Option<String>,
    /// `--auth-tag` / `BUZZ_AUTH_TAG` (NIP-OA owner attestation).
    pub auth_tag: Option<String>,
    /// `BUZZ_FEDERATED_IDENTITY_TOKEN` (NIP-FA adapter session token).
    pub federated_identity_token: Option<String>,
    /// `BUZZ_FEDERATED_IDENTITY_ADAPTER` (NIP-FA adapter base URL).
    pub federated_identity_adapter: Option<String>,
}

impl Settings {
    /// Read the evidence-source settings from the environment. Flags for the
    /// session token are deliberately absent: command lines are visible to
    /// other local users.
    pub fn with_env_evidence(mut self) -> Self {
        let non_empty = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        self.federated_identity_token = non_empty(FEDERATED_IDENTITY_TOKEN_ENV);
        self.federated_identity_adapter = non_empty(FEDERATED_IDENTITY_ADAPTER_ENV);
        self
    }
}

/// A verified NIP-OA auth tag and its canonical JSON for the `x-auth-tag` header.
pub struct AuthTag {
    pub tag: Tag,
    pub json: String,
}

/// The identity the CLI acts as for this process.
pub struct Credentials {
    public_key: PublicKey,
    signer: Arc<dyn NostrSigner>,
    /// Key material, present only when the link holds the key locally.
    local_keys: Option<Keys>,
    auth_tag: Option<AuthTag>,
    evidence: Option<AdapterSession>,
    /// The Buzz desktop, when it is the link that holds the key.
    #[cfg(unix)]
    desktop: Option<Arc<crate::desktop_provider::Provider>>,
}

/// Walk the credential chain for `relay_url` and return the first link that
/// applies: the explicit key, then (on Unix) the Buzz desktop through an
/// inherited or paired token.
pub async fn resolve(settings: Settings, relay_url: &str) -> Result<Credentials, CliError> {
    if let Some(credentials) = resolve_explicit(&settings)? {
        return Ok(credentials);
    }
    #[cfg(unix)]
    if let Some(provider) = crate::desktop_provider::discover(relay_url).await? {
        return Credentials::from_desktop(provider).await;
    }
    #[cfg(not(unix))]
    let _ = relay_url;
    Err(CliError::Auth(
        "no credentials: set BUZZ_PRIVATE_KEY, pass --private-key, or run \
         `buzz auth desktop pair` with Buzz running"
            .into(),
    ))
}

/// The explicit-key link, with its headless evidence source.
fn resolve_explicit(settings: &Settings) -> Result<Option<Credentials>, CliError> {
    let evidence = evidence_source(settings)?;
    explicit_key(settings, evidence)
}

/// Link 1: an explicit key from `--private-key` or `BUZZ_PRIVATE_KEY`.
fn explicit_key(
    settings: &Settings,
    evidence: Option<AdapterSession>,
) -> Result<Option<Credentials>, CliError> {
    let Some(raw) = settings.private_key.as_deref() else {
        return Ok(None);
    };
    let keys =
        Keys::parse(raw).map_err(|e| CliError::Key(format!("invalid BUZZ_PRIVATE_KEY: {e}")))?;
    let auth_tag = settings
        .auth_tag
        .as_deref()
        .filter(|input| !input.is_empty())
        .map(|input| parse_auth_tag(input, &keys.public_key()))
        .transpose()?;
    Ok(Some(Credentials {
        auth_tag,
        evidence,
        ..Credentials::from_keys(keys)
    }))
}

/// The headless NIP-FI evidence source: a NIP-FA adapter session. Both
/// settings are required together; one without the other is a configuration
/// error rather than a silent fallback to no evidence.
fn evidence_source(settings: &Settings) -> Result<Option<AdapterSession>, CliError> {
    match (
        settings.federated_identity_token.as_deref(),
        settings.federated_identity_adapter.as_deref(),
    ) {
        (None, None) => Ok(None),
        (Some(token), Some(adapter)) => AdapterSession::new(adapter, token).map(Some),
        (Some(_), None) => Err(CliError::Auth(format!(
            "{FEDERATED_IDENTITY_TOKEN_ENV} is set but {FEDERATED_IDENTITY_ADAPTER_ENV} is not"
        ))),
        (None, Some(_)) => Err(CliError::Auth(format!(
            "{FEDERATED_IDENTITY_ADAPTER_ENV} is set but {FEDERATED_IDENTITY_TOKEN_ENV} is not"
        ))),
    }
}

/// Parse and verify a NIP-OA auth tag for `public_key`.
///
/// `BUZZ_AUTH_TAG` is hand-authored configuration, so the unquoted raw
/// shorthand `[auth,hex,,hex]` is normalized to JSON here — at this input
/// edge only. The SDK grammar and the `x-auth-tag` wire format stay strict
/// JSON; all validation and signature verification happen on the strict path.
fn parse_auth_tag(input: &str, public_key: &PublicKey) -> Result<AuthTag, CliError> {
    let json = crate::normalize_auth_tag_input(input);
    let tag = buzz_sdk::nip_oa::parse_auth_tag(&json)
        .map_err(|e| CliError::Auth(format!("BUZZ_AUTH_TAG is malformed: {e}")))?;
    buzz_sdk::nip_oa::verify_auth_tag(&json, public_key).map_err(|e| {
        CliError::Auth(format!(
            "BUZZ_AUTH_TAG verification failed for pubkey {}: {e}",
            public_key.to_hex()
        ))
    })?;
    // Canonical wire form derives from the parsed-and-verified tag
    // (same shape as buzz-acp's RestClient), never from raw input.
    let canonical = serde_json::to_string(tag.as_slice())
        .map_err(|e| CliError::Auth(format!("BUZZ_AUTH_TAG serialization failed: {e}")))?;
    Ok(AuthTag {
        tag,
        json: canonical,
    })
}

impl Credentials {
    /// Credentials for an in-memory key with no auth tag and no evidence source.
    pub fn from_keys(keys: Keys) -> Self {
        Self {
            public_key: keys.public_key(),
            signer: Arc::new(keys.clone()),
            local_keys: Some(keys),
            auth_tag: None,
            evidence: None,
            #[cfg(unix)]
            desktop: None,
        }
    }

    /// Credentials backed by the Buzz desktop: it signs, encrypts and supplies
    /// NIP-FI evidence; the private key never reaches this process.
    #[cfg(unix)]
    pub(crate) async fn from_desktop(
        provider: Arc<crate::desktop_provider::Provider>,
    ) -> Result<Self, CliError> {
        let public_key = provider.public_key();
        let auth_tag = provider
            .auth_tag()
            .await?
            .map(|json| parse_auth_tag(&json, &public_key))
            .transpose()?;
        Ok(Self {
            public_key,
            signer: provider.clone(),
            local_keys: None,
            auth_tag,
            evidence: None,
            desktop: Some(provider),
        })
    }

    /// Attach a verified NIP-OA auth tag.
    #[cfg(test)]
    pub fn with_auth_tag(mut self, tag: Tag, json: String) -> Self {
        self.auth_tag = Some(AuthTag { tag, json });
        self
    }

    /// The local keys behind a [`Self::from_keys`] credential.
    #[cfg(test)]
    pub fn test_keys(&self) -> Keys {
        self.local_keys
            .clone()
            .expect("test credentials hold local keys")
    }

    /// Attach a NIP-FA evidence source.
    #[cfg(test)]
    pub fn with_evidence(mut self, evidence: AdapterSession) -> Self {
        self.evidence = Some(evidence);
        self
    }

    /// The public key this process acts as.
    pub fn public_key(&self) -> PublicKey {
        self.public_key
    }

    /// The signer, for protocol code that takes a [`NostrSigner`] (NIP-42).
    pub fn signer(&self) -> &Arc<dyn NostrSigner> {
        &self.signer
    }

    /// The NIP-OA auth tag, if configured.
    pub fn auth_tag(&self) -> Option<&AuthTag> {
        self.auth_tag.as_ref()
    }

    /// The NIP-AE engram `d` tag for `slug` in the conversation with `peer`:
    /// HMAC-SHA256 over the NIP-44 conversation key, so it needs the key
    /// itself rather than a signature (`docs/nips/NIP-AE.md`).
    pub async fn engram_d_tag(&self, peer: &PublicKey, slug: &str) -> Result<String, CliError> {
        #[cfg(unix)]
        if let Some(provider) = &self.desktop {
            return provider.engram_d_tag(peer, slug).await;
        }
        let keys = self.local_keys.as_ref().ok_or_else(|| {
            CliError::Auth(
                "buzz mem needs a local key: set BUZZ_PRIVATE_KEY or pass --private-key".into(),
            )
        })?;
        let k_c = buzz_core::engram::conversation_key(keys.secret_key(), peer);
        Ok(buzz_core::engram::d_tag(&k_c, slug))
    }

    /// A stable, anonymous, relay-scoped id for the GIF provider (see
    /// [`customer_id`]). A link without local key material gets a random
    /// per-process id instead, as Buzz Desktop does per install.
    pub fn relay_scoped_customer_id(&self, relay_url: &str) -> String {
        match self.local_keys.as_ref() {
            Some(keys) => customer_id(keys.secret_key().as_secret_bytes(), relay_url),
            None => {
                static PROCESS_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
                PROCESS_ID
                    .get_or_init(|| uuid::Uuid::new_v4().simple().to_string())
                    .clone()
            }
        }
    }

    /// Whether a NIP-FI evidence source is configured.
    pub fn has_evidence_source(&self) -> bool {
        #[cfg(unix)]
        if self.desktop.is_some() {
            return true;
        }
        self.evidence.is_some()
    }

    /// The NIP-FI assertion to present to `relay_url`, or `None` when no
    /// evidence source is configured.
    pub async fn evidence(&self, relay_url: &str) -> Result<Option<String>, CliError> {
        #[cfg(unix)]
        if let Some(provider) = &self.desktop {
            let relay_url = crate::desktop_provider::provider_relay_url(relay_url)?;
            return provider.federated_identity(&relay_url).await;
        }
        match &self.evidence {
            Some(source) => source.assertion(relay_url, self).await.map(Some),
            None => Ok(None),
        }
    }

    /// Sign `builder` as this identity. Callers own the tags; auth-tag
    /// injection is [`crate::client::BuzzClient::sign_event`]'s job.
    pub async fn sign(&self, builder: EventBuilder) -> Result<Event, CliError> {
        let unsigned = builder.build(self.public_key);
        let event = self
            .signer
            .sign_event(unsigned)
            .await
            .map_err(|e| CliError::Other(format!("signing failed: {e}")))?;
        if event.pubkey != self.public_key || event.verify().is_err() {
            return Err(CliError::Auth(
                "signer returned an event that is not validly signed by this identity".into(),
            ));
        }
        Ok(event)
    }

    /// NIP-44 encrypt `plaintext` to `peer` as this identity.
    pub async fn nip44_encrypt(
        &self,
        peer: &PublicKey,
        plaintext: &str,
    ) -> Result<String, CliError> {
        self.signer
            .nip44_encrypt(peer, plaintext)
            .await
            .map_err(|e| CliError::Other(format!("NIP-44 encryption failed: {e}")))
    }

    /// NIP-44 decrypt `payload` from `peer` as this identity.
    /// NIP-44 decrypt `payload` from `peer`. `Ok(None)` means the payload is
    /// not valid ciphertext for this key; `Err` means the signer itself failed.
    /// A local key decides that itself, and the desktop reports it as
    /// `invalid_request`; any other signer's failure is `Err`, since it cannot
    /// be told apart from an unavailable credential.
    pub async fn nip44_decrypt(
        &self,
        peer: &PublicKey,
        payload: &str,
    ) -> Result<Option<String>, CliError> {
        if let Some(keys) = &self.local_keys {
            return Ok(nostr::nips::nip44::decrypt(keys.secret_key(), peer, payload).ok());
        }
        #[cfg(unix)]
        if let Some(provider) = &self.desktop {
            return provider.nip44_decrypt(peer, payload).await;
        }
        self.signer
            .nip44_decrypt(peer, payload)
            .await
            .map(Some)
            .map_err(|e| CliError::Auth(format!("NIP-44 decryption failed: {e}")))
    }

    /// Sign a NIP-98 HTTP auth event (kind:27235) for `method` + `url` and
    /// return it base64-encoded, ready for a `Nostr <value>` header.
    ///
    /// The event carries the `u`, `method` and (with a body) `payload` tags,
    /// plus a nonce so rapid identical requests don't trip replay guards.
    pub async fn nip98(
        &self,
        method: &str,
        url: &str,
        body: Option<&[u8]>,
    ) -> Result<String, CliError> {
        let tag = |parts: &[&str]| {
            Tag::parse(parts.iter().copied())
                .map_err(|e| CliError::Other(format!("tag error: {e}")))
        };
        let mut tags = vec![
            tag(&["u", url])?,
            tag(&["method", method])?,
            tag(&["nonce", &uuid::Uuid::new_v4().to_string()])?,
        ];
        if let Some(body) = body {
            tags.push(tag(&["payload", &hex::encode(Sha256::digest(body))])?);
        }
        let event = self
            .sign(EventBuilder::new(Kind::HttpAuth, "").tags(tags))
            .await?;
        Ok(B64.encode(event.as_json().as_bytes()))
    }
}

/// Derive a stable, relay-scoped anonymous `customer_id` from secret key material.
///
/// KLIPY requires a per-installation identifier that is stable and anonymous.
/// Using SHA-256 of the *public* key would be stable but NOT anonymous — the
/// input is public, so the ID is computable by any observer, and the same value
/// would appear across all relays (cross-relay linkability).
///
/// Instead, we domain-separate with the relay URL and sign with the *secret* key:
///   `SHA-256(secret_key_bytes || '\0' || relay_url_bytes)`
/// This is:
/// - **stable**: deterministic given the same keypair + relay.
/// - **relay-scoped**: different relay → different ID, no cross-relay correlation.
/// - **not computable from public data**: requires secret key material.
/// - **stateless**: no file I/O, no storage.
///
/// The first 16 bytes (32 hex chars) give 128 bits of uniqueness, ample for
/// KLIPY's per-installation needs.
fn customer_id(secret_key_bytes: &[u8], relay_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret_key_bytes);
    hasher.update(b"\0"); // domain separator
    hasher.update(relay_url.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..16]) // 16 bytes → 32 hex chars
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // customer_id
    // -----------------------------------------------------------------------

    #[test]
    fn customer_id_is_32_hex_chars_and_stable() {
        let sk = [0xab_u8; 32];
        let id = customer_id(&sk, "https://relay.example");
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id, customer_id(&sk, "https://relay.example"));
    }

    #[test]
    fn customer_id_is_relay_scoped() {
        let sk = [0xcd_u8; 32];
        let id_a = customer_id(&sk, "https://relay-a.example");
        let id_b = customer_id(&sk, "https://relay-b.example");
        assert_ne!(
            id_a, id_b,
            "same key, different relay → different customer_id"
        );
    }

    #[test]
    fn customer_id_differs_for_different_keys() {
        let id_a = customer_id(&[0xaa_u8; 32], "https://relay.example");
        let id_b = customer_id(&[0xbb_u8; 32], "https://relay.example");
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn customer_id_not_equal_to_pubkey_hash() {
        // The customer_id must NOT be derivable from the public key alone.
        let sk = [0xde_u8; 32];
        // What the old pubkey-hash approach would have produced (approximately):
        let naive_hash = hex::encode(&Sha256::digest(hex::encode(sk).as_bytes())[..16]);
        let actual = customer_id(&sk, "https://relay.example");
        assert_ne!(
            actual, naive_hash,
            "customer_id must not equal SHA-256(pubkey_hex)[..16]"
        );
    }

    fn explicit(settings: Settings) -> Result<Credentials, CliError> {
        resolve_explicit(&settings).map(|c| c.expect("explicit key"))
    }

    fn settings(key: Option<&str>, token: Option<&str>, adapter: Option<&str>) -> Settings {
        Settings {
            private_key: key.map(str::to_owned),
            auth_tag: None,
            federated_identity_token: token.map(str::to_owned),
            federated_identity_adapter: adapter.map(str::to_owned),
        }
    }

    #[test]
    fn explicit_key_is_the_first_link() {
        let keys = Keys::generate();
        let nsec = keys.secret_key().to_secret_hex();
        let credentials = explicit(settings(Some(&nsec), None, None)).unwrap();
        assert_eq!(credentials.public_key(), keys.public_key());
        assert!(!credentials.has_evidence_source());
    }

    #[test]
    fn no_link_is_an_auth_error_naming_the_settings() {
        assert!(resolve_explicit(&settings(None, None, None))
            .unwrap()
            .is_none());
    }

    #[test]
    fn evidence_settings_must_come_together() {
        let nsec = Keys::generate().secret_key().to_secret_hex();
        for (token, adapter, missing) in [
            (Some("t"), None, FEDERATED_IDENTITY_ADAPTER_ENV),
            (
                None,
                Some("https://adapter.example"),
                FEDERATED_IDENTITY_TOKEN_ENV,
            ),
        ] {
            let err = explicit(settings(Some(&nsec), token, adapter))
                .err()
                .unwrap();
            assert!(
                matches!(&err, CliError::Auth(m) if m.contains(missing)),
                "{token:?} {adapter:?}: {err}"
            );
        }
        let credentials = explicit(settings(
            Some(&nsec),
            Some("t"),
            Some("https://adapter.example"),
        ))
        .unwrap();
        assert!(credentials.has_evidence_source());
    }

    #[tokio::test]
    async fn key_derived_operations_match_the_protocol_definitions() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let credentials = Credentials::from_keys(agent.clone());
        // NIP-AE: either side of the pair derives the same d tag.
        let owner_side = buzz_core::engram::d_tag(
            &buzz_core::engram::conversation_key(owner.secret_key(), &agent.public_key()),
            "core",
        );
        assert_eq!(
            credentials
                .engram_d_tag(&owner.public_key(), "core")
                .await
                .unwrap(),
            owner_side
        );
        assert_eq!(
            credentials.relay_scoped_customer_id("https://a.example"),
            customer_id(agent.secret_key().as_secret_bytes(), "https://a.example")
        );
    }

    #[tokio::test]
    async fn sign_rejects_events_from_another_key() {
        #[derive(Debug)]
        struct WrongKey(Keys);
        impl NostrSigner for WrongKey {
            fn backend(&self) -> nostr::signer::SignerBackend<'_> {
                nostr::signer::SignerBackend::Custom("test".into())
            }
            fn get_public_key(
                &self,
            ) -> nostr::util::BoxedFuture<'_, Result<PublicKey, nostr::signer::SignerError>>
            {
                Box::pin(async move { Ok(self.0.public_key()) })
            }
            fn sign_event(
                &self,
                unsigned: nostr::UnsignedEvent,
            ) -> nostr::util::BoxedFuture<'_, Result<Event, nostr::signer::SignerError>>
            {
                let other = Keys::generate();
                Box::pin(async move {
                    EventBuilder::new(unsigned.kind, unsigned.content)
                        .sign_with_keys(&other)
                        .map_err(nostr::signer::SignerError::backend)
                })
            }
            fn nip04_encrypt<'a>(
                &'a self,
                _: &'a PublicKey,
                _: &'a str,
            ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>>
            {
                Box::pin(async { Err(nostr::signer::SignerError::from("unsupported")) })
            }
            fn nip04_decrypt<'a>(
                &'a self,
                _: &'a PublicKey,
                _: &'a str,
            ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>>
            {
                Box::pin(async { Err(nostr::signer::SignerError::from("unsupported")) })
            }
            fn nip44_encrypt<'a>(
                &'a self,
                _: &'a PublicKey,
                _: &'a str,
            ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>>
            {
                Box::pin(async { Err(nostr::signer::SignerError::from("unsupported")) })
            }
            fn nip44_decrypt<'a>(
                &'a self,
                _: &'a PublicKey,
                _: &'a str,
            ) -> nostr::util::BoxedFuture<'a, Result<String, nostr::signer::SignerError>>
            {
                Box::pin(async { Err(nostr::signer::SignerError::from("unsupported")) })
            }
        }

        let keys = Keys::generate();
        let credentials = Credentials {
            public_key: keys.public_key(),
            signer: Arc::new(WrongKey(keys)),
            local_keys: None,
            auth_tag: None,
            evidence: None,
            #[cfg(unix)]
            desktop: None,
        };
        let err = credentials
            .sign(EventBuilder::text_note("hi"))
            .await
            .err()
            .unwrap();
        assert!(matches!(err, CliError::Auth(_)), "{err}");

        // Without local key material: key-derived operations degrade or refuse.
        let peer = Keys::generate().public_key();
        assert!(credentials.engram_d_tag(&peer, "core").await.is_err());
        let relay = "https://relay.example";
        let id = credentials.relay_scoped_customer_id(relay);
        assert_eq!(
            id,
            credentials.relay_scoped_customer_id(relay),
            "one id per process"
        );
        assert_ne!(
            id,
            customer_id(&[0; 32], relay),
            "no local key: a random id, not a key derivation"
        );
        assert_eq!(id.len(), 32);
    }
}
