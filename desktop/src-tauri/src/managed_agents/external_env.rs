//! Env-block assembly for [`BackendKind::External`] agents.
//!
//! An external agent runs `buzz-acp` on hardware Buzz does not control —
//! typically the user's own container. Buzz mints the identity and publishes the
//! profile; the user starts the harness. This module produces the env the user
//! pastes into their container so that harness comes up with the same effective
//! configuration a local spawn would have given it.
//!
//! # Relationship to `spawn_agent_child`
//!
//! This is the second of two env assemblers. `spawn_agent_child`
//! (`super::runtime`) builds a *host process* env; this builds a *portable* one.
//! They agree on every protocol variable and deliberately diverge on host
//! concerns — see [`external_agent_env`] for the exclusion list and why each
//! group is dropped.
//!
//! The divergence is the accepted cost of not refactoring a 500-line function
//! that interleaves env writes with log-file creation, readiness probing, and
//! spawning. A new `BUZZ_ACP_*` protocol variable added at the spawn site must
//! be mirrored here by hand. Revisit extraction if a third consumer appears.
//!
//! [`BackendKind::External`]: super::types::BackendKind::External

use std::collections::BTreeMap;

use super::discovery::known_acp_runtime;
use super::effective_config::EffectiveAgentConfig;
use super::readiness::EffectiveHarnessDescriptor;
use super::runtime::{resolve_session_title, SESSION_TITLE_ENV_VAR};
use super::types::ManagedAgentRecord;

/// Assemble the portable env block for an external agent.
///
/// Insertion order is lowest-precedence first; `descriptor.env` is written last
/// so user-supplied env wins over every Buzz-set variable, exactly as
/// `spawn_agent_child` does. Reserved identity keys were already stripped from
/// `descriptor.env` upstream (`super::env_vars::merged_user_env`), so user env
/// cannot clobber `BUZZ_PRIVATE_KEY` and friends.
///
/// No `AppHandle`, no filesystem, no process env — fully unit-testable.
///
/// # Deliberately excluded
///
/// * `PATH`, `RUST_LOG`, `BUZZ_ACP_LAZY_POOL` — host-process concerns. The
///   container supplies its own PATH; log level and pool laziness are the
///   operator's call, not a property of the agent.
/// * Every absolute path `spawn_agent_child` resolves via `resolve_command`
///   (the harness command, the agent command, `CLAUDE_CODE_EXECUTABLE`,
///   `git-credential-nostr`). Host filesystem layout does not transfer; commands
///   are emitted **bare** so the container resolves them on its own PATH.
/// * `GIT_CONFIG_*` — points at a host `git-credential-nostr` binary. The image
///   configures its own helper; `NOSTR_PRIVATE_KEY` is all the helper needs and
///   *is* emitted.
/// * `BUZZ_ACP_SETUP_PAYLOAD` — desktop-computed readiness for *this* host,
///   meaningless for a container Buzz cannot inspect.
/// * `BUZZ_MANAGED_AGENT`, `BUZZ_MANAGED_AGENT_START_NONCE` — desktop-ownership
///   stamps. An external agent is by definition not desktop-owned, and emitting
///   them would make the orphan sweep treat the container as a process to reap.
/// * `MCP_HOOK_SERVERS` — only meaningful with a desktop-injected hook server.
/// * `HERMES_ACP_SKIP_CONFIGURED_MCP` and any other per-runtime env default from
///   `config::default_agent_env` — the harness applies those itself, keyed on the
///   agent command (`crates/buzz-acp/src/config.rs`). Emitting them here would
///   duplicate truth and drift when the harness changes.
pub(crate) fn external_agent_env(
    record: &ManagedAgentRecord,
    descriptor: &EffectiveHarnessDescriptor,
    cfg: &EffectiveAgentConfig,
    relay_url: &str,
    team_instructions: Option<&str>,
    gate: &[(&'static str, String)],
) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let runtime_meta = known_acp_runtime(&descriptor.command);

    // ── Identity ────────────────────────────────────────────────────────────
    env.insert(
        "BUZZ_PRIVATE_KEY".to_string(),
        record.private_key_nsec.clone(),
    );
    // Mirrors BUZZ_PRIVATE_KEY for git-sign-nostr / git-credential-nostr, same
    // as `spawn_agent_child`. Unlike spawn we emit it unconditionally: spawn
    // gates on finding the helper binary *on this host*, which says nothing
    // about the container. The container is responsible for shipping the helper.
    env.insert(
        "NOSTR_PRIVATE_KEY".to_string(),
        record.private_key_nsec.clone(),
    );
    if let Some(auth_tag) = record.auth_tag.as_deref() {
        env.insert("BUZZ_AUTH_TAG".to_string(), auth_tag.to_string());
    }
    env.insert("BUZZ_RELAY_URL".to_string(), relay_url.to_string());

    // ── Harness → agent wiring (bare commands; see exclusion list) ──────────
    env.insert(
        "BUZZ_ACP_AGENT_COMMAND".to_string(),
        descriptor.command.clone(),
    );
    env.insert("BUZZ_ACP_AGENT_ARGS".to_string(), descriptor.args.join(","));
    // Empty is meaningful, not missing: `build_mcp_servers` in buzz-acp returns
    // no servers for an empty command. Harnesses whose metadata declares no MCP
    // command (goose, claude, and every tier-2 preset including Hermes) get
    // their Buzz tooling from their own config rather than an injected server —
    // emitting the empty value keeps that parity explicit.
    env.insert(
        "BUZZ_ACP_MCP_COMMAND".to_string(),
        runtime_meta
            .and_then(|meta| meta.mcp_command)
            .unwrap_or("")
            .to_string(),
    );

    // Per-runtime env defaults (e.g. GOOSE_MODE=auto). `spawn_agent_child`
    // applies these only when absent from the desktop's own environment, so an
    // operator's shell can override. A container inherits nothing, so the
    // default is unconditionally correct here.
    if let Some(meta) = runtime_meta {
        for (key, value) in meta.default_env {
            env.insert((*key).to_string(), (*value).to_string());
        }
    }

    // ── Effective configuration ─────────────────────────────────────────────
    if let Some(prompt) = cfg.system_prompt.value.as_deref() {
        env.insert("BUZZ_ACP_SYSTEM_PROMPT".to_string(), prompt.to_string());
    }
    if let Some(model) = cfg.model.value.as_deref() {
        env.insert("BUZZ_ACP_MODEL".to_string(), model.to_string());
    }
    if let Some(title) = resolve_session_title(record.display_name.as_deref(), &record.name) {
        env.insert(SESSION_TITLE_ENV_VAR.to_string(), title);
    }
    if let Some(instructions) = team_instructions {
        env.insert(
            "BUZZ_ACP_TEAM_INSTRUCTIONS".to_string(),
            instructions.to_string(),
        );
    }
    env.insert(
        "BUZZ_ACP_AGENTS".to_string(),
        record.parallelism.to_string(),
    );
    // Only emitted when explicitly overridden — otherwise the harness applies
    // its own defaults, which are the single source of truth (see
    // DEFAULT_IDLE_TIMEOUT_SECS in crates/buzz-acp/src/config.rs).
    if let Some(idle) = record.idle_timeout_seconds {
        env.insert("BUZZ_ACP_IDLE_TIMEOUT".to_string(), idle.to_string());
    }
    if let Some(max_dur) = record.max_turn_duration_seconds {
        env.insert(
            "BUZZ_ACP_MAX_TURN_DURATION".to_string(),
            max_dur.to_string(),
        );
    }

    // ── Protocol defaults ───────────────────────────────────────────────────
    env.insert(
        "BUZZ_ACP_MULTIPLE_EVENT_HANDLING".to_string(),
        "steer".to_string(),
    );
    env.insert("BUZZ_ACP_DEDUP".to_string(), "queue".to_string());
    env.insert("BUZZ_ACP_RELAY_OBSERVER".to_string(), "true".to_string());

    // ── Inbound author gate ─────────────────────────────────────────────────
    // Only the `set` half of `build_respond_to_env` applies: its `remove` half
    // clears inherited process env, and a fresh container has none to clear.
    for (key, value) in gate {
        env.insert((*key).to_string(), value.clone());
    }

    // ── User env last so it wins (reserved keys already stripped) ───────────
    for (key, value) in &descriptor.env {
        env.insert(key.clone(), value.clone());
    }

    env
}

/// Render an env map as `KEY=value` lines for `docker run --env-file`.
///
/// One key per line, sorted (the map is a `BTreeMap`). Values are emitted raw:
/// `--env-file` treats everything after the first `=` literally, with no quote
/// or escape processing, so quoting would corrupt values rather than protect
/// them. Keys are POSIX-validated and newline-free before they reach this
/// function (`super::env_vars::merged_user_env`), so no key can forge a line
/// break and inject a second assignment.
pub(crate) fn render_env_file(env: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    for (key, value) in env {
        out.push_str(key);
        out.push('=');
        out.push_str(value);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::effective_config::{ConfigSource, ResolvedField};
    use crate::managed_agents::types::{BackendKind, RespondTo};

    fn record() -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: "aa".repeat(32),
            name: "hermes-vps".to_string(),
            persona_id: None,
            private_key_nsec: "nsec1testkey".to_string(),
            auth_tag: Some(r#"["auth","ownerpk","","sig"]"#.to_string()),
            relay_url: "wss://relay.example.com".to_string(),
            avatar_url: None,
            acp_command: "buzz-acp".to_string(),
            agent_command: "hermes-acp".to_string(),
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: "".to_string(),
            turn_timeout_seconds: 300,
            idle_timeout_seconds: None,
            max_turn_duration_seconds: None,
            parallelism: 2,
            system_prompt: None,
            model: None,
            provider: None,
            persona_source_version: None,
            env_vars: BTreeMap::new(),
            start_on_app_launch: false,
            runtime_pid: None,
            backend: BackendKind::External,
            backend_agent_id: None,
            provider_binary_path: None,
            team_id: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: "".to_string(),
            updated_at: "".to_string(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: RespondTo::OwnerOnly,
            respond_to_allowlist: vec![],
            display_name: None,
            slug: None,
            runtime: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            relay_mesh: None,
            auto_restart_on_config_change: false,
            definition_respond_to: None,
            definition_respond_to_allowlist: vec![],
            definition_parallelism: None,
        }
    }

    fn descriptor(command: &str) -> EffectiveHarnessDescriptor {
        EffectiveHarnessDescriptor {
            command: command.to_string(),
            args: vec![],
            env: BTreeMap::new(),
        }
    }

    fn unset() -> ResolvedField<String> {
        ResolvedField {
            value: None,
            source: ConfigSource::Global,
        }
    }

    fn cfg() -> EffectiveAgentConfig {
        EffectiveAgentConfig {
            model: unset(),
            provider: unset(),
            system_prompt: unset(),
        }
    }

    fn gate() -> Vec<(&'static str, String)> {
        vec![("BUZZ_ACP_RESPOND_TO", "anyone".to_string())]
    }

    fn build(
        rec: &ManagedAgentRecord,
        desc: &EffectiveHarnessDescriptor,
    ) -> BTreeMap<String, String> {
        external_agent_env(rec, desc, &cfg(), "wss://relay.example.com", None, &gate())
    }

    #[test]
    fn emits_the_variables_a_container_cannot_start_without() {
        let env = build(&record(), &descriptor("hermes-acp"));
        assert_eq!(env.get("BUZZ_PRIVATE_KEY").unwrap(), "nsec1testkey");
        assert_eq!(env.get("NOSTR_PRIVATE_KEY").unwrap(), "nsec1testkey");
        assert_eq!(
            env.get("BUZZ_AUTH_TAG").unwrap(),
            r#"["auth","ownerpk","","sig"]"#
        );
        assert_eq!(
            env.get("BUZZ_RELAY_URL").unwrap(),
            "wss://relay.example.com"
        );
        assert_eq!(env.get("BUZZ_ACP_AGENT_COMMAND").unwrap(), "hermes-acp");
        assert_eq!(env.get("BUZZ_ACP_RESPOND_TO").unwrap(), "anyone");
        assert_eq!(env.get("BUZZ_ACP_SESSION_TITLE").unwrap(), "hermes-vps");
    }

    #[test]
    fn excludes_host_specific_and_desktop_ownership_variables() {
        let env = build(&record(), &descriptor("hermes-acp"));
        for key in [
            "PATH",
            "RUST_LOG",
            "BUZZ_ACP_LAZY_POOL",
            "BUZZ_MANAGED_AGENT",
            "BUZZ_MANAGED_AGENT_START_NONCE",
            "BUZZ_ACP_SETUP_PAYLOAD",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_KEY_0",
            "MCP_HOOK_SERVERS",
            // Applied by the harness itself, keyed on the agent command.
            "HERMES_ACP_SKIP_CONFIGURED_MCP",
        ] {
            assert!(
                !env.contains_key(key),
                "{key} is host- or desktop-specific and must not reach a container"
            );
        }
    }

    #[test]
    fn agent_command_stays_bare_not_an_absolute_host_path() {
        let env = build(&record(), &descriptor("hermes-acp"));
        let command = env.get("BUZZ_ACP_AGENT_COMMAND").unwrap();
        assert!(
            !command.contains('/') && !command.contains('\\'),
            "container resolves the command on its own PATH, got {command:?}"
        );
    }

    #[test]
    fn user_env_wins_over_buzz_defaults_but_cannot_touch_identity() {
        let mut desc = descriptor("hermes-acp");
        desc.env
            .insert("BUZZ_ACP_DEDUP".to_string(), "drop".to_string());
        // Reserved keys are stripped upstream, so descriptor.env can never
        // legitimately carry one. Prove the layering anyway: if a reserved key
        // ever leaked through, identity must still win.
        desc.env
            .insert("SOME_USER_TOKEN".to_string(), "abc".to_string());
        let env = build(&record(), &desc);
        assert_eq!(env.get("BUZZ_ACP_DEDUP").unwrap(), "drop");
        assert_eq!(env.get("SOME_USER_TOKEN").unwrap(), "abc");
        assert_eq!(env.get("BUZZ_PRIVATE_KEY").unwrap(), "nsec1testkey");
    }

    #[test]
    fn omits_optional_timeouts_so_the_harness_default_applies() {
        let env = build(&record(), &descriptor("hermes-acp"));
        assert!(!env.contains_key("BUZZ_ACP_IDLE_TIMEOUT"));
        assert!(!env.contains_key("BUZZ_ACP_MAX_TURN_DURATION"));

        let mut rec = record();
        rec.idle_timeout_seconds = Some(90);
        rec.max_turn_duration_seconds = Some(600);
        let env = build(&rec, &descriptor("hermes-acp"));
        assert_eq!(env.get("BUZZ_ACP_IDLE_TIMEOUT").unwrap(), "90");
        assert_eq!(env.get("BUZZ_ACP_MAX_TURN_DURATION").unwrap(), "600");
    }

    #[test]
    fn legacy_record_without_auth_tag_falls_back_to_owner_env() {
        let mut rec = record();
        rec.auth_tag = None;
        // What `build_respond_to_env` produces for a legacy record.
        let gate = vec![("BUZZ_ACP_AGENT_OWNER", "ownerpk".to_string())];
        let env = external_agent_env(
            &rec,
            &descriptor("hermes-acp"),
            &cfg(),
            "wss://relay.example.com",
            None,
            &gate,
        );
        assert!(!env.contains_key("BUZZ_AUTH_TAG"));
        assert_eq!(env.get("BUZZ_ACP_AGENT_OWNER").unwrap(), "ownerpk");
    }

    #[test]
    fn applies_runtime_default_env_unconditionally() {
        // goose declares GOOSE_MODE=auto; spawn skips it when the desktop's own
        // env already has it, but a container inherits nothing.
        let env = build(&record(), &descriptor("goose"));
        assert_eq!(env.get("GOOSE_MODE").unwrap(), "auto");
    }

    #[test]
    fn mcp_command_is_empty_for_harnesses_that_declare_none() {
        let env = build(&record(), &descriptor("hermes-acp"));
        assert_eq!(env.get("BUZZ_ACP_MCP_COMMAND").unwrap(), "");
    }

    #[test]
    fn render_env_file_emits_one_assignment_per_line() {
        let env = build(&record(), &descriptor("hermes-acp"));
        let rendered = render_env_file(&env);
        for line in rendered.lines() {
            assert!(line.contains('='), "not an assignment: {line:?}");
        }
        assert_eq!(
            rendered.lines().count(),
            env.len(),
            "no key may forge an extra line"
        );
        assert!(rendered.contains("BUZZ_PRIVATE_KEY=nsec1testkey\n"));
    }
}
