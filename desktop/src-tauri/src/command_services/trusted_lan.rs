use crate::command_services::ssh::ProtectedFile;
use serde::Deserialize;
use std::net::IpAddr;
use std::path::Path;
use url::{Host, Url};

const MAXIMUM_CONFIG_BYTES: u64 = 64 * 1024;
const TRUSTED_MODE: &str = "OFFICIAL_TRUSTED_LAN";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrustedLanEndpoint(String);

impl TrustedLanEndpoint {
    pub(crate) fn parse_memory(value: &str) -> Result<Self, TrustedLanError> {
        Self::parse(value, "/mcp")
    }

    pub(crate) fn parse_rag(value: &str) -> Result<Self, TrustedLanError> {
        Self::parse(value, "/mcp/")
    }

    fn parse(value: &str, required_path: &str) -> Result<Self, TrustedLanError> {
        let parsed = Url::parse(value).map_err(|_| TrustedLanError::InvalidEndpoint)?;
        let host = match parsed.host() {
            Some(Host::Ipv4(host)) if host.is_private() => IpAddr::V4(host),
            _ => return Err(TrustedLanError::InvalidEndpoint),
        };
        if parsed.scheme() != "http"
            || parsed.port().is_none()
            || parsed.path() != required_path
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.host_str() != Some(&host.to_string())
        {
            return Err(TrustedLanError::InvalidEndpoint);
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CloudProviderConfig {
    enabled: bool,
    endpoint: String,
    model: String,
    keychain_key: String,
}

impl CloudProviderConfig {
    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn keychain_key(&self) -> &str {
        &self.keychain_key
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TrustedLanConfig {
    memory_url: TrustedLanEndpoint,
    rag_url: TrustedLanEndpoint,
    litellm: CloudProviderConfig,
    openai: CloudProviderConfig,
}

impl TrustedLanConfig {
    pub(crate) fn load(path: &Path) -> Result<Self, TrustedLanError> {
        let bytes = ProtectedFile::open(path, MAXIMUM_CONFIG_BYTES)
            .and_then(|file| file.read_all())
            .map_err(|_| TrustedLanError::UnprotectedConfig)?;
        Self::parse(&bytes)
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, TrustedLanError> {
        let raw: RawTrustedLanConfig =
            serde_json::from_slice(bytes).map_err(|_| TrustedLanError::InvalidConfig)?;
        if raw.schema_version != 1
            || raw.mode != TRUSTED_MODE
            || !raw.automatic_cloud_fallback_acknowledged
        {
            return Err(TrustedLanError::InvalidConfig);
        }
        let memory_url = TrustedLanEndpoint::parse_memory(&raw.memory_url)?;
        let rag_url = TrustedLanEndpoint::parse_rag(&raw.rag_url)?;
        let litellm = validate_cloud(raw.litellm, CloudKind::LiteLlm)?;
        let openai = validate_cloud(raw.openai, CloudKind::OpenAi)?;
        Ok(Self {
            memory_url,
            rag_url,
            litellm,
            openai,
        })
    }

    pub(crate) fn memory_url(&self) -> &TrustedLanEndpoint {
        &self.memory_url
    }

    pub(crate) fn rag_url(&self) -> &TrustedLanEndpoint {
        &self.rag_url
    }

    pub(crate) fn litellm(&self) -> &CloudProviderConfig {
        &self.litellm
    }

    pub(crate) fn openai(&self) -> &CloudProviderConfig {
        &self.openai
    }
}

#[derive(Clone, Copy)]
enum CloudKind {
    LiteLlm,
    OpenAi,
}

fn validate_cloud(
    raw: RawCloudProviderConfig,
    kind: CloudKind,
) -> Result<CloudProviderConfig, TrustedLanError> {
    if !raw.enabled || !valid_identifier(&raw.model, 160) || !valid_keychain_key(&raw.keychain_key)
    {
        return Err(TrustedLanError::InvalidConfig);
    }
    let endpoint = Url::parse(&raw.endpoint).map_err(|_| TrustedLanError::InvalidEndpoint)?;
    let valid_endpoint = match kind {
        CloudKind::LiteLlm => {
            endpoint.scheme() == "http"
                && matches!(endpoint.host(), Some(Host::Ipv4(host)) if host.is_private())
                && endpoint.port().is_some()
                && endpoint.path() == "/v1/chat/completions"
        }
        CloudKind::OpenAi => {
            endpoint.scheme() == "https"
                && endpoint.host_str() == Some("api.openai.com")
                && endpoint.port().is_none()
                && endpoint.path() == "/v1/responses"
        }
    };
    if !valid_endpoint
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(TrustedLanError::InvalidEndpoint);
    }
    Ok(CloudProviderConfig {
        enabled: raw.enabled,
        endpoint: raw.endpoint,
        model: raw.model,
        keychain_key: raw.keychain_key,
    })
}

fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/'))
}

fn valid_keychain_key(value: &str) -> bool {
    value.starts_with("command.cloud.") && valid_identifier(value, 128)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrustedLanConfig {
    schema_version: u32,
    mode: String,
    memory_url: String,
    rag_url: String,
    automatic_cloud_fallback_acknowledged: bool,
    litellm: RawCloudProviderConfig,
    openai: RawCloudProviderConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCloudProviderConfig {
    enabled: bool,
    endpoint: String,
    model: String,
    keychain_key: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrustedLanError {
    InvalidConfig,
    InvalidEndpoint,
    UnprotectedConfig,
}
