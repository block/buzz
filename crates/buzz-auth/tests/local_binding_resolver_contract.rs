//! Compile-time proof that a separate storage crate can implement the narrow
//! local resolver contract without gaining a raw claims-to-authority builder.

use buzz_auth::{
    ActiveLocalBinding, BindingResolutionRequest, CurrentBindingStatusEvidenceRequest,
    LocalBindingResolution, LocalBindingResolver, LocalBindingResolverCapability,
    VerifiedFederatedAssertion,
};
use buzz_core::CanonicalCurrentBindingEvidence;
use chrono::{DateTime, Utc};
use nostr::PublicKey;
use std::future::{pending, Future};
use uuid::Uuid;

struct ExternalPostgresResolver;

impl LocalBindingResolver for ExternalPostgresResolver {
    type Error = ();

    fn capability(&self) -> LocalBindingResolverCapability {
        LocalBindingResolverCapability::DirectAndDelegatedOwnerBound
    }

    fn resolve<'a>(
        &'a self,
        _request: &'a BindingResolutionRequest,
    ) -> impl Future<Output = Result<LocalBindingResolution, Self::Error>> + Send + 'a {
        pending()
    }

    fn current_status_evidence<'a>(
        &'a self,
        _request: &'a CurrentBindingStatusEvidenceRequest,
    ) -> impl Future<Output = Result<CanonicalCurrentBindingEvidence, Self::Error>> + Send + 'a
    {
        pending()
    }

    fn recheck_current_status_evidence<'a>(
        &'a self,
        _evidence: &'a CanonicalCurrentBindingEvidence,
    ) -> impl Future<Output = Result<CanonicalCurrentBindingEvidence, Self::Error>> + Send + 'a
    {
        pending()
    }
}

fn adapt_direct_storage_row(
    assertion: &VerifiedFederatedAssertion,
    binding_id: Uuid,
    binding_version: u64,
    event_author_pubkey: PublicKey,
    expires_at: Option<DateTime<Utc>>,
) -> Option<LocalBindingResolution> {
    let key = assertion.principal_storage_key();
    let _parameterized_binds = (key.issuer(), key.subject());
    let binding = ActiveLocalBinding::from_storage(
        assertion.authorization_domain(),
        assertion.principal_for_storage(),
        binding_id,
        binding_version,
        event_author_pubkey,
        expires_at,
    )?;
    Some(LocalBindingResolution::direct(assertion.clone(), binding))
}

#[test]
fn external_storage_adapter_surface_compiles() {
    fn assert_resolver<T: LocalBindingResolver>() {}
    assert_resolver::<ExternalPostgresResolver>();

    let adapter = adapt_direct_storage_row;
    let _ = adapter;
}
