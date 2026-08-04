use std::sync::Arc;
use std::time::Duration;

use buzz_client::{
    AuthContext, BuzzClient, ClientConfig, ClientError, CommunityEndpoint, RetryPolicy,
};
use nostr::signer::SignerBackend;
use nostr::util::BoxedFuture;
use nostr::{Event, Keys, NostrSigner, PublicKey, SignerError, UnsignedEvent};

#[derive(Debug)]
struct FailingSigner;

impl NostrSigner for FailingSigner {
    fn backend(&self) -> SignerBackend<'_> {
        SignerBackend::Custom("failing-test-signer".into())
    }

    fn get_public_key(&self) -> BoxedFuture<'_, Result<PublicKey, SignerError>> {
        Box::pin(async { Err(SignerError::from("public key unavailable")) })
    }

    fn sign_event(&self, _unsigned: UnsignedEvent) -> BoxedFuture<'_, Result<Event, SignerError>> {
        Box::pin(async { Err(SignerError::from("signing unavailable")) })
    }

    fn nip04_encrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-04 unavailable")) })
    }

    fn nip04_decrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-04 unavailable")) })
    }

    fn nip44_encrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-44 unavailable")) })
    }

    fn nip44_decrypt<'a>(
        &'a self,
        _public_key: &'a PublicKey,
        _payload: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async { Err(SignerError::from("NIP-44 unavailable")) })
    }
}

#[test]
fn config_and_retry_bounds_are_validated_without_environment_lookup() {
    let retry = RetryPolicy::new(4, Duration::from_millis(20), Duration::from_millis(200))
        .expect("valid retry policy");
    assert_eq!(retry.max_attempts(), 4);
    let config = ClientConfig::new(Duration::from_secs(20), Duration::from_secs(5), retry)
        .expect("valid client config");
    assert_eq!(config.request_timeout(), Duration::from_secs(20));
    assert_eq!(config.connect_timeout(), Duration::from_secs(5));

    assert!(RetryPolicy::new(0, Duration::from_millis(1), Duration::from_secs(1)).is_err());
    assert!(RetryPolicy::new(2, Duration::ZERO, Duration::from_secs(1)).is_err());
    assert!(RetryPolicy::new(2, Duration::from_secs(2), Duration::from_secs(1)).is_err());
    assert!(ClientConfig::new(
        Duration::ZERO,
        Duration::from_secs(1),
        RetryPolicy::default(),
    )
    .is_err());
    assert!(ClientConfig::new(
        Duration::from_secs(1),
        Duration::from_secs(2),
        RetryPolicy::default(),
    )
    .is_err());
}

#[tokio::test]
async fn key_backed_signer_builds_and_caches_its_public_key() {
    let keys = Keys::generate();
    let expected = keys.public_key();
    let endpoint = CommunityEndpoint::parse("https://buzz.example").expect("endpoint");
    let client = BuzzClient::builder(endpoint.clone(), Arc::new(keys))
        .build()
        .await
        .expect("client");

    assert_eq!(client.public_key(), expected);
    assert_eq!(client.endpoint(), &endpoint);
    assert_eq!(client.config(), &ClientConfig::default());
}

#[tokio::test]
async fn signer_public_key_failure_is_typed() {
    let endpoint = CommunityEndpoint::parse("https://buzz.example").expect("endpoint");
    let error = BuzzClient::builder(endpoint, Arc::new(FailingSigner))
        .build()
        .await
        .expect_err("failing signer must reject construction");

    assert!(matches!(error, ClientError::Signer(_)));
}

#[tokio::test]
async fn nip_oa_context_is_validated_against_the_active_signer() {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let other_agent = Keys::generate();
    let tag_json = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "kind=9")
        .expect("auth tag");
    let auth = AuthContext::nip_oa(&tag_json).expect("parsed auth context");
    assert!(auth.has_nip_oa());
    let endpoint = CommunityEndpoint::parse("https://buzz.example").expect("endpoint");

    BuzzClient::builder(endpoint.clone(), Arc::new(agent))
        .auth_context(auth.clone())
        .build()
        .await
        .expect("matching signer");

    let error = BuzzClient::builder(endpoint, Arc::new(other_agent))
        .auth_context(auth)
        .build()
        .await
        .expect_err("mismatched signer must fail before requests");
    assert!(matches!(error, ClientError::Authentication(_)));
}

#[test]
fn malformed_nip_oa_context_is_rejected_at_the_input_boundary() {
    let error = AuthContext::nip_oa("not JSON").expect_err("malformed auth must fail");
    assert!(matches!(error, ClientError::Authentication(_)));
}
