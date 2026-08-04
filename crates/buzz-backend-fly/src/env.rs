use crate::config::ProviderConfig;
use crate::wire::{AgentPayload, LaunchBlock};
use std::collections::BTreeMap;

const AUTHORITATIVE_KEYS: &[&str] = &[
    "BUZZ_RELAY_URL",
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_ACP_AGENT_OWNER",
    "BUZZ_ACP_AGENT_COMMAND",
    "BUZZ_ACP_AGENT_ARGS",
    "BUZZ_ACP_RESPOND_TO",
    "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
    "BUZZ_ACP_MCP_COMMAND",
    "BUZZ_ACP_MCP_SERVERS",
];

const HOST_ONLY_KEYS: &[&str] = &[
    "PATH",
    "CLAUDE_CODE_EXECUTABLE",
    "BUZZ_ACP_SETUP_PAYLOAD",
    "BUZZ_MANAGED_AGENT",
    "BUZZ_DESKTOP_MODEL_OVERRIDE",
    "BUZZ_DESKTOP_PROVIDER_OVERRIDE",
];

const MAX_ENV_VALUE_BYTES: usize = 1024 * 1024;

fn valid_env_name(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn identity_component(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

pub fn build_env(
    agent: &AgentPayload,
    config: &ProviderConfig,
) -> Result<BTreeMap<String, String>, String> {
    let fallback = LaunchBlock::default();
    let launch = agent.launch.as_ref().unwrap_or(&fallback);

    let mut env = launch.policy_env.clone();
    if agent.launch.is_some() {
        env.extend(launch.env.clone());
    } else {
        env.extend(agent.env_vars.clone());
    }
    for key in HOST_ONLY_KEYS {
        env.remove(*key);
    }
    for (key, value) in &env {
        if !valid_env_name(key) {
            return Err(format!(
                "agent environment key {key:?} is not a valid environment variable name"
            ));
        }
        if value.contains('\0') {
            return Err(format!("agent environment value for {key:?} contains NUL"));
        }
        if key.eq_ignore_ascii_case("BUZZ_ACP_NO_PRESENCE") {
            return Err(
                "BUZZ_ACP_NO_PRESENCE cannot be set for a remote agent because relay presence is its liveness signal"
                    .to_string(),
            );
        }
    }

    for key in AUTHORITATIVE_KEYS {
        env.remove(*key);
    }
    let relay_url = identity_component(&agent.relay_url)
        .ok_or_else(|| "deploy refused: relay_url is empty".to_string())?;
    env.insert("BUZZ_RELAY_URL".into(), relay_url.to_string());
    env.insert("BUZZ_PRIVATE_KEY".into(), agent.private_key_nsec.clone());
    env.insert("NOSTR_PRIVATE_KEY".into(), agent.private_key_nsec.clone());

    let auth_tag = agent.auth_tag.as_deref().and_then(identity_component);
    let owner = launch.owner_pubkey.as_deref().and_then(identity_component);
    if auth_tag.is_none() && owner.is_none() {
        return Err(
            "deploy refused: neither auth_tag nor launch.owner_pubkey resolved; the agent could not honor !shutdown"
                .to_string(),
        );
    }
    if let Some(value) = auth_tag {
        env.insert("BUZZ_AUTH_TAG".into(), value.to_string());
    }
    if let Some(value) = owner {
        env.insert("BUZZ_ACP_AGENT_OWNER".into(), value.to_string());
    }

    let command = launch
        .command
        .as_deref()
        .and_then(identity_component)
        .unwrap_or("buzz-agent");
    let known_buzz_agent_image = config.image.starts_with("ghcr.io/block/buzz-sprig@")
        || config.image == crate::config::DEFAULT_IMAGE;
    if known_buzz_agent_image && command != "buzz-agent" {
        return Err(format!(
            "the default Sprig image contains the buzz-agent runtime, but this agent resolves to {command:?}; select Buzz Agent or provide a digest-pinned custom image containing that command"
        ));
    }
    if command == "buzz-agent" {
        validate_buzz_agent_config(&env)?;
    }
    env.insert("BUZZ_ACP_AGENT_COMMAND".into(), command.to_string());
    if !launch.args.is_empty() {
        if launch.args.iter().any(|argument| argument.contains(',')) {
            return Err(
                "agent arguments containing ',' cannot be represented by BUZZ_ACP_AGENT_ARGS"
                    .to_string(),
            );
        }
        env.insert("BUZZ_ACP_AGENT_ARGS".into(), launch.args.join(","));
    }
    env.insert("BUZZ_ACP_MCP_COMMAND".into(), "buzz-dev-mcp".into());

    if let Some(value) = agent.respond_to.as_deref().and_then(identity_component) {
        env.insert("BUZZ_ACP_RESPOND_TO".into(), value.to_string());
    }
    if let Some(values) = agent
        .respond_to_allowlist
        .as_ref()
        .filter(|values| !values.is_empty())
    {
        env.insert("BUZZ_ACP_RESPOND_TO_ALLOWLIST".into(), values.join(","));
    }

    let total_value_bytes: usize = env.values().map(String::len).sum();
    if total_value_bytes > MAX_ENV_VALUE_BYTES {
        return Err(format!(
            "agent environment contains {total_value_bytes} value bytes; the Fly secret import limit for this provider is {MAX_ENV_VALUE_BYTES}"
        ));
    }

    Ok(env)
}

fn validate_buzz_agent_config(env: &BTreeMap<String, String>) -> Result<(), String> {
    let value = |key: &str| {
        env.get(key)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
    };
    let provider = value("BUZZ_AGENT_PROVIDER").ok_or_else(|| {
        "deploy refused: Buzz Agent has no LLM provider; edit the agent in Buzz and choose an LLM provider before starting it on Fly"
            .to_string()
    })?;
    if value("BUZZ_AGENT_MODEL").is_none() {
        return Err(
            "deploy refused: Buzz Agent has no model; edit the agent in Buzz and choose a model before starting it on Fly"
                .to_string(),
        );
    }

    let required_key = match provider {
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "openai" | "openai-compat" => Some("OPENAI_COMPAT_API_KEY"),
        "openrouter" => Some("OPENROUTER_API_KEY"),
        "databricks" | "databricks_v2" | "databricks-v2" => Some("DATABRICKS_HOST"),
        _ => None,
    };
    if let Some(key) = required_key.filter(|key| value(key).is_none()) {
        return Err(format!(
            "deploy refused: provider {provider:?} requires {key}; add it through the agent's Buzz credential field before starting it on Fly"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ProviderConfig {
        crate::config::parse(&serde_json::json!({})).unwrap()
    }

    fn agent() -> AgentPayload {
        serde_json::from_value(serde_json::json!({
            "relay_url":"wss://relay.example",
            "private_key_nsec":"nsec1example",
            "auth_tag":"owner-attestation",
            "launch":{
                "command":"buzz-agent",
                "env":{
                    "BUZZ_AGENT_PROVIDER":"openai",
                    "BUZZ_AGENT_MODEL":"gpt-test",
                    "OPENAI_COMPAT_API_KEY":"scoped"
                },
                "policy_env":{"BUZZ_ACP_AGENTS":"1"}
            }
        }))
        .unwrap()
    }

    #[test]
    fn authoritative_identity_overwrites_user_values() {
        let mut agent = agent();
        agent
            .launch
            .as_mut()
            .unwrap()
            .env
            .insert("BUZZ_PRIVATE_KEY".into(), "wrong".into());
        let env = build_env(&agent, &config()).unwrap();
        assert_eq!(env["BUZZ_PRIVATE_KEY"], "nsec1example");
        assert_eq!(env["OPENAI_COMPAT_API_KEY"], "scoped");
    }

    #[test]
    fn default_image_refuses_missing_runtime() {
        let mut agent = agent();
        agent.launch.as_mut().unwrap().command = Some("codex-acp".into());
        let error = build_env(&agent, &config()).unwrap_err();
        assert!(error.contains("contains the buzz-agent runtime"));
    }

    #[test]
    fn refuses_ownerless_agent() {
        let mut agent = agent();
        agent.auth_tag = None;
        agent.launch.as_mut().unwrap().owner_pubkey = None;
        let error = build_env(&agent, &config()).unwrap_err();
        assert!(error.contains("neither auth_tag"));
    }

    #[test]
    fn refuses_oversized_secret_payload() {
        let mut agent = agent();
        agent
            .launch
            .as_mut()
            .unwrap()
            .env
            .insert("OVERSIZED".into(), "x".repeat(MAX_ENV_VALUE_BYTES + 1));
        let error = build_env(&agent, &config()).unwrap_err();
        assert!(error.contains("Fly secret import limit"));
    }

    #[test]
    fn refuses_buzz_agent_without_provider_before_fly_access() {
        let mut agent = agent();
        agent
            .launch
            .as_mut()
            .unwrap()
            .env
            .remove("BUZZ_AGENT_PROVIDER");
        let error = build_env(&agent, &config()).unwrap_err();
        assert!(error.contains("no LLM provider"));
    }

    #[test]
    fn refuses_buzz_agent_without_model_before_fly_access() {
        let mut agent = agent();
        agent
            .launch
            .as_mut()
            .unwrap()
            .env
            .remove("BUZZ_AGENT_MODEL");
        let error = build_env(&agent, &config()).unwrap_err();
        assert!(error.contains("no model"));
    }

    #[test]
    fn refuses_buzz_agent_without_provider_credential_before_fly_access() {
        let mut agent = agent();
        agent
            .launch
            .as_mut()
            .unwrap()
            .env
            .remove("OPENAI_COMPAT_API_KEY");
        let error = build_env(&agent, &config()).unwrap_err();
        assert!(error.contains("requires OPENAI_COMPAT_API_KEY"));
    }

    #[test]
    fn drops_agent_owned_mcp_configuration() {
        let mut agent = agent();
        agent.launch.as_mut().unwrap().env.insert(
            "BUZZ_ACP_MCP_SERVERS".into(),
            r#"[{"name":"crm","command":"mcp-remote","args":["https://mcp.example"]}]"#.into(),
        );
        let env = build_env(&agent, &config()).unwrap();
        assert!(!env.contains_key("BUZZ_ACP_MCP_SERVERS"));
    }
}
