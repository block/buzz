//! Provider-neutral inventory of relay authorization surfaces.
//!
//! This is the single reviewable registry for HTTP routes and long-lived
//! protocol operations.  It records both protected operations and deliberate
//! exemptions.  Runtime code derives the requested portable capability from
//! this module; request data never selects a provider profile or policy.

use axum::http::Method;
use buzz_auth::{AuthTransport, AuthorizationCapability};

use crate::authorization_runtime::finalization::AuthorizationMode;

/// Closed identifier for every backend-visible effect family.
///
/// Adding an effect requires adding a registry row and choosing an explicit
/// Enforce disposition. Dynamic helper names cannot manufacture a permit.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EffectSurfaceId {
    /// Durable Nostr event storage.
    EventPersistence,
    /// Channel, member, profile, reaction, moderation, or deletion projection.
    EventDomainProjection,
    /// Invitation creation.
    InviteMint,
    /// Invitation consumption and final membership creation.
    InviteClaim,
    /// PostgreSQL-authoritative media visibility.
    MediaPublication,
    /// PostgreSQL-authoritative Git ref visibility.
    GitPublication,
    /// Existing-member audio session admission.
    AudioAdmission,
    /// Legacy automatic audio membership creation.
    AudioAutomaticMembership,
    /// Durable audio lifecycle event persistence.
    AudioLifecyclePersistence,
    /// Automatic last-participant channel archival.
    AudioAutomaticArchive,
    /// Interactive workflow definition or state mutation.
    WorkflowStateMutation,
    /// Autonomous or delayed workflow execution.
    WorkflowBackgroundExecution,
    /// Arbitrary outbound HTTP webhook.
    OutboundWebhook,
    /// Any helper that has not been assigned a closed effect identifier.
    UnclassifiedHelper,
    /// Recipient-fenced local WebSocket delivery.
    LocalFanout,
    /// Recipient-fenced Redis delivery hint.
    RedisFanout,
    /// Redis-backed presence visible only through retained authority.
    ProtectedPresence,
    /// Persistent relay-signed helper event.
    RelaySignedHelperEvent,
    /// External push delivery.
    PushDelivery,
    /// Legacy best-effort audit-channel delivery; O5 owns durable audit.
    LegacyAuditDelivery,
    /// Cache eviction or connection cancellation derived from a commit.
    CacheAndConnectionInvalidation,
    /// Repairable legacy media sidecar written after authoritative publication.
    MediaLegacySidecar,
    /// Legacy moderation upload record emitted by object-store creation.
    MediaUploadRecord,
    /// Repairable legacy Git pointer written after authoritative publication.
    GitLegacyPointer,
    /// Durable invalidation polling and reconciliation.
    AuthorizationReconciliation,
    /// Durable public-assertion retirement derived from committed identity lifecycle state.
    PublicProjectionRetirement,
    /// Retryable local and cross-replica delivery of a committed public retirement.
    PublicProjectionRetirementDelivery,
    /// Dedicated exact-connection delivery of current binding status or withdrawal.
    ClientStatusDelivery,
    /// Storage garbage collection.
    StorageGarbageCollection,
    /// Reminder claim or publication.
    ReminderWorker,
    /// Push matching or delivery worker.
    PushWorker,
    /// Partition, expiry, or retention maintenance.
    DatabaseMaintenance,
    /// Audio mesh ownership maintenance.
    AudioMeshMaintenance,
    /// Metrics-only observation.
    MetricsObservation,
}

/// Effect relationship to protected business state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EffectClass {
    /// The effect is the durable source of protected state or visibility.
    AuthoritativeMutation,
    /// The effect is derived from an already-committed authoritative result.
    DerivedDelivery,
    /// The effect is server-owned maintenance rather than user authority.
    SystemMaintenance,
}

/// Code location category used by inventory coverage checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EffectOrigin {
    /// HTTP route.
    Http,
    /// WebSocket operation.
    WebSocket,
    /// Shared helper invoked from more than one route.
    Helper,
    /// Autonomous or delayed background task.
    Background,
}

/// Why an effect is deliberately unavailable in Enforce.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum UnavailableReason {
    /// No transaction-owned authorization permit is retained.
    MissingTransactionAuthority,
    /// No reviewed autonomous/system authority model exists.
    MissingBackgroundAuthority,
    /// The target cannot provide an authoritative idempotent commit boundary.
    MissingExternalCommitPrimitive,
    /// The legacy behavior would create membership implicitly.
    AutomaticMembershipForbidden,
    /// The effect has not been classified and registered.
    UnclassifiedEffect,
}

/// Enforce behavior selected for a registered effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EnforceDisposition {
    /// The effect has the required authoritative or release primitive.
    Supported,
    /// Deny synchronously before the effect begins.
    DenyBeforeEffect(UnavailableReason),
}

/// One machine-readable effect registry row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectSurface {
    /// Closed effect identifier.
    pub id: EffectSurfaceId,
    /// Stable provider-neutral inventory label.
    pub name: &'static str,
    /// Relationship to protected state.
    pub class: EffectClass,
    /// Code location category.
    pub origin: EffectOrigin,
    /// Portable capability, when the effect acts for a user operation.
    pub capability: Option<AuthorizationCapability>,
    /// Enforce behavior.
    pub enforce: EnforceDisposition,
    /// Mandatory lifetime checkpoints.
    pub guard_points: &'static [GuardPoint],
}

/// Enforce implementation selected for one authenticated EVENT kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventMutationDisposition {
    /// The event and thread metadata use the common PostgreSQL executor.
    TransactionalPersistence,
    /// The kind requires a projection that has no transaction-aware adapter.
    UnavailableProjection,
    /// The kind enters a command/workflow executor without a shared commit.
    UnavailableCommandOrWorkflow,
}

/// Classify every EVENT kind before any Enforce mutation begins.
///
/// Unknown kinds still fail in the ingest allowlist. This function only
/// chooses the commit primitive for kinds that pass normal protocol checks.
pub fn event_mutation_disposition(kind: u32) -> EventMutationDisposition {
    use buzz_core::kind::*;

    if buzz_core::kind::is_moderation_command_kind(kind)
        || matches!(kind, KIND_REPORT | KIND_GIT_REPO_ANNOUNCEMENT)
    {
        return EventMutationDisposition::TransactionalPersistence;
    }
    if matches!(
        kind,
        KIND_WORKFLOW_DEF
            | KIND_WORKFLOW_TRIGGER
            | KIND_APPROVAL_GRANT
            | KIND_APPROVAL_DENY
            | KIND_PUSH_LEASE
    ) {
        return EventMutationDisposition::UnavailableCommandOrWorkflow;
    }
    if matches!(
        kind,
        KIND_DM_OPEN
            | KIND_DM_ADD_MEMBER
            | KIND_DM_HIDE
            | KIND_PRODUCT_FEEDBACK
            | KIND_NIP43_LEAVE_REQUEST
            | KIND_NIP29_CREATE_INVITE
    ) || buzz_core::kind::is_relay_admin_kind(kind)
        || buzz_core::kind::is_identity_archive_request_kind(kind)
    {
        return EventMutationDisposition::TransactionalPersistence;
    }
    if matches!(
        kind,
        9003..=9004
            | 9006
            | 9010..=9020
            | 41001..=41003
            | 40099
    ) {
        return EventMutationDisposition::UnavailableProjection;
    }
    EventMutationDisposition::TransactionalPersistence
}

/// Recheck the stable handler surface, proof transport, and selected portable
/// capability before a resolver can observe the request.
pub fn protected_operation_matches(
    surface: &str,
    transport: AuthTransport,
    capability: AuthorizationCapability,
) -> bool {
    use AuthorizationCapability as Capability;
    match surface {
        "ws_req" | "ws_count" | "ws_fanout" | "client.status.current" => {
            transport == AuthTransport::RelayWebSocket && capability == Capability::CommunityRead
        }
        "ws_event" => {
            transport == AuthTransport::RelayWebSocket
                && matches!(
                    capability,
                    Capability::CommunityWrite | Capability::Moderate
                )
        }
        "event_ingest" => {
            matches!(
                transport,
                AuthTransport::RelayWebSocket | AuthTransport::HttpBridge
            ) && matches!(
                capability,
                Capability::CommunityWrite | Capability::Moderate
            )
        }
        "http_events" => {
            transport == AuthTransport::HttpBridge
                && matches!(
                    capability,
                    Capability::CommunityWrite | Capability::Moderate
                )
        }
        "http_query" | "http_count" => {
            transport == AuthTransport::HttpBridge && capability == Capability::CommunityRead
        }
        "http_moderation_read" => {
            transport == AuthTransport::HttpBridge && capability == Capability::Moderate
        }
        "media.upload" => {
            transport == AuthTransport::MediaUpload && capability == Capability::MediaWrite
        }
        "media.read" => {
            transport == AuthTransport::MediaDownload && capability == Capability::MediaRead
        }
        "git.info_refs" => {
            transport == AuthTransport::Git
                && matches!(capability, Capability::GitRead | Capability::GitWrite)
        }
        "git.upload_pack" => transport == AuthTransport::Git && capability == Capability::GitRead,
        "git.receive_pack" => transport == AuthTransport::Git && capability == Capability::GitWrite,
        "audio.join" => transport == AuthTransport::Audio && capability == Capability::AudioJoin,
        "invite.mint" => {
            transport == AuthTransport::HttpBridge && capability == Capability::InviteMint
        }
        "invite.claim" => {
            transport == AuthTransport::HttpBridge && capability == Capability::InviteClaim
        }
        _ => false,
    }
}

/// Why a registered surface is deliberately outside tenant authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SurfaceExemption {
    /// Public relay metadata (NIP-05 and NIP-11-adjacent information).
    PublicMetadata,
    /// Kubernetes or service health endpoint.
    HealthProbe,
    /// Public pre-membership policy bootstrap.
    JoinBootstrap,
    /// Deployment-global operator authentication.
    OperatorAuth,
    /// Deployment-admin host/session authentication.
    AdminAuth,
    /// Loopback-only, HMAC-authenticated Git hook callback.
    LocalHookCallback,
    /// Disabled-by-default mesh testbed endpoint.
    TestbedOnly,
    /// Static UI fallback that cannot reach an API handler.
    StaticUiFallback,
}

/// How a registered surface participates in protected authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceProtection {
    /// One fixed portable capability is required for the request.
    Capability(AuthorizationCapability),
    /// The request body or protocol operation determines the capability.
    DynamicCapability,
    /// A fixed capability committed through an authoritative transaction/CAS.
    AtomicMutation(AuthorizationCapability),
    /// A dynamically selected mutation committed through an authoritative executor.
    DynamicAtomicMutation,
    /// A fixed capability whose Enforce path remains unavailable until a
    /// backend-specific transaction/CAS executor owns the complete commit.
    AtomicMutationUnavailable(AuthorizationCapability),
    /// A dynamically selected mutation capability with the same fail-closed
    /// backend-executor requirement.
    DynamicAtomicMutationUnavailable,
    /// Authentication is completed after the HTTP upgrade.
    Session {
        /// Fixed capability for the session, or `None` for per-operation WS
        /// authorization after NIP-42 AUTH.
        capability: Option<AuthorizationCapability>,
    },
    /// A long-lived session whose admission mutates protected state and is
    /// therefore unavailable in Enforce without an atomic backend executor.
    AtomicMutationSessionUnavailable {
        /// Fixed capability required by the session admission.
        capability: AuthorizationCapability,
    },
    /// A non-persistent session admitted under a bounded lease and cancellation fence.
    LeasedSession {
        /// Fixed capability required by the session admission.
        capability: AuthorizationCapability,
    },
    /// Plain GET/HEAD is public metadata; a WebSocket upgrade enters protected
    /// per-operation session authorization.
    ConditionalWebSocketUpgrade,
    /// Protection depends on the server-owned media-read setting.
    ConditionalMediaRead(AuthorizationCapability),
    /// Deliberately outside tenant protected authorization.
    Exempt(SurfaceExemption),
}

impl SurfaceProtection {
    /// Stable low-cardinality label for tracing and inventory exports.
    pub const fn trace_label(self) -> &'static str {
        match self {
            Self::Capability(_) => "required",
            Self::DynamicCapability => "required_dynamic_capability",
            Self::AtomicMutation(_) => "required_atomic_commit",
            Self::DynamicAtomicMutation => "required_dynamic_atomic_commit",
            Self::AtomicMutationUnavailable(_) => "enforce_unavailable_without_atomic_executor",
            Self::DynamicAtomicMutationUnavailable => {
                "enforce_unavailable_without_dynamic_atomic_executor"
            }
            Self::Session { .. } => "required_at_session_auth",
            Self::AtomicMutationSessionUnavailable { .. } => {
                "enforce_session_unavailable_without_atomic_executor"
            }
            Self::LeasedSession { .. } => "required_leased_session",
            Self::ConditionalWebSocketUpgrade => "required_on_websocket_upgrade",
            Self::ConditionalMediaRead(_) => "required_when_media_reads_protected",
            Self::Exempt(SurfaceExemption::PublicMetadata) => "exempt_public_metadata",
            Self::Exempt(SurfaceExemption::HealthProbe) => "exempt_health_probe",
            Self::Exempt(SurfaceExemption::JoinBootstrap) => "exempt_join_bootstrap",
            Self::Exempt(SurfaceExemption::OperatorAuth) => "exempt_operator_auth",
            Self::Exempt(SurfaceExemption::AdminAuth) => "exempt_admin_auth",
            Self::Exempt(SurfaceExemption::LocalHookCallback) => "exempt_local_hook_callback",
            Self::Exempt(SurfaceExemption::TestbedOnly) => "exempt_testbed_only",
            Self::Exempt(SurfaceExemption::StaticUiFallback) => "exempt_static_ui_fallback",
        }
    }
}

/// Required lifetime checkpoints for a protected operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GuardPoint {
    /// Validate before handler work begins.
    Request,
    /// Revalidate before a durable or externally visible mutation commits.
    PreCommit,
    /// Revalidate after asynchronous fetches and before buffered output is released.
    PreEmission,
    /// Revalidate before each streamed chunk or live event emission.
    StreamEmission,
    /// Renew or close a long-lived session before its lease expires.
    SessionRenewal,
}

const REQUEST: &[GuardPoint] = &[GuardPoint::Request];
const REQUEST_COMMIT: &[GuardPoint] = &[GuardPoint::Request, GuardPoint::PreCommit];
const REQUEST_EMIT: &[GuardPoint] = &[GuardPoint::Request, GuardPoint::PreEmission];
const REQUEST_STREAM: &[GuardPoint] = &[
    GuardPoint::Request,
    GuardPoint::PreEmission,
    GuardPoint::StreamEmission,
];
const SESSION_STREAM: &[GuardPoint] = &[
    GuardPoint::Request,
    GuardPoint::SessionRenewal,
    GuardPoint::StreamEmission,
];
const SESSION_COMMIT_STREAM: &[GuardPoint] = &[
    GuardPoint::Request,
    GuardPoint::PreCommit,
    GuardPoint::PreEmission,
    GuardPoint::SessionRenewal,
    GuardPoint::StreamEmission,
];

const fn effect(
    id: EffectSurfaceId,
    name: &'static str,
    class: EffectClass,
    origin: EffectOrigin,
    capability: Option<AuthorizationCapability>,
    enforce: EnforceDisposition,
    guard_points: &'static [GuardPoint],
) -> EffectSurface {
    EffectSurface {
        id,
        name,
        class,
        origin,
        capability,
        enforce,
        guard_points,
    }
}

/// Exhaustive provider-neutral inventory of backend-visible effect families.
pub const EFFECT_SURFACES: &[EffectSurface] = &[
    effect(
        EffectSurfaceId::EventPersistence,
        "event.persistence",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Helper,
        Some(AuthorizationCapability::CommunityWrite),
        EnforceDisposition::Supported,
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::EventDomainProjection,
        "event.domain_projection",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Helper,
        Some(AuthorizationCapability::CommunityWrite),
        EnforceDisposition::Supported,
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::InviteMint,
        "invite.mint_commit",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Http,
        Some(AuthorizationCapability::InviteMint),
        EnforceDisposition::Supported,
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::InviteClaim,
        "invite.claim_commit",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Http,
        Some(AuthorizationCapability::InviteClaim),
        EnforceDisposition::Supported,
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::MediaPublication,
        "media.publication",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Http,
        Some(AuthorizationCapability::MediaWrite),
        EnforceDisposition::Supported,
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::GitPublication,
        "git.publication",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Http,
        Some(AuthorizationCapability::GitWrite),
        EnforceDisposition::Supported,
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::AudioAdmission,
        "audio.existing_member_admission",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Http,
        Some(AuthorizationCapability::AudioJoin),
        EnforceDisposition::Supported,
        SESSION_COMMIT_STREAM,
    ),
    effect(
        EffectSurfaceId::AudioAutomaticMembership,
        "audio.automatic_membership",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Helper,
        Some(AuthorizationCapability::AudioJoin),
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::AutomaticMembershipForbidden),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::AudioLifecyclePersistence,
        "audio.lifecycle_persistence",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Helper,
        Some(AuthorizationCapability::AudioJoin),
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingTransactionAuthority),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::AudioAutomaticArchive,
        "audio.automatic_archive",
        EffectClass::SystemMaintenance,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::WorkflowStateMutation,
        "workflow.interactive_state",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Helper,
        Some(AuthorizationCapability::CommunityWrite),
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingTransactionAuthority),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::WorkflowBackgroundExecution,
        "workflow.background_execution",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::OutboundWebhook,
        "workflow.outbound_webhook",
        EffectClass::DerivedDelivery,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingExternalCommitPrimitive),
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::UnclassifiedHelper,
        "background.unclassified_helper",
        EffectClass::DerivedDelivery,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::UnclassifiedEffect),
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::LocalFanout,
        "delivery.local_fanout",
        EffectClass::DerivedDelivery,
        EffectOrigin::Helper,
        Some(AuthorizationCapability::CommunityRead),
        EnforceDisposition::Supported,
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::RedisFanout,
        "delivery.redis_fanout",
        EffectClass::DerivedDelivery,
        EffectOrigin::Helper,
        Some(AuthorizationCapability::CommunityRead),
        EnforceDisposition::Supported,
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::ProtectedPresence,
        "delivery.protected_presence",
        EffectClass::DerivedDelivery,
        EffectOrigin::WebSocket,
        Some(AuthorizationCapability::CommunityRead),
        EnforceDisposition::Supported,
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::RelaySignedHelperEvent,
        "delivery.relay_signed_helper_event",
        EffectClass::DerivedDelivery,
        EffectOrigin::Helper,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::PushDelivery,
        "delivery.external_push",
        EffectClass::DerivedDelivery,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingExternalCommitPrimitive),
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::LegacyAuditDelivery,
        "delivery.legacy_audit_channel",
        EffectClass::DerivedDelivery,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::CacheAndConnectionInvalidation,
        "delivery.cache_connection_invalidation",
        EffectClass::DerivedDelivery,
        EffectOrigin::Helper,
        None,
        EnforceDisposition::Supported,
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::MediaLegacySidecar,
        "delivery.media_legacy_sidecar",
        EffectClass::DerivedDelivery,
        EffectOrigin::Helper,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::MediaUploadRecord,
        "delivery.media_upload_record",
        EffectClass::DerivedDelivery,
        EffectOrigin::Helper,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::GitLegacyPointer,
        "delivery.git_legacy_pointer",
        EffectClass::DerivedDelivery,
        EffectOrigin::Helper,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::AuthorizationReconciliation,
        "maintenance.authorization_reconciliation",
        EffectClass::SystemMaintenance,
        EffectOrigin::Background,
        None,
        EnforceDisposition::Supported,
        REQUEST,
    ),
    effect(
        EffectSurfaceId::PublicProjectionRetirement,
        "maintenance.public_projection_retirement",
        EffectClass::AuthoritativeMutation,
        EffectOrigin::Background,
        None,
        EnforceDisposition::Supported,
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::PublicProjectionRetirementDelivery,
        "delivery.public_projection_retirement",
        EffectClass::DerivedDelivery,
        EffectOrigin::Background,
        None,
        EnforceDisposition::Supported,
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::ClientStatusDelivery,
        "client.status.dedicated_delivery",
        EffectClass::DerivedDelivery,
        EffectOrigin::WebSocket,
        Some(AuthorizationCapability::CommunityRead),
        EnforceDisposition::Supported,
        REQUEST_EMIT,
    ),
    effect(
        EffectSurfaceId::StorageGarbageCollection,
        "maintenance.storage_gc",
        EffectClass::SystemMaintenance,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::ReminderWorker,
        "maintenance.reminder_worker",
        EffectClass::SystemMaintenance,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::PushWorker,
        "maintenance.push_worker",
        EffectClass::SystemMaintenance,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::DatabaseMaintenance,
        "maintenance.database",
        EffectClass::SystemMaintenance,
        EffectOrigin::Background,
        None,
        EnforceDisposition::DenyBeforeEffect(UnavailableReason::MissingBackgroundAuthority),
        REQUEST_COMMIT,
    ),
    effect(
        EffectSurfaceId::AudioMeshMaintenance,
        "maintenance.audio_mesh",
        EffectClass::SystemMaintenance,
        EffectOrigin::Background,
        None,
        EnforceDisposition::Supported,
        REQUEST,
    ),
    effect(
        EffectSurfaceId::MetricsObservation,
        "maintenance.metrics_observation",
        EffectClass::SystemMaintenance,
        EffectOrigin::Helper,
        None,
        EnforceDisposition::Supported,
        REQUEST,
    ),
];

/// Unforgeable proof that one registered effect is permitted in the selected mode.
pub struct EffectPermit {
    id: EffectSurfaceId,
}

impl EffectPermit {
    /// Registered effect represented by this permit.
    pub const fn id(&self) -> EffectSurfaceId {
        self.id
    }
}

/// Fail-closed effect classification error.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EffectPermitError {
    /// The closed identifier has no registry row.
    #[error("protected effect is not classified")]
    Unclassified,
    /// The registered effect is deliberately unavailable in protected mode.
    #[error("protected effect is unavailable in protected mode")]
    Unavailable(UnavailableReason),
}

/// Require a registered pre-effect permit for one exact activation mode.
pub fn require_effect_permit(
    mode: Option<AuthorizationMode>,
    id: EffectSurfaceId,
) -> Result<EffectPermit, EffectPermitError> {
    let surface = EFFECT_SURFACES
        .iter()
        .find(|surface| surface.id == id)
        .ok_or(EffectPermitError::Unclassified)?;
    if mode.is_some_and(AuthorizationMode::protects_surfaces) {
        if let EnforceDisposition::DenyBeforeEffect(reason) = surface.enforce {
            return Err(EffectPermitError::Unavailable(reason));
        }
    }
    Ok(EffectPermit { id })
}

/// Resolve only a registered stable helper name; unknown names never fall back.
pub fn effect_surface_by_name(name: &str) -> Option<&'static EffectSurface> {
    EFFECT_SURFACES.iter().find(|surface| surface.name == name)
}

/// Require a registered stable effect name. Unknown helper names preserve
/// legacy modes but map to the explicit unclassified-denial row in Enforce.
pub fn require_effect_name(
    mode: Option<AuthorizationMode>,
    name: &str,
) -> Result<EffectPermit, EffectPermitError> {
    let id = effect_surface_by_name(name)
        .map_or(EffectSurfaceId::UnclassifiedHelper, |surface| surface.id);
    require_effect_permit(mode, id)
}

/// One registered HTTP route and its protected-authorization contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpSurface {
    /// HTTP method as an uppercase token.
    pub method: &'static str,
    /// Axum matched-path template, never a literal untrusted path.
    pub matched_path: &'static str,
    /// Authorization classification.
    pub protection: SurfaceProtection,
    /// Mandatory lifetime checkpoints.
    pub guard_points: &'static [GuardPoint],
}

const fn http(
    method: &'static str,
    matched_path: &'static str,
    protection: SurfaceProtection,
    guard_points: &'static [GuardPoint],
) -> HttpSurface {
    HttpSurface {
        method,
        matched_path,
        protection,
        guard_points,
    }
}

const fn exempt(reason: SurfaceExemption) -> SurfaceProtection {
    SurfaceProtection::Exempt(reason)
}

/// Exhaustive inventory of API routes registered by the relay router.
///
/// Static UI fallbacks are recorded separately in [`NON_ROUTER_SURFACES`]
/// because they have no Axum `MatchedPath` and cannot reach an API handler.
pub const HTTP_SURFACES: &[HttpSurface] = &[
    http(
        "GET",
        "/",
        SurfaceProtection::ConditionalWebSocketUpgrade,
        SESSION_STREAM,
    ),
    http(
        "HEAD",
        "/",
        exempt(SurfaceExemption::PublicMetadata),
        REQUEST,
    ),
    http(
        "GET",
        "/info",
        exempt(SurfaceExemption::PublicMetadata),
        REQUEST,
    ),
    http(
        "GET",
        "/.well-known/nostr.json",
        exempt(SurfaceExemption::PublicMetadata),
        REQUEST,
    ),
    http(
        "GET",
        "/health",
        exempt(SurfaceExemption::HealthProbe),
        REQUEST,
    ),
    http(
        "GET",
        "/_liveness",
        exempt(SurfaceExemption::HealthProbe),
        REQUEST,
    ),
    http(
        "GET",
        "/_readiness",
        exempt(SurfaceExemption::HealthProbe),
        REQUEST,
    ),
    http(
        "GET",
        "/_status",
        exempt(SurfaceExemption::HealthProbe),
        REQUEST,
    ),
    http(
        "GET",
        "/_mesh",
        exempt(SurfaceExemption::HealthProbe),
        REQUEST,
    ),
    http(
        "POST",
        "/events",
        SurfaceProtection::DynamicAtomicMutation,
        REQUEST_COMMIT,
    ),
    http(
        "POST",
        "/query",
        SurfaceProtection::Capability(AuthorizationCapability::CommunityRead),
        REQUEST_EMIT,
    ),
    http(
        "POST",
        "/count",
        SurfaceProtection::Capability(AuthorizationCapability::CommunityRead),
        REQUEST_EMIT,
    ),
    http(
        "GET",
        "/operator/communities",
        exempt(SurfaceExemption::OperatorAuth),
        REQUEST,
    ),
    http(
        "POST",
        "/operator/communities",
        exempt(SurfaceExemption::OperatorAuth),
        REQUEST,
    ),
    http(
        "POST",
        "/operator/communities/archive",
        exempt(SurfaceExemption::OperatorAuth),
        REQUEST,
    ),
    http(
        "POST",
        "/operator/communities/unarchive",
        exempt(SurfaceExemption::OperatorAuth),
        REQUEST,
    ),
    http(
        "GET",
        "/operator/communities/availability",
        exempt(SurfaceExemption::OperatorAuth),
        REQUEST,
    ),
    http(
        "POST",
        "/operator/communities/transfer",
        exempt(SurfaceExemption::OperatorAuth),
        REQUEST,
    ),
    http(
        "POST",
        "/api/invites",
        SurfaceProtection::AtomicMutation(AuthorizationCapability::InviteMint),
        REQUEST_COMMIT,
    ),
    http(
        "POST",
        "/api/invites/claim",
        SurfaceProtection::AtomicMutation(AuthorizationCapability::InviteClaim),
        REQUEST_COMMIT,
    ),
    http(
        "GET",
        "/api/join-policy",
        exempt(SurfaceExemption::JoinBootstrap),
        REQUEST,
    ),
    http(
        "GET",
        "/api/join-policy/terms",
        exempt(SurfaceExemption::JoinBootstrap),
        REQUEST,
    ),
    http(
        "GET",
        "/api/join-policy/privacy",
        exempt(SurfaceExemption::JoinBootstrap),
        REQUEST,
    ),
    http(
        "POST",
        "/api/invites/accept-policy",
        exempt(SurfaceExemption::JoinBootstrap),
        REQUEST,
    ),
    http(
        "GET",
        "/moderation/reports",
        SurfaceProtection::Capability(AuthorizationCapability::Moderate),
        REQUEST_EMIT,
    ),
    http(
        "GET",
        "/moderation/audit",
        SurfaceProtection::Capability(AuthorizationCapability::Moderate),
        REQUEST_EMIT,
    ),
    http(
        "GET",
        "/moderation/restricted",
        SurfaceProtection::Capability(AuthorizationCapability::Moderate),
        REQUEST_EMIT,
    ),
    http(
        "POST",
        "/hooks/{id}",
        SurfaceProtection::AtomicMutationUnavailable(AuthorizationCapability::CommunityWrite),
        REQUEST_COMMIT,
    ),
    http(
        "POST",
        "/_mesh/demo/echo",
        exempt(SurfaceExemption::TestbedOnly),
        REQUEST,
    ),
    http(
        "POST",
        "/internal/git/policy",
        exempt(SurfaceExemption::LocalHookCallback),
        REQUEST,
    ),
    http(
        "GET",
        "/huddle/{channel_id}/audio",
        SurfaceProtection::LeasedSession {
            capability: AuthorizationCapability::AudioJoin,
        },
        SESSION_COMMIT_STREAM,
    ),
    http(
        "PUT",
        "/upload",
        SurfaceProtection::AtomicMutation(AuthorizationCapability::MediaWrite),
        REQUEST_COMMIT,
    ),
    http(
        "PUT",
        "/media/upload",
        SurfaceProtection::AtomicMutation(AuthorizationCapability::MediaWrite),
        REQUEST_COMMIT,
    ),
    http(
        "GET",
        "/media/{sha256_ext}",
        SurfaceProtection::ConditionalMediaRead(AuthorizationCapability::MediaRead),
        REQUEST_STREAM,
    ),
    http(
        "HEAD",
        "/media/{sha256_ext}",
        SurfaceProtection::ConditionalMediaRead(AuthorizationCapability::MediaRead),
        REQUEST_EMIT,
    ),
    http(
        "GET",
        "/git/{owner}/{repo}/info/refs",
        SurfaceProtection::DynamicCapability,
        REQUEST_STREAM,
    ),
    http(
        "POST",
        "/git/{owner}/{repo}/git-upload-pack",
        SurfaceProtection::Capability(AuthorizationCapability::GitRead),
        REQUEST_STREAM,
    ),
    http(
        "POST",
        "/git/{owner}/{repo}/git-receive-pack",
        SurfaceProtection::AtomicMutation(AuthorizationCapability::GitWrite),
        REQUEST_COMMIT,
    ),
    http(
        "GET",
        "/api/admin/v1/reports",
        exempt(SurfaceExemption::AdminAuth),
        REQUEST,
    ),
    http(
        "GET",
        "/api/admin/v1/reports/{id}",
        exempt(SurfaceExemption::AdminAuth),
        REQUEST,
    ),
    http(
        "GET",
        "/api/admin/v1/feedback",
        exempt(SurfaceExemption::AdminAuth),
        REQUEST,
    ),
    http(
        "GET",
        "/api/admin/v1/feedback/{id}",
        exempt(SurfaceExemption::AdminAuth),
        REQUEST,
    ),
    http(
        "GET",
        "/api/admin/v1/feedback/{id}/attachments/{sha256}",
        exempt(SurfaceExemption::AdminAuth),
        REQUEST_STREAM,
    ),
];

/// Surface outside Axum's matched-route inventory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NonRouterSurface {
    /// Stable surface name.
    pub name: &'static str,
    /// Explicit protection or exemption.
    pub protection: SurfaceProtection,
}

/// Explicit inventory for request fallbacks and other non-router surfaces.
pub const NON_ROUTER_SURFACES: &[NonRouterSurface] = &[NonRouterSurface {
    name: "static_ui_fallback",
    protection: SurfaceProtection::Exempt(SurfaceExemption::StaticUiFallback),
}];

/// Classify a registered method and trusted Axum matched-path template.
pub fn classify_http(method: &Method, matched_path: &str) -> Option<&'static HttpSurface> {
    let exact = HTTP_SURFACES
        .iter()
        .find(|surface| surface.method == method.as_str() && surface.matched_path == matched_path);
    if exact.is_some() || method != Method::HEAD {
        return exact;
    }
    HTTP_SURFACES
        .iter()
        .find(|surface| surface.method == "GET" && surface.matched_path == matched_path)
}

/// Whether any registered method names the matched template.
pub fn is_known_http_path(matched_path: &str) -> bool {
    HTTP_SURFACES
        .iter()
        .any(|surface| surface.matched_path == matched_path)
}

/// RFC 9110 `Allow` value for a known matched template.
pub fn allowed_http_methods(matched_path: &str) -> Option<String> {
    let mut methods = Vec::new();
    for surface in HTTP_SURFACES
        .iter()
        .filter(|surface| surface.matched_path == matched_path)
    {
        if !methods.contains(&surface.method) {
            methods.push(surface.method);
        }
        if surface.method == "GET" && !methods.contains(&"HEAD") {
            methods.push("HEAD");
        }
    }
    (!methods.is_empty()).then(|| methods.join(", "))
}

/// Protected WebSocket operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum WebSocketOperation {
    /// NIP-42 session bootstrap. It establishes identity but grants no data-plane capability.
    Auth,
    /// Historical or live subscription creation.
    Req,
    /// Aggregate query.
    Count,
    /// Persistent or ephemeral event ingest.
    Event {
        /// Nostr event kind used to select write or moderation authority.
        kind: u32,
    },
    /// Live event delivery to a subscription.
    Fanout,
}

/// One protocol-level WebSocket operation and its release/commit contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebSocketSurface {
    /// Stable low-cardinality operation label.
    pub operation: &'static str,
    /// Operation-level authorization classification.
    pub protection: SurfaceProtection,
    /// Required checks from dispatch through commit or socket emission.
    pub guard_points: &'static [GuardPoint],
}

/// Operation-level inventory for the WebSocket route.
///
/// The HTTP `/` row describes only upgrade/session lifetime. This table keeps
/// EVENT commit fencing distinct from REQ/COUNT/fanout release fencing.
pub const WEBSOCKET_SURFACES: &[WebSocketSurface] = &[
    WebSocketSurface {
        operation: "AUTH",
        protection: SurfaceProtection::Session { capability: None },
        guard_points: REQUEST,
    },
    WebSocketSurface {
        operation: "REQ",
        protection: SurfaceProtection::Capability(AuthorizationCapability::CommunityRead),
        guard_points: REQUEST_STREAM,
    },
    WebSocketSurface {
        operation: "COUNT",
        protection: SurfaceProtection::Capability(AuthorizationCapability::CommunityRead),
        guard_points: REQUEST_EMIT,
    },
    WebSocketSurface {
        operation: "EVENT",
        protection: SurfaceProtection::DynamicAtomicMutation,
        guard_points: REQUEST_COMMIT,
    },
    WebSocketSurface {
        operation: "fanout",
        protection: SurfaceProtection::Capability(AuthorizationCapability::CommunityRead),
        guard_points: REQUEST_STREAM,
    },
];

/// Return the exact capability for a WebSocket operation.
///
/// AUTH deliberately returns `None`: verification or authentication alone must
/// never become data-plane authority. Every subsequent operation requests its
/// own capability through the same direct/delegated runtime path.
pub const fn websocket_capability(
    operation: WebSocketOperation,
) -> Option<AuthorizationCapability> {
    match operation {
        WebSocketOperation::Auth => None,
        WebSocketOperation::Req | WebSocketOperation::Count | WebSocketOperation::Fanout => {
            Some(AuthorizationCapability::CommunityRead)
        }
        WebSocketOperation::Event { kind: 9040..=9044 } => Some(AuthorizationCapability::Moderate),
        WebSocketOperation::Event { .. } => Some(AuthorizationCapability::CommunityWrite),
    }
}

/// Return the exact HTTP bridge event-ingest capability.
pub const fn event_ingest_capability(kind: u32) -> AuthorizationCapability {
    match kind {
        9040..=9044 => AuthorizationCapability::Moderate,
        _ => AuthorizationCapability::CommunityWrite,
    }
}

/// Resolve Git's `info/refs` capability from the server-validated service.
pub fn git_info_refs_capability(service: &str) -> Option<AuthorizationCapability> {
    match service {
        "git-upload-pack" => Some(AuthorizationCapability::GitRead),
        "git-receive-pack" => Some(AuthorizationCapability::GitWrite),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn effect_inventory_is_closed_unique_and_guarded() {
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for surface in EFFECT_SURFACES {
            assert!(
                ids.insert(surface.id),
                "duplicate effect id: {:?}",
                surface.id
            );
            assert!(
                names.insert(surface.name),
                "duplicate effect name: {}",
                surface.name
            );
            assert!(!surface.name.is_empty());
            assert!(!surface.guard_points.is_empty());
            if surface.class == EffectClass::AuthoritativeMutation
                && surface.enforce == EnforceDisposition::Supported
                && surface.id != EffectSurfaceId::AudioAdmission
            {
                assert!(surface.guard_points.contains(&GuardPoint::PreCommit));
            }
            if surface.class == EffectClass::DerivedDelivery
                && surface.enforce == EnforceDisposition::Supported
            {
                assert!(surface.guard_points.contains(&GuardPoint::PreEmission));
            }
            assert_eq!(effect_surface_by_name(surface.name), Some(surface));
        }
    }

    #[test]
    fn event_effect_classification_separates_transactional_and_unavailable_paths() {
        for kind in [
            buzz_core::kind::KIND_TEXT_NOTE,
            buzz_core::kind::KIND_REACTION,
            buzz_core::kind::KIND_DELETION,
            buzz_core::kind::KIND_REPORT,
            buzz_core::kind::KIND_LONG_FORM,
            buzz_core::kind::KIND_MODERATION_BAN,
            buzz_core::kind::KIND_GIT_REPO_ANNOUNCEMENT,
            buzz_core::kind::KIND_PROFILE,
            buzz_core::kind::KIND_AGENT_PROFILE,
            buzz_core::kind::KIND_PRODUCT_FEEDBACK,
            buzz_core::kind::KIND_NIP43_LEAVE_REQUEST,
            buzz_core::kind::KIND_IA_ARCHIVE_REQUEST,
            buzz_core::kind::KIND_IA_UNARCHIVE_REQUEST,
            buzz_core::kind::RELAY_ADMIN_ADD_MEMBER,
            buzz_core::kind::RELAY_ADMIN_REMOVE_MEMBER,
            buzz_core::kind::RELAY_ADMIN_CHANGE_ROLE,
            buzz_core::kind::RELAY_ADMIN_SET_WORKSPACE_PROFILE,
        ] {
            assert_eq!(
                event_mutation_disposition(kind),
                EventMutationDisposition::TransactionalPersistence,
                "interactive kind {kind} must retain transaction-owned persistence"
            );
        }
        for kind in [
            buzz_core::kind::KIND_NIP29_PUT_USER,
            buzz_core::kind::KIND_NIP29_REMOVE_USER,
            buzz_core::kind::KIND_NIP29_EDIT_METADATA,
            buzz_core::kind::KIND_NIP29_DELETE_EVENT,
            buzz_core::kind::KIND_NIP29_CREATE_GROUP,
            buzz_core::kind::KIND_NIP29_DELETE_GROUP,
            buzz_core::kind::KIND_NIP29_JOIN_REQUEST,
            buzz_core::kind::KIND_NIP29_LEAVE_REQUEST,
            buzz_core::kind::KIND_NIP29_CREATE_INVITE,
        ] {
            assert_eq!(
                event_mutation_disposition(kind),
                EventMutationDisposition::TransactionalPersistence
            );
        }
        assert_eq!(
            event_mutation_disposition(buzz_core::kind::KIND_WORKFLOW_TRIGGER),
            EventMutationDisposition::UnavailableCommandOrWorkflow
        );
    }

    #[test]
    fn enforce_denies_background_webhooks_and_automatic_membership_before_effect() {
        for id in [
            EffectSurfaceId::WorkflowBackgroundExecution,
            EffectSurfaceId::OutboundWebhook,
            EffectSurfaceId::UnclassifiedHelper,
            EffectSurfaceId::AudioAutomaticMembership,
            EffectSurfaceId::ReminderWorker,
            EffectSurfaceId::PushWorker,
            EffectSurfaceId::LegacyAuditDelivery,
        ] {
            assert!(require_effect_permit(Some(AuthorizationMode::Enforce), id).is_err());
            assert!(require_effect_permit(Some(AuthorizationMode::Off), id).is_ok());
            assert!(require_effect_permit(Some(AuthorizationMode::Shadow), id).is_ok());
            assert!(require_effect_permit(Some(AuthorizationMode::VerifyOnly), id).is_ok());
            assert!(require_effect_permit(None, id).is_ok());
        }
    }

    #[test]
    fn unknown_helper_name_has_no_enforce_fallback() {
        assert!(effect_surface_by_name("helper.added_without_classification").is_none());
        assert!(require_effect_name(
            Some(AuthorizationMode::Enforce),
            "helper.added_without_classification"
        )
        .is_err());
        assert!(require_effect_name(
            Some(AuthorizationMode::Off),
            "helper.added_without_classification"
        )
        .is_ok());
    }

    fn assert_ordered(source: &str, first: &str, second: &str) {
        let first_index = source.find(first).expect("first boundary exists");
        let second_index = source.find(second).expect("second boundary exists");
        assert!(first_index < second_index, "{first} must precede {second}");
    }

    #[test]
    fn inventory_keys_are_unique_and_every_row_has_guards() {
        let mut seen = HashSet::new();
        for surface in HTTP_SURFACES {
            assert!(
                seen.insert((surface.method, surface.matched_path)),
                "duplicate protected-surface row for {} {}",
                surface.method,
                surface.matched_path
            );
            assert!(!surface.guard_points.is_empty());
        }
        for surface in WEBSOCKET_SURFACES {
            assert!(seen.insert(("WS", surface.operation)));
            assert!(!surface.guard_points.is_empty());
        }
    }

    #[test]
    fn dynamic_operations_request_exact_capabilities() {
        assert_eq!(websocket_capability(WebSocketOperation::Auth), None);
        assert_eq!(
            websocket_capability(WebSocketOperation::Req),
            Some(AuthorizationCapability::CommunityRead)
        );
        assert_eq!(
            event_ingest_capability(9042),
            AuthorizationCapability::Moderate
        );
        assert_eq!(
            event_ingest_capability(1),
            AuthorizationCapability::CommunityWrite
        );
        assert!(protected_operation_matches(
            "invite.claim",
            AuthTransport::HttpBridge,
            AuthorizationCapability::InviteClaim,
        ));
        assert!(!protected_operation_matches(
            "invite.claim",
            AuthTransport::HttpBridge,
            AuthorizationCapability::InviteMint,
        ));
    }

    #[test]
    fn conditional_and_session_roots_are_explicit() {
        assert!(matches!(
            classify_http(&Method::GET, "/").map(|surface| surface.protection),
            Some(SurfaceProtection::ConditionalWebSocketUpgrade)
        ));
        assert!(matches!(
            classify_http(&Method::HEAD, "/").map(|surface| surface.protection),
            Some(SurfaceProtection::Exempt(SurfaceExemption::PublicMetadata))
        ));
        assert!(matches!(
            classify_http(&Method::GET, "/huddle/{channel_id}/audio")
                .map(|surface| surface.protection),
            Some(SurfaceProtection::LeasedSession {
                capability: AuthorizationCapability::AudioJoin
            })
        ));
        assert!(matches!(
            classify_http(&Method::HEAD, "/media/{sha256_ext}").map(|surface| surface.protection),
            Some(SurfaceProtection::ConditionalMediaRead(
                AuthorizationCapability::MediaRead
            ))
        ));
    }

    #[test]
    fn exemptions_are_named_and_unknown_routes_fail_classification() {
        assert!(matches!(
            classify_http(&Method::GET, "/_readiness").map(|surface| surface.protection),
            Some(SurfaceProtection::Exempt(SurfaceExemption::HealthProbe))
        ));
        assert!(classify_http(&Method::GET, "/unclassified").is_none());
        assert!(classify_http(&Method::GET, "/media/literal-sha").is_none());
        assert_eq!(NON_ROUTER_SURFACES.len(), 1);
        assert!(matches!(
            NON_ROUTER_SURFACES[0].protection,
            SurfaceProtection::Exempt(SurfaceExemption::StaticUiFallback)
        ));
    }

    #[test]
    fn git_advertisement_service_selects_read_or_write() {
        assert_eq!(
            git_info_refs_capability("git-upload-pack"),
            Some(AuthorizationCapability::GitRead)
        );
        assert_eq!(
            git_info_refs_capability("git-receive-pack"),
            Some(AuthorizationCapability::GitWrite)
        );
        assert_eq!(git_info_refs_capability("unknown"), None);
    }

    #[test]
    fn every_mutating_route_declares_its_authoritative_or_unavailable_boundary() {
        for (method, path) in [
            (Method::POST, "/events"),
            (Method::POST, "/api/invites"),
            (Method::POST, "/api/invites/claim"),
        ] {
            let protection = classify_http(&method, path)
                .expect("mutating route is inventoried")
                .protection;
            assert!(matches!(
                protection,
                SurfaceProtection::AtomicMutation(_) | SurfaceProtection::DynamicAtomicMutation
            ));
        }
        assert!(matches!(
            classify_http(&Method::POST, "/hooks/{id}")
                .expect("webhook is inventoried")
                .protection,
            SurfaceProtection::AtomicMutationUnavailable(_)
        ));
        for (method, path) in [
            (Method::PUT, "/upload"),
            (Method::PUT, "/media/upload"),
            (Method::POST, "/git/{owner}/{repo}/git-receive-pack"),
        ] {
            assert!(matches!(
                classify_http(&method, path)
                    .expect("functional mutation is inventoried")
                    .protection,
                SurfaceProtection::AtomicMutation(_)
            ));
        }
        assert!(matches!(
            classify_http(&Method::GET, "/huddle/{channel_id}/audio")
                .expect("audio is inventoried")
                .protection,
            SurfaceProtection::LeasedSession { .. }
        ));
        assert!(WEBSOCKET_SURFACES.iter().any(|surface| {
            surface.operation == "EVENT"
                && surface.protection == SurfaceProtection::DynamicAtomicMutation
                && surface.guard_points.contains(&GuardPoint::PreCommit)
        }));
    }

    #[test]
    fn enforce_mutation_gates_precede_every_shared_business_effect_boundary() {
        let ingest = include_str!("handlers/ingest.rs");
        assert_ordered(
            ingest,
            "event_mutation_disposition",
            "handle_moderation_command",
        );
        assert_ordered(
            ingest,
            "begin_authorized_operation",
            "insert_event_with_thread_metadata_tx",
        );
        assert_ordered(
            ingest,
            "begin_authorized_operation",
            "apply_nip29_mutation_tx",
        );
        let report_path = ingest
            .split_once("if kind_u32 == KIND_REPORT")
            .expect("report path exists")
            .1;
        assert_ordered(
            report_path,
            "handle_report_event_enforced",
            "return Ok(IngestResult",
        );
        let moderation_path = ingest
            .split_once("if buzz_core::kind::is_moderation_command_kind(kind_u32)")
            .expect("moderation path exists")
            .1;
        assert_ordered(
            moderation_path,
            "handle_moderation_command_enforced",
            "return Ok(IngestResult",
        );
        let feedback_path = ingest
            .split_once("if kind_u32 == KIND_PRODUCT_FEEDBACK")
            .expect("feedback path exists")
            .1;
        assert_ordered(
            feedback_path,
            "handle_enforced(tenant, &event, state, &protected)",
            "emit_product_feedback_success",
        );
        let relay_admin = include_str!("handlers/relay_admin.rs");
        let enforced_relay_admin = relay_admin
            .split_once("handle_relay_admin_event_enforced")
            .expect("protected relay-admin executor exists")
            .1;
        assert_ordered(
            enforced_relay_admin,
            "begin_authorized_operation",
            "execute_relay_admin_command_tx",
        );
        let identity_archive = include_str!("handlers/identity_archive.rs");
        let enforced_archive = identity_archive
            .split_once("handle_identity_archive_event_tx")
            .expect("protected archive transaction exists")
            .1;
        assert_ordered(enforced_archive, "determine_consent_path_tx", "archive_tx");
        let relay_leave = ingest
            .split_once("if kind_u32 == KIND_NIP43_LEAVE_REQUEST")
            .expect("relay leave path exists")
            .1;
        assert_ordered(
            relay_leave,
            "begin_authorized_operation",
            "remove_relay_member_tx",
        );
        assert_ordered(
            ingest,
            "begin_authorized_operation",
            "replace_protected_announcement_tx",
        );
        let media = include_str!("api/media.rs");
        assert_ordered(
            media,
            "fn acquire_protected_upload_permit(",
            "if upload_rate_limited(",
        );
        assert_ordered(
            media,
            "fn acquire_protected_upload_permit(",
            "acquire_upload_permit(state, community_id, pubkey)",
        );
        let media_upload = media
            .split_once("pub async fn upload_blob")
            .expect("media upload handler exists")
            .1;
        assert_ordered(
            media_upload,
            "commit_media_publication(",
            "Ok(Json(descriptor))",
        );

        let git = include_str!("api/git/transport.rs");
        let finalize_push = git
            .split_once("async fn finalize_push")
            .expect("Git push finalizer exists")
            .1;
        assert_ordered(
            finalize_push,
            "begin_authorized_operation(",
            "compare_and_publish_git(",
        );
        let after_commit = finalize_push
            .split_once("operation.commit(&payload)")
            .expect("PostgreSQL Git publication commits a receipt")
            .1;
        assert!(after_commit.contains("build_git_response(\"receive-pack\""));

        let bridge = include_str!("api/bridge.rs");
        let webhook = bridge
            .split_once("pub async fn workflow_webhook")
            .expect("workflow webhook handler exists")
            .1;
        assert_ordered(webhook, "require_effect_permit(", ".get_workflow(");
        assert_ordered(webhook, "require_effect_permit(", ".create_workflow_run(");
        assert_ordered(
            include_str!("workflow_sink.rs"),
            "require_effect_permit(",
            ".insert_event_with_thread_metadata(",
        );

        let workflow_engine = include_str!("../../buzz-workflow/src/lib.rs");
        let on_event = workflow_engine
            .split_once("pub async fn on_event")
            .expect("event workflow trigger exists")
            .1;
        assert_ordered(on_event, "self.require_mutation(", ".create_workflow_run(");
        let scheduler = workflow_engine
            .split_once("pub async fn run")
            .expect("workflow scheduler exists")
            .1;
        assert_ordered(
            scheduler,
            "self.require_mutation(",
            ".claim_scheduled_workflow_fire(",
        );
        assert_ordered(scheduler, "self.require_mutation(", ".create_workflow_run(");

        let workflow_executor = include_str!("../../buzz-workflow/src/executor.rs");
        let action_dispatch = workflow_executor
            .split_once("pub async fn dispatch_action")
            .expect("workflow action dispatcher exists")
            .1;
        assert_ordered(
            action_dispatch,
            "engine.require_mutation(",
            "add_reaction_impl(",
        );
        assert_ordered(
            action_dispatch,
            "engine.require_outbound_webhook(",
            "call_webhook_impl(",
        );

        let relay_main = include_str!("main.rs");
        assert_ordered(relay_main, "set_mutation_gate(", "wf_cron.run(");
        assert_ordered(
            relay_main,
            "enforcing_protected_domain_ids()",
            "reap_expired_ephemeral_channels_excluding",
        );
        assert_ordered(
            relay_main,
            "enforcing_protected_domain_ids()",
            "query_due_reminders_excluding",
        );

        let push = include_str!("push_runtime.rs");
        assert_ordered(
            push,
            "enforcing_protected_domain_ids()",
            "claim_due_push_match_batch_excluding",
        );
        assert_ordered(push, "is_protected_enforcing", "claim_due_push_wakes");

        let websocket = include_str!("handlers/event.rs");
        let ephemeral = websocket
            .split_once("async fn handle_ephemeral_event")
            .expect("ephemeral event handler exists")
            .1;
        assert_ordered(ephemeral, "authority.revalidate()", ".publish_event(");
        assert_ordered(
            ephemeral,
            "authority.revalidate()",
            "fan_out_event_to_local_subscribers",
        );
        assert!(ephemeral.contains("fan_out_event_to_local_subscribers_with_authority"));
        let observer = websocket
            .split_once("async fn handle_agent_observer_event")
            .expect("observer event handler exists")
            .1;
        assert_ordered(observer, "authority.revalidate()", ".publish_event(");
        assert_ordered(
            observer,
            "authority.revalidate()",
            "fan_out_event_to_local_subscribers",
        );
        assert!(observer.contains("fan_out_event_to_local_subscribers_with_authority"));
        let presence = websocket
            .split_once("if event_kind_u32(&event) == KIND_PRESENCE_UPDATE")
            .expect("presence effect boundary exists")
            .1;
        assert_ordered(presence, "encode_presence(", ".set_presence(");
        assert!(websocket.contains("send_to_text_bytes_guarded_pair"));
        assert_ordered(
            websocket,
            "if legacy_audit_delivery_allowed",
            "enqueue_event_created_audit(",
        );
        let connection = include_str!("connection.rs");
        assert!(connection.contains("CombinedReleaseFence"));
        assert!(connection.contains("self.sender.release().await"));
        assert!(connection.contains("self.recipient.release().await"));

        let presence_read = bridge
            .split_once("async fn synthesize_presence")
            .expect("presence read boundary exists")
            .1;
        assert_ordered(presence_read, "decode_presence(", "verify_actor_context(");
        assert_ordered(
            presence_read,
            "verify_actor_context(",
            "presence_map.insert(",
        );

        let invites = include_str!("api/invites.rs");
        let mint = invites
            .split_once("pub async fn mint_invite")
            .expect("invite mint handler exists")
            .1;
        assert_ordered(mint, "begin_authorized_operation", "mint_relay_invite_tx");
        let claim = invites
            .split_once("pub async fn claim_invite")
            .expect("invite claim handler exists")
            .1;
        assert_ordered(
            claim,
            "begin_authorized_enrollment",
            "claim_relay_invite_with_identity_tx",
        );

        assert_ordered(
            include_str!("audio/handler.rs"),
            "debug_assert!(!protected_authority.is_enforcing())",
            ".add_member_with_identity(",
        );

        let corporate_identity = include_str!("corporate_identity.rs")
            .split_once("async fn record_identity_binding_audit")
            .expect("identity audit helper exists")
            .1;
        assert_ordered(
            corporate_identity,
            "require_effect_permit(",
            "let Some(audit_tx)",
        );

        let media_audit = media
            .split_once("// Audit via bounded channel")
            .expect("media audit boundary exists")
            .1;
        assert_ordered(media_audit, "require_effect_permit(", "audit_tx");
    }
}
