//! Versioned MCP configuration shared by launchers and the ACP harness.
//!
//! The launcher that writes an ephemeral configuration and the harness that
//! consumes it must use the same wire contract. Keeping the schema here avoids
//! a successful launch producing a document that the harness cannot parse.
//!
//! Version 1 supports stdio servers only. A consumer must not pass the agent
//! process environment through to an MCP child. It starts from an empty
//! environment, restores only the target's minimal non-secret process
//! essentials, and then applies the server environment declared here. This
//! prevents agent identity and authorization values from leaking into tools.

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
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct McpLaunchConfigDocument {
    /// Wire-schema version.
    version: u32,
    /// Ordered MCP servers supplied to the ACP session.
    servers: Vec<ConfiguredMcpServer>,
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
    pub fn new(servers: Vec<ConfiguredMcpServer>) -> Result<Self, McpConfigError> {
        let document = Self {
            version: MCP_CONFIG_VERSION,
            servers,
        };
        document.validate()?;
        Ok(document)
    }

    /// Return the document's wire-schema version.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Return the validated servers in launch order.
    pub fn servers(&self) -> &[ConfiguredMcpServer] {
        &self.servers
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

        let mut raw_string_bytes = 0_u64;
        let mut names = HashSet::with_capacity(self.servers.len());
        for (index, server) in self.servers.iter().enumerate() {
            let ConfiguredMcpServer::Stdio {
                name,
                command,
                args,
                env,
            } = server;
            add_string_bytes(&mut raw_string_bytes, name)?;
            add_string_bytes(&mut raw_string_bytes, command)?;
            for argument in args {
                add_string_bytes(&mut raw_string_bytes, argument)?;
            }
            for (key, value) in env {
                add_string_bytes(&mut raw_string_bytes, key)?;
                add_string_bytes(&mut raw_string_bytes, value)?;
            }
            if !valid_mcp_server_name(name) {
                return Err(McpConfigError::Invalid(format!(
                    "MCP server {} has invalid name '{}': use 1 to {MCP_SERVER_NAME_MAX_BYTES} ASCII letters, digits, underscores, or hyphens, without '__'",
                    index + 1,
                    name
                )));
            }
            if !names.insert(name.as_str()) {
                return Err(McpConfigError::Invalid(format!(
                    "duplicate MCP server name '{name}'"
                )));
            }
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
        let encoded = serde_json::to_vec(self)
            .map_err(|_| McpConfigError::Invalid("failed to encode MCP config".to_string()))?;
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
/// The transport tag is part of version 1, which supports only stdio. A new
/// transport requires a new schema version so an older reader cannot mistake
/// a document containing unsupported launch behavior for version 1.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum ConfiguredMcpServer {
    /// A local MCP child process connected over stdio.
    Stdio {
        /// Stable ACP identifier for this server.
        name: String,
        /// Executable invoked directly without shell parsing.
        command: String,
        /// Arguments passed to the executable in their configured order.
        args: Vec<String>,
        /// Complete server-specific environment in deterministic key order.
        ///
        /// Consumers apply these entries only after clearing the ambient
        /// process environment and restoring minimal non-secret target values.
        env: BTreeMap<String, String>,
    },
}

impl std::fmt::Debug for ConfiguredMcpServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdio {
                name, args, env, ..
            } => formatter
                .debug_struct("Stdio")
                .field("name", name)
                .field("arg_count", &args.len())
                .field("env_count", &env.len())
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
    #[derive(Deserialize)]
    struct VersionHeader {
        version: u32,
    }

    let header: VersionHeader = serde_json::from_slice(content).map_err(sanitized_json_error)?;
    if header.version != MCP_CONFIG_VERSION {
        return Err(McpConfigError::Invalid(format!(
            "unsupported MCP config version {} (expected {MCP_CONFIG_VERSION})",
            header.version
        )));
    }

    let wire: WireMcpLaunchConfigDocument =
        serde_json::from_slice(content).map_err(sanitized_json_error)?;
    let document = McpLaunchConfigDocument {
        version: wire.version,
        servers: wire.servers.into_iter().map(Into::into).collect(),
    };
    document.validate()?;
    Ok(document.servers)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireMcpLaunchConfigDocument {
    version: u32,
    servers: Vec<WireConfiguredMcpServer>,
}

#[derive(Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
enum WireConfiguredMcpServer {
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        #[serde(deserialize_with = "deserialize_mcp_env")]
        env: BTreeMap<String, String>,
    },
}

impl From<WireConfiguredMcpServer> for ConfiguredMcpServer {
    fn from(server: WireConfiguredMcpServer) -> Self {
        match server {
            WireConfiguredMcpServer::Stdio {
                name,
                command,
                args,
                env,
            } => Self::Stdio {
                name,
                command,
                args,
                env,
            },
        }
    }
}

fn sanitized_json_error(error: serde_json::Error) -> McpConfigError {
    McpConfigError::Invalid(format!(
        "invalid MCP config JSON at line {}, column {}",
        error.line(),
        error.column()
    ))
}

fn add_string_bytes(total: &mut u64, value: &str) -> Result<(), McpConfigError> {
    let length = u64::try_from(value.len())
        .map_err(|_| McpConfigError::Invalid("MCP config is too large".to_string()))?;
    *total = total
        .checked_add(length)
        .ok_or_else(|| McpConfigError::Invalid("MCP config is too large".to_string()))?;
    if *total > MCP_CONFIG_MAX_BYTES {
        return Err(McpConfigError::Invalid(format!(
            "MCP config exceeds the {MCP_CONFIG_MAX_BYTES} byte limit"
        )));
    }
    Ok(())
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
        let document = McpLaunchConfigDocument::new(vec![stdio_server("analytics")]).unwrap();
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

        assert!(document.is_err());
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

        let error = document.unwrap_err().to_string();
        assert!(!error.contains(&invalid_key));
        assert!(!error.contains(pasted_secret));
    }

    #[test]
    fn missing_or_unknown_transport_is_rejected() {
        for content in [
            br#"{"version":1,"servers":[{"name":"one","command":"mcp","args":[],"env":{}}]}"#
                .as_slice(),
            br#"{"version":1,"servers":[{"name":"one","transport":"http","url":"https://example.test"}]}"#
                .as_slice(),
        ] {
            assert!(parse_mcp_config_document(content).is_err());
        }
    }

    #[test]
    fn newer_versions_are_reported_before_their_unknown_shape() {
        let error = parse_mcp_config_document(
            br#"{"version":2,"servers":[{"transport":"future-secret-transport"}]}"#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("unsupported MCP config version 2"));
        assert!(!error.contains("future-secret-transport"));
    }

    #[test]
    fn parse_errors_do_not_echo_untrusted_values() {
        let error = parse_mcp_config_document(
            br#"{"version":1,"servers":"pasted-secret-that-must-not-appear"}"#,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("invalid MCP config JSON"));
        assert!(!error.contains("pasted-secret-that-must-not-appear"));
    }

    #[test]
    fn validation_rejects_duplicate_names_and_protected_environment() {
        let duplicate =
            McpLaunchConfigDocument::new(vec![stdio_server("one"), stdio_server("one")]);
        assert!(duplicate.is_err());

        let protected = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Stdio {
            name: "one".to_string(),
            command: "mcp".to_string(),
            args: Vec::new(),
            env: BTreeMap::from([("BUZZ_PRIVATE_KEY".to_string(), "secret".to_string())]),
        }]);
        assert!(protected.is_err());
    }

    #[test]
    fn validation_rejects_blank_commands() {
        let document = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Stdio {
            name: "one".to_string(),
            command: " \t\n ".to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }]);

        assert!(document.is_err());
    }

    #[test]
    fn debug_output_redacts_environment_values() {
        let output = format!(
            "{:?}",
            McpLaunchConfigDocument::new(vec![stdio_server("one")]).unwrap()
        );
        assert!(output.contains("arg_count: 1"));
        assert!(output.contains("env_count: 1"));
        assert!(!output.contains("TOKEN"));
        assert!(!output.contains("secret-value"));
        assert!(!output.contains("mcp-server"));
        assert!(!output.contains("--stdio"));
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

        assert!(oversized.is_err());
        assert!(parse_mcp_config_document(&vec![b' '; MCP_CONFIG_MAX_BYTES as usize + 1]).is_err());
    }

    #[test]
    fn in_memory_documents_are_bounded_before_encoding() {
        let oversized = McpLaunchConfigDocument::new(vec![ConfiguredMcpServer::Stdio {
            name: "one".to_string(),
            command: "x".repeat(MCP_CONFIG_MAX_BYTES as usize + 1),
            args: Vec::new(),
            env: BTreeMap::new(),
        }]);

        assert!(oversized.is_err());
    }
}
