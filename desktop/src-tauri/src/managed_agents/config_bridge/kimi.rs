use std::path::PathBuf;

use super::types::{ExtensionEntry, RuntimeFileConfig};

/// Read Kimi Code config from `~/.kimi-code/config.toml` (or `$KIMI_CODE_HOME/config.toml`).
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let path = kimi_config_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    parse_kimi_config(&raw).map(|mut cfg| {
        cfg.extensions = read_mcp_config();
        cfg
    })
}

fn parse_kimi_config(toml_str: &str) -> Option<RuntimeFileConfig> {
    let table: toml::Table = toml_str.parse().ok()?;

    let model = toml_string(&table, "default_model");
    let mut provider = None;
    let mut context_limit = None;

    if let Some(model_id) = model.as_deref() {
        if let Some(models) = table.get("models").and_then(|v| v.as_table()) {
            if let Some(model_table) = models.get(model_id).and_then(|v| v.as_table()) {
                provider = toml_table_string(model_table, "provider");
                context_limit = toml_table_scalar_string(model_table, "max_context_size");
            }
        }
    }

    let config_json = toml_to_json(&toml::Value::Table(table));
    let skip = &[
        "default_model",
        "models",
        "providers",
        "permission",
        "permissions",
        "mcp",
    ];
    let mut extra = super::schema_walker::extract_config_fields(&config_json, skip);

    if let Some(serde_json::Value::Object(providers)) = config_json.get("providers") {
        for (name, provider_config) in providers {
            extra.insert(format!("providers.{name}"), "configured".to_string());
            if provider.is_none() {
                if let Some(kind) = provider_config.get("type").and_then(|v| v.as_str()) {
                    provider = Some(kind.to_string());
                }
            }
        }
    }

    if let Some(serde_json::Value::Object(models)) = config_json.get("models") {
        for (name, _) in models {
            extra.insert(format!("models.{name}"), "configured".to_string());
        }
    }

    Some(RuntimeFileConfig {
        model,
        provider,
        mode: None,
        thinking_effort: None,
        max_output_tokens: None,
        context_limit,
        system_prompt: None,
        extensions: Vec::new(),
        extra,
    })
}

fn read_mcp_config() -> Vec<ExtensionEntry> {
    let Some(path) = kimi_mcp_config_path() else {
        return Vec::new();
    };
    let Some(raw) = std::fs::read_to_string(path).ok() else {
        return Vec::new();
    };
    let Some(json) = serde_json::from_str::<serde_json::Value>(&raw).ok() else {
        return Vec::new();
    };

    let servers = json
        .get("mcpServers")
        .or_else(|| json.get("mcp_servers"))
        .and_then(|v| v.as_object());
    let Some(servers) = servers else {
        return Vec::new();
    };

    servers
        .keys()
        .map(|name| ExtensionEntry {
            name: name.clone(),
            kind: "mcp".to_string(),
            enabled: true,
        })
        .collect()
}

fn toml_string(table: &toml::Table, key: &str) -> Option<String> {
    table
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn toml_table_string(table: &toml::value::Table, key: &str) -> Option<String> {
    table
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn toml_table_scalar_string(table: &toml::value::Table, key: &str) -> Option<String> {
    match table.get(key)? {
        toml::Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        toml::Value::Integer(i) => Some(i.to_string()),
        toml::Value::Float(f) => Some(f.to_string()),
        _ => None,
    }
}

fn toml_to_json(val: &toml::Value) -> serde_json::Value {
    match val {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        toml::Value::Array(arr) => serde_json::Value::Array(arr.iter().map(toml_to_json).collect()),
        toml::Value::Table(tbl) => {
            let map = tbl
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

pub(crate) fn kimi_config_path() -> Option<PathBuf> {
    Some(kimi_home_dir()?.join("config.toml"))
}

pub(crate) fn kimi_mcp_config_path() -> Option<PathBuf> {
    Some(kimi_home_dir()?.join("mcp.json"))
}

fn kimi_home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("KIMI_CODE_HOME") {
        return Some(PathBuf::from(home));
    }
    let home = dirs::home_dir()?;
    Some(home.join(".kimi-code"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_model_provider_and_context() {
        let toml = r#"
default_model = "kimi-code/kimi-for-coding"

[models."kimi-code/kimi-for-coding"]
provider = "kimi-code"
model = "kimi-for-coding"
max_context_size = 200000

[providers.kimi-code]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
"#;
        let cfg = parse_kimi_config(toml).unwrap();
        assert_eq!(cfg.model.as_deref(), Some("kimi-code/kimi-for-coding"));
        assert_eq!(cfg.provider.as_deref(), Some("kimi-code"));
        assert_eq!(cfg.context_limit.as_deref(), Some("200000"));
        assert_eq!(
            cfg.extra.get("providers.kimi-code").map(String::as_str),
            Some("configured")
        );
        assert_eq!(
            cfg.extra
                .get("models.kimi-code/kimi-for-coding")
                .map(String::as_str),
            Some("configured")
        );
    }

    #[test]
    fn parse_provider_type_fallback_when_default_model_missing() {
        let toml = r#"
[providers.kimi-code]
type = "kimi"
"#;
        let cfg = parse_kimi_config(toml).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("kimi"));
        assert!(cfg.model.is_none());
    }

    #[test]
    fn invalid_toml_returns_none() {
        assert!(parse_kimi_config("{{{{not valid").is_none());
    }
}
