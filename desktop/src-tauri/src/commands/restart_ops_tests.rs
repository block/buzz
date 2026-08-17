//! Stop-spy tests for the four restart-authorization operations.
//!
//! Each op is driven with an injected global loader and (for the under-lock
//! ops) an injected stop effect that a spy observes. The Ready-but-secrets-
//! unavailable record is the case readiness alone misses: its empty hydrated
//! env computes `Ready`, so only the secret gate stops the bounce. Together
//! with a failing global loader these drive every gate, so removing any one of
//! them turns a test red — the acceptance the direct-helper tests could not
//! meet.

use super::*;
use crate::managed_agents::{GlobalAgentConfig, ManagedAgentRecord};
use std::cell::Cell;

const BUZZ_AGENT: &str = "buzz-agent";

/// A local record whose effective env satisfies the `buzz-agent` readiness
/// gate (provider + model + provider key present inline), so `agent_readiness`
/// returns `Ready` and the command resolves to `buzz-agent`. This is the
/// baseline that makes the secret-gate assertions meaningful — a record that
/// failed readiness for unrelated reasons could not distinguish the gates.
fn ready_record() -> ManagedAgentRecord {
    let mut rec: ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{}",
            "name": "restart-ops-test",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "buzz-agent",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }}"#,
        "aa".repeat(32)
    ))
    .unwrap();
    rec.env_vars
        .insert("BUZZ_AGENT_PROVIDER".into(), "anthropic".into());
    rec.env_vars
        .insert("BUZZ_AGENT_MODEL".into(), "claude-opus-4-5".into());
    rec.env_vars
        .insert("ANTHROPIC_API_KEY".into(), "sk-test".into());
    rec
}

fn ok_global() -> impl FnOnce() -> Result<GlobalAgentConfig, String> {
    || Ok(GlobalAgentConfig::default())
}

fn err_global() -> impl FnOnce() -> Result<GlobalAgentConfig, String> {
    || Err("global env_vars unavailable: gen abc123 not found in keyring".to_string())
}

// ── select_post_install_restart_candidates ────────────────────────────────

/// Positive control: a Ready record with matching runtime, live PID, and setup
/// mode is a candidate. This proves the negative cases below isolate a single
/// gate rather than failing readiness for an unrelated reason.
#[test]
fn test_select_post_install_ready_record_is_candidate() {
    let records = [ready_record()];
    let out =
        select_post_install_restart_candidates(&records, &[], BUZZ_AGENT, ok_global(), |_| {
            (true, true)
        });
    assert_eq!(
        out,
        vec![records[0].pubkey.clone()],
        "a Ready setup-mode record on the matching runtime must be a candidate"
    );
}

/// M1: dropping the `effective_secrets_unavailable` consultation in the op lets
/// a secrets-unavailable record (whose empty env still computes Ready) become a
/// candidate. With the gate present it is excluded.
#[test]
fn test_select_post_install_secrets_unavailable_record_is_not_candidate() {
    let mut record = ready_record();
    record.secrets_unavailable = true;
    let records = [record];
    let out =
        select_post_install_restart_candidates(&records, &[], BUZZ_AGENT, ok_global(), |_| {
            (true, true)
        });
    assert!(
        out.is_empty(),
        "a record with unavailable secrets must NOT be a post-install candidate"
    );
}

/// M5: restoring `unwrap_or_default()` for the global load lets the scan run
/// against a default global on an unreadable committed ref. With the strict
/// gate present, an `Err` from the loader yields zero candidates.
#[test]
fn test_select_post_install_unavailable_global_yields_zero_candidates() {
    let records = [ready_record()];
    let out =
        select_post_install_restart_candidates(&records, &[], BUZZ_AGENT, err_global(), |_| {
            (true, true)
        });
    assert!(
        out.is_empty(),
        "an unreadable committed global must yield zero post-install candidates"
    );
}

// ── authorize_post_install_restart ────────────────────────────────────────

/// Positive control: a Ready record with an available global and setup mode
/// runs the stop. This makes the refusal cases below meaningful.
#[test]
fn test_authorize_post_install_ready_record_runs_stop() {
    let record = ready_record();
    let stops = Cell::new(0u32);
    let out = authorize_post_install_restart(&record, &[], BUZZ_AGENT, true, ok_global(), || {
        stops.set(stops.get() + 1);
        Ok(())
    });
    assert!(out.is_ok(), "an eligible record must authorize the bounce");
    assert_eq!(
        stops.get(),
        1,
        "the stop must fire exactly once when eligible"
    );
}

/// M2: deleting the under-lock `refuse_restart_on_unavailable_secrets` call
/// lets the stop fire for a secrets-unavailable record (its empty env still
/// computes Ready). With the gate present, the op refuses BEFORE the stop.
#[test]
fn test_authorize_post_install_secrets_unavailable_refuses_before_stop() {
    let mut record = ready_record();
    record.secrets_unavailable = true;
    let stops = Cell::new(0u32);
    let out = authorize_post_install_restart(&record, &[], BUZZ_AGENT, true, ok_global(), || {
        stops.set(stops.get() + 1);
        Ok(())
    });
    assert!(out.is_err(), "a secrets-unavailable record must be refused");
    assert_eq!(
        stops.get(),
        0,
        "no stop may fire when secrets are unavailable — the FailedAfterStop path"
    );
}

/// M6: restoring `unwrap_or_default()` for the under-lock global load lets the
/// stop fire on an unreadable committed ref. With the strict gate present, the
/// op refuses before the stop.
#[test]
fn test_authorize_post_install_unavailable_global_refuses_before_stop() {
    let record = ready_record();
    let stops = Cell::new(0u32);
    let out = authorize_post_install_restart(&record, &[], BUZZ_AGENT, true, err_global(), || {
        stops.set(stops.get() + 1);
        Ok(())
    });
    assert!(
        out.is_err(),
        "an unreadable committed global must be refused"
    );
    assert_eq!(
        stops.get(),
        0,
        "no stop may fire when the committed global is unreadable"
    );
}

// ── select_config_change_restart_candidates ───────────────────────────────

/// A record whose readiness unblocks NotReady→Ready across the global change:
/// old global lacks the provider key, new global supplies it. The record keeps
/// provider+model inline so only the key gates readiness.
fn key_from_global_record() -> ManagedAgentRecord {
    let mut rec = ready_record();
    rec.env_vars.remove("ANTHROPIC_API_KEY");
    rec
}

fn global_with_key() -> GlobalAgentConfig {
    let mut g = GlobalAgentConfig::default();
    g.env_vars
        .insert("ANTHROPIC_API_KEY".into(), "sk-test".into());
    g
}

/// Positive control: a NotReady→Ready record with a live runtime is a
/// config-change candidate.
#[test]
fn test_select_config_change_unblocked_record_is_candidate() {
    let records = [key_from_global_record()];
    let old_global = GlobalAgentConfig::default();
    let new_global = global_with_key();
    let out =
        select_config_change_restart_candidates(&records, &[], &old_global, &new_global, |_| true);
    assert_eq!(
        out,
        vec![records[0].pubkey.clone()],
        "a NotReady→Ready record with a live runtime must be a candidate"
    );
}

/// M3: dropping the `effective_secrets_unavailable` consultation lets a
/// secrets-unavailable record become a config-change candidate. With the gate
/// present it is excluded even though it unblocked NotReady→Ready.
#[test]
fn test_select_config_change_secrets_unavailable_record_is_not_candidate() {
    let mut record = key_from_global_record();
    record.secrets_unavailable = true;
    let records = [record];
    let old_global = GlobalAgentConfig::default();
    let new_global = global_with_key();
    let out =
        select_config_change_restart_candidates(&records, &[], &old_global, &new_global, |_| true);
    assert!(
        out.is_empty(),
        "a record with unavailable secrets must NOT be a config-change candidate"
    );
}

// ── authorize_config_change_restart ───────────────────────────────────────

/// Positive control: a NotReady→Ready record with an available global runs the
/// stop.
#[test]
fn test_authorize_config_change_unblocked_record_runs_stop() {
    let record = key_from_global_record();
    let old_global = GlobalAgentConfig::default();
    let new_global = global_with_key();
    let stops = Cell::new(0u32);
    let out = authorize_config_change_restart(
        &record,
        &[],
        &old_global,
        &new_global,
        ok_global(),
        || {
            stops.set(stops.get() + 1);
            Ok(())
        },
    );
    assert!(out.is_ok(), "an unblocked record must authorize the bounce");
    assert_eq!(
        stops.get(),
        1,
        "the stop must fire exactly once when eligible"
    );
}

/// M4: dropping the `effective_secrets_unavailable` consultation in the restart
/// predicate lets the stop fire for a secrets-unavailable record. With the gate
/// present, the op refuses before the stop.
#[test]
fn test_authorize_config_change_secrets_unavailable_refuses_before_stop() {
    let mut record = key_from_global_record();
    record.secrets_unavailable = true;
    let old_global = GlobalAgentConfig::default();
    let new_global = global_with_key();
    let stops = Cell::new(0u32);
    let out = authorize_config_change_restart(
        &record,
        &[],
        &old_global,
        &new_global,
        ok_global(),
        || {
            stops.set(stops.get() + 1);
            Ok(())
        },
    );
    assert!(out.is_err(), "a secrets-unavailable record must be refused");
    assert_eq!(
        stops.get(),
        0,
        "no stop may fire when secrets are unavailable"
    );
}

/// M7: deleting the under-lock committed-global re-read lets the stop fire on
/// an unreadable committed ref (the phase-1 snapshots said Ready). With the
/// re-read present, the op refuses before the stop.
#[test]
fn test_authorize_config_change_unavailable_global_refuses_before_stop() {
    let record = key_from_global_record();
    let old_global = GlobalAgentConfig::default();
    let new_global = global_with_key();
    let stops = Cell::new(0u32);
    let out = authorize_config_change_restart(
        &record,
        &[],
        &old_global,
        &new_global,
        err_global(),
        || {
            stops.set(stops.get() + 1);
            Ok(())
        },
    );
    assert!(
        out.is_err(),
        "an unreadable committed global must be refused before the stop"
    );
    assert_eq!(
        stops.get(),
        0,
        "no stop may fire when the committed global re-read fails"
    );
}
