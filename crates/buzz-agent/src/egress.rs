use std::collections::{BTreeMap, BTreeSet, HashSet};

use reqwest::Url;
use serde::Deserialize;

use crate::command_evidence::CommandEvidenceGate;
use crate::lmstudio::{EphemeralMcpIntegration, LmStudioChatResponse, LmStudioOutput};
use crate::types::ExecutedToolProvider;

const MAX_INTEGRATIONS_JSON_BYTES: usize = 64 * 1024;
const MAX_MCP_INTEGRATIONS: usize = 8;
const MAX_SERVER_LABEL_BYTES: usize = 64;
const MAX_MCP_URL_BYTES: usize = 2 * 1024;
const MAX_TOOLS_PER_INTEGRATION: usize = 64;
const MAX_TOTAL_ALLOWED_TOOLS: usize = 256;
const MAX_TOOL_NAME_BYTES: usize = 128;
const MAX_API_TOKEN_BYTES: usize = 4 * 1024;
const MAX_MCP_BEARER_TOKEN_BYTES: usize = 256;

/// Information-handling classification enforced at the model egress boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Material explicitly approved for public processing.
    Public,
    /// Official material which must remain on the local runtime.
    Official,
}

impl Classification {
    /// Parses the exact classification vocabulary, defaulting omissions to
    /// `OFFICIAL`.
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw {
            None => Ok(Self::Official),
            Some("PUBLIC") => Ok(Self::Public),
            Some("OFFICIAL") => Ok(Self::Official),
            Some(other) => Err(format!(
                "config: BUZZ_AGENT_CLASSIFICATION={other:?} not supported (use PUBLIC|OFFICIAL)"
            )),
        }
    }
}

/// Validated native LM Studio base endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LmStudioEndpoint(Url);

impl LmStudioEndpoint {
    /// Parses a native LM Studio endpoint.
    ///
    /// Phase 2 intentionally limits both classifications to a literal local
    /// loopback HTTP listener. PUBLIC cloud routing remains on the existing
    /// providers and is not enabled through the native LM Studio runtime.
    pub fn parse(raw: &str, _classification: Classification) -> Result<Self, String> {
        validate_literal_loopback_authority(raw)?;
        let url = Url::parse(raw).map_err(|e| format!("config: invalid LM Studio URL: {e}"))?;
        if url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
            || !matches!(url.path(), "" | "/")
            || url.port().is_none_or(|port| port == 0)
            || !matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        {
            return Err(
                "config: LM Studio endpoint must be literal http://127.0.0.1:<port> or http://[::1]:<port>"
                    .into(),
            );
        }
        Ok(Self(url))
    }

    fn request_url(&self, path: &str) -> Url {
        let mut url = self.0.clone();
        url.set_path(path);
        url
    }

    fn same_origin(&self, candidate: &Url) -> bool {
        self.0.scheme() == candidate.scheme()
            && self.0.host_str() == candidate.host_str()
            && self.0.port() == candidate.port()
    }
}

fn validate_literal_loopback_authority(raw: &str) -> Result<(), String> {
    let remainder = raw
        .strip_prefix("http://")
        .ok_or_else(|| "config: LM Studio endpoint must use literal loopback HTTP".to_string())?;
    let authority = remainder
        .split(['/', '?', '#'])
        .next()
        .ok_or_else(|| "config: LM Studio endpoint is missing an authority".to_string())?;
    let port = authority
        .strip_prefix("127.0.0.1:")
        .or_else(|| authority.strip_prefix("[::1]:"))
        .ok_or_else(|| {
            "config: LM Studio endpoint must use literal 127.0.0.1 or [::1]".to_string()
        })?;
    if port.is_empty()
        || !port.bytes().all(|byte| byte.is_ascii_digit())
        || port.starts_with('0')
        || port.parse::<u16>().is_err()
    {
        return Err("config: LM Studio endpoint requires a valid explicit port".into());
    }
    Ok(())
}

/// A validated explicit native MCP integration.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeMcpIntegration {
    server_label: String,
    endpoint: LoopbackMcpEndpoint,
    allowed_tools: Vec<String>,
    authorization: String,
}

impl std::fmt::Debug for NativeMcpIntegration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeMcpIntegration")
            .field("server_label", &self.server_label)
            .field("endpoint", &self.endpoint)
            .field("allowed_tools", &self.allowed_tools)
            .field("authorization", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoopbackMcpEndpoint(Url);

impl LoopbackMcpEndpoint {
    fn parse(raw: &str, server_label: &str) -> Result<Self, String> {
        if raw.len() > MAX_MCP_URL_BYTES {
            return Err(format!(
                "config: MCP server URL exceeds {MAX_MCP_URL_BYTES} bytes"
            ));
        }
        validate_literal_loopback_authority(raw)?;
        let url = Url::parse(raw).map_err(|e| format!("config: invalid MCP server URL: {e}"))?;
        let expected_path = if server_label == "rag" {
            "/mcp/"
        } else {
            "/mcp"
        };
        if url.scheme() != "http"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
            || url.port().is_none()
            || url.host_str() != Some("127.0.0.1")
            || url.path() != expected_path
        {
            return Err(format!(
                "config: MCP server URL for {server_label:?} must use exact path {expected_path}"
            ));
        }
        Ok(Self(url))
    }
}

impl NativeMcpIntegration {
    /// Returns the unique label supplied to LM Studio.
    pub fn server_label(&self) -> &str {
        &self.server_label
    }

    /// Returns the exact non-empty tool allowlist.
    pub fn allowed_tools(&self) -> &[String] {
        &self.allowed_tools
    }

    /// Converts this validated policy record to the native wire contract.
    pub fn to_wire(&self) -> EphemeralMcpIntegration {
        EphemeralMcpIntegration::new(
            self.server_label.clone(),
            self.endpoint.0.as_str(),
            self.allowed_tools.clone(),
            BTreeMap::from([("Authorization".to_string(), self.authorization.clone())]),
        )
    }
}

/// Purpose of one native LM Studio request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRequestPurpose {
    /// Discover currently available local model instances.
    ModelDiscovery,
    /// Start a new native chat response.
    Chat,
    /// Continue a prior stateful native response.
    Continuation,
    /// Produce a bounded context summary.
    Summary,
}

impl NativeRequestPurpose {
    fn path(self) -> &'static str {
        match self {
            Self::ModelDiscovery => "/api/v1/models",
            Self::Chat | Self::Continuation | Self::Summary => "/api/v1/chat",
        }
    }
}

/// Fully validated policy for the LM Studio-native runtime.
#[derive(Clone, PartialEq, Eq)]
pub struct LmStudioRuntimeConfig {
    classification: Classification,
    endpoint: LmStudioEndpoint,
    integrations: Vec<NativeMcpIntegration>,
    evidence_gate: CommandEvidenceGate,
    api_token: Option<String>,
}

impl std::fmt::Debug for LmStudioRuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LmStudioRuntimeConfig")
            .field("classification", &self.classification)
            .field("endpoint", &self.endpoint)
            .field("integrations", &self.integrations)
            .field("evidence_gate", &self.evidence_gate)
            .field("api_token", &self.api_token.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

impl LmStudioRuntimeConfig {
    /// Parses and validates the complete local runtime egress configuration.
    pub fn parse(
        classification: Option<&str>,
        endpoint: &str,
        fallback_provider: Option<&str>,
        integrations_json: Option<&str>,
    ) -> Result<Self, String> {
        Self::parse_with_token_and_evidence(
            classification,
            endpoint,
            fallback_provider,
            integrations_json,
            None,
            None,
        )
    }

    /// Parses the runtime policy with an optional native server bearer token.
    ///
    /// The token is bounded, rejects HTTP control characters, and is redacted
    /// from all debug output.
    pub fn parse_with_token(
        classification: Option<&str>,
        endpoint: &str,
        fallback_provider: Option<&str>,
        integrations_json: Option<&str>,
        api_token: Option<&str>,
    ) -> Result<Self, String> {
        Self::parse_with_token_and_evidence(
            classification,
            endpoint,
            fallback_provider,
            integrations_json,
            api_token,
            None,
        )
    }

    /// Parses the runtime policy with catalog-owned MCP evidence bindings.
    pub fn parse_with_token_and_evidence(
        classification: Option<&str>,
        endpoint: &str,
        fallback_provider: Option<&str>,
        integrations_json: Option<&str>,
        api_token: Option<&str>,
        evidence_policy_json: Option<&str>,
    ) -> Result<Self, String> {
        let classification = Classification::parse(classification)?;
        if fallback_provider.is_some_and(|fallback| !fallback.is_empty()) {
            return Err(
                "config: LM Studio native runtime does not permit a fallback provider".into(),
            );
        }
        let endpoint = LmStudioEndpoint::parse(endpoint, classification)?;
        let integrations = parse_integrations(integrations_json, classification)?;
        let tool_bindings = integrations
            .iter()
            .map(|integration| {
                (
                    integration.server_label.clone(),
                    integration
                        .allowed_tools
                        .iter()
                        .cloned()
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let evidence_gate = CommandEvidenceGate::parse(evidence_policy_json, &tool_bindings)
            .map_err(|error| format!("config: command evidence policy {}", error.code()))?;
        let api_token = match api_token {
            None | Some("") => None,
            Some(token)
                if token.len() <= MAX_API_TOKEN_BYTES
                    && !token.bytes().any(|byte| byte.is_ascii_control()) =>
            {
                Some(token.to_owned())
            }
            Some(_) => {
                return Err(format!(
                    "config: LM_STUDIO_API_TOKEN must be 1..={MAX_API_TOKEN_BYTES} bytes without control characters"
                ));
            }
        };
        Ok(Self {
            classification,
            endpoint,
            integrations,
            evidence_gate,
            api_token,
        })
    }

    /// Returns the enforced classification.
    pub fn classification(&self) -> Classification {
        self.classification
    }

    /// Returns the validated explicit MCP integrations.
    pub fn integrations(&self) -> &[NativeMcpIntegration] {
        &self.integrations
    }

    pub(crate) fn api_token(&self) -> Option<&str> {
        self.api_token.as_deref()
    }

    /// Returns the exact native wire integrations approved by this policy.
    pub fn wire_integrations(&self) -> Vec<EphemeralMcpIntegration> {
        self.integrations
            .iter()
            .map(NativeMcpIntegration::to_wire)
            .collect()
    }

    /// Verifies that all executed tool evidence came from an explicitly
    /// configured ephemeral server and tool allowlist.
    pub fn validate_response_evidence(
        &self,
        response: &LmStudioChatResponse,
    ) -> Result<(), String> {
        for output in &response.output {
            let LmStudioOutput::ToolCall(call) = output else {
                continue;
            };
            let server_label = match &call.provider {
                ExecutedToolProvider::EphemeralMcp { server_label } => server_label,
                ExecutedToolProvider::Plugin { .. } => {
                    return Err(
                        "egress denied: LM Studio plugin tool evidence is not permitted".into(),
                    );
                }
            };
            let authorized = self.integrations.iter().any(|integration| {
                integration.server_label == *server_label
                    && integration
                        .allowed_tools
                        .iter()
                        .any(|tool| tool == &call.name)
            });
            if !authorized {
                return Err(format!(
                    "egress denied: tool {:?} from MCP server {:?} was not explicitly allowlisted",
                    call.name, server_label
                ));
            }
            self.evidence_gate
                .validate_tool_call(call)
                .map_err(|error| format!("command evidence rejected: {}", error.code()))?;
        }
        Ok(())
    }

    /// Builds the exact native request URL for a permitted operation.
    pub fn request_url(&self, purpose: NativeRequestPurpose) -> Result<Url, String> {
        let url = self.endpoint.request_url(purpose.path());
        self.authorize_request(purpose, &url)?;
        Ok(url)
    }

    /// Authorizes a request immediately before network emission.
    pub fn authorize_request(
        &self,
        purpose: NativeRequestPurpose,
        candidate: &Url,
    ) -> Result<(), String> {
        if !self.endpoint.same_origin(candidate)
            || candidate.path() != purpose.path()
            || candidate.query().is_some()
            || candidate.fragment().is_some()
            || !candidate.username().is_empty()
            || candidate.password().is_some()
        {
            return Err(format!(
                "egress denied: {:?} request is outside the configured native LM Studio route",
                purpose
            ));
        }
        validate_literal_loopback_authority(candidate.as_str())?;
        if candidate.scheme() != "http"
            || !matches!(candidate.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
            || candidate.port().is_none()
        {
            return Err(
                "egress denied: native LM Studio route is not literal loopback HTTP".into(),
            );
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIntegration {
    #[serde(rename = "type")]
    integration_type: RawIntegrationType,
    server_label: String,
    server_url: String,
    allowed_tools: Vec<String>,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawIntegrationType {
    EphemeralMcp,
}

fn parse_integrations(
    raw: Option<&str>,
    _classification: Classification,
) -> Result<Vec<NativeMcpIntegration>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    if raw.len() > MAX_INTEGRATIONS_JSON_BYTES {
        return Err(format!(
            "config: LM_STUDIO_MCP_INTEGRATIONS exceeds {MAX_INTEGRATIONS_JSON_BYTES} bytes"
        ));
    }
    let parsed: Vec<RawIntegration> = serde_json::from_str(raw)
        .map_err(|e| format!("config: invalid LM_STUDIO_MCP_INTEGRATIONS: {e}"))?;
    if parsed.len() > MAX_MCP_INTEGRATIONS {
        return Err(format!(
            "config: at most {MAX_MCP_INTEGRATIONS} native MCP integrations are allowed"
        ));
    }

    let mut labels = HashSet::new();
    let mut total_tools = 0usize;
    let mut integrations = Vec::with_capacity(parsed.len());
    for integration in parsed {
        if !matches!(
            integration.integration_type,
            RawIntegrationType::EphemeralMcp
        ) {
            return Err("config: only ephemeral_mcp integrations are allowed".into());
        }
        validate_identifier(
            "server_label",
            &integration.server_label,
            MAX_SERVER_LABEL_BYTES,
        )?;
        if !labels.insert(integration.server_label.clone()) {
            return Err(format!(
                "config: duplicate MCP server label {:?}",
                integration.server_label
            ));
        }
        if integration.allowed_tools.is_empty()
            || integration.allowed_tools.len() > MAX_TOOLS_PER_INTEGRATION
        {
            return Err(format!(
                "config: MCP integration {:?} must allow 1..={MAX_TOOLS_PER_INTEGRATION} tools",
                integration.server_label
            ));
        }
        total_tools = total_tools
            .checked_add(integration.allowed_tools.len())
            .ok_or_else(|| "config: MCP tool count overflow".to_string())?;
        if total_tools > MAX_TOTAL_ALLOWED_TOOLS {
            return Err(format!(
                "config: at most {MAX_TOTAL_ALLOWED_TOOLS} MCP tools may be allowed"
            ));
        }
        let mut tool_names = HashSet::new();
        for tool in &integration.allowed_tools {
            validate_identifier("tool name", tool, MAX_TOOL_NAME_BYTES)?;
            if !tool_names.insert(tool.as_str()) {
                return Err(format!(
                    "config: duplicate allowed tool {tool:?} for MCP integration {:?}",
                    integration.server_label
                ));
            }
        }
        if integration.headers.len() != 1 {
            return Err(format!(
                "config: MCP integration {:?} requires exactly one Authorization header",
                integration.server_label
            ));
        }
        let authorization = integration
            .headers
            .get("Authorization")
            .filter(|value| {
                value.strip_prefix("Bearer ").is_some_and(|token| {
                    (16..=MAX_MCP_BEARER_TOKEN_BYTES).contains(&token.len())
                        && !token
                            .bytes()
                            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
                })
            })
            .ok_or_else(|| {
                format!(
                    "config: MCP integration {:?} requires a bounded Bearer Authorization header",
                    integration.server_label
                )
            })?
            .clone();
        let endpoint =
            LoopbackMcpEndpoint::parse(&integration.server_url, &integration.server_label)?;
        integrations.push(NativeMcpIntegration {
            server_label: integration.server_label,
            endpoint,
            allowed_tools: integration.allowed_tools,
            authorization,
        });
    }
    Ok(integrations)
}

fn validate_identifier(label: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(format!(
            "config: {label} must be 1..={max_bytes} ASCII identifier bytes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence_policy(labels: &[&str]) -> String {
        let services = labels
            .iter()
            .map(|label| match *label {
                "memory" => serde_json::json!({
                    "server_label": "memory",
                    "kind": "memory",
                    "active_identity": "node:command"
                }),
                "rag" => serde_json::json!({
                    "server_label": "rag",
                    "kind": "rag",
                    "active_identity": "f".repeat(64)
                }),
                _ => panic!("unsupported fixture label"),
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "version": 1,
            "maximum_evidence_age_seconds": 3600,
            "services": services,
            "allowed_apple_ids": [],
            "allowed_file_paths": []
        })
        .to_string()
    }

    #[test]
    fn classification_is_exact_and_defaults_to_official() {
        assert_eq!(
            Classification::parse(None).expect("default classification"),
            Classification::Official
        );
        assert_eq!(
            Classification::parse(Some("PUBLIC")).expect("PUBLIC"),
            Classification::Public
        );
        assert_eq!(
            Classification::parse(Some("OFFICIAL")).expect("OFFICIAL"),
            Classification::Official
        );
        for invalid in ["public", "official", " OFFICIAL", "OFFICIAL ", ""] {
            assert!(
                Classification::parse(Some(invalid)).is_err(),
                "{invalid:?} must be rejected"
            );
        }
    }

    #[test]
    fn official_endpoint_accepts_only_literal_loopback_http_with_port() {
        for accepted in [
            "http://127.0.0.1:1234",
            "http://127.0.0.1:1234/",
            "http://[::1]:1234",
            "http://[::1]:1234/",
        ] {
            assert!(
                LmStudioEndpoint::parse(accepted, Classification::Official).is_ok(),
                "{accepted} must be accepted"
            );
        }

        for rejected in [
            "http://localhost:1234",
            "http://127.0.0.2:1234",
            "http://127.1:1234",
            "http://2130706433:1234",
            "http://0.0.0.0:1234",
            "http://[::]:1234",
            "http://192.168.1.4:1234",
            "http://10.0.0.4:1234",
            "http://8.8.8.8:1234",
            "https://127.0.0.1:1234",
            "ftp://127.0.0.1:1234",
            "http://user@127.0.0.1:1234",
            "http://127.0.0.1:1234/#fragment",
            "http://127.0.0.1:1234?query=1",
            "http://127.0.0.1",
            "http://127.0.0.1:1234/api/v1",
            "http://0x7f000001:1234",
            "http://0177.0.0.1:1234",
            "http://[::ffff:127.0.0.1]:1234",
            "http://[::1%25lo0]:1234",
            "http://127.0.0.1:0",
            "http://127.0.0.1:65536",
        ] {
            assert!(
                LmStudioEndpoint::parse(rejected, Classification::Official).is_err(),
                "{rejected} must be rejected"
            );
        }
    }

    #[test]
    fn official_runtime_rejects_fallback_provider() {
        let err = LmStudioRuntimeConfig::parse(
            Some("OFFICIAL"),
            "http://127.0.0.1:1234",
            Some("openai"),
            None,
        )
        .expect_err("OFFICIAL fallback must fail");
        assert!(err.contains("fallback"), "{err}");
    }

    #[test]
    fn native_api_token_is_bounded_and_redacted_from_debug() {
        let token = "native-secret-token";
        let cfg = LmStudioRuntimeConfig::parse_with_token(
            None,
            "http://127.0.0.1:1234",
            None,
            None,
            Some(token),
        )
        .expect("token config");
        let debug = format!("{cfg:?}");
        assert!(!debug.contains(token), "{debug}");
        assert!(debug.contains("[REDACTED]"), "{debug}");

        assert!(LmStudioRuntimeConfig::parse_with_token(
            None,
            "http://127.0.0.1:1234",
            None,
            None,
            Some("bad\nheader")
        )
        .is_err());
        assert!(LmStudioRuntimeConfig::parse_with_token(
            None,
            "http://127.0.0.1:1234",
            None,
            None,
            Some(&"x".repeat(MAX_API_TOKEN_BYTES))
        )
        .is_ok());
        assert!(LmStudioRuntimeConfig::parse_with_token(
            None,
            "http://127.0.0.1:1234",
            None,
            None,
            Some(&"x".repeat(MAX_API_TOKEN_BYTES + 1))
        )
        .is_err());
    }

    #[test]
    fn official_mcp_integrations_are_exact_bounded_and_allowlisted() {
        let valid = r#"[
          {
            "type":"ephemeral_mcp",
            "server_label":"memory",
            "server_url":"http://127.0.0.1:9100/mcp",
            "allowed_tools":["search_events","record_event"],
            "headers":{"Authorization":"Bearer fixture-token-123456"}
          },
          {
            "type":"ephemeral_mcp",
            "server_label":"rag",
            "server_url":"http://127.0.0.1:9200/mcp/",
            "allowed_tools":["search"],
            "headers":{"Authorization":"Bearer fixture-token-654321"}
          }
        ]"#;
        let policy = evidence_policy(&["memory", "rag"]);
        let cfg = LmStudioRuntimeConfig::parse_with_token_and_evidence(
            None,
            "http://127.0.0.1:1234",
            None,
            Some(valid),
            None,
            Some(&policy),
        )
        .expect("valid official config");
        assert_eq!(cfg.classification(), Classification::Official);
        assert_eq!(cfg.integrations().len(), 2);
        assert_eq!(cfg.integrations()[0].server_label(), "memory");
        assert_eq!(
            cfg.integrations()[0].allowed_tools(),
            &["search_events".to_string(), "record_event".to_string()]
        );
    }

    #[test]
    fn mcp_endpoint_requires_authenticated_literal_ipv4_loopback() {
        let valid = r#"[{"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100/mcp","allowed_tools":["search"],"headers":{"Authorization":"Bearer fixture-token-123456"}}]"#;
        let policy = evidence_policy(&["memory"]);
        assert!(LmStudioRuntimeConfig::parse_with_token_and_evidence(
            None,
            "http://127.0.0.1:1234",
            None,
            Some(valid),
            None,
            Some(&policy),
        )
        .is_ok());
        let rag_policy = evidence_policy(&["rag"]);
        let valid_rag = r#"[{"type":"ephemeral_mcp","server_label":"rag","server_url":"http://127.0.0.1:9200/mcp/","allowed_tools":["search"],"headers":{"Authorization":"Bearer fixture-token-123456"}}]"#;
        assert!(LmStudioRuntimeConfig::parse_with_token_and_evidence(
            None,
            "http://127.0.0.1:1234",
            None,
            Some(valid_rag),
            None,
            Some(&rag_policy),
        )
        .is_ok());
        let wrong_rag_path = r#"[{"type":"ephemeral_mcp","server_label":"rag","server_url":"http://127.0.0.1:9200/mcp","allowed_tools":["search"],"headers":{"Authorization":"Bearer fixture-token-123456"}}]"#;
        assert!(LmStudioRuntimeConfig::parse_with_token_and_evidence(
            None,
            "http://127.0.0.1:1234",
            None,
            Some(wrong_rag_path),
            None,
            Some(&rag_policy),
        )
        .is_err());
        for url in [
            "http://[::1]:9100/mcp",
            "http://127.0.0.1:0/mcp",
            "http://127.0.0.1:9100/mcp/v1",
            "http://127.0.0.1:9100/",
            "http://127.0.0.1:9100/mcp?token=secret",
            "http://127.0.0.1:9100/mcp#fragment",
            "http://user@127.0.0.1:9100/mcp",
        ] {
            let raw = format!(
                r#"[{{"type":"ephemeral_mcp","server_label":"memory","server_url":"{url}","allowed_tools":["search"],"headers":{{"Authorization":"Bearer fixture-token-123456"}}}}]"#
            );
            assert!(
                LmStudioRuntimeConfig::parse_with_token_and_evidence(
                    None,
                    "http://127.0.0.1:1234",
                    None,
                    Some(&raw),
                    None,
                    Some(&policy),
                )
                .is_err(),
                "{url} must be rejected"
            );
        }
        for headers in [
            r#"{}"#,
            r#"{"Authorization":"secret"}"#,
            r#"{"authorization":"Bearer fixture-token-123456"}"#,
            r#"{"Authorization":"Bearer short"}"#,
            r#"{"Authorization":"Bearer fixture-token-123456","X-Unsafe":"value"}"#,
        ] {
            let raw = format!(
                r#"[{{"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100/mcp","allowed_tools":["search"],"headers":{headers}}}]"#
            );
            assert!(
                LmStudioRuntimeConfig::parse_with_token_and_evidence(
                    None,
                    "http://127.0.0.1:1234",
                    None,
                    Some(&raw),
                    None,
                    Some(&policy),
                )
                .is_err(),
                "{headers} must be rejected"
            );
        }
    }

    #[test]
    fn official_mcp_integrations_reject_unsafe_or_ambiguous_shapes() {
        let cases = [
            r#"[{"type":"plugin","plugin_id":"memory","allowed_tools":["search"]}]"#,
            r#"[{"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100","allowed_tools":["search"],"headers":{"Authorization":"secret"}}]"#,
            r#"[{"type":"ephemeral_mcp","server_label":"memory","server_url":"http://localhost:9100","allowed_tools":["search"]}]"#,
            r#"[{"type":"ephemeral_mcp","server_label":"","server_url":"http://127.0.0.1:9100","allowed_tools":["search"]}]"#,
            r#"[{"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100","allowed_tools":[]}]"#,
            r#"[{"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100","allowed_tools":["search","search"]}]"#,
            r#"[
              {"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100","allowed_tools":["search"]},
              {"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9200","allowed_tools":["recall"]}
            ]"#,
            r#"[{"type":"ephemeral_mcp","server_label":"first","server_label":"second","server_url":"http://127.0.0.1:9100/mcp","allowed_tools":["search"]}]"#,
            r#"[{"type":"ephemeral_mcp","type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100/mcp","allowed_tools":["search"]}]"#,
            r#"[{"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100/mcp","server_url":"http://127.0.0.1:9200/mcp","allowed_tools":["search"]}]"#,
            r#"[{"type":"ephemeral_mcp","server_label":"memory","server_url":"http://127.0.0.1:9100/mcp","allowed_tools":["search"],"allowed_tools":["write"]}]"#,
        ];

        for raw in cases {
            assert!(
                LmStudioRuntimeConfig::parse(
                    Some("OFFICIAL"),
                    "http://127.0.0.1:1234",
                    None,
                    Some(raw),
                )
                .is_err(),
                "must reject {raw}"
            );
        }
    }

    #[test]
    fn integration_resource_bounds_accept_maximum_and_reject_max_plus_one() {
        let padded = format!("[]{}", " ".repeat(MAX_INTEGRATIONS_JSON_BYTES - 2));
        assert_eq!(padded.len(), MAX_INTEGRATIONS_JSON_BYTES);
        assert!(parse_integrations(Some(&padded), Classification::Official).is_ok());
        let too_large = format!("{padded} ");
        assert!(parse_integrations(Some(&too_large), Classification::Official).is_err());

        let max_label = "a".repeat(MAX_SERVER_LABEL_BYTES);
        let max_tool = "t".repeat(MAX_TOOL_NAME_BYTES);
        let one = serde_json::json!([{
            "type": "ephemeral_mcp",
            "server_label": max_label,
            "server_url": "http://127.0.0.1:9100/mcp",
            "allowed_tools": [max_tool],
            "headers": {"Authorization": "Bearer fixture-token-123456"},
        }]);
        assert!(parse_integrations(Some(&one.to_string()), Classification::Official).is_ok());

        let too_long_label = "a".repeat(MAX_SERVER_LABEL_BYTES + 1);
        let invalid_label = serde_json::json!([{
            "type": "ephemeral_mcp",
            "server_label": too_long_label,
            "server_url": "http://127.0.0.1:9100/mcp",
            "allowed_tools": ["search"],
            "headers": {"Authorization": "Bearer fixture-token-123456"},
        }]);
        assert!(
            parse_integrations(Some(&invalid_label.to_string()), Classification::Official).is_err()
        );

        let too_long_tool = "t".repeat(MAX_TOOL_NAME_BYTES + 1);
        let invalid_tool = serde_json::json!([{
            "type": "ephemeral_mcp",
            "server_label": "memory",
            "server_url": "http://127.0.0.1:9100/mcp",
            "allowed_tools": [too_long_tool],
            "headers": {"Authorization": "Bearer fixture-token-123456"},
        }]);
        assert!(
            parse_integrations(Some(&invalid_tool.to_string()), Classification::Official).is_err()
        );

        let max_integrations: Vec<_> = (0..MAX_MCP_INTEGRATIONS)
            .map(|index| {
                serde_json::json!({
                    "type": "ephemeral_mcp",
                    "server_label": format!("server_{index}"),
                    "server_url": format!("http://127.0.0.1:{}/mcp", 9100 + index),
                    "allowed_tools": ["search"],
                    "headers": {"Authorization": "Bearer fixture-token-123456"},
                })
            })
            .collect();
        assert!(parse_integrations(
            Some(&serde_json::to_string(&max_integrations).expect("test JSON")),
            Classification::Official
        )
        .is_ok());

        let prefix = "http://127.0.0.1:9100/";
        let too_long_url = format!(
            "{prefix}{}",
            "a".repeat(MAX_MCP_URL_BYTES + 1 - prefix.len())
        );
        let too_long_url_config = serde_json::json!([{
            "type": "ephemeral_mcp",
            "server_label": "memory",
            "server_url": too_long_url,
            "allowed_tools": ["search"],
            "headers": {"Authorization": "Bearer fixture-token-123456"},
        }]);
        assert!(parse_integrations(
            Some(&too_long_url_config.to_string()),
            Classification::Official
        )
        .is_err());
        let too_many: Vec<_> = (0..=MAX_MCP_INTEGRATIONS)
            .map(|index| {
                serde_json::json!({
                    "type": "ephemeral_mcp",
                    "server_label": format!("server_{index}"),
                    "server_url": format!("http://127.0.0.1:{}/mcp", 9100 + index),
                    "allowed_tools": ["search"],
                    "headers": {"Authorization": "Bearer fixture-token-123456"},
                })
            })
            .collect();
        assert!(parse_integrations(
            Some(&serde_json::to_string(&too_many).expect("test JSON")),
            Classification::Official
        )
        .is_err());

        let max_tools: Vec<_> = (0..MAX_TOOLS_PER_INTEGRATION)
            .map(|index| format!("tool_{index}"))
            .collect();
        let max_tool_count = serde_json::json!([{
            "type": "ephemeral_mcp",
            "server_label": "memory",
            "server_url": "http://127.0.0.1:9100/mcp",
            "allowed_tools": max_tools,
            "headers": {"Authorization": "Bearer fixture-token-123456"},
        }]);
        assert!(
            parse_integrations(Some(&max_tool_count.to_string()), Classification::Official).is_ok()
        );
        let too_many_tools: Vec<_> = (0..=MAX_TOOLS_PER_INTEGRATION)
            .map(|index| format!("tool_{index}"))
            .collect();
        let invalid_tool_count = serde_json::json!([{
            "type": "ephemeral_mcp",
            "server_label": "memory",
            "server_url": "http://127.0.0.1:9100/mcp",
            "allowed_tools": too_many_tools,
            "headers": {"Authorization": "Bearer fixture-token-123456"},
        }]);
        assert!(parse_integrations(
            Some(&invalid_tool_count.to_string()),
            Classification::Official
        )
        .is_err());

        let max_total: Vec<_> = (0..4)
            .map(|server| {
                let tools: Vec<_> = (0..64)
                    .map(|tool| format!("tool_{server}_{tool}"))
                    .collect();
                serde_json::json!({
                    "type": "ephemeral_mcp",
                    "server_label": format!("server_{server}"),
                    "server_url": format!("http://127.0.0.1:{}/mcp", 9200 + server),
                    "allowed_tools": tools,
                    "headers": {"Authorization": "Bearer fixture-token-123456"},
                })
            })
            .collect();
        assert_eq!(4 * 64, MAX_TOTAL_ALLOWED_TOOLS);
        assert!(parse_integrations(
            Some(&serde_json::to_string(&max_total).expect("test JSON")),
            Classification::Official
        )
        .is_ok());
        let mut too_many_total = max_total;
        too_many_total.push(serde_json::json!({
            "type": "ephemeral_mcp",
            "server_label": "server_extra",
            "server_url": "http://127.0.0.1:9300/mcp",
            "allowed_tools": ["tool_extra"],
            "headers": {"Authorization": "Bearer fixture-token-123456"},
        }));
        assert!(parse_integrations(
            Some(&serde_json::to_string(&too_many_total).expect("test JSON")),
            Classification::Official
        )
        .is_err());
    }

    #[test]
    fn request_authorization_allows_only_native_paths_on_configured_origin() {
        let cfg = LmStudioRuntimeConfig::parse(None, "http://127.0.0.1:1234", None, None)
            .expect("official config");
        for purpose in [
            NativeRequestPurpose::ModelDiscovery,
            NativeRequestPurpose::Chat,
            NativeRequestPurpose::Continuation,
            NativeRequestPurpose::Summary,
        ] {
            let url = cfg.request_url(purpose).expect("authorized native URL");
            assert!(cfg.authorize_request(purpose, &url).is_ok());
        }

        let wrong_origin =
            reqwest::Url::parse("http://127.0.0.1:4321/api/v1/chat").expect("test URL");
        assert!(cfg
            .authorize_request(NativeRequestPurpose::Chat, &wrong_origin)
            .is_err());
        let wrong_path =
            reqwest::Url::parse("http://127.0.0.1:1234/v1/chat/completions").expect("test URL");
        assert!(cfg
            .authorize_request(NativeRequestPurpose::Chat, &wrong_path)
            .is_err());
    }
}
