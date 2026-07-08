use std::path::PathBuf;

use super::{
    classify_runtime, create_time_agent_command_override, default_agent_command,
    divergent_agent_command_override, effective_agent_command, find_via_login_shell,
    managed_agent_avatar_url, normalize_agent_args, record_agent_command,
    update_time_agent_command_override, BUZZ_AGENT_AVATAR_URL, CLAUDE_CODE_AVATAR_URL,
    CODEX_AVATAR_URL, GOOSE_AVATAR_URL,
};
use crate::managed_agents::AcpAvailabilityStatus;

#[test]
fn resolves_known_avatar_for_bare_command() {
    let avatar_url = managed_agent_avatar_url("goose").expect("goose avatar should resolve");

    assert_eq!(avatar_url, GOOSE_AVATAR_URL);
}

#[test]
fn resolves_known_avatar_for_command_paths_and_aliases() {
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/codex-acp"),
        Some(CODEX_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url("Claude Code"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url(r"C:\Tools\claude-agent-acp.exe"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/claude-code-acp"),
        Some(CLAUDE_CODE_AVATAR_URL.to_string())
    );
}

#[test]
fn returns_none_for_unknown_commands() {
    assert!(managed_agent_avatar_url("custom-agent").is_none());
}

#[test]
fn default_agent_command_resolves_bundled_buzz_agent() {
    // The create-path default must be the bundled buzz-agent, never the
    // bare `goose` that isn't on PATH on a stock Windows install.
    assert_eq!(default_agent_command(), "buzz-agent");
    // And buzz-agent takes no `acp` arg — confirm no arg leakage from the default.
    assert_eq!(
        normalize_agent_args(&default_agent_command(), vec!["acp".into()]),
        Vec::<String>::new()
    );
}

#[test]
fn normalizes_claude_and_codex_args_to_empty() {
    assert_eq!(
        normalize_agent_args("claude-agent-acp", vec!["acp".into()]),
        Vec::<String>::new()
    );
    assert_eq!(
        normalize_agent_args("claude-code-acp", vec!["acp".into()]),
        Vec::<String>::new()
    );
    assert_eq!(
        normalize_agent_args("codex-acp", vec!["acp".into()]),
        Vec::<String>::new()
    );
}

#[test]
fn resolves_buzz_agent_avatar() {
    assert_eq!(
        managed_agent_avatar_url("buzz-agent"),
        Some(BUZZ_AGENT_AVATAR_URL.to_string())
    );
    assert_eq!(
        managed_agent_avatar_url("/usr/local/bin/buzz-agent"),
        Some(BUZZ_AGENT_AVATAR_URL.to_string())
    );
}

#[test]
fn normalizes_buzz_agent_args_to_empty() {
    assert_eq!(
        normalize_agent_args("buzz-agent", Vec::new()),
        Vec::<String>::new()
    );
    assert_eq!(
        normalize_agent_args("buzz-agent", vec!["acp".into()]),
        Vec::<String>::new()
    );
}

#[test]
fn login_shell_lookup_treats_command_as_data() {
    let marker =
        std::env::temp_dir().join(format!("buzz-discovery-marker-{}", uuid::Uuid::new_v4()));
    let payload = format!("doesnotexist; touch {} #", marker.display());

    let resolved = find_via_login_shell(&payload);

    assert!(
        resolved.is_none(),
        "payload should not resolve to a command"
    );
    assert!(
        !marker.exists(),
        "shell lookup must not execute injected commands"
    );
}

#[cfg(unix)]
#[test]
fn explicit_path_resolution_ignores_non_executable_files() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("buzz-discovery-path-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bin = dir.join("buzz-acp");
    std::fs::write(&bin, "").expect("write placeholder");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644))
        .expect("chmod placeholder");

    assert!(
        super::resolve_workspace_command(bin.to_str().expect("utf8 path")).is_none(),
        "non-executable placeholder must not resolve"
    );

    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
        .expect("chmod executable");
    assert_eq!(
        super::resolve_workspace_command(bin.to_str().expect("utf8 path")),
        Some(bin.clone())
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn classifies_available_when_adapter_found() {
    let (status, cmd, path) = classify_runtime(
        Some(("goose", PathBuf::from("/usr/local/bin/goose"))),
        None,
        false,
    );
    assert_eq!(status, AcpAvailabilityStatus::Available);
    assert_eq!(cmd.as_deref(), Some("goose"));
    assert_eq!(path.as_deref(), Some("/usr/local/bin/goose"));
}

#[test]
fn classifies_adapter_missing_when_cli_present() {
    let (status, cmd, path) = classify_runtime(None, Some("claude"), true);
    assert_eq!(status, AcpAvailabilityStatus::AdapterMissing);
    assert!(cmd.is_none());
    assert!(path.is_none());
}

#[test]
fn classifies_not_installed_when_nothing_found() {
    let (status, cmd, path) = classify_runtime(None, Some("claude"), false);
    assert_eq!(status, AcpAvailabilityStatus::NotInstalled);
    assert!(cmd.is_none());
    assert!(path.is_none());
}

#[test]
fn classifies_not_installed_when_no_underlying_cli() {
    let (status, cmd, path) = classify_runtime(None, None, false);
    assert_eq!(status, AcpAvailabilityStatus::NotInstalled);
    assert!(cmd.is_none());
    assert!(path.is_none());
}

#[test]
fn classifies_cli_missing_when_adapter_found_but_cli_absent() {
    let (status, cmd, path) = classify_runtime(
        Some(("codex-acp", PathBuf::from("/opt/homebrew/bin/codex-acp"))),
        Some("codex"),
        false,
    );
    assert_eq!(status, AcpAvailabilityStatus::CliMissing);
    assert_eq!(cmd.as_deref(), Some("codex-acp"));
    assert_eq!(path.as_deref(), Some("/opt/homebrew/bin/codex-acp"));
}

fn persona_with_runtime(id: &str, runtime: Option<&str>) -> crate::managed_agents::PersonaRecord {
    crate::managed_agents::PersonaRecord {
        id: id.to_string(),
        display_name: id.to_string(),
        avatar_url: None,
        system_prompt: String::new(),
        runtime: runtime.map(str::to_string),
        model: None,
        provider: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: std::collections::BTreeMap::new(),
        created_at: "2026-06-09T00:00:00Z".to_string(),
        updated_at: "2026-06-09T00:00:00Z".to_string(),
    }
}

#[test]
fn effective_agent_command_explicit_override_wins() {
    // An explicit pin beats the persona's runtime.
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        effective_agent_command(Some("p1"), &personas, Some("codex-acp")),
        "codex-acp"
    );
}

/// Minimal record for `record_agent_command` tests. Only the resolution
/// inputs (runtime / persona_id / agent_command_override) vary.
fn record_with(
    runtime: Option<&str>,
    persona_id: Option<&str>,
    override_cmd: Option<&str>,
) -> crate::managed_agents::types::ManagedAgentRecord {
    crate::managed_agents::types::ManagedAgentRecord {
        pubkey: String::new(),
        name: "r".to_string(),
        persona_id: persona_id.map(str::to_string),
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: String::new(),
        agent_command: String::new(),
        agent_command_override: override_cmd.map(str::to_string),
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        mcp_toolsets: None,
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        env_vars: std::collections::BTreeMap::new(),
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        respond_to: Default::default(),
        respond_to_allowlist: vec![],
        display_name: None,
        slug: None,
        runtime: runtime.map(str::to_string),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        relay_mesh: None,
    }
}

#[test]
fn record_agent_command_own_runtime_wins_over_persona() {
    // A record with its own materialized runtime never consults the
    // persona list — the unified-model resolution.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let record = record_with(Some("claude"), Some("p1"), None);
    assert_eq!(record_agent_command(&record, &personas), "claude-agent-acp");
}

#[test]
fn record_agent_command_override_beats_runtime() {
    let record = record_with(Some("claude"), None, Some("codex-acp"));
    assert_eq!(record_agent_command(&record, &[]), "codex-acp");
}

#[test]
fn record_agent_command_legacy_persona_fallback() {
    // Pre-migration record: persona_id set, no runtime — resolves through
    // the legacy persona path unchanged.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let record = record_with(None, Some("p1"), None);
    assert_eq!(record_agent_command(&record, &personas), "goose");
}

#[test]
fn record_agent_command_bare_record_defaults() {
    let record = record_with(None, None, None);
    assert_eq!(record_agent_command(&record, &[]), default_agent_command());
}

#[test]
fn effective_agent_command_inherits_persona_runtime() {
    // No override → persona runtime id maps to its primary command.
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        effective_agent_command(Some("p1"), &personas, None),
        "claude-agent-acp"
    );
}

#[test]
fn effective_agent_command_empty_override_is_inherit() {
    // A blank/whitespace override is treated as "inherit", not a pin.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        effective_agent_command(Some("p1"), &personas, Some("   ")),
        "goose"
    );
}

#[test]
fn effective_agent_command_falls_back_to_default() {
    // No override, no persona runtime, and a deleted persona all fall back
    // to the bundled default.
    let personas = vec![persona_with_runtime("p1", None)];
    assert_eq!(
        effective_agent_command(Some("p1"), &personas, None),
        default_agent_command()
    );
    assert_eq!(
        effective_agent_command(Some("gone"), &personas, None),
        default_agent_command()
    );
    assert_eq!(
        effective_agent_command(None, &personas, None),
        default_agent_command()
    );
}

#[test]
fn divergent_override_none_when_picked_matches_persona_runtime() {
    // The persona-backed create/edit flow sends the persona's resolved
    // command. It must be treated as "inherit" (None), not a pin.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        divergent_agent_command_override(Some("p1"), &personas, Some("goose")),
        None
    );
}

#[test]
fn divergent_override_none_for_alternate_command_of_same_runtime() {
    // A client with only `claude-code-acp` installed sends that command for
    // a `claude` persona whose primary command is `claude-agent-acp`. Both
    // map to the `claude` runtime, so it inherits — string equality would
    // wrongly bake a pin (CRITICAL-3).
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        divergent_agent_command_override(Some("p1"), &personas, Some("claude-code-acp")),
        None
    );
}

#[test]
fn divergent_override_some_when_picked_is_different_runtime() {
    // A deliberate pin to a different runtime is preserved.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        divergent_agent_command_override(Some("p1"), &personas, Some("codex-acp")),
        Some("codex-acp".to_string())
    );
}

#[test]
fn divergent_override_none_for_empty_or_absent_pick() {
    // The "Inherit from persona" sentinel (empty) and a name-only edit
    // (absent) both clear the pin.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        divergent_agent_command_override(Some("p1"), &personas, Some("   ")),
        None
    );
    assert_eq!(
        divergent_agent_command_override(Some("p1"), &personas, None),
        None
    );
}

#[test]
fn create_time_override_none_when_persona_runtime_not_installed() {
    // CRITICAL-3 (Case 3): a `claude`-persona agent created on a machine
    // where the claude adapter isn't installed. `resolvePersonaRuntime`
    // falls back to the default (`buzz-agent`) and sends THAT command with
    // `harness_override` false (the user did not pick it). At create this
    // is a fallback, not a deliberate pin — it must store `None` so the
    // agent inherits the persona's runtime once it's installed and the
    // persona is re-edited. Baking `Some("buzz-agent")` here is the exact
    // bug this resolver chain exists to kill.
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        create_time_agent_command_override(Some("p1"), &personas, Some("buzz-agent"), false),
        None
    );
}

#[test]
fn create_time_override_some_when_user_deliberately_overrides_installed_runtime() {
    // Case 2 + deliberate override: the persona's `claude` runtime IS
    // available, but the user explicitly picked `codex` in a deploy dialog's
    // runtime selector ("overriding persona preferences"), so the frontend
    // sends `codex-acp` with `harness_override` true. This is a real pin and
    // MUST be preserved — returning `None` would silently swallow the
    // deliberate override and inherit `claude` on spawn.
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        create_time_agent_command_override(Some("p1"), &personas, Some("codex-acp"), true),
        Some("codex-acp".to_string())
    );
}

#[test]
fn create_time_override_none_when_persona_runtime_installed() {
    // Case 2: the persona's runtime is available, so `resolvePersonaRuntime`
    // sends the persona's own command with no override. Inherits — no pin.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        create_time_agent_command_override(Some("p1"), &personas, Some("goose"), false),
        None
    );
}

#[test]
fn create_time_override_preserves_selected_runtime_alias() {
    // A `claude` persona inherits the primary command `claude-agent-acp`,
    // but discovery may select an installed alias such as `claude-code-acp`.
    // When UI marks that create-time selection as explicit, preserve the
    // alias so the first spawn uses a command known to be installed.
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        create_time_agent_command_override(Some("p1"), &personas, Some("claude-code-acp"), true),
        Some("claude-code-acp".to_string())
    );
}

#[test]
fn create_time_override_inherits_exact_persona_command() {
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        create_time_agent_command_override(Some("p1"), &personas, Some("claude-agent-acp"), true),
        None
    );
}

#[test]
fn create_time_override_preserves_pin_for_persona_less_create() {
    // The standalone CreateAgentDialog creates persona-LESS agents. With no
    // persona to inherit, the picked command IS the agent's harness and must
    // be preserved as a real pin (divergence from the bundled default),
    // regardless of the override flag.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        create_time_agent_command_override(None, &personas, Some("codex-acp"), false),
        Some("codex-acp".to_string())
    );
}

#[test]
fn update_time_override_preserves_same_runtime_pin_when_overriding() {
    // The bug this fixes: the user picks "Custom command" in the edit
    // dialog and saves `goose` verbatim for a goose persona. That is a
    // deliberate pin (harness_override true) — it must be kept so future
    // persona runtime edits stop propagating, even though it maps to the
    // persona's own runtime. `divergent_agent_command_override` alone would
    // wrongly drop it to `None`.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        update_time_agent_command_override(Some("p1"), &personas, Some("goose"), true),
        Some("goose".to_string())
    );
}

#[test]
fn update_time_override_preserves_exact_persona_command_when_overriding() {
    // Even when the pick is byte-identical to the persona's own command, an
    // explicit Custom selection (harness_override true) is a deliberate pin
    // and is preserved. This is the core divergence from the create-time
    // contract: at update, equality reached the force branch only because
    // the user picked Custom.
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        update_time_agent_command_override(Some("p1"), &personas, Some("claude-agent-acp"), true),
        Some("claude-agent-acp".to_string())
    );
}

#[test]
fn update_time_override_preserves_alias_pin_when_overriding() {
    // A `claude` persona with an installed `claude-code-acp` alias: picking
    // it as a Custom pin is a deliberate divergence from the primary
    // command and must be preserved when overriding.
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        update_time_agent_command_override(Some("p1"), &personas, Some("claude-code-acp"), true),
        Some("claude-code-acp".to_string())
    );
}

#[test]
fn update_time_override_defers_to_divergent_when_not_overriding() {
    // Without the explicit intent bit (e.g. a name-only edit that still
    // echoes the command), the persona stays authoritative: a same-runtime
    // command inherits, a different runtime pins.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        update_time_agent_command_override(Some("p1"), &personas, Some("goose"), false),
        None
    );
    assert_eq!(
        update_time_agent_command_override(Some("p1"), &personas, Some("codex-acp"), false),
        Some("codex-acp".to_string())
    );
}

#[test]
fn update_time_override_clears_pin_for_inherit_sentinel() {
    // The empty "Inherit from persona" sentinel always clears the pin,
    // regardless of the override flag.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        update_time_agent_command_override(Some("p1"), &personas, Some("   "), true),
        None
    );
    assert_eq!(
        update_time_agent_command_override(Some("p1"), &personas, None, true),
        None
    );
}

#[test]
fn update_time_override_preserves_pin_for_persona_less_agent() {
    // A persona-less agent has no runtime to inherit, so any picked command
    // is a real pin — preserved even without the override flag (mirrors the
    // create-time persona-less contract).
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    assert_eq!(
        update_time_agent_command_override(None, &personas, Some("codex-acp"), false),
        Some("codex-acp".to_string())
    );
}
