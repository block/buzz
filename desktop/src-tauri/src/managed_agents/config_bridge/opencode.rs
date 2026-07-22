use std::path::PathBuf;

use super::types::{ExtensionEntry, RuntimeFileConfig};

/// Read OpenCode config from `~/.config/opencode/opencode.json` (or
/// `$OPENCODE_CONFIG`). Model strings are `provider/model`; MCP servers live
/// under the `mcp` key of the same file.
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let path = opencode_config_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    parse_opencode_config(&raw)
}

fn parse_opencode_config(raw: &str) -> Option<RuntimeFileConfig> {
    let json: serde_json::Value = serde_json::from_str(raw).ok()?;

    let model = json_string(&json, "model");
    let provider = model
        .as_deref()
        .and_then(|m| m.split_once('/'))
        .map(|(provider, _)| provider.to_string());

    let skip = &[
        "$schema",
        "model",
        "small_model",
        "provider",
        "mcp",
        "agent",
        "permission",
    ];
    let mut extra = super::schema_walker::extract_config_fields(&json, skip);

    if let Some(serde_json::Value::Object(providers)) = json.get("provider") {
        for (name, _) in providers {
            extra.insert(format!("provider.{name}"), "configured".to_string());
        }
    }
    if let Some(serde_json::Value::Object(agents)) = json.get("agent") {
        for (name, _) in agents {
            extra.insert(format!("agent.{name}"), "configured".to_string());
        }
    }

    let extensions = json
        .get("mcp")
        .and_then(|v| v.as_object())
        .map(|servers| {
            servers
                .iter()
                .map(|(name, config)| ExtensionEntry {
                    name: name.clone(),
                    kind: "mcp".to_string(),
                    enabled: config
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                })
                .collect()
        })
        .unwrap_or_default();

    Some(RuntimeFileConfig {
        model,
        provider,
        mode: None,
        thinking_effort: None,
        max_output_tokens: None,
        context_limit: None,
        system_prompt: None,
        extensions,
        extra,
    })
}

fn json_string(val: &serde_json::Value, key: &str) -> Option<String> {
    val.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn opencode_config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OPENCODE_CONFIG") {
        return Some(PathBuf::from(path));
    }
    let config_root = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))?;
    Some(config_root.join("opencode").join("opencode.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_model_and_provider_from_model_string() {
        let cfg = parse_opencode_config(
            r#"{"$schema": "https://opencode.ai/config.json", "model": "anthropic/claude-sonnet-4-5"}"#,
        )
        .unwrap();
        assert_eq!(cfg.model.as_deref(), Some("anthropic/claude-sonnet-4-5"));
        assert_eq!(cfg.provider.as_deref(), Some("anthropic"));
        assert!(!cfg.extra.contains_key("model"));
        assert!(!cfg.extra.contains_key("$schema"));
    }

    #[test]
    fn provider_and_agent_tables_marked_configured() {
        let cfg = parse_opencode_config(
            r#"{
                "provider": {"openrouter": {"apiKey": "sk"}},
                "agent": {"review": {"model": "anthropic/claude-haiku-4-5"}}
            }"#,
        )
        .unwrap();
        assert_eq!(
            cfg.extra.get("provider.openrouter").map(String::as_str),
            Some("configured")
        );
        assert_eq!(
            cfg.extra.get("agent.review").map(String::as_str),
            Some("configured")
        );
        assert!(cfg.model.is_none());
        assert!(cfg.provider.is_none());
    }

    #[test]
    fn mcp_servers_become_extensions_with_enabled_flag() {
        let cfg = parse_opencode_config(
            r#"{"mcp": {
                "filesystem": {"type": "local", "command": ["npx", "mcp-fs"]},
                "disabled-one": {"type": "local", "command": ["x"], "enabled": false}
            }}"#,
        )
        .unwrap();
        assert_eq!(cfg.extensions.len(), 2);
        let enabled: Vec<_> = cfg
            .extensions
            .iter()
            .filter(|e| e.enabled)
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(enabled, vec!["filesystem"]);
    }

    #[test]
    fn invalid_json_returns_none() {
        assert!(parse_opencode_config("{{{{not valid").is_none());
    }
}
