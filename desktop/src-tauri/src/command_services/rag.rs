use crate::command_services::policy::{
    admission_secrets_are_independent, cache_verified_service, clear_cached_service,
    probe_authenticated_mcp, validate_rag_literal_loopback_mcp_endpoint, AdmissionError,
    KnowledgeServiceKind, ServiceAdmissionPolicy, VerifiedService, RAG_CATALOG_TOOLS,
};
use crate::command_services::ssh::ProtectedFile;
use crate::secret_store::SecretStore;
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use tauri::Manager;

const CONFIG_FILE_NAME: &str = "command-rag.json";
const MAXIMUM_CONFIG_BYTES: u64 = 64 * 1024;
const MAXIMUM_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAXIMUM_ACTIVATION_BYTES: u64 = 1024 * 1024;
const MAXIMUM_CATALOGUE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RagConfig {
    schema_version: u32,
    endpoint: String,
    state_root: PathBuf,
    expected_server_identity: String,
    expected_active_snapshot_id: String,
    trusted_signer_fingerprint: String,
    credential_key: String,
    attestation_credential_key: String,
    tool_allowlist: Vec<String>,
    maximum_snapshot_age_hours: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RagServiceStatus {
    Ready,
    NotConfigured,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RagValidationState {
    Verified,
    Failed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RagFreshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RagServiceReadiness {
    status: RagServiceStatus,
    server_identity: Option<String>,
    active_snapshot_id: Option<String>,
    signature_fingerprint: Option<String>,
    snapshot_time: Option<String>,
    last_successful_activation: Option<String>,
    freshness: RagFreshness,
    validation: RagValidationState,
    endpoint: Option<String>,
    tool_allowlist: Vec<String>,
    observed_at: String,
    error: Option<String>,
    #[serde(skip)]
    admitted: Option<VerifiedService>,
    #[serde(skip)]
    verified_physical_collections: Vec<String>,
    verified_logical_collections: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RagError {
    NotConfigured,
    InvalidConfig,
    AuthenticationFailed,
    ServiceUnavailable,
    InvalidResponse,
    ResponseTooLarge,
    ServerIdentityMismatch,
    ToolCatalogMismatch,
    SnapshotIdentityMismatch,
    SnapshotHashMismatch,
    ActivationHashMismatch,
    SignerMismatch,
    SnapshotStale,
    ValidationFailed,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RagSnapshotError {
    Invalid,
    Changed,
}

/// A cryptographically verified active RAG snapshot frozen for one brief run.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedRagSnapshot {
    snapshot_id: String,
    verified_at: String,
    snapshot_time: String,
    physical_collections: Vec<String>,
    logical_collections: Vec<String>,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
impl VerifiedRagSnapshot {
    pub(crate) fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }

    pub(crate) fn verified_at(&self) -> &str {
        &self.verified_at
    }

    pub(crate) fn snapshot_time(&self) -> &str {
        &self.snapshot_time
    }

    pub(crate) fn physical_collections(&self) -> &[String] {
        &self.physical_collections
    }

    pub(crate) fn logical_collections(&self) -> &[String] {
        &self.logical_collections
    }

    pub(crate) fn verify_unchanged(
        &self,
        observed_snapshot_id: &str,
    ) -> Result<(), RagSnapshotError> {
        if !valid_digest(observed_snapshot_id) {
            return Err(RagSnapshotError::Invalid);
        }
        if self.snapshot_id != observed_snapshot_id {
            return Err(RagSnapshotError::Changed);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn for_test(snapshot_id: &str, verified_at: &str, snapshot_time: &str) -> Self {
        Self {
            snapshot_id: snapshot_id.to_string(),
            verified_at: verified_at.to_string(),
            snapshot_time: snapshot_time.to_string(),
            physical_collections: vec!["documents".to_string()],
            logical_collections: vec!["navy-publications".to_string()],
        }
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
pub(crate) fn verified_snapshot_from_readiness(
    readiness: &RagServiceReadiness,
) -> Result<VerifiedRagSnapshot, RagSnapshotError> {
    if readiness.status != RagServiceStatus::Ready
        || readiness.validation != RagValidationState::Verified
        || readiness.freshness != RagFreshness::Fresh
    {
        return Err(RagSnapshotError::Invalid);
    }
    let snapshot_id = readiness
        .active_snapshot_id
        .as_deref()
        .filter(|value| valid_digest(value))
        .ok_or(RagSnapshotError::Invalid)?;
    let snapshot_time = readiness
        .snapshot_time
        .as_deref()
        .filter(|value| DateTime::parse_from_rfc3339(value).is_ok())
        .ok_or(RagSnapshotError::Invalid)?;
    if DateTime::parse_from_rfc3339(&readiness.observed_at).is_err() {
        return Err(RagSnapshotError::Invalid);
    }
    Ok(VerifiedRagSnapshot {
        snapshot_id: snapshot_id.to_string(),
        verified_at: readiness.observed_at.clone(),
        snapshot_time: snapshot_time.to_string(),
        physical_collections: readiness.verified_physical_collections.clone(),
        logical_collections: readiness.verified_logical_collections.clone(),
    })
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RagEvidenceRecord {
    pub(crate) source_id: String,
    pub(crate) collection: String,
    pub(crate) document_id: String,
    pub(crate) chunk_id: String,
    pub(crate) retrieved_at: String,
    pub(crate) location: String,
    pub(crate) quote: String,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
pub(crate) fn extract_verified_rag_evidence(
    snapshot: &VerifiedRagSnapshot,
    expected_query: &str,
    value: &Value,
) -> Result<Vec<RagEvidenceRecord>, RagSnapshotError> {
    let policy = crate::command_services::policy::AdviserContextPolicy {
        active_snapshot_id: snapshot.snapshot_id.clone(),
        allowed_apple_ids: std::collections::BTreeSet::new(),
        allowed_file_paths: std::collections::BTreeSet::new(),
    };
    crate::command_services::policy::validate_rag_context(&policy, value)
        .map_err(|_| RagSnapshotError::Invalid)?;
    if value.get("query").and_then(Value::as_str) != Some(expected_query) {
        return Err(RagSnapshotError::Invalid);
    }
    let retrieved_at = value
        .get("retrieved_at")
        .and_then(Value::as_str)
        .ok_or(RagSnapshotError::Invalid)?;
    value
        .get("results")
        .and_then(Value::as_array)
        .ok_or(RagSnapshotError::Invalid)?
        .iter()
        .map(|result| {
            let source = result
                .get("source")
                .and_then(Value::as_object)
                .ok_or(RagSnapshotError::Invalid)?;
            if source
                .get("collection")
                .and_then(Value::as_str)
                .is_none_or(|collection| {
                    !snapshot
                        .logical_collections
                        .iter()
                        .any(|allowed| allowed == collection)
                })
            {
                return Err(RagSnapshotError::Invalid);
            }
            let text = |key: &str| {
                source
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or(RagSnapshotError::Invalid)
            };
            let location = serde_jcs::to_vec(
                source
                    .get("quoted_location")
                    .ok_or(RagSnapshotError::Invalid)?,
            )
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .ok_or(RagSnapshotError::Invalid)?;
            Ok(RagEvidenceRecord {
                source_id: text("source_id")?,
                collection: text("collection")?,
                document_id: text("document_id")?,
                chunk_id: text("chunk_id")?,
                retrieved_at: retrieved_at.to_string(),
                location,
                quote: result
                    .get("quoted_text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or(RagSnapshotError::Invalid)?,
            })
        })
        .collect()
}

impl RagError {
    fn code(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::InvalidConfig => "invalid_config",
            Self::AuthenticationFailed => "authentication_failed",
            Self::ServiceUnavailable => "local_service_unavailable",
            Self::InvalidResponse => "invalid_response",
            Self::ResponseTooLarge => "response_too_large",
            Self::ServerIdentityMismatch => "server_identity_mismatch",
            Self::ToolCatalogMismatch => "tool_catalog_mismatch",
            Self::SnapshotIdentityMismatch => "snapshot_identity_mismatch",
            Self::SnapshotHashMismatch => "snapshot_hash_mismatch",
            Self::ActivationHashMismatch => "activation_hash_mismatch",
            Self::SignerMismatch => "signer_mismatch",
            Self::SnapshotStale => "snapshot_stale",
            Self::ValidationFailed => "validation_failed",
        }
    }
}

#[derive(Clone, Debug)]
struct McpAttestation {
    server_identity: String,
    tools: Vec<String>,
    snapshot_status: Value,
}

trait McpProbe {
    fn attest(
        &self,
        config: &RagConfig,
        bearer_token: &str,
        attestation_secret: &str,
    ) -> Result<McpAttestation, RagError>;
}

struct HttpMcpProbe;

impl McpProbe for HttpMcpProbe {
    fn attest(
        &self,
        config: &RagConfig,
        bearer_token: &str,
        attestation_secret: &str,
    ) -> Result<McpAttestation, RagError> {
        let identity = format!(
            "snapshot:{};signer-sha256:{}",
            config.expected_active_snapshot_id, config.trusted_signer_fingerprint
        );
        let attestation = probe_authenticated_mcp(
            &config.endpoint,
            bearer_token,
            attestation_secret,
            "rag",
            &identity,
            Some("get_snapshot_status"),
        )
        .map_err(map_admission_error)?;
        Ok(McpAttestation {
            server_identity: attestation.server_identity,
            tools: attestation.tools,
            snapshot_status: attestation.status.ok_or(RagError::InvalidResponse)?,
        })
    }
}

fn map_admission_error(error: AdmissionError) -> RagError {
    match error {
        AdmissionError::AuthenticationUnavailable => RagError::AuthenticationFailed,
        AdmissionError::ServiceUnavailable => RagError::ServiceUnavailable,
        AdmissionError::ResponseTooLarge => RagError::ResponseTooLarge,
        AdmissionError::ServerIdentityMismatch => RagError::ServerIdentityMismatch,
        AdmissionError::ActiveIdentityMismatch => RagError::SnapshotIdentityMismatch,
        AdmissionError::MissingRequiredTool | AdmissionError::UnexpectedToolCatalog => {
            RagError::ToolCatalogMismatch
        }
        AdmissionError::EndpointNotLiteralLoopback
        | AdmissionError::InvalidResponse
        | AdmissionError::InvalidAttestation => RagError::InvalidConfig,
    }
}

fn exact_object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>, RagError> {
    let object = value.as_object().ok_or(RagError::InvalidResponse)?;
    if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
        return Err(RagError::InvalidResponse);
    }
    Ok(object)
}

fn field_text<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, RagError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .ok_or(RagError::InvalidResponse)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, RagError> {
    serde_jcs::to_vec(value).map_err(|_| RagError::InvalidResponse)
}

fn digest(value: &Value) -> Result<String, RagError> {
    canonical_json_bytes(value).map(|bytes| crate::command_services::policy::sha256_hex(&bytes))
}

fn validate_config(config: &RagConfig) -> Result<(), RagError> {
    let unique_tools = config
        .tool_allowlist
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    if config.schema_version != 1
        || validate_rag_literal_loopback_mcp_endpoint(&config.endpoint).is_err()
        || config.expected_server_identity != "rag"
        || !valid_digest(&config.expected_active_snapshot_id)
        || !valid_digest(&config.trusted_signer_fingerprint)
        || !config.state_root.is_absolute()
        || !config.credential_key.starts_with("rag.")
        || config.credential_key.len() <= "rag.".len()
        || config.credential_key.len() > 128
        || !config
            .credential_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || !config.attestation_credential_key.starts_with("rag.")
        || config.attestation_credential_key.len() <= "rag.".len()
        || config.attestation_credential_key.len() > 128
        || !config
            .attestation_credential_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || config.attestation_credential_key == config.credential_key
        || !(1..=24 * 30).contains(&config.maximum_snapshot_age_hours)
        || config.tool_allowlist.len() != RAG_CATALOG_TOOLS.len()
        || unique_tools.len() != config.tool_allowlist.len()
        || config
            .tool_allowlist
            .iter()
            .any(|tool| !RAG_CATALOG_TOOLS.contains(&tool.as_str()))
    {
        return Err(RagError::InvalidConfig);
    }
    Ok(())
}

struct SignedSnapshotDocuments<'a> {
    manifest: &'a Value,
    catalogue: &'a Value,
}

fn verify_rag_service(
    config: &RagConfig,
    bearer_token: &str,
    attestation_secret: &str,
    documents: SignedSnapshotDocuments<'_>,
    activation: &Value,
    probe: &dyn McpProbe,
    observed_at: &str,
) -> Result<RagServiceReadiness, RagError> {
    let manifest = documents.manifest;
    let catalogue = documents.catalogue;
    validate_config(config)?;
    let observed = DateTime::parse_from_rfc3339(observed_at)
        .map_err(|_| RagError::InvalidConfig)?
        .with_timezone(&Utc);
    if bearer_token.len() < 16
        || bearer_token.len() > 256
        || bearer_token
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        || !admission_secrets_are_independent(bearer_token, attestation_secret)
    {
        return Err(RagError::AuthenticationFailed);
    }

    let manifest_object = exact_object(
        manifest,
        &[
            "format",
            "snapshot_time",
            "service",
            "signer",
            "retrieval_models",
            "collections",
            "catalogue",
            "golden_queries",
            "objects",
        ],
    )?;
    if field_text(manifest_object, "format")? != "rag-snapshot-v1" {
        return Err(RagError::InvalidResponse);
    }
    let snapshot_id = digest(manifest)?;
    if snapshot_id != config.expected_active_snapshot_id {
        return Err(RagError::SnapshotHashMismatch);
    }
    let signer = exact_object(
        manifest_object
            .get("signer")
            .ok_or(RagError::InvalidResponse)?,
        &["algorithm", "public_key_sha256"],
    )?;
    if field_text(signer, "algorithm")? != "ed25519"
        || field_text(signer, "public_key_sha256")? != config.trusted_signer_fingerprint
    {
        return Err(RagError::SignerMismatch);
    }
    let snapshot_time = field_text(manifest_object, "snapshot_time")?;
    let snapshot_timestamp = DateTime::parse_from_rfc3339(snapshot_time)
        .map_err(|_| RagError::InvalidResponse)?
        .with_timezone(&Utc);
    let age = observed.signed_duration_since(snapshot_timestamp);
    if age.num_seconds() < -300
        || age > chrono::Duration::hours(i64::from(config.maximum_snapshot_age_hours))
    {
        return Err(RagError::SnapshotStale);
    }

    let activation_object = exact_object(
        activation,
        &[
            "format",
            "snapshot_id",
            "manifest_sha256",
            "signer_fingerprint",
            "snapshot_time",
            "service",
            "retrieval_models",
            "collections",
            "golden_object_sha256",
            "golden_queries",
            "activated_at",
        ],
    )?;
    if field_text(activation_object, "format")? != "rag-activation-v2"
        || field_text(activation_object, "snapshot_id")? != snapshot_id
        || field_text(activation_object, "manifest_sha256")? != snapshot_id
        || field_text(activation_object, "signer_fingerprint")? != config.trusted_signer_fingerprint
        || field_text(activation_object, "snapshot_time")? != snapshot_time
        || activation_object.get("service") != manifest_object.get("service")
        || activation_object.get("retrieval_models") != manifest_object.get("retrieval_models")
    {
        return Err(RagError::ValidationFailed);
    }
    let manifest_collections = manifest_object
        .get("collections")
        .and_then(Value::as_array)
        .ok_or(RagError::InvalidResponse)?;
    let (physical_collections, logical_collections) =
        verify_signed_catalogue(manifest_object, manifest_collections, catalogue)?;
    let expected_collections = manifest_collections
        .iter()
        .map(|collection| {
            let collection = collection.as_object().ok_or(RagError::InvalidResponse)?;
            let name = field_text(collection, "name")?;
            Ok(serde_json::json!({
                "name": name,
                "runtime_name": format!("staging-{}-{name}", &snapshot_id[..12]),
                "point_count": collection
                    .get("point_count")
                    .ok_or(RagError::InvalidResponse)?,
                "schema": collection.get("schema").ok_or(RagError::InvalidResponse)?,
            }))
        })
        .collect::<Result<Vec<_>, RagError>>()?;
    let manifest_golden_hash = manifest_object
        .get("golden_queries")
        .and_then(Value::as_object)
        .and_then(|golden| golden.get("sha256"))
        .and_then(Value::as_str)
        .ok_or(RagError::InvalidResponse)?;
    if activation_object.get("collections") != Some(&Value::Array(expected_collections))
        || field_text(activation_object, "golden_object_sha256")? != manifest_golden_hash
    {
        return Err(RagError::ValidationFailed);
    }
    let golden = activation_object
        .get("golden_queries")
        .and_then(Value::as_object)
        .ok_or(RagError::InvalidResponse)?;
    let case_count = golden
        .get("case_count")
        .and_then(Value::as_u64)
        .ok_or(RagError::InvalidResponse)?;
    if golden.get("passed").and_then(Value::as_bool) != Some(true)
        || golden.get("passed_count").and_then(Value::as_u64) != Some(case_count)
        || case_count > 10_000
    {
        return Err(RagError::ValidationFailed);
    }
    let activation_id = digest(activation)?;

    let attestation = probe.attest(config, bearer_token, attestation_secret)?;
    if attestation.server_identity != config.expected_server_identity {
        return Err(RagError::ServerIdentityMismatch);
    }
    let readiness = exact_object(
        &attestation.snapshot_status,
        &[
            "format",
            "active_activation_id",
            "active_snapshot_id",
            "signature_fingerprint",
            "snapshot_time",
            "service",
            "retrieval_models",
            "collections",
            "golden_queries",
            "last_successful_activation",
        ],
    )?;
    if field_text(readiness, "format")? != "rag-readiness-v2"
        || field_text(readiness, "active_activation_id")? != activation_id
        || field_text(readiness, "active_snapshot_id")? != snapshot_id
        || field_text(readiness, "signature_fingerprint")? != config.trusted_signer_fingerprint
        || field_text(readiness, "snapshot_time")? != snapshot_time
        || readiness.get("service") != activation_object.get("service")
        || readiness.get("retrieval_models") != activation_object.get("retrieval_models")
        || readiness.get("collections") != activation_object.get("collections")
        || readiness.get("golden_queries") != activation_object.get("golden_queries")
        || field_text(readiness, "last_successful_activation")?
            != field_text(activation_object, "activated_at")?
    {
        return Err(RagError::ActivationHashMismatch);
    }
    let expected_tools = config
        .tool_allowlist
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let observed_tools = attestation
        .tools
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    if expected_tools != observed_tools || attestation.tools.len() != observed_tools.len() {
        return Err(RagError::ToolCatalogMismatch);
    }
    let admitted = VerifiedService {
        kind: KnowledgeServiceKind::Rag,
        server_identity: attestation.server_identity.clone(),
        endpoint: config.endpoint.clone(),
        bearer_token: bearer_token.to_string(),
        active_identity: snapshot_id.clone(),
        advertised_tools: attestation.tools,
        verified_at: observed_at.to_string(),
    };
    ServiceAdmissionPolicy::for_service(
        KnowledgeServiceKind::Rag,
        &config.expected_server_identity,
        &config.expected_active_snapshot_id,
        RAG_CATALOG_TOOLS,
    )
    .verify(&admitted)
    .map_err(|error| match error {
        crate::command_services::policy::AdmissionError::ServerIdentityMismatch => {
            RagError::ServerIdentityMismatch
        }
        crate::command_services::policy::AdmissionError::ActiveIdentityMismatch => {
            RagError::SnapshotIdentityMismatch
        }
        crate::command_services::policy::AdmissionError::AuthenticationUnavailable => {
            RagError::AuthenticationFailed
        }
        crate::command_services::policy::AdmissionError::MissingRequiredTool
        | crate::command_services::policy::AdmissionError::UnexpectedToolCatalog => {
            RagError::ToolCatalogMismatch
        }
        _ => RagError::InvalidConfig,
    })?;

    Ok(RagServiceReadiness {
        status: RagServiceStatus::Ready,
        server_identity: Some(attestation.server_identity),
        active_snapshot_id: Some(snapshot_id),
        signature_fingerprint: Some(config.trusted_signer_fingerprint.clone()),
        snapshot_time: Some(snapshot_time.to_string()),
        last_successful_activation: Some(
            field_text(readiness, "last_successful_activation")?.to_string(),
        ),
        freshness: RagFreshness::Fresh,
        validation: RagValidationState::Verified,
        endpoint: Some(config.endpoint.clone()),
        tool_allowlist: config.tool_allowlist.clone(),
        observed_at: observed_at.to_string(),
        error: None,
        admitted: Some(admitted),
        verified_physical_collections: physical_collections,
        verified_logical_collections: logical_collections,
    })
}

fn verify_signed_catalogue(
    manifest: &Map<String, Value>,
    manifest_collections: &[Value],
    catalogue: &Value,
) -> Result<(Vec<String>, Vec<String>), RagError> {
    let reference = exact_object(
        manifest.get("catalogue").ok_or(RagError::InvalidResponse)?,
        &["document_count", "path", "sha256"],
    )?;
    if digest(catalogue)? != field_text(reference, "sha256")? {
        return Err(RagError::SnapshotHashMismatch);
    }
    let catalogue = exact_object(catalogue, &["collections", "documents"])?;
    let collections = catalogue
        .get("collections")
        .and_then(Value::as_array)
        .filter(|collections| !collections.is_empty() && collections.len() <= 256)
        .ok_or(RagError::InvalidResponse)?;
    if collections.len() != manifest_collections.len() {
        return Err(RagError::ValidationFailed);
    }
    let physical = collections
        .iter()
        .zip(manifest_collections)
        .map(|(catalogue_collection, manifest_collection)| {
            let catalogue_collection =
                exact_object(catalogue_collection, &["name", "point_count", "schema"])?;
            let manifest_collection = manifest_collection
                .as_object()
                .ok_or(RagError::InvalidResponse)?;
            if catalogue_collection.get("point_count") != manifest_collection.get("point_count")
                || catalogue_collection.get("schema") != manifest_collection.get("schema")
            {
                return Err(RagError::ValidationFailed);
            }
            let name = field_text(catalogue_collection, "name")?;
            if manifest_collection.get("name").and_then(Value::as_str) != Some(name)
                || !valid_catalogue_name(name)
            {
                return Err(RagError::ValidationFailed);
            }
            Ok(name.to_string())
        })
        .collect::<Result<Vec<_>, RagError>>()?;
    if physical.iter().collect::<BTreeSet<_>>().len() != physical.len() {
        return Err(RagError::InvalidResponse);
    }
    let documents = catalogue
        .get("documents")
        .and_then(Value::as_array)
        .filter(|documents| documents.len() <= 100_000)
        .ok_or(RagError::InvalidResponse)?;
    if reference.get("document_count").and_then(Value::as_u64) != Some(documents.len() as u64) {
        return Err(RagError::ValidationFailed);
    }
    let mut document_ids = BTreeSet::new();
    let mut logical = Vec::with_capacity(documents.len());
    for document in documents {
        let document = exact_object(document, &["doc_id", "collection"])?;
        let document_id = field_text(document, "doc_id")?;
        let collection = field_text(document, "collection")?;
        if !valid_catalogue_name(document_id)
            || !valid_catalogue_name(collection)
            || !document_ids.insert(document_id)
        {
            return Err(RagError::InvalidResponse);
        }
        logical.push(collection.to_string());
    }
    logical.sort();
    logical.dedup();
    if logical.is_empty() || logical.len() > 256 {
        return Err(RagError::InvalidResponse);
    }
    Ok((physical, logical))
}

fn valid_catalogue_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn fail_soft_readiness(error: RagError) -> RagServiceReadiness {
    RagServiceReadiness {
        status: if error == RagError::NotConfigured {
            RagServiceStatus::NotConfigured
        } else {
            RagServiceStatus::Unavailable
        },
        server_identity: None,
        active_snapshot_id: None,
        signature_fingerprint: None,
        snapshot_time: None,
        last_successful_activation: None,
        freshness: if error == RagError::SnapshotStale {
            RagFreshness::Stale
        } else {
            RagFreshness::Unknown
        },
        validation: if matches!(
            error,
            RagError::NotConfigured | RagError::ServiceUnavailable
        ) {
            RagValidationState::Unknown
        } else {
            RagValidationState::Failed
        },
        endpoint: None,
        tool_allowlist: Vec::new(),
        observed_at: Utc::now().to_rfc3339(),
        error: Some(error.code().to_string()),
        admitted: None,
        verified_physical_collections: Vec::new(),
        verified_logical_collections: Vec::new(),
    }
}

struct FixedMcpProbe(McpAttestation);

impl McpProbe for FixedMcpProbe {
    fn attest(
        &self,
        _config: &RagConfig,
        _bearer_token: &str,
        _attestation_secret: &str,
    ) -> Result<McpAttestation, RagError> {
        Ok(self.0.clone())
    }
}

fn protected_bytes(path: &Path, maximum: u64) -> Result<Vec<u8>, RagError> {
    ProtectedFile::open(path, maximum)
        .and_then(|file| file.read_all())
        .map_err(|_| RagError::InvalidConfig)
}

fn protected_canonical_json(path: &Path, maximum: u64) -> Result<(Value, Vec<u8>), RagError> {
    let bytes = protected_bytes(path, maximum)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| RagError::InvalidResponse)?;
    if canonical_json_bytes(&value).map_err(|_| RagError::InvalidResponse)? != bytes {
        return Err(RagError::InvalidResponse);
    }
    Ok((value, bytes))
}

fn verify_manifest_signature(
    snapshot_directory: &Path,
    manifest: &Value,
    manifest_bytes: &[u8],
) -> Result<(), RagError> {
    let public_key = protected_bytes(&snapshot_directory.join("manifest.pub"), 32)?;
    let signature = protected_bytes(&snapshot_directory.join("manifest.sig"), 64)?;
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| RagError::ValidationFailed)?;
    let signature = Signature::from_slice(&signature).map_err(|_| RagError::ValidationFailed)?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key).map_err(|_| RagError::ValidationFailed)?;
    verifying_key
        .verify(manifest_bytes, &signature)
        .map_err(|_| RagError::ValidationFailed)?;
    let manifest_object = manifest.as_object().ok_or(RagError::InvalidResponse)?;
    let signer = manifest_object
        .get("signer")
        .and_then(Value::as_object)
        .ok_or(RagError::InvalidResponse)?;
    let expected = signer
        .get("public_key_sha256")
        .and_then(Value::as_str)
        .ok_or(RagError::InvalidResponse)?;
    let observed = Sha256::digest(public_key);
    if hex::encode(observed) != expected {
        return Err(RagError::SignerMismatch);
    }
    Ok(())
}

fn read_config(path: &Path) -> Result<RagConfig, RagError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(RagError::NotConfigured)
        }
        Err(_) => return Err(RagError::InvalidConfig),
    }
    let bytes = protected_bytes(path, MAXIMUM_CONFIG_BYTES)?;
    let config =
        serde_json::from_slice::<RagConfig>(&bytes).map_err(|_| RagError::InvalidConfig)?;
    validate_config(&config)?;
    Ok(config)
}

fn query_rag_readiness(
    config_path: &Path,
    credentials: &SecretStore,
) -> Result<RagServiceReadiness, RagError> {
    let config = read_config(config_path)?;
    let bearer_token = credentials
        .load(&config.credential_key)
        .map_err(|_| RagError::AuthenticationFailed)?
        .ok_or(RagError::AuthenticationFailed)?;
    let attestation_secret = credentials
        .load(&config.attestation_credential_key)
        .map_err(|_| RagError::AuthenticationFailed)?
        .ok_or(RagError::AuthenticationFailed)?;
    if !admission_secrets_are_independent(&bearer_token, &attestation_secret) {
        return Err(RagError::AuthenticationFailed);
    }
    let attestation = HttpMcpProbe.attest(&config, &bearer_token, &attestation_secret)?;
    let readiness = exact_object(
        &attestation.snapshot_status,
        &[
            "format",
            "active_activation_id",
            "active_snapshot_id",
            "signature_fingerprint",
            "snapshot_time",
            "service",
            "retrieval_models",
            "collections",
            "golden_queries",
            "last_successful_activation",
        ],
    )?;
    let active_activation_id = field_text(readiness, "active_activation_id")?;
    if !valid_digest(active_activation_id) {
        return Err(RagError::InvalidResponse);
    }
    let snapshot_directory = config
        .state_root
        .join("snapshots")
        .join(&config.expected_active_snapshot_id);
    let (manifest, manifest_bytes) = protected_canonical_json(
        &snapshot_directory.join("manifest.json"),
        MAXIMUM_MANIFEST_BYTES,
    )?;
    verify_manifest_signature(&snapshot_directory, &manifest, &manifest_bytes)?;
    let catalogue_path = manifest
        .get("catalogue")
        .and_then(Value::as_object)
        .and_then(|catalogue| catalogue.get("path"))
        .and_then(Value::as_str)
        .ok_or(RagError::InvalidResponse)?;
    let catalogue_path = signed_snapshot_object_path(&snapshot_directory, catalogue_path)?;
    let (catalogue, _) = protected_canonical_json(&catalogue_path, MAXIMUM_CATALOGUE_BYTES)?;
    let (activation, _) = protected_canonical_json(
        &config
            .state_root
            .join("activations")
            .join(active_activation_id)
            .join("activation.json"),
        MAXIMUM_ACTIVATION_BYTES,
    )?;
    let observed_at = Utc::now().to_rfc3339();
    let result = verify_rag_service(
        &config,
        &bearer_token,
        &attestation_secret,
        SignedSnapshotDocuments {
            manifest: &manifest,
            catalogue: &catalogue,
        },
        &activation,
        &FixedMcpProbe(attestation),
        &observed_at,
    )?;
    if let Some(admitted) = result.admitted.clone() {
        cache_verified_service(admitted);
    }
    Ok(result)
}

fn signed_snapshot_object_path(root: &Path, relative: &str) -> Result<PathBuf, RagError> {
    let path = Path::new(relative);
    let components = path.components().collect::<Vec<_>>();
    if components.len() < 2
        || components.first() != Some(&Component::Normal("objects".as_ref()))
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(RagError::InvalidResponse);
    }
    Ok(root.join(path))
}

fn config_path<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, RagError> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(CONFIG_FILE_NAME))
        .map_err(|_| RagError::InvalidConfig)
}

#[tauri::command]
pub(crate) async fn get_rag_service_readiness(app: tauri::AppHandle) -> RagServiceReadiness {
    let task = tauri::async_runtime::spawn_blocking(move || {
        let path = config_path(&app)?;
        let store = SecretStore::shared(crate::app_state::keyring_service());
        query_rag_readiness(&path, store)
    });
    match task.await {
        Ok(Ok(readiness)) => readiness,
        Ok(Err(error)) => {
            clear_cached_service(KnowledgeServiceKind::Rag);
            fail_soft_readiness(error)
        }
        Err(_) => {
            clear_cached_service(KnowledgeServiceKind::Rag);
            fail_soft_readiness(RagError::ServiceUnavailable)
        }
    }
}

#[cfg(test)]
mod tests;
