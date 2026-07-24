use super::{
    catalog_tools, probe_authenticated_mcp, tool_set, valid_bearer_token,
    validate_service_endpoint, AdmissionError, KnowledgeServiceKind, ServiceAdmissionPolicy,
    VerifiedService, MEMORY_CATALOG_TOOLS, MEMORY_READ_ONLY_TOOLS, MEMORY_WORKFLOW_WRITE_TOOLS,
    RAG_CATALOG_TOOLS,
};
use crate::command_services::ssh::ProtectedFile;
use crate::secret_store::SecretStore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

const ADMISSION_CACHE_TTL: Duration = Duration::from_secs(30);
const MAXIMUM_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandKnowledgeWorkflow {
    Adviser,
    CommandMemory,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub(crate) struct NativeMcpIntegration {
    #[serde(rename = "type")]
    integration_type: &'static str,
    pub(crate) server_label: String,
    pub(crate) server_url: String,
    pub(crate) allowed_tools: Vec<String>,
    headers: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub(crate) struct CommandEvidencePolicy {
    version: u32,
    maximum_evidence_age_seconds: u64,
    services: Vec<CommandEvidenceService>,
    allowed_apple_ids: Vec<String>,
    allowed_file_paths: Vec<String>,
}

#[derive(Serialize)]
struct CommandEvidenceService {
    server_label: &'static str,
    kind: &'static str,
    active_identity: String,
}

pub(crate) fn build_command_evidence_policy(
    services: &[VerifiedService],
) -> Result<CommandEvidencePolicy, AdmissionError> {
    let integrations = build_catalog_integrations(services, CommandKnowledgeWorkflow::Adviser)?;
    let labels = integrations
        .iter()
        .map(|integration| integration.server_label.as_str())
        .collect::<BTreeSet<_>>();
    let mut ordered = services.to_vec();
    ordered.sort_by_key(|service| service.kind);
    let mut policy_services = Vec::with_capacity(ordered.len());
    for service in ordered {
        let (server_label, kind) = match service.kind {
            KnowledgeServiceKind::Memory => ("memory", "memory"),
            KnowledgeServiceKind::Rag => ("rag", "rag"),
        };
        if !labels.contains(server_label) || service.active_identity.is_empty() {
            return Err(AdmissionError::InvalidAttestation);
        }
        policy_services.push(CommandEvidenceService {
            server_label,
            kind,
            active_identity: service.active_identity,
        });
    }
    Ok(CommandEvidencePolicy {
        version: 1,
        maximum_evidence_age_seconds: 24 * 60 * 60,
        services: policy_services,
        allowed_apple_ids: Vec::new(),
        allowed_file_paths: Vec::new(),
    })
}

impl std::fmt::Debug for NativeMcpIntegration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeMcpIntegration")
            .field("integration_type", &self.integration_type)
            .field("server_label", &self.server_label)
            .field("server_url", &self.server_url)
            .field("allowed_tools", &self.allowed_tools)
            .field("headers", &"[REDACTED]")
            .finish()
    }
}

pub(crate) fn build_catalog_integrations(
    services: &[VerifiedService],
    workflow: CommandKnowledgeWorkflow,
) -> Result<Vec<NativeMcpIntegration>, AdmissionError> {
    if services.len() > 2 {
        return Err(AdmissionError::UnexpectedToolCatalog);
    }
    let mut ordered = services.to_vec();
    ordered.sort_by_key(|service| service.kind);
    let mut seen = BTreeSet::new();
    let mut result = Vec::with_capacity(ordered.len());
    for service in ordered {
        if !seen.insert(service.kind) {
            return Err(AdmissionError::UnexpectedToolCatalog);
        }
        validate_service_endpoint(service.kind, &service.endpoint)?;
        if !valid_bearer_token(&service.bearer_token) {
            return Err(AdmissionError::AuthenticationUnavailable);
        }
        let expected_identity = match service.kind {
            KnowledgeServiceKind::Memory => "memory",
            KnowledgeServiceKind::Rag => "rag",
        };
        if service.server_identity != expected_identity || service.active_identity.is_empty() {
            return Err(AdmissionError::ServerIdentityMismatch);
        }
        let observed = tool_set(&service.advertised_tools)?;
        if observed.iter().any(|tool| {
            !catalog_tools(service.kind)
                .iter()
                .any(|candidate| candidate == &tool.as_str())
        }) {
            return Err(AdmissionError::UnexpectedToolCatalog);
        }
        let allowed: Vec<String> = match service.kind {
            KnowledgeServiceKind::Rag => RAG_CATALOG_TOOLS
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            KnowledgeServiceKind::Memory => MEMORY_READ_ONLY_TOOLS
                .iter()
                .chain(
                    (workflow == CommandKnowledgeWorkflow::CommandMemory)
                        .then_some(MEMORY_WORKFLOW_WRITE_TOOLS)
                        .into_iter()
                        .flatten(),
                )
                .filter(|tool| observed.contains(**tool))
                .map(|tool| (*tool).to_string())
                .collect(),
        };
        if allowed.is_empty() || allowed.iter().any(|tool| !observed.contains(tool)) {
            return Err(AdmissionError::MissingRequiredTool);
        }
        result.push(NativeMcpIntegration {
            integration_type: "ephemeral_mcp",
            server_label: expected_identity.to_string(),
            server_url: service.endpoint,
            allowed_tools: allowed,
            headers: BTreeMap::from([(
                "Authorization".to_string(),
                format!("Bearer {}", service.bearer_token),
            )]),
        });
    }
    Ok(result)
}

struct CachedAdmissions {
    expires_at: Instant,
    services: Vec<VerifiedService>,
}

static ADMISSION_CACHE: LazyLock<Mutex<Option<CachedAdmissions>>> =
    LazyLock::new(|| Mutex::new(None));

pub(crate) fn cache_verified_service(service: VerifiedService) {
    let Ok(mut cache) = ADMISSION_CACHE.lock() else {
        return;
    };
    let now = Instant::now();
    let mut services = cache
        .take()
        .filter(|cached| cached.expires_at > now)
        .map(|cached| cached.services)
        .unwrap_or_default();
    services.retain(|candidate| candidate.kind != service.kind);
    services.push(service);
    *cache = Some(CachedAdmissions {
        expires_at: now + ADMISSION_CACHE_TTL,
        services,
    });
}

pub(crate) fn clear_cached_service(kind: KnowledgeServiceKind) {
    let Ok(mut cache) = ADMISSION_CACHE.lock() else {
        return;
    };
    if let Some(cached) = cache.as_mut() {
        cached.services.retain(|service| service.kind != kind);
        if cached.services.is_empty() {
            *cache = None;
        }
    }
}

fn cached_services() -> Vec<VerifiedService> {
    ADMISSION_CACHE
        .lock()
        .ok()
        .and_then(|mut cache| {
            let now = Instant::now();
            if cache
                .as_ref()
                .is_some_and(|cached| cached.expires_at <= now)
            {
                *cache = None;
            }
            cache.as_ref().map(|cached| cached.services.clone())
        })
        .unwrap_or_default()
}

pub(crate) fn catalog_integrations_json(workflow: CommandKnowledgeWorkflow) -> String {
    let services = cached_services();
    build_catalog_integrations(&services, workflow)
        .and_then(|integrations| {
            serde_json::to_string(&integrations).map_err(|_| AdmissionError::InvalidAttestation)
        })
        .unwrap_or_else(|_| "[]".to_string())
}

pub(crate) fn catalog_evidence_policy_json() -> Option<String> {
    let services = cached_services();
    if services.is_empty() {
        return None;
    }
    build_command_evidence_policy(&services)
        .and_then(|policy| {
            serde_json::to_string(&policy).map_err(|_| AdmissionError::InvalidAttestation)
        })
        .ok()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryCredentialKeys {
    local_read: String,
    local_attestation: String,
    local_replicate: String,
    remote_read: String,
    remote_replicate: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryAdmissionConfig {
    schema_version: u32,
    local_port: u16,
    home_host_alias: String,
    home_user: String,
    pinned_host_fingerprint: String,
    known_hosts_path: std::path::PathBuf,
    identity_file: std::path::PathBuf,
    remote_loopback_port: u16,
    local_node_id: String,
    home_node_id: String,
    sync_interval_minutes: u32,
    tool_allowlist: Vec<String>,
    credential_keys: MemoryCredentialKeys,
}

fn valid_memory_credential_key(value: &str) -> bool {
    value.starts_with("memory.")
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_memory_admission_config(config: &MemoryAdmissionConfig) -> Result<(), AdmissionError> {
    let tools = tool_set(&config.tool_allowlist)?;
    let credential_keys = [
        config.credential_keys.local_read.as_str(),
        config.credential_keys.local_attestation.as_str(),
        config.credential_keys.local_replicate.as_str(),
        config.credential_keys.remote_read.as_str(),
        config.credential_keys.remote_replicate.as_str(),
    ];
    if config.schema_version != 1
        || config.local_port == 0
        || config.remote_loopback_port == 0
        || config.local_node_id == config.home_node_id
        || config.local_node_id.len() > 128
        || !config.local_node_id.starts_with("node:")
        || config.home_node_id.len() > 128
        || !config.home_node_id.starts_with("node:")
        || !(5..=1440).contains(&config.sync_interval_minutes)
        || tools.iter().any(|tool| {
            !MEMORY_CATALOG_TOOLS
                .iter()
                .any(|candidate| candidate == &tool.as_str())
        })
        || !MEMORY_READ_ONLY_TOOLS
            .iter()
            .any(|required| tools.contains(*required))
        || credential_keys
            .iter()
            .any(|key| !valid_memory_credential_key(key))
        || credential_keys
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != credential_keys.len()
        || config.home_host_alias.is_empty()
        || config.home_user.is_empty()
        || config.pinned_host_fingerprint.is_empty()
        || !config.known_hosts_path.is_absolute()
        || !config.identity_file.is_absolute()
    {
        return Err(AdmissionError::InvalidAttestation);
    }
    Ok(())
}

pub(crate) fn admit_memory_for_catalog(
    app: &tauri::AppHandle,
    readiness_node_id: &str,
    observed_at: &str,
) -> Result<VerifiedService, AdmissionError> {
    let path = app
        .path()
        .app_config_dir()
        .map_err(|_| AdmissionError::InvalidAttestation)?
        .join("command-memory.json");
    let bytes = ProtectedFile::open(&path, MAXIMUM_CONFIG_BYTES)
        .and_then(|file| file.read_all())
        .map_err(|_| AdmissionError::InvalidAttestation)?;
    let config: MemoryAdmissionConfig =
        serde_json::from_slice(&bytes).map_err(|_| AdmissionError::InvalidAttestation)?;
    validate_memory_admission_config(&config)?;
    if config.local_node_id != readiness_node_id {
        return Err(AdmissionError::ActiveIdentityMismatch);
    }
    let store = SecretStore::shared(crate::app_state::keyring_service());
    let bearer_token = store
        .load(&config.credential_keys.local_read)
        .map_err(|_| AdmissionError::AuthenticationUnavailable)?
        .ok_or(AdmissionError::AuthenticationUnavailable)?;
    let attestation_secret = store
        .load(&config.credential_keys.local_attestation)
        .map_err(|_| AdmissionError::AuthenticationUnavailable)?
        .ok_or(AdmissionError::AuthenticationUnavailable)?;
    let endpoint = format!("http://127.0.0.1:{}/mcp", config.local_port);
    let attestation = probe_authenticated_mcp(
        &endpoint,
        &bearer_token,
        &attestation_secret,
        "memory",
        &config.local_node_id,
        None,
    )?;
    let advertised = tool_set(&attestation.tools)?;
    if config
        .tool_allowlist
        .iter()
        .any(|tool| !advertised.contains(tool))
    {
        return Err(AdmissionError::UnexpectedToolCatalog);
    }
    let service = VerifiedService {
        kind: KnowledgeServiceKind::Memory,
        server_identity: attestation.server_identity,
        endpoint,
        bearer_token,
        active_identity: config.local_node_id.clone(),
        advertised_tools: config.tool_allowlist.clone(),
        verified_at: observed_at.to_string(),
    };
    let expected: Vec<&str> = config.tool_allowlist.iter().map(String::as_str).collect();
    ServiceAdmissionPolicy::for_service(
        KnowledgeServiceKind::Memory,
        "memory",
        &config.local_node_id,
        &expected,
    )
    .verify(&service)?;
    Ok(service)
}
