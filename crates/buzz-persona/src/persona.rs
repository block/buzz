//! Core persona types and `.persona.md` parser.
//!
//! A `.persona.md` file is YAML frontmatter (between `---` delimiters)
//! followed by a markdown body that becomes the system prompt.
//!
//! ```text
//! ---
//! name: my-bot
//! display_name: My Bot
//! description: Does things.
//! ---
//! You are My Bot. You do things.
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ── Safety limits ─────────────────────────────────────────────────────────────

/// Maximum YAML frontmatter size in bytes (1 MiB).
pub const MAX_FRONTMATTER_BYTES: usize = 1_048_576;

/// Maximum persona prompt (markdown body) size in bytes (256 KiB).
pub const MAX_BODY_BYTES: usize = 262_144;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PersonaError {
    #[error("failed to read file: {0}")]
    Io(#[from] std::io::Error),

    #[error("missing `---` frontmatter delimiters")]
    NoFrontmatter,

    #[error("frontmatter exceeds {MAX_FRONTMATTER_BYTES} bytes")]
    FrontmatterTooLarge,

    #[error("body exceeds {MAX_BODY_BYTES} bytes")]
    BodyTooLarge,

    #[error("file too large: {0}")]
    TooLarge(String),

    #[error("failed to parse YAML frontmatter: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("missing required field: {0}")]
    MissingField(String),
}

// ── Supporting types ──────────────────────────────────────────────────────────

/// Controls which messages trigger a response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RespondTo {
    /// Respond when mentioned. Default: true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentions: Option<bool>,

    /// Respond when any of these keywords appear.
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Respond to every message in subscribed channels. Default: false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_messages: Option<bool>,
}

/// A single MCP server attached to this persona.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Lifecycle hooks (paths are pack-relative).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Hooks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_start: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_stop: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_message: Option<String>,
}

// ── Core struct ───────────────────────────────────────────────────────────────

/// Typed representation of a `.persona.md` file (V7 spec).
///
/// The `prompt` field holds the markdown body (system prompt).
/// All other fields come from the YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PersonaConfig {
    // ── Identity ──────────────────────────────────────────────────────────
    /// Machine name (slug). Required.
    pub name: String,

    /// Human-readable display name. Required.
    pub display_name: String,

    /// Pack-relative path to avatar image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,

    /// One-line description. Required.
    pub description: String,

    // ── OPS compatibility ─────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,

    // ── Skills & MCP ──────────────────────────────────────────────────────
    /// Pack-relative paths to skill directories.
    #[serde(default)]
    pub skills: Vec<String>,

    /// Per-persona MCP server definitions.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,

    // ── Behavioral config ─────────────────────────────────────────────────
    /// Channel names to monitor.
    ///
    /// - `None` (omitted or `null`) → fall through to pack default
    /// - `Some(vec![])` → intentional "subscribe to nothing"
    /// - `Some(vec!["#general"])` → explicit channels
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<Vec<String>>,

    /// Message matching triggers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggers: Option<RespondTo>,

    /// Model string in `"provider:model-id"` format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Preferred ACP runtime ID (e.g., 'goose', 'claude'). Maps to PersonaRecord.runtime during
    /// pack import.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_context_tokens: Option<u64>,

    /// Reply in-thread. Default: true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_replies: Option<bool>,

    /// Broadcast replies to the channel. Default: false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broadcast_replies: Option<bool>,

    // ── Hooks ─────────────────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks: Option<Hooks>,

    // ── System prompt (markdown body) ─────────────────────────────────────
    /// The markdown body of the `.persona.md` file.
    #[serde(default)]
    pub prompt: String,
}

// ── Frontmatter-only intermediate ────────────────────────────────────────────

/// Deserializes just the YAML frontmatter (no `prompt`).
/// Unknown keys are rejected — typos cause parse errors instead of silent drops.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
struct Frontmatter {
    name: Option<String>,
    display_name: Option<String>,
    avatar: Option<String>,
    description: Option<String>,
    version: Option<String>,
    author: Option<String>,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    subscribe: Option<Vec<String>>,
    #[serde(alias = "respond_to")]
    triggers: Option<RespondTo>,
    model: Option<String>,
    runtime: Option<String>,
    temperature: Option<f64>,
    max_context_tokens: Option<u64>,
    thread_replies: Option<bool>,
    broadcast_replies: Option<bool>,
    hooks: Option<Hooks>,
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse a `.persona.md` file into a [`PersonaConfig`].
///
/// Expects YAML frontmatter between `---` delimiters followed by a markdown
/// body. The body becomes `PersonaConfig::prompt`.
///
/// # Limits
/// - Frontmatter: max 1 MiB
/// - Body: max 256 KiB
pub fn parse_persona_md(content: &str) -> Result<PersonaConfig, PersonaError> {
    let (fm_str, body) = split_frontmatter(content)?;

    if fm_str.len() > MAX_FRONTMATTER_BYTES {
        return Err(PersonaError::FrontmatterTooLarge);
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(PersonaError::BodyTooLarge);
    }

    let fm: Frontmatter = serde_yaml::from_str(fm_str)?;

    let name = fm.name.ok_or(PersonaError::MissingField("name".into()))?;
    let display_name = fm
        .display_name
        .ok_or(PersonaError::MissingField("display_name".into()))?;
    let description = fm
        .description
        .ok_or(PersonaError::MissingField("description".into()))?;

    // Fix #1: enforce non-empty required strings
    if name.trim().is_empty() {
        return Err(PersonaError::MissingField("name (empty)".into()));
    }
    if display_name.trim().is_empty() {
        return Err(PersonaError::MissingField("display_name (empty)".into()));
    }
    if description.trim().is_empty() {
        return Err(PersonaError::MissingField("description (empty)".into()));
    }

    Ok(PersonaConfig {
        name,
        display_name,
        avatar: fm.avatar,
        description,
        version: fm.version,
        author: fm.author,
        skills: fm.skills,
        mcp_servers: fm.mcp_servers,
        subscribe: fm.subscribe,
        triggers: fm.triggers,
        model: fm.model,
        runtime: fm.runtime,
        temperature: fm.temperature,
        max_context_tokens: fm.max_context_tokens,
        thread_replies: fm.thread_replies,
        broadcast_replies: fm.broadcast_replies,
        hooks: fm.hooks,
        prompt: body.to_string(),
    })
}

/// Parse a `.persona.md` file from disk.
pub fn parse_persona_file(path: &Path) -> Result<PersonaConfig, PersonaError> {
    // Fix #4: check file size before reading to avoid large allocations
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_FRONTMATTER_BYTES as u64 + MAX_BODY_BYTES as u64 + 100 {
        return Err(PersonaError::TooLarge("file exceeds maximum size".into()));
    }
    let content = std::fs::read_to_string(path)?;
    parse_persona_md(&content)
}

/// Split content into `(frontmatter_str, body_str)`.
///
/// Expects the file to begin with `---\n` and contain a second `---` line.
/// The closing `---` must be on its own line: followed by `\n`, `\r\n`, or EOF.
/// A line like `---junk` is NOT treated as a closing delimiter.
pub fn split_frontmatter(content: &str) -> Result<(&str, &str), PersonaError> {
    // Must start with "---"
    let rest = content
        .strip_prefix("---")
        .ok_or(PersonaError::NoFrontmatter)?;

    // Skip optional \r after the opening ---
    let rest = rest.strip_prefix('\r').unwrap_or(rest);
    let rest = rest.strip_prefix('\n').ok_or(PersonaError::NoFrontmatter)?;

    // Find the closing --- that is on its own line (followed by \r\n, \n, or EOF).
    // A line like "---junk" is not a valid delimiter — keep searching.
    let mut search_from = 0;
    let close = loop {
        let pos = rest[search_from..]
            .find("\n---")
            .map(|p| p + search_from)
            .ok_or(PersonaError::NoFrontmatter)?;
        let after_dashes = pos + 4; // position after "\n---"
        if after_dashes >= rest.len() {
            // "---" at EOF — valid closing delimiter
            break pos;
        }
        match rest.as_bytes().get(after_dashes) {
            Some(b'\n') | Some(b'\r') => break pos, // valid delimiter
            _ => {
                search_from = after_dashes; // not a delimiter, keep looking
                continue;
            }
        }
    };

    let fm_str = &rest[..close];
    let after_close = &rest[close + 4..]; // skip "\n---"

    // Skip optional \r\n or \n after closing ---
    let body = after_close
        .strip_prefix("\r\n")
        .or_else(|| after_close.strip_prefix('\n'))
        .unwrap_or(after_close);

    Ok((fm_str, body))
}

// ── Model string helper ───────────────────────────────────────────────────────

/// Split `"provider:model-id"` into `(Some("provider"), "model-id")`.
///
/// If there is no colon, returns `(None, full_string)`.
pub fn split_model(model: &str) -> (Option<&str>, &str) {
    match model.split_once(':') {
        Some((provider, id)) => (Some(provider), id),
        None => (None, model),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────

    fn minimal() -> &'static str {
        "---\nname: my-bot\ndisplay_name: My Bot\ndescription: Does things.\n---\n"
    }

    // ── Happy path ────────────────────────────────────────────────────────

    #[test]
    fn parse_minimal_valid() {
        let p = parse_persona_md(minimal()).unwrap();
        assert_eq!(p.name, "my-bot");
        assert_eq!(p.display_name, "My Bot");
        assert_eq!(p.description, "Does things.");
        assert_eq!(p.prompt, "");
    }

    #[test]
    fn parse_with_body() {
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\n---\nYou are Bot.\n";
        let p = parse_persona_md(src).unwrap();
        assert_eq!(p.prompt, "You are Bot.\n");
    }

    #[test]
    fn empty_body_is_valid() {
        let p = parse_persona_md(minimal()).unwrap();
        assert_eq!(p.prompt, "");
    }

    #[test]
    fn parse_full_fields() {
        let src = indoc(
            "---
name: full-bot
display_name: Full Bot
avatar: assets/avatar.png
description: Full featured.
version: 1.2.3
author: Tyler
skills:
  - skills/search
mcp_servers:
  - name: my-mcp
    command: npx
    args: ['-y', 'my-server']
    env:
      TOKEN: abc123
subscribe:
  - '#general'
respond_to:
  mentions: true
  keywords: [hello, help]
  all_messages: false
model: openai:gpt-4o
temperature: 0.7
max_context_tokens: 8192
thread_replies: true
broadcast_replies: false
hooks:
  on_start: hooks/start.sh
  on_stop: hooks/stop.sh
---
You are Full Bot.
",
        );
        let p = parse_persona_md(src).unwrap();
        assert_eq!(p.name, "full-bot");
        assert_eq!(p.avatar.as_deref(), Some("assets/avatar.png"));
        assert_eq!(p.skills, vec!["skills/search"]);
        assert_eq!(p.mcp_servers.len(), 1);
        assert_eq!(p.mcp_servers[0].name, "my-mcp");
        assert_eq!(p.mcp_servers[0].env["TOKEN"], "abc123");
        assert_eq!(p.subscribe, Some(vec!["#general".to_owned()]));
        let rt = p.triggers.unwrap();
        assert_eq!(rt.keywords, vec!["hello", "help"]);
        assert_eq!(p.model.as_deref(), Some("openai:gpt-4o"));
        assert_eq!(p.temperature, Some(0.7));
        assert_eq!(p.max_context_tokens, Some(8192));
        let hooks = p.hooks.unwrap();
        assert_eq!(hooks.on_start.as_deref(), Some("hooks/start.sh"));
        assert_eq!(p.prompt, "You are Full Bot.\n");
    }

    #[test]
    fn unknown_frontmatter_keys_error() {
        // deny_unknown_fields: typos in frontmatter keys cause a parse error.
        let src =
            "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\nunknown_key: surprise\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(matches!(err, PersonaError::Yaml(_)), "got: {err}");
    }

    #[test]
    fn unknown_hook_key_errors() {
        // deny_unknown_fields on Hooks means a typo like "on_init" is caught.
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\nhooks:\n  on_start: hooks/start.sh\n  on_init: hooks/init.sh\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(matches!(err, PersonaError::Yaml(_)), "got: {err}");
    }

    // ── Missing required fields ───────────────────────────────────────────

    #[test]
    fn missing_name_errors() {
        let src = "---\ndisplay_name: Bot\ndescription: A bot.\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(
            matches!(&err, PersonaError::MissingField(f) if f == "name"),
            "got: {err}"
        );
    }

    #[test]
    fn missing_display_name_errors() {
        let src = "---\nname: bot\ndescription: A bot.\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(
            matches!(&err, PersonaError::MissingField(f) if f == "display_name"),
            "got: {err}"
        );
    }

    #[test]
    fn missing_description_errors() {
        let src = "---\nname: bot\ndisplay_name: Bot\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(
            matches!(&err, PersonaError::MissingField(f) if f == "description"),
            "got: {err}"
        );
    }

    // ── Empty required fields (Fix #1) ────────────────────────────────────

    #[test]
    fn empty_name_errors() {
        let src = "---\nname: \"\"\ndisplay_name: Bot\ndescription: A bot.\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(
            matches!(&err, PersonaError::MissingField(f) if f.contains("name")),
            "got: {err}"
        );
    }

    #[test]
    fn whitespace_only_name_errors() {
        let src = "---\nname: \"   \"\ndisplay_name: Bot\ndescription: A bot.\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(
            matches!(&err, PersonaError::MissingField(f) if f.contains("name")),
            "got: {err}"
        );
    }

    #[test]
    fn empty_display_name_errors() {
        let src = "---\nname: bot\ndisplay_name: \"\"\ndescription: A bot.\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(
            matches!(&err, PersonaError::MissingField(f) if f.contains("display_name")),
            "got: {err}"
        );
    }

    #[test]
    fn empty_description_errors() {
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: \"\"\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(
            matches!(&err, PersonaError::MissingField(f) if f.contains("description")),
            "got: {err}"
        );
    }

    // ── Delimiter errors ──────────────────────────────────────────────────

    #[test]
    fn no_frontmatter_delimiters_errors() {
        let err = parse_persona_md("Just plain markdown.").unwrap_err();
        assert!(matches!(err, PersonaError::NoFrontmatter));
    }

    #[test]
    fn missing_closing_delimiter_errors() {
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(matches!(err, PersonaError::NoFrontmatter));
    }

    #[test]
    fn closing_delimiter_with_trailing_junk_is_not_valid() {
        // "---junk" must NOT be treated as a closing delimiter.
        // The parser should keep searching and ultimately return NoFrontmatter.
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\n---junk\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(
            matches!(err, PersonaError::NoFrontmatter),
            "expected NoFrontmatter, got: {err}"
        );
    }

    #[test]
    fn closing_delimiter_with_junk_skipped_finds_real_close() {
        // A "---junk" line inside a YAML block scalar should be skipped;
        // the real "---" on its own line should still be found.
        // Use a literal block scalar (|) so "---junk" is valid YAML content.
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: |\n  some text\n  ---junk\n---\nBody here.\n";
        let p = parse_persona_md(src).unwrap();
        assert_eq!(p.name, "bot");
        assert_eq!(p.prompt, "Body here.\n");
    }

    // ── Malformed YAML ────────────────────────────────────────────────────

    #[test]
    fn malformed_yaml_errors() {
        let src = "---\n: bad: yaml: here\n---\n";
        let err = parse_persona_md(src).unwrap_err();
        assert!(matches!(err, PersonaError::Yaml(_)));
    }

    // ── Size limits ───────────────────────────────────────────────────────

    #[test]
    fn frontmatter_too_large_errors() {
        // Build a frontmatter that exceeds 1 MiB
        let big = "x".repeat(MAX_FRONTMATTER_BYTES + 1);
        let src = format!("---\n{big}\n---\n");
        let err = parse_persona_md(&src).unwrap_err();
        assert!(matches!(err, PersonaError::FrontmatterTooLarge));
    }

    #[test]
    fn body_too_large_errors() {
        let big = "x".repeat(MAX_BODY_BYTES + 1);
        let src = format!("---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\n---\n{big}");
        let err = parse_persona_md(&src).unwrap_err();
        assert!(matches!(err, PersonaError::BodyTooLarge));
    }

    // ── split_model ───────────────────────────────────────────────────────

    #[test]
    fn split_model_with_colon() {
        let (provider, id) = split_model("openai:gpt-4o");
        assert_eq!(provider, Some("openai"));
        assert_eq!(id, "gpt-4o");
    }

    #[test]
    fn split_model_without_colon() {
        let (provider, id) = split_model("gpt-4o");
        assert_eq!(provider, None);
        assert_eq!(id, "gpt-4o");
    }

    #[test]
    fn split_model_multiple_colons_uses_first() {
        let (provider, id) = split_model("databricks:gpt-5:preview");
        assert_eq!(provider, Some("databricks"));
        assert_eq!(id, "gpt-5:preview");
    }

    // ── Subscribe three-state semantics (S2) ─────────────────────────────

    #[test]
    fn parse_subscribe_null_is_none() {
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\nsubscribe: null\n---\n";
        let p = parse_persona_md(src).unwrap();
        assert_eq!(p.subscribe, None, "YAML null should deserialize to None");
    }

    #[test]
    fn parse_subscribe_empty_is_some_empty() {
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\nsubscribe: []\n---\n";
        let p = parse_persona_md(src).unwrap();
        assert_eq!(
            p.subscribe,
            Some(vec![]),
            "YAML [] should deserialize to Some(empty vec)"
        );
    }

    #[test]
    fn parse_subscribe_omitted_is_none() {
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\n---\n";
        let p = parse_persona_md(src).unwrap();
        assert_eq!(p.subscribe, None, "omitted subscribe should be None");
    }

    #[test]
    fn parse_triggers_canonical_key() {
        // `triggers:` is the canonical YAML key (spec Section 4).
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\ntriggers:\n  mentions: true\n  keywords: [hello, help]\n  all_messages: false\n---\n";
        let p = parse_persona_md(src).unwrap();
        let t = p.triggers.expect("triggers should be Some");
        assert_eq!(t.mentions, Some(true));
        assert_eq!(t.keywords, vec!["hello", "help"]);
        assert_eq!(t.all_messages, Some(false));
    }

    #[test]
    fn parse_triggers_legacy_respond_to_alias() {
        // `respond_to:` is accepted as a legacy alias for `triggers:`.
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\nrespond_to:\n  mentions: true\n  keywords: [hello, help]\n  all_messages: false\n---\n";
        let p = parse_persona_md(src).unwrap();
        let t = p
            .triggers
            .expect("triggers should be Some via respond_to alias");
        assert_eq!(t.mentions, Some(true));
        assert_eq!(t.keywords, vec!["hello", "help"]);
    }

    #[test]
    fn parse_subscribe_with_channels() {
        let src = "---\nname: bot\ndisplay_name: Bot\ndescription: A bot.\nsubscribe:\n  - \"#general\"\n  - \"#random\"\n---\n";
        let p = parse_persona_md(src).unwrap();
        assert_eq!(
            p.subscribe,
            Some(vec!["#general".to_owned(), "#random".to_owned()])
        );
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    /// Trim leading newline from indented string literals.
    fn indoc(s: &str) -> &str {
        s.strip_prefix('\n').unwrap_or(s)
    }
}
