use std::collections::BTreeMap;

use super::*;
use crate::managed_agents::discovery::known_acp_runtime_exact;

/// Build a minimal `EffectiveAgentEnv` with the given env map and command.
fn make_env(command: &str, env: BTreeMap<String, String>) -> EffectiveAgentEnv {
    let runtime = known_acp_runtime_exact(command);
    EffectiveAgentEnv {
        env,
        config_file_path: runtime.and_then(|r| r.config_file_path),
        effective_command: command.to_string(),
    }
}

fn env_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ── buzz-agent tests ──────────────────────────────────────────────────

#[test]
fn buzz_agent_missing_provider_returns_not_ready_with_normalized_field() {
    let env = make_env(
        "buzz-agent",
        env_with(&[("BUZZ_AGENT_MODEL", "claude-opus-4-5")]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "missing BUZZ_AGENT_PROVIDER should be NotReady"
    );
    let reqs = result.requirements();
    assert!(
        reqs.contains(&Requirement::NormalizedField {
            field: "provider".to_string()
        }),
        "requirements should include NormalizedField(provider); got {reqs:?}"
    );
}

#[test]
fn buzz_agent_missing_model_returns_not_ready_with_normalized_field() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ("ANTHROPIC_API_KEY", "sk-test"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(!result.is_ready());
    assert!(result
        .requirements()
        .contains(&Requirement::NormalizedField {
            field: "model".to_string()
        }));
}

#[test]
fn buzz_agent_missing_anthropic_key_returns_not_ready_with_env_key() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ("BUZZ_AGENT_MODEL", "claude-opus-4-5"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(!result.is_ready());
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "ANTHROPIC_API_KEY".to_string()
    }));
}

#[test]
fn buzz_agent_missing_openai_key_returns_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "openai"),
            ("BUZZ_AGENT_MODEL", "gpt-4o"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(!result.is_ready());
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "OPENAI_COMPAT_API_KEY".to_string()
    }));
}

#[test]
fn buzz_agent_missing_gemini_key_returns_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "gemini"),
            ("BUZZ_AGENT_MODEL", "gemini-2.5-flash"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(!result.is_ready());
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "GEMINI_API_KEY".to_string()
    }));
}

#[test]
fn buzz_agent_gemini_with_all_fields_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "gemini"),
            ("BUZZ_AGENT_MODEL", "gemini-2.5-flash"),
            ("GEMINI_API_KEY", "AIza-test"),
        ]),
    );
    assert!(agent_readiness(&env).is_ready());
}

#[test]
fn buzz_agent_gemini_model_fallback_env_key_is_ready() {
    // GEMINI_MODEL (not BUZZ_AGENT_MODEL) should satisfy the model requirement,
    // mirroring the provider-specific fallback for the other providers.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "gemini"),
            ("GEMINI_MODEL", "gemini-2.5-pro"),
            ("GEMINI_API_KEY", "AIza-test"),
        ]),
    );
    assert!(agent_readiness(&env).is_ready());
}

#[test]
fn buzz_agent_anthropic_with_all_fields_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ("BUZZ_AGENT_MODEL", "claude-opus-4-5"),
            ("ANTHROPIC_API_KEY", "sk-test"),
        ]),
    );
    assert!(agent_readiness(&env).is_ready());
}

#[test]
fn buzz_agent_databricks_with_host_and_model_is_ready_without_token() {
    // DATABRICKS_TOKEN is NOT required — OAuth PKCE is the normal path.
    // No token present, no OAuth cache present → still Ready because we
    // cannot evaluate OAuth state from the env map alone.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks"),
            ("BUZZ_AGENT_MODEL", "dbrx-instruct"),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
            // NOTE: no DATABRICKS_TOKEN
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "Databricks with HOST+model but no TOKEN should still be Ready (OAuth path)"
    );
}

#[test]
fn buzz_agent_databricks_missing_host_returns_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks"),
            ("BUZZ_AGENT_MODEL", "dbrx-instruct"),
            // NOTE: no DATABRICKS_HOST
        ]),
    );
    let result = agent_readiness(&env);
    assert!(!result.is_ready());
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "DATABRICKS_HOST".to_string()
    }));
}

#[test]
fn buzz_agent_databricks_v2_missing_host_returns_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
            (
                "BUZZ_AGENT_MODEL",
                "databricks/meta-llama-4-maverick-17b-instruct",
            ),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(!result.is_ready());
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "DATABRICKS_HOST".to_string()
    }));
}

// ── goose tests ───────────────────────────────────────────────────────

#[test]
fn goose_missing_provider_returns_not_ready() {
    // Call goose_requirements directly with None file config so the test is
    // deterministic — the `agent_readiness` path reads the real
    // ~/.config/goose/config.yaml which may silence requirements on
    // developer machines.
    let env = make_env("goose", env_with(&[("GOOSE_MODEL", "claude-opus-4-5")]));
    let reqs = goose_requirements(&env, None);
    assert!(
        !reqs.is_empty(),
        "missing GOOSE_PROVIDER with no file config must produce requirements"
    );
    assert!(
        reqs.contains(&Requirement::NormalizedField {
            field: "provider".to_string()
        }),
        "requirements must include NormalizedField(provider); got {reqs:?}"
    );
}

#[test]
fn goose_with_provider_and_model_and_key_is_ready() {
    let env = make_env(
        "goose",
        env_with(&[
            ("GOOSE_PROVIDER", "anthropic"),
            ("GOOSE_MODEL", "claude-opus-4-5"),
            ("ANTHROPIC_API_KEY", "sk-test"),
        ]),
    );
    assert!(agent_readiness(&env).is_ready());
}

// ── empty-string semantics ────────────────────────────────────────────
//
// A key present with an empty value ("") must be treated as MISSING, to
// match the dialog's (envVars[key] ?? "").length === 0 emptiness check.

#[test]
fn buzz_agent_empty_string_provider_is_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", ""),
            ("BUZZ_AGENT_MODEL", "claude-opus-4-5"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "empty-string BUZZ_AGENT_PROVIDER must be treated as missing"
    );
    assert!(result
        .requirements()
        .contains(&Requirement::NormalizedField {
            field: "provider".to_string()
        }));
}

#[test]
fn buzz_agent_empty_string_model_is_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ("BUZZ_AGENT_MODEL", ""),
            ("ANTHROPIC_API_KEY", "sk-test"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "empty-string BUZZ_AGENT_MODEL must be treated as missing"
    );
    assert!(result
        .requirements()
        .contains(&Requirement::NormalizedField {
            field: "model".to_string()
        }));
}

#[test]
fn buzz_agent_empty_string_anthropic_key_is_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ("BUZZ_AGENT_MODEL", "claude-opus-4-5"),
            ("ANTHROPIC_API_KEY", ""),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "empty-string ANTHROPIC_API_KEY must be treated as missing"
    );
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "ANTHROPIC_API_KEY".to_string()
    }));
}

#[test]
fn buzz_agent_empty_string_databricks_host_is_not_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks"),
            ("BUZZ_AGENT_MODEL", "dbrx-instruct"),
            ("DATABRICKS_HOST", ""),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "empty-string DATABRICKS_HOST must be treated as missing"
    );
    assert!(result.requirements().contains(&Requirement::EnvKey {
        key: "DATABRICKS_HOST".to_string()
    }));
}

#[test]
fn goose_empty_string_provider_is_not_ready() {
    // Call goose_requirements directly with None file config so the test is
    // deterministic — the `agent_readiness` path reads the real
    // ~/.config/goose/config.yaml which may silence requirements on
    // developer machines.
    let env = make_env(
        "goose",
        env_with(&[("GOOSE_PROVIDER", ""), ("GOOSE_MODEL", "claude-opus-4-5")]),
    );
    let reqs = goose_requirements(&env, None);
    assert!(
        !reqs.is_empty(),
        "empty-string GOOSE_PROVIDER must be treated as missing"
    );
    assert!(
        reqs.contains(&Requirement::NormalizedField {
            field: "provider".to_string()
        }),
        "requirements must include NormalizedField(provider); got {reqs:?}"
    );
}

#[test]
fn goose_empty_string_anthropic_key_is_not_ready() {
    // Call goose_requirements directly with None file config so the test is
    // deterministic — the `agent_readiness` path reads the real
    // ~/.config/goose/config.yaml which may silence requirements on
    // developer machines.
    let env = make_env(
        "goose",
        env_with(&[
            ("GOOSE_PROVIDER", "anthropic"),
            ("GOOSE_MODEL", "claude-opus-4-5"),
            ("ANTHROPIC_API_KEY", ""),
        ]),
    );
    let reqs = goose_requirements(&env, None);
    assert!(
        !reqs.is_empty(),
        "empty-string ANTHROPIC_API_KEY must be treated as missing (goose)"
    );
    assert!(
        reqs.contains(&Requirement::EnvKey {
            key: "ANTHROPIC_API_KEY".to_string()
        }),
        "requirements must include ANTHROPIC_API_KEY; got {reqs:?}"
    );
}

// ── goose google (Gemini) provider tests ──────────────────────────────

#[test]
fn goose_google_provider_missing_key_returns_not_ready() {
    // Goose's native Gemini provider (`google`) requires GOOGLE_API_KEY. With
    // provider + model set but no key, readiness must surface exactly that key.
    let env = make_env(
        "goose",
        env_with(&[
            ("GOOSE_PROVIDER", "google"),
            ("GOOSE_MODEL", "gemini-2.5-pro"),
        ]),
    );
    let reqs = goose_requirements(&env, None);
    assert_eq!(
        reqs,
        vec![Requirement::EnvKey {
            key: "GOOGLE_API_KEY".to_string(),
        }],
        "goose+google without a key must require GOOGLE_API_KEY; got {reqs:?}"
    );
}

#[test]
fn goose_google_provider_with_all_fields_is_ready() {
    let env = make_env(
        "goose",
        env_with(&[
            ("GOOSE_PROVIDER", "google"),
            ("GOOSE_MODEL", "gemini-2.5-pro"),
            ("GOOGLE_API_KEY", "secret"),
        ]),
    );
    let reqs = goose_requirements(&env, None);
    assert!(
        reqs.is_empty(),
        "goose+google with GOOGLE_API_KEY, provider and model must be ready; got {reqs:?}"
    );
}

#[test]
fn goose_anthropic_provider_remains_ready() {
    // Regression: adding the google arm must not affect other goose providers.
    let env = make_env(
        "goose",
        env_with(&[
            ("GOOSE_PROVIDER", "anthropic"),
            ("GOOSE_MODEL", "claude-opus-4-5"),
            ("ANTHROPIC_API_KEY", "secret"),
        ]),
    );
    let reqs = goose_requirements(&env, None);
    assert!(
        reqs.is_empty(),
        "goose+anthropic must be ready; got {reqs:?}"
    );
}

#[test]
fn buzz_agent_gemini_provider_remains_supported() {
    // Gemini IS a first-class buzz-agent provider (OpenAI-compatible,
    // GEMINI_API_KEY) — distinct from goose's `google` provider. buzz-agent
    // readiness must remain unchanged.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "gemini"),
            ("BUZZ_AGENT_MODEL", "models/gemini-3.5-flash"),
            ("GEMINI_API_KEY", "secret"),
        ]),
    );
    let reqs = buzz_agent_requirements(&env);
    assert!(
        reqs.is_empty(),
        "buzz-agent+gemini with key+model must be ready; got {reqs:?}"
    );
}

// ── codex tests ───────────────────────────────────────────────────────

#[test]
fn codex_not_ready_copy_does_not_mention_openai_api_key() {
    // codex uses its own credential store via `codex login` (OAuth or API key).
    // The nudge copy must NOT say "set OPENAI_API_KEY".
    // Use a not-installed runtime so the requirement is always emitted
    // regardless of whether codex is on the test machine's PATH.
    let rt = make_cli_runtime(&["__buzz_nonexistent_adapter_xyz789__"], None);
    let reqs = cli_login::requirements(&["codex", "login", "status"], "run `codex login`", &rt);
    // Whether codex is installed or not, the copy (if any) must not mention OPENAI_API_KEY.
    for req in &reqs {
        if let Requirement::CliLogin { setup_copy, .. } = req {
            assert!(
                !setup_copy.contains("OPENAI_API_KEY"),
                "codex nudge copy must not mention OPENAI_API_KEY; got: {setup_copy:?}"
            );
            assert!(
                setup_copy.contains("codex login"),
                "codex nudge copy should mention `codex login`; got: {setup_copy:?}"
            );
        }
    }
}

// ── cli_login_requirements: resolve_command integration ─────────────

/// Construct a minimal `KnownAcpRuntime` stub for testing cli_login_requirements.
/// `commands` are the adapter binaries; `underlying_cli` is the CLI name.
fn make_cli_runtime(
    commands: &'static [&'static str],
    underlying_cli: Option<&'static str>,
) -> KnownAcpRuntime {
    KnownAcpRuntime {
        id: "test-cli-runtime",
        label: "Test CLI",
        commands,
        aliases: &[],
        avatar_url: "",
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli,
        cli_install_commands: &[],
        cli_install_commands_windows: &[],
        adapter_install_commands: &[],
        install_instructions_url: "",
        cli_install_hint: "",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: false,
        config_file_path: None,
        config_file_format: None,
        model_env_var: None,
        provider_env_var: None,
        provider_locked: false,
        default_env: &[],
        supports_acp_native_config: false,
        thinking_env_var: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        required_normalized_fields: &[],
        login_hint: None,
        auth_probe_args: None,
    }
}

/// Returns the absolute path of the currently-running test binary as a
/// `&'static str`.  Host-portable stand-in for a "present" binary:
/// the path is absolute so `find_command` resolves it via `path.exists()`
/// rather than searching `PATH`, and the file always exists on the host.
///
/// The tiny allocation is intentionally leaked — this runs at most once per
/// test process and the process exits immediately after tests complete.
fn present_binary_str() -> &'static str {
    let path = std::env::current_exe().expect("current_exe must be available in tests");
    Box::leak(path.to_string_lossy().into_owned().into_boxed_str())
}

/// Leak a runtime slice of `'static` strs for use in `make_cli_runtime`.
fn static_commands(commands: Vec<&'static str>) -> &'static [&'static str] {
    Box::leak(commands.into_boxed_slice())
}

#[test]
fn cli_login_requirements_missing_binary_is_not_ready() {
    // Both adapter and underlying CLI are nonexistent → NotInstalled state
    // → must return a CliLogin requirement with availability=NotInstalled.
    let rt = make_cli_runtime(
        &["__buzz_nonexistent_adapter_abc123__"],
        Some("__buzz_nonexistent_cli_abc123__"),
    );
    let reqs = cli_login::requirements(
        &["__buzz_nonexistent_binary_abc123__", "status"],
        "install the tool first",
        &rt,
    );
    assert!(
        !reqs.is_empty(),
        "missing binary must produce a CliLogin requirement (NotReady)"
    );
    assert!(
        matches!(reqs[0], Requirement::CliLogin { .. }),
        "requirement must be CliLogin; got {:?}",
        reqs[0]
    );
    if let Requirement::CliLogin {
        ref availability, ..
    } = reqs[0]
    {
        assert_eq!(
            *availability,
            crate::managed_agents::AcpAvailabilityStatus::NotInstalled,
            "both missing → NotInstalled"
        );
    }
}

#[test]
fn cli_login_requirements_adapter_missing_emits_adapter_missing() {
    // Underlying CLI present (use the running test binary as a portable
    // stand-in — it's always present and resolves via absolute path),
    // adapter absent.
    // → AdapterMissing state → no probe run → CliLogin{AdapterMissing}.
    let exe = present_binary_str();
    let rt = make_cli_runtime(&["__buzz_nonexistent_adapter_xyz789__"], Some(exe));
    let reqs = cli_login::requirements(&[exe, "--list"], "install the adapter", &rt);
    assert!(
        !reqs.is_empty(),
        "adapter missing must produce a CliLogin requirement"
    );
    if let Requirement::CliLogin {
        ref availability, ..
    } = reqs[0]
    {
        assert_eq!(
            *availability,
            crate::managed_agents::AcpAvailabilityStatus::AdapterMissing,
            "adapter absent, CLI present → AdapterMissing"
        );
    }
}

#[test]
fn cli_login_requirements_cli_missing_emits_cli_missing() {
    // Adapter present (use the running test binary as a portable stand-in),
    // underlying CLI absent.
    // → CliMissing state → no probe run → CliLogin{CliMissing}.
    let exe = present_binary_str();
    let rt = make_cli_runtime(
        static_commands(vec![exe]),              // adapter found via absolute path
        Some("__buzz_nonexistent_cli_abc123__"), // underlying CLI missing
    );
    let reqs = cli_login::requirements(&[exe, "--list"], "install the CLI", &rt);
    assert!(
        !reqs.is_empty(),
        "CLI missing must produce a CliLogin requirement"
    );
    if let Requirement::CliLogin {
        ref availability, ..
    } = reqs[0]
    {
        assert_eq!(
            *availability,
            crate::managed_agents::AcpAvailabilityStatus::CliMissing,
            "adapter present, CLI absent → CliMissing"
        );
    }
}

#[test]
fn cli_login_requirements_resolvable_binary_runs_probe_at_resolved_path() {
    // Both adapter and CLI present (use the running test binary as a
    // portable stand-in — always present, resolves via absolute path),
    // probe exits 0 (run with `--list` which lists tests and exits 0).
    // → logged_in = true → requirements is empty (Ready).
    let exe = present_binary_str();
    let rt = make_cli_runtime(static_commands(vec![exe]), Some(exe));
    let reqs = cli_login::requirements(
        &[exe, "--list"],
        "this should not show (probe exits 0)",
        &rt,
    );
    assert!(
        reqs.is_empty(),
        "expected Ready (no requirements) when probe binary resolves and exits 0; \
             got {:?}",
        reqs
    );
}

#[test]
fn cli_login_requirements_logged_out_emits_available() {
    // Both adapter and CLI present, but probe exits non-zero (logged out).
    // Use the test binary with an unrecognized argument as the probe —
    // libtest exits non-zero for unknown flags on all platforms.
    // → CliLogin{Available} (tooling installed, needs login).
    let exe = present_binary_str();
    let rt = make_cli_runtime(static_commands(vec![exe]), Some(exe));
    let reqs = cli_login::requirements(&[exe, "--buzz-probe-fail-xyz"], "run `tool login`", &rt);
    assert!(
        !reqs.is_empty(),
        "non-zero probe must produce a CliLogin requirement (logged out)"
    );
    if let Requirement::CliLogin {
        ref availability, ..
    } = reqs[0]
    {
        assert_eq!(
            *availability,
            crate::managed_agents::AcpAvailabilityStatus::Available,
            "tooling installed, probe fails → Available (logged-out)"
        );
    }
}

// ── codex readiness version gate ───────────────────────────────────────

/// Build a minimal `KnownAcpRuntime` for testing the codex version gate.
/// `adapter_commands` are the exact strings passed to `find_command` — use
/// `&["codex-acp"]` when the binary is on PATH, or `&[<absolute_path>]`
/// when resolving via absolute path.  `underlying_cli` is a portable
/// stand-in so the adapter is not misclassified as `CliMissing`.
fn make_codex_runtime(
    adapter_commands: &'static [&'static str],
    underlying_cli: Option<&'static str>,
) -> KnownAcpRuntime {
    KnownAcpRuntime {
        id: "codex",
        label: "Codex",
        commands: adapter_commands,
        aliases: &[],
        avatar_url: "",
        mcp_command: None,
        mcp_hooks: false,
        underlying_cli,
        cli_install_commands: &[],
        cli_install_commands_windows: &[],
        adapter_install_commands: &[],
        install_instructions_url: "",
        cli_install_hint: "",
        adapter_install_hint: "",
        skill_dir: None,
        supports_acp_model_switching: false,
        config_file_path: None,
        config_file_format: None,
        model_env_var: None,
        provider_env_var: None,
        provider_locked: false,
        default_env: &[],
        supports_acp_native_config: false,
        thinking_env_var: None,
        max_tokens_env_var: None,
        context_limit_env_var: None,
        required_normalized_fields: &[],
        login_hint: None,
        auth_probe_args: None,
    }
}

/// Build a temp dir containing a `codex-acp` script with the given body,
/// prepend it to PATH, and clear the resolve cache.  Returns the temp dir
/// and the original PATH string for restoration.
#[cfg(unix)]
fn setup_temp_codex_acp(script_body: &str) -> (tempfile::TempDir, String) {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("create temp dir");
    let bin = dir.path().join("codex-acp");
    std::fs::write(&bin, script_body).expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", dir.path().display(), original_path);
    std::env::set_var("PATH", &new_path);
    crate::managed_agents::clear_resolve_cache();

    (dir, original_path)
}

#[cfg(unix)]
fn leaked_adapter_commands(bin: &std::path::Path) -> &'static [&'static str] {
    let command = Box::leak(bin.display().to_string().into_boxed_str());
    Box::leak(vec![command as &'static str].into_boxed_slice())
}

/// Restore PATH and clear the resolve cache after a PATH-mutating test.
#[cfg(unix)]
fn restore_path(original: &str) {
    std::env::set_var("PATH", original);
    crate::managed_agents::clear_resolve_cache();
}

/// Codex readiness: outdated adapter (exits non-zero) → AdapterOutdated,
/// login probe skipped.
#[cfg(unix)]
#[test]
fn cli_login_requirements_codex_outdated_adapter_emits_adapter_outdated() {
    let _guard = crate::managed_agents::lock_path_mutex();

    let (dir, orig) = setup_temp_codex_acp("#!/bin/sh\nexit 1\n");
    let exe = present_binary_str();
    // Use the fixture's absolute adapter path here. Bare `codex-acp`
    // intentionally prefers Buzz's managed npm shim when it exists, which
    // would make this version-gate regression test depend on machine state.
    let rt = make_codex_runtime(
        leaked_adapter_commands(&dir.path().join("codex-acp")),
        Some(exe),
    );
    let reqs = cli_login::requirements(
        &[exe, "--buzz-probe-must-not-run-xyz"],
        "run `codex login`",
        &rt,
    );

    restore_path(&orig);
    drop(dir);

    assert!(
        !reqs.is_empty(),
        "outdated codex adapter must produce a requirement; got {reqs:?}"
    );
    if let Requirement::CliLogin {
        ref availability, ..
    } = reqs[0]
    {
        assert_eq!(
            *availability,
            crate::managed_agents::AcpAvailabilityStatus::AdapterOutdated,
            "0.x codex adapter must yield AdapterOutdated; got {availability:?}"
        );
    } else {
        panic!("expected CliLogin requirement; got {:?}", reqs[0]);
    }
}

/// Codex readiness: adapter exits 0 but output is not a parseable version
/// → AdapterOutdated (garbage output treated as outdated, same as non-zero).
#[cfg(unix)]
#[test]
fn cli_login_requirements_codex_garbage_version_output_emits_adapter_outdated() {
    let _guard = crate::managed_agents::lock_path_mutex();

    let (dir, orig) = setup_temp_codex_acp("#!/bin/sh\necho 'not a version string'\nexit 0\n");
    let exe = present_binary_str();
    let rt = make_codex_runtime(
        leaked_adapter_commands(&dir.path().join("codex-acp")),
        Some(exe),
    );
    let reqs = cli_login::requirements(
        &[exe, "--buzz-probe-must-not-run-xyz"],
        "run `codex login`",
        &rt,
    );

    restore_path(&orig);
    drop(dir);

    assert!(
        !reqs.is_empty(),
        "garbage version output must produce a requirement; got {reqs:?}"
    );
    if let Requirement::CliLogin {
        ref availability, ..
    } = reqs[0]
    {
        assert_eq!(
            *availability,
            crate::managed_agents::AcpAvailabilityStatus::AdapterOutdated,
            "unparseable version output must yield AdapterOutdated; got {availability:?}"
        );
    } else {
        panic!("expected CliLogin requirement; got {:?}", reqs[0]);
    }
}

// ── custom/unknown command ─────────────────────────────────────────────

#[test]
fn unknown_command_is_always_ready() {
    let env = make_env("my-custom-harness", BTreeMap::new());
    assert!(
        agent_readiness(&env).is_ready(),
        "unknown/custom command should always be Ready (no requirements)"
    );
}

// ── AgentReadiness helpers ─────────────────────────────────────────────

#[test]
fn agent_readiness_ready_has_empty_requirements() {
    assert!(AgentReadiness::Ready.requirements().is_empty());
}

#[test]
fn agent_readiness_not_ready_exposes_requirements() {
    let r = AgentReadiness::NotReady {
        requirements: vec![Requirement::EnvKey {
            key: "FOO".to_string(),
        }],
    };
    assert!(!r.is_ready());
    assert_eq!(r.requirements().len(), 1);
}

// ── Requirement serialization ─────────────────────────────────────────

#[test]
fn requirement_serializes_with_surface_tag() {
    let r = Requirement::NormalizedField {
        field: "provider".to_string(),
    };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["surface"], "normalized_field");
    assert_eq!(json["field"], "provider");
}

#[test]
fn git_bash_requirement_serializes_correctly() {
    let json = serde_json::to_value(Requirement::GitBash).unwrap();
    assert_eq!(json, serde_json::json!({ "surface": "git_bash" }));
}

#[test]
fn env_key_requirement_serializes_correctly() {
    let r = Requirement::EnvKey {
        key: "ANTHROPIC_API_KEY".to_string(),
    };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["surface"], "env_key");
    assert_eq!(json["key"], "ANTHROPIC_API_KEY");
}

#[test]
fn cli_login_requirement_serializes_correctly() {
    let r = Requirement::CliLogin {
        probe_args: vec![
            "codex".to_string(),
            "login".to_string(),
            "status".to_string(),
        ],
        setup_copy: "run `codex login`".to_string(),
        availability: crate::managed_agents::AcpAvailabilityStatus::Available,
    };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["surface"], "cli_login");
    assert!(json["probe_args"].is_array());
    assert!(json["setup_copy"].as_str().unwrap().contains("codex login"));
}

// ── resolve_effective_agent_env ─────────────────────────────────────────

#[test]
fn resolve_effective_agent_env_user_env_wins_over_structured_fields() {
    // A record whose env_vars explicitly set provider/model must win over
    // any baked defaults. In OSS test builds the baked map is empty, so
    // this test validates the user-env layer is present in the output.
    let mut env_vars = BTreeMap::new();
    env_vars.insert("BUZZ_AGENT_PROVIDER".to_string(), "anthropic".to_string());
    env_vars.insert(
        "BUZZ_AGENT_MODEL".to_string(),
        "claude-opus-4-5".to_string(),
    );

    // Minimal record: only the fields resolve_effective_agent_env reads.
    let record = crate::managed_agents::types::ManagedAgentRecord {
        pubkey: "test-pubkey".to_string(),
        name: "test-agent".to_string(),
        persona_id: None,
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: "buzz-acp".to_string(),
        agent_command: "buzz-agent".to_string(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        env_vars,
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
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
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
    };

    let runtime = known_acp_runtime_exact("buzz-agent");
    let effective = resolve_effective_agent_env(&record, &[], runtime, &Default::default());

    // User env_vars must be present in the output (last-write-wins).
    assert_eq!(
        effective.env.get("BUZZ_AGENT_PROVIDER").map(String::as_str),
        Some("anthropic")
    );
    assert_eq!(
        effective.env.get("BUZZ_AGENT_MODEL").map(String::as_str),
        Some("claude-opus-4-5")
    );
}

// ── provider-specific model fallback tests ────────────────────────────

#[test]
fn buzz_agent_databricks_v2_with_databricks_model_but_no_buzz_agent_model_is_ready() {
    // The baked buzz-releases env sets DATABRICKS_MODEL but not BUZZ_AGENT_MODEL.
    // An agent with only DATABRICKS_MODEL must pass the readiness gate.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
            ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "DATABRICKS_MODEL must satisfy the model requirement for databricks_v2"
    );
}

#[test]
fn buzz_agent_databricks_v2_hyphen_alias_with_databricks_model_is_ready() {
    // buzz-agent accepts both "databricks_v2" and "databricks-v2". The
    // readiness gate must recognize the hyphen alias and accept DATABRICKS_MODEL.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks-v2"),
            ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "databricks-v2 alias with DATABRICKS_MODEL must be Ready"
    );
}

#[test]
fn buzz_agent_databricks_hyphen_alias_missing_host_returns_not_ready() {
    // The hyphen alias "databricks-v2" requires DATABRICKS_HOST just like
    // the underscore variants. Without it the agent cannot reach the endpoint.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks-v2"),
            ("DATABRICKS_MODEL", "goose-claude-4-6-sonnet"),
            // DATABRICKS_HOST intentionally absent
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "databricks-v2 without DATABRICKS_HOST must be NotReady"
    );
    let reqs = result.requirements();
    assert!(
        reqs.iter()
            .any(|r| matches!(r, Requirement::EnvKey { key } if key == "DATABRICKS_HOST")),
        "missing requirements must include DATABRICKS_HOST; got {reqs:?}"
    );
}

#[test]
fn buzz_agent_databricks_v1_with_databricks_model_but_no_buzz_agent_model_is_ready() {
    // V1 (Model Serving) also resolves DATABRICKS_MODEL — same fallback applies.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks"),
            ("DATABRICKS_MODEL", "dbrx-instruct"),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "DATABRICKS_MODEL must satisfy the model requirement for databricks (V1)"
    );
}

#[test]
fn buzz_agent_anthropic_with_anthropic_model_but_no_buzz_agent_model_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "anthropic"),
            ("ANTHROPIC_MODEL", "claude-opus-4-5"),
            ("ANTHROPIC_API_KEY", "sk-test"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "ANTHROPIC_MODEL must satisfy the model requirement for anthropic"
    );
}

#[test]
fn buzz_agent_openai_with_openai_compat_model_but_no_buzz_agent_model_is_ready() {
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "openai"),
            ("OPENAI_COMPAT_MODEL", "gpt-4o"),
            ("OPENAI_COMPAT_API_KEY", "sk-test"),
        ]),
    );
    assert!(
        agent_readiness(&env).is_ready(),
        "OPENAI_COMPAT_MODEL must satisfy the model requirement for openai"
    );
}

#[test]
fn buzz_agent_empty_provider_model_fallback_key_is_not_ready() {
    // An empty DATABRICKS_MODEL with no BUZZ_AGENT_MODEL must still be NotReady.
    let env = make_env(
        "buzz-agent",
        env_with(&[
            ("BUZZ_AGENT_PROVIDER", "databricks_v2"),
            ("DATABRICKS_MODEL", ""),
            ("DATABRICKS_HOST", "https://dbc.example.com"),
        ]),
    );
    let result = agent_readiness(&env);
    assert!(
        !result.is_ready(),
        "empty DATABRICKS_MODEL with no BUZZ_AGENT_MODEL must be NotReady"
    );
    assert!(result
        .requirements()
        .contains(&Requirement::NormalizedField {
            field: "model".to_string()
        }));
}
