use super::types::{ExtensionEntry, RuntimeFileConfig};
use std::collections::BTreeMap;

/// Read Codex config from `~/.codex/config.toml` (or `$CODEX_HOME/config.toml`).
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let path = codex_config_path(&BTreeMap::new())?;
    let raw = std::fs::read_to_string(path).ok()?;
    parse_codex_config(&raw)
}

/// Return whether the active custom Codex provider has a usable env-backed
/// credential. Codex accepts provider API keys through
/// `[model_providers.<id>] env_key`, but `codex login status` only checks its
/// persisted credential store. Buzz must account for both authentication
/// paths before deciding that the runtime is logged out.
pub(crate) fn env_key_auth_satisfied(effective_env: &BTreeMap<String, String>) -> bool {
    let Some(env_key) = read_active_provider_env_key(effective_env) else {
        return false;
    };

    env_key_value_is_set(effective_env, &env_key)
}

fn env_key_value_is_set(effective_env: &BTreeMap<String, String>, env_key: &str) -> bool {
    env_key_value_is_set_from(effective_env, env_key, std::env::var_os(env_key))
}

fn env_key_value_is_set_from(
    effective_env: &BTreeMap<String, String>,
    env_key: &str,
    process_value: Option<std::ffi::OsString>,
) -> bool {
    match effective_env.get(env_key) {
        Some(value) => !value.is_empty(),
        None => process_value.is_some_and(|value| !value.is_empty()),
    }
}

fn read_active_provider_env_key(effective_env: &BTreeMap<String, String>) -> Option<String> {
    let path = codex_config_path(effective_env)?;
    let raw = std::fs::read_to_string(path).ok()?;
    parse_active_provider_env_key(&raw)
}

fn parse_active_provider_env_key(toml_str: &str) -> Option<String> {
    let table: toml::Table = toml_str.parse().ok()?;
    let provider = toml_string(&table, "model_provider")?;
    let env_key = table
        .get("model_providers")?
        .as_table()?
        .get(&provider)?
        .as_table()?
        .get("env_key")?
        .as_str()?
        .trim();

    if crate::managed_agents::env_vars::is_well_formed_env_key(env_key) {
        Some(env_key.to_string())
    } else {
        None
    }
}

fn parse_codex_config(toml_str: &str) -> Option<RuntimeFileConfig> {
    let table: toml::Table = toml_str.parse().ok()?;

    let model = toml_string(&table, "model");
    let model_provider = toml_string(&table, "model_provider");
    let approval_policy = toml_string(&table, "approval_policy");
    let sandbox_mode = toml_string(&table, "sandbox_mode");
    let reasoning_effort = toml_string(&table, "model_reasoning_effort");
    let context_window = toml_string(&table, "model_context_window");

    // Two-axis mode: approval_policy × sandbox_mode
    let mode = match (approval_policy.as_deref(), sandbox_mode.as_deref()) {
        (Some(ap), Some(sm)) => Some(format!("{ap}/{sm}")),
        (Some(ap), None) => Some(ap.to_string()),
        (None, Some(sm)) => Some(format!("default/{sm}")),
        (None, None) => None,
    };

    // MCP servers from [mcp_servers.<id>] tables
    let extensions = parse_mcp_servers(&table);

    // Config-driven extra fields — skip normalized keys to avoid double-counting.
    // The skip list covers fields extracted into typed struct fields above.
    let config_json = toml_to_json(&toml::Value::Table(table));
    let skip = &[
        "model",
        "model_provider",
        "approval_policy",
        "sandbox_mode",
        "model_reasoning_effort",
        "model_context_window",
        "instructions",
        "mcp_servers",
        "model_providers",
    ];
    let mut extra = super::schema_walker::extract_config_fields(&config_json, skip);

    // Custom model providers from [model_providers.<id>] — surface as
    // "model_providers.<name> = configured" rather than flattening their internals.
    if let Some(serde_json::Value::Object(providers)) = config_json.get("model_providers") {
        for (name, _) in providers {
            extra.insert(format!("model_providers.{name}"), "configured".to_string());
        }
    }

    Some(RuntimeFileConfig {
        model,
        // Default to OpenAI when no provider is configured — that is Codex's
        // implicit provider when model_provider is absent.
        provider: model_provider.or_else(|| Some("openai".to_string())),
        mode,
        thinking_effort: reasoning_effort,
        max_output_tokens: None,
        context_limit: context_window,
        system_prompt: toml_to_json_string(&config_json, "instructions"),
        extensions,
        extra,
    })
}

fn parse_mcp_servers(table: &toml::Table) -> Vec<ExtensionEntry> {
    let servers = match table.get("mcp_servers").and_then(|v| v.as_table()) {
        Some(s) => s,
        None => return Vec::new(),
    };

    servers
        .iter()
        .map(|(name, _config)| ExtensionEntry {
            name: name.clone(),
            kind: "mcp".to_string(),
            enabled: true,
        })
        .collect()
}

/// Convert a TOML value to a serde_json Value.
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

/// Extract a string value from a JSON object by key (mirrors toml_string).
fn toml_to_json_string(val: &serde_json::Value, key: &str) -> Option<String> {
    val.get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn toml_string(table: &toml::Table, key: &str) -> Option<String> {
    table
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn codex_config_path(
    effective_env: &BTreeMap<String, String>,
) -> Option<std::path::PathBuf> {
    codex_config_path_from(
        effective_env,
        std::env::var_os("CODEX_HOME"),
        dirs::home_dir(),
    )
}

fn codex_config_path_from(
    effective_env: &BTreeMap<String, String>,
    process_codex_home: Option<std::ffi::OsString>,
    default_home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let root = match effective_env.get("CODEX_HOME") {
        Some(home) => std::path::PathBuf::from(home),
        None => process_codex_home
            .map(std::path::PathBuf::from)
            .or_else(|| default_home.map(|home| home.join(".codex")))?,
    };
    Some(root.join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_config() {
        let toml = r#"
model = "o3"
model_provider = "openai"
approval_policy = "unless-allow-listed"
sandbox_mode = "permissive"
model_reasoning_effort = "high"
"#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.model.as_deref(), Some("o3"));
        assert_eq!(cfg.provider.as_deref(), Some("openai"));
        assert_eq!(cfg.mode.as_deref(), Some("unless-allow-listed/permissive"));
        assert_eq!(cfg.thinking_effort.as_deref(), Some("high"));
    }

    #[test]
    fn parse_mcp_servers() {
        let toml = r#"
model = "gpt-4.1"

[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@anthropic-ai/mcp-filesystem"]

[mcp_servers.github]
command = "gh"
"#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.extensions.len(), 2);
    }

    #[test]
    fn parse_custom_providers() {
        let toml = r#"
model = "my-model"
model_provider = "custom-provider"

[model_providers.custom-provider]
base_url = "http://localhost:8080"
"#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("custom-provider"));
        assert!(cfg.extra.contains_key("model_providers.custom-provider"));
    }

    #[test]
    fn parses_active_custom_provider_env_key() {
        let toml = r#"
model_provider = "custom-provider"

[model_providers.custom-provider]
env_key = "CUSTOM_API_KEY"
"#;

        assert_eq!(
            parse_active_provider_env_key(toml).as_deref(),
            Some("CUSTOM_API_KEY")
        );
    }

    #[test]
    fn ignores_env_key_for_inactive_provider() {
        let toml = r#"
model_provider = "active-provider"

[model_providers.inactive-provider]
env_key = "INACTIVE_API_KEY"
"#;

        assert_eq!(parse_active_provider_env_key(toml), None);
    }

    #[test]
    fn rejects_malformed_provider_env_key() {
        let toml = r#"
model_provider = "custom-provider"

[model_providers.custom-provider]
env_key = "CUSTOM_API_KEY=secret"
"#;

        assert_eq!(parse_active_provider_env_key(toml), None);
    }

    #[test]
    fn accepts_non_empty_effective_env_credential() {
        let env = BTreeMap::from([("BUZZ_TEST_CODEX_API_KEY".to_string(), "secret".to_string())]);

        assert!(env_key_value_is_set(&env, "BUZZ_TEST_CODEX_API_KEY"));
    }

    #[test]
    fn rejects_empty_effective_env_credential() {
        let env = BTreeMap::from([("BUZZ_TEST_CODEX_API_KEY".to_string(), String::new())]);

        assert!(!env_key_value_is_set_from(
            &env,
            "BUZZ_TEST_CODEX_API_KEY",
            Some("parent-secret".into())
        ));
    }

    #[test]
    fn effective_codex_home_wins_over_process_home() {
        let env = BTreeMap::from([("CODEX_HOME".to_string(), "/child/codex".to_string())]);

        assert_eq!(
            codex_config_path_from(
                &env,
                Some("/desktop/codex".into()),
                Some("/user".into())
            ),
            Some(std::path::PathBuf::from("/child/codex/config.toml"))
        );
    }

    #[test]
    fn process_codex_home_is_used_when_effective_env_omits_it() {
        assert_eq!(
            codex_config_path_from(
                &BTreeMap::new(),
                Some("/desktop/codex".into()),
                Some("/user".into())
            ),
            Some(std::path::PathBuf::from("/desktop/codex/config.toml"))
        );
    }

    #[test]
    fn env_auth_reads_config_and_credential_from_effective_env() {
        let codex_home = tempfile::tempdir().expect("temp CODEX_HOME");
        std::fs::write(
            codex_home.path().join("config.toml"),
            r#"model_provider = "custom"

[model_providers.custom]
env_key = "CUSTOM_API_KEY"
"#,
        )
        .expect("write config");
        let env = BTreeMap::from([
            (
                "CODEX_HOME".to_string(),
                codex_home.path().to_string_lossy().into_owned(),
            ),
            ("CUSTOM_API_KEY".to_string(), "secret".to_string()),
        ]);

        assert!(env_key_auth_satisfied(&env));
    }

    #[test]
    fn approval_only_mode() {
        let toml = r#"approval_policy = "on-failure""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.mode.as_deref(), Some("on-failure"));
    }

    #[test]
    fn sandbox_only_mode() {
        let toml = r#"sandbox_mode = "strict""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.mode.as_deref(), Some("default/strict"));
    }

    #[test]
    fn empty_config() {
        let cfg = parse_codex_config("").unwrap();
        assert!(cfg.model.is_none());
        // No model_provider → defaults to openai
        assert_eq!(cfg.provider.as_deref(), Some("openai"));
        assert!(cfg.mode.is_none());
    }

    #[test]
    fn explicit_provider_wins_over_default() {
        let toml = r#"model_provider = "azure""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(cfg.provider.as_deref(), Some("azure"));
    }

    #[test]
    fn invalid_toml_returns_none() {
        assert!(parse_codex_config("{{{{not valid").is_none());
    }

    #[test]
    fn extra_contains_fast_mode_from_features() {
        // features is a nested table — walker flattens to features.fast_mode
        let toml = r#"
model = "o3"

[features]
fast_mode = true
"#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(
            cfg.extra.get("features.fast_mode").map(|s| s.as_str()),
            Some("true"),
            "features.fast_mode should appear in extra as 'true'"
        );
    }

    #[test]
    fn extra_contains_service_tier() {
        let toml = r#"service_tier = "flex""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(
            cfg.extra.get("service_tier").map(|s| s.as_str()),
            Some("flex"),
            "service_tier should appear in extra"
        );
    }

    #[test]
    fn extra_contains_plan_mode_reasoning_effort() {
        let toml = r#"plan_mode_reasoning_effort = "medium""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(
            cfg.extra
                .get("plan_mode_reasoning_effort")
                .map(|s| s.as_str()),
            Some("medium"),
            "plan_mode_reasoning_effort should appear in extra"
        );
    }

    #[test]
    fn extra_contains_model_reasoning_summary() {
        let toml = r#"model_reasoning_summary = "auto""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(
            cfg.extra.get("model_reasoning_summary").map(|s| s.as_str()),
            Some("auto"),
            "model_reasoning_summary should appear in extra"
        );
    }

    #[test]
    fn extra_contains_unknown_future_field() {
        // Config-driven: any key the user has set appears, even if we've never
        // heard of it. This is the core benefit of the config-driven approach.
        let toml = r#"some_new_codex_field = "value""#;
        let cfg = parse_codex_config(toml).unwrap();
        assert_eq!(
            cfg.extra.get("some_new_codex_field").map(|s| s.as_str()),
            Some("value"),
            "unknown future fields should appear in extra"
        );
    }

    #[test]
    fn normalized_fields_not_duplicated_in_extra() {
        let toml = r#"
model = "o3"
model_provider = "openai"
approval_policy = "unless-allow-listed"
sandbox_mode = "permissive"
model_reasoning_effort = "high"
model_context_window = "128000"
instructions = "You are helpful."
"#;
        let cfg = parse_codex_config(toml).unwrap();
        // None of the normalized/skip fields should appear in extra
        for key in &[
            "model",
            "model_provider",
            "approval_policy",
            "sandbox_mode",
            "model_reasoning_effort",
            "model_context_window",
            "instructions",
        ] {
            assert!(
                !cfg.extra.contains_key(*key),
                "normalized field '{key}' should not appear in extra"
            );
        }
    }
}
