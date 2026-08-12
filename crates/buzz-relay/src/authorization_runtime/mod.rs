//! Provider-free NIP-FI relay authorization runtime.
//!
//! This module owns relay-side composition only. Storage transactions,
//! protected-resource witnesses, and the public current-status wire contract
//! enter through typed canonical interfaces.

/// Crash-durable exact-connection status delivery.
pub mod outbox;

mod authority;
mod canonical_admission;
mod config;
mod error;
mod invalidation;
mod jwks;
mod restore;
mod routes;
mod startup;
mod status;

pub use authority::{
    AdmissionCommitPort, AuthorizedRoute, ProtectedResourceWitness, RuntimeAdmissionRequest,
    RuntimeAuthority,
};
pub use canonical_admission::CanonicalAdmissionAdapter;
pub use config::{
    ClientStatusAdmissionPolicy, DelegationConfig, EnforceRuntimeConfig, JwksSourceConfig,
    ProviderFreeRuntimeConfig, ProviderFreeRuntimeMode, CONFIG_ENV,
};
pub use error::RuntimeAuthorizationError;
pub use invalidation::{
    InvalidationNotice, InvalidationRegistration, InvalidationRegistry, ReconciledInvalidationState,
};
pub use jwks::{
    DynamicVerifier, JwksDocumentLoader, JwksRefreshPolicy, JwksSnapshot, ReqwestJwksDocumentLoader,
};
pub use restore::{
    RestoreComponent, RestoreCoordinator, RestoreDecision, RestoreDelta, RestoreManifest,
    RestoreStateReader,
};
pub use routes::{
    ProtectedEffect, ProtectedIngress, ProtectedResourceKind, ResolvedProtectedRoute,
    RouteAuthority, RouteRule,
};
pub use startup::{
    AggregateReadiness, DatabaseRoleWitness, DelegationReadinessWitness,
    InstalledAuthorizationRuntime, ReadinessReason, RestoreReconciliationWitness,
    RouteInventoryWitness, RuntimeStartup, RuntimeStateComponents, SchemaWitness,
    VerifierReadinessWitness,
};
pub use status::{
    ConnectionLocalStatusContract, ConnectionStatusSession, CurrentStatusAuthorization,
    CurrentStatusContract, CurrentStatusEvidenceSource, CurrentStatusSink,
    LocalBindingStatusEvidenceSource, StatusCadence, StatusSessionError,
    UnchangedBootstrapDelivery,
};

#[cfg(test)]
#[path = "../../tests/nip_fi_runtime/mod.rs"]
mod nip_fi_runtime;
