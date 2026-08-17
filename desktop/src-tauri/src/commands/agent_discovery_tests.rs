//! Tests for `agent_discovery.rs`: npm-install detection, EACCES hints, adapter
//! install planning, and the post-install restart-eligibility predicate
//! (`should_restart_after_install`, including its fail-closed
//! `secrets_unavailable` guard).
//!
//! Extracted from `agent_discovery.rs` via `#[path]` so that module stays under
//! the desktop file-size ratchet; `super::*` resolves against it, matching the
//! `runtime_commands_tests.rs` convention.

use super::*;

// ── is_npm_global_install ─────────────────────────────────────────────────

#[test]
fn test_is_npm_global_install_accepts_catalog_claude_command() {
    assert!(is_npm_global_install(
        "npm install -g @agentclientprotocol/claude-agent-acp"
    ));
}

#[test]
fn test_is_npm_global_install_accepts_catalog_codex_command() {
    assert!(is_npm_global_install(
        "npm install -g @agentclientprotocol/codex-acp"
    ));
}

#[test]
fn test_is_npm_global_install_accepts_short_flag() {
    assert!(is_npm_global_install("npm i -g some-package"));
}

#[test]
fn test_is_npm_global_install_accepts_uninstall() {
    assert!(is_npm_global_install(
        "npm uninstall -g @zed-industries/codex-acp"
    ));
}

#[test]
fn test_is_npm_global_install_accepts_leading_whitespace() {
    assert!(is_npm_global_install("  npm install -g foo"));
}

#[test]
fn test_is_npm_global_install_rejects_curl_pipe() {
    assert!(!is_npm_global_install(
        "curl -fsSL https://example.com/install.sh | bash"
    ));
}

#[test]
fn test_is_npm_global_install_rejects_non_global_install() {
    assert!(!is_npm_global_install("npm install foo"));
}

#[test]
fn test_is_npm_global_install_rejects_unrelated_command() {
    assert!(!is_npm_global_install("cargo install some-tool"));
}

// ── npm_eacces_hint ───────────────────────────────────────────────────────

#[test]
fn test_npm_eacces_hint_detects_old_format() {
    let stderr = "npm ERR! code EACCES\nnpm ERR! syscall mkdir\nnpm ERR! path /usr/local/lib/node_modules\nnpm ERR! errno -13\nnpm ERR! Error: EACCES: permission denied, mkdir '/usr/local/lib/node_modules'";
    assert!(npm_eacces_hint(stderr, "npm install -g foo").is_some());
}

#[test]
fn test_npm_eacces_hint_detects_new_format() {
    let stderr = "npm error EACCES: permission denied, mkdir '/usr/local/lib/node_modules'";
    assert!(npm_eacces_hint(stderr, "npm install -g foo").is_some());
}

#[test]
fn test_npm_eacces_hint_returns_none_for_404_stderr() {
    let stderr = "npm error 404 Not Found - GET https://registry.npmjs.org/no-such-pkg";
    assert!(npm_eacces_hint(stderr, "npm install -g no-such-pkg").is_none());
}

// ── adapter_needs_install (codex version gate) ────────────────────────────

/// plan_adapter_install is the pure install-plan seam used by
/// install_acp_runtime_blocking. These tests verify:
///   - A 0.x binary (AdapterOutdated) → uninstall-then-install sequence returned
///   - A current 1.x binary (Available) → None (no reinstall)
///   - A 1.x binary below the floor → install plan returned
///   - Missing binary (None path) → catalog install commands returned
#[cfg(unix)]
#[test]
fn test_plan_adapter_install_selects_npm_command_for_outdated_0x_codex_binary() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("codex-acp");
    // Simulate old 0.16.x: --version exits non-zero (unrecognised flag)
    std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
    let plan = plan_adapter_install("codex", Some(&bin), install_cmds, Some("/usr/bin:/bin"));

    assert!(
        plan.is_some(),
        "0.x codex adapter must trigger install plan"
    );
    let cmds = plan.unwrap();
    // Outdated arm: must uninstall the old package first, then install new.
    assert_eq!(
        cmds,
        vec![
            "npm uninstall -g @zed-industries/codex-acp",
            "npm install -g @agentclientprotocol/codex-acp",
        ],
        "outdated codex adapter must produce uninstall-then-install sequence; got {cmds:?}"
    );
}

#[cfg(unix)]
#[test]
fn test_plan_adapter_install_returns_none_for_current_1x_codex_binary() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("codex-acp");
    // Simulate the minimum supported adapter version.
    std::fs::write(
        &bin,
        "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.7'\nexit 0\n",
    )
    .expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
    let plan = plan_adapter_install("codex", Some(&bin), install_cmds, Some("/usr/bin:/bin"));

    assert!(
        plan.is_none(),
        "current codex adapter must not trigger install plan (no reinstall needed)"
    );
}

#[cfg(unix)]
#[test]
fn test_plan_adapter_install_updates_older_1x_codex_binary() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("codex-acp");
    // A 1.x adapter below MIN_CODEX_ACP_VERSION must still be reinstalled.
    std::fs::write(
        &bin,
        "#!/bin/sh\necho '@agentclientprotocol/codex-acp 1.1.5'\nexit 0\n",
    )
    .expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
    let plan = plan_adapter_install("codex", Some(&bin), install_cmds, Some("/usr/bin:/bin"));

    assert!(
        plan.is_some(),
        "older 1.x codex adapter must trigger update plan"
    );
}

#[test]
fn test_plan_adapter_install_returns_catalog_cmds_when_no_adapter_path() {
    let install_cmds = &["npm install -g @agentclientprotocol/codex-acp"];
    let plan = plan_adapter_install("codex", None, install_cmds, None);
    assert!(plan.is_some(), "missing adapter must trigger install plan");
    // Missing arm: use the catalog's install commands directly (no prior
    // package to uninstall — fresh install, not a reinstall).
    assert_eq!(
        plan.unwrap(),
        vec!["npm install -g @agentclientprotocol/codex-acp"],
        "missing codex adapter must use catalog install commands only"
    );
}

#[cfg(unix)]
#[test]
fn test_plan_adapter_install_non_codex_runtime_never_reinstalls() {
    use std::os::unix::fs::PermissionsExt;

    // For non-codex runtimes, any resolved binary means no install needed.
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("goose-acp");
    std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("write script");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod script");

    let install_cmds = &["npm install -g @block/goose-acp"];
    let plan = plan_adapter_install("goose", Some(&bin), install_cmds, None);
    assert!(
        plan.is_none(),
        "non-codex runtime with resolved binary must not trigger reinstall"
    );
}

// ── should_restart_after_install ─────────────────────────────────────────

/// Setup-mode agent on matching runtime that is now Ready → restart.
#[test]
fn test_should_restart_after_install_setup_mode_now_ready_is_candidate() {
    assert!(
        should_restart_after_install(true, true, true, true, true, false),
        "setup-mode codex agent that became Ready must be restarted after install"
    );
}

/// Setup-mode agent still NotReady after install (e.g. logged out) → no restart.
#[test]
fn test_should_restart_after_install_still_not_ready_is_not_candidate() {
    assert!(
        !should_restart_after_install(true, true, true, true, false, false),
        "setup-mode agent still NotReady must NOT be restarted (would re-enter setup mode)"
    );
}

/// Healthy in-pool agent (setup_mode=false) → no restart, even if now Ready.
#[test]
fn test_should_restart_after_install_healthy_agent_is_not_candidate() {
    assert!(
        !should_restart_after_install(true, true, true, false, true, false),
        "healthy in-pool agent (setup_mode=false) must NOT be bounced on install"
    );
}

/// Agent on a different runtime_id → no restart.
#[test]
fn test_should_restart_after_install_different_runtime_is_not_candidate() {
    assert!(
        !should_restart_after_install(true, true, false, true, true, false),
        "agent on a different runtime must NOT be restarted by this install"
    );
}

/// Remote/provider-backend agent → no restart (not local).
#[test]
fn test_should_restart_after_install_non_local_is_not_candidate() {
    assert!(
        !should_restart_after_install(false, true, true, true, true, false),
        "non-local (provider-backend) agent must NOT be restarted"
    );
}

/// Dead process (pid_alive=false) → no restart.
#[test]
fn test_should_restart_after_install_dead_pid_is_not_candidate() {
    assert!(
        !should_restart_after_install(true, false, true, true, true, false),
        "agent whose process is no longer running must NOT be restarted"
    );
}

/// A setup-mode agent that would otherwise be a candidate (all five prior
/// inputs true) but whose secrets are unavailable → NO restart. An
/// unavailable tier's empty hydrated env makes readiness compute `Ready`,
/// so this is the only input distinguishing a bounce that would succeed
/// from one that would stop the process and hit the spawn refusal
/// (`FailedAfterStop`). Mutation check: drop `&& !secrets_unavailable` from
/// the predicate and this test fails while the positive control stays green.
#[test]
fn test_should_restart_after_install_secrets_unavailable_is_not_candidate() {
    assert!(
        !should_restart_after_install(true, true, true, true, true, true),
        "an otherwise-eligible setup-mode agent with unavailable secrets must NOT be bounced"
    );
}

// ── Restart-gate fixtures ─────────────────────────────────────────────────
//
// `local_record()` is the healthy baseline for the under-lock secret-gate
// tests below and (via the ops module) the production-seam binding tests. With
// `secrets_unavailable=false` and empty personas, every tier of
// `effective_secrets_unavailable` is false.

/// Minimal local `ManagedAgentRecord`: local backend, no persona link, no
/// harness pin. With `secrets_unavailable=false` and empty personas, every
/// tier of `effective_secrets_unavailable` is false — the healthy baseline
/// that makes the unavailable-tier assertions below meaningful.
fn local_record() -> crate::managed_agents::ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{}",
            "name": "restart-gate-test",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }}"#,
        "aa".repeat(32)
    ))
    .unwrap()
}

// ── Production-seam binding: refuse_restart_on_unavailable_secrets ────────
//
// The under-lock recheck calls this before the stop. Drive it with real
// records so dropping the consultation turns a regression red.

/// A healthy record passes the under-lock secret gate (Ok → proceed to stop).
#[test]
fn test_refuse_restart_on_unavailable_secrets_allows_healthy_record() {
    let record = local_record();
    assert!(
        refuse_restart_on_unavailable_secrets(&record, &[]).is_ok(),
        "a healthy record must pass the under-lock secret gate"
    );
}

/// An unavailable record is refused BEFORE the stop (Err). Mutation check:
/// replace the `effective_secrets_unavailable(..)` call with `false` and this
/// flips to Ok while the control above stays green — binds the under-lock call
/// site (the destructive stop-then-fail path Thufir flagged).
#[test]
fn test_refuse_restart_on_unavailable_secrets_refuses_unavailable_record() {
    let mut record = local_record();
    record.secrets_unavailable = true;
    let out = refuse_restart_on_unavailable_secrets(&record, &[]);
    let err = out.expect_err("an unavailable record must be refused before the stop");
    assert!(
        err.contains("secrets unavailable"),
        "the refusal must name the secret-unavailable cause: {err}"
    );
}

// ── Production-seam binding: refuse_restart_on_unavailable_global ─────────
//
// Both post-install sites and the global-config under-lock recheck route the
// strict global load through this seam. Bind it directly so restoring
// `unwrap_or_default()` (an Err mapped to Ok(default)) turns a regression red.

/// A global-load Err (committed ref unhydratable) must refuse the restart
/// rather than fall through to `unwrap_or_default()` — the pre-`FailedAfterStop`
/// class Thufir found for the global tier. Mutation check: swap the seam body
/// for `Ok(global_load.unwrap_or_default())` and this fails — Ok not Err.
#[test]
fn test_refuse_restart_on_unavailable_global_refuses_on_load_error() {
    let out = super::refuse_restart_on_unavailable_global(Err(
        "global env_vars unavailable: gen abc123 not found in keyring".to_string(),
    ));
    let err = out.expect_err("an unavailable global must refuse the restart before the stop");
    assert!(
        err.contains("global agent config unavailable") && err.contains("not found in keyring"),
        "the refusal must explain the restart is blocked and carry the loader cause: {err}"
    );
}

/// A successful global load passes through unchanged so the caller reuses it
/// for env resolution. Pins that only the Err arm is mapped.
#[test]
fn test_refuse_restart_on_unavailable_global_passes_through_available_global() {
    let global = crate::managed_agents::GlobalAgentConfig {
        model: Some("gpt-5".to_string()),
        ..Default::default()
    };
    let out = super::refuse_restart_on_unavailable_global(Ok(global.clone()));
    assert_eq!(
        out.expect("an available global must pass through").model,
        global.model,
        "the seam must return the loaded config unchanged on success"
    );
}

// ── badge availability-drift (Phase 2) ───────────────────────────────────
//
// `availability_drift` is a pure predicate over two `Option` values —
// no global state, no parallelism hazard.

/// Both sides known and different → drift detected.
#[test]
fn test_availability_drift_detected_when_stamped_differs_from_current() {
    use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
    assert!(
        availability_drift(
            Some(&AcpAvailabilityStatus::Available),
            Some(AcpAvailabilityStatus::AdapterOutdated),
        ),
        "Available stamped vs AdapterOutdated current must be detected as drift"
    );
}

/// Both sides known and equal → no drift.
#[test]
fn test_availability_drift_no_drift_when_stamped_equals_current() {
    use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
    assert!(
        !availability_drift(
            Some(&AcpAvailabilityStatus::Available),
            Some(AcpAvailabilityStatus::Available),
        ),
        "matching stamped and current must not show drift"
    );
}

/// Stamped is None (cold cache at spawn) → no drift regardless of current.
#[test]
fn test_availability_drift_none_stamp_never_drifts() {
    use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
    assert!(
        !availability_drift(None, Some(AcpAvailabilityStatus::Available)),
        "None stamp (cold cache at spawn) must never signal drift"
    );
}

/// Current is None (cache cold now) → no drift regardless of stamp.
#[test]
fn test_availability_drift_none_current_never_drifts() {
    use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
    assert!(
        !availability_drift(Some(&AcpAvailabilityStatus::Available), None),
        "None current (cache cold) must never signal drift"
    );
}

/// Non-codex agent (stamp is None) → no drift (None case).
#[test]
fn test_availability_drift_non_codex_none_never_drifts() {
    use crate::managed_agents::{availability_drift, AcpAvailabilityStatus};
    // Non-codex agents have `adapter_availability = None` — must never flip.
    assert!(
        !availability_drift(None, Some(AcpAvailabilityStatus::AdapterMissing)),
        "non-codex agent (None stamp) must never trigger drift badge"
    );
}

// ── Phase A: install shell selection ─────────────────────────────────────

/// On Unix, resolve_install_shell always succeeds (returns zsh or bash).
#[cfg(unix)]
#[test]
fn test_resolve_install_shell_succeeds_on_unix() {
    let result = super::resolve_install_shell();
    assert!(result.is_ok(), "Unix must always resolve a shell");
    let shell = result.unwrap();
    assert!(
        shell == std::path::Path::new("/bin/zsh") || shell == std::path::Path::new("/bin/bash"),
        "expected /bin/zsh or /bin/bash, got {shell:?}"
    );
}

/// install_shell_command returns a valid Command on Unix.
#[cfg(unix)]
#[test]
fn test_install_shell_command_returns_ok_on_unix() {
    let result = super::install_shell_command("echo test");
    assert!(result.is_ok(), "install_shell_command must succeed on Unix");
}

// ── pipefail: install pipes must not mask a failing left-hand side ────────

/// The command handed to the install shell must run under `set -o pipefail;`
/// with the vendor command preserved verbatim, so `curl … | bash` fails when
/// `curl` does. Platform-agnostic: only the PATH prelude differs by OS, and
/// `test_install_shell_args_shape_per_platform` pins that.
#[test]
fn test_install_shell_command_enables_pipefail() {
    let cmd = super::install_shell_command("curl -fsSL https://example.test/i.sh | bash")
        .expect("install shell must resolve on a test host");
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let body = &args[2];
    assert!(
        body.contains("set -o pipefail; "),
        "the install body must set pipefail; got: {body}"
    );
    assert!(
        body.ends_with("curl -fsSL https://example.test/i.sh | bash"),
        "the vendor command must be preserved verbatim; got: {body}"
    );
}

/// The PATH prelude is emitted only where it helps, and the exact argument
/// vector is the contract: a stray trailing positional with no `$1` reader,
/// or an export whose `$1` the shell cannot split, both corrupt PATH.
/// Windows is excluded because `join_paths` is `;`-separated there while bash
/// splits PATH on `:` — and it is the platform where the inherited fallback
/// always fires. See `install_shell_args` for the full reasoning.
#[test]
fn test_install_shell_args_shape_per_platform() {
    let composed = std::ffi::OsString::from("/buzz/node/bin:/usr/bin");
    let windows_composed = std::ffi::OsString::from(r"C:\buzz\node;C:\Windows\system32");
    let bare = ["-l", "-c", "set -o pipefail; echo hi"].map(std::ffi::OsString::from);

    assert_eq!(
        super::install_shell_args("echo hi", Some(&composed), false),
        [
            "-l",
            "-c",
            "export PATH=\"$1\"; set -o pipefail; echo hi",
            "buzz-install",
            "/buzz/node/bin:/usr/bin",
        ]
        .map(std::ffi::OsString::from),
        "Unix must re-export the composed PATH after login init"
    );
    assert_eq!(
        super::install_shell_args("echo hi", Some(&windows_composed), true),
        bare,
        "Windows must not re-export a `;`-joined PATH inside bash"
    );
    assert_eq!(
        super::install_shell_args("echo hi", None, false),
        bare,
        "no composed PATH must yield the bare pipefail body and no positionals"
    );
}

/// Regression for the login-startup-file overwrite: `cmd.env("PATH", …)` is
/// installed *before* `-l` sources the user's profile, so a profile that
/// assigns PATH silently discards the composed one. Uses `/bin/bash`
/// explicitly — the planted profile is bash-specific, so resolving the host
/// shell (which prefers zsh) would make this vacuous.
#[cfg(unix)]
#[test]
fn test_composed_path_survives_a_profile_that_clears_it() {
    let home = tempfile::tempdir().expect("temp HOME");
    std::fs::write(home.path().join(".bash_profile"), "export PATH=\n")
        .expect("plant a hostile login profile");
    let composed = std::ffi::OsString::from("/buzz/sentinel/bin:/usr/bin:/bin");

    // `echo` is a shell builtin, so the child needs no PATH to report one.
    let out = std::process::Command::new("/bin/bash")
        .args(super::install_shell_args(
            "echo \"$PATH\"",
            Some(&composed),
            false,
        ))
        .env("HOME", home.path())
        .env("PATH", &composed)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("bash must spawn");

    let path = String::from_utf8_lossy(&out.stdout);
    assert!(
        path.contains("/buzz/sentinel/bin"),
        "the composed PATH must survive login init; got: {path:?}"
    );
}

/// End-to-end on the real resolved install shell (no network): a pipeline
/// whose left-hand side fails must exit non-zero, while a fully successful
/// pipeline must still succeed. Without `pipefail` the status is the
/// right-hand side's and the left-hand failure is invisible.
#[cfg(unix)]
#[test]
fn test_install_shell_pipeline_status_follows_left_side() {
    for (command, expect_success) in [("false | true", false), ("echo ok | cat", true)] {
        let status = super::install_shell_command(command)
            .expect("Unix must always resolve an install shell")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("install shell must spawn");
        assert_eq!(
            status.success(),
            expect_success,
            "`{command}` must report success={expect_success}; got {status:?}"
        );
    }
}

// ── Phase A: Windows install shell selection ───────────────────────────────

/// On Windows (CI runner has Git pre-installed), resolve_install_shell succeeds.
#[cfg(windows)]
#[test]
fn test_resolve_install_shell_succeeds_on_windows_with_git() {
    let result = super::resolve_install_shell();
    assert!(
        result.is_ok(),
        "Windows CI runner has Git — resolve_install_shell must succeed; got: {:?}",
        result.err()
    );
    let shell = result.unwrap();
    // The resolved path must end with bash.exe (Git Bash).
    let fname = shell.file_name().and_then(|n| n.to_str()).unwrap_or("");
    assert!(
        fname.eq_ignore_ascii_case("bash.exe"),
        "Windows install shell must be bash.exe, got: {shell:?}"
    );
}

/// On Windows, when no Git Bash is found, the error carries the Doctor hint.
#[cfg(windows)]
#[test]
fn test_resolve_install_shell_error_contains_doctor_hint() {
    // We can't force resolve_install_shell to fail on CI (Git is installed),
    // but we can verify the error string it would use matches the hint.
    let hint = crate::managed_agents::git_bash::GIT_BASH_INSTALL_HINT;
    assert!(
        hint.contains("Git for Windows"),
        "GIT_BASH_INSTALL_HINT must mention Git for Windows; got: {hint}"
    );
    assert!(
        hint.contains("PATH"),
        "GIT_BASH_INSTALL_HINT must mention PATH option; got: {hint}"
    );
}

/// install_shell_command returns a valid Command on Windows.
#[cfg(windows)]
#[test]
fn test_install_shell_command_returns_ok_on_windows() {
    let result = super::install_shell_command("echo test");
    assert!(
        result.is_ok(),
        "install_shell_command must succeed on Windows with Git; got: {:?}",
        result.err()
    );
}

/// On Windows, `install_shell_command` must set PATH to a value that
/// includes the inherited process PATH, so node/npm are visible inside
/// the install shell even when no managed Node runtime is present.
#[cfg(windows)]
#[test]
fn test_install_shell_command_includes_process_path_on_windows() {
    let _guard = crate::managed_agents::lock_path_mutex();
    let previous = std::env::var_os("PATH");
    // Plant a sentinel in the process PATH that the test can detect.
    let sentinel = r"C:\TestSentinel\bin";
    std::env::set_var("PATH", sentinel);

    let result = super::install_shell_command("echo test");

    match previous {
        Some(p) => std::env::set_var("PATH", p),
        None => std::env::remove_var("PATH"),
    }

    let cmd = result.expect("install_shell_command must succeed on Windows with Git");
    let path_value = cmd
        .get_envs()
        .find(|(key, _)| *key == "PATH")
        .and_then(|(_, val)| val)
        .map(|v| v.to_string_lossy().into_owned())
        .expect("install_shell_command must always set a PATH env var on Windows");

    // The sentinel (inherited process PATH) must appear in the composed PATH.
    assert!(
        path_value.contains(sentinel),
        "install_shell_command PATH must include the inherited process PATH; got: {path_value}"
    );
    // The sentinel must appear LAST — managed Buzz dirs must have precedence.
    assert!(
            path_value.ends_with(sentinel),
            "inherited process PATH must be appended LAST so managed dirs keep precedence; got: {path_value}"
        );
}

// ── Phase B: per-OS install commands ──────────────────────────────────────

/// On non-Windows, cli_install_commands_for_os returns the default commands.
#[cfg(not(windows))]
#[test]
fn test_cli_install_commands_for_os_returns_default_on_unix() {
    let claude = crate::managed_agents::known_acp_runtime_exact("claude").unwrap();
    assert_eq!(
        claude.cli_install_commands_for_os(),
        claude.cli_install_commands,
        "on Unix, cli_install_commands_for_os must return the default install.sh commands"
    );
}

/// buzz-agent has no install commands on any platform.
#[test]
fn test_buzz_agent_has_no_install_commands() {
    let buzz = crate::managed_agents::known_acp_runtime_exact("buzz-agent").unwrap();
    assert!(
        buzz.cli_install_commands_for_os().is_empty(),
        "buzz-agent ships with the app — must never have install commands"
    );
}

// ── PowerShell routing ────────────────────────────────────────────────────

/// Commands beginning with `powershell.exe` (any casing) must be identified
/// as PowerShell commands; all others must not.
#[cfg(windows)]
#[test]
fn test_is_powershell_command_detects_powershell_commands() {
    assert!(
        super::is_powershell_command(
            r#"powershell.exe -NoProfile -NonInteractive -Command "irm https://chatgpt.com/codex/install.ps1 | iex""#
        ),
        "canonical codex install command must be detected as PowerShell"
    );
    assert!(
        super::is_powershell_command("POWERSHELL.EXE -Command foo"),
        "is_powershell_command must be case-insensitive"
    );
    assert!(
        !super::is_powershell_command("npm install -g @agentclientprotocol/claude-agent-acp"),
        "npm commands must NOT be detected as PowerShell"
    );
    assert!(
        !super::is_powershell_command(r"curl -fsSL https://example.com | bash"),
        "bash pipe commands must NOT be detected as PowerShell"
    );
    assert!(
        !super::is_powershell_command(""),
        "empty string must not be detected as PowerShell"
    );
}

/// On Windows, `build_install_command` must return a `Command` whose
/// program is `powershell.exe` (not `bash.exe`) for PowerShell commands.
#[cfg(windows)]
#[test]
fn test_build_install_command_uses_powershell_natively_on_windows() {
    let ps_command = r#"powershell.exe -NoProfile -NonInteractive -Command "irm https://chatgpt.com/codex/install.ps1 | iex""#;
    let result = super::build_install_command(ps_command);
    assert!(
        result.is_ok(),
        "build_install_command must succeed for a PowerShell command; got: {:?}",
        result.err()
    );
    let cmd = result.unwrap();
    let program = cmd.get_program().to_string_lossy().to_lowercase();
    assert!(
        program.contains("powershell"),
        "PowerShell install command must use powershell.exe, not bash; got: {program}"
    );
    assert!(
        !program.contains("bash"),
        "PowerShell install command must NOT go through bash; got: {program}"
    );
}

/// On Windows, `build_install_command` must route non-PowerShell commands
/// through Git Bash (program must be bash.exe).
#[cfg(windows)]
#[test]
fn test_build_install_command_uses_git_bash_for_non_powershell_on_windows() {
    let npm_command = "npm install -g @agentclientprotocol/claude-agent-acp";
    let result = super::build_install_command(npm_command);
    assert!(
        result.is_ok(),
        "build_install_command must succeed for an npm command on Windows with Git; got: {:?}",
        result.err()
    );
    let cmd = result.unwrap();
    let program = cmd.get_program().to_string_lossy().to_lowercase();
    assert!(
        program.contains("bash"),
        "non-PowerShell install command must still use bash.exe on Windows; got: {program}"
    );
}

/// On non-Windows, `build_install_command` must always use the Unix shell
/// (zsh or bash), never powershell.exe.
#[cfg(not(windows))]
#[test]
fn test_build_install_command_uses_unix_shell_on_non_windows() {
    let command = r"curl -fsSL https://example.com/install.sh | bash";
    let result = super::build_install_command(command);
    assert!(
        result.is_ok(),
        "build_install_command must succeed on Unix; got: {:?}",
        result.err()
    );
    let cmd = result.unwrap();
    let program = cmd.get_program().to_string_lossy();
    assert!(
        program.contains("bash") || program.contains("zsh"),
        "Unix install command must use bash or zsh, got: {program}"
    );
}

/// On Windows, `install_powershell_command` must build an exact argv:
/// flags before `-Command` forwarded, body unquoted (outer catalog quotes stripped),
/// no bash flags, and `-Command` found on token boundary not as substring.
#[cfg(windows)]
#[test]
fn test_powershell_command_argv_exact() {
    // Catalog format: body wrapped in one outer double-quote pair (Bash-layer serialization).
    let body = "$ErrorActionPreference='Stop'; $installer=Join-Path $env:TEMP 'buzz-install-codex.ps1'; Invoke-RestMethod https://chatgpt.com/codex/install.ps1 -OutFile $installer; & $installer; exit $LASTEXITCODE";
    let cmd = super::install_powershell_command(&format!(
        r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "{body}""#
    ));
    assert_eq!(
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", body],
        "argv must be exact with outer quotes stripped"
    );
}

/// Token that merely contains `-command` as a substring must not be treated
/// as the `-Command` boundary; only an exact token match (case-insensitive) counts.
#[cfg(windows)]
#[test]
fn test_powershell_command_token_boundary_not_substring() {
    let cmd =
        super::install_powershell_command(r#"powershell.exe -x-command-y -Command "echo hello""#);
    assert_eq!(
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["-x-command-y", "-Command", "echo hello"],
        "substring must not consume -Command boundary early"
    );
}

/// Claude Code catalog command must dequote to the two-step download-then-execute body.
#[cfg(windows)]
#[test]
fn test_powershell_command_claude_catalog_dequoted() {
    let cmd = super::install_powershell_command(
        r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference='Stop'; $installer=Join-Path $env:TEMP 'buzz-install-claude.ps1'; Invoke-RestMethod https://claude.ai/install.ps1 -OutFile $installer; & $installer; exit $LASTEXITCODE""#,
    );
    assert_eq!(
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "$ErrorActionPreference='Stop'; $installer=Join-Path $env:TEMP 'buzz-install-claude.ps1'; Invoke-RestMethod https://claude.ai/install.ps1 -OutFile $installer; & $installer; exit $LASTEXITCODE",
            ],
            "Claude catalog command must be dequoted correctly"
        );
}

/// Goose Windows catalog command must dequote to the two-step download-then-execute body
/// with the `$env:CONFIGURE` prefix intact — no backslash before the dollar sign.
/// This proves the `\$` → `$` contract: post-#2750 the spawn is native and
/// PowerShell receives the body verbatim, so a residual `\` would produce
/// `\$env:CONFIGURE='false'` which is a malformed statement.
#[cfg(windows)]
#[test]
fn test_powershell_command_goose_catalog_dequoted() {
    let cmd = super::install_powershell_command(
        r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "$env:CONFIGURE='false'; $ErrorActionPreference='Stop'; $installer=Join-Path $env:TEMP 'buzz-install-goose.ps1'; Invoke-RestMethod https://raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1 -OutFile $installer; & $installer; exit $LASTEXITCODE""#,
    );
    assert_eq!(
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "$env:CONFIGURE='false'; $ErrorActionPreference='Stop'; $installer=Join-Path $env:TEMP 'buzz-install-goose.ps1'; Invoke-RestMethod https://raw.githubusercontent.com/aaif-goose/goose/main/download_cli.ps1 -OutFile $installer; & $installer; exit $LASTEXITCODE",
            ],
            "Goose catalog command must dequote with bare $env: (no backslash before $)"
        );
}
