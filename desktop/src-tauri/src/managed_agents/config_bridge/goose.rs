use std::{collections::BTreeMap, path::PathBuf};

use super::types::{ExtensionEntry, RuntimeFileConfig};

/// Read goose config from `~/.config/goose/config.yaml` (or `$GOOSE_PATH_ROOT`).
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let path = goose_config_path()?;
    read_config_from_path(&path)
}

fn read_config_from_path(path: &std::path::Path) -> Option<RuntimeFileConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    parse_goose_config(&raw)
}

fn parse_goose_config(yaml_str: &str) -> Option<RuntimeFileConfig> {
    let map: std::collections::HashMap<String, serde_yaml::Value> =
        serde_yaml::from_str(yaml_str).ok()?;

    let active_provider = yaml_string(&map, "active_provider");

    // Flat-key extraction (top-level env-style keys).
    let goose_provider = yaml_string(&map, "GOOSE_PROVIDER");
    let goose_model = yaml_string(&map, "GOOSE_MODEL");
    let goose_mode = yaml_string(&map, "GOOSE_MODE");
    let goose_max_tokens = yaml_string(&map, "GOOSE_MAX_TOKENS");
    let goose_context_limit = yaml_string(&map, "GOOSE_CONTEXT_LIMIT");

    // Nested provider format: active_provider → providers.<name>.{model,host,...}
    let nested = active_provider
        .as_deref()
        .and_then(|ap| nested_provider_fields(&map, ap));

    let provider = goose_provider.or_else(|| active_provider.clone());
    let model = goose_model.or_else(|| nested.as_ref().and_then(|n| n.model.clone()));
    let mode = goose_mode;

    let extensions = parse_extensions(&map);

    let mut extra = BTreeMap::new();
    if let Some(ref ap) = active_provider {
        extra.insert("active_provider".to_string(), ap.clone());
    }
    if let Some(host) = yaml_string(&map, "DATABRICKS_HOST")
        .or_else(|| nested.as_ref().and_then(|n| n.host.clone()))
    {
        let host_key = match active_provider.as_deref() {
            Some("databricks_v2") | Some("databricks") => "DATABRICKS_HOST".to_string(),
            Some(p) => format!("{p}.host"),
            None => "provider.host".to_string(),
        };
        extra.insert(host_key, host);
    }

    Some(RuntimeFileConfig {
        model,
        provider,
        mode,
        thinking_effort: yaml_string(&map, "GOOSE_THINKING_EFFORT"),
        max_output_tokens: goose_max_tokens,
        context_limit: goose_context_limit,
        system_prompt: None,
        extensions,
        extra,
    })
}

struct NestedProviderFields {
    model: Option<String>,
    host: Option<String>,
}

fn nested_provider_fields(
    map: &std::collections::HashMap<String, serde_yaml::Value>,
    active_provider: &str,
) -> Option<NestedProviderFields> {
    let providers = map.get("providers").and_then(|v| v.as_mapping())?;
    let entry = providers
        .get(serde_yaml::Value::String(active_provider.to_owned()))?
        .as_mapping()?;

    let model = mapping_string(entry, "model");
    let host = mapping_string(entry, "host");

    Some(NestedProviderFields { model, host })
}

fn parse_extensions(
    map: &std::collections::HashMap<String, serde_yaml::Value>,
) -> Vec<ExtensionEntry> {
    let extensions = match map.get("extensions").and_then(|v| v.as_mapping()) {
        Some(m) => m,
        None => return Vec::new(),
    };

    extensions
        .iter()
        .filter_map(|(k, v)| {
            let name = k.as_str()?.to_string();
            let kind = v
                .as_mapping()
                .and_then(|m| mapping_string(m, "type"))
                .unwrap_or_else(|| "unknown".to_string());
            let enabled = v
                .as_mapping()
                .and_then(|m| {
                    m.get(serde_yaml::Value::String("enabled".to_owned()))
                        .and_then(|v| v.as_bool())
                })
                .unwrap_or(true);
            Some(ExtensionEntry {
                name,
                kind,
                enabled,
            })
        })
        .collect()
}

fn yaml_string(
    map: &std::collections::HashMap<String, serde_yaml::Value>,
    key: &str,
) -> Option<String> {
    map.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn mapping_string(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    map.get(serde_yaml::Value::String(key.to_owned()))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn goose_config_path() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("GOOSE_PATH_ROOT") {
        return Some(PathBuf::from(root).join("config").join("config.yaml"));
    }
    let home = dirs::home_dir()?;
    Some(home.join(".config").join("goose").join("config.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_flat_keys() {
        let yaml = r#"
GOOSE_PROVIDER: anthropic
GOOSE_MODEL: claude-sonnet-4-20250514
GOOSE_MODE: auto
GOOSE_MAX_TOKENS: "8192"
"#;
        let cfg = parse_goose_config(yaml).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("anthropic"));
        assert_eq!(cfg.model.as_deref(), Some("claude-sonnet-4-20250514"));
        assert_eq!(cfg.mode.as_deref(), Some("auto"));
        assert_eq!(cfg.max_output_tokens.as_deref(), Some("8192"));
    }

    #[test]
    fn parse_nested_provider() {
        let yaml = r#"
active_provider: databricks_v2
providers:
  databricks_v2:
    model: goose-claude-4-6-opus
    host: https://dbc.example
"#;
        let cfg = parse_goose_config(yaml).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("databricks_v2"));
        assert_eq!(cfg.model.as_deref(), Some("goose-claude-4-6-opus"));
        assert_eq!(
            cfg.extra.get("DATABRICKS_HOST").map(|s| s.as_str()),
            Some("https://dbc.example")
        );
    }

    #[test]
    fn non_databricks_provider_uses_provider_host_key() {
        let yaml = r#"
active_provider: anthropic
providers:
  anthropic:
    model: claude-opus-4
    host: https://api.anthropic.com
"#;
        let cfg = parse_goose_config(yaml).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            cfg.extra.get("anthropic.host").map(|s| s.as_str()),
            Some("https://api.anthropic.com")
        );
        assert!(!cfg.extra.contains_key("DATABRICKS_HOST"));
    }

    #[test]
    fn flat_model_wins_over_nested() {
        let yaml = r#"
active_provider: databricks_v2
GOOSE_MODEL: flat-model
providers:
  databricks_v2:
    model: nested-model
"#;
        let cfg = parse_goose_config(yaml).unwrap();
        assert_eq!(cfg.model.as_deref(), Some("flat-model"));
    }

    #[test]
    fn parse_extensions() {
        let yaml = r#"
extensions:
  developer:
    type: builtin
    enabled: true
  my-mcp:
    type: stdio
    enabled: false
"#;
        let cfg = parse_goose_config(yaml).unwrap();
        assert_eq!(cfg.extensions.len(), 2);
        assert!(cfg
            .extensions
            .iter()
            .any(|e| e.name == "developer" && e.enabled));
        assert!(cfg
            .extensions
            .iter()
            .any(|e| e.name == "my-mcp" && !e.enabled));
    }

    #[test]
    fn invalid_yaml_returns_none() {
        assert!(parse_goose_config("{{{{not valid").is_none());
    }

    #[test]
    fn empty_yaml_returns_empty_config() {
        let cfg = parse_goose_config("{}").unwrap();
        assert!(cfg.model.is_none());
        assert!(cfg.provider.is_none());
    }
}
