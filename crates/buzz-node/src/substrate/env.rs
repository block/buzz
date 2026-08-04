//! Shared harness environment contract for workload substrates.
//!
//! Both the process substrate and the Docker substrate launch the same sprig
//! ACP harness (`buzz-acp`) and hand it the environment described by the
//! workload's [`LaunchSpec`] — the contract Desktop resolves once and every
//! execution body consumes. This module owns the layering so the two
//! substrates cannot drift: policy environment below, the authoritative
//! layered environment above it (user values win, mirroring local spawn),
//! node-controlled identity and transport values on top. How those variables
//! reach the body — `Command::env` for a child process, an env-file for
//! `docker run` — stays substrate-local, as does resolving the contract's
//! command *names* to executables.

use std::collections::BTreeMap;

use buzz_core::execution::{AgentWorkloadContext, WorkloadSpec};
use zeroize::Zeroizing;

/// LLM provider credentials and endpoints forwarded from the node operator's
/// own environment into every workload body.
///
/// This is a deliberate allowlist, not blanket inheritance: provider
/// credentials are node-operator environment — never part of the workload
/// spec ("keep secrets out of configuration"). Desktop strips the same names
/// from the launch contract at the node boundary
/// (`LaunchSpec::without_provider_credentials`); sharing the list through
/// `buzz-core` keeps the strip and this forward from drifting.
pub(crate) const PROVIDER_ENV: &[&str] = buzz_core::execution::PROVIDER_CREDENTIAL_ENV;

/// Variables the node controls regardless of what the launch contract says.
///
/// The contract arrives over the relay from the verified owner, but identity,
/// transport, and launch-key material are the node's to set: a wire value for
/// any of these is dropped before the node writes its own. `GIT_CONFIG_*` is
/// prefix-matched because the git credential-helper block owns that family.
const RESERVED_ENV: &[&str] = &[
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_RELAY_URL",
    "BUZZ_AUTH_TAG",
    "BUZZ_ACP_AGENT_OWNER",
    "BUZZ_ACP_SETUP_PAYLOAD",
    "BUZZ_ACP_AGENT_COMMAND",
    "BUZZ_ACP_AGENT_ARGS",
    "BUZZ_ACP_MCP_COMMAND",
    "BUZZ_ACP_EXIT_AFTER_INACTIVITY",
];

fn reserved(name: &str) -> bool {
    RESERVED_ENV.contains(&name) || name.starts_with("GIT_CONFIG_")
}

/// Substrate identity and image details for one known runtime identifier.
///
/// This is all that remains of the node-side runtime catalog: which agent
/// body image variant carries a runtime, and whether its adapter needs a
/// `claude` CLI pointer. Commands, arguments, and every environment value
/// come from the workload's launch contract — the node never reconstructs
/// them from the runtime identifier.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct KnownRuntime {
    /// Whether to point the Claude adapter at a `claude` CLI through
    /// `CLAUDE_CODE_EXECUTABLE`. Substrate-appropriate: the process substrate
    /// resolves a host path, the Docker substrate the in-image install path.
    pub wants_claude_cli: bool,
    /// Agent-body image variant carrying this runtime, when it is not part
    /// of the slim image (`Dockerfile.agent` builds one image per runtime,
    /// selected by its `RUNTIME` build arg; `just agent-image <variant>`
    /// tags it `buzz-agent:<variant>`). `None` means the slim image already
    /// carries the runtime (the sprig personalities) or the runtime is
    /// unknown and runs whatever image the operator configured.
    pub image_variant: Option<&'static str>,
}

/// Look up a normalized (trimmed, lowercased) runtime identifier. Unknown
/// identifiers get the defaults: configured image, no CLI pointer.
pub(crate) fn known_runtime(normalized: &str) -> KnownRuntime {
    match normalized {
        "goose" => KnownRuntime {
            wants_claude_cli: false,
            image_variant: Some("goose"),
        },
        "claude" | "claude-code" | "claudecode" | "claude-agent-acp" | "claude-code-acp" => {
            KnownRuntime {
                wants_claude_cli: true,
                image_variant: Some("claude"),
            }
        }
        "codex" | "codex-acp" => KnownRuntime {
            wants_claude_cli: false,
            image_variant: Some("codex"),
        },
        _ => KnownRuntime::default(),
    }
}

/// The launch contract's command names, adapted by the substrate: host
/// executable paths for the process substrate, in-image command names for the
/// Docker substrate. Either way the harness contract is identical.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolvedCommands<'a> {
    /// Inner agent command the harness runs (`BUZZ_ACP_AGENT_COMMAND`).
    pub agent_command: &'a str,
    /// Developer MCP command, when the contract carries one.
    pub mcp_command: Option<&'a str>,
}

/// Build the harness environment for one workload body.
///
/// Layering, lowest to highest:
/// 1. `launch.policy_env` — Buzz-set policy (harness contract, prompts,
///    timeouts, audience gate), resolved by Desktop.
/// 2. `launch.env` — the authoritative layered environment; user values win
///    over policy, mirroring the desktop launcher.
/// 3. Node-controlled values — the one-time key handoff, relay binding, and
///    the substrate-resolved commands. Wire values for these are dropped
///    (see [`RESERVED_ENV`]).
///
/// The returned set includes the one-time launch key (`BUZZ_PRIVATE_KEY`), so
/// every entry is zeroized on drop and the whole set must never be persisted
/// or logged.
pub(crate) fn harness_environment(
    spec: &WorkloadSpec,
    agent: &AgentWorkloadContext,
    owner: &str,
    launch_key: &str,
    relay_url: &str,
    resolved: &ResolvedCommands<'_>,
    inactivity_seconds: u64,
) -> Vec<(String, Zeroizing<String>)> {
    let mut env: BTreeMap<String, Zeroizing<String>> = BTreeMap::new();
    fn set(env: &mut BTreeMap<String, Zeroizing<String>>, name: &str, value: &str) {
        env.insert(name.to_string(), Zeroizing::new(value.to_string()));
    }

    // ── The resolved contract: policy below, layered env above. ─────────────
    for (name, value) in &spec.launch.policy_env {
        if !reserved(name) {
            set(&mut env, name, value);
        }
    }
    for (name, value) in &spec.launch.env {
        if !reserved(name) {
            set(&mut env, name, value);
        }
    }
    // Contracts resolved from a blank display name carry no session title;
    // fall back to the workload's own name rather than an untitled session.
    env.entry("BUZZ_ACP_SESSION_TITLE".to_string())
        .or_insert_with(|| Zeroizing::new(spec.display_name.clone()));

    // ── Identity and relay: the one-time key handoff. ───────────────────────
    set(&mut env, "BUZZ_PRIVATE_KEY", launch_key);
    set(&mut env, "BUZZ_RELAY_URL", relay_url);
    if let Some(auth_tag) = &agent.auth_tag {
        set(&mut env, "BUZZ_AUTH_TAG", auth_tag);
    } else {
        // A body with no resolvable owner cannot match `!shutdown` and drops
        // everything under `respond_to=owner-only`, so when there is no
        // NIP-OA attestation to derive the owner from, hand the harness the
        // verified command signer through its own fallback
        // (`BUZZ_ACP_AGENT_OWNER`, `resolve_agent_owner` in
        // crates/buzz-acp/src/lib.rs).
        set(&mut env, "BUZZ_ACP_AGENT_OWNER", owner);
    }

    // ── Substrate-resolved commands from the contract. ──────────────────────
    set(&mut env, "BUZZ_ACP_AGENT_COMMAND", resolved.agent_command);
    set(&mut env, "BUZZ_ACP_AGENT_ARGS", &spec.launch.args.join(","));
    set(
        &mut env,
        "BUZZ_ACP_MCP_COMMAND",
        resolved.mcp_command.unwrap_or(""),
    );

    // Inactivity self-termination (the spec's I5 enforcement point,
    // docs/remote-agents.md §Auto-Stop): remote bodies opt IN, and a node is
    // remote by definition. `0` is the legal "no inactivity bound" — the
    // harness default already means disabled, so the variable is omitted
    // instead of set to 0.
    if inactivity_seconds > 0 {
        set(
            &mut env,
            "BUZZ_ACP_EXIT_AFTER_INACTIVITY",
            &inactivity_seconds.to_string(),
        );
    }

    env.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::execution::{LaunchSpec, WorkloadId};
    use nostr::Keys;

    const OWNER: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn launch_spec() -> LaunchSpec {
        let mut policy_env = BTreeMap::new();
        policy_env.insert("BUZZ_ACP_SYSTEM_PROMPT".to_string(), "prompt".to_string());
        policy_env.insert("GOOSE_MODE".to_string(), "auto".to_string());
        let mut env = BTreeMap::new();
        env.insert("GOOSE_MODE".to_string(), "custom".to_string());
        // A hostile or stale contract cannot smuggle identity material.
        env.insert("BUZZ_PRIVATE_KEY".to_string(), "nsec1evil".to_string());
        env.insert("GIT_CONFIG_COUNT".to_string(), "9".to_string());
        LaunchSpec::new(
            "buzz-agent",
            vec!["--flag".to_string()],
            Some("buzz-dev-mcp".to_string()),
            env,
            policy_env,
            Some(OWNER.to_string()),
        )
        .expect("launch spec")
    }

    fn environment_for(auth_tag: Option<String>) -> Vec<(String, Zeroizing<String>)> {
        let agent =
            AgentWorkloadContext::new(Keys::generate().public_key().to_hex(), None, auth_tag, None)
                .expect("agent context");
        let mut spec = WorkloadSpec::agent(
            WorkloadId::random(),
            "Owner fallback test agent",
            "buzz-agent",
            None,
            None,
            Vec::new(),
            launch_spec(),
        )
        .expect("workload spec");
        spec.agent = Some(agent);
        let agent = spec.agent.clone().expect("agent");
        let resolved = ResolvedCommands {
            agent_command: "/opt/bin/buzz-agent",
            mcp_command: None,
        };
        harness_environment(
            &spec,
            &agent,
            OWNER,
            "nsec1launchkey",
            "wss://relay.example",
            &resolved,
            0,
        )
    }

    fn value_of<'a>(
        env: &'a [(String, Zeroizing<String>)],
        name: &str,
    ) -> Option<&'a Zeroizing<String>> {
        env.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    #[test]
    fn without_an_auth_tag_the_body_gets_the_owner_fallback() {
        let env = environment_for(None);
        assert_eq!(
            value_of(&env, "BUZZ_ACP_AGENT_OWNER").map(|value| value.as_str()),
            Some(OWNER),
            "a body with no attestation must still resolve its owner"
        );
        assert!(value_of(&env, "BUZZ_AUTH_TAG").is_none());
    }

    #[test]
    fn with_an_auth_tag_the_attestation_stays_the_only_owner_source() {
        let env = environment_for(Some("auth-tag-payload".to_string()));
        assert_eq!(
            value_of(&env, "BUZZ_AUTH_TAG").map(|value| value.as_str()),
            Some("auth-tag-payload")
        );
        assert!(
            value_of(&env, "BUZZ_ACP_AGENT_OWNER").is_none(),
            "the attestation already carries the owner"
        );
    }

    #[test]
    fn the_layered_env_wins_over_policy_and_the_node_wins_over_the_wire() {
        let env = environment_for(None);
        assert_eq!(
            value_of(&env, "GOOSE_MODE").map(|value| value.as_str()),
            Some("custom"),
            "launch.env must override launch.policy_env"
        );
        assert_eq!(
            value_of(&env, "BUZZ_ACP_SYSTEM_PROMPT").map(|value| value.as_str()),
            Some("prompt"),
            "policy values without a user override must apply"
        );
        assert_eq!(
            value_of(&env, "BUZZ_PRIVATE_KEY").map(|value| value.as_str()),
            Some("nsec1launchkey"),
            "the node's one-time key must displace any wire value"
        );
        assert!(
            value_of(&env, "GIT_CONFIG_COUNT").is_none(),
            "the git credential family is node-owned"
        );
    }

    #[test]
    fn commands_and_args_come_from_the_resolved_contract() {
        let env = environment_for(None);
        assert_eq!(
            value_of(&env, "BUZZ_ACP_AGENT_COMMAND").map(|value| value.as_str()),
            Some("/opt/bin/buzz-agent"),
        );
        assert_eq!(
            value_of(&env, "BUZZ_ACP_AGENT_ARGS").map(|value| value.as_str()),
            Some("--flag"),
        );
        assert_eq!(
            value_of(&env, "BUZZ_ACP_SESSION_TITLE").map(|value| value.as_str()),
            Some("Owner fallback test agent"),
            "a contract without a session title falls back to the workload name"
        );
    }
}
