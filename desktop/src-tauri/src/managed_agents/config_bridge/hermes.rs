use std::path::PathBuf;

use super::types::RuntimeFileConfig;

/// Read Hermes Agent config from `$HERMES_HOME/config.yaml` or `~/.hermes/config.yaml`.
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let path = hermes_config_path()?;
    read_config_from_path(&path)
}

fn read_config_from_path(path: &std::path::Path) -> Option<RuntimeFileConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    parse_hermes_config(&raw)
}

fn parse_hermes_config(yaml_str: &str) -> Option<RuntimeFileConfig> {
    let root: serde_yaml::Value = serde_yaml::from_str(yaml_str).ok()?;
    let map = root.as_mapping()?;

    let model_value = mapping_value(map, "model");
    let model = match model_value {
        Some(serde_yaml::Value::String(value)) => nonempty(value),
        Some(serde_yaml::Value::Mapping(model_map)) => mapping_string(model_map, "default"),
        _ => None,
    };
    let provider = model_value
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|model_map| mapping_string(model_map, "provider"));
    let thinking_effort = mapping_value(map, "agent")
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|agent_map| mapping_string(agent_map, "reasoning_effort"));

    let mut extra = std::collections::BTreeMap::new();
    if let Some(base_url) = model_value
        .and_then(serde_yaml::Value::as_mapping)
        .and_then(|model_map| mapping_string(model_map, "base_url"))
    {
        extra.insert("model.base_url".to_string(), base_url);
    }

    let extensions = mapping_value(map, "mcp_servers")
        .and_then(serde_yaml::Value::as_mapping)
        .map(|servers| {
            servers
                .iter()
                .filter_map(|(name, value)| {
                    let name = nonempty(name.as_str()?)?;
                    let server = value.as_mapping()?;
                    let kind = if mapping_string(server, "url").is_some() {
                        "http"
                    } else if mapping_string(server, "command").is_some() {
                        "stdio"
                    } else {
                        "unknown"
                    };
                    let enabled = mapping_value(server, "enabled")
                        .and_then(serde_yaml::Value::as_bool)
                        .unwrap_or(true);
                    Some(super::types::ExtensionEntry {
                        name,
                        kind: kind.to_string(),
                        enabled,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(RuntimeFileConfig {
        model,
        provider,
        thinking_effort,
        extensions,
        extra,
        ..RuntimeFileConfig::default()
    })
}

fn mapping_value<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a serde_yaml::Value> {
    map.get(serde_yaml::Value::String(key.to_string()))
}

fn mapping_string(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    mapping_value(map, key)
        .and_then(serde_yaml::Value::as_str)
        .and_then(nonempty)
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn hermes_config_path() -> Option<PathBuf> {
    if let Ok(root) = std::env::var("HERMES_HOME") {
        return Some(PathBuf::from(root).join("config.yaml"));
    }
    dirs::home_dir().map(|home| home.join(".hermes").join("config.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_provider_reasoning_and_mcp_servers() {
        let yaml = r#"
model:
  default: anthropic/claude-sonnet-4
  provider: openrouter
  base_url: https://openrouter.ai/api/v1
agent:
  reasoning_effort: high
mcp_servers:
  filesystem:
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem"]
  remote:
    url: https://mcp.example.test
    enabled: false
"#;

        let cfg = parse_hermes_config(yaml).expect("valid Hermes YAML should parse");
        assert_eq!(cfg.model.as_deref(), Some("anthropic/claude-sonnet-4"));
        assert_eq!(cfg.provider.as_deref(), Some("openrouter"));
        assert_eq!(cfg.thinking_effort.as_deref(), Some("high"));
        assert_eq!(
            cfg.extra.get("model.base_url").map(String::as_str),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(cfg
            .extensions
            .iter()
            .any(|entry| entry.name == "filesystem" && entry.kind == "stdio" && entry.enabled));
        assert!(cfg
            .extensions
            .iter()
            .any(|entry| entry.name == "remote" && entry.kind == "http" && !entry.enabled));
    }

    #[test]
    fn accepts_legacy_scalar_model_and_empty_yaml() {
        let scalar = parse_hermes_config("model: glm-5").expect("scalar model should parse");
        assert_eq!(scalar.model.as_deref(), Some("glm-5"));
        assert!(scalar.provider.is_none());

        let empty = parse_hermes_config("{}").expect("empty config should parse");
        assert!(empty.model.is_none());
        assert!(empty.provider.is_none());
        assert!(empty.extensions.is_empty());
    }

    #[test]
    fn invalid_yaml_returns_none() {
        assert!(parse_hermes_config("{{{{not yaml").is_none());
    }
}
