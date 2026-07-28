#![forbid(unsafe_code)]

//! A small, protocol-faithful bridge from an AGNTCY/OASF Agent Record to ACP.
//!
//! The bridge is intentionally a subprocess. Buzz owns the ACP session and UI;
//! the source runtime owns its agent identity, context, execution, and keys.

use base64::Engine;
use clap::Parser;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use url::Url;
use uuid::Uuid;

const MAX_RECORD_BYTES: usize = 2 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: usize = 2 * 1024 * 1024;
const MAX_ACP_LINE_BYTES: usize = 1024 * 1024;
const DEFAULT_TASK_POLL_SECS: u64 = 7_200;
const TASK_POLL_BACKOFF_SECS: [u64; 4] = [1, 5, 15, 30];

/// Configuration supplied by Buzz's BYOH subprocess definition.
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    /// Local path or HTTP(S) URL for an OASF Agent Record.
    pub record: String,
    /// Optional operator-supplied token for the A2A endpoint.
    pub bearer_token: Option<String>,
    /// Exact endpoint where the operator permits the bearer token to be sent.
    pub bearer_token_endpoint: Option<String>,
    /// Optional caller-supplied A2A conversation context identifier.
    pub context_id: Option<String>,
    /// Maximum time to wait for an asynchronous A2A task.
    pub task_poll_secs: u64,
}

#[derive(Debug, Parser)]
#[command(
    name = "buzz-a2a-acp",
    about = "Expose an OASF Agent Record as an ACP subprocess"
)]
struct Cli {
    /// Local path or HTTP(S) URL for an OASF 1.0 Agent Record.
    #[arg(long, env = "BUZZ_A2A_AGENT_RECORD")]
    record: String,

    /// Exact A2A endpoint where the operator permits the bearer token to be sent.
    #[arg(long, env = "BUZZ_A2A_BEARER_ENDPOINT")]
    bearer_token_endpoint: Option<String>,

    /// Optional stable A2A conversation context identifier.
    #[arg(long, env = "BUZZ_A2A_CONTEXT_ID")]
    context_id: Option<String>,

    /// Maximum time to wait for an asynchronous A2A task.
    #[arg(
        long,
        env = "BUZZ_A2A_TASK_POLL_SECS",
        default_value_t = DEFAULT_TASK_POLL_SECS
    )]
    task_poll_secs: u64,
}

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("record source is empty")]
    EmptyRecord,
    #[error("record source is not a local path or HTTP(S) URL: {0}")]
    InvalidSource(String),
    #[error("fetch {what} failed with HTTP {status}")]
    HttpStatus {
        what: &'static str,
        status: StatusCode,
    },
    #[error("{what} exceeds the {limit} byte limit")]
    TooLarge { what: &'static str, limit: usize },
    #[error("read {what}: {source}")]
    Read {
        what: &'static str,
        source: std::io::Error,
    },
    #[error("decode {what}: {source}")]
    Decode {
        what: &'static str,
        source: serde_json::Error,
    },
    #[error("invalid OASF Agent Record: {0}")]
    InvalidRecord(String),
    #[error("invalid OASF A2A artifact: {0}")]
    InvalidArtifact(String),
    #[error("A2A endpoint is not advertised by the Agent Card")]
    MissingEndpoint,
    #[error("unsafe endpoint URL: {0}")]
    UnsafeEndpoint(String),
    #[error("bearer token is not authorized for A2A endpoint {0}")]
    UnauthorizedTokenEndpoint(String),
    #[error("remote A2A task did not complete before the {0} second timeout")]
    TaskTimeout(u64),
    #[error("A2A request failed: {0}")]
    Request(String),
    #[error("A2A response was invalid: {0}")]
    InvalidResponse(String),
    #[error("ACP protocol error: {0}")]
    Acp(String),
}

#[derive(Debug, Deserialize)]
struct AgentRecord {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    schema_version: Option<String>,
    #[serde(default)]
    modules: Vec<OasfModule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordSource {
    LocalPath(PathBuf),
    HttpUrl(Url),
}

impl RecordSource {
    fn parse(source: &str) -> Result<Self, AdapterError> {
        if source.trim().is_empty() {
            return Err(AdapterError::EmptyRecord);
        }
        if let Ok(url) = Url::parse(source) {
            if matches!(url.scheme(), "http" | "https") {
                validate_http_url(source)
                    .map_err(|_| AdapterError::InvalidSource(source.to_owned()))?;
                return Ok(Self::HttpUrl(url));
            }
            if source.contains("://") {
                return Err(AdapterError::InvalidSource(source.to_owned()));
            }
        }
        Ok(Self::LocalPath(PathBuf::from(source)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordVerification {
    OperatorReviewedLocal,
    TlsOnly,
}

impl RecordVerification {
    fn label(self) -> &'static str {
        match self {
            Self::OperatorReviewedLocal => "operator-reviewed-local",
            Self::TlsOnly => "tls-only",
        }
    }
}

struct ResolvedRecord {
    record: AgentRecord,
    base: Option<Url>,
    content_digest: String,
    verification: RecordVerification,
}

#[derive(Debug, Deserialize)]
struct OasfModule {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    artifact: Option<Box<RawValue>>,
    #[serde(default)]
    data: Option<A2aData>,
}

#[derive(Debug, Deserialize)]
struct A2aData {
    #[serde(default)]
    card_data: Option<Value>,
    #[serde(default, rename = "card_schema_version")]
    _card_schema_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Descriptor {
    #[serde(default)]
    digest: Option<String>,
    #[serde(default, rename = "media_type")]
    media_type: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    json: Option<Box<RawValue>>,
    #[serde(default)]
    urls: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
/// Public A2A Agent Card fields used to select an invocation interface.
pub struct AgentCard {
    /// Non-standard identifier used only by the vendor compatibility path.
    #[serde(default, rename = "id")]
    pub vendor_id: Option<String>,
    /// Human-readable name, when advertised.
    #[serde(default)]
    pub name: Option<String>,
    /// Human-readable description, when advertised.
    #[serde(default)]
    pub description: Option<String>,
    /// A2A 0.3 card endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Non-standard endpoint field used by the vendor compatibility path.
    #[serde(default, rename = "serviceEndpoint")]
    pub service_endpoint: Option<String>,
    /// Current A2A interface declarations.
    #[serde(default, rename = "supportedInterfaces")]
    pub supported_interfaces: Vec<SupportedInterface>,
}

#[derive(Debug, Clone, Deserialize)]
/// A protocol endpoint declared by an A2A Agent Card.
pub struct SupportedInterface {
    /// URL to the protocol endpoint.
    #[serde(default)]
    pub url: Option<String>,
    /// Protocol binding name, for example `JSONRPC`.
    #[serde(default, rename = "protocolBinding")]
    pub protocol_binding: Option<String>,
    /// Protocol version declared by the remote agent.
    #[serde(default, rename = "protocolVersion")]
    pub protocol_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolMode {
    /// A2A JSON-RPC interface declared by a current Agent Card.
    JsonRpc {
        endpoint: String,
        protocol_version: Option<String>,
    },
    /// Compatibility with a deployed vendor card and method shape.
    VendorServiceEndpoint { endpoint: String },
}

impl ProtocolMode {
    fn endpoint(&self) -> &str {
        match self {
            Self::JsonRpc { endpoint, .. } | Self::VendorServiceEndpoint { endpoint } => endpoint,
        }
    }

    fn a2a_version(&self) -> Option<&'static str> {
        match self {
            Self::JsonRpc {
                protocol_version, ..
            } if protocol_version
                .as_deref()
                .is_some_and(|version| version.starts_with("1.")) =>
            {
                Some("1.0")
            }
            Self::JsonRpc { .. } => Some("0.3"),
            Self::VendorServiceEndpoint { .. } => None,
        }
    }

    fn method(&self, task: bool) -> &str {
        match self {
            Self::JsonRpc {
                protocol_version, ..
            } => {
                if protocol_version
                    .as_deref()
                    .is_some_and(|version| version.starts_with("1."))
                {
                    if task {
                        "GetTask"
                    } else {
                        "SendMessage"
                    }
                } else if task {
                    "tasks/get"
                } else {
                    "message/send"
                }
            }
            Self::VendorServiceEndpoint { .. } => {
                if task {
                    "agent/getTask"
                } else {
                    "agent/sendMessage"
                }
            }
        }
    }
}

/// A resolved public record and its invocation mode.
#[derive(Debug, Clone)]
/// Resolved public metadata and invocation mode for one remote agent.
pub struct ResolvedAgent {
    /// Name from the OASF record.
    pub record_name: Option<String>,
    /// OASF schema version from the record.
    pub record_schema_version: Option<String>,
    /// Public A2A card resolved from the OASF module.
    pub card: AgentCard,
    /// Selected current or compatibility invocation mode.
    pub mode: ProtocolMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardSource {
    Artifact,
    DeprecatedCardData,
}

static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

fn pinned_http_client(url: &Url, addresses: &[SocketAddr]) -> Result<Client, AdapterError> {
    let raw_host = url
        .host_str()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(url.to_string()))?;
    let host = normalized_host(raw_host);
    let mut builder = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30));
    // Pin every address that passed our policy check. This prevents reqwest
    // from performing a second DNS lookup while preserving IPv4/IPv6 fallback.
    if host.parse::<IpAddr>().is_err() {
        if addresses.is_empty() {
            return Err(AdapterError::UnsafeEndpoint(url.to_string()));
        }
        builder = builder.resolve_to_addrs(&host, addresses);
    }
    builder
        .build()
        .map_err(|e| AdapterError::Request(format!("build HTTP client: {e}")))
}

fn validate_http_url(raw: &str) -> Result<Url, AdapterError> {
    let url = Url::parse(raw).map_err(|_| AdapterError::UnsafeEndpoint(raw.to_owned()))?;
    let raw_host = url
        .host_str()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(raw.to_owned()))?;
    let host = normalized_host(raw_host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        // Local A2A runtimes are allowed over loopback HTTP. Private and
        // link-local addresses remain rejected for every other scheme.
        if url.scheme() == "http" && ip.is_loopback() {
            return Ok(url);
        }
        if is_private_ip(ip) {
            return Err(AdapterError::UnsafeEndpoint(raw.to_owned()));
        }
    }
    if url.scheme() == "https" && host.eq_ignore_ascii_case("localhost") {
        return Err(AdapterError::UnsafeEndpoint(raw.to_owned()));
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback_host(&host) => Ok(url),
        _ => Err(AdapterError::UnsafeEndpoint(raw.to_owned())),
    }
}

fn normalized_host(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn is_private_ip(ip: IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V6(address) => address
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(address)),
        address => address,
    };
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                // IPv4-transitional address ranges can encode private IPv4
                // targets while still presenting as IPv6 DNS answers.
                || (segments[0] == 0x0064
                    && segments[1] == 0xff9b
                    && segments[2..6] == [0, 0, 0, 0])
                || segments[0] == 0x2002
                || (segments[0] == 0x2001 && segments[1] == 0)
                || segments[..6] == [0, 0, 0, 0, 0, 0]
        }
    }
}

async fn resolve_network_url(url: &Url) -> Result<Vec<SocketAddr>, AdapterError> {
    let raw_host = url
        .host_str()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(url.to_string()))?;
    let host = normalized_host(raw_host);
    if let Ok(ip) = host.parse::<IpAddr>() {
        if url.scheme() == "http" && ip.is_loopback() {
            return Ok(vec![SocketAddr::new(
                ip,
                url.port_or_known_default().unwrap_or(80),
            )]);
        }
        if !is_private_ip(ip) {
            return Ok(vec![SocketAddr::new(
                ip,
                url.port_or_known_default().unwrap_or(443),
            )]);
        }
        return Err(AdapterError::UnsafeEndpoint(url.to_string()));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(url.to_string()))?;
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| AdapterError::UnsafeEndpoint(url.to_string()))?
        .collect();
    validate_resolved_addresses(url, &addresses)?;
    Ok(addresses)
}

fn validate_resolved_addresses(url: &Url, addresses: &[SocketAddr]) -> Result<(), AdapterError> {
    let raw_host = url
        .host_str()
        .ok_or_else(|| AdapterError::UnsafeEndpoint(url.to_string()))?;
    let host = normalized_host(raw_host);
    if addresses.is_empty() {
        return Err(AdapterError::UnsafeEndpoint(url.to_string()));
    }
    let is_local_http = url.scheme() == "http" && is_loopback_host(&host);
    if url.scheme() == "http" && !is_local_http {
        return Err(AdapterError::UnsafeEndpoint(url.to_string()));
    }
    if is_local_http {
        if addresses.iter().any(|address| !address.ip().is_loopback()) {
            return Err(AdapterError::UnsafeEndpoint(url.to_string()));
        }
    } else if addresses.iter().any(|address| is_private_ip(address.ip())) {
        return Err(AdapterError::UnsafeEndpoint(url.to_string()));
    }
    Ok(())
}

async fn response_bytes(
    mut response: reqwest::Response,
    what: &'static str,
    limit: usize,
) -> Result<Vec<u8>, AdapterError> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(AdapterError::TooLarge { what, limit });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| AdapterError::Request(format!("read {what}: {e}")))?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(AdapterError::TooLarge { what, limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_source(
    source: &str,
    what: &'static str,
    limit: usize,
) -> Result<Vec<u8>, AdapterError> {
    if source.trim().is_empty() {
        return Err(AdapterError::EmptyRecord);
    }
    if let Ok(url) = Url::parse(source) {
        if matches!(url.scheme(), "http" | "https") {
            validate_http_url(source)
                .map_err(|_| AdapterError::InvalidSource(source.to_owned()))?;
            let addresses = resolve_network_url(&url).await?;
            let response = pinned_http_client(&url, &addresses)?
                .get(url)
                .send()
                .await
                .map_err(|e| AdapterError::Request(format!("fetch {what}: {e}")))?;
            let status = response.status();
            if !status.is_success() {
                return Err(AdapterError::HttpStatus { what, status });
            }
            return response_bytes(response, what, limit).await;
        }
        if source.contains("://") {
            return Err(AdapterError::InvalidSource(source.to_owned()));
        }
    }
    let body = tokio::fs::read(Path::new(source))
        .await
        .map_err(|source| AdapterError::Read { what, source })?;
    if body.len() > limit {
        return Err(AdapterError::TooLarge { what, limit });
    }
    Ok(body)
}

async fn load_record(source: &str) -> Result<ResolvedRecord, AdapterError> {
    let source = RecordSource::parse(source)?;
    let source_text = match &source {
        RecordSource::LocalPath(path) => path.to_string_lossy().into_owned(),
        RecordSource::HttpUrl(url) => url.to_string(),
    };
    let bytes = read_source(&source_text, "Agent Record", MAX_RECORD_BYTES).await?;
    let record = if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'[') {
        let mut records: Vec<AgentRecord> =
            serde_json::from_slice(&bytes).map_err(|source| AdapterError::Decode {
                what: "Agent Record",
                source,
            })?;
        if records.len() != 1 {
            return Err(AdapterError::InvalidRecord(format!(
                "expected exactly one Agent Record, got {}",
                records.len()
            )));
        }
        records
            .pop()
            .ok_or_else(|| AdapterError::InvalidRecord("Agent Record collection is empty".into()))?
    } else {
        serde_json::from_slice(&bytes).map_err(|source| AdapterError::Decode {
            what: "Agent Record",
            source,
        })?
    };
    let (base, verification) = match source {
        RecordSource::LocalPath(_) => (None, RecordVerification::OperatorReviewedLocal),
        RecordSource::HttpUrl(url) if url.scheme() == "https" => {
            (Some(url), RecordVerification::TlsOnly)
        }
        RecordSource::HttpUrl(url) => (Some(url), RecordVerification::OperatorReviewedLocal),
    };
    Ok(ResolvedRecord {
        record,
        base,
        content_digest: format!("sha256:{}", hex::encode(Sha256::digest(&bytes))),
        verification,
    })
}

fn descriptor_from_raw(value: &RawValue) -> Result<Descriptor, AdapterError> {
    let raw = value.get();
    if raw.trim_start().starts_with('[') {
        let mut descriptors: Vec<Descriptor> = serde_json::from_str(raw)
            .map_err(|e| AdapterError::InvalidArtifact(format!("descriptor: {e}")))?;
        if descriptors.len() != 1 {
            return Err(AdapterError::InvalidArtifact(format!(
                "expected exactly one artifact descriptor, got {}",
                descriptors.len()
            )));
        }
        descriptors
            .pop()
            .ok_or_else(|| AdapterError::InvalidArtifact("artifact descriptor is absent".into()))
    } else {
        serde_json::from_str(raw)
            .map_err(|e| AdapterError::InvalidArtifact(format!("descriptor: {e}")))
    }
}

fn verify_descriptor(descriptor: &Descriptor, bytes: &[u8]) -> Result<(), AdapterError> {
    let size = descriptor.size.ok_or_else(|| {
        AdapterError::InvalidArtifact("OASF artifact descriptor requires size".into())
    })?;
    if size != bytes.len() as u64 {
        return Err(AdapterError::InvalidArtifact(format!(
            "descriptor size {size} does not match {}",
            bytes.len()
        )));
    }
    let digest = descriptor.digest.as_deref().ok_or_else(|| {
        AdapterError::InvalidArtifact("OASF artifact descriptor requires digest".into())
    })?;
    let Some(expected) = digest
        .strip_prefix("sha256:")
        .or_else(|| digest.strip_prefix("sha256-"))
    else {
        return Err(AdapterError::InvalidArtifact(format!(
            "unsupported digest {digest:?}; expected sha256:<hex>"
        )));
    };
    let actual = hex::encode(Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(AdapterError::InvalidArtifact(format!(
            "sha256 digest mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

async fn descriptor_bytes(
    descriptor: &Descriptor,
    record_url: Option<&Url>,
) -> Result<Vec<u8>, AdapterError> {
    let media_type = descriptor.media_type.as_deref().ok_or_else(|| {
        AdapterError::InvalidArtifact("OASF artifact descriptor requires media_type".into())
    })?;
    if !media_type.to_ascii_lowercase().contains("json") {
        return Err(AdapterError::InvalidArtifact(format!(
            "A2A artifact media type must be JSON, got {media_type:?}"
        )));
    }
    if let Some(value) = descriptor.json.as_ref() {
        let bytes = value.get().as_bytes().to_vec();
        verify_descriptor(descriptor, &bytes)?;
        return Ok(bytes);
    }
    if let Some(data) = descriptor.data.as_deref() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| {
                AdapterError::InvalidArtifact(format!("descriptor data is not base64: {e}"))
            })?;
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(AdapterError::TooLarge {
                what: "A2A artifact",
                limit: MAX_ARTIFACT_BYTES,
            });
        }
        verify_descriptor(descriptor, &bytes)?;
        return Ok(bytes);
    }
    if let Some(raw_url) = descriptor.urls.first() {
        if descriptor.digest.is_none() {
            return Err(AdapterError::InvalidArtifact(
                "remote artifact descriptors require a sha256 digest".into(),
            ));
        }
        let url = if let Ok(url) = Url::parse(raw_url) {
            url
        } else if let Some(base) = record_url {
            base.join(raw_url)
                .map_err(|_| AdapterError::UnsafeEndpoint(raw_url.clone()))?
        } else {
            return Err(AdapterError::InvalidArtifact(format!(
                "relative artifact URL {raw_url:?} requires an HTTP(S) record source"
            )));
        };
        validate_http_url(url.as_str())?;
        let bytes = read_source(url.as_str(), "A2A artifact", MAX_ARTIFACT_BYTES).await?;
        verify_descriptor(descriptor, &bytes)?;
        return Ok(bytes);
    }
    Err(AdapterError::InvalidArtifact(
        "descriptor has no json, data, or urls".into(),
    ))
}

fn is_a2a_module(module: &OasfModule) -> bool {
    module.name.as_deref() == Some("integration/a2a")
        || module.id.as_ref().and_then(Value::as_u64) == Some(203)
}

async fn resolve_card(
    record: AgentRecord,
    record_url: Option<&Url>,
) -> Result<(ResolvedAgent, CardSource), AdapterError> {
    let module = record
        .modules
        .iter()
        .find(|m| is_a2a_module(m))
        .ok_or_else(|| {
            AdapterError::InvalidRecord("missing integration/a2a module (id 203)".into())
        })?;
    let (card_value, source) = if let Some(artifact) = module.artifact.as_ref() {
        let descriptor = descriptor_from_raw(artifact)?;
        let bytes = descriptor_bytes(&descriptor, record_url).await?;
        (
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|e| AdapterError::InvalidArtifact(format!("Agent Card JSON: {e}")))?,
            CardSource::Artifact,
        )
    } else if let Some(data) = module.data.as_ref().and_then(|data| data.card_data.clone()) {
        (data, CardSource::DeprecatedCardData)
    } else {
        return Err(AdapterError::InvalidRecord(
            "integration/a2a module has no artifact; deprecated data.card_data is also absent"
                .into(),
        ));
    };
    let card: AgentCard = serde_json::from_value(card_value)
        .map_err(|e| AdapterError::InvalidArtifact(format!("Agent Card shape: {e}")))?;
    let mode = select_protocol_mode(&card)?;
    Ok((
        ResolvedAgent {
            record_name: record.name,
            record_schema_version: record.schema_version,
            card,
            mode,
        },
        source,
    ))
}

/// Select the declared JSON-RPC interface, with a named pre-1.0 compatibility path.
pub fn select_protocol_mode(card: &AgentCard) -> Result<ProtocolMode, AdapterError> {
    if let Some(interface) = card.supported_interfaces.iter().find(|i| {
        i.protocol_binding
            .as_deref()
            .is_some_and(|binding| binding.to_ascii_lowercase().contains("jsonrpc"))
    }) {
        if let Some(endpoint) = interface.url.clone() {
            validate_http_url(&endpoint)?;
            return Ok(ProtocolMode::JsonRpc {
                endpoint,
                protocol_version: interface.protocol_version.clone(),
            });
        }
    }
    if let Some(endpoint) = card.service_endpoint.clone() {
        validate_http_url(&endpoint)?;
        return Ok(ProtocolMode::VendorServiceEndpoint { endpoint });
    }
    if let Some(endpoint) = card.url.clone() {
        validate_http_url(&endpoint)?;
        return Ok(ProtocolMode::JsonRpc {
            endpoint,
            protocol_version: Some("0.3".into()),
        });
    }
    Err(AdapterError::MissingEndpoint)
}

fn protocol_request(
    client: &Client,
    mode: &ProtocolMode,
    endpoint: &str,
) -> reqwest::RequestBuilder {
    let request = client.post(endpoint);
    match mode.a2a_version() {
        Some(version) => request.header("A2A-Version", version),
        None => request,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PromptBlock {
    Text {
        text: String,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Deserialize)]
struct PromptParams {
    #[serde(rename = "sessionId")]
    session_id: String,
    prompt: Vec<PromptBlock>,
}

#[derive(Debug, Deserialize)]
struct CancelParams {
    #[serde(rename = "sessionId")]
    session_id: String,
}

fn prompt_text(blocks: &[PromptBlock]) -> Result<String, AdapterError> {
    let text = blocks
        .iter()
        .filter_map(|block| match block {
            PromptBlock::Text { text } => Some(text.as_str()),
            PromptBlock::Unsupported => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return Err(AdapterError::Acp("prompt contains no text content".into()));
    }
    Ok(text)
}

fn extract_text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(parts) = value.get("parts").and_then(Value::as_array) {
        let joined = parts
            .iter()
            .filter_map(extract_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    if let Some(artifacts) = value.get("artifacts").and_then(Value::as_array) {
        let joined = artifacts
            .iter()
            .rev()
            .filter_map(extract_text)
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    if let Some(history) = value.get("history").and_then(Value::as_array) {
        for item in history.iter().rev() {
            if item.get("role").and_then(Value::as_str) != Some("user") {
                if let Some(text) = extract_text(item) {
                    return Some(text);
                }
            }
        }
    }
    value.get("message").and_then(extract_text)
}

async fn invoke(
    resolved: &ResolvedAgent,
    token: Option<&str>,
    token_endpoint: Option<&str>,
    task_poll_secs: u64,
    session_id: &str,
    text: &str,
) -> Result<String, AdapterError> {
    validate_endpoint_binding(token, token_endpoint, resolved.mode.endpoint())?;
    let endpoint = Url::parse(resolved.mode.endpoint())
        .map_err(|_| AdapterError::UnsafeEndpoint(resolved.mode.endpoint().to_owned()))?;
    let addresses = resolve_network_url(&endpoint).await?;
    let client = pinned_http_client(&endpoint, &addresses)?;
    let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let payload = request_payload(
        &resolved.mode,
        resolved.card.vendor_id.as_deref(),
        id,
        session_id,
        text,
    );
    let mut request =
        protocol_request(&client, &resolved.mode, resolved.mode.endpoint()).json(&payload);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|e| AdapterError::Request(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(AdapterError::HttpStatus {
            what: "A2A request",
            status,
        });
    }
    let bytes = response_bytes(response, "A2A response", MAX_ARTIFACT_BYTES).await?;
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|e| AdapterError::Request(format!("decode A2A response: {e}")))?;
    if let Some(error) = body.get("error") {
        return Err(AdapterError::InvalidResponse(error.to_string()));
    }
    let result = body.get("result").unwrap_or(&body);
    let result = result
        .get("task")
        .or_else(|| result.get("message"))
        .unwrap_or(result);
    if result.pointer("/status/state").is_some() {
        let task_id = result
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if let Some(text) = task_outcome(result, task_id)? {
            return Ok(text);
        }
        if task_id != "unknown" {
            return poll_task(
                resolved,
                token,
                token_endpoint,
                task_id,
                task_poll_secs,
                &client,
            )
            .await;
        }
        return Err(AdapterError::InvalidResponse(
            "A2A task response has no task id".into(),
        ));
    }
    extract_text(result).ok_or_else(|| {
        AdapterError::InvalidResponse("A2A response contains no message or task state".into())
    })
}

async fn poll_task(
    resolved: &ResolvedAgent,
    token: Option<&str>,
    token_endpoint: Option<&str>,
    task_id: &str,
    task_poll_secs: u64,
    client: &Client,
) -> Result<String, AdapterError> {
    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(task_poll_secs);
    let mut poll_attempt = 0usize;
    while started.elapsed() < timeout {
        let remaining = timeout.saturating_sub(started.elapsed());
        let delay = std::time::Duration::from_secs(
            TASK_POLL_BACKOFF_SECS[poll_attempt.min(TASK_POLL_BACKOFF_SECS.len() - 1)],
        )
        .min(remaining);
        tokio::time::sleep(delay).await;
        poll_attempt = poll_attempt.saturating_add(1);
        if started.elapsed() >= timeout {
            break;
        }
        let id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let params = match resolved.mode {
            ProtocolMode::JsonRpc { .. } => json!({ "id": task_id }),
            ProtocolMode::VendorServiceEndpoint { .. } => json!({ "taskId": task_id }),
        };
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": resolved.mode.method(true),
            "params": params,
        });
        validate_endpoint_binding(token, token_endpoint, resolved.mode.endpoint())?;
        let mut request =
            protocol_request(client, &resolved.mode, resolved.mode.endpoint()).json(&payload);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|e| AdapterError::Request(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(AdapterError::HttpStatus {
                what: "A2A task poll",
                status,
            });
        }
        let bytes = response_bytes(response, "A2A task response", MAX_ARTIFACT_BYTES).await?;
        let body: Value = serde_json::from_slice(&bytes)
            .map_err(|e| AdapterError::Request(format!("decode A2A task response: {e}")))?;
        if let Some(error) = body.get("error") {
            return Err(AdapterError::InvalidResponse(error.to_string()));
        }
        let result = body.get("result").unwrap_or(&body);
        let result = result
            .get("task")
            .or_else(|| result.get("message"))
            .unwrap_or(result);
        if let Some(text) = task_outcome(result, task_id)? {
            return Ok(text);
        }
    }
    Err(AdapterError::TaskTimeout(task_poll_secs))
}

fn validate_endpoint_binding(
    token: Option<&str>,
    expected_endpoint: Option<&str>,
    actual_endpoint: &str,
) -> Result<(), AdapterError> {
    let endpoints_match = match expected_endpoint {
        Some(expected) => {
            let expected = validate_http_url(expected)?;
            let actual = validate_http_url(actual_endpoint)?;
            expected == actual
        }
        None => token.is_none(),
    };
    if !endpoints_match {
        return Err(AdapterError::UnauthorizedTokenEndpoint(
            actual_endpoint.to_owned(),
        ));
    }
    Ok(())
}

fn task_outcome(result: &Value, task_id: &str) -> Result<Option<String>, AdapterError> {
    let wire_state = result
        .get("status")
        .and_then(|status| status.get("state"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AdapterError::InvalidResponse(format!("A2A task {task_id} has no status state"))
        })?;
    let normalized_state = wire_state.trim().to_ascii_lowercase();
    let state = normalized_state
        .strip_prefix("task_state_")
        .unwrap_or(&normalized_state);
    match state {
        "completed" => {
            Ok(Some(extract_text(result).unwrap_or_else(|| {
                format!("A2A task {task_id} completed")
            })))
        }
        "accepted" | "submitted" | "working" | "pending" => Ok(None),
        "failed" | "canceled" | "cancelled" | "rejected" | "input-required" | "input_required" => {
            let detail = extract_text(result)
                .map(|text| format!(": {text}"))
                .unwrap_or_default();
            Err(AdapterError::InvalidResponse(format!(
                "A2A task {task_id} ended in {state}{detail}"
            )))
        }
        other => Err(AdapterError::InvalidResponse(format!(
            "A2A task {task_id} has unknown state {wire_state} (normalized as {other})"
        ))),
    }
}

fn request_payload(
    mode: &ProtocolMode,
    agent_id: Option<&str>,
    id: u64,
    session_id: &str,
    text: &str,
) -> Value {
    let params = match mode {
        ProtocolMode::JsonRpc {
            protocol_version, ..
        } if protocol_version
            .as_deref()
            .is_some_and(|version| version.starts_with("1.")) =>
        {
            json!({
                "message": { "messageId": format!("buzz-{id}"), "role": "ROLE_USER", "contextId": session_id, "parts": [{ "text": text }] },
            })
        }
        ProtocolMode::JsonRpc { .. } => json!({
            "message": { "messageId": format!("buzz-{id}"), "role": "user", "contextId": session_id, "parts": [{ "kind": "text", "text": text }] },
        }),
        ProtocolMode::VendorServiceEndpoint { .. } => json!({
            "agentId": agent_id,
            "message": { "role": "user", "parts": [{ "type": "text", "text": text }] },
            "contextId": session_id,
        }),
    };
    json!({ "jsonrpc": "2.0", "id": id, "method": mode.method(false), "params": params })
}

async fn send_json<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: Value,
) -> Result<(), AdapterError> {
    let mut line = serde_json::to_vec(&value)
        .map_err(|e| AdapterError::Acp(format!("encode response: {e}")))?;
    line.push(b'\n');
    writer
        .write_all(&line)
        .await
        .map_err(|e| AdapterError::Acp(format!("write response: {e}")))?;
    writer
        .flush()
        .await
        .map_err(|e| AdapterError::Acp(format!("flush response: {e}")))?;
    Ok(())
}

enum AcpAction {
    Response(Value),
    Prompt {
        id: Value,
        session_id: String,
        text: String,
    },
    Cancel {
        id: Option<Value>,
        session_id: String,
    },
}

fn handle_acp_message(
    message: &Value,
    sessions: &mut HashSet<String>,
    agent_name: &str,
    configured_context_id: Option<&str>,
) -> Result<Option<AcpAction>, AdapterError> {
    let method = message.get("method").and_then(Value::as_str);
    if method == Some("session/cancel") {
        let params: CancelParams =
            serde_json::from_value(message.get("params").cloned().unwrap_or(Value::Null))
                .map_err(|e| AdapterError::Acp(format!("session/cancel params: {e}")))?;
        return Ok(Some(AcpAction::Cancel {
            id: message.get("id").cloned(),
            session_id: params.session_id,
        }));
    }
    let Some(id) = message.get("id").cloned() else {
        return Ok(None);
    };
    match method {
        Some("initialize") => Ok(Some(AcpAction::Response(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": message.pointer("/params/protocolVersion").and_then(Value::as_u64).unwrap_or(1).min(1),
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": false },
                    "mcpCapabilities": { "http": false, "sse": false },
                },
                "agentInfo": { "name": agent_name, "version": "oasf-a2a" },
            }
        })))),
        Some("session/new") => {
            let session_id = configured_context_id
                .map(str::to_owned)
                .unwrap_or_else(|| format!("a2a-{}", Uuid::new_v4()));
            sessions.insert(session_id.clone());
            Ok(Some(AcpAction::Response(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "sessionId": session_id },
            }))))
        }
        Some("session/prompt") => {
            let params: PromptParams =
                serde_json::from_value(message.get("params").cloned().unwrap_or(Value::Null))
                    .map_err(|e| AdapterError::Acp(format!("session/prompt params: {e}")))?;
            if !sessions.contains(&params.session_id) {
                return Ok(Some(AcpAction::Response(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32602, "message": "unknown session" },
                }))));
            }
            let text = match prompt_text(&params.prompt) {
                Ok(text) => text,
                Err(error) => {
                    return Ok(Some(AcpAction::Response(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32602, "message": error.to_string() },
                    }))));
                }
            };
            Ok(Some(AcpAction::Prompt {
                id,
                session_id: params.session_id,
                text,
            }))
        }
        Some(method) => Ok(Some(AcpAction::Response(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {method}") },
        })))),
        None => Ok(None),
    }
}

fn prompt_success(id: Value, session_id: &str, text: &str) -> [Value; 2] {
    [
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text },
                }
            }
        }),
        json!({ "jsonrpc": "2.0", "id": id, "result": { "stopReason": "end_turn" } }),
    ]
}

struct ActivePrompt {
    id: Value,
    session_id: String,
    task: tokio::task::JoinHandle<Result<String, AdapterError>>,
}

enum LoopEvent {
    Input(Option<Result<Option<String>, AdapterError>>),
    PromptFinished(Result<Result<String, AdapterError>, tokio::task::JoinError>),
}

/// Run the adapter over ACP JSON-RPC lines on stdin/stdout.
pub async fn run(config: AdapterConfig) -> Result<(), AdapterError> {
    let record = load_record(&config.record).await?;
    eprintln!(
        "buzz-a2a-acp: resolved Agent Record {} ({})",
        record.content_digest,
        record.verification.label()
    );
    let (resolved, source) = resolve_card(record.record, record.base.as_ref()).await?;
    if source == CardSource::DeprecatedCardData {
        eprintln!(
            "buzz-a2a-acp: using deprecated OASF integration/a2a data.card_data compatibility path"
        );
    }
    let mut sessions = HashSet::new();
    let mut lines = spawn_line_reader(BufReader::new(tokio::io::stdin()));
    let mut writer = tokio::io::stdout();
    let mut active_prompt: Option<ActivePrompt> = None;
    loop {
        let event = if let Some(active) = active_prompt.as_mut() {
            tokio::select! {
                line = lines.recv() => LoopEvent::Input(line),
                result = &mut active.task => LoopEvent::PromptFinished(result),
            }
        } else {
            LoopEvent::Input(lines.recv().await)
        };
        match event {
            LoopEvent::PromptFinished(result) => {
                let Some(active) = active_prompt.take() else {
                    return Err(AdapterError::Acp(
                        "prompt completed without an active request".into(),
                    ));
                };
                match result {
                    Ok(Ok(text)) => {
                        for value in prompt_success(active.id, &active.session_id, &text) {
                            send_json(&mut writer, value).await?;
                        }
                    }
                    Ok(Err(error)) => {
                        send_json(
                            &mut writer,
                            json!({ "jsonrpc": "2.0", "id": active.id, "error": { "code": -32000, "message": error.to_string() } }),
                        )
                        .await?;
                    }
                    Err(error) => {
                        send_json(
                            &mut writer,
                            json!({ "jsonrpc": "2.0", "id": active.id, "error": { "code": -32000, "message": format!("remote prompt task failed: {error}") } }),
                        )
                        .await?;
                    }
                }
            }
            LoopEvent::Input(None | Some(Ok(None))) => return Ok(()),
            LoopEvent::Input(Some(Err(error))) => {
                eprintln!("buzz-a2a-acp: ignored malformed ACP input: {error}");
            }
            LoopEvent::Input(Some(Ok(Some(line)))) => {
                let message: Value = match serde_json::from_str(line.trim()) {
                    Ok(message) => message,
                    Err(error) => {
                        eprintln!("buzz-a2a-acp: ignored malformed JSON-RPC line: {error}");
                        continue;
                    }
                };
                let action = match handle_acp_message(
                    &message,
                    &mut sessions,
                    resolved.card.name.as_deref().unwrap_or("remote-a2a-agent"),
                    config.context_id.as_deref(),
                ) {
                    Ok(action) => action,
                    Err(error) => {
                        if let Some(id) = message.get("id").cloned() {
                            send_json(
                                &mut writer,
                                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": error.to_string() } }),
                            )
                            .await?;
                        } else {
                            eprintln!("buzz-a2a-acp: ignored invalid notification: {error}");
                        }
                        continue;
                    }
                };
                match action {
                    Some(AcpAction::Response(response)) => send_json(&mut writer, response).await?,
                    Some(AcpAction::Prompt {
                        id,
                        session_id,
                        text,
                    }) => {
                        if active_prompt.is_some() {
                            send_json(
                                &mut writer,
                                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32001, "message": "another prompt is already active" } }),
                            )
                            .await?;
                            continue;
                        }
                        let prompt_resolved = resolved.clone();
                        let prompt_token = config.bearer_token.clone();
                        let prompt_token_endpoint = config.bearer_token_endpoint.clone();
                        let prompt_task_poll_secs = config.task_poll_secs;
                        let prompt_session_id = session_id.clone();
                        let task = tokio::spawn(async move {
                            invoke(
                                &prompt_resolved,
                                prompt_token.as_deref(),
                                prompt_token_endpoint.as_deref(),
                                prompt_task_poll_secs,
                                &prompt_session_id,
                                &text,
                            )
                            .await
                        });
                        active_prompt = Some(ActivePrompt {
                            id,
                            session_id,
                            task,
                        });
                    }
                    Some(AcpAction::Cancel { id, session_id }) => {
                        if active_prompt
                            .as_ref()
                            .is_some_and(|active| active.session_id == session_id)
                        {
                            let Some(active) = active_prompt.take() else {
                                return Err(AdapterError::Acp(
                                    "matching prompt disappeared during cancellation".into(),
                                ));
                            };
                            active.task.abort();
                            send_json(
                                &mut writer,
                                json!({ "jsonrpc": "2.0", "id": active.id, "result": { "stopReason": "cancelled" } }),
                            )
                            .await?;
                            if let Some(id) = id {
                                send_json(
                                    &mut writer,
                                    json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                                )
                                .await?;
                            }
                        } else if let Some(id) = id {
                            send_json(
                                &mut writer,
                                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": "no active prompt for session" } }),
                            )
                            .await?;
                        }
                    }
                    None => {}
                }
            }
        }
    }
}

fn spawn_line_reader<R>(
    mut reader: R,
) -> tokio::sync::mpsc::Receiver<Result<Option<String>, AdapterError>>
where
    R: tokio::io::AsyncBufRead + Send + Unpin + 'static,
{
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    tokio::spawn(async move {
        loop {
            let line = read_bounded_line(&mut reader).await;
            let reached_eof = matches!(line, Ok(None));
            let transport_failed = matches!(line, Err(AdapterError::Read { .. }));
            if sender.send(line).await.is_err() || reached_eof || transport_failed {
                break;
            }
        }
    });
    receiver
}

async fn read_bounded_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<String>, AdapterError> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader
            .fill_buf()
            .await
            .map_err(|source| AdapterError::Read {
                what: "ACP request",
                source,
            })?;
        if chunk.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            return Err(AdapterError::Acp("unterminated request at EOF".into()));
        }
        let take = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > MAX_ACP_LINE_BYTES {
            let ended = chunk[..take].ends_with(b"\n");
            reader.consume(take);
            if !ended {
                discard_until_newline(reader).await?;
            }
            return Err(AdapterError::Acp("request exceeds 1 MiB".into()));
        }
        bytes.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if bytes.ends_with(b"\n") {
            bytes.pop();
            if bytes.ends_with(b"\r") {
                bytes.pop();
            }
            return String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| AdapterError::Acp("request is not UTF-8".into()));
        }
    }
}

async fn discard_until_newline<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<(), AdapterError> {
    loop {
        let chunk = reader
            .fill_buf()
            .await
            .map_err(|source| AdapterError::Read {
                what: "ACP request",
                source,
            })?;
        if chunk.is_empty() {
            return Ok(());
        }
        let take = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |index| index + 1);
        let ended = chunk[..take].ends_with(b"\n");
        reader.consume(take);
        if ended {
            return Ok(());
        }
    }
}

/// Run the adapter as a normal CLI process. Sprig uses this entry point for
/// the `buzz-a2a-acp` multicall personality.
pub fn run_cli() -> Result<(), String> {
    let args = Cli::parse();
    let bearer_token = std::env::var("BUZZ_A2A_BEARER_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("build runtime: {error}"))?
        .block_on(run(AdapterConfig {
            record: args.record,
            bearer_token,
            bearer_token_endpoint: args.bearer_token_endpoint,
            context_id: args.context_id,
            task_poll_secs: args.task_poll_secs,
        }))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn card(endpoint: &str) -> Value {
        json!({ "id": "example-agent", "name": "Example Agent", "serviceEndpoint": endpoint })
    }

    #[tokio::test]
    async fn resolves_oasf_artifact_and_validates_descriptor() {
        let card = card("http://127.0.0.1:1337/a2a");
        let bytes = serde_json::to_vec(&card).expect("test card serializes");
        let digest = format!("sha256:{}", hex::encode(Sha256::digest(&bytes)));
        let record = json!({ "name": "Example Agent", "schema_version": "1.0.0", "modules": [{ "name": "integration/a2a", "id": 203, "artifact": { "json": card, "digest": digest, "media_type": "application/a2a-agent-card+json", "size": bytes.len() } }] });
        let path = std::env::temp_dir().join(format!(
            "buzz-a2a-record-{}-{}.json",
            std::process::id(),
            REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(
            &path,
            serde_json::to_vec(&record).expect("test record serializes"),
        )
        .expect("write record");
        let loaded = load_record(path.to_str().expect("temp path is utf8"))
            .await
            .expect("load record");
        let (resolved, source) = resolve_card(loaded.record, loaded.base.as_ref())
            .await
            .expect("resolve card");
        assert_eq!(source, CardSource::Artifact);
        assert_eq!(
            resolved.mode,
            ProtocolMode::VendorServiceEndpoint {
                endpoint: "http://127.0.0.1:1337/a2a".into()
            }
        );
        let _ = fs::remove_file(path);
    }

    #[tokio::test]
    async fn accepts_one_record_collection_and_preserves_embedded_artifact_bytes() {
        let raw_card = r#"{"name":"Example Agent","version":"1.0.0","supportedInterfaces":[{"url":"http://127.0.0.1:1337/a2a/agent","protocolBinding":"JSONRPC","protocolVersion":"1.0"}]}"#;
        let digest = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(raw_card.as_bytes()))
        );
        let record = format!(
            r#"[{{"name":"Example Agent","schema_version":"1.0.0","modules":[{{"name":"integration/a2a","id":203,"artifact":{{"json":{raw_card},"digest":"{digest}","media_type":"application/a2a-agent-card+json","size":{}}}}}]}}]"#,
            raw_card.len()
        );
        let path = std::env::temp_dir().join(format!(
            "buzz-a2a-record-collection-{}-{}.json",
            std::process::id(),
            REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&path, record).expect("write record collection");
        let record = load_record(path.to_str().expect("temp path is utf8"))
            .await
            .expect("load one-record collection");
        let (resolved, source) = resolve_card(record.record, record.base.as_ref())
            .await
            .expect("resolve exact embedded artifact bytes");
        assert_eq!(source, CardSource::Artifact);
        assert_eq!(resolved.mode.endpoint(), "http://127.0.0.1:1337/a2a/agent");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn prefers_declared_jsonrpc_interface() {
        let card: AgentCard = serde_json::from_value(json!({ "supportedInterfaces": [{ "url": "https://agent.example/rpc", "protocolBinding": "JSONRPC", "protocolVersion": "1.0" }], "serviceEndpoint": "https://legacy.example/a2a" })).expect("card");
        assert_eq!(
            select_protocol_mode(&card).expect("mode"),
            ProtocolMode::JsonRpc {
                endpoint: "https://agent.example/rpc".into(),
                protocol_version: Some("1.0".into()),
            }
        );
    }

    #[test]
    fn builds_current_and_vendor_requests() {
        let current = request_payload(
            &ProtocolMode::JsonRpc {
                endpoint: "https://agent.example/rpc".into(),
                protocol_version: Some("1.0".into()),
            },
            Some("remote"),
            1,
            "session",
            "hello",
        );
        assert_eq!(current["method"], "SendMessage");
        assert_eq!(current["params"]["message"]["role"], "ROLE_USER");
        assert!(current["params"]["message"]["parts"][0]["kind"].is_null());
        assert_eq!(current["params"]["message"]["parts"][0]["text"], "hello");
        let vendor = request_payload(
            &ProtocolMode::VendorServiceEndpoint {
                endpoint: "http://127.0.0.1:1337/a2a".into(),
            },
            Some("remote"),
            2,
            "session",
            "hello",
        );
        assert_eq!(vendor["method"], "agent/sendMessage");
        assert_eq!(vendor["params"]["agentId"], "remote");
    }

    #[test]
    fn sends_a2a_version_header_for_standard_modes_only() {
        let client = Client::new();
        for (mode, expected) in [
            (
                ProtocolMode::JsonRpc {
                    endpoint: "https://agent.example/rpc".into(),
                    protocol_version: Some("1.0".into()),
                },
                Some("1.0"),
            ),
            (
                ProtocolMode::JsonRpc {
                    endpoint: "https://agent.example/rpc".into(),
                    protocol_version: Some("0.3".into()),
                },
                Some("0.3"),
            ),
            (
                ProtocolMode::VendorServiceEndpoint {
                    endpoint: "https://agent.example/rpc".into(),
                },
                None,
            ),
        ] {
            let request = protocol_request(&client, &mode, mode.endpoint())
                .build()
                .expect("request builds");
            assert_eq!(
                request
                    .headers()
                    .get("A2A-Version")
                    .and_then(|value| value.to_str().ok()),
                expected
            );
        }
    }

    #[test]
    fn builds_a2a_0_3_request_and_preserves_context() {
        let request = request_payload(
            &ProtocolMode::JsonRpc {
                endpoint: "https://agent.example/rpc".into(),
                protocol_version: Some("0.3".into()),
            },
            Some("remote"),
            3,
            "buzz-session",
            "hello",
        );
        assert_eq!(request["method"], "message/send");
        assert!(request["params"]["contextId"].is_null());
        assert_eq!(request["params"]["message"]["contextId"], "buzz-session");
        assert_eq!(request["params"]["message"]["parts"][0]["kind"], "text");
    }

    #[test]
    fn rejects_non_loopback_http_endpoints() {
        let card: AgentCard = serde_json::from_value(json!({
            "supportedInterfaces": [{
                "url": "http://remote.example/a2a",
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }]
        }))
        .expect("card");
        assert!(matches!(
            select_protocol_mode(&card),
            Err(AdapterError::UnsafeEndpoint(_))
        ));
        let private_card: AgentCard = serde_json::from_value(json!({
            "supportedInterfaces": [{
                "url": "https://127.0.0.1/a2a",
                "protocolBinding": "JSONRPC",
                "protocolVersion": "1.0"
            }]
        }))
        .expect("card");
        assert!(matches!(
            select_protocol_mode(&private_card),
            Err(AdapterError::UnsafeEndpoint(_))
        ));
    }

    #[tokio::test]
    async fn pinned_localhost_client_falls_back_across_checked_addresses() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind IPv4 listener");
        let port = listener.local_addr().expect("listener address").port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .expect("write response");
        });
        let url = Url::parse(&format!("http://localhost:{port}/a2a")).expect("url");
        let addresses = [
            SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), port),
            SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port),
        ];
        validate_resolved_addresses(&url, &addresses).expect("loopback addresses");

        let response = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            pinned_http_client(&url, &addresses)
                .expect("pinned client")
                .get(url)
                .send(),
        )
        .await
        .expect("connect using validated fallback")
        .expect("HTTP response");
        assert_eq!(response.status(), StatusCode::OK);
        server.await.expect("server task");
    }

    #[test]
    fn resolver_policy_rejects_mixed_or_private_addresses() {
        let localhost = Url::parse("http://localhost:1337/a2a").expect("url");
        assert!(validate_resolved_addresses(
            &localhost,
            &[
                SocketAddr::from(([127, 0, 0, 1], 1337)),
                SocketAddr::from(([8, 8, 8, 8], 1337))
            ]
        )
        .is_err());
        let public = Url::parse("https://agent.example/a2a").expect("url");
        assert!(validate_resolved_addresses(
            &public,
            &[
                SocketAddr::from(([8, 8, 8, 8], 443)),
                SocketAddr::from(([10, 0, 0, 1], 443))
            ]
        )
        .is_err());
        assert!(
            validate_resolved_addresses(&public, &[SocketAddr::from(([8, 8, 8, 8], 443))]).is_ok()
        );
    }

    #[test]
    fn reviewed_endpoint_is_enforced_even_without_a_token() {
        assert!(validate_endpoint_binding(
            None,
            Some("https://reviewed.example/a2a"),
            "https://reviewed.example/a2a"
        )
        .is_ok());
        assert!(matches!(
            validate_endpoint_binding(
                None,
                Some("https://reviewed.example/a2a"),
                "https://other.example/a2a"
            ),
            Err(AdapterError::UnauthorizedTokenEndpoint(_))
        ));
        assert!(matches!(
            validate_endpoint_binding(Some("secret"), None, "https://agent.example/a2a"),
            Err(AdapterError::UnauthorizedTokenEndpoint(_))
        ));
    }

    #[tokio::test]
    async fn deprecated_card_data_is_explicit_compatibility_path() {
        let record: AgentRecord = serde_json::from_value(json!({ "modules": [{ "name": "integration/a2a", "data": { "card_data": card("http://127.0.0.1:1337/a2a"), "card_schema_version": "0.3" } }] })).expect("record");
        let (_, source) = resolve_card(record, None).await.expect("resolve card");
        assert_eq!(source, CardSource::DeprecatedCardData);
    }

    #[tokio::test]
    async fn digest_mismatch_is_rejected() {
        let artifact = json!({ "name": "wrong" });
        let artifact_bytes = serde_json::to_vec(&artifact).expect("artifact serializes");
        let descriptor: Descriptor = serde_json::from_value(json!({
            "json": artifact,
            "digest": "sha256:00",
            "media_type": "application/json",
            "size": artifact_bytes.len()
        }))
        .expect("descriptor");
        let err = descriptor_bytes(&descriptor, None)
            .await
            .expect_err("mismatch");
        assert!(err.to_string().contains("digest mismatch"));
    }

    #[tokio::test]
    async fn missing_oasf_descriptor_media_type_is_rejected() {
        let artifact = json!({ "name": "missing media type" });
        let artifact_bytes = serde_json::to_vec(&artifact).expect("artifact serializes");
        let descriptor: Descriptor = serde_json::from_value(json!({
            "json": artifact,
            "digest": format!("sha256:{}", hex::encode(Sha256::digest(&artifact_bytes))),
            "size": artifact_bytes.len()
        }))
        .expect("descriptor");
        let err = descriptor_bytes(&descriptor, None)
            .await
            .expect_err("missing media_type");
        assert!(err.to_string().contains("requires media_type"));
    }

    #[tokio::test]
    async fn acp_transcript_handles_initialize_new_and_prompt() {
        let mut sessions = HashSet::new();
        let initialize = handle_acp_message(
            &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": 1 } }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("initialize action")
        .expect("initialize response");
        let AcpAction::Response(initialize) = initialize else {
            panic!("initialize must return a response");
        };
        assert_eq!(initialize["result"]["protocolVersion"], 1);
        assert_eq!(initialize["result"]["agentInfo"]["name"], "remote-agent");

        let new = handle_acp_message(
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {} }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("session/new action")
        .expect("session/new response");
        let AcpAction::Response(new) = new else {
            panic!("session/new must return a response");
        };
        let session_id = new["result"]["sessionId"]
            .as_str()
            .expect("session id")
            .to_owned();
        assert!(sessions.contains(&session_id));

        let prompt = handle_acp_message(
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "session/prompt",
                "params": { "sessionId": session_id, "prompt": [{ "type": "text", "text": "ship it" }] }
            }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("session/prompt action")
        .expect("session/prompt request");
        let AcpAction::Prompt {
            id,
            session_id,
            text,
        } = prompt
        else {
            panic!("session/prompt must invoke the remote runtime");
        };
        assert_eq!(id, 3);
        assert_eq!(text, "ship it");
        let [update, result] = prompt_success(id, &session_id, "done");
        assert_eq!(update["method"], "session/update");
        assert_eq!(update["params"]["sessionId"], session_id);
        assert_eq!(update["params"]["update"]["content"]["text"], "done");
        assert_eq!(result["result"]["stopReason"], "end_turn");
    }

    #[test]
    fn terminal_task_polling_distinguishes_working_and_terminal_states() {
        assert_eq!(
            task_outcome(&json!({ "status": { "state": "working" } }), "task-1")
                .expect("working is nonterminal"),
            None,
        );
        assert_eq!(
            task_outcome(
                &json!({ "status": { "state": "TASK_STATE_SUBMITTED" } }),
                "task-1"
            )
            .expect("protobuf-style submitted is nonterminal"),
            None,
        );
        assert_eq!(
            task_outcome(&json!({ "status": { "state": "completed" } }), "task-1")
                .expect("completed is successful"),
            Some("A2A task task-1 completed".into()),
        );
        assert_eq!(
            task_outcome(
                &json!({ "status": { "state": "TASK_STATE_COMPLETED" } }),
                "task-1"
            )
            .expect("protobuf-style completed is successful"),
            Some("A2A task task-1 completed".into()),
        );
        let failed = task_outcome(
            &json!({
                "status": { "state": "TASK_STATE_FAILED" },
                "parts": [{ "text": "remote failure" }]
            }),
            "task-1",
        );
        assert!(failed
            .expect_err("failed tasks are ACP errors")
            .to_string()
            .contains("remote failure"));
    }

    #[test]
    fn unwraps_current_a2a_task_and_message_results() {
        let task = json!({ "task": { "id": "task-2", "status": { "state": "completed" }, "artifacts": [{ "parts": [{ "text": "complete" }] }] } });
        let task_result = task.get("task").expect("task result");
        assert_eq!(task_result["id"], "task-2");
        assert_eq!(
            task_outcome(task_result, "task-2").expect("completed task"),
            Some("complete".into()),
        );

        let message = json!({ "message": { "messageId": "m-1", "role": "ROLE_AGENT", "parts": [{ "text": "direct response" }] } });
        let message_result = message.get("message").expect("message result");
        assert_eq!(extract_text(message_result), Some("direct response".into()));
    }

    #[tokio::test]
    async fn acp_reader_accepts_multiple_bounded_lines() {
        let input = b"{\"method\":\"initialize\"}\r\n{\"method\":\"session/new\"}\n";
        let mut reader = BufReader::new(&input[..]);
        assert_eq!(
            read_bounded_line(&mut reader).await.expect("first line"),
            Some("{\"method\":\"initialize\"}".into())
        );
        assert_eq!(
            read_bounded_line(&mut reader).await.expect("second line"),
            Some("{\"method\":\"session/new\"}".into())
        );
        assert_eq!(read_bounded_line(&mut reader).await.expect("eof"), None);
    }

    #[tokio::test]
    async fn spawned_reader_preserves_partial_lines_while_other_work_completes() {
        use tokio::io::AsyncWriteExt;

        let (mut writer, reader) = tokio::io::duplex(32 * 1024);
        let mut lines = spawn_line_reader(BufReader::with_capacity(8 * 1024, reader));
        let line = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"session/prompt\",\"padding\":\"{}\"}}",
            "P".repeat(20 * 1024)
        );
        writer
            .write_all(&line.as_bytes()[..12 * 1024])
            .await
            .expect("write first fragment");
        tokio::task::yield_now().await;
        writer
            .write_all(&line.as_bytes()[12 * 1024..])
            .await
            .expect("write second fragment");
        writer.write_all(b"\n").await.expect("finish line");

        assert_eq!(
            lines
                .recv()
                .await
                .expect("reader remains available")
                .expect("line is valid"),
            Some(line),
        );
    }

    #[tokio::test]
    async fn spawned_reader_stops_after_a_transport_error() {
        use std::{
            pin::Pin,
            sync::{
                atomic::{AtomicUsize, Ordering},
                Arc,
            },
            task::{Context, Poll},
        };
        use tokio::io::{AsyncBufRead, AsyncRead, ReadBuf};

        struct FailingReader {
            reads: Arc<AtomicUsize>,
        }

        impl AsyncRead for FailingReader {
            fn poll_read(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buf: &mut ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Err(std::io::Error::other("transport failed")))
            }
        }

        impl AsyncBufRead for FailingReader {
            fn poll_fill_buf(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::io::Result<&[u8]>> {
                self.reads.fetch_add(1, Ordering::Relaxed);
                Poll::Ready(Err(std::io::Error::other("transport failed")))
            }

            fn consume(self: Pin<&mut Self>, _amount: usize) {}
        }

        let reads = Arc::new(AtomicUsize::new(0));
        let mut lines = spawn_line_reader(FailingReader {
            reads: Arc::clone(&reads),
        });
        assert!(matches!(
            lines.recv().await,
            Some(Err(AdapterError::Read {
                what: "ACP request",
                ..
            }))
        ));
        assert!(lines.recv().await.is_none());
        assert_eq!(reads.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn idless_notifications_do_not_produce_json_rpc_responses() {
        let mut sessions = HashSet::new();
        let action = handle_acp_message(
            &json!({ "jsonrpc": "2.0", "method": "unknown/notification" }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("notification is valid");
        assert!(action.is_none());
    }

    #[test]
    fn cancel_notification_remains_actionable_without_an_id() {
        let mut sessions = HashSet::new();
        let action = handle_acp_message(
            &json!({
                "jsonrpc": "2.0",
                "method": "session/cancel",
                "params": { "sessionId": "session-1" }
            }),
            &mut sessions,
            "remote-agent",
            None,
        )
        .expect("cancel is valid")
        .expect("cancel action");
        let AcpAction::Cancel { id, session_id } = action else {
            panic!("cancel notification must produce a local action");
        };
        assert!(id.is_none());
        assert_eq!(session_id, "session-1");
    }

    #[test]
    fn private_ipv4_transitional_ipv6_addresses_are_rejected() {
        for address in [
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::0a00:0001",
            "2002:0a00:0001::",
            "2001:0000:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(
                is_private_ip(address.parse().expect("test IP")),
                "{address} must not bypass the private-address policy"
            );
        }
    }

    #[tokio::test]
    async fn remote_artifact_requires_a_digest_before_fetch() {
        let descriptor: Descriptor = serde_json::from_value(json!({
            "urls": ["https://agent.example/card.json"],
            "media_type": "application/a2a-agent-card+json",
            "size": 1
        }))
        .expect("descriptor");
        let error = descriptor_bytes(&descriptor, None)
            .await
            .expect_err("unsigned remote artifact");
        assert!(error.to_string().contains("require a sha256 digest"));
    }
}
