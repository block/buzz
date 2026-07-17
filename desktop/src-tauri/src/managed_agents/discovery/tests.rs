use std::path::PathBuf;

use super::overrides::{divergent_agent_command_override, update_time_agent_command_override};
use super::{
    apply_agent_command_update, classify_runtime, codex_adapter_availability,
    codex_adapter_is_outdated, create_time_agent_command_override, default_agent_command,
    effective_agent_command, find_nvm_default_bin, find_via_login_shell,
    is_login_shell_path_uninit, is_safe_nvm_tag, managed_agent_avatar_url, normalize_agent_args,
    parse_semver_tag, probe_codex_acp_major_version, record_agent_command,
    refresh_login_shell_path, BUZZ_AGENT_AVATAR_URL, CLAUDE_CODE_AVATAR_URL, CODEX_AVATAR_URL,
    GOOSE_AVATAR_URL,
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
fn default_agent_command_resolves_bundled_goose() {
    // The create-path default must be the bundled sidecar, never the bare
    // `goose` that isn't on PATH on a stock install.
    assert_eq!(default_agent_command(), "goose-acp");
    // goose-acp is already an ACP server and takes no `acp` subcommand.
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
        Some(("goose-acp", PathBuf::from("/app/goose-acp"))),
        None,
        false,
    );
    assert_eq!(status, AcpAvailabilityStatus::Available);
    assert_eq!(cmd.as_deref(), Some("goose-acp"));
    assert_eq!(path.as_deref(), Some("/app/goose-acp"));
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

fn persona_with_runtime(id: &str, runtime: Option<&str>) -> crate::managed_agents::AgentDefinition {
    crate::managed_agents::AgentDefinition {
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
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
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
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        env_vars: std::collections::BTreeMap::new(),
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
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
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
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
    assert_eq!(record_agent_command(&record, &personas), "goose-acp");
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
        "goose-acp"
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
    // falls back to the default (`goose-acp`) and sends THAT command with
    // `harness_override` false (the user did not pick it). At create this
    // is a fallback, not a deliberate pin — it must store `None` so the
    // agent inherits the persona's runtime once it's installed and the
    // persona is re-edited. Baking `Some("goose-acp")` here is the exact
    // bug this resolver chain exists to kill.
    let personas = vec![persona_with_runtime("p1", Some("claude"))];
    assert_eq!(
        create_time_agent_command_override(Some("p1"), &personas, Some("goose-acp"), false),
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

#[test]
fn apply_agent_command_update_inherit_sentinel_clears_pin_and_runtime() {
    // Choosing Inherit on a persona-linked record clears BOTH the explicit
    // pin and the materialized runtime, so resolution falls through to the
    // live definition immediately — not on the next spawn.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let mut record = record_with(Some("claude"), Some("p1"), Some("codex-acp"));

    apply_agent_command_update(&mut record, &personas, "", false);

    assert_eq!(record.agent_command_override, None);
    assert_eq!(record.runtime, None);
    assert_eq!(record_agent_command(&record, &personas), "goose-acp");
}

#[test]
fn apply_agent_command_update_sentinel_keeps_runtime_for_definition_less_record() {
    // For a record with no persona link the materialized runtime is the only
    // harness source left once the pin is cleared — a stray empty
    // agent_command must not change what the agent runs.
    let mut record = record_with(Some("claude"), None, Some("codex-acp"));

    apply_agent_command_update(&mut record, &[], "", false);

    assert_eq!(record.agent_command_override, None);
    assert_eq!(record.runtime.as_deref(), Some("claude"));
    assert_eq!(record_agent_command(&record, &[]), "claude-agent-acp");
}

#[test]
fn apply_agent_command_update_concrete_pin_keeps_materialized_runtime() {
    // A concrete pick only sets the pin; the materialized runtime is left for
    // the next snapshot apply. The pin shadows it in resolution either way.
    let personas = vec![persona_with_runtime("p1", Some("goose"))];
    let mut record = record_with(Some("claude"), Some("p1"), None);

    apply_agent_command_update(&mut record, &personas, "codex-acp", true);

    assert_eq!(record.agent_command_override.as_deref(), Some("codex-acp"));
    assert_eq!(record.runtime.as_deref(), Some("claude"));
    assert_eq!(record_agent_command(&record, &personas), "codex-acp");
}

// ── probe_codex_acp_major_version ─────────────────────────────────────────────

mod managed_path_resolution;

#[cfg(unix)]
#[test]
fn probe_codex_acp_major_version_parses_1x_output() {
    use std::os::unix::fs::PermissionsExt;

    // Simulate `@agentclientprotocol/codex-acp 1.1.2` output (1.x adapter)
    let dir = std::env::temp_dir().join(format!("buzz-probe-1x-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bin = dir.join("codex-acp");
    std::fs::write(
        &bin,
        "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.2'\nexit 0\n",
    )
    .expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let major = probe_codex_acp_major_version(&bin);
    let _ = std::fs::remove_dir_all(dir);

    assert_eq!(major, Some(1), "1.x adapter must return major version 1");
}

mod codex_version;

#[cfg(unix)]
#[test]
fn probe_codex_acp_major_version_returns_none_for_nonzero_exit() {
    use std::os::unix::fs::PermissionsExt;

    // Simulate old 0.16.x adapter: `--version` is unrecognised, exits non-zero
    let dir = std::env::temp_dir().join(format!("buzz-probe-0x-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bin = dir.join("codex-acp");
    std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let major = probe_codex_acp_major_version(&bin);
    let _ = std::fs::remove_dir_all(dir);

    assert_eq!(
        major, None,
        "old 0.16.x adapter (non-zero exit) must return None"
    );
}

#[cfg(unix)]
#[test]
fn probe_codex_acp_major_version_returns_none_for_missing_binary() {
    let path = std::path::Path::new("/nonexistent/path/codex-acp-does-not-exist");
    let major = probe_codex_acp_major_version(path);
    assert_eq!(major, None, "missing binary must return None");
}

// ── codex_adapter_availability / codex_adapter_is_outdated ───────────────────
//
// Outcome-level classification: verify helpers map probe results to the correct
// AcpAvailabilityStatus and boolean without duplicating version-gate logic.

#[cfg(unix)]
#[test]
fn codex_adapter_availability_available_for_1x_binary() {
    use std::os::unix::fs::PermissionsExt;

    let dir = std::env::temp_dir().join(format!("buzz-avail-1x-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bin = dir.join("codex-acp");
    std::fs::write(
        &bin,
        "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.2'\nexit 0\n",
    )
    .expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let status = codex_adapter_availability(&bin);
    let _ = std::fs::remove_dir_all(dir);

    assert_eq!(
        status,
        AcpAvailabilityStatus::Available,
        "1.x adapter must classify as Available"
    );
}

#[cfg(unix)]
#[test]
fn codex_adapter_availability_outdated_for_0x_binary() {
    use std::os::unix::fs::PermissionsExt;

    // Simulate old 0.16.x: `--version` exits non-zero (unrecognised flag)
    let dir = std::env::temp_dir().join(format!("buzz-avail-0x-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bin = dir.join("codex-acp");
    std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let status = codex_adapter_availability(&bin);
    let _ = std::fs::remove_dir_all(dir);

    assert_eq!(
        status,
        AcpAvailabilityStatus::AdapterOutdated,
        "0.x adapter (non-zero exit) must classify as AdapterOutdated"
    );
}

#[cfg(unix)]
#[test]
fn codex_adapter_availability_outdated_for_missing_binary() {
    let path = std::path::Path::new("/nonexistent/codex-acp-probe-test");
    assert_eq!(
        codex_adapter_availability(path),
        AcpAvailabilityStatus::AdapterOutdated,
        "missing binary must classify as AdapterOutdated"
    );
    // Thin wrapper consistency
    assert!(
        codex_adapter_is_outdated(path),
        "missing binary must be classified as outdated via thin wrapper"
    );
}

#[cfg(unix)]
#[test]
fn probe_codex_acp_major_version_returns_none_for_hung_direct_child() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    // Simulate a process that writes version to stdout then blocks forever.
    // The probe reads stdout only after the child exits, so it will time out.
    // `exec sleep 300` replaces the shell so killing the child reaps `sleep` too.
    let dir = std::env::temp_dir().join(format!("buzz-probe-hung-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bin = dir.join("codex-acp");
    std::fs::write(
        &bin,
        "#!/bin/sh\nprintf '@agentclientprotocol/codex-acp 1.1.2\\n'\nexec sleep 300\n",
    )
    .expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let start = Instant::now();
    let major = probe_codex_acp_major_version(&bin);
    let elapsed = start.elapsed();
    let _ = std::fs::remove_dir_all(dir);

    assert_eq!(
        major, None,
        "hung binary must return None (timeout kills child)"
    );
    // The timeout is 5 s; give a 10 s margin for parallel pre-push suites.
    assert!(
        elapsed.as_secs() < 15,
        "probe must complete within timeout bound; elapsed: {elapsed:?}"
    );
}

#[cfg(unix)]
#[test]
fn probe_codex_acp_major_version_returns_version_when_descendant_holds_pipe_open() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    // Simulate a process that forks a background child which inherits stdout
    // and stays alive, while the parent writes version and exits 0.
    //
    // The probe writes the child's stdout to a temp file, then reads from the
    // file after the parent process exits.  Because the file has reached EOF
    // (the parent closed its write end), read_to_end() returns immediately
    // without waiting for the descendant to close its inherited fd.
    //
    // `(exec sleep 60 &)` forks a subshell that execs `sleep 60`; the subshell
    // inherits the parent's stdout fd and keeps it open.
    let dir = std::env::temp_dir().join(format!("buzz-probe-descendant-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let bin = dir.join("codex-acp");
    std::fs::write(
        &bin,
        "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.2'\n(exec sleep 60 &)\nexit 0\n",
    )
    .expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let start = Instant::now();
    let major = probe_codex_acp_major_version(&bin);
    let elapsed = start.elapsed();
    let _ = std::fs::remove_dir_all(dir);

    // Must return within ~1 s: non-blocking read, no waiting for descendant.
    // Give a 9 s margin for parallel pre-push suites.
    assert!(
        elapsed.as_secs() < 10,
        "probe must not block on descendant pipe; elapsed: {elapsed:?}"
    );
    assert_eq!(
        major,
        Some(1),
        "1.x version must be parsed even when descendant holds pipe open"
    );
}

// ── parse_semver_tag ──────────────────────────────────────────────────────────

#[test]
fn parse_semver_tag_accepts_plain_version() {
    assert_eq!(parse_semver_tag("v20.11.1"), Some((20, 11, 1)));
}

#[test]
fn parse_semver_tag_accepts_prerelease_suffix() {
    assert_eq!(parse_semver_tag("v18.0.0-rc1"), Some((18, 0, 0)));
}

#[test]
fn parse_semver_tag_rejects_missing_v_prefix() {
    assert_eq!(parse_semver_tag("20.11.1"), None);
}

#[test]
fn parse_semver_tag_rejects_non_numeric() {
    assert_eq!(parse_semver_tag("vX.Y.Z"), None);
}

#[test]
fn semver_ordering_chooses_highest() {
    let mut tags = [
        parse_semver_tag("v16.0.0").unwrap(),
        parse_semver_tag("v20.11.1").unwrap(),
        parse_semver_tag("v18.12.0").unwrap(),
    ];
    tags.sort();
    assert_eq!(tags.last(), parse_semver_tag("v20.11.1").as_ref());
}

// ── find_nvm_default_bin ──────────────────────────────────────────────────────

#[cfg(unix)]
fn make_nvm_version_dir(home: &std::path::Path, tag: &str) {
    let bin = home.join(".nvm/versions/node").join(tag).join("bin");
    std::fs::create_dir_all(&bin).unwrap();
}

#[cfg(unix)]
fn write_alias(home: &std::path::Path, name: &str, content: &str) {
    let alias_dir = home.join(".nvm/alias");
    std::fs::create_dir_all(&alias_dir).unwrap();
    std::fs::write(alias_dir.join(name), content).unwrap();
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_returns_dir_from_alias_default() {
    let home = tempfile::tempdir().unwrap();
    make_nvm_version_dir(home.path(), "v20.11.1");
    write_alias(home.path(), "default", "v20.11.1\n");

    let result = find_nvm_default_bin(home.path());
    assert_eq!(
        result,
        Some(home.path().join(".nvm/versions/node/v20.11.1/bin"))
    );
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_follows_one_alias_hop() {
    let home = tempfile::tempdir().unwrap();
    make_nvm_version_dir(home.path(), "v18.0.0");
    // alias/default → "lts/hydrogen", alias/lts/hydrogen → "v18.0.0"
    write_alias(home.path(), "default", "lts/hydrogen\n");
    let alias_dir = home.path().join(".nvm/alias/lts");
    std::fs::create_dir_all(&alias_dir).unwrap();
    std::fs::write(alias_dir.join("hydrogen"), "v18.0.0\n").unwrap();

    let result = find_nvm_default_bin(home.path());
    assert_eq!(
        result,
        Some(home.path().join(".nvm/versions/node/v18.0.0/bin"))
    );
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_falls_back_to_highest_semver() {
    let home = tempfile::tempdir().unwrap();
    make_nvm_version_dir(home.path(), "v16.0.0");
    make_nvm_version_dir(home.path(), "v20.11.1");
    make_nvm_version_dir(home.path(), "v18.12.0");
    // No alias/default file.

    let result = find_nvm_default_bin(home.path());
    assert_eq!(
        result,
        Some(home.path().join(".nvm/versions/node/v20.11.1/bin"))
    );
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_returns_none_when_nvm_absent() {
    let home = tempfile::tempdir().unwrap();
    // No ~/.nvm directory.
    assert_eq!(find_nvm_default_bin(home.path()), None);
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_returns_none_when_versions_dir_empty() {
    let home = tempfile::tempdir().unwrap();
    let versions_dir = home.path().join(".nvm/versions/node");
    std::fs::create_dir_all(&versions_dir).unwrap();
    assert_eq!(find_nvm_default_bin(home.path()), None);
}

// ── refresh_login_shell_path ──────────────────────────────────────────────────

#[test]
fn refresh_login_shell_path_clears_cache() {
    let _guard = crate::managed_agents::lock_path_mutex();
    // Populate the cache with a first call.
    let before = super::login_shell_path();
    assert!(
        !is_login_shell_path_uninit(),
        "cache must be Probed(…) after a login_shell_path() call"
    );

    // Refresh must reset the cache to Uninit so the next call re-fetches.
    refresh_login_shell_path();
    assert!(
        is_login_shell_path_uninit(),
        "cache must be Uninit immediately after refresh_login_shell_path()"
    );

    // Re-fetching must produce the same value (deterministic in this environment).
    let after = super::login_shell_path();
    assert_eq!(
        before, after,
        "login_shell_path must return the same value after refresh + re-fetch"
    );
    // And the cache is Probed again.
    assert!(
        !is_login_shell_path_uninit(),
        "cache must be Probed(…) after the second login_shell_path() call"
    );
}

// ── is_safe_nvm_tag ───────────────────────────────────────────────────────────

#[test]
fn is_safe_nvm_tag_accepts_version_tags() {
    assert!(is_safe_nvm_tag("v20.11.1"));
    assert!(is_safe_nvm_tag("v18.0.0-rc1"));
    assert!(is_safe_nvm_tag("lts/hydrogen"));
    assert!(is_safe_nvm_tag("v22.1.0"));
}

#[test]
fn is_safe_nvm_tag_rejects_absolute_paths() {
    assert!(!is_safe_nvm_tag("/tmp/evil"));
    assert!(!is_safe_nvm_tag("/usr/local/bin"));
    assert!(!is_safe_nvm_tag("/"));
}

#[test]
fn is_safe_nvm_tag_rejects_dotdot_traversal() {
    assert!(!is_safe_nvm_tag("../../../etc/passwd"));
    assert!(!is_safe_nvm_tag("v20.11.1/../../../etc"));
    assert!(!is_safe_nvm_tag(".."));
}

#[test]
fn is_safe_nvm_tag_rejects_junk_charset() {
    assert!(!is_safe_nvm_tag("v20.11.1; rm -rf ~"));
    assert!(!is_safe_nvm_tag("v20.11.1\n/tmp/evil"));
    assert!(!is_safe_nvm_tag("v20.11.1\0"));
    assert!(!is_safe_nvm_tag("$(evil)"));
}

#[test]
fn is_safe_nvm_tag_rejects_empty() {
    assert!(!is_safe_nvm_tag(""));
}

// ── find_nvm_default_bin alias security ──────────────────────────────────────

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_rejects_absolute_path_alias() {
    let home = tempfile::tempdir().unwrap();
    // Alias points at an absolute path — PathBuf::join would replace the base.
    write_alias(home.path(), "default", "/tmp/evil\n");
    // Must NOT return Some("/tmp/evil/bin") — must fall through to semver scan.
    let result = find_nvm_default_bin(home.path());
    // No version dirs exist, so result must be None (not Some("/tmp/evil/bin")).
    assert_eq!(result, None, "absolute-path alias must be rejected");
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_rejects_dotdot_traversal_alias() {
    let home = tempfile::tempdir().unwrap();
    write_alias(home.path(), "default", "../../etc/passwd\n");
    let result = find_nvm_default_bin(home.path());
    assert_eq!(result, None, "dotdot traversal alias must be rejected");
}

#[cfg(unix)]
#[test]
fn find_nvm_default_bin_rejects_absolute_hop_tag() {
    let home = tempfile::tempdir().unwrap();
    // First hop points at a safe tag that resolves to another alias file which
    // then contains an absolute path.
    write_alias(home.path(), "default", "lts/testing\n");
    let lts_dir = home.path().join(".nvm/alias/lts");
    std::fs::create_dir_all(&lts_dir).unwrap();
    std::fs::write(lts_dir.join("testing"), "/tmp/evil\n").unwrap();
    let result = find_nvm_default_bin(home.path());
    assert_eq!(result, None, "absolute-path hop tag must be rejected");
}

// ── Phase C: command_basenames ──────────────────────────────────────────────

/// On non-Windows, command_basenames returns only the executable_basename.
#[cfg(not(windows))]
#[test]
fn test_command_basenames_single_candidate_on_unix() {
    let candidates = super::command_basenames("codex-acp");
    assert_eq!(
        candidates,
        vec!["codex-acp"],
        "Unix must produce a single candidate"
    );
}

/// On Windows, command_basenames adds .exe, .cmd, and .bat candidates.
#[cfg(windows)]
#[test]
fn test_command_basenames_includes_cmd_bat_on_windows() {
    let candidates = super::command_basenames("codex-acp");
    assert_eq!(
        candidates,
        vec![
            "codex-acp.exe".to_string(),
            "codex-acp.cmd".to_string(),
            "codex-acp.bat".to_string(),
        ],
        "Windows must produce .exe, .cmd, and .bat candidates"
    );
}

/// command_basenames with a dotted name never adds .cmd/.bat.
#[test]
fn test_command_basenames_dotted_name_no_extra_candidates() {
    let candidates = super::command_basenames("codex-acp.exe");
    // Should contain the executable_basename only, no .cmd/.bat.
    assert_eq!(
        candidates.len(),
        1,
        "dotted name must produce exactly one candidate"
    );
}

// ── Phase B: cli_install_commands_for_os ────────────────────────────────────

/// Claude and Codex have non-empty default cli_install_commands (install.sh).
#[test]
fn test_claude_and_codex_have_cli_install_commands() {
    let claude = super::known_acp_runtime_exact("claude").unwrap();
    let codex = super::known_acp_runtime_exact("codex").unwrap();
    assert!(
        !claude.cli_install_commands.is_empty(),
        "claude must have cli install commands"
    );
    assert!(
        !codex.cli_install_commands.is_empty(),
        "codex must have cli install commands"
    );
}

/// cli_install_commands_for_os returns a non-empty slice for claude and codex.
#[test]
fn test_cli_install_commands_for_os_non_empty_for_claude_codex() {
    let claude = super::known_acp_runtime_exact("claude").unwrap();
    let codex = super::known_acp_runtime_exact("codex").unwrap();
    assert!(
        !claude.cli_install_commands_for_os().is_empty(),
        "claude must have install commands on every platform"
    );
    assert!(
        !codex.cli_install_commands_for_os().is_empty(),
        "codex must have install commands on every platform"
    );
}

/// On Windows, Claude and Codex select the PowerShell install commands.
#[cfg(windows)]
#[test]
fn test_cli_install_commands_for_os_selects_powershell_on_windows() {
    let claude = super::known_acp_runtime_exact("claude").unwrap();
    let codex = super::known_acp_runtime_exact("codex").unwrap();

    // Windows must select the PowerShell commands, not the curl|bash ones.
    let claude_cmds = claude.cli_install_commands_for_os();
    let codex_cmds = codex.cli_install_commands_for_os();

    assert_ne!(
        claude_cmds, claude.cli_install_commands,
        "Windows must NOT use the default curl|bash commands for claude"
    );
    assert_ne!(
        codex_cmds, codex.cli_install_commands,
        "Windows must NOT use the default curl|bash commands for codex"
    );

    // Verify they are the PowerShell installers.
    assert!(
        claude_cmds.iter().any(|c| c.contains("powershell")),
        "claude Windows install must use powershell; got: {claude_cmds:?}"
    );
    assert!(
        codex_cmds.iter().any(|c| c.contains("powershell")),
        "codex Windows install must use powershell; got: {codex_cmds:?}"
    );

    // Goose and buzz-agent must NOT use Windows-specific commands.
    let goose = super::known_acp_runtime_exact("goose").unwrap();
    assert_eq!(
        goose.cli_install_commands_for_os(),
        goose.cli_install_commands,
        "goose must use the same commands on all platforms"
    );
}

// ── Phase C: login_shell_candidates ─────────────────────────────────────────

/// On Unix, login_shell_candidates returns at least one candidate.
#[cfg(unix)]
#[test]
fn test_login_shell_candidates_non_empty_on_unix() {
    let candidates = super::login_shell_candidates();
    assert!(
        !candidates.is_empty(),
        "Unix must have at least one login shell candidate"
    );
    // The first candidate should be /bin/zsh or /bin/bash.
    let first = &candidates[0];
    assert!(
        first == std::path::Path::new("/bin/zsh") || first == std::path::Path::new("/bin/bash"),
        "expected /bin/zsh or /bin/bash, got {first:?}"
    );
}

// ── Regression: POSIX PATH must never reach native Windows consumers ───────

/// `login_shell_path()` must return `None` on Windows so native-process
/// consumers (`agent_models`, `build_augmented_path`, `cli_probe`) inherit
/// the real Windows PATH instead of a POSIX colon-delimited string from
/// Git Bash's `echo $PATH`.
#[cfg(windows)]
#[test]
fn test_login_shell_path_returns_none_on_windows() {
    let _guard = crate::managed_agents::lock_path_mutex();

    // Force a fresh fetch — don't rely on whatever prior tests cached.
    refresh_login_shell_path();
    let path = super::login_shell_path();

    assert_eq!(
        path, None,
        "login_shell_path() must return None on Windows to prevent POSIX PATH leaking into native children"
    );
}

// ── Phase C: .cmd shim resolution on Windows ───────────────────────────────

/// `resolve_command_uncached` finds `.cmd` shims via the Windows PATH scan
/// (discovery.rs lines 648-657). Creates a temp dir with ONLY a `.cmd` shim
/// (no `.exe`), mutates PATH under the serialized lock, and calls the actual
/// resolver — proving the full resolution chain finds `.cmd` extensions.
#[cfg(windows)]
#[test]
fn test_cmd_shim_resolves_from_path() {
    let _guard = crate::managed_agents::lock_path_mutex();

    // Create a temp dir with only a .cmd shim — no .exe.
    let temp = tempfile::tempdir().expect("tempdir");
    let shim = temp.path().join("test-shim-cmd-resolve.cmd");
    std::fs::write(&shim, "@echo off\r\n").expect("write shim");

    // Prepend the temp dir to PATH so the resolver sees it.
    // path_candidates_from_env_raw reads PATH live (std::env::var_os each call).
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::env::split_paths(&old_path).collect::<Vec<_>>();
    new_path.insert(0, temp.path().to_path_buf());
    let joined = std::env::join_paths(&new_path).expect("join PATH");
    std::env::set_var("PATH", &joined);

    let result = super::resolve_command_uncached("test-shim-cmd-resolve");

    // Restore PATH before any assertion can panic.
    std::env::set_var("PATH", &old_path);

    assert_eq!(
        result.as_deref(),
        Some(shim.as_path()),
        "resolve_command_uncached must find a .cmd shim on PATH when no .exe exists"
    );
}

// ── Phase A: no-shell-resolved error on Windows ────────────────────────────

/// When all resolution sources are empty AND the registry is disabled,
/// `resolve_git_bash_no_registry` returns `None` deterministically —
/// regardless of the CI runner's installed software.
#[cfg(windows)]
#[test]
fn test_no_git_bash_resolved_returns_none() {
    use crate::managed_agents::git_bash;

    // Empty PATH, no overrides, no well-known dirs, registry disabled.
    let result = git_bash::resolve_git_bash_no_registry("", None, None, None, None, None, None);
    assert_eq!(
        result, None,
        "empty environment with registry disabled must not resolve a Git Bash"
    );
}

/// When `resolve_bash_path` returns `None` (no Git Bash anywhere),
/// `install_shell_from` maps it to `Err(GIT_BASH_INSTALL_HINT)` — the exact
/// Doctor hint shown to the user. Tests the pure error-mapping seam, not the
/// resolution chain.
#[cfg(windows)]
#[test]
fn test_install_shell_from_none_returns_hint() {
    use crate::commands::install_shell_from;
    use crate::managed_agents::git_bash;

    let result = install_shell_from(None);
    assert_eq!(
        result,
        Err(git_bash::GIT_BASH_INSTALL_HINT.to_string()),
        "install_shell_from(None) must return the Git Bash install hint"
    );
}

/// When `resolve_bash_path` returns `Some(path)`, `install_shell_from`
/// passes it through as `Ok`.
#[cfg(windows)]
#[test]
fn test_install_shell_from_some_returns_path() {
    use crate::commands::install_shell_from;

    let path = std::path::PathBuf::from(r"C:\Git\bin\bash.exe");
    let result = install_shell_from(Some(path.clone()));
    assert_eq!(
        result,
        Ok(path),
        "install_shell_from(Some) must return the path as Ok"
    );
}
