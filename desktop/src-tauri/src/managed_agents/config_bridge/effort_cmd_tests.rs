//! Command-boundary strip and production-sequence seam tests for effort.
//!
//! Split from `effort_tests.rs` to stay within the file-size ratchet.
//! Covers `strip_effort_keys_from_command` tombstone assertions and the
//! child-process spawn sequence via `apply_effort_to_spawn_command`.
//!
//! The production-sequence tests call `apply_effort_to_spawn_command`
//! (`runtime.rs`), the same function `spawn_agent_child` calls. Deleting
//! `apply_spawn_effort_env` from that wrapper turns these tests RED.
//! Deleting the `apply_effort_to_spawn_command` call from `spawn_agent_child`
//! is a compile error: `spawn_with_effort_proof` consumes the returned
//! `EffortApplied` by value, so removing the binding leaves `effort` undefined
//! at the spawn site.

use std::collections::BTreeMap;

use super::super::strip_effort_keys_from_command;
use super::*;
use crate::managed_agents::runtime::apply_effort_to_spawn_command;

// --------------------------------------------------------------------------
// Command-boundary strip (P1: inherited + baked collision)
// --------------------------------------------------------------------------

/// ACP sentinel baked/inherited collision: registered for removal after strip.
#[test]
fn strip_removes_baked_acp_sentinel_collision() {
    let mut cmd = std::process::Command::new("echo");
    cmd.env(ACP_KEY, "high");
    strip_effort_keys_from_command(&mut cmd);
    let removed = cmd
        .get_envs()
        .any(|(key, value)| key == ACP_KEY && value.is_none());
    assert!(
        removed,
        "ACP sentinel must be registered for removal after strip"
    );
}

/// Baked `GOOSE_THINKING_EFFORT` collision: stripped before descriptor overlay.
#[test]
fn strip_removes_baked_goose_native_key_collision() {
    let mut cmd = std::process::Command::new("echo");
    cmd.env(GOOSE_KEY, "high");
    strip_effort_keys_from_command(&mut cmd);
    let removed = cmd
        .get_envs()
        .any(|(key, value)| key == GOOSE_KEY && value.is_none());
    assert!(
        removed,
        "GOOSE_THINKING_EFFORT must be registered for removal after strip"
    );
}

/// Baked `BUZZ_AGENT_THINKING_EFFORT` collision: legacy alias stripped.
#[test]
fn strip_removes_baked_buzz_agent_native_key_collision() {
    let mut cmd = std::process::Command::new("echo");
    cmd.env(BUZZ_AGENT_KEY, "medium");
    strip_effort_keys_from_command(&mut cmd);
    let removed = cmd
        .get_envs()
        .any(|(key, value)| key == BUZZ_AGENT_KEY && value.is_none());
    assert!(
        removed,
        "BUZZ_AGENT_THINKING_EFFORT must be registered for removal after strip"
    );
}

/// Lowercase inherited key: both canonical and lowercase variants are stripped.
#[test]
fn strip_removes_lowercase_goose_key_inherited_from_shell() {
    let lower = GOOSE_KEY.to_ascii_lowercase();
    let mut cmd = std::process::Command::new("echo");
    cmd.env(&lower, "stale");
    strip_effort_keys_from_command(&mut cmd);
    let removed = cmd
        .get_envs()
        .any(|(key, value)| key == lower.as_str() && value.is_none());
    assert!(
        removed,
        "lowercase GOOSE key must be registered for removal"
    );
}

/// Custom passthrough: non-suppress-set keys are not removed.
#[test]
fn strip_does_not_remove_unrelated_env_key() {
    let mut cmd = std::process::Command::new("echo");
    cmd.env("MY_CUSTOM_EFFORT", "high");
    strip_effort_keys_from_command(&mut cmd);
    let value_present = cmd
        .get_envs()
        .any(|(key, value)| key == "MY_CUSTOM_EFFORT" && value.is_some());
    assert!(
        value_present,
        "strip must not touch env keys outside the suppress set"
    );
}

// --------------------------------------------------------------------------
// Production-sequence seam tests
// --------------------------------------------------------------------------
// Spawn the child directly so its actual env is the ground truth.
// These call `apply_effort_to_spawn_command` (in `runtime.rs`), the same function
// `spawn_agent_child` calls. Deleting `apply_spawn_effort_env` from that wrapper
// turns these tests RED. The `EffortApplied` sentinel makes the call site in
// `spawn_agent_child` a compile-time requirement.
// Deletion proofs:
//   - remove `build_buzz_agent_provider_defaults` inside → baked keys leak;
//   - remove `effort_launch_projection` → suppress list is empty, keys leak;
//   - remove `apply_effort_launch_to_command` → stale keys remain, assertion fails.
//
// Inherited-state tests seed the parent env via `std::env::set_var` under the
// crate-wide env lock (`crate::managed_agents::lock_env_mutex`). `EnvVarGuard`
// restores the exact prior value (including non-Unicode) in `Drop`, so panics
// do not leak the seeded value into unrelated child-spawn tests.

/// RAII guard: snapshots a process-env variable and restores the exact prior
/// value (or removes it if it was absent) on `Drop`, even on panic.
/// Uses `OsString` so a pre-existing non-Unicode value is restored exactly
/// rather than being silently lost.
struct EnvVarGuard {
    key: String,
    prior: Option<std::ffi::OsString>,
}
impl EnvVarGuard {
    fn set(key: &str, value: &str) -> Self {
        let prior = std::env::var_os(key);
        #[allow(deprecated)]
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            key: key.to_string(),
            prior,
        }
    }
}
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        #[allow(deprecated)]
        unsafe {
            match &self.prior {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }
}

fn run_env_cmd(cmd: &mut std::process::Command) -> String {
    let output = cmd
        .output()
        .expect("env-dump command must be executable on this host");
    assert!(
        output.status.success(),
        "env command failed: {:?}",
        output.status
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// After projection + strip + emit, the child sees exactly the projected Goose
/// key with no collision. Inherited lowercase key is seeded via EnvVarGuard.
#[test]
#[cfg(not(target_os = "windows"))]
fn production_sequence_goose_inherited_collision_resolved_in_child() {
    let lower = GOOSE_KEY.to_ascii_lowercase();
    let _lock = crate::managed_agents::lock_env_mutex();
    let _guard = EnvVarGuard::set(&lower, "inherited-low");

    let mut cmd = std::process::Command::new("/usr/bin/env");
    cmd.env(GOOSE_KEY, "baked-high");
    cmd.env(BUZZ_AGENT_KEY, "legacy-medium");
    cmd.env("MY_AGENT_CONFIG", "keep-me");

    let mut r = record();
    r.effort_level = Some("high".into());
    let _effort = apply_effort_to_spawn_command(
        &mut cmd,
        &r,
        Some(goose()),
        &[],
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let child_env = run_env_cmd(&mut cmd);

    assert!(
        child_env.contains(&format!("{GOOSE_KEY}=high")),
        "child must receive the projected Goose key; env:\n{child_env}"
    );
    assert!(
        !child_env.contains(BUZZ_AGENT_KEY),
        "legacy buzz-agent key must not reach child; env:\n{child_env}"
    );
    assert!(
        !child_env.contains(ACP_KEY),
        "ACP sentinel must not reach child for Goose; env:\n{child_env}"
    );
    assert!(
        !child_env.contains(&format!("{lower}=inherited-low")),
        "inherited lowercase key must be stripped; env:\n{child_env}"
    );
    assert!(
        child_env.contains("MY_AGENT_CONFIG=keep-me"),
        "unrelated key must survive; env:\n{child_env}"
    );
    let effort_key_count = [GOOSE_KEY, BUZZ_AGENT_KEY, ACP_KEY]
        .iter()
        .filter(|k| child_env.contains(&format!("{k}=")))
        .count();
    assert_eq!(
        effort_key_count, 1,
        "exactly one effort key must reach child; env:\n{child_env}"
    );
}

/// Windows: OS case-folds env keys, so stripping canonical removes ALL case variants.
#[test]
#[cfg(target_os = "windows")]
fn production_sequence_arbitrary_mixedcase_collision_absent_from_child_windows() {
    let mixed = "GoOsE_ThInKiNg_EfFoRt";
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/c", "set"]);
    cmd.env_clear();
    cmd.env(mixed, "stale-mixed");
    let mut r = record();
    r.effort_level = Some("high".into());
    let _effort = apply_effort_to_spawn_command(
        &mut cmd,
        &r,
        Some(goose()),
        &[],
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let child_env = run_env_cmd(&mut cmd);
    assert!(
        child_env
            .to_ascii_uppercase()
            .contains(&format!("{}=HIGH", GOOSE_KEY.to_ascii_uppercase())),
        "canonical effort key must reach the child; env:\n{child_env}"
    );
    assert!(
        !child_env
            .to_ascii_uppercase()
            .contains(&format!("{}=STALE-MIXED", mixed.to_ascii_uppercase())),
        "mixed-case effort key must not reach the child; env:\n{child_env}"
    );
}

/// Custom passthrough: non-suppress-set effort keys survive the production sequence.
#[test]
#[cfg(not(target_os = "windows"))]
fn production_sequence_custom_passthrough_survives() {
    let mut cmd = std::process::Command::new("/usr/bin/env");
    cmd.env_clear();
    cmd.env("MY_HARNESS_EFFORT", "high");
    cmd.env("MY_UNRELATED_CONFIG", "keep");
    let _effort = apply_effort_to_spawn_command(
        &mut cmd,
        &record(),
        None,
        &[],
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let child_env = run_env_cmd(&mut cmd);
    assert!(
        child_env.contains("MY_HARNESS_EFFORT=high"),
        "custom key must survive; env:\n{child_env}"
    );
    assert!(
        child_env.contains("MY_UNRELATED_CONFIG=keep"),
        "unrelated key must survive; env:\n{child_env}"
    );
}

/// Custom-runtime: inherited `GOOSE_THINKING_EFFORT` survives (unknown-runtime
/// suppress set excludes foreign effort keys).
#[test]
#[cfg(not(target_os = "windows"))]
fn production_sequence_custom_inherited_goose_key_survives() {
    let _lock = crate::managed_agents::lock_env_mutex();
    let _guard = EnvVarGuard::set(GOOSE_KEY, "inherited-high");
    let mut cmd = std::process::Command::new("/usr/bin/env");
    let _effort = apply_effort_to_spawn_command(
        &mut cmd,
        &record(),
        None,
        &[],
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let child_env = run_env_cmd(&mut cmd);
    assert!(
        child_env.contains(&format!("{GOOSE_KEY}=inherited-high")),
        "GOOSE key must survive for unknown runtime; env:\n{child_env}"
    );
}

/// Custom-runtime: inherited ACP sentinel survives as pass-through (no column).
#[test]
#[cfg(not(target_os = "windows"))]
fn production_sequence_custom_inherited_acp_sentinel_survives() {
    let _lock = crate::managed_agents::lock_env_mutex();
    let _guard = EnvVarGuard::set(ACP_KEY, "inherited-val");
    let mut cmd = std::process::Command::new("/usr/bin/env");
    let _effort = apply_effort_to_spawn_command(
        &mut cmd,
        &record(),
        None,
        &[],
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let child_env = run_env_cmd(&mut cmd);
    assert!(
        child_env.contains(&format!("{ACP_KEY}=inherited-val")),
        "ACP sentinel must survive for unknown runtime with no column; env:\n{child_env}"
    );
}

/// Windows: custom-wrapper effort keys survive the production sequence.
#[test]
#[cfg(target_os = "windows")]
fn production_sequence_custom_passthrough_survives() {
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/c", "set"]);
    cmd.env_clear();
    cmd.env("MY_HARNESS_EFFORT", "high");
    cmd.env("MY_UNRELATED_CONFIG", "keep");
    let _effort = apply_effort_to_spawn_command(
        &mut cmd,
        &record(),
        None,
        &[],
        None,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    let child_env = run_env_cmd(&mut cmd);
    assert!(child_env
        .to_ascii_uppercase()
        .contains("MY_HARNESS_EFFORT=HIGH"));
    assert!(child_env
        .to_ascii_uppercase()
        .contains("MY_UNRELATED_CONFIG=KEEP"));
}
