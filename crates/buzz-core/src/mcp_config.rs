//! Versioned MCP configuration shared by launchers and the ACP harness.
//!
//! The launcher that writes an ephemeral configuration and the harness that
//! consumes it must use the same wire contract. Keeping the schema here avoids
//! a successful launch producing a document that the harness cannot parse.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Current structured MCP configuration version.
pub const MCP_CONFIG_VERSION: u32 = 1;
/// Maximum encoded size accepted for one structured MCP configuration.
pub const MCP_CONFIG_MAX_BYTES: u64 = 64 * 1024;
/// Maximum number of MCP servers in one launch document.
pub const MCP_SERVER_MAX_COUNT: usize = 16;
/// Maximum number of arguments for one stdio MCP server.
pub const MCP_SERVER_MAX_ARGS: usize = 128;
/// Maximum number of environment entries for one stdio MCP server.
pub const MCP_SERVER_MAX_ENV: usize = 128;
/// Maximum encoded length of an MCP server name.
pub const MCP_SERVER_NAME_MAX_BYTES: usize = 128;

/// Harness identity and authorization variables that an MCP server may not override.
pub const PROTECTED_MCP_ENV_NAMES: [&str; 6] = [
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_API_TOKEN",
    "BUZZ_ACP_PRIVATE_KEY",
    "BUZZ_ACP_API_TOKEN",
];

/// A versioned collection of MCP servers supplied to an agent launch.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpLaunchConfigDocument {
    /// Wire-schema version.
    pub version: u32,
    /// Ordered MCP servers supplied to the ACP session.
    pub servers: Vec<ConfiguredMcpServer>,
}

impl std::fmt::Debug for McpLaunchConfigDocument {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpLaunchConfigDocument")
            .field("version", &self.version)
            .field("servers", &self.servers)
            .finish()
    }
}

impl McpLaunchConfigDocument {
    /// Construct a document using the current wire-schema version.
    pub fn new(servers: Vec<ConfiguredMcpServer>) -> Self {
        Self {
            version: MCP_CONFIG_VERSION,
            servers,
        }
    }

    /// Validate this launch document independently of any compatibility input.
    pub fn validate(&self) -> Result<(), McpConfigError> {
        if self.version != MCP_CONFIG_VERSION {
            return Err(McpConfigError::Invalid(format!(
                "unsupported MCP config version {} (expected {MCP_CONFIG_VERSION})",
                self.version
            )));
        }

        if self.servers.len() > MCP_SERVER_MAX_COUNT {
            return Err(McpConfigError::Invalid(format!(
                "too many MCP servers ({} configured, max {MCP_SERVER_MAX_COUNT})",
                self.servers.len()
            )));
        }

        let mut names = HashSet::with_capacity(self.servers.len());
        for (index, server) in self.servers.iter().enumerate() {
            let name = server.name();
            if !valid_mcp_server_name(name) {
                return Err(McpConfigError::Invalid(format!(
                    "MCP server {} has invalid name '{}': use 1 to {MCP_SERVER_NAME_MAX_BYTES} ASCII letters, digits, underscores, or hyphens, without '__'",
                    index + 1,
                    name
                )));
            }
            if !names.insert(name) {
                return Err(McpConfigError::Invalid(format!(
                    "duplicate MCP server name '{name}'"
                )));
            }
            let ConfiguredMcpServer::Stdio {
                command, args, env, ..
            } = server
            else {
                let ConfiguredMcpServer::Http { url, headers, .. } = server else {
                    continue;
                };
                if url.is_empty() {
                    return Err(McpConfigError::Invalid(format!(
                        "remote MCP server '{name}' requires a resolved URL"
                    )));
                }
                if headers.len() > MCP_SERVER_MAX_ENV {
                    return Err(McpConfigError::Invalid(format!(
                        "remote MCP server '{name}' has too many headers ({}, max {MCP_SERVER_MAX_ENV})",
                        headers.len()
                    )));
                }
                for header in headers {
                    if header.name.trim().is_empty()
                        || header.name.contains(['\r', '\n', '\0'])
                        || header.value.is_empty()
                        || header.value.contains(['\r', '\n', '\0'])
                    {
                        return Err(McpConfigError::Invalid(format!(
                            "remote MCP server '{name}' has an invalid header configuration"
                        )));
                    }
                }
                continue;
            };
            if command.trim().is_empty() || command.contains('\0') {
                return Err(McpConfigError::Invalid(format!(
                    "MCP server '{name}' command must not be blank and must contain no NUL bytes"
                )));
            }
            if args.len() > MCP_SERVER_MAX_ARGS {
                return Err(McpConfigError::Invalid(format!(
                    "MCP server '{name}' has too many arguments ({}, max {MCP_SERVER_MAX_ARGS})",
                    args.len()
                )));
            }
            if args.iter().any(|argument| argument.contains('\0')) {
                return Err(McpConfigError::Invalid(format!(
                    "MCP server '{name}' arguments must contain no NUL bytes"
                )));
            }
            if env.len() > MCP_SERVER_MAX_ENV {
                return Err(McpConfigError::Invalid(format!(
                    "MCP server '{name}' has too many environment entries ({}, max {MCP_SERVER_MAX_ENV})",
                    env.len()
                )));
            }
            let mut normalized_env_names = HashSet::with_capacity(env.len());
            for (env_index, (key, value)) in env.iter().enumerate() {
                if !valid_mcp_env_name(key) {
                    return Err(McpConfigError::Invalid(format!(
                        "MCP server '{name}' environment entry {} has an invalid key",
                        env_index + 1
                    )));
                }
                if !normalized_env_names.insert(key.to_ascii_uppercase()) {
                    return Err(McpConfigError::Invalid(format!(
                        "MCP server '{name}' has environment keys that differ only by ASCII case"
                    )));
                }
                if PROTECTED_MCP_ENV_NAMES
                    .iter()
                    .any(|protected| key.eq_ignore_ascii_case(protected))
                {
                    return Err(McpConfigError::Invalid(format!(
                        "MCP server '{name}' may not configure a protected environment key"
                    )));
                }
                if value.contains('\0') {
                    return Err(McpConfigError::Invalid(format!(
                        "MCP server '{name}' environment entry {} contains a NUL byte",
                        env_index + 1
                    )));
                }
            }
        }
        Ok(())
    }

    /// Validate and encode this document within the wire-size limit.
    pub fn to_json(&self) -> Result<Vec<u8>, McpConfigError> {
        self.validate()?;
        let encoded =
            serde_json::to_vec(self).map_err(|error| McpConfigError::Invalid(error.to_string()))?;
        if encoded.len() as u64 > MCP_CONFIG_MAX_BYTES {
            return Err(McpConfigError::Invalid(format!(
                "MCP config exceeds the {MCP_CONFIG_MAX_BYTES} byte limit"
            )));
        }
        Ok(encoded)
    }
}

/// One MCP server loaded from the structured MCP configuration.
///
/// Version 1 includes both stdio and Streamable HTTP so its meaning is fixed
/// before either transport ships. Additional transports require a new schema
/// version rather than changing an already-published version in place.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfiguredMcpServer {
    /// A local MCP child process connected over stdio.
    Stdio {
        /// Stable ACP identifier for this server.
        name: String,
        /// Executable invoked directly without shell parsing.
        command: String,
        /// Arguments passed to the executable in their configured order.
        args: Vec<String>,
        /// Server-specific environment in deterministic key order.
        #[serde(deserialize_with = "deserialize_mcp_env")]
        env: BTreeMap<String, String>,
    },
    /// A remote Streamable HTTP MCP server.
    Http {
        /// Stable ACP identifier for this server.
        name: String,
        /// Resolved URL supplied by the trusted launch resolver.
        url: String,
        /// Resolved headers supplied by the trusted launch resolver.
        #[serde(default)]
        headers: Vec<McpHttpHeaderConfig>,
    },
}

impl ConfiguredMcpServer {
    /// Stable name supplied to ACP for this server.
    pub fn name(&self) -> &str {
        match self {
            Self::Stdio { name, .. } | Self::Http { name, .. } => name,
        }
    }
}

/// Header attached to requests for a remote HTTP MCP server.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpHttpHeaderConfig {
    /// HTTP header name.
    pub name: String,
    /// Final resolved header value.
    pub value: String,
}

impl std::fmt::Debug for McpHttpHeaderConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpHttpHeaderConfig")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

struct RedactedMcpEnv<'a>(&'a BTreeMap<String, String>);

impl std::fmt::Debug for RedactedMcpEnv<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = formatter.debug_map();
        for key in self.0.keys() {
            map.entry(key, &"[REDACTED]");
        }
        map.finish()
    }
}

impl std::fmt::Debug for ConfiguredMcpServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdio {
                name,
                command,
                args,
                env,
            } => formatter
                .debug_struct("Stdio")
                .field("name", name)
                .field("command", command)
                .field("arg_count", &args.len())
                .field("env", &RedactedMcpEnv(env))
                .finish(),
            Self::Http { name, headers, .. } => formatter
                .debug_struct("Http")
                .field("name", name)
                .field("url", &"[REDACTED]")
                .field("headers", headers)
                .finish(),
        }
    }
}

/// Structured MCP configuration validation failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum McpConfigError {
    /// The document or one of its server entries is invalid.
    #[error("{0}")]
    Invalid(String),
}

/// Parse and validate one structured MCP configuration document.
pub fn parse_mcp_config_document(
    content: &[u8],
) -> Result<Vec<ConfiguredMcpServer>, McpConfigError> {
    if content.len() as u64 > MCP_CONFIG_MAX_BYTES {
        return Err(McpConfigError::Invalid(format!(
            "MCP config exceeds the {MCP_CONFIG_MAX_BYTES} byte limit"
        )));
    }
    let document: McpLaunchConfigDocument = serde_json::from_slice(content)
        .map_err(|error| McpConfigError::Invalid(error.to_string()))?;
    document.validate()?;
    Ok(document.servers)
}

fn valid_mcp_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MCP_SERVER_NAME_MAX_BYTES
        && !name.contains("__")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_mcp_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn deserialize_mcp_env<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct EnvVisitor;

    impl<'de> serde::de::Visitor<'de> for EnvVisitor {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an object containing unique environment variable names")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut env = BTreeMap::new();
            let mut normalized_names = HashSet::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if !normalized_names.insert(key.to_ascii_uppercase()) {
                    return Err(serde::de::Error::custom(
                        "environment keys must be unique ignoring ASCII case",
                    ));
                }
                env.insert(key, value);
            }
            Ok(env)
        }
    }

    deserializer.deserialize_map(EnvVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_server(name: &str) -> ConfiguredMcpServer {
        ConfiguredMcpServer::Stdio {
            name: name.to_string(),
            command: "mcp-server".to_string(),
            args: vec!["--stdio".to_string()],
            env: BTreeMap::from([("TOKEN".to_string(), "secret-value".to_string())]),
        }
    }

    #[test]
    fn serialized_document_carries_the_transport_tag() {
        let document = McpLaunchConfigDocument::new(vec![stdio_server("analytics")]);
        let encoded = document.to_json().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(json["version"], MCP_CONFIG_VERSION);
        assert_eq!(json["servers"][0]["transport"], "stdio");
        assert_eq!(
            parse_mcp_config_document(&encoded).unwrap(),
            vec![stdio_server("analytics")]
        );
    }

    #[test]
    fn writer_rejects_case_insensitive_environment_collisions() {
        let document = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Stdio {
            name: "one".to_string(),
            command: "mcp".to_string(),
            args: Vec::new(),
            env: BTreeMap::from([
                ("Token".to_string(), "first".to_string()),
                ("TOKEN".to_string(), "second".to_string()),
            ]),
        }]);

        assert!(document.to_json().is_err());
    }

    #[test]
    fn environment_validation_errors_do_not_echo_untrusted_keys() {
        let pasted_secret = "pasted-secret-that-must-not-appear";
        let invalid_key = format!("BAD-KEY-{pasted_secret}");
        let document = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Stdio {
            name: "one".to_string(),
            command: "mcp".to_string(),
            args: Vec::new(),
            env: BTreeMap::from([(invalid_key.clone(), "value".to_string())]),
        }]);

        let error = document.validate().unwrap_err().to_string();
        assert!(!error.contains(&invalid_key));
        assert!(!error.contains(pasted_secret));
    }

    #[test]
    fn missing_or_unknown_transport_is_rejected() {
        for content in [
            br#"{"version":1,"servers":[{"name":"one","command":"mcp","args":[],"env":{}}]}"#
                .as_slice(),
            br#"{"version":1,"servers":[{"name":"one","transport":"tcp","url":"https://example.test"}]}"#
                .as_slice(),
        ] {
            assert!(parse_mcp_config_document(content).is_err());
        }
    }

    #[test]
    fn http_transport_round_trips_in_the_shared_document() {
        let document = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Http {
            name: "hosted-context".to_string(),
            url: "https://mcp.example.test/mcp".to_string(),
            headers: vec![McpHttpHeaderConfig {
                name: "Authorization".to_string(),
                value: "Bearer secret".to_string(),
            }],
        }]);

        let encoded = document.to_json().unwrap();
        assert_eq!(
            parse_mcp_config_document(&encoded).unwrap(),
            document.servers
        );
        assert!(!format!("{document:?}").contains("Bearer secret"));
    }

    #[test]
    fn http_transport_requires_resolved_control_free_values() {
        for server in [
            ConfiguredMcpServer::Http {
                name: "missing-url".to_string(),
                url: String::new(),
                headers: Vec::new(),
            },
            ConfiguredMcpServer::Http {
                name: "empty-header".to_string(),
                url: "https://mcp.example.test/mcp".to_string(),
                headers: vec![McpHttpHeaderConfig {
                    name: "Authorization".to_string(),
                    value: String::new(),
                }],
            },
            ConfiguredMcpServer::Http {
                name: "injected-header".to_string(),
                url: "https://mcp.example.test/mcp".to_string(),
                headers: vec![McpHttpHeaderConfig {
                    name: "Authorization".to_string(),
                    value: "Bearer safe\r\nInjected: true".to_string(),
                }],
            },
        ] {
            assert!(McpLaunchConfigDocument::new(vec![server])
                .validate()
                .is_err());
        }
    }

    #[test]
    fn validation_rejects_duplicate_names_and_protected_environment() {
        let duplicate =
            McpLaunchConfigDocument::new(vec![stdio_server("one"), stdio_server("one")]);
        assert!(duplicate.validate().is_err());

        let protected = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Stdio {
            name: "one".to_string(),
            command: "mcp".to_string(),
            args: Vec::new(),
            env: BTreeMap::from([("BUZZ_PRIVATE_KEY".to_string(), "secret".to_string())]),
        }]);
        assert!(protected.validate().is_err());
    }

    #[test]
    fn validation_rejects_blank_commands() {
        let document = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Stdio {
            name: "one".to_string(),
            command: " \t\n ".to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }]);

        assert!(document.validate().is_err());
    }

    #[test]
    fn debug_output_redacts_environment_values() {
        let output = format!(
            "{:?}",
            McpLaunchConfigDocument::new(vec![stdio_server("one")])
        );
        assert!(output.contains("TOKEN"));
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains("secret-value"));
    }

    #[test]
    fn encoding_and_parsing_enforce_the_same_size_limit() {
        let oversized = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Stdio {
            name: "one".to_string(),
            command: "mcp".to_string(),
            args: Vec::new(),
            env: BTreeMap::from([(
                "TOKEN".to_string(),
                "x".repeat(MCP_CONFIG_MAX_BYTES as usize),
            )]),
        }]);

        assert!(oversized.to_json().is_err());
        assert!(parse_mcp_config_document(&vec![b' '; MCP_CONFIG_MAX_BYTES as usize + 1]).is_err());
    }
}
