//! Pack manifest types and `plugin.json` parser.
//!
//! Every persona pack ships a `.plugin/plugin.json` that describes the pack
//! (OPS metadata) and tells Sprout where to find personas, hooks, and MCP
//! config.
//!
//! ```json
//! {
//!   "id": "my-pack",
//!   "name": "My Pack",
//!   "version": "1.0.0",
//!   "personas": ["personas/bot.persona.md"]
//! }
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::persona::RespondTo;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read file: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("missing required field: {0}")]
    MissingField(String),
}

// ── Supporting types ──────────────────────────────────────────────────────────

/// Semver engine constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Engines {
    /// Semver range the Sprout runtime must satisfy (e.g. `">=0.9.0"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sprout: Option<String>,
}

/// Pack-wide behavioral defaults.
///
/// Persona-level values take precedence; these fill in the gaps.
/// Same shape as the persona behavioral config fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BehavioralDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none", alias = "respond_to")]
    pub triggers: Option<RespondTo>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_replies: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_replies: Option<bool>,
}

// ── Core struct ───────────────────────────────────────────────────────────────

/// The pack manifest from `.plugin/plugin.json`.
///
/// OPS required fields (`id`, `name`, `version`) are validated after
/// deserialization because `serde_json` would otherwise surface confusing
/// errors for missing keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PackManifest {
    // ── OPS required ──────────────────────────────────────────────────────
    pub id: String,
    pub name: String,
    pub version: String,

    // ── OPS optional ──────────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    #[serde(default)]
    pub keywords: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub engines: Option<Engines>,

    // ── Sprout extensions ─────────────────────────────────────────────────
    /// Paths to `.persona.md` files (pack-relative).
    #[serde(default)]
    pub personas: Vec<String>,

    /// Path to the pack-level instructions markdown file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack_instructions: Option<String>,

    /// Path to `.mcp.json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<String>,

    /// Path to `hooks/hooks.json`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks_config: Option<String>,

    /// Pack-wide behavioral defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<BehavioralDefaults>,
}

// ── Intermediate for post-parse validation ────────────────────────────────────

/// Mirrors `PackManifest` but with required fields as `Option` so we can
/// produce a clean `MissingField` error instead of a serde path error.
///
/// Intentionally permissive (no `deny_unknown_fields`): `plugin.json` is an
/// OPS superset and may carry fields from other tools (e.g. `ops_category`,
/// `marketplace_tags`). Unknown fields are silently ignored here; the
/// validator issues advisory warnings for Sprout-unknown keys.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct RawManifest {
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    author: Option<String>,
    license: Option<String>,
    homepage: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    engines: Option<Engines>,
    #[serde(default)]
    personas: Vec<String>,
    pack_instructions: Option<String>,
    mcp_config: Option<String>,
    hooks_config: Option<String>,
    defaults: Option<BehavioralDefaults>,
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse a `plugin.json` string into a [`PackManifest`].
pub fn parse_manifest(content: &str) -> Result<PackManifest, ManifestError> {
    let raw: RawManifest = serde_json::from_str(content)?;

    let id = raw.id.ok_or(ManifestError::MissingField("id".into()))?;
    let name = raw.name.ok_or(ManifestError::MissingField("name".into()))?;
    let version = raw
        .version
        .ok_or(ManifestError::MissingField("version".into()))?;

    if id.trim().is_empty() {
        return Err(ManifestError::MissingField("id (empty)".into()));
    }
    if name.trim().is_empty() {
        return Err(ManifestError::MissingField("name (empty)".into()));
    }
    if version.trim().is_empty() {
        return Err(ManifestError::MissingField("version (empty)".into()));
    }

    Ok(PackManifest {
        id,
        name,
        version,
        description: raw.description,
        author: raw.author,
        license: raw.license,
        homepage: raw.homepage,
        keywords: raw.keywords,
        engines: raw.engines,
        personas: raw.personas,
        pack_instructions: raw.pack_instructions,
        mcp_config: raw.mcp_config,
        hooks_config: raw.hooks_config,
        defaults: raw.defaults,
    })
}

/// Parse a `plugin.json` file from disk.
pub fn parse_manifest_file(path: &Path) -> Result<PackManifest, ManifestError> {
    let content = std::fs::read_to_string(path)?;
    parse_manifest(&content)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────

    fn minimal_json() -> &'static str {
        r#"{"id":"my-pack","name":"My Pack","version":"1.0.0","personas":["personas/bot.persona.md"]}"#
    }

    // ── Happy path ────────────────────────────────────────────────────────

    #[test]
    fn parse_minimal_valid() {
        let m = parse_manifest(minimal_json()).unwrap();
        assert_eq!(m.id, "my-pack");
        assert_eq!(m.name, "My Pack");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.personas, vec!["personas/bot.persona.md"]);
        assert!(m.defaults.is_none());
    }

    #[test]
    fn parse_full_manifest() {
        let json = r#"{
            "id": "full-pack",
            "name": "Full Pack",
            "version": "2.3.4",
            "description": "A full-featured pack.",
            "author": "Tyler",
            "license": "MIT",
            "homepage": "https://example.com",
            "keywords": ["ai", "bot"],
            "engines": {"sprout": ">=0.9.0"},
            "personas": ["personas/a.persona.md", "personas/b.persona.md"],
            "pack_instructions": "instructions.md",
            "mcp_config": ".mcp.json",
            "hooks_config": "hooks/hooks.json",
            "defaults": {
                "model": "openai:gpt-4o",
                "temperature": 0.5,
                "thread_replies": true
            }
        }"#;
        let m = parse_manifest(json).unwrap();
        assert_eq!(m.id, "full-pack");
        assert_eq!(m.keywords, vec!["ai", "bot"]);
        assert_eq!(m.engines.unwrap().sprout.as_deref(), Some(">=0.9.0"));
        assert_eq!(m.personas.len(), 2);
        assert_eq!(m.pack_instructions.as_deref(), Some("instructions.md"));
        let d = m.defaults.unwrap();
        assert_eq!(d.model.as_deref(), Some("openai:gpt-4o"));
        assert_eq!(d.temperature, Some(0.5));
        assert_eq!(d.thread_replies, Some(true));
    }

    #[test]
    fn missing_personas_array_defaults_empty() {
        // personas is optional — omitting it yields an empty vec.
        let json = r#"{"id":"p","name":"P","version":"1.0.0"}"#;
        let m = parse_manifest(json).unwrap();
        assert!(m.personas.is_empty());
    }

    #[test]
    fn empty_defaults_block_is_valid() {
        let json = r#"{"id":"p","name":"P","version":"1.0.0","defaults":{}}"#;
        let m = parse_manifest(json).unwrap();
        let d = m.defaults.unwrap();
        assert!(d.model.is_none());
        assert!(d.temperature.is_none());
    }

    #[test]
    fn defaults_with_triggers() {
        let json = r#"{
            "id": "p", "name": "P", "version": "1.0.0",
            "defaults": {
                "triggers": {"mentions": true, "keywords": ["hey"], "all_messages": false}
            }
        }"#;
        let m = parse_manifest(json).unwrap();
        let rt = m.defaults.unwrap().triggers.unwrap();
        assert_eq!(rt.keywords, vec!["hey"]);
    }

    #[test]
    fn defaults_with_legacy_respond_to_alias() {
        let json = r#"{
            "id": "p", "name": "P", "version": "1.0.0",
            "defaults": {
                "respond_to": {"mentions": true, "keywords": ["hey"], "all_messages": false}
            }
        }"#;
        let m = parse_manifest(json).unwrap();
        let rt = m.defaults.unwrap().triggers.unwrap();
        assert_eq!(rt.keywords, vec!["hey"]);
    }

    // ── Missing required fields ───────────────────────────────────────────

    #[test]
    fn missing_id_errors() {
        let json = r#"{"name":"P","version":"1.0.0"}"#;
        let err = parse_manifest(json).unwrap_err();
        assert!(
            matches!(&err, ManifestError::MissingField(f) if f == "id"),
            "got: {err}"
        );
    }

    #[test]
    fn missing_name_errors() {
        let json = r#"{"id":"p","version":"1.0.0"}"#;
        let err = parse_manifest(json).unwrap_err();
        assert!(
            matches!(&err, ManifestError::MissingField(f) if f == "name"),
            "got: {err}"
        );
    }

    #[test]
    fn missing_version_errors() {
        let json = r#"{"id":"p","name":"P"}"#;
        let err = parse_manifest(json).unwrap_err();
        assert!(
            matches!(&err, ManifestError::MissingField(f) if f == "version"),
            "got: {err}"
        );
    }

    // ── Empty required fields ─────────────────────────────────────────────

    #[test]
    fn empty_id_errors() {
        let json = r#"{"id":"","name":"P","version":"1.0.0"}"#;
        let err = parse_manifest(json).unwrap_err();
        assert!(
            matches!(&err, ManifestError::MissingField(f) if f.contains("id")),
            "got: {err}"
        );
    }

    #[test]
    fn whitespace_id_errors() {
        let json = r#"{"id":"   ","name":"P","version":"1.0.0"}"#;
        let err = parse_manifest(json).unwrap_err();
        assert!(
            matches!(&err, ManifestError::MissingField(f) if f.contains("id")),
            "got: {err}"
        );
    }

    #[test]
    fn empty_name_errors() {
        let json = r#"{"id":"p","name":"","version":"1.0.0"}"#;
        let err = parse_manifest(json).unwrap_err();
        assert!(
            matches!(&err, ManifestError::MissingField(f) if f.contains("name")),
            "got: {err}"
        );
    }

    #[test]
    fn empty_version_errors() {
        let json = r#"{"id":"p","name":"P","version":""}"#;
        let err = parse_manifest(json).unwrap_err();
        assert!(
            matches!(&err, ManifestError::MissingField(f) if f.contains("version")),
            "got: {err}"
        );
    }

    // ── Malformed JSON ────────────────────────────────────────────────────

    #[test]
    fn malformed_json_errors() {
        let err = parse_manifest("{not valid json}").unwrap_err();
        assert!(matches!(err, ManifestError::Json(_)));
    }
}
