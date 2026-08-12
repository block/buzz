//! Blossom-compatible media upload, retrieval, and existence check handlers.
//!
//! Routes:
//!   PUT  /upload                — BUD-02 exact-byte upload (auth required)
//!   PUT  /media/upload          — temporary media-only legacy alias
//!   GET  /media/{sha256_ext}    — BUD-01 serve blob
//!   HEAD /media/{sha256_ext}    — BUD-01 existence check

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::header;
use axum::{
    extract::{FromRequestParts, Path, State},
    http::{request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine;
use buzz_audit::{AuditAction, NewAuditEntry};
use buzz_auth::{BlossomAuthorizationVerb, RouteCapability, SealedTransportEvidence};
use buzz_core::tenant::TenantContext;
use buzz_db::authorization_admission::{
    AdmissionApplicationContext, AdmissionApplicationEffect, AdmissionApplicationOutcome,
    AdmissionApplicationResult, AdmissionApplicationResultSchema, AdmissionCommitError,
    AdmissionCommitOutcome, AdmissionCommitReceipt, AdmissionCommitRequest, AdmissionObject,
    AdmissionObjectKind, AdmissionReplayClaim, AdmissionReplayClaimKind,
    CanonicalAdmissionCommitter,
};
use buzz_media::{
    BlobDescriptor, MediaError, MediaStorage, StagedMediaUpload, UploadAttribution,
    UploadNetworkInfo,
};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

use crate::api::git::cas_publish::StagedGitPublication;
use crate::state::AppState;

/// Installs origin-sealed transport replay state onto S3's sole admission
/// request without creating an alternate admission or lifecycle authority.
///
/// S5 calls this only after canonical assertion/proof preparation. Consuming
/// the move-only evidence ensures confidential assertion bytes do not survive
/// beyond installation, while S3 remains responsible for atomic claim use.
#[derive(Debug, Default)]
pub struct ProtectedTransportInstaller;

impl ProtectedTransportInstaller {
    /// Bind the trusted-proxy nonce claim to the exact server-owned admission
    /// domain. Domain drift fails before canonical admission performs I/O.
    pub fn install(
        request: AdmissionCommitRequest,
        evidence: SealedTransportEvidence,
    ) -> Result<AdmissionCommitRequest, AdmissionCommitError> {
        let (request_transport, request_transport_context) = request.transport_binding();
        let claim = verified_transport_replay_claim(
            request.authorization_domain(),
            request.request_fingerprint(),
            request_transport,
            request_transport_context,
            &evidence,
        )?;
        Ok(request.with_replay_claim(claim))
    }
}

fn verified_transport_replay_claim(
    authorization_domain: buzz_core::CommunityId,
    request_fingerprint: [u8; 32],
    transport: buzz_auth::ProofTransport,
    transport_context_fingerprint: [u8; 32],
    evidence: &SealedTransportEvidence,
) -> Result<AdmissionReplayClaim, AdmissionCommitError> {
    if authorization_domain != evidence.authorization_domain()
        || request_fingerprint != *evidence.request_fingerprint()
        || transport != evidence.transport()
        || transport_context_fingerprint != *evidence.transport_context_fingerprint()
    {
        return Err(AdmissionCommitError::InvalidRequest);
    }
    AdmissionReplayClaim::new(
        AdmissionReplayClaimKind::TrustedProxyNonce,
        *evidence.nonce_claim().claim_key(),
        evidence.nonce_claim().retain_until(),
    )
}

/// Typed server-owned publication coordinate used by canonical admission.
///
/// The opaque object key is derived from the authorization domain and exact
/// target coordinates. Repository targets bind both owner and canonical repo;
/// media targets bind the canonical blob digest. This prevents two plans with
/// byte-identical artifact manifests from sharing an admission result.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ProtectedPublicationTarget {
    authorization_domain: buzz_core::CommunityId,
    object: AdmissionObject,
}

impl ProtectedPublicationTarget {
    /// Derive the canonical repository object from exact server-resolved path
    /// coordinates.
    pub fn repository(
        authorization_domain: buzz_core::CommunityId,
        owner: &str,
        repo: &str,
    ) -> Result<Self, AdmissionCommitError> {
        let canonical_repo = repo.strip_suffix(".git").unwrap_or(repo);
        if authorization_domain.as_uuid().is_nil()
            || owner.len() != 64
            || !owner
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || canonical_repo.is_empty()
            || canonical_repo.len() > 64
            || canonical_repo.starts_with('.')
            || canonical_repo.contains("..")
            || !canonical_repo
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Self::derive(
            authorization_domain,
            AdmissionObjectKind::Repository,
            b"buzz:nip-fi:repository-target:v1",
            &[owner.as_bytes(), canonical_repo.as_bytes()],
        )
    }

    /// Derive the canonical media object from an exact lower-hex blob digest.
    pub fn media(
        authorization_domain: buzz_core::CommunityId,
        blob_sha256: &str,
    ) -> Result<Self, AdmissionCommitError> {
        if authorization_domain.as_uuid().is_nil()
            || blob_sha256.len() != 64
            || !blob_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Self::derive(
            authorization_domain,
            AdmissionObjectKind::Media,
            b"buzz:nip-fi:media-target:v1",
            &[blob_sha256.as_bytes()],
        )
    }

    fn derive(
        authorization_domain: buzz_core::CommunityId,
        kind: AdmissionObjectKind,
        domain: &[u8],
        coordinates: &[&[u8]],
    ) -> Result<Self, AdmissionCommitError> {
        let mut fields = Vec::with_capacity(coordinates.len() + 1);
        fields.push(authorization_domain.as_uuid().as_bytes().as_slice());
        fields.extend_from_slice(coordinates);
        let key = framed_digest(domain, &fields);
        let object = AdmissionObject::new(kind, key).ok_or(AdmissionCommitError::InvalidRequest)?;
        Ok(Self {
            authorization_domain,
            object,
        })
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(self) -> buzz_core::CommunityId {
        self.authorization_domain
    }

    /// Closed kind and opaque key supplied to canonical admission.
    pub const fn admission_object(self) -> AdmissionObject {
        self.object
    }

    /// Derive the application result committed by canonical admission from an
    /// immutable artifact-manifest digest and this exact server-owned target.
    pub fn bind_artifact_result(self, artifact_digest: [u8; 32]) -> [u8; 32] {
        framed_digest(
            b"buzz:nip-fi:protected-publication-result:v1",
            &[
                self.authorization_domain.as_uuid().as_bytes(),
                &[self.object.kind().database_code() as u8],
                self.object.key(),
                &artifact_digest,
            ],
        )
    }

    /// Exact typed application result persisted by canonical admission.
    pub fn application_result(
        self,
        artifact_digest: [u8; 32],
    ) -> Result<AdmissionApplicationResult, AdmissionCommitError> {
        publication_application_result(self, artifact_digest)
    }

    /// Validate a storage-issued receipt and typed result against this exact
    /// target and immutable artifact manifest.
    pub fn validate_receipt(
        self,
        artifact_digest: [u8; 32],
        application_intent_digest: [u8; 32],
        admission: AdmissionCommitReceipt,
        application_result: &AdmissionApplicationResult,
    ) -> Result<(), AdmissionCommitError> {
        validate_publication_binding(
            self,
            artifact_digest,
            application_intent_digest,
            admission,
            application_result,
        )
    }
}

impl std::fmt::Debug for ProtectedPublicationTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProtectedPublicationTarget([REDACTED])")
    }
}

/// Closed immutable publication staged before S3 canonical admission.
///
/// Neither variant is visible until an exact applied admission receipt is
/// bound by [`ProtectedPublicationReceipt::bind`]. Raw object-store pointers
/// and media sidecars remain post-commit projections only.
pub enum ProtectedPublicationPlan {
    /// Media manifest, blob artifacts, and deterministic moderation plan.
    Media(Box<StagedMediaUpload>),
    /// Git manifest and content-addressed pack artifacts.
    Git(Box<StagedGitPublication>),
}

impl ProtectedPublicationPlan {
    /// Exact server-owned target bound to this plan.
    pub fn target(&self) -> Result<ProtectedPublicationTarget, AdmissionCommitError> {
        match self {
            Self::Media(staged) => ProtectedPublicationTarget::media(
                staged.publication().manifest().community_id(),
                staged.publication().manifest().blob_sha256(),
            ),
            Self::Git(staged) => ProtectedPublicationTarget::repository(
                staged.community_id(),
                staged.owner(),
                staged.repo(),
            ),
        }
    }

    /// Raw immutable artifact-manifest digest before target binding.
    pub fn artifact_digest(&self) -> [u8; 32] {
        match self {
            Self::Media(staged) => staged.publication().result_digest(),
            Self::Git(staged) => staged.result_digest(),
        }
    }

    /// Target-bound application result that canonical admission must persist.
    pub fn result_digest(&self) -> Result<[u8; 32], AdmissionCommitError> {
        Ok(self.target()?.bind_artifact_result(self.artifact_digest()))
    }

    /// Build the bounded application mutation installed on canonical admission.
    ///
    /// The staged plan remains with the caller for receipt binding and
    /// post-commit projection. The effect retains only credential-free target,
    /// immutable object-store, and deterministic outbox coordinates.
    pub fn application_effect(
        &self,
    ) -> Result<ProtectedPublicationApplicationEffect, AdmissionCommitError> {
        let target = self.target()?;
        let artifact_digest = self.artifact_digest();
        let projections = match self {
            Self::Media(staged) => {
                let publication = staged.publication();
                let manifest = publication.manifest();
                let mut projections = vec![ProtectedProjectionPlan {
                    kind: 2,
                    projection_key: MediaStorage::sidecar_key(
                        target.authorization_domain(),
                        manifest.blob_sha256(),
                    ),
                    staged_object_key: publication.manifest_key().to_owned(),
                    payload_digest: publication.result_digest(),
                }];
                if let Some(record) = staged.moderation_projection() {
                    projections.push(ProtectedProjectionPlan {
                        kind: 1,
                        projection_key: record.projection_key().to_owned(),
                        staged_object_key: record.plan_key().to_owned(),
                        payload_digest: record.digest(),
                    });
                }
                projections
            }
            Self::Git(staged) => vec![ProtectedProjectionPlan {
                kind: 3,
                projection_key: staged.pointer_cache_key(),
                staged_object_key: staged.manifest_key().to_owned(),
                payload_digest: staged.result_digest(),
            }],
        };
        ProtectedPublicationApplicationEffect::new(target, artifact_digest, projections)
    }
}

impl std::fmt::Debug for ProtectedPublicationPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProtectedPublicationPlan([REDACTED])")
    }
}

impl From<StagedMediaUpload> for ProtectedPublicationPlan {
    fn from(staged: StagedMediaUpload) -> Self {
        Self::Media(Box::new(staged))
    }
}

impl From<StagedGitPublication> for ProtectedPublicationPlan {
    fn from(staged: StagedGitPublication) -> Self {
        Self::Git(Box::new(staged))
    }
}

const PROTECTED_PUBLICATION_RESULT_CODE: i16 = 1;

#[derive(Debug)]
struct ProtectedProjectionPlan {
    kind: i16,
    projection_key: String,
    staged_object_key: String,
    payload_digest: [u8; 32],
}

/// Credential-free publication DML executed inside canonical final admission.
///
/// All outbox coordinates are derived from immutable staged artifacts. The
/// canonical transaction supplies domain, object, operation, request, schema,
/// and result binding; callers cannot substitute any of those coordinates.
pub struct ProtectedPublicationApplicationEffect {
    target: ProtectedPublicationTarget,
    artifact_digest: [u8; 32],
    projections: Vec<ProtectedProjectionPlan>,
    intent_digest: [u8; 32],
}

impl ProtectedPublicationApplicationEffect {
    fn new(
        target: ProtectedPublicationTarget,
        artifact_digest: [u8; 32],
        projections: Vec<ProtectedProjectionPlan>,
    ) -> Result<Self, AdmissionCommitError> {
        if artifact_digest == [0; 32] || projections.is_empty() {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        let intent_digest = publication_intent_digest(target, artifact_digest, &projections);
        if intent_digest == [0; 32] {
            return Err(AdmissionCommitError::InvalidRequest);
        }
        Ok(Self {
            target,
            artifact_digest,
            projections,
            intent_digest,
        })
    }
}

impl std::fmt::Debug for ProtectedPublicationApplicationEffect {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProtectedPublicationApplicationEffect([REDACTED])")
    }
}

impl AdmissionApplicationEffect for ProtectedPublicationApplicationEffect {
    fn intent_digest(&self) -> [u8; 32] {
        self.intent_digest
    }

    fn result_schema(&self) -> AdmissionApplicationResultSchema {
        publication_result_schema()
    }

    fn apply<'a, 'transaction>(
        &'a mut self,
        transaction: &'a mut Transaction<'transaction, Postgres>,
        context: &'a AdmissionApplicationContext<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<AdmissionApplicationOutcome, AdmissionCommitError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if context.authorization_domain() != self.target.authorization_domain()
                || context.object() != self.target.admission_object()
            {
                return Err(AdmissionCommitError::AuthorizationDenied);
            }

            let result = publication_application_result(self.target, self.artifact_digest)?;
            let effect_digest = publication_effect_digest(context, self.intent_digest);
            let outcome = AdmissionApplicationOutcome::new(result, effect_digest)?;
            let publication_result_digest = context.canonical_result_digest(&outcome)?;
            let schema = outcome.result().schema();

            for projection in &self.projections {
                sqlx::query(
                    "INSERT INTO protected_publication_projection_outbox \
                     (community_id, operation_id, request_fingerprint, \
                      publication_result_digest, object_kind, object_key, \
                      application_type, application_version, application_effect_digest, \
                      projection_kind, projection_key, staged_object_key, payload_digest) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                )
                .bind(context.authorization_domain().as_uuid())
                .bind(context.operation_id())
                .bind(context.request_fingerprint().as_slice())
                .bind(publication_result_digest.as_slice())
                .bind(context.object().kind().database_code())
                .bind(context.object().key().as_slice())
                .bind(schema.type_key().as_slice())
                .bind(
                    i16::try_from(schema.version())
                        .map_err(|_| AdmissionCommitError::InvalidRequest)?,
                )
                .bind(effect_digest.as_slice())
                .bind(projection.kind)
                .bind(&projection.projection_key)
                .bind(&projection.staged_object_key)
                .bind(projection.payload_digest.as_slice())
                .execute(&mut **transaction)
                .await
                .map_err(|_| AdmissionCommitError::DependencyUnavailable)?;
            }
            Ok(outcome)
        })
    }
}

fn publication_result_schema() -> AdmissionApplicationResultSchema {
    AdmissionApplicationResultSchema::protected_publication()
}

fn publication_application_result(
    target: ProtectedPublicationTarget,
    artifact_digest: [u8; 32],
) -> Result<AdmissionApplicationResult, AdmissionCommitError> {
    let mut payload = Vec::with_capacity(83);
    payload.push(1);
    payload.extend_from_slice(target.authorization_domain().as_uuid().as_bytes());
    payload.extend_from_slice(
        &target
            .admission_object()
            .kind()
            .database_code()
            .to_be_bytes(),
    );
    payload.extend_from_slice(target.admission_object().key());
    payload.extend_from_slice(&artifact_digest);
    AdmissionApplicationResult::new(
        publication_result_schema(),
        PROTECTED_PUBLICATION_RESULT_CODE,
        payload,
    )
}

fn publication_intent_digest(
    target: ProtectedPublicationTarget,
    artifact_digest: [u8; 32],
    projections: &[ProtectedProjectionPlan],
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"buzz:nip-fi:protected-publication-effect-intent:v1");
    for field in [
        target
            .authorization_domain()
            .as_uuid()
            .as_bytes()
            .as_slice(),
        &target
            .admission_object()
            .kind()
            .database_code()
            .to_be_bytes(),
        target.admission_object().key(),
        &artifact_digest,
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    for projection in projections {
        for field in [
            projection.kind.to_be_bytes().as_slice(),
            projection.projection_key.as_bytes(),
            projection.staged_object_key.as_bytes(),
            projection.payload_digest.as_slice(),
        ] {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field);
        }
    }
    digest.finalize().into()
}

fn publication_effect_digest(
    context: &AdmissionApplicationContext<'_>,
    intent_digest: [u8; 32],
) -> [u8; 32] {
    framed_digest(
        b"buzz:nip-fi:protected-publication-effect:v1",
        &[
            context.authorization_domain().as_uuid().as_bytes(),
            context.operation_id().as_bytes(),
            context.request_fingerprint(),
            &intent_digest,
        ],
    )
}

/// Exact canonical admission receipt bound to immutable staged artifacts.
///
/// Construction rejects cross-domain or result-digest substitution. Applied
/// and exact-replay outcomes use the same durable S3 receipt and therefore the
/// same deterministic plan.
pub struct ProtectedPublicationReceipt {
    plan: ProtectedPublicationPlan,
    admission: AdmissionCommitReceipt,
}

/// Closed binding result for fresh publication versus exact replay.
///
/// Only the fresh variant owns a projection plan. Exact replay exposes the
/// byte-identical persisted typed result but is structurally incapable of
/// yielding post-commit projection work.
pub enum ProtectedPublicationBinding {
    /// A newly committed admission whose projection plan may run once.
    Committed(ProtectedPublicationReceipt),
    /// An already committed operation with no projection plan.
    ExactReplay(ProtectedPublicationReplay),
}

/// Persisted exact-replay result without any post-commit projection authority.
pub struct ProtectedPublicationReplay {
    admission: AdmissionCommitReceipt,
    application_result: AdmissionApplicationResult,
}

impl ProtectedPublicationReplay {
    /// Durable receipt for the already committed operation.
    pub const fn admission(&self) -> AdmissionCommitReceipt {
        self.admission
    }

    /// Byte-identical typed result reconstructed by canonical storage.
    pub const fn application_result(&self) -> &AdmissionApplicationResult {
        &self.application_result
    }
}

impl ProtectedPublicationReceipt {
    /// Bind S3's typed application result to the exact staged publication.
    ///
    /// The outcome carries only storage-issued domain/object coordinates and
    /// the byte-identical typed result reconstructed on exact replay. A caller
    /// cannot supply replacement coordinates or compare against the unrelated
    /// whole-admission envelope digest.
    pub fn bind(
        plan: ProtectedPublicationPlan,
        outcome: &AdmissionCommitOutcome,
    ) -> Result<ProtectedPublicationBinding, AdmissionCommitError> {
        let (admission, application_result, committed) = match outcome {
            AdmissionCommitOutcome::Committed {
                receipt,
                application_result,
                ..
            } => (*receipt, application_result.as_ref(), true),
            AdmissionCommitOutcome::ExactReplay {
                receipt,
                application_result,
            } => (*receipt, application_result.as_ref(), false),
        };
        let application_result = application_result.ok_or(AdmissionCommitError::IntentConflict)?;
        let target = plan.target()?;
        let application_intent_digest = plan.application_effect()?.intent_digest();
        validate_publication_binding(
            target,
            plan.artifact_digest(),
            application_intent_digest,
            admission,
            application_result,
        )?;
        if committed {
            Ok(ProtectedPublicationBinding::Committed(Self {
                plan,
                admission,
            }))
        } else {
            Ok(ProtectedPublicationBinding::ExactReplay(
                ProtectedPublicationReplay {
                    admission,
                    application_result: application_result.clone(),
                },
            ))
        }
    }

    /// Durable canonical admission receipt.
    pub const fn admission(&self) -> AdmissionCommitReceipt {
        self.admission
    }

    /// Exact immutable artifacts authorized by the receipt.
    pub const fn plan(&self) -> &ProtectedPublicationPlan {
        &self.plan
    }

    /// Consume the receipt for post-commit deterministic projection.
    pub fn into_plan(self) -> ProtectedPublicationPlan {
        self.plan
    }
}

fn validate_publication_binding(
    target: ProtectedPublicationTarget,
    artifact_digest: [u8; 32],
    application_intent_digest: [u8; 32],
    admission: AdmissionCommitReceipt,
    application_result: &AdmissionApplicationResult,
) -> Result<(), AdmissionCommitError> {
    let expected = publication_application_result(target, artifact_digest)?;
    let expected_digest = canonical_publication_result_digest(
        target,
        *admission.semantic_fingerprint(),
        application_intent_digest,
        &expected,
    )?;
    if admission.authorization_domain() != target.authorization_domain()
        || admission.object() != target.admission_object()
        || admission.application_result_digest() != Some(&expected_digest)
        || application_result != &expected
    {
        return Err(AdmissionCommitError::IntentConflict);
    }
    Ok(())
}

fn canonical_publication_result_digest(
    target: ProtectedPublicationTarget,
    semantic_fingerprint: [u8; 32],
    application_intent_digest: [u8; 32],
    result: &AdmissionApplicationResult,
) -> Result<[u8; 32], AdmissionCommitError> {
    if semantic_fingerprint == [0; 32] || application_intent_digest == [0; 32] {
        return Err(AdmissionCommitError::InvalidRequest);
    }
    let schema = result.schema();
    Ok(admission_framed_digest(
        b"buzz:canonical-application-result:v1",
        &[
            target.authorization_domain().as_uuid().as_bytes(),
            &target
                .admission_object()
                .kind()
                .database_code()
                .to_be_bytes(),
            target.admission_object().key(),
            &semantic_fingerprint,
            &application_intent_digest,
            schema.type_key(),
            &schema.version().to_be_bytes(),
            &result.code().to_be_bytes(),
            result.payload(),
        ],
    ))
}

fn admission_framed_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

fn framed_digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

impl std::fmt::Debug for ProtectedPublicationReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProtectedPublicationReceipt([REDACTED])")
    }
}

impl std::fmt::Debug for ProtectedPublicationBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Committed(_) => "ProtectedPublicationBinding::Committed([REDACTED])",
            Self::ExactReplay(_) => "ProtectedPublicationBinding::ExactReplay([REDACTED])",
        })
    }
}

impl std::fmt::Debug for ProtectedPublicationReplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProtectedPublicationReplay([REDACTED])")
    }
}

/// Axum extractor that validates Blossom auth, the BUD-11 hash binding, and
/// relay membership (NIP-43, when enabled) from headers BEFORE the request
/// body is read. This prevents unauthenticated clients from forcing the
/// server to buffer up to 50MB of body data.
///
/// Axum processes `FromRequestParts` extractors before `FromRequest` (body)
/// extractors, so auth rejection happens before any body buffering.
pub(crate) struct AuthenticatedUpload {
    auth_event: nostr::Event,
    /// Community resolved from the request host at extraction time (row zero for
    /// this HTTP door), identical to the WS door in `router.rs` and the bridge
    /// door in `bridge.rs`. Server-resolved, never client-supplied.
    tenant: TenantContext,
    route_mode: UploadRouteMode,
    attribution: UploadAttributionSnapshot,
    canonical_admission: Option<AdmissionCommitRequest>,
    _upload_permit: UploadPermit,
}

/// Mutable upload labels and network facts captured once at admission.
///
/// Fresh publication consumes these owned values after body processing. Exact
/// replay must instead use its persisted first result; neither path re-reads a
/// profile or request header after admission succeeds.
struct UploadAttributionSnapshot(Option<UploadAttribution>);

impl UploadAttributionSnapshot {
    fn capture(
        enabled: bool,
        uploader_name: Option<String>,
        upload_ip_header: &Option<String>,
        upload_port_header: &Option<String>,
        headers: &HeaderMap,
    ) -> Self {
        if !enabled {
            return Self(None);
        }

        let header_value = |name: &Option<String>| {
            name.as_deref()
                .and_then(|header| headers.get(header))
                .and_then(|value| value.to_str().ok())
        };
        let ip = header_value(upload_ip_header).and_then(buzz_media::parse_public_ip);
        let port = ip.and(header_value(upload_port_header).and_then(buzz_media::parse_port));

        Self(Some(UploadAttribution {
            uploader_name,
            net: UploadNetworkInfo { ip, port },
        }))
    }

    fn into_attribution(self) -> Option<UploadAttribution> {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UploadRouteMode {
    Upload,
    LegacyMedia,
}

impl UploadRouteMode {
    const fn path(self) -> &'static [u8] {
        match self {
            Self::Upload => b"/upload",
            Self::LegacyMedia => b"/media/upload",
        }
    }
}

fn should_stream_as_video(sniff: &[u8]) -> bool {
    infer::get(sniff).is_some_and(|kind| kind.mime_type() == "video/mp4")
        || buzz_media::looks_like_iso_bmff(sniff)
}

fn upload_route_mode(path: &str) -> Result<UploadRouteMode, MediaError> {
    match path {
        "/upload" => Ok(UploadRouteMode::Upload),
        "/media/upload" => Ok(UploadRouteMode::LegacyMedia),
        _ => Err(MediaError::NotFound),
    }
}

struct MediaReadAuth {
    tenant: TenantContext,
}

#[derive(Clone, Copy)]
enum MediaReadMethod {
    Get,
    Head,
}

impl MediaReadMethod {
    const fn code(self) -> &'static [u8] {
        match self {
            Self::Get => b"GET",
            Self::Head => b"HEAD",
        }
    }
}

fn media_read_fingerprint(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

async fn authorize_canonical_media_read(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    auth_event: &nostr::Event,
    sha256: &str,
    method: MediaReadMethod,
) -> Result<(), MediaError> {
    let domain = tenant.community();
    let event_id = auth_event.id.to_bytes();
    let target =
        ProtectedPublicationTarget::media(domain, sha256).map_err(|_| MediaError::Unauthorized)?;
    let request_fingerprint = media_read_fingerprint(
        b"buzz:nip-fi:media-read-request:v1",
        &[
            domain.as_uuid().as_bytes(),
            method.code(),
            sha256.as_bytes(),
            auth_event.pubkey.as_bytes(),
            &event_id,
        ],
    );
    let transport_context_fingerprint = media_read_fingerprint(
        b"buzz:nip-fi:media-read-transport:v1",
        &[
            domain.as_uuid().as_bytes(),
            tenant.host().as_bytes(),
            &event_id,
        ],
    );
    let coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        crate::authorization_runtime::ProtectedIngress::MediaRead,
        domain,
        RouteCapability::MediaRead,
        target.admission_object(),
        buzz_auth::ProofTransport::Blossom,
        request_fingerprint,
        transport_context_fingerprint,
    )
    .map_err(map_protected_media_error)?;
    let assertion_token = crate::protected_ingress::exact_assertion(
        headers,
        state.config.corporate_identity.jwt_header.as_str(),
    )
    .map_err(map_protected_media_error)?;
    let assertion =
        crate::protected_ingress::verify_assertion(state, &assertion_token, coordinates)
            .await
            .map_err(map_protected_media_error)?;
    let proof = buzz_auth::verify_blossom_authorization_proof(
        auth_event,
        BlossomAuthorizationVerb::Get,
        tenant.host(),
        sha256,
        600,
        &assertion,
        request_fingerprint,
        *target.admission_object().key(),
        transport_context_fingerprint,
    )
    .map_err(|_| MediaError::Unauthorized)?;
    crate::protected_ingress::authorize_read(state, coordinates, assertion, proof)
        .await
        .map(|_| ())
        .map_err(map_protected_media_error)
}

async fn prepare_canonical_media_upload(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    auth_event: &nostr::Event,
    sha256: &str,
    route_mode: UploadRouteMode,
) -> Result<AdmissionCommitRequest, MediaError> {
    let domain = tenant.community();
    let target =
        ProtectedPublicationTarget::media(domain, sha256).map_err(|_| MediaError::Unauthorized)?;
    let request_fingerprint = media_read_fingerprint(
        b"buzz:nip-fi:media-upload-request:v1",
        &[
            domain.as_uuid().as_bytes(),
            b"PUT",
            route_mode.path(),
            sha256.as_bytes(),
            auth_event.pubkey.as_bytes(),
        ],
    );
    let transport_context_fingerprint = media_read_fingerprint(
        b"buzz:nip-fi:media-upload-transport:v1",
        &[
            domain.as_uuid().as_bytes(),
            tenant.host().as_bytes(),
            b"PUT",
            route_mode.path(),
        ],
    );
    let coordinates = crate::protected_ingress::ProtectedRequestCoordinates::new(
        crate::authorization_runtime::ProtectedIngress::MediaWrite,
        domain,
        RouteCapability::MediaWrite,
        target.admission_object(),
        buzz_auth::ProofTransport::Blossom,
        request_fingerprint,
        transport_context_fingerprint,
    )
    .map_err(map_protected_media_error)?;
    let assertion_token = crate::protected_ingress::exact_assertion(
        headers,
        state.config.corporate_identity.jwt_header.as_str(),
    )
    .map_err(map_protected_media_error)?;
    let assertion =
        crate::protected_ingress::verify_assertion(state, &assertion_token, coordinates)
            .await
            .map_err(map_protected_media_error)?;
    let proof = buzz_auth::verify_blossom_authorization_proof(
        auth_event,
        BlossomAuthorizationVerb::Upload,
        tenant.host(),
        sha256,
        3600,
        &assertion,
        request_fingerprint,
        *target.admission_object().key(),
        transport_context_fingerprint,
    )
    .map_err(|_| MediaError::Unauthorized)?;
    crate::protected_ingress::prepare_mutation(state, coordinates, assertion, proof)
        .await
        .map_err(map_protected_media_error)
}

fn map_protected_media_error(error: crate::protected_ingress::ProtectedIngressError) -> MediaError {
    match error {
        crate::protected_ingress::ProtectedIngressError::Denied => MediaError::Unauthorized,
        crate::protected_ingress::ProtectedIngressError::Expired => MediaError::TokenExpired,
        crate::protected_ingress::ProtectedIngressError::Unavailable => MediaError::Internal,
    }
}

async fn verify_media_corporate_identity(
    state: &AppState,
    tenant: &TenantContext,
    headers: &HeaderMap,
    pubkey: nostr::PublicKey,
) -> Result<crate::corporate_identity::CorporateIdentityProof, MediaError> {
    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    let identity_jwt = crate::corporate_identity::identity_jwt_from_headers(
        headers,
        &state.config.corporate_identity,
    );
    crate::corporate_identity::verify_corporate_identity(
        state,
        tenant.community(),
        pubkey,
        identity_jwt.as_deref(),
        auth_tag,
    )
    .await
    .map_err(|e| {
        tracing::warn!(pubkey = %pubkey.to_hex(), error = %e, "media: corporate identity denied");
        if e.status_code() == StatusCode::UNAUTHORIZED {
            MediaError::Unauthorized
        } else {
            MediaError::RelayMembershipRequired
        }
    })
}

async fn finalize_media_corporate_identity(
    state: &AppState,
    tenant: &TenantContext,
    pubkey: nostr::PublicKey,
    proof: crate::corporate_identity::CorporateIdentityProof,
) -> Result<(), MediaError> {
    crate::corporate_identity::finalize_corporate_identity(
        state,
        tenant.community(),
        pubkey,
        proof,
    )
    .await
    .map(|_| ())
    .map_err(|e| {
        tracing::warn!(pubkey = %pubkey.to_hex(), error = %e, "media: corporate identity finalization denied");
        if e.status_code() == StatusCode::UNAUTHORIZED {
            MediaError::Unauthorized
        } else {
            MediaError::RelayMembershipRequired
        }
    })
}

async fn deny_banned_media_principal(
    state: &AppState,
    tenant: &TenantContext,
    pubkey: &nostr::PublicKey,
    auth_tag: Option<&str>,
) -> Result<(), MediaError> {
    let restriction = state
        .db
        .moderation_restriction_state(tenant.community(), pubkey.as_bytes())
        .await
        .map_err(|_| MediaError::Internal)?;
    if restriction.banned {
        return Err(MediaError::Unauthorized);
    }
    if let Some(owner) =
        crate::api::relay_members::extract_nip_oa_owner(pubkey.as_bytes(), auth_tag)
    {
        let owner_restriction = state
            .db
            .moderation_restriction_state(tenant.community(), owner.as_bytes())
            .await
            .map_err(|_| MediaError::Internal)?;
        if owner_restriction.banned {
            return Err(MediaError::Unauthorized);
        }
    }
    Ok(())
}

const MEDIA_UPLOAD_RATE_WINDOW: Duration = Duration::from_secs(60);

struct UploadPermit {
    _global: tokio::sync::OwnedSemaphorePermit,
    in_flight: Arc<dashmap::DashMap<crate::state::ScopedPubkeyKey, u32>>,
    key: crate::state::ScopedPubkeyKey,
}

impl Drop for UploadPermit {
    fn drop(&mut self) {
        use dashmap::mapref::entry::Entry;

        if let Entry::Occupied(mut entry) = self.in_flight.entry(self.key) {
            if *entry.get() <= 1 {
                entry.remove();
            } else {
                *entry.get_mut() -= 1;
            }
        }
    }
}

fn upload_rate_limited(
    state: &AppState,
    community_id: buzz_core::CommunityId,
    pubkey: &nostr::PublicKey,
) -> bool {
    let key = (community_id, pubkey.to_bytes());
    let now = Instant::now();
    let limit = state.config.media_uploads_per_minute;
    let mut entry = state
        .media_upload_rate_limiter
        .entry(key)
        .or_insert((0, now));
    let (count, window_start) = entry.value_mut();
    if now.duration_since(*window_start) >= MEDIA_UPLOAD_RATE_WINDOW {
        *count = 1;
        *window_start = now;
        return false;
    }
    if *count >= limit {
        return true;
    }
    *count += 1;
    false
}

fn acquire_upload_permit(
    state: &AppState,
    community_id: buzz_core::CommunityId,
    pubkey: &nostr::PublicKey,
) -> Result<UploadPermit, MediaError> {
    let global = state
        .media_upload_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|_| MediaError::UploadConcurrencyLimitReached)?;

    let key = (community_id, pubkey.to_bytes());
    let mut in_flight = state.media_uploads_in_flight.entry(key).or_insert(0);
    if *in_flight >= state.config.media_max_concurrent_uploads_per_pubkey {
        return Err(MediaError::UploadConcurrencyLimitReached);
    }
    *in_flight += 1;
    drop(in_flight);

    Ok(UploadPermit {
        _global: global,
        in_flight: Arc::clone(&state.media_uploads_in_flight),
        key,
    })
}

impl FromRequestParts<Arc<AppState>> for AuthenticatedUpload {
    type Rejection = MediaError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let headers = &parts.headers;

        // 1. Row zero: bind this upload to its community from the request host,
        // identical to the WS door in `router.rs` and the bridge door in
        // `bridge.rs`. Fail-closed: an unmapped host or lookup failure is a
        // generic `NotFound` (404) — never a default tenant, never echoing the
        // host, so an unauthenticated caller cannot probe which communities
        // exist on this deployment.
        //
        // This MUST run before Blossom auth verification (step 2) so the
        // `server`-tag check validates against the *bound tenant host*, not a
        // process-global domain — a relay process serves many tenant hosts, and
        // the stock CLI tags its own configured relay host (conformance row 52).
        // Binding only reads the Host header — no request body is buffered — so
        // doing it first preserves the pre-body auth-rejection guarantee.
        let raw_host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let tenant = crate::tenant::bind_community(&state.db, raw_host)
            .await
            .map_err(|_| MediaError::NotFound)?;

        let route_mode = upload_route_mode(parts.uri.path())?;
        if state.config.nip_fi_mode == buzz_auth::NipFiMode::DenyProtected {
            return Err(MediaError::Unauthorized);
        }

        // 2. Extract and validate Blossom auth event against the bound host.
        let auth_event = extract_blossom_auth(headers)?;
        // Use the permissive window (3600s) here because we don't know the
        // content type yet.  The upload functions re-verify with the correct
        // per-type window (600s for images, 3600s for video) after the body
        // has been consumed and the SHA-256 computed.
        buzz_media::auth::verify_blossom_auth_event(&auth_event, Some(tenant.host()), 3600)?;

        // 3. Require X-SHA-256 header (BUD-11: mandatory for PUT /upload)
        let claimed_hash = headers
            .get("x-sha-256")
            .and_then(|v| v.to_str().ok())
            .ok_or(MediaError::MissingTag("x-sha-256"))?;

        // Validate format: exactly 64 lowercase hex characters
        if claimed_hash.len() != 64
            || !claimed_hash
                .chars()
                .all(|c| matches!(c, '0'..='9' | 'a'..='f'))
        {
            return Err(MediaError::HashMismatch);
        }

        // 4. Validate X-SHA-256 matches at least one x tag in the auth event
        let has_matching_x = auth_event
            .tags
            .iter()
            .any(|tag| tag.kind().to_string() == "x" && (tag.content() == Some(claimed_hash)));
        if !has_matching_x {
            return Err(MediaError::HashMismatch);
        }

        // 5. Relay membership gate (NIP-43). Blossom auth proves the signer
        // authorized this exact upload hash for this server; NIP-43 answers
        // whether that Nostr key may use this community's media store. This is
        // the only upload authority: independent of bearer-token / api_tokens
        // storage and of `require_auth_token` (which governs the REST API, not
        // media). On open relays (membership disabled) any valid Blossom signer
        // may upload, matching the WS door's admission policy.
        let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
        let legacy_identity_proof = if state.config.nip_fi_mode == buzz_auth::NipFiMode::Off {
            Some(verify_media_corporate_identity(state, &tenant, headers, auth_event.pubkey).await?)
        } else {
            None
        };
        let canonical_admission = match state.config.nip_fi_mode {
            buzz_auth::NipFiMode::Off => None,
            buzz_auth::NipFiMode::Enforce => Some(
                prepare_canonical_media_upload(
                    state,
                    &tenant,
                    headers,
                    &auth_event,
                    claimed_hash,
                    route_mode,
                )
                .await?,
            ),
            buzz_auth::NipFiMode::DenyProtected => return Err(MediaError::Unauthorized),
        };

        crate::api::relay_members::enforce_relay_membership(
            state,
            tenant.community(),
            auth_event.pubkey.as_bytes(),
            auth_tag,
        )
        .await
        .map_err(|_| MediaError::RelayMembershipRequired)?;
        deny_banned_media_principal(state, &tenant, &auth_event.pubkey, auth_tag).await?;
        if upload_rate_limited(state, tenant.community(), &auth_event.pubkey) {
            metrics::counter!("buzz_media_upload_rejections_total", "reason" => "rate_limit")
                .increment(1);
            return Err(MediaError::UploadRateLimitExceeded);
        }
        let upload_permit = acquire_upload_permit(state, tenant.community(), &auth_event.pubkey)
            .inspect_err(|_| {
                metrics::counter!("buzz_media_upload_rejections_total", "reason" => "concurrency")
                    .increment(1);
            })?;
        if state.config.nip_fi_mode == buzz_auth::NipFiMode::Off {
            let identity_proof = legacy_identity_proof.ok_or(MediaError::Internal)?;
            finalize_media_corporate_identity(state, &tenant, auth_event.pubkey, identity_proof)
                .await?;
        }
        let attribution =
            snapshot_upload_attribution(state, &tenant, &auth_event.pubkey, headers).await;

        Ok(AuthenticatedUpload {
            auth_event,
            tenant,
            route_mode,
            attribution,
            canonical_admission,
            _upload_permit: upload_permit,
        })
    }
}

/// Build per-event upload attribution when upload records are enabled
/// (`BUZZ_MEDIA_UPLOAD_RECORDS`). Returns `None` when the feature is off —
/// the upload pipeline then writes no `_uploads/` record at all.
///
/// - `uploader_name` is the uploader's current display name in the bound
///   community (best-effort label; lookup failure degrades to absent).
/// - `net.ip` is read from the operator-configured trusted edge header
///   (`BUZZ_MEDIA_UPLOAD_IP_HEADER`) and validated as a public IP —
///   fail-empty: missing/malformed/non-public values record nothing. The
///   socket address is never used; behind a sidecar it is meaningless, and a
///   wrong address is worse than none.
/// - `net.port` (optional companion header) is only kept alongside a valid IP.
async fn snapshot_upload_attribution(
    state: &AppState,
    tenant: &TenantContext,
    uploader: &nostr::PublicKey,
    headers: &HeaderMap,
) -> UploadAttributionSnapshot {
    let cfg = &state.config.media;
    if !cfg.upload_records_enabled {
        return UploadAttributionSnapshot(None);
    }

    let uploader_name = state
        .db
        .get_user(tenant.community(), &uploader.to_bytes())
        .await
        .ok()
        .flatten()
        .and_then(|profile| profile.display_name)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());

    UploadAttributionSnapshot::capture(
        true,
        uploader_name,
        &cfg.upload_ip_header,
        &cfg.upload_port_header,
        headers,
    )
}

/// PUT `/upload` or the temporary media-only `/media/upload` alias.
///
/// Auth is validated via the [`AuthenticatedUpload`] extractor BEFORE the body
/// is read, preventing unauthenticated clients from forcing body buffering.
// AuthenticatedUpload is pub(crate) — it's an internal extractor type, never
// exposed outside this crate. The warning is benign: axum resolves it at
// compile time via trait bounds, not by name.
#[allow(private_interfaces)]
///
/// Expects:
///   - `Authorization: Nostr <base64(kind:24242 event)>` — Blossom auth
///   - `X-SHA-256: <hex>` — Required per BUD-11
///   - `Content-Type` is advisory only; a bounded body prefix selects the
///     streaming video path from actual bytes
///   - Raw binary body (the file bytes)
///
/// Returns a [`BlobDescriptor`] JSON on success.
// TODO(v2): Add persistent per-pubkey storage quotas. Admission limits below
// bound active parser/storage work, but they do not cap durable bytes stored.
pub async fn upload_blob(
    State(state): State<Arc<AppState>>,
    auth: AuthenticatedUpload,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Result<Json<BlobDescriptor>, MediaError> {
    let attribution = auth.attribution.into_attribution();

    if auth.route_mode == UploadRouteMode::LegacyMedia {
        metrics::counter!("buzz_media_legacy_upload_route_total").increment(1);
    }

    // Probe actual bytes without trusting Content-Type. Keep the chunks used
    // for the bounded probe and replay them into the selected pipeline so the
    // stored/hash-verified body remains byte-identical.
    use futures_util::StreamExt;
    const SNIFF_BYTES: usize = 4096;
    let mut source = body.into_data_stream();
    let mut replay_chunks = Vec::new();
    let mut sniff = Vec::with_capacity(SNIFF_BYTES);
    while sniff.len() < SNIFF_BYTES {
        match source.next().await {
            Some(Ok(chunk)) => {
                let needed = SNIFF_BYTES - sniff.len();
                sniff.extend_from_slice(&chunk[..chunk.len().min(needed)]);
                replay_chunks.push(chunk);
            }
            Some(Err(error)) => return Err(MediaError::Io(error.to_string())),
            None => break,
        }
    }
    let replay = futures_util::stream::iter(replay_chunks.into_iter().map(Ok)).chain(source);

    let staged = if should_stream_as_video(&sniff) {
        // Video path: stream body directly to disk — never fully buffered in RAM.
        let content_length = headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        buzz_media::stage_video_upload(
            &state.media_storage,
            &state.config.media,
            &auth.tenant,
            &auth.auth_event,
            replay,
            content_length,
            attribution,
        )
        .await?
    } else {
        // Non-video path: buffer the body (bounded by the larger of the image
        // and generic-file caps), then decide image-vs-generic by sniffed MIME.
        // Images go through the thumbnailing pipeline; non-media attachments
        // (docs, archives, text, data) take the generic file path and are
        // served as downloads. Recognized audio/video cannot fall through it.
        let max = state
            .config
            .media
            .max_image_bytes
            .max(state.config.media.max_file_bytes);
        let bytes = axum::body::to_bytes(axum::body::Body::from_stream(replay), max as usize)
            .await
            .map_err(|_| MediaError::FileTooLarge { size: 0, max })?;

        let is_image = matches!(
            infer::get(&bytes).map(|t| t.mime_type()),
            Some("image/jpeg" | "image/png" | "image/gif" | "image/webp")
        );

        if is_image {
            buzz_media::stage_upload(
                &state.media_storage,
                &state.config.media,
                &auth.tenant,
                &auth.auth_event,
                bytes,
                attribution,
            )
            .await?
        } else if auth.route_mode == UploadRouteMode::LegacyMedia {
            let mime = infer::get(&bytes)
                .map(|kind| kind.mime_type().to_string())
                .unwrap_or_else(|| "application/octet-stream".to_string());
            return Err(MediaError::DisallowedContentType(mime));
        } else {
            buzz_media::stage_file_upload(
                &state.media_storage,
                &state.config.media,
                &auth.tenant,
                &auth.auth_event,
                bytes,
                attribution,
            )
            .await?
        }
    };

    let mut descriptor =
        finalize_staged_media_upload(&state, &auth.tenant, staged, auth.canonical_admission)
            .await?;

    rewrite_descriptor_urls_for_tenant(
        &mut descriptor,
        &state.config.relay_url,
        auth.tenant.host(),
    );

    // Normalize MIME to a known set to bound label cardinality.
    let mime_label = match descriptor.mime_type.as_str() {
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "video/mp4" => {
            &descriptor.mime_type
        }
        _ => "other",
    };
    metrics::counter!(
        "buzz_media_uploads_total",
        "mime" => mime_label.to_owned(),
        "community" => auth.tenant.host().to_owned()
    )
    .increment(1);

    // Audit via bounded channel — same pattern as event audit.
    if let Some(audit_tx) = &state.audit_tx {
        let desc = descriptor.clone();
        if let Err(e) = audit_tx
            .send(NewAuditEntry {
                community_id: auth.tenant.community(),
                action: AuditAction::MediaUploaded,
                actor_pubkey: Some(auth.auth_event.pubkey.to_bytes().to_vec()),
                object_id: Some(desc.sha256.clone()),
                detail: serde_json::json!({
                    "sha256": desc.sha256,
                    "size": desc.size,
                    "mime": desc.mime_type,
                }),
            })
            .await
        {
            tracing::error!("Media audit channel closed — entry lost: {e}");
            metrics::counter!("buzz_audit_send_errors_total").increment(1);
        }
    }

    Ok(Json(descriptor))
}

async fn finalize_staged_media_upload(
    state: &AppState,
    tenant: &TenantContext,
    staged: StagedMediaUpload,
    canonical_admission: Option<AdmissionCommitRequest>,
) -> Result<BlobDescriptor, MediaError> {
    let Some(request) = canonical_admission else {
        staged
            .publish_post_commit_projections(
                &state.media_storage,
                tenant,
                &state.config.media.public_base_url,
            )
            .await?;
        return Ok(staged.into_descriptor());
    };

    let replay_descriptor = staged.descriptor().clone();
    let plan = ProtectedPublicationPlan::from(staged);
    let effect = plan
        .application_effect()
        .map_err(map_canonical_commit_media_error)?;
    let request = request
        .with_application_effect(Box::new(effect))
        .map_err(map_canonical_commit_media_error)?;
    let committer =
        crate::protected_ingress::mutation_committer(state).map_err(map_protected_media_error)?;
    let outcome = committer
        .commit(request)
        .await
        .map_err(map_canonical_commit_media_error)?;
    match ProtectedPublicationReceipt::bind(plan, &outcome)
        .map_err(map_canonical_commit_media_error)?
    {
        ProtectedPublicationBinding::Committed(receipt) => match receipt.into_plan() {
            ProtectedPublicationPlan::Media(staged) => {
                staged
                    .publish_post_commit_projections(
                        &state.media_storage,
                        tenant,
                        &state.config.media.public_base_url,
                    )
                    .await?;
                Ok(staged.into_descriptor())
            }
            ProtectedPublicationPlan::Git(_) => Err(MediaError::Internal),
        },
        ProtectedPublicationBinding::ExactReplay(_) => Ok(replay_descriptor),
    }
}

fn map_canonical_commit_media_error(error: AdmissionCommitError) -> MediaError {
    match error {
        AdmissionCommitError::DependencyUnavailable
        | AdmissionCommitError::AuditUnavailable
        | AdmissionCommitError::RecordedAuditUnavailable => MediaError::Internal,
        _ => MediaError::Unauthorized,
    }
}

pub(crate) fn media_base_url_for_tenant(config_relay_url: &str, tenant_host: &str) -> String {
    let scheme = if config_relay_url.trim_start().starts_with("wss://")
        || config_relay_url.trim_start().starts_with("https://")
    {
        "https"
    } else {
        "http"
    };
    format!("{scheme}://{tenant_host}/media")
}

fn rewrite_descriptor_urls_for_tenant(
    descriptor: &mut BlobDescriptor,
    config_relay_url: &str,
    tenant_host: &str,
) {
    let base = media_base_url_for_tenant(config_relay_url, tenant_host);
    let ext = descriptor
        .url
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| is_safe_ext(ext))
        .unwrap_or("bin");
    descriptor.url = format!("{base}/{}.{ext}", descriptor.sha256);
    if descriptor.thumb.is_some() {
        descriptor.thumb = Some(format!("{base}/{}.thumb.jpg", descriptor.sha256));
    }
}

async fn bind_media_read_tenant(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, MediaError> {
    let raw_host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| MediaError::NotFound)
}

async fn authenticate_media_read(
    state: &AppState,
    headers: &HeaderMap,
    sha256_ext: &str,
    method: MediaReadMethod,
) -> Result<MediaReadAuth, MediaError> {
    let tenant = bind_media_read_tenant(state, headers).await?;

    let auth_event = extract_blossom_auth(headers)?;
    let sha256 = sha256_ext.split('.').next().unwrap_or(sha256_ext);
    buzz_media::auth::verify_blossom_get_auth(&auth_event, sha256, Some(tenant.host()), 3600)?;

    let auth_tag = headers.get("x-auth-tag").and_then(|v| v.to_str().ok());
    let legacy_identity_proof = if state.config.nip_fi_mode == buzz_auth::NipFiMode::Off {
        Some(verify_media_corporate_identity(state, &tenant, headers, auth_event.pubkey).await?)
    } else {
        None
    };
    match state.config.nip_fi_mode {
        buzz_auth::NipFiMode::Off => {}
        buzz_auth::NipFiMode::Enforce => {
            authorize_canonical_media_read(state, &tenant, headers, &auth_event, sha256, method)
                .await?;
        }
        buzz_auth::NipFiMode::DenyProtected => return Err(MediaError::Unauthorized),
    }
    crate::api::relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        auth_event.pubkey.as_bytes(),
        auth_tag,
    )
    .await
    .map_err(|_| MediaError::RelayMembershipRequired)?;
    if let Some(identity_proof) = legacy_identity_proof {
        finalize_media_corporate_identity(state, &tenant, auth_event.pubkey, identity_proof)
            .await?;
    }

    Ok(MediaReadAuth { tenant })
}

fn blob_cache_control() -> &'static str {
    "private, max-age=31536000, immutable"
}

/// Whether a path-segment extension is a safe token.
///
/// The sidecar's `ext` field is the *authoritative* extension — the serve and
/// resolve paths always compare the requested ext against it. This check is a
/// cheap structural gate to reject obviously hostile path segments (traversal,
/// overlong, non-alphanumeric) before any storage lookup. Accepts 1–8 lowercase
/// alphanumeric chars, which covers every extension the generic file path emits
/// (jpg, png, mp4, pdf, docx, xlsx, tar, 7z, mp3, flac, json, bin, …).
pub(crate) fn is_safe_ext(ext: &str) -> bool {
    !ext.is_empty() && ext.len() <= 8 && ext.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9'))
}

/// Validate that `sha256_ext` is a safe path segment.
///
/// Accepted forms (max 3 segments):
///   - `{sha256}`                   — bare 64-char lowercase hex
///   - `{sha256}.{ext}`             — hash + extension
///   - `{sha256}.thumb.jpg`          — hash + thumb variant (always JPEG)
///
/// `{ext}` must be a safe token (see [`is_safe_ext`]); the sidecar comparison
/// downstream enforces the actual canonical extension.
/// Rejects path traversal, leading underscores, and any non-hex first segment.
fn validate_media_path(sha256_ext: &str) -> Result<(), MediaError> {
    let segments: Vec<&str> = sha256_ext.split('.').collect();

    // 1–3 segments only (hash, optional thumb, optional ext)
    if segments.is_empty() || segments.len() > 3 {
        return Err(MediaError::NotFound);
    }

    // First segment must be exactly 64 lowercase hex chars (SHA-256)
    let hash = segments[0];
    if hash.len() != 64 || !hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        return Err(MediaError::NotFound);
    }

    // Validate remaining segments
    match segments.len() {
        1 => {} // bare hash — ok
        2 => {
            // {hash}.{ext}
            if !is_safe_ext(segments[1]) {
                return Err(MediaError::NotFound);
            }
        }
        3 => {
            // {hash}.thumb.jpg — thumbnails are always JPEG
            if segments[1] != "thumb" || segments[2] != "jpg" {
                return Err(MediaError::NotFound);
            }
        }
        _ => return Err(MediaError::NotFound),
    }

    Ok(())
}

/// Maximum bytes returned in a single 206 range response (16 MiB).
///
/// Caps memory per request and prevents clients from using range requests to
/// bypass the intent of chunked delivery. Clients that need more simply issue
/// additional range requests.
const MAX_RANGE_CHUNK: u64 = 16 * 1024 * 1024;

/// GET /media/{sha256_ext} — Blossom BUD-01 serve blob, with HTTP 206 range support.
///
/// `sha256_ext` is either:
///   - `<sha256>.<ext>` — direct key (e.g. `abc123.jpg`)
///   - `<sha256>` — bare hash; extension resolved from sidecar
///   - `<sha256>.thumb.jpg` — thumbnail variant
///
/// Range request behaviour (RFC 9110 §14.2):
///   - No `Range` header → 200 with full body
///   - `Range: bytes=START-END` → 206 with slice; `Content-Range: bytes START-END/TOTAL`
///   - Unsatisfiable range (start ≥ total) → 416 with `Content-Range: bytes */TOTAL`
///   - Suffix ranges (`bytes=-N`) → 206 with last N bytes (RFC 9110 §14.1.2)
///   - Chunk capped at 16 MiB; clients request additional ranges for the rest
///
/// All responses include `Accept-Ranges: bytes` so video players know seeking is supported.
pub async fn get_blob(
    State(state): State<Arc<AppState>>,
    Path(sha256_ext): Path<String>,
    req_headers: HeaderMap,
) -> Result<Response, MediaError> {
    validate_media_path(&sha256_ext)?;
    let media_auth =
        authenticate_media_read(&state, &req_headers, &sha256_ext, MediaReadMethod::Get).await?;
    serve_blob_for_tenant(&state, &media_auth.tenant, &sha256_ext, &req_headers).await
}

/// Serve a validated blob from an already-authorized tenant context.
///
/// This is the common byte-serving mechanism for Blossom reads and narrowly
/// scoped internal readers. Callers must establish their own authorization
/// before entering this function; the tenant is never derived from client input.
pub(crate) async fn serve_blob_for_tenant(
    state: &AppState,
    tenant: &TenantContext,
    sha256_ext: &str,
    req_headers: &HeaderMap,
) -> Result<Response, MediaError> {
    validate_media_path(sha256_ext)?;
    let cache_control = blob_cache_control();

    // Sidecar gate FIRST — reject before any blob I/O. Storage is not authoritative.
    let content_type = if sha256_ext.ends_with(".thumb.jpg") {
        let parent_hash = sha256_ext.strip_suffix(".thumb.jpg").unwrap_or(sha256_ext);
        let _ = state
            .media_storage
            .read_sidecar_mime(tenant, parent_hash)
            .await
            .ok_or(MediaError::NotFound)?;
        "image/jpeg".to_string()
    } else {
        // For explicit paths (hash.ext), verify the requested extension matches
        // the sidecar's canonical extension — sidecar is authoritative.
        let sidecar_mime = state
            .media_storage
            .read_sidecar_mime(tenant, sha256_ext)
            .await
            .ok_or(MediaError::NotFound)?;
        if sha256_ext.contains('.') {
            let requested_ext = sha256_ext.rsplit('.').next().unwrap_or("");
            let sidecar = state
                .media_storage
                .get_sidecar(tenant, sha256_ext.split('.').next().unwrap_or(sha256_ext))
                .await
                .map_err(|_| MediaError::NotFound)?;
            if requested_ext != sidecar.ext {
                return Err(MediaError::NotFound);
            }
        }
        sidecar_mime
    };

    // Images and video render inline; generic files force download. This is the
    // primary defence for non-previewable types — combined with `nosniff` and
    // `CSP: default-src 'none'`, an attachment disposition prevents an uploaded
    // file from ever executing or rendering as active content in the client.
    let disposition = if buzz_media::serve_inline(&content_type) {
        "inline"
    } else {
        "attachment"
    };

    let key = resolve_s3_key(&state.media_storage, tenant, sha256_ext).await?;

    // Parse optional Range header.
    let range_header = req_headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    // Extract single-range value, if present. Multi-range (comma-separated) is
    // unsupported — we ignore it and serve the full body per RFC 9110 §14.2.
    let single_range = range_header.filter(|r| !r.contains(','));

    match single_range {
        None => {
            // Full response — 200 OK. Stream from S3 — never loads full blob into RAM.
            let total = state
                .media_storage
                .head_with_metadata(&key)
                .await?
                .ok_or(MediaError::NotFound)?
                .size;
            let stream = state.media_storage.get_stream(&key).await?;
            let resp = axum::response::Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, &content_type)
                .header(header::CONTENT_LENGTH, total.to_string())
                .header(header::CONTENT_DISPOSITION, disposition)
                .header(header::CACHE_CONTROL, cache_control)
                .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'")
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header(header::ACCEPT_RANGES, "bytes")
                .body(axum::body::Body::from_stream(stream))
                .map_err(|_| MediaError::Internal)?;
            Ok(resp)
        }
        Some(range_str) => {
            // S3-native single-range response, capped to bound request memory.
            let total = state
                .media_storage
                .head_with_metadata(&key)
                .await?
                .ok_or(MediaError::NotFound)?
                .size;

            let parsed = parse_byte_range(&range_str, total);
            match parsed {
                Some((start, end)) => {
                    if start >= total {
                        return axum::response::Response::builder()
                            .status(StatusCode::RANGE_NOT_SATISFIABLE)
                            .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                            .body(axum::body::Body::empty())
                            .map_err(|_| MediaError::Internal);
                    }

                    let end = end.min(total.saturating_sub(1));
                    let end = end
                        .min(start.saturating_add(MAX_RANGE_CHUNK - 1))
                        .min(total.saturating_sub(1));
                    let chunk = state.media_storage.get_range(&key, start, end).await?;
                    let content_range = format!("bytes {start}-{end}/{total}");

                    Ok(axum::response::Response::builder()
                        .status(StatusCode::PARTIAL_CONTENT)
                        .header(header::CONTENT_TYPE, &content_type)
                        .header(header::CONTENT_RANGE, content_range)
                        .header(header::CONTENT_LENGTH, chunk.len().to_string())
                        .header(header::CONTENT_DISPOSITION, disposition)
                        .header(header::ACCEPT_RANGES, "bytes")
                        .header(header::CACHE_CONTROL, cache_control)
                        .header(header::CONTENT_SECURITY_POLICY, "default-src 'none'")
                        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                        .body(axum::body::Body::from(chunk))
                        .map_err(|_| MediaError::Internal)?)
                }
                None => Ok(axum::response::Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .body(axum::body::Body::empty())
                    .map_err(|_| MediaError::Internal)?),
            }
        }
    }
}

/// Parse a `Range: bytes=START-END` header value.
///
/// Returns `Some((start, end))` for a valid absolute or suffix range.
/// Supported forms:
///   - `bytes=START-END` → absolute range
///   - `bytes=START-`    → from START to end of file
///   - `bytes=-N`        → last N bytes (suffix range, per RFC 9110 §14.1.2)
///
/// Returns `None` for malformed values or non-bytes units — callers respond with 416.
fn parse_byte_range(range: &str, total: u64) -> Option<(u64, u64)> {
    let range = range.strip_prefix("bytes=")?;

    // Suffix range: "bytes=-N" → last N bytes of the file.
    if let Some(suffix) = range.strip_prefix('-') {
        let n: u64 = suffix.parse().ok()?;
        if n == 0 || total == 0 {
            return None;
        }
        let start = total.saturating_sub(n);
        return Some((start, total - 1));
    }

    let (start_str, end_str) = range.split_once('-')?;
    let start: u64 = start_str.parse().ok()?;

    // Open-ended range: "bytes=START-" → from start to end of file.
    let end: u64 = if end_str.is_empty() {
        u64::MAX
    } else {
        end_str.parse().ok()?
    };

    if start > end {
        return None;
    }

    Some((start, end))
}

/// HEAD /media/{sha256_ext} — Blossom BUD-01 existence check.
///
/// Content-type is derived from the validated sidecar only — never from raw S3
/// object metadata — to prevent MIME spoofing via tampered storage. If the sidecar
/// is missing, we return 404 rather than fall back to untrusted metadata.
pub async fn head_blob(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(sha256_ext): Path<String>,
) -> Result<Response, MediaError> {
    validate_media_path(&sha256_ext)?;
    let media_auth =
        authenticate_media_read(&state, &headers, &sha256_ext, MediaReadMethod::Head).await?;
    let tenant = media_auth.tenant;
    let cache_control = blob_cache_control();

    // Sidecar gate FIRST — reject before any blob I/O.
    let content_type = if sha256_ext.ends_with(".thumb.jpg") {
        let parent_hash = sha256_ext.strip_suffix(".thumb.jpg").unwrap_or(&sha256_ext);
        let _ = state
            .media_storage
            .read_sidecar_mime(&tenant, parent_hash)
            .await
            .ok_or(MediaError::NotFound)?;
        "image/jpeg".to_string()
    } else {
        let sidecar_mime = state
            .media_storage
            .read_sidecar_mime(&tenant, &sha256_ext)
            .await
            .ok_or(MediaError::NotFound)?;
        if sha256_ext.contains('.') {
            let requested_ext = sha256_ext.rsplit('.').next().unwrap_or("");
            let sidecar = state
                .media_storage
                .get_sidecar(&tenant, sha256_ext.split('.').next().unwrap_or(&sha256_ext))
                .await
                .map_err(|_| MediaError::NotFound)?;
            if requested_ext != sidecar.ext {
                return Err(MediaError::NotFound);
            }
        }
        sidecar_mime
    };

    let key = resolve_s3_key(&state.media_storage, &tenant, &sha256_ext).await?;
    match state.media_storage.head_with_metadata(&key).await? {
        Some(meta) => {
            let size_str = meta.size.to_string();
            Ok((
                StatusCode::OK,
                [
                    ("content-type", content_type.as_str()),
                    ("content-length", size_str.as_str()),
                    ("accept-ranges", "bytes"),
                    ("cache-control", cache_control),
                ],
            )
                .into_response())
        }
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

/// Resolve the S3 key from a URL path segment.
///
/// - `sha256.ext`       → used as-is (already validated by `validate_media_path`)
/// - `sha256` (no dot)  → read sidecar to get extension, return `sha256.ext`
///
/// Sidecar-derived extensions are validated as safe tokens to prevent
/// object-key confusion if sidecar data is ever tampered with.
async fn resolve_s3_key(
    storage: &buzz_media::MediaStorage,
    tenant: &TenantContext,
    sha256_ext: &str,
) -> Result<String, MediaError> {
    if sha256_ext.contains('.') {
        Ok(sha256_ext.to_string())
    } else {
        let sidecar = storage
            .get_sidecar(tenant, sha256_ext)
            .await
            .map_err(|_| MediaError::NotFound)?;
        // Validate sidecar ext — never trust storage as authoritative for path construction
        if !is_safe_ext(&sidecar.ext) {
            return Err(MediaError::NotFound);
        }
        Ok(format!("{}.{}", sha256_ext, sidecar.ext))
    }
}

/// Extract and verify a kind:24242 Blossom auth event from the `Authorization` header.
///
/// Accepts both base64url (BUD-11 spec) and standard base64 (nostr-tools compat).
fn extract_blossom_auth(headers: &HeaderMap) -> Result<nostr::Event, MediaError> {
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};

    let header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or(MediaError::MissingAuth)?;

    let token = header
        .strip_prefix("Nostr ")
        .ok_or(MediaError::InvalidAuthScheme)?;

    let json_bytes = URL_SAFE_NO_PAD
        .decode(token)
        .or_else(|_| STANDARD.decode(token))
        .map_err(|_| MediaError::InvalidBase64)?;

    let event: nostr::Event =
        serde_json::from_slice(&json_bytes).map_err(|_| MediaError::InvalidAuthEvent)?;

    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, OnceLock,
        },
    };

    use async_trait::async_trait;
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use buzz_auth::{
        HttpHeaderField, TrustedProxyNonceClaim, TrustedProxyNonceReplayReader,
        TrustedProxyProvenanceVerifier, TrustedProxyReplayReadError, TrustedProxyRequest,
        ASSERTION_HEADER_NAME, PROVENANCE_HEADER_NAME,
    };
    use chrono::TimeZone;
    use hmac::{Hmac, KeyInit, Mac};
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tower::ServiceExt;
    use uuid::Uuid;

    const VALID_HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    const PROXY_NOW: u64 = 1_800_000_000;
    const PROXY_SECRET: [u8; 32] = [0x53; 32];

    fn proxy_assertion() -> &'static str {
        static VALUE: OnceLock<String> = OnceLock::new();
        VALUE.get_or_init(|| {
            [
                ["eyJhbGciOiJF", "UzI1NiJ9"].concat(),
                ["eyJzdWIiOiJz", "dWJqZWN0In0"].concat(),
                ["c2lnbmF0", "dXJl"].concat(),
            ]
            .join(".")
        })
    }

    struct CountingJwksLoader {
        document: Vec<u8>,
        requests: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl crate::authorization_runtime::JwksDocumentLoader for CountingJwksLoader {
        async fn load(
            &self,
            _source: &crate::authorization_runtime::JwksSourceConfig,
            _expected_issuer: &str,
            _policy: crate::authorization_runtime::JwksRefreshPolicy,
        ) -> Result<Vec<u8>, crate::authorization_runtime::RuntimeAuthorizationError> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            Ok(self.document.clone())
        }
    }

    #[test]
    fn canonical_media_read_coordinates_separate_method_and_target() {
        let domain = Uuid::from_u128(1);
        let get = media_read_fingerprint(
            b"buzz:nip-fi:media-read-request:v1",
            &[
                domain.as_bytes(),
                MediaReadMethod::Get.code(),
                VALID_HASH.as_bytes(),
            ],
        );
        let head = media_read_fingerprint(
            b"buzz:nip-fi:media-read-request:v1",
            &[
                domain.as_bytes(),
                MediaReadMethod::Head.code(),
                VALID_HASH.as_bytes(),
            ],
        );
        let other = media_read_fingerprint(
            b"buzz:nip-fi:media-read-request:v1",
            &[
                domain.as_bytes(),
                MediaReadMethod::Get.code(),
                "1".repeat(64).as_bytes(),
            ],
        );

        assert_ne!(get, head);
        assert_ne!(get, other);
        assert_ne!(get, [0; 32]);
    }

    #[test]
    fn canonical_media_assertion_rejects_ambiguous_headers() {
        let mut headers = HeaderMap::new();
        headers.append(
            ASSERTION_HEADER_NAME,
            proxy_assertion().parse().expect("valid proxy assertion"),
        );
        headers.append(
            ASSERTION_HEADER_NAME,
            proxy_assertion().parse().expect("valid proxy assertion"),
        );

        assert!(matches!(
            crate::protected_ingress::exact_assertion(&headers, ASSERTION_HEADER_NAME),
            Err(crate::protected_ingress::ProtectedIngressError::Denied)
        ));
    }

    #[derive(Default)]
    struct EmptyReplayReader;

    impl TrustedProxyNonceReplayReader for EmptyReplayReader {
        fn is_committed<'a>(
            &'a self,
            _authorization_domain: buzz_core::CommunityId,
            _claim: &'a TrustedProxyNonceClaim,
        ) -> Pin<Box<dyn Future<Output = Result<bool, TrustedProxyReplayReadError>> + Send + 'a>>
        {
            Box::pin(async { Ok(false) })
        }
    }

    #[test]
    fn publication_receipt_cannot_cross_domain_or_repository_target() {
        let domain_a = buzz_core::CommunityId::from_uuid(Uuid::from_u128(1));
        let domain_b = buzz_core::CommunityId::from_uuid(Uuid::from_u128(2));
        let owner = "a".repeat(64);
        let target_a =
            ProtectedPublicationTarget::repository(domain_a, &owner, "repo-a").expect("target A");
        let target_b =
            ProtectedPublicationTarget::repository(domain_a, &owner, "repo-b").expect("target B");
        let target_other_domain =
            ProtectedPublicationTarget::repository(domain_b, &owner, "repo-a")
                .expect("other-domain target");
        let artifact = [7_u8; 32];
        let result = publication_application_result(target_a, artifact).expect("typed result");
        let semantic_fingerprint = [9; 32];
        let application_intent_digest = [12; 32];
        let application_result_digest = canonical_publication_result_digest(
            target_a,
            semantic_fingerprint,
            application_intent_digest,
            &result,
        )
        .expect("canonical application result digest");
        let receipt = AdmissionCommitReceipt::from_storage(
            domain_a,
            target_a.admission_object(),
            Uuid::from_u128(10),
            [8; 32],
            semantic_fingerprint,
            buzz_db::authorization_admission::AdmissionCommitDigests::new(
                [10; 32],
                Some(application_result_digest),
            )
            .expect("digests"),
            Uuid::from_u128(11),
        )
        .expect("receipt");

        assert!(validate_publication_binding(
            target_a,
            artifact,
            application_intent_digest,
            receipt,
            &result
        )
        .is_ok());
        assert_eq!(
            validate_publication_binding(
                target_b,
                artifact,
                application_intent_digest,
                receipt,
                &result,
            ),
            Err(AdmissionCommitError::IntentConflict)
        );
        assert_eq!(
            validate_publication_binding(
                target_other_domain,
                artifact,
                application_intent_digest,
                receipt,
                &result,
            ),
            Err(AdmissionCommitError::IntentConflict)
        );
        let wrong_digest_receipt = AdmissionCommitReceipt::from_storage(
            domain_a,
            target_a.admission_object(),
            Uuid::from_u128(10),
            [8; 32],
            semantic_fingerprint,
            buzz_db::authorization_admission::AdmissionCommitDigests::new([10; 32], Some([13; 32]))
                .expect("wrong digests"),
            Uuid::from_u128(11),
        )
        .expect("wrong-digest receipt");
        assert_eq!(
            validate_publication_binding(
                target_a,
                artifact,
                application_intent_digest,
                wrong_digest_receipt,
                &result,
            ),
            Err(AdmissionCommitError::IntentConflict)
        );
        let arbitrary_result =
            publication_application_result(target_a, [14; 32]).expect("arbitrary typed result");
        assert_eq!(
            validate_publication_binding(
                target_a,
                artifact,
                application_intent_digest,
                receipt,
                &arbitrary_result,
            ),
            Err(AdmissionCommitError::IntentConflict)
        );
    }

    #[test]
    fn exact_replay_binding_contains_typed_result_but_no_projection_plan() {
        let domain = buzz_core::CommunityId::from_uuid(Uuid::from_u128(20));
        let target = ProtectedPublicationTarget::repository(domain, &"b".repeat(64), "replay-only")
            .expect("replay target");
        let result = publication_application_result(target, [21; 32]).expect("typed result");
        let receipt = AdmissionCommitReceipt::from_storage(
            domain,
            target.admission_object(),
            Uuid::from_u128(22),
            [23; 32],
            [24; 32],
            buzz_db::authorization_admission::AdmissionCommitDigests::new([25; 32], Some([26; 32]))
                .expect("digests"),
            Uuid::from_u128(27),
        )
        .expect("receipt");
        let binding = ProtectedPublicationBinding::ExactReplay(ProtectedPublicationReplay {
            admission: receipt,
            application_result: result.clone(),
        });

        let ProtectedPublicationBinding::ExactReplay(replay) = binding else {
            panic!("constructed replay binding changed variant");
        };
        assert_eq!(replay.admission(), receipt);
        assert_eq!(replay.application_result(), &result);
        // The replay type intentionally has no plan/into_plan API. Projection
        // authority exists only on ProtectedPublicationReceipt in Committed.
    }

    #[tokio::test]
    async fn mixed_same_domain_evidence_is_rejected_before_a_claim_exists() {
        let domain = buzz_core::CommunityId::from_uuid(Uuid::from_u128(3));
        let verifier = TrustedProxyProvenanceVerifier::new(
            vec![PROXY_SECRET.to_vec()],
            Duration::from_secs(60),
            Duration::from_secs(5),
        )
        .expect("proxy verifier");
        let first = verified_proxy_evidence(
            &verifier,
            domain,
            buzz_auth::ProofTransport::Nip98,
            "/upload?slot=a",
            b"body-a",
            &[1; 16],
        )
        .await;
        let mixed = verified_proxy_evidence(
            &verifier,
            domain,
            buzz_auth::ProofTransport::Blossom,
            "/upload?slot=b",
            b"body-b",
            &[2; 16],
        )
        .await;

        assert!(matches!(
            verified_transport_replay_claim(
                domain,
                *first.request_fingerprint(),
                first.transport(),
                *first.transport_context_fingerprint(),
                &mixed,
            ),
            Err(AdmissionCommitError::InvalidRequest)
        ));
        let unconsumed = verified_transport_replay_claim(
            domain,
            *mixed.request_fingerprint(),
            mixed.transport(),
            *mixed.transport_context_fingerprint(),
            &mixed,
        )
        .expect("failed comparison did not consume or mutate the evidence claim");
        assert_eq!(
            unconsumed.claim_key(),
            mixed.nonce_claim().claim_key(),
            "only an exact three-way match can materialize the replay claim"
        );
    }

    async fn verified_proxy_evidence(
        verifier: &TrustedProxyProvenanceVerifier,
        domain: buzz_core::CommunityId,
        transport: buzz_auth::ProofTransport,
        path: &str,
        body: &[u8],
        nonce: &[u8],
    ) -> SealedTransportEvidence {
        let request = TrustedProxyRequest::from_server_request(
            domain,
            transport,
            "POST",
            "relay.example.com:443",
            path,
            body,
        )
        .expect("trusted request");
        let assertion = format!("Bearer {}", proxy_assertion());
        let assertion_digest: [u8; 32] = Sha256::digest(proxy_assertion().as_bytes()).into();
        let body_digest: [u8; 32] = Sha256::digest(body).into();
        let transport_code = match transport {
            buzz_auth::ProofTransport::Nip42 => 1,
            buzz_auth::ProofTransport::Nip98 => 2,
            buzz_auth::ProofTransport::GitSmartHttpSession => 3,
            buzz_auth::ProofTransport::Blossom => 4,
        };
        let mut input = b"NIP-FI-PROXY-1".to_vec();
        for field in [
            PROXY_NOW.to_be_bytes().as_slice(),
            nonce,
            &assertion_digest,
            domain.as_uuid().as_bytes(),
            b"POST".as_slice(),
            b"relay.example.com:443".as_slice(),
            path.as_bytes(),
            &body_digest,
            &[transport_code],
        ] {
            input.extend_from_slice(&(field.len() as u64).to_be_bytes());
            input.extend_from_slice(field);
        }
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&PROXY_SECRET).expect("HMAC key");
        mac.update(&input);
        let provenance = format!(
            "v1.{PROXY_NOW}.{}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(nonce),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
        );
        verifier
            .verify(
                &[
                    HttpHeaderField::new(ASSERTION_HEADER_NAME, assertion.as_bytes()),
                    HttpHeaderField::new(PROVENANCE_HEADER_NAME, provenance.as_bytes()),
                ],
                &request,
                chrono::Utc
                    .timestamp_opt(PROXY_NOW as i64, 0)
                    .single()
                    .expect("test time"),
                &EmptyReplayReader,
            )
            .await
            .expect("valid proxy evidence")
    }

    #[test]
    fn upload_routes_distinguish_standard_and_legacy_modes() {
        assert_eq!(
            upload_route_mode("/upload").expect("standard upload route"),
            UploadRouteMode::Upload
        );
        assert_eq!(
            upload_route_mode("/media/upload").expect("legacy upload route"),
            UploadRouteMode::LegacyMedia
        );
        assert!(matches!(
            upload_route_mode("/media"),
            Err(MediaError::NotFound)
        ));
    }

    #[test]
    fn upload_attribution_snapshot_owns_captured_values() {
        let ip_header = Some("x-client-ip".to_owned());
        let port_header = Some("x-client-port".to_owned());
        let mut headers = HeaderMap::new();
        headers.insert("x-client-ip", "8.8.8.8".parse().expect("IP header"));
        headers.insert("x-client-port", "443".parse().expect("port header"));
        let mut mutable_name = "admitted name".to_owned();

        let snapshot = UploadAttributionSnapshot::capture(
            true,
            Some(mutable_name.clone()),
            &ip_header,
            &port_header,
            &headers,
        );

        mutable_name.replace_range(.., "later profile");
        headers.insert(
            "x-client-ip",
            "1.1.1.1".parse().expect("replacement IP header"),
        );
        headers.insert(
            "x-client-port",
            "8443".parse().expect("replacement port header"),
        );

        let attribution = snapshot
            .into_attribution()
            .expect("enabled attribution snapshot");
        assert_eq!(attribution.uploader_name.as_deref(), Some("admitted name"));
        assert_eq!(
            attribution.net.ip,
            Some("8.8.8.8".parse().expect("expected IP"))
        );
        assert_eq!(attribution.net.port, Some(443));
    }

    #[test]
    fn upload_attribution_disabled_captures_nothing() {
        let headers = HeaderMap::new();
        assert!(UploadAttributionSnapshot::capture(
            false,
            Some("unused".to_owned()),
            &Some("x-client-ip".to_owned()),
            &Some("x-client-port".to_owned()),
            &headers,
        )
        .into_attribution()
        .is_none());
    }

    #[test]
    fn upload_attribution_snapshot_fails_empty_without_a_public_ip() {
        let ip_header = Some("x-client-ip".to_owned());
        let port_header = Some("x-client-port".to_owned());
        for raw_ip in [None, Some("not-an-ip"), Some("10.0.0.1")] {
            let mut headers = HeaderMap::new();
            if let Some(raw_ip) = raw_ip {
                headers.insert("x-client-ip", raw_ip.parse().expect("IP header"));
            }
            headers.insert("x-client-port", "443".parse().expect("port header"));

            let attribution =
                UploadAttributionSnapshot::capture(true, None, &ip_header, &port_header, &headers)
                    .into_attribution()
                    .expect("enabled attribution snapshot");
            assert_eq!(attribution.net, UploadNetworkInfo::default());
        }
    }

    #[test]
    fn proprietary_iso_bmff_brand_still_uses_video_pipeline() {
        let bytes = b"\x00\x00\x00\x18ftypPRIV\x00\x00\x00\x00isommp42";
        assert!(infer::get(bytes).is_none());
        assert!(should_stream_as_video(bytes));
    }

    async fn test_state() -> Arc<AppState> {
        test_state_with_corporate_identity(false).await
    }

    async fn test_state_with_corporate_identity(require_corporate_identity: bool) -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.nip_fi_mode = buzz_auth::NipFiMode::Off;
        config.corporate_identity.require = require_corporate_identity;
        if require_corporate_identity {
            config.corporate_identity.jwks_uri = "http://127.0.0.1:9/jwks".to_string();
            config.corporate_identity.issuer = "https://idp.example".to_string();
            config.corporate_identity.audience = "buzz-relay".to_string();
        }
        build_media_test_state(config, None).await
    }

    async fn enforced_canonical_media_state(
        jwks: Vec<u8>,
        jwks_requests: Arc<AtomicUsize>,
        storage_endpoint: String,
    ) -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.nip_fi_mode = buzz_auth::NipFiMode::Enforce;
        config.corporate_identity.require = true;
        config.corporate_identity.jwt_header = "x-buzz-identity-token".to_owned();
        config.media.s3_endpoint = storage_endpoint;
        let policy = buzz_auth::CanonicalVerifierPolicy::new(
            "https://idp.example".to_owned(),
            "buzz-relay".to_owned(),
            "sub".to_owned(),
            Some("buzz_npub".to_owned()),
            0,
            3_600,
        )
        .expect("canonical media verifier policy");
        let verifier = Arc::new(
            crate::authorization_runtime::DynamicVerifier::new(
                policy,
                "https://idp.example".to_owned(),
                crate::authorization_runtime::JwksSourceConfig::JwksUri(
                    url::Url::parse("https://idp.example/jwks").expect("canonical test JWKS URL"),
                ),
                crate::authorization_runtime::JwksRefreshPolicy::new(
                    64 * 1024,
                    Duration::from_secs(2),
                    Duration::from_secs(300),
                )
                .expect("canonical test refresh policy"),
                Arc::new(CountingJwksLoader {
                    document: jwks,
                    requests: jwks_requests,
                }),
            )
            .expect("canonical media verifier"),
        );
        let snapshot = verifier
            .refresh(chrono::Utc::now())
            .await
            .expect("load canonical media keys");
        let runtime =
            crate::authorization_runtime::InstalledAuthorizationRuntime::for_canonical_assertion_test(
                verifier,
                snapshot,
            );
        build_media_test_state(config, Some(runtime)).await
    }

    async fn build_media_test_state(
        mut config: crate::config::Config,
        authorization_runtime: Option<crate::authorization_runtime::InstalledAuthorizationRuntime>,
    ) -> Arc<AppState> {
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.media_uploads_per_minute = 1;
        config.media_max_concurrent_uploads = 2;
        config.media_max_concurrent_uploads_per_pubkey = 1;

        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        db.ensure_configured_community("relay.example")
            .await
            .expect("seed relay.example community for host-bound media tests");
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = match authorization_runtime {
            Some(authorization_runtime) => AppState::new_with_authorization_runtime(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
                authorization_runtime,
            ),
            None => AppState::new(
                config,
                db,
                redis_pool,
                audit,
                pubsub,
                auth,
                search,
                workflow_engine,
                nostr::Keys::generate(),
                media_storage,
            ),
        };
        Arc::new(state)
    }

    async fn media_get_auth_router() -> axum::Router {
        let state = test_state().await;
        axum::Router::new()
            .route(
                "/media/{sha256_ext}",
                axum::routing::get(get_blob).head(head_blob),
            )
            .with_state(state)
    }

    fn media_get_auth_header(keys: &Keys, tags: Vec<Tag>) -> String {
        let event = EventBuilder::new(Kind::from(24242), "Get media")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign get auth");
        format!(
            "Nostr {}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
        )
    }

    fn media_get_tags_for(host: &str, sha256: Option<&str>) -> Vec<Tag> {
        let now = Timestamp::now().as_secs();
        let expiration = (now + 300).to_string();
        let mut tags = vec![
            Tag::parse(["t", "get"]).expect("t tag"),
            Tag::parse(["expiration", &expiration]).expect("expiration tag"),
            Tag::parse(["server", host]).expect("server tag"),
        ];
        if let Some(sha256) = sha256 {
            tags.push(Tag::parse(["x", sha256]).expect("x tag"));
        }
        tags
    }

    fn media_request(method: &str, auth: Option<String>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("/media/{VALID_HASH}.jpg"))
            .header(header::HOST, "relay.example");
        if let Some(auth) = auth {
            builder = builder.header(header::AUTHORIZATION, auth);
        }
        builder.body(Body::empty()).expect("request")
    }

    fn test_http_response(status: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    async fn spawn_counting_http_server(
        response: String,
    ) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind media test HTTP server");
        let address = listener.local_addr().expect("media test server address");
        let requests = Arc::new(AtomicUsize::new(0));
        let request_count = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                request_count.fetch_add(1, Ordering::SeqCst);
                let mut request = [0_u8; 2_048];
                let Ok(bytes_read) = stream.read(&mut request).await else {
                    return;
                };
                if bytes_read == 0 || stream.write_all(response.as_bytes()).await.is_err() {
                    return;
                }
            }
        });
        (format!("http://{address}"), requests, server)
    }

    fn media_identity_claims(
        subject: &str,
        pubkey: nostr::PublicKey,
        issued_at: i64,
        expires_at: i64,
    ) -> serde_json::Value {
        serde_json::json!({
            "iss": "https://idp.example",
            "aud": "buzz-relay",
            "sub": subject,
            "buzz_npub": pubkey.to_hex(),
            "iat": issued_at,
            "nbf": issued_at,
            "exp": expires_at,
        })
    }

    #[tokio::test]
    async fn media_reads_reject_unauthenticated_get_and_head_before_sidecar_gate() {
        for method in ["GET", "HEAD"] {
            let response = media_get_auth_router()
                .await
                .oneshot(media_request(method, None))
                .await
                .expect("response");

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method}");
        }
    }

    #[tokio::test]
    async fn media_read_with_valid_server_scoped_token_reaches_sidecar_gate() {
        let keys = Keys::generate();
        let auth = media_get_auth_header(&keys, media_get_tags_for("relay.example", None));
        let response = media_get_auth_router()
            .await
            .oneshot(media_request("GET", Some(auth)))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn protected_media_get_and_head_reject_invalid_assertions_before_storage() {
        const KID: &str = "media-canonical-test";
        let keys = Keys::generate();
        let other_keys = Keys::generate();
        let now = chrono::Utc::now().timestamp();
        let valid_claims =
            media_identity_claims("media-user", keys.public_key(), now - 1, now + 600);
        let expired_claims =
            media_identity_claims("media-user", keys.public_key(), now - 600, now - 120);
        let mismatched_claims =
            media_identity_claims("media-user", other_keys.public_key(), now - 1, now + 600);
        let current_jwk = crate::corporate_identity::canonical_test_support::jwk(0, KID);
        let jwks = serde_json::to_string(&jsonwebtoken::jwk::JwkSet {
            keys: vec![current_jwk],
        })
        .expect("serialize media test JWKS");
        let jwks_requests = Arc::new(AtomicUsize::new(0));
        let (storage_endpoint, storage_requests, storage_server) =
            spawn_counting_http_server(test_http_response("404 Not Found", "text/plain", "")).await;
        let state = enforced_canonical_media_state(
            jwks.into_bytes(),
            Arc::clone(&jwks_requests),
            storage_endpoint,
        )
        .await;
        assert_eq!(storage_requests.load(Ordering::SeqCst), 0);
        let app = axum::Router::new()
            .route(
                "/media/{sha256_ext}",
                axum::routing::get(get_blob).head(head_blob),
            )
            .with_state(state);
        let expired =
            crate::corporate_identity::canonical_test_support::signed_jwt(&expired_claims, 0, KID);
        let rotated_key =
            crate::corporate_identity::canonical_test_support::signed_jwt(&valid_claims, 1, KID);
        let mismatched = crate::corporate_identity::canonical_test_support::signed_jwt(
            &mismatched_claims,
            0,
            KID,
        );
        let cases = [
            ("absent", None),
            ("expired", Some(expired)),
            ("revoked-key", Some(rotated_key)),
            ("mismatched", Some(mismatched)),
        ];

        for method in ["GET", "HEAD"] {
            for (case, assertion) in &cases {
                let auth = media_get_auth_header(&keys, media_get_tags_for("relay.example", None));
                let mut request = media_request(method, Some(auth));
                if let Some(assertion) = assertion {
                    request.headers_mut().insert(
                        "x-buzz-identity-token",
                        assertion.parse().expect("identity assertion header"),
                    );
                }
                let response = app
                    .clone()
                    .oneshot(request)
                    .await
                    .expect("protected media response");

                assert_eq!(
                    response.status(),
                    StatusCode::UNAUTHORIZED,
                    "{method}/{case}"
                );
                assert_eq!(
                    storage_requests.load(Ordering::SeqCst),
                    0,
                    "{method}/{case} reached object storage before canonical admission"
                );
            }
        }

        assert_eq!(
            jwks_requests.load(Ordering::SeqCst),
            1,
            "one cached current-key fetch must cover every presented assertion"
        );
        assert_eq!(storage_requests.load(Ordering::SeqCst), 0);
        storage_server.abort();
    }

    #[tokio::test]
    async fn media_read_rejects_upload_verb_wrong_server_and_wrong_x() {
        let keys = Keys::generate();
        let now = Timestamp::now().as_secs();
        let expiration = (now + 300).to_string();
        let cases = [
            vec![
                Tag::parse(["t", "upload"]).expect("t tag"),
                Tag::parse(["expiration", &expiration]).expect("expiration tag"),
                Tag::parse(["server", "relay.example"]).expect("server tag"),
            ],
            vec![
                Tag::parse(["t", "get"]).expect("t tag"),
                Tag::parse(["expiration", &expiration]).expect("expiration tag"),
                Tag::parse(["server", "evil.example"]).expect("server tag"),
            ],
            vec![
                Tag::parse(["t", "get"]).expect("t tag"),
                Tag::parse(["expiration", &expiration]).expect("expiration tag"),
                Tag::parse(["x", &"f".repeat(64)]).expect("x tag"),
            ],
        ];

        for tags in cases {
            let auth = media_get_auth_header(&keys, tags);
            let response = media_get_auth_router()
                .await
                .oneshot(media_request("GET", Some(auth)))
                .await
                .expect("response");

            assert_ne!(response.status(), StatusCode::NOT_FOUND);
            assert!(
                response.status() == StatusCode::UNAUTHORIZED
                    || response.status() == StatusCode::FORBIDDEN,
                "unexpected status {}",
                response.status()
            );
        }
    }

    #[tokio::test]
    async fn media_read_accepts_range_header_only_after_auth() {
        let keys = Keys::generate();
        let auth = media_get_auth_header(&keys, media_get_tags_for("relay.example", None));
        let mut request = media_request("GET", Some(auth));
        request
            .headers_mut()
            .insert(header::RANGE, "bytes=0-0".parse().expect("range header"));

        let response = media_get_auth_router()
            .await
            .oneshot(request)
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn upload_rate_limiter_is_scoped_by_community() {
        let state = test_state().await;
        let pubkey = nostr::Keys::generate().public_key();
        let community_a = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xBBBB));

        assert!(!upload_rate_limited(&state, community_a, &pubkey));
        assert!(upload_rate_limited(&state, community_a, &pubkey));
        assert!(
            !upload_rate_limited(&state, community_b, &pubkey),
            "A's exhausted upload budget must not rate-limit the same key in B"
        );
    }

    #[tokio::test]
    async fn upload_concurrency_limit_is_scoped_by_community() {
        let state = test_state().await;
        let pubkey = nostr::Keys::generate().public_key();
        let community_a = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = buzz_core::CommunityId::from_uuid(Uuid::from_u128(0xBBBB));

        let permit_a =
            acquire_upload_permit(&state, community_a, &pubkey).expect("first A upload allowed");
        assert!(matches!(
            acquire_upload_permit(&state, community_a, &pubkey),
            Err(MediaError::UploadConcurrencyLimitReached)
        ));
        let permit_b = acquire_upload_permit(&state, community_b, &pubkey)
            .expect("A's in-flight upload must not block B");

        drop(permit_b);
        drop(permit_a);
    }

    #[test]
    fn test_validate_media_path_bare_hash() {
        assert!(validate_media_path(VALID_HASH).is_ok());
    }

    #[test]
    fn test_validate_media_path_hash_ext() {
        for ext in &["jpg", "png", "gif", "webp", "mp4"] {
            assert!(validate_media_path(&format!("{VALID_HASH}.{ext}")).is_ok());
        }
    }

    #[test]
    fn test_validate_media_path_thumb_jpg_only() {
        assert!(validate_media_path(&format!("{VALID_HASH}.thumb.jpg")).is_ok());
        // Other thumb extensions rejected — thumbnails are always JPEG
        assert!(validate_media_path(&format!("{VALID_HASH}.thumb.png")).is_err());
        assert!(validate_media_path(&format!("{VALID_HASH}.thumb.webp")).is_err());
    }

    #[test]
    fn test_validate_media_path_accepts_generic_exts() {
        // Path validation now accepts any safe ext token — the deny-list for
        // dangerous *content* lives in the upload validator, not here. The
        // sidecar ext comparison is the authoritative check at serve time.
        assert!(validate_media_path(&format!("{VALID_HASH}.pdf")).is_ok());
        assert!(validate_media_path(&format!("{VALID_HASH}.docx")).is_ok());
        assert!(validate_media_path(&format!("{VALID_HASH}.zip")).is_ok());
        assert!(validate_media_path(&format!("{VALID_HASH}.mp3")).is_ok());
        assert!(validate_media_path(&format!("{VALID_HASH}.bin")).is_ok());
    }

    #[test]
    fn test_validate_media_path_rejects_malformed_ext() {
        // Reject ext tokens that aren't safe: uppercase, too long, special chars.
        assert!(validate_media_path(&format!("{VALID_HASH}.PDF")).is_err());
        assert!(validate_media_path(&format!("{VALID_HASH}.toolongext")).is_err());
        // 3-segment paths are only valid as the `.thumb.jpg` variant; a
        // hash.tar.gz form is rejected (compound extensions aren't a thing here —
        // the canonical ext is a single token like `gz`).
        assert!(validate_media_path(&format!("{VALID_HASH}.tar.gz")).is_err());
    }

    #[test]
    fn test_is_safe_ext() {
        assert!(is_safe_ext("jpg"));
        assert!(is_safe_ext("docx"));
        assert!(is_safe_ext("7z"));
        assert!(is_safe_ext("bin"));
        assert!(!is_safe_ext("")); // empty
        assert!(!is_safe_ext("PDF")); // uppercase
        assert!(!is_safe_ext("ta r")); // space
        assert!(!is_safe_ext("toolongext")); // > 8 chars
        assert!(!is_safe_ext("../etc")); // traversal chars
    }

    #[test]
    fn test_validate_media_path_rejects_short_hash() {
        assert!(validate_media_path("abc123.jpg").is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_uppercase_hash() {
        let upper = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        assert!(validate_media_path(&format!("{upper}.jpg")).is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_traversal() {
        assert!(validate_media_path("../etc/passwd").is_err());
        assert!(validate_media_path(&format!("../{VALID_HASH}.jpg")).is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_too_many_segments() {
        assert!(validate_media_path(&format!("{VALID_HASH}.thumb.jpg.extra")).is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_empty() {
        assert!(validate_media_path("").is_err());
    }

    #[test]
    fn test_validate_media_path_rejects_upload_record_keys() {
        // The `_uploads/` per-event records (and `_meta/` sidecars) must be
        // unreachable through the serve path. Axum's single path segment
        // can't even contain `/`, but assert the validator rejects these
        // shapes outright so the property survives any routing change.
        assert!(validate_media_path("_uploads").is_err());
        assert!(validate_media_path(&format!("_uploads/c/{VALID_HASH}/01J.json")).is_err());
        assert!(validate_media_path("_meta").is_err());
        assert!(validate_media_path(&format!("_meta/c/{VALID_HASH}.json")).is_err());
        // Suffix-style metadata keys are also non-servable (>3 segments / bad ext).
        assert!(validate_media_path(&format!("{VALID_HASH}.png.metadata")).is_err());
    }

    #[test]
    fn media_base_url_for_tenant_uses_tenant_host_and_http_scheme() {
        assert_eq!(
            media_base_url_for_tenant("wss://config.example", "tenant-b.example"),
            "https://tenant-b.example/media"
        );
        assert_eq!(
            media_base_url_for_tenant("ws://config.example", "localhost:3100"),
            "http://localhost:3100/media"
        );
    }

    #[test]
    fn rewrite_descriptor_urls_for_tenant_replaces_global_media_host() {
        let hash = "a".repeat(64);
        let mut descriptor = BlobDescriptor {
            url: format!("https://primary.example/media/{hash}.jpg"),
            sha256: hash.clone(),
            size: 42,
            mime_type: "image/jpeg".to_string(),
            uploaded: 1700000000,
            dim: Some("1x1".to_string()),
            blurhash: None,
            thumb: Some(format!("https://primary.example/media/{hash}.thumb.jpg")),
            duration: None,
        };

        rewrite_descriptor_urls_for_tenant(
            &mut descriptor,
            "wss://primary.example",
            "tenant-b.example",
        );

        assert_eq!(
            descriptor.url,
            format!("https://tenant-b.example/media/{hash}.jpg")
        );
        assert_eq!(
            descriptor.thumb,
            Some(format!("https://tenant-b.example/media/{hash}.thumb.jpg"))
        );
    }

    #[test]
    fn test_parse_byte_range_basic() {
        assert_eq!(parse_byte_range("bytes=0-499", 1000), Some((0, 499)));
        assert_eq!(parse_byte_range("bytes=500-999", 1000), Some((500, 999)));
    }

    #[test]
    fn test_parse_byte_range_open_ended() {
        // "bytes=500-" means from 500 to end of file
        assert_eq!(parse_byte_range("bytes=500-", 1000), Some((500, u64::MAX)));
    }

    #[test]
    fn test_parse_byte_range_suffix() {
        // "bytes=-500" on a 1000-byte file → last 500 bytes
        assert_eq!(parse_byte_range("bytes=-500", 1000), Some((500, 999)));
    }

    #[test]
    fn test_parse_byte_range_suffix_larger_than_file() {
        // Suffix larger than file → clamp to start of file
        assert_eq!(parse_byte_range("bytes=-5000", 1000), Some((0, 999)));
    }

    #[test]
    fn test_parse_byte_range_suffix_zero() {
        // "bytes=-0" is nonsensical → None
        assert_eq!(parse_byte_range("bytes=-0", 1000), None);
    }

    #[test]
    fn test_parse_byte_range_suffix_empty_file() {
        // Suffix on empty file → None
        assert_eq!(parse_byte_range("bytes=-500", 0), None);
    }

    #[test]
    fn test_parse_byte_range_rejects_inverted() {
        // start > end is invalid
        assert_eq!(parse_byte_range("bytes=999-0", 1000), None);
    }

    #[test]
    fn test_parse_byte_range_rejects_non_bytes_unit() {
        assert_eq!(parse_byte_range("items=0-10", 1000), None);
    }

    #[test]
    fn test_parse_byte_range_rejects_malformed() {
        assert_eq!(parse_byte_range("bytes=abc-def", 1000), None);
        assert_eq!(parse_byte_range("garbage", 1000), None);
        assert_eq!(parse_byte_range("bytes=", 1000), None);
    }

    #[test]
    fn test_parse_byte_range_zero_start() {
        assert_eq!(parse_byte_range("bytes=0-0", 1000), Some((0, 0)));
    }
}
