use crate::{
    net::{read_source, validate_http_url},
    select_protocol_mode, AdapterError, AgentCard, ResolvedAgent, MAX_ARTIFACT_BYTES,
    MAX_RECORD_BYTES,
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{value::RawValue, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use url::Url;

#[derive(Debug, Deserialize)]
pub(super) struct AgentRecord {
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
pub(super) enum RecordVerification {
    OperatorReviewedLocal,
    TlsOnly,
}

impl RecordVerification {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::OperatorReviewedLocal => "operator-reviewed-local",
            Self::TlsOnly => "tls-only",
        }
    }
}

pub(super) struct ResolvedRecord {
    pub(super) record: AgentRecord,
    pub(super) base: Option<Url>,
    pub(super) content_digest: String,
    pub(super) verification: RecordVerification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CardSource {
    Artifact,
    DeprecatedCardData,
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
pub(super) struct Descriptor {
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

pub(super) async fn load_record(source: &str) -> Result<ResolvedRecord, AdapterError> {
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

pub(super) async fn descriptor_bytes(
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

pub(super) async fn resolve_card(
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
