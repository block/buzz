use std::path::PathBuf;

use super::types::{ExtensionEntry, RuntimeFileConfig};

/// Read OpenCode config from `$OPENCODE_CONFIG`, else
/// `$XDG_CONFIG_HOME/opencode/opencode.json(c)`, else
/// `~/.config/opencode/opencode.json(c)`.
///
/// This tier matters more for OpenCode than for the other harnesses: `opencode
/// acp` takes no `--model` flag and reads no model env var, so the config file
/// is the ONLY place its model is set. Without this reader the model field is
/// blank in the panel even when the harness is perfectly well configured.
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let raw = std::fs::read_to_string(opencode_config_path()?).ok()?;
    parse_opencode_config(&raw)
}

/// Canonical config path for display and for the reader.
///
/// Returns the `.json` path even when nothing exists on disk yet — the panel
/// shows where the file *would* live, matching how `claude` reports
/// `~/.claude.json` unconditionally.
pub(crate) fn opencode_config_path() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("OPENCODE_CONFIG") {
        let path = PathBuf::from(explicit);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }

    let dir = opencode_config_dir()?;
    let json = dir.join("opencode.json");
    if json.exists() {
        return Some(json);
    }
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.exists() {
        return Some(jsonc);
    }
    Some(json)
}

fn opencode_config_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let base = PathBuf::from(xdg);
        if !base.as_os_str().is_empty() {
            return Some(base.join("opencode"));
        }
    }
    Some(dirs::home_dir()?.join(".config").join("opencode"))
}

fn parse_opencode_config(raw: &str) -> Option<RuntimeFileConfig> {
    let value: serde_json::Value = serde_json::from_str(&strip_jsonc(raw)).ok()?;

    // OpenCode writes the model as `provider_id/model_id`. Split it so the
    // normalized provider and model fields each carry their own half rather
    // than repeating the whole pair in both.
    let (provider, model) = match json_string(&value, "model") {
        Some(spec) => match spec.split_once('/') {
            Some((p, m)) if !p.is_empty() && !m.is_empty() => {
                (Some(p.to_string()), Some(m.to_string()))
            }
            // No slash (or a malformed one) — surface the value as written
            // instead of guessing at a provider.
            _ => (None, Some(spec)),
        },
        None => (None, None),
    };

    let extensions = parse_mcp_servers(&value);

    // Config-driven extra fields — skip keys extracted into typed fields above.
    let skip = &["model", "provider", "mcp"];
    let mut extra = super::schema_walker::extract_config_fields(&value, skip);

    // Custom providers from `provider.<id>` — surface as
    // "provider.<name> = configured" rather than flattening their model tables,
    // mirroring how the codex reader handles `model_providers`.
    if let Some(serde_json::Value::Object(providers)) = value.get("provider") {
        for name in providers.keys() {
            extra.insert(format!("provider.{name}"), "configured".to_string());
        }
    }

    Some(RuntimeFileConfig {
        model,
        provider,
        // OpenCode has no single mode/effort/limit key: permissions live under
        // `permission`, reasoning effort is per-model under
        // `provider.<id>.models.<id>.options`. Both reach the panel via `extra`.
        mode: None,
        thinking_effort: None,
        max_output_tokens: None,
        context_limit: None,
        // `instructions` is a list of file PATHS, not prompt text, so it is not
        // a system prompt. The walker surfaces it in `extra`.
        system_prompt: None,
        extensions,
        extra,
    })
}

fn parse_mcp_servers(value: &serde_json::Value) -> Vec<ExtensionEntry> {
    let Some(servers) = value.get("mcp").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    servers
        .iter()
        .map(|(name, config)| ExtensionEntry {
            name: name.clone(),
            kind: "mcp".to_string(),
            // OpenCode runs an MCP server unless it explicitly opts out.
            enabled: config
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        })
        .collect()
}

fn json_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Make a JSONC document parseable by `serde_json`: drop comments, then drop
/// trailing commas. OpenCode documents `.jsonc` as a first-class config format
/// and its own docs use both, so a plain `serde_json` parse would reject real
/// user configs.
fn strip_jsonc(raw: &str) -> String {
    strip_trailing_commas(&strip_comments(raw))
}

/// Remove `//` and `/* */` comments. String-aware: `//` inside a JSON string
/// must survive — every OpenCode config carries at least one URL (`$schema`).
/// Newlines inside block comments are preserved so line-based error offsets
/// still line up with the original file.
fn strip_comments(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_string = false;

    while i < chars.len() {
        let c = chars[i];

        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
                i = i.saturating_add(2).min(chars.len());
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }

    out
}

/// Drop a `,` whose next significant character is `}` or `]`. Runs on
/// already-comment-free text, so "significant" only has to skip whitespace.
fn strip_trailing_commas(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_string = false;

    while i < chars.len() {
        let c = chars[i];

        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '"' {
            in_string = true;
            out.push(c);
            i += 1;
            continue;
        }

        if c == ',' {
            let next = chars[i + 1..].iter().find(|ch| !ch.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }

        out.push(c);
        i += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_splits_into_provider_and_model() {
        let cfg = parse_opencode_config(r#"{"model": "anthropic/claude-sonnet-4-5"}"#).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("anthropic"));
        assert_eq!(cfg.model.as_deref(), Some("claude-sonnet-4-5"));
    }

    #[test]
    fn model_id_containing_slashes_keeps_everything_after_the_first() {
        // `lmstudio/google/gemma-3n-e4b` — provider is the FIRST segment; the
        // rest is the model id, which may itself contain slashes.
        let cfg = parse_opencode_config(r#"{"model": "lmstudio/google/gemma-3n-e4b"}"#).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("lmstudio"));
        assert_eq!(cfg.model.as_deref(), Some("google/gemma-3n-e4b"));
    }

    #[test]
    fn model_without_a_provider_prefix_is_surfaced_as_written() {
        let cfg = parse_opencode_config(r#"{"model": "gpt-5"}"#).unwrap();
        assert_eq!(cfg.model.as_deref(), Some("gpt-5"));
        assert!(cfg.provider.is_none());
    }

    #[test]
    fn mcp_servers_become_extensions_and_honor_enabled_false() {
        let cfg = parse_opencode_config(
            r#"{"mcp": {
                "filesystem": {"type": "local", "command": ["npx", "-y", "fs"]},
                "sentry": {"type": "remote", "url": "https://x", "enabled": false}
            }}"#,
        )
        .unwrap();
        assert_eq!(cfg.extensions.len(), 2);
        let sentry = cfg.extensions.iter().find(|e| e.name == "sentry").unwrap();
        assert!(!sentry.enabled);
        let fs = cfg
            .extensions
            .iter()
            .find(|e| e.name == "filesystem")
            .unwrap();
        assert!(fs.enabled, "an mcp entry with no `enabled` key defaults on");
    }

    #[test]
    fn custom_providers_are_summarized_not_flattened() {
        let cfg = parse_opencode_config(
            r#"{
                "model": "helicone/gpt-4o",
                "provider": {"helicone": {"npm": "@ai-sdk/openai-compatible", "models": {"gpt-4o": {}}}}
            }"#,
        )
        .unwrap();
        assert_eq!(
            cfg.extra.get("provider.helicone").map(String::as_str),
            Some("configured")
        );
        assert!(
            !cfg.extra
                .keys()
                .any(|k| k.starts_with("provider.helicone.")),
            "provider internals must not be flattened into extra"
        );
    }

    #[test]
    fn normalized_keys_are_not_duplicated_in_extra() {
        let cfg = parse_opencode_config(
            r#"{"model": "anthropic/x", "mcp": {"a": {}}, "theme": "opencode"}"#,
        )
        .unwrap();
        assert!(!cfg.extra.contains_key("model"));
        assert!(!cfg.extra.contains_key("mcp.a"));
        assert_eq!(cfg.extra.get("theme").map(String::as_str), Some("opencode"));
    }

    #[test]
    fn unknown_future_fields_reach_extra() {
        let cfg = parse_opencode_config(r#"{"some_new_opencode_field": "value"}"#).unwrap();
        assert_eq!(
            cfg.extra.get("some_new_opencode_field").map(String::as_str),
            Some("value")
        );
    }

    #[test]
    fn jsonc_comments_are_stripped_without_eating_urls() {
        let raw = r#"{
            // the schema line is a comment magnet
            "$schema": "https://opencode.ai/config.json",
            "model": "openai/gpt-5" /* inline block */
        }"#;
        let cfg = parse_opencode_config(raw).unwrap();
        assert_eq!(cfg.model.as_deref(), Some("gpt-5"));
        assert_eq!(
            cfg.extra.get("$schema").map(String::as_str),
            Some("https://opencode.ai/config.json"),
            "a `//` inside a string must survive comment stripping"
        );
    }

    #[test]
    fn a_double_slash_inside_a_string_is_never_treated_as_a_comment() {
        let stripped = strip_comments(r#"{"url": "http://localhost:8080/v1"}"#);
        assert_eq!(stripped, r#"{"url": "http://localhost:8080/v1"}"#);
    }

    #[test]
    fn an_escaped_quote_does_not_end_the_string_scan() {
        let cfg = parse_opencode_config(r#"{"username": "say \"hi\" // not a comment"}"#).unwrap();
        assert_eq!(
            cfg.extra.get("username").map(String::as_str),
            Some(r#"say "hi" // not a comment"#)
        );
    }

    #[test]
    fn trailing_commas_are_tolerated() {
        let raw = r#"{
            "model": "openai/gpt-5",
            "instructions": ["A.md", "B.md",],
        }"#;
        let cfg = parse_opencode_config(raw).unwrap();
        assert_eq!(cfg.model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn a_comma_inside_a_string_is_not_mistaken_for_a_trailing_comma() {
        let cfg = parse_opencode_config(r#"{"username": "last, first"}"#).unwrap();
        assert_eq!(
            cfg.extra.get("username").map(String::as_str),
            Some("last, first")
        );
    }

    #[test]
    fn empty_config_parses_to_an_empty_surface() {
        let cfg = parse_opencode_config("{}").unwrap();
        assert!(cfg.model.is_none());
        assert!(cfg.provider.is_none());
        assert!(cfg.extensions.is_empty());
    }

    #[test]
    fn unparseable_config_returns_none() {
        assert!(parse_opencode_config("{{{{ not json").is_none());
    }
}
