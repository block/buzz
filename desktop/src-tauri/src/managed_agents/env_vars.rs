//! Per-persona and per-agent env var overrides.
//!
//! Personas and managed agents can each carry a `BTreeMap<String, String>`
//! of env vars that get layered into the spawned agent's environment.
//! Precedence: desktop parent env < persona env < agent env (last wins on
//! key collision). See `runtime::spawn_agent_child`.
//!
//! A small set of *reserved* keys includes Buzz's identity, secrets, security
//! gates, and control-plane values. Save-time validation rejects those keys.
//! Runtime filtering strips old persisted overrides. Behavior knobs
//! (GOOSE_MODE, BUZZ_ACP_MODEL, BUZZ_ACP_SYSTEM_PROMPT, …) remain freely
//! overridable. Power users can still bypass their dedicated UI fields.
//! `BUZZ_ACP_AGENTS` is reserved because Desktop applies harness-specific caps
//! before it writes the provider launch policy.

use std::collections::BTreeMap;

/// Env var keys that are *derived* from the structured `AgentDefinition.provider`
/// and `AgentDefinition.model` fields at spawn/deploy time. These must NOT be
/// persisted in `AgentDefinition.env_vars` because they would shadow the
/// structured fields after the user edits provider/model in the UI.
///
/// At local spawn time, `runtime_metadata_env_vars` re-derives these from the
/// current structured fields, so they are always up-to-date. At remote deploy
/// time, `build_deploy_payload` projects the structured fields directly.
///
/// Non-structured knobs (`GOOSE_TEMPERATURE`, `GOOSE_CONTEXT_LIMIT`) are NOT
/// in this list — they have no structured counterpart and must be preserved.
pub(crate) const DERIVED_PROVIDER_MODEL_ENV_KEYS: &[&str] = &[
    "GOOSE_MODEL",
    "GOOSE_PROVIDER",
    "BUZZ_AGENT_MODEL",
    "BUZZ_AGENT_PROVIDER",
];

/// Returns `true` if `key` is a derived provider/model env key that should be
/// filtered out of persisted `AgentDefinition.env_vars` at pack import time.
pub(crate) fn is_derived_provider_model_key(key: &str) -> bool {
    DERIVED_PROVIDER_MODEL_ENV_KEYS
        .iter()
        .any(|k| k.eq_ignore_ascii_case(key))
}

// Canonical reserved-key list + predicate, shared verbatim with `build.rs`.
// See `reserved_env_keys.rs` for why this is `include!`d rather than a module.
include!("reserved_env_keys.rs");

/// Returns true if `key` is a well-formed POSIX-shaped env var name:
/// `[A-Za-z_][A-Za-z0-9_]*`. This is a hard requirement, not a stylistic
/// nit: Rust's `Command::env` will happily accept a key containing `=`
/// or whitespace and pass it straight into the child's environ block,
/// where `getenv("FOO")` then matches whatever comes after the first
/// `=`. That means a key like `BUZZ_AUTH_TAG=x` with value `forged`
/// lands as `BUZZ_AUTH_TAG=x=forged` in the child env and
/// `getenv("BUZZ_AUTH_TAG")` returns `"x=forged"` — a full reserved-
/// key bypass. Rejecting non-POSIX keys closes this hole at the
/// boundary where the input enters the system.
pub(crate) fn is_well_formed_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Render a malformed env-var key for inclusion in user-visible errors
/// and logs without leaking values.
///
/// A common slip is pasting a full `.env` line into the key field —
/// `ANTHROPIC_API_KEY=sk-...`. The reserved-key validator rejects it
/// (good), but echoing the whole key back in an error or log would surface
/// the secret. This helper:
///
/// - Truncates at the first `=` (the value lives after).
/// - Replaces ASCII control bytes with `?` so a NUL or newline can't
///   corrupt the log line.
/// - Caps the displayed length at 64 characters.
///
/// The result is purely cosmetic — the on-disk record still carries the
/// original key, and the runtime filter still drops it.
pub(crate) fn display_invalid_key(key: &str) -> String {
    const MAX: usize = 64;
    let truncated_at_eq: String = key.chars().take_while(|c| *c != '=').collect();
    let sanitized: String = truncated_at_eq
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect();
    if sanitized.chars().count() > MAX {
        let head: String = sanitized.chars().take(MAX).collect();
        format!("{head}…")
    } else if sanitized.len() < truncated_at_eq.len() || truncated_at_eq.len() < key.len() {
        // We dropped *something* — make it obvious to the reader.
        format!("{sanitized}…")
    } else {
        sanitized
    }
}

/// Validate user-supplied env var keys at save time. Returns an error
/// listing problems so the GUI can surface them in one go.
///
/// Rules:
/// - Keys must match `[A-Za-z_][A-Za-z0-9_]*` (POSIX-ish). This rejects
///   empty keys, keys containing `=` (reserved-key bypass — see
///   [`is_well_formed_env_key`]), whitespace, NULs, and leading digits.
/// - Keys (case-insensitive) must not appear in [`RESERVED_ENV_KEYS`].
/// - Values must not contain interior NULs (Rust's `Command::env` panics
///   on NUL bytes) and must be under [`MAX_ENV_VALUE_BYTES`]. The total
///   payload (sum of key + value bytes) is capped at
///   [`MAX_ENV_TOTAL_BYTES`] so a malformed IPC caller can't bloat the
///   persona/agent record file.
pub fn validate_user_env_keys(env_vars: &BTreeMap<String, String>) -> Result<(), String> {
    let mut malformed: Vec<&str> = env_vars
        .keys()
        .filter(|k| !is_well_formed_env_key(k))
        .map(String::as_str)
        .collect();
    malformed.sort_unstable();
    malformed.dedup();
    if !malformed.is_empty() {
        let pretty: Vec<String> = malformed
            .iter()
            .map(|k| {
                if k.is_empty() {
                    "(empty)".to_string()
                } else {
                    format!("\"{}\"", display_invalid_key(k))
                }
            })
            .collect();
        return Err(format!(
            "env var keys must match [A-Za-z_][A-Za-z0-9_]*; invalid: {}",
            pretty.join(", ")
        ));
    }
    let mut reserved: Vec<&str> = env_vars
        .keys()
        .filter(|k| is_reserved_env_key(k))
        .map(String::as_str)
        .collect();
    reserved.sort_unstable();
    reserved.dedup();
    if !reserved.is_empty() {
        return Err(format!(
            "the following env vars are reserved by Buzz and cannot be overridden: {}",
            reserved.join(", ")
        ));
    }
    // Value validation. Keep these errors *generic* — values frequently
    // contain secrets and we'd rather not surface even a truncated view.
    let mut total: usize = 0;
    for (k, v) in env_vars {
        if v.contains('\0') {
            return Err(format!("env var `{k}`: values cannot contain NUL bytes"));
        }
        if v.len() > MAX_ENV_VALUE_BYTES {
            return Err(format!(
                "env var `{k}`: value is {} bytes; per-value limit is {MAX_ENV_VALUE_BYTES}",
                v.len()
            ));
        }
        total = total.saturating_add(k.len()).saturating_add(v.len());
    }
    if total > MAX_ENV_TOTAL_BYTES {
        return Err(format!(
            "total env var payload is {total} bytes; limit is {MAX_ENV_TOTAL_BYTES}"
        ));
    }
    Ok(())
}

/// Returns `true` when `key` is safe to show verbatim — not a credential.
///
/// Default-deny: every key NOT in this explicit allowlist is masked. Callers
/// that display env values (baked-env UI, spawn-diff tooltip) share this
/// single authority — no second list.
///
/// Allowlist (case-insensitive):
/// - `BUZZ_AGENT_PROVIDER`, `BUZZ_AGENT_MODEL` — agent runtime selection
/// - `BUZZ_AGENT_THINKING_EFFORT` — non-secret enum (none/minimal/low/medium/high/xhigh/max)
/// - `DATABRICKS_HOST`, `DATABRICKS_MODEL` — Block non-secret defaults
pub(crate) fn is_safe_to_reveal(key: &str) -> bool {
    const SAFE_KEYS: &[&str] = &[
        "BUZZ_AGENT_PROVIDER",
        "BUZZ_AGENT_MODEL",
        "BUZZ_AGENT_THINKING_EFFORT",
        "DATABRICKS_HOST",
        "DATABRICKS_MODEL",
    ];
    let upper = key.to_ascii_uppercase();
    SAFE_KEYS.iter().any(|safe| upper == *safe)
}

/// Per-value byte cap for env values. 32 KiB is generous for credentials,
/// JWT-ish tokens, certs etc., but small enough that a malformed IPC
/// caller can't blow up the persona/agent JSON file. Tune up if real
/// users hit it.
pub(crate) const MAX_ENV_VALUE_BYTES: usize = 32 * 1024;

/// Total payload cap across all keys + values in a single persona/agent
/// env_vars map. Sized for ~32 entries at the per-value cap with plenty
/// of slack. Mostly a guard against unbounded growth via a buggy or
/// malicious IPC caller.
pub(crate) const MAX_ENV_TOTAL_BYTES: usize = 256 * 1024;

/// Merge persona env vars and per-agent env vars.
///
/// Precedence: persona env first, then agent overrides, last-wins on key
/// collision. Returns a `BTreeMap` so the caller can iterate in stable
/// order (the order is irrelevant for `Command::env` but useful for test
/// assertions and future logging).
///
/// Reserved keys are silently stripped — see [`RESERVED_ENV_KEYS`].
/// Save-time validation (see [`validate_user_env_keys`]) rejects them up
/// front so this filter is defense-in-depth for older on-disk records.
pub(crate) fn merged_user_env(
    persona_env: &BTreeMap<String, String>,
    agent_env: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = persona_env.clone();
    for (k, v) in agent_env {
        merged.insert(k.clone(), v.clone());
    }
    merged.retain(|k, v| {
        if is_reserved_env_key(k) {
            eprintln!(
                "buzz-desktop: ignoring reserved env var `{k}` from persona/agent overrides"
            );
            return false;
        }
        if !is_well_formed_env_key(k) {
            // Defense in depth: drop malformed keys at spawn time so older
            // on-disk records (saved before the tightened validator) can't
            // smuggle a reserved key past us via `=`-in-key tricks. See
            // `is_well_formed_env_key` for the exploit.
            eprintln!(
                "buzz-desktop: ignoring malformed env var key `{}` from persona/agent overrides",
                display_invalid_key(k)
            );
            return false;
        }
        if v.contains('\0') {
            // `Command::env` panics on interior NULs. Older records may
            // have escaped the value validator; drop them here rather
            // than crash the spawn. We deliberately do NOT log the value.
            eprintln!(
                "buzz-desktop: ignoring env var `{k}` with NUL byte in value"
            );
            return false;
        }
        if v.len() > MAX_ENV_VALUE_BYTES {
            eprintln!(
                "buzz-desktop: ignoring env var `{k}` with oversize value ({} bytes > {MAX_ENV_VALUE_BYTES})",
                v.len()
            );
            return false;
        }
        true
    });
    merged
}

/// Look up the live env map of `persona_id` within an already-loaded persona
/// slice. Returns an empty map for standalone agents (`None`) and for links
/// to personas that no longer exist. The latter is an orphaned instance,
/// which `spawn_agent_child` refuses before this is ever read at spawn time
/// (see `effective_config::resolve_effective_config`'s `OrphanedInstance`
/// arm via `require_resolved`) — an empty map here only matters for
/// non-spawn callers like readiness/hash that must still resolve
/// a slice for a record whose orphan status hasn't been checked yet.
pub(crate) fn live_persona_env(
    personas: &[super::types::AgentDefinition],
    persona_id: Option<&str>,
) -> BTreeMap<String, String> {
    persona_id
        .and_then(|pid| personas.iter().find(|p| p.id == pid))
        .map(|p| p.env_vars.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
