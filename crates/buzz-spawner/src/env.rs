//! Assemble the environment for a `buzz-acp` harness container.
//!
//! This mirrors the desktop's `spawn_agent_child`
//! (`desktop/src-tauri/src/managed_agents/runtime.rs`) so a VPS agent and a
//! laptop agent are configured identically — a Fizz on the server should differ
//! from a Fizz on a laptop only in where its process happens to run.

use buzz_sdk::spawner::{RespondTo, SpawnerAgentSpec};

use crate::store::AgentRecord;

/// Environment variable names the spawner owns outright.
///
/// Operator-supplied passthrough env (`BUZZ_SPAWNER_AGENT_ENV`) is filtered
/// against this list so a misconfigured compose file cannot hand an agent a
/// different identity, a different relay, or somebody else's attestation. This
/// mirrors the desktop's reserved-key strip in
/// `managed_agents/env_vars.rs`.
pub const RESERVED_KEYS: &[&str] = &[
    // Code-execution surface: which binary the harness spawns. Reserved so
    // operator passthrough cannot set it either — it comes from the spawner's
    // own `BUZZ_SPAWNER_AGENT_COMMAND`, applied below.
    "BUZZ_ACP_AGENT_COMMAND",
    "BUZZ_ACP_AGENT_ARGS",
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_API_TOKEN",
    "BUZZ_RELAY_URL",
    "BUZZ_ACP_PRIVATE_KEY",
    "BUZZ_ACP_API_TOKEN",
    "BUZZ_ACP_SETUP_PAYLOAD",
];

/// The resolved prompt material for an agent, gathered from its spec and the
/// persona it references.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedPrompt {
    /// The agent's system prompt.
    pub system_prompt: Option<String>,
    /// Team-level instructions appended after the system prompt.
    pub team_instructions: Option<String>,
    /// Model id, in `provider:model` or bare-model form.
    pub model: Option<String>,
    /// Inference provider id.
    pub provider: Option<String>,
}

/// Which ACP agent binary the harness runs inside the container.
///
/// Operator-supplied only. This selects what executes, so it deliberately has no
/// path from a kind:30178 spec — those are owner-authored and world-readable.
pub struct AgentRuntime<'a> {
    /// ACP agent binary, or `None` to keep the image default.
    pub command: Option<&'a str>,
    /// Comma-separated args for `command`.
    pub args: Option<&'a str>,
}

/// Env var names an owner-delivered credential displaces.
const OWNER_CREDENTIAL_KEYS: &[&str] = &["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"];

/// Build the full environment for an agent container.
///
/// `passthrough` is the operator-configured env (LLM credentials and the like).
/// It is applied *first* so the Buzz-owned variables below always win: an
/// operator cannot accidentally — or deliberately — override an agent's identity
/// by naming `BUZZ_PRIVATE_KEY` in the compose file.
///
/// `owner_credential` is the per-owner provider credential delivered over the
/// encrypted kind:24201 channel. When present it displaces any host-global
/// Anthropic credential in `passthrough` — each owner's agents bill against
/// their own token, never the operator's.
pub fn build_agent_env(
    record: &AgentRecord,
    spec: &SpawnerAgentSpec,
    prompt: &ResolvedPrompt,
    relay_url: &str,
    passthrough: &[(String, String)],
    runtime: &AgentRuntime<'_>,
    owner_credential: Option<&str>,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = passthrough
        .iter()
        .filter(|(k, _)| !RESERVED_KEYS.contains(&k.as_str()))
        .filter(|(k, _)| owner_credential.is_none() || !OWNER_CREDENTIAL_KEYS.contains(&k.as_str()))
        .cloned()
        .collect();

    if let Some(token) = owner_credential {
        env.push((
            crate::credentials::credential_env_key(token).to_string(),
            token.to_string(),
        ));
    }

    let mut set = |key: &str, value: String| env.push((key.to_string(), value));

    // Runtime selection, from operator config only — never from a spec.
    if let Some(command) = runtime.command {
        set("BUZZ_ACP_AGENT_COMMAND", command.to_string());
    }
    if let Some(args) = runtime.args {
        set("BUZZ_ACP_AGENT_ARGS", args.to_string());
    }

    // Identity and transport.
    set("BUZZ_PRIVATE_KEY", record.private_key_nsec.clone());
    set("BUZZ_RELAY_URL", relay_url.to_string());
    if let Some(auth_tag) = &record.auth_tag {
        set("BUZZ_AUTH_TAG", auth_tag.clone());
    }
    // git-credential-nostr signs NIP-98 git auth with the same key.
    set("NOSTR_PRIVATE_KEY", record.private_key_nsec.clone());

    // Prompt material.
    if let Some(system_prompt) = &prompt.system_prompt {
        set("BUZZ_ACP_SYSTEM_PROMPT", system_prompt.clone());
    }
    if let Some(team_instructions) = &prompt.team_instructions {
        set("BUZZ_ACP_TEAM_INSTRUCTIONS", team_instructions.clone());
    }
    // Two layers read these, and both must be satisfied. `BUZZ_ACP_MODEL` is
    // the harness's own setting; `BUZZ_AGENT_MODEL`/`BUZZ_AGENT_PROVIDER` are
    // what the `buzz-agent` binary requires of its own config, and it refuses
    // to start without them. The desktop makes the same mapping through its
    // runtime table (`managed_agents/discovery.rs`, `model_env_var` /
    // `provider_env_var`); a spawner has only one runtime, so it is direct.
    if let Some(model) = &prompt.model {
        set("BUZZ_ACP_MODEL", model.clone());
        set("BUZZ_AGENT_MODEL", model.clone());
    }
    if let Some(provider) = &prompt.provider {
        set("BUZZ_AGENT_PROVIDER", provider.clone());
    }

    // Behavior.
    set("BUZZ_ACP_AGENTS", spec.parallelism.to_string());
    set(
        "BUZZ_ACP_RESPOND_TO",
        match spec.respond_to {
            RespondTo::Anyone => "anyone",
            RespondTo::OwnerOnly => "owner-only",
            RespondTo::Allowlist => "allowlist",
        }
        .to_string(),
    );
    if spec.respond_to == RespondTo::Allowlist {
        set(
            "BUZZ_ACP_RESPOND_TO_ALLOWLIST",
            spec.respond_to_allowlist.join(","),
        );
    }
    set("BUZZ_ACP_RELAY_OBSERVER", "true".to_string());
    set("BUZZ_ACP_DEDUP", "queue".to_string());
    set("BUZZ_ACP_MULTIPLE_EVENT_HANDLING", "steer".to_string());

    // Provenance, so an operator inspecting a container can tell where it came
    // from without cross-referencing the state file.
    set("BUZZ_SPAWNED_BY", "buzz-spawner".to_string());
    set("BUZZ_SPAWNER_SPEC_SLUG", record.slug.clone());

    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> AgentRecord {
        AgentRecord {
            slug: "fizz-prod".into(),
            owner_pubkey: "b".repeat(64),
            agent_pubkey: "a".repeat(64),
            private_key_nsec: "nsec1realsecret".into(),
            auth_tag: Some(r#"["auth","owner","","sig"]"#.into()),
            pending_nonce: None,
            attestation_sent_at: None,
            spec_hash: None,
            prompt: None,
            restart_count: 0,
            last_failure_at: None,
            carried_team_instructions: None,
        }
    }

    fn spec() -> SpawnerAgentSpec {
        SpawnerAgentSpec {
            name: "Fizz".into(),
            agent_pubkey: None,
            persona_id: Some("builtin:fizz".into()),
            system_prompt: None,
            model: None,
            provider: None,
            parallelism: 2,
            respond_to: RespondTo::Anyone,
            respond_to_allowlist: vec![],
            resources: None,
            enabled: true,
        }
    }

    /// Image-default runtime: the spawner sets no override.
    const DEFAULT_RUNTIME: AgentRuntime<'static> = AgentRuntime {
        command: None,
        args: None,
    };

    fn lookup(env: &[(String, String)], key: &str) -> Option<String> {
        env.iter().rfind(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn sets_identity_prompt_and_behavior() {
        let prompt = ResolvedPrompt {
            system_prompt: Some("You are Fizz.".into()),
            team_instructions: None,
            model: Some("claude-opus-5".into()),
            provider: Some("anthropic".into()),
        };
        let env = build_agent_env(
            &record(),
            &spec(),
            &prompt,
            "wss://relay.example",
            &[],
            &DEFAULT_RUNTIME,
            None,
        );

        assert_eq!(
            lookup(&env, "BUZZ_PRIVATE_KEY").as_deref(),
            Some("nsec1realsecret")
        );
        assert_eq!(
            lookup(&env, "BUZZ_RELAY_URL").as_deref(),
            Some("wss://relay.example")
        );
        assert_eq!(
            lookup(&env, "BUZZ_ACP_SYSTEM_PROMPT").as_deref(),
            Some("You are Fizz.")
        );
        assert_eq!(
            lookup(&env, "BUZZ_ACP_MODEL").as_deref(),
            Some("claude-opus-5")
        );
        assert_eq!(lookup(&env, "BUZZ_ACP_AGENTS").as_deref(), Some("2"));
        assert_eq!(
            lookup(&env, "BUZZ_ACP_RESPOND_TO").as_deref(),
            Some("anyone")
        );
    }

    #[test]
    fn owner_credential_wins_over_host_global_passthrough() {
        let passthrough = vec![
            (
                "ANTHROPIC_API_KEY".to_string(),
                "sk-host-global".to_string(),
            ),
            (
                "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
                "sk-host-oauth".to_string(),
            ),
            ("OTHER_VAR".to_string(), "kept".to_string()),
        ];
        let env = build_agent_env(
            &record(),
            &spec(),
            &ResolvedPrompt::default(),
            "wss://r",
            &passthrough,
            &DEFAULT_RUNTIME,
            Some("sk-ant-oat01-owner"),
        );
        assert_eq!(
            lookup(&env, "CLAUDE_CODE_OAUTH_TOKEN").as_deref(),
            Some("sk-ant-oat01-owner")
        );
        // Host-global Anthropic credentials are fully displaced, not shadowed.
        assert!(lookup(&env, "ANTHROPIC_API_KEY").is_none());
        assert!(!env.iter().any(|(_, v)| v.starts_with("sk-host")));
        assert_eq!(lookup(&env, "OTHER_VAR").as_deref(), Some("kept"));
    }

    #[test]
    fn owner_api_key_is_injected_under_the_api_key_var() {
        let env = build_agent_env(
            &record(),
            &spec(),
            &ResolvedPrompt::default(),
            "wss://r",
            &[],
            &DEFAULT_RUNTIME,
            Some("sk-ant-api03-owner"),
        );
        assert_eq!(
            lookup(&env, "ANTHROPIC_API_KEY").as_deref(),
            Some("sk-ant-api03-owner")
        );
        assert!(lookup(&env, "CLAUDE_CODE_OAUTH_TOKEN").is_none());
    }

    #[test]
    fn without_an_owner_credential_passthrough_flows_unchanged() {
        let passthrough = vec![("ANTHROPIC_API_KEY".to_string(), "sk-host".to_string())];
        let env = build_agent_env(
            &record(),
            &spec(),
            &ResolvedPrompt::default(),
            "wss://r",
            &passthrough,
            &DEFAULT_RUNTIME,
            None,
        );
        assert_eq!(
            lookup(&env, "ANTHROPIC_API_KEY").as_deref(),
            Some("sk-host")
        );
    }

    #[test]
    fn passthrough_cannot_override_reserved_keys() {
        // A compose file naming BUZZ_PRIVATE_KEY must not be able to hand an
        // agent a different identity, nor swap the relay out from under it.
        let passthrough = vec![
            ("ANTHROPIC_API_KEY".to_string(), "sk-real".to_string()),
            ("BUZZ_PRIVATE_KEY".to_string(), "nsec1attacker".to_string()),
            (
                "BUZZ_RELAY_URL".to_string(),
                "wss://evil.example".to_string(),
            ),
            (
                "BUZZ_AUTH_TAG".to_string(),
                "[\"auth\",\"attacker\"]".to_string(),
            ),
        ];
        let env = build_agent_env(
            &record(),
            &spec(),
            &ResolvedPrompt::default(),
            "wss://relay.example",
            &passthrough,
            &DEFAULT_RUNTIME,
            None,
        );

        assert_eq!(
            lookup(&env, "ANTHROPIC_API_KEY").as_deref(),
            Some("sk-real")
        );
        assert_eq!(
            lookup(&env, "BUZZ_PRIVATE_KEY").as_deref(),
            Some("nsec1realsecret")
        );
        assert_eq!(
            lookup(&env, "BUZZ_RELAY_URL").as_deref(),
            Some("wss://relay.example")
        );
        assert!(!env
            .iter()
            .any(|(_, v)| v.contains("attacker") || v.contains("evil.example")));
    }

    #[test]
    fn runtime_override_selects_the_agent_binary() {
        let env = build_agent_env(
            &record(),
            &spec(),
            &ResolvedPrompt::default(),
            "wss://r",
            &[],
            &AgentRuntime {
                command: Some("claude-agent-acp"),
                args: None,
            },
            None,
        );
        assert_eq!(
            lookup(&env, "BUZZ_ACP_AGENT_COMMAND").as_deref(),
            Some("claude-agent-acp")
        );
    }

    #[test]
    fn leaves_the_image_default_when_no_runtime_is_configured() {
        let env = build_agent_env(
            &record(),
            &spec(),
            &ResolvedPrompt::default(),
            "wss://r",
            &[],
            &DEFAULT_RUNTIME,
            None,
        );
        assert!(lookup(&env, "BUZZ_ACP_AGENT_COMMAND").is_none());
    }

    #[test]
    fn passthrough_cannot_choose_the_agent_binary() {
        // The binary the harness spawns is a code-execution surface. Operator
        // passthrough goes through the same reserved-key filter as everything
        // else, so it has to come from BUZZ_SPAWNER_AGENT_COMMAND.
        let passthrough = vec![("BUZZ_ACP_AGENT_COMMAND".to_string(), "/bin/sh".to_string())];
        let env = build_agent_env(
            &record(),
            &spec(),
            &ResolvedPrompt::default(),
            "wss://r",
            &passthrough,
            &AgentRuntime {
                command: Some("claude-agent-acp"),
                args: None,
            },
            None,
        );
        assert_eq!(
            lookup(&env, "BUZZ_ACP_AGENT_COMMAND").as_deref(),
            Some("claude-agent-acp")
        );
        assert!(!env.iter().any(|(_, v)| v == "/bin/sh"));
    }

    #[test]
    fn omits_the_auth_tag_until_attested() {
        let mut record = record();
        record.auth_tag = None;
        let env = build_agent_env(
            &record,
            &spec(),
            &ResolvedPrompt::default(),
            "wss://relay.example",
            &[],
            &DEFAULT_RUNTIME,
            None,
        );
        assert!(lookup(&env, "BUZZ_AUTH_TAG").is_none());
    }

    #[test]
    fn emits_the_allowlist_only_when_gated_on_it() {
        let mut spec = spec();
        let env = build_agent_env(
            &record(),
            &spec,
            &ResolvedPrompt::default(),
            "wss://r",
            &[],
            &DEFAULT_RUNTIME,
            None,
        );
        assert!(lookup(&env, "BUZZ_ACP_RESPOND_TO_ALLOWLIST").is_none());

        spec.respond_to = RespondTo::Allowlist;
        spec.respond_to_allowlist = vec!["c".repeat(64), "d".repeat(64)];
        let env = build_agent_env(
            &record(),
            &spec,
            &ResolvedPrompt::default(),
            "wss://r",
            &[],
            &DEFAULT_RUNTIME,
            None,
        );
        assert_eq!(
            lookup(&env, "BUZZ_ACP_RESPOND_TO_ALLOWLIST"),
            Some(format!("{}{}{}", "c".repeat(64), ",", "d".repeat(64)))
        );
    }
}
