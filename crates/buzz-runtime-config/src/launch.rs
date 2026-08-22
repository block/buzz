//! Mapper from the unified native model to the versioned stdio MCP launch
//! document consumed by agent launchers.
//!
//! [`RuntimeMcpConfig`] is the runtime-agnostic view read from an external
//! framework's native config (Hermes YAML / Kimi Code JSON). The launcher-side
//! contract — the wire document that #5349 browser_harness consumes — is a
//! separate, stricter, versioned shape. This module translates the former into
//! the latter.
//!
//! The target schema mirrors buzz-core's `mcp_config` v1 launch document
//! (`transport: "stdio"`), and mirrors its validation — including every one of
//! #5349's hard limits — so a mapping that cannot be represented as a valid
//! launch document is rejected loudly. Validation runs at **construction**
//! ([`McpLaunchConfigDocument::from_runtime_config`]) rather than only at
//! encode time, so an invalid document cannot even be instantiated, let alone
//! serialized out through any `Serialize` path. An enabled native server whose
//! name or environment fails the stricter launch-schema rules surfaces as an
//! error naming the server, never silently dropped.
use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::model::{McpServerConfig, RuntimeKind, RuntimeMcpConfig};

/// Wire-schema version of the launch document we emit (buzz-core v1).
pub const MCP_LAUNCH_DOC_VERSION: u32 = 1;
/// Transport tag of the emitted launch entries (v1 supports stdio only).
pub const STDIO_TRANSPORT: &str = "stdio";

// --- Limits mirrored from buzz-core `mcp_config` v1 (#5349) -----------------
// These are kept in sync with the wire schema so that any document this crate
// can build is, by construction, accepted by the launcher-side parser.

/// Maximum number of MCP servers in one launch document.
pub const MCP_SERVER_MAX_COUNT: usize = 16;
/// Maximum number of arguments for one stdio MCP server.
pub const MCP_SERVER_MAX_ARGS: usize = 128;
/// Maximum number of environment entries for one stdio MCP server.
pub const MCP_SERVER_MAX_ENV: usize = 128;
/// Maximum encoded size accepted for one structured MCP configuration.
pub const MCP_CONFIG_MAX_BYTES: u64 = 64 * 1024;

/// Harness identity/authorization variables the launch schema forbids a server
/// from configuring. Kept in sync with buzz-core's `PROTECTED_MCP_ENV_NAMES`.
pub const PROTECTED_MCP_ENV_NAMES: [&str; 6] = [
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_API_TOKEN",
    "BUZZ_ACP_PRIVATE_KEY",
    "BUZZ_ACP_API_TOKEN",
];

/// One native server mapped into launch-document form.
///
/// Field order and `serde` tagging match the versioned stdio MCP launch
/// document so the emitted JSON is directly consumable by a parse.
///
/// Note the manual [`Debug`] impl: it prints only the server name and the
/// argument / environment *counts*. Arguments and environment values may
/// contain secrets, so they are never echoed in debug output.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct LaunchStdioServer {
    /// `transport` tag from the launch schema (always `"stdio"`).
    #[serde(rename = "transport")]
    transport: &'static str,
    /// Stable server name.
    name: String,
    /// Executable invoked directly without shell parsing.
    command: String,
    /// Arguments passed to `command` in configured order.
    args: Vec<String>,
    /// Server-specific environment in deterministic key order.
    env: BTreeMap<String, String>,
}

impl std::fmt::Debug for LaunchStdioServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LaunchStdioServer")
            .field("name", &self.name)
            .field("arg_count", &self.args.len())
            .field("env_count", &self.env.len())
            .finish()
    }
}

/// A versioned collection of stdio MCP servers ready for agent launch.
///
/// Validation runs at construction (`from_runtime_config`), so a valid
/// [`McpLaunchConfigDocument`] is guaranteed to satisfy every launch-schema
/// limit and to encode without leaking secrets through `Debug`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpLaunchConfigDocument {
    /// Wire-schema version.
    version: u32,
    /// Ordered stdio servers in launch order.
    servers: Vec<LaunchStdioServer>,
}

impl McpLaunchConfigDocument {
    /// Return the mapped launch entries in launch order.
    pub fn servers(&self) -> &[LaunchStdioServer] {
        &self.servers
    }

    /// Map every *enabled* native server into a versioned stdio launch
    /// document, validating each against the launch schema.
    ///
    /// Disabled servers (`enabled: Some(false)`) are excluded. Servers whose
    /// native entry does not specify `enabled` (treat as enabled) are included.
    /// Field values map 1:1 from the native entry into the launch entry.
    ///
    /// Returns `Err` — rather than building a document — the moment any
    /// mapped server would violate a launch-schema rule or one of #5349's hard
    /// limits (server count, argument count, environment count, or the 64 KiB
    /// wire-size bound). This makes an invalid document un-constructible, so it
    /// cannot be serialized out through any `Serialize` path.
    pub fn from_runtime_config(config: &RuntimeMcpConfig) -> Result<Self, LaunchMapError> {
        let servers = config
            .servers
            .iter()
            .filter(|server| server.is_enabled())
            .map(native_to_launch)
            .collect();
        let document = Self {
            version: MCP_LAUNCH_DOC_VERSION,
            servers,
        };
        document.validate()?;
        Ok(document)
    }

    /// Encode this document as JSON after validating it against the launch
    /// schema (mirrors `buzz-core::mcp_config::McpLaunchConfigDocument`).
    pub fn to_json(&self) -> Result<Vec<u8>, LaunchMapError> {
        self.validate()?;
        let encoded =
            serde_json::to_vec(self).map_err(|err| LaunchMapError::Encode(err.to_string()))?;
        if encoded.len() as u64 > MCP_CONFIG_MAX_BYTES {
            return Err(LaunchMapError::Invalid(format!(
                "MCP config exceeds the {MCP_CONFIG_MAX_BYTES} byte limit"
            )));
        }
        Ok(encoded)
    }

    /// Validate the mapped document against the launch-schema rules. Returns
    /// an error naming the first offending server; a document that validates
    /// here is guaranteed to be accepted by the launch-schema parser.
    ///
    /// This mirrors `buzz-core::mcp_config` v1 validation exactly, including
    /// its hard limits. Called from `from_runtime_config` so construction is
    /// the gate; kept as a public method so `to_json` (and callers that have
    /// not gone through `from_runtime_config`) get the same guarantees.
    pub fn validate(&self) -> Result<(), LaunchMapError> {
        if self.version != MCP_LAUNCH_DOC_VERSION {
            return Err(LaunchMapError::Invalid(format!(
                "unsupported MCP config version {} (expected {MCP_LAUNCH_DOC_VERSION})",
                self.version
            )));
        }
        if self.servers.len() > MCP_SERVER_MAX_COUNT {
            return Err(LaunchMapError::Invalid(format!(
                "too many MCP servers ({} configured, max {MCP_SERVER_MAX_COUNT})",
                self.servers.len()
            )));
        }
        let mut raw_string_bytes = 0_u64;
        let mut names = HashSet::with_capacity(self.servers.len());
        for server in &self.servers {
            add_string_bytes(&mut raw_string_bytes, &server.name)?;
            add_string_bytes(&mut raw_string_bytes, &server.command)?;

            if !valid_server_name(&server.name) {
                return Err(LaunchMapError::Invalid(format!(
                    "MCP server '{}' has invalid name: use 1..=128 ASCII letters, digits, underscores, or hyphens, without '__'",
                    server.name
                )));
            }
            if !names.insert(server.name.as_str()) {
                return Err(LaunchMapError::Invalid(format!(
                    "duplicate MCP server name '{}'",
                    server.name
                )));
            }
            if server.command.trim().is_empty() || server.command.contains('\0') {
                return Err(LaunchMapError::Invalid(format!(
                    "MCP server '{}' command must not be blank and must contain no NUL bytes",
                    server.name
                )));
            }
            if server.args.len() > MCP_SERVER_MAX_ARGS {
                return Err(LaunchMapError::Invalid(format!(
                    "MCP server '{}' has too many arguments ({}, max {MCP_SERVER_MAX_ARGS})",
                    server.name,
                    server.args.len()
                )));
            }
            if server.args.iter().any(|argument| argument.contains('\0')) {
                return Err(LaunchMapError::Invalid(format!(
                    "MCP server '{}' arguments must contain no NUL bytes",
                    server.name
                )));
            }
            for argument in &server.args {
                add_string_bytes(&mut raw_string_bytes, argument)?;
            }
            if server.env.len() > MCP_SERVER_MAX_ENV {
                return Err(LaunchMapError::Invalid(format!(
                    "MCP server '{}' has too many environment entries ({}, max {MCP_SERVER_MAX_ENV})",
                    server.name,
                    server.env.len()
                )));
            }
            let mut env_names = HashSet::with_capacity(server.env.len());
            // Report env issues by 1-based index, never by echoing the key or
            // value — untrusted names and values may themselves be secrets.
            for (env_index, (key, value)) in server.env.iter().enumerate() {
                add_string_bytes(&mut raw_string_bytes, key)?;
                add_string_bytes(&mut raw_string_bytes, value)?;
                if !valid_env_name(key) {
                    return Err(LaunchMapError::Invalid(format!(
                        "MCP server '{}' environment entry {} has an invalid key (must start with an ASCII letter or '_' and contain only letters, digits, '_')",
                        server.name,
                        env_index + 1
                    )));
                }
                if !env_names.insert(key.to_ascii_uppercase()) {
                    return Err(LaunchMapError::Invalid(format!(
                        "MCP server '{}' has environment keys that differ only by ASCII case",
                        server.name
                    )));
                }
                if PROTECTED_MCP_ENV_NAMES
                    .iter()
                    .any(|protected| key.eq_ignore_ascii_case(protected))
                {
                    return Err(LaunchMapError::Invalid(format!(
                        "MCP server '{}' may not configure a protected environment key",
                        server.name
                    )));
                }
                if value.contains('\0') {
                    return Err(LaunchMapError::Invalid(format!(
                        "MCP server '{}' environment entry {} contains a NUL byte",
                        server.name,
                        env_index + 1
                    )));
                }
            }
        }

        // Enforce the 64 KiB *encoded* size too, not just the raw string byte
        // sum above. Raw bytes are a necessary but not sufficient bound: JSON
        // escaping (e.g. a newline -> `\n`, a quote -> `\"`) only ever expands
        // a document, so a doc whose raw strings are under the limit can still
        // encode to more than the wire bound. Because construction (and this
        // `validate`) is the only gate a `McpLaunchConfigDocument` can pass
        // through, checking the encoded size here closes the `Serialize` path
        // where a caller could otherwise `serde_json::to_vec` a constructible
        // document straight out, bypassing `to_json`'s size check.
        let encoded =
            serde_json::to_vec(self).map_err(|err| LaunchMapError::Encode(err.to_string()))?;
        if encoded.len() as u64 > MCP_CONFIG_MAX_BYTES {
            return Err(LaunchMapError::Invalid(format!(
                "MCP config exceeds the {MCP_CONFIG_MAX_BYTES} byte limit after JSON encoding"
            )));
        }

        Ok(())
    }
}

/// Map one native server into launch-document form. Field values are copied
/// verbatim — this mapper performs a structural translation, not a value
/// rewrite.
fn native_to_launch(server: &McpServerConfig) -> LaunchStdioServer {
    LaunchStdioServer {
        transport: STDIO_TRANSPORT,
        name: server.name.clone(),
        command: server.command.clone(),
        args: server.args.clone(),
        env: server.env.clone(),
    }
}

/// Mirror of the launch-schema's `valid_mcp_server_name`.
fn valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains("__")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Mirror of the launch-schema's `valid_mcp_env_name`.
fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Accumulate `value`'s byte length toward the 64 KiB in-memory bound, failing
/// as soon as the running total exceeds it. Mirrors buzz-core's
/// `add_string_bytes`, bounding documents *before* encoding so an oversized
/// document can never be built.
fn add_string_bytes(total: &mut u64, value: &str) -> Result<(), LaunchMapError> {
    let length = u64::try_from(value.len())
        .map_err(|_| LaunchMapError::Invalid("MCP config is too large".to_string()))?;
    *total = total
        .checked_add(length)
        .ok_or_else(|| LaunchMapError::Invalid("MCP config is too large".to_string()))?;
    if *total > MCP_CONFIG_MAX_BYTES {
        return Err(LaunchMapError::Invalid(format!(
            "MCP config exceeds the {MCP_CONFIG_MAX_BYTES} byte limit"
        )));
    }
    Ok(())
}

/// A mapping-time failure: either the document cannot be encoded or a mapped
/// server would not be accepted by the launch schema.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchMapError {
    /// The document violates the launch schema.
    #[error("{0}")]
    Invalid(String),
    /// The document is valid but failed to serialize.
    #[error("failed to encode MCP launch document: {0}")]
    Encode(String),
}

/// Convenience: map a native runtime config straight to encoded launch JSON.
///
/// Returns the encoded document bytes on success. On failure, names the
/// offending server.
pub fn to_launch_json(config: &RuntimeMcpConfig) -> Result<Vec<u8>, LaunchMapError> {
    let document = McpLaunchConfigDocument::from_runtime_config(config)?;
    document.to_json()
}

/// Human-readable runtime label for diagnostics.
pub fn runtime_label(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Hermes => "Hermes",
        RuntimeKind::KimiCode => "Kimi Code",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(name: &str, command: &str, enabled: Option<bool>) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            command: command.to_string(),
            args: vec![],
            env: BTreeMap::new(),
            enabled,
        }
    }

    fn config(servers: Vec<McpServerConfig>) -> RuntimeMcpConfig {
        RuntimeMcpConfig {
            runtime: RuntimeKind::Hermes,
            servers,
        }
    }

    #[test]
    fn maps_enabled_servers_and_skips_disabled() {
        let cfg = config(vec![
            native("on", "npx", Some(true)),
            native("off", "npx", Some(false)),
            native("unset", "uv", None),
        ]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap();
        let names: Vec<_> = doc.servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["on", "unset"]);
        assert_eq!(doc.version, MCP_LAUNCH_DOC_VERSION);
    }

    #[test]
    fn wire_carries_stdio_transport_tag() {
        let cfg = config(vec![native("sciverse", "npx", None)]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap();
        let json = serde_json::to_value(doc).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["servers"][0]["transport"], "stdio");
        assert_eq!(json["servers"][0]["name"], "sciverse");
        assert_eq!(json["servers"][0]["command"], "npx");
    }

    #[test]
    fn maps_env_verbatim() {
        let mut server = native("sciverse", "npx", None);
        server
            .args
            .extend(["-y".to_string(), "sciverse-mcp-server".to_string()]);
        server
            .env
            .insert("SCIVERSE_API_TOKEN".to_string(), "token-value".to_string());
        let cfg = config(vec![server]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap();
        let json = serde_json::to_value(doc).unwrap();
        assert_eq!(json["servers"][0]["args"][0], "-y");
        assert_eq!(
            json["servers"][0]["env"]["SCIVERSE_API_TOKEN"],
            "token-value"
        );
    }

    #[test]
    fn rejects_duplicate_names() {
        let cfg = config(vec![native("dup", "npx", None), native("dup", "uv", None)]);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("duplicate MCP server name"));
    }

    #[test]
    fn rejects_protected_env_key() {
        let mut server = native("one", "npx", None);
        server
            .env
            .insert("BUZZ_PRIVATE_KEY".to_string(), "secret".to_string());
        let cfg = config(vec![server]);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("protected environment key"));
    }

    #[test]
    fn rejects_env_keys_differing_only_by_case() {
        let mut server = native("one", "npx", None);
        server.env.insert("Token".to_string(), "a".to_string());
        server.env.insert("TOKEN".to_string(), "b".to_string());
        let cfg = config(vec![server]);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("differ only by ASCII case"));
    }

    #[test]
    fn rejects_invalid_env_key_without_echoing_it() {
        let mut server = native("one", "npx", None);
        let pasted_secret = "pasted-secret-that-must-not-appear";
        let bad_key = format!("BAD-KEY-{pasted_secret}");
        server.env.insert(bad_key.clone(), "value".to_string());
        let cfg = config(vec![server]);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(!err.to_string().contains(&bad_key));
        assert!(!err.to_string().contains(pasted_secret));
        assert!(err.to_string().contains("invalid key"));
    }

    #[test]
    fn rejects_too_many_servers() {
        let servers = (0..=MCP_SERVER_MAX_COUNT)
            .map(|i| native(&format!("srv{i:02}"), "npx", None))
            .collect();
        let cfg = config(servers);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("too many MCP servers"));
    }

    #[test]
    fn rejects_too_many_args() {
        let mut server = native("one", "npx", None);
        server.args = vec!["arg".to_string(); MCP_SERVER_MAX_ARGS + 1];
        let cfg = config(vec![server]);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("too many arguments"));
    }

    #[test]
    fn rejects_too_many_env_entries() {
        let mut server = native("one", "npx", None);
        for i in 0..=MCP_SERVER_MAX_ENV {
            server.env.insert(format!("KEY_{i}"), format!("v{i}"));
        }
        let cfg = config(vec![server]);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("too many environment entries"));
    }

    #[test]
    fn rejects_oversized_document_before_encoding() {
        // A single oversized argument must fail construction, proving the
        // 64 KiB in-memory bound is enforced inline rather than after encode.
        let mut server = native("one", "npx", None);
        server.args = vec!["x".repeat(MCP_CONFIG_MAX_BYTES as usize + 1)];
        let cfg = config(vec![server]);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("byte limit"));
    }

    #[test]
    fn rejects_escape_expansion_past_encoded_limit() {
        // Regression for the serialization bypass wolfyy970 flagged: raw string
        // bytes are bounded at construction, but JSON escaping only ever expands
        // a document. A raw value below the 64 KiB raw limit (e.g. a long string
        // of newline characters) encodes to more than the wire bound because each
        // newline becomes the two bytes `\n`. Construction must reject it — not
        // just `to_json()` — so a caller cannot `serde_json::to_vec` a
        // constructible-but-over-limit document directly.
        let mut server = native("one", "npx", None);
        // 40_000 raw newline bytes: comfortably under 64 KiB raw, but JSON
        // escaping turns each into `\n` (2 bytes), pushing the encoded
        // document past the 64 KiB limit.
        server.args = vec!["\n".repeat(40_000)];
        let cfg = config(vec![server]);
        let err = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap_err();
        assert!(err.to_string().contains("byte limit"));
    }

    #[test]
    fn accepts_large_valid_document() {
        // A large but legitimate document — many arguments within the 128-count
        // bound, totalling well under the 64 KiB wire-size limit — must build
        // and encode cleanly (no false rejections).
        let mut server = native("srv00", "npx", None);
        server.args = vec!["a".repeat(300); MCP_SERVER_MAX_ARGS];
        let cfg = config(vec![server]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap();
        assert!(doc.to_json().is_ok());
    }

    #[test]
    fn debug_output_redacts_args_and_env() {
        let mut server = native("one", "npx", None);
        server.args = vec![
            "--api-key".to_string(),
            "hunter2-secret-arg-9999".to_string(),
        ];
        server.env.insert(
            "SCIVERSE_API_TOKEN".to_string(),
            "env-token-654321".to_string(),
        );
        let cfg = config(vec![server]);
        let doc = McpLaunchConfigDocument::from_runtime_config(&cfg).unwrap();

        let output = format!("{doc:?}");
        assert!(output.contains("one"));
        assert!(output.contains("arg_count: 2"));
        assert!(output.contains("env_count: 1"));
        // Neither the secret value in args nor the secret value in env may
        // leak through debug output.
        assert!(!output.contains("hunter2-secret-arg-9999"));
        assert!(!output.contains("env-token-654321"));
        assert!(!output.contains("--api-key"));
        assert!(!output.contains("SCIVERSE_API_TOKEN"));
    }

    #[test]
    fn roundtrip_real_sciverse_entry_validates() {
        // The real Hermes `sciverse` entry (redacted token) must map cleanly.
        let mut server = native("sciverse", "npx", Some(true));
        server
            .args
            .extend(["-y".to_string(), "sciverse-mcp-server".to_string()]);
        server
            .env
            .insert("SCIVERSE_API_TOKEN".to_string(), "sci_redacted".to_string());
        let cfg = config(vec![server]);
        let bytes = to_launch_json(&cfg).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed["servers"][0]["name"], "sciverse");
        assert_eq!(
            parsed["servers"][0]["env"]["SCIVERSE_API_TOKEN"],
            "sci_redacted"
        );
    }
}
