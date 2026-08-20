//! The single harness-agnostic effort authority (plan-of-record, PR #4625).
//!
//! ## One projection, one destination key, one snapshot leaf
//!
//! [`effort_launch_projection`] resolves the effective startup effort a spawn
//! would apply, over the canonical persisted column (`record.effort_level`) AND
//! the sanitized per-tier env inputs, in the CLEAR authority order:
//!
//! ```text
//! record native(valid) > canonical column(valid) > record legacy(valid)
//!   > persona(native, then legacy) > global(native) > definition(native)
//!   > baked(native)
//! ```
//!
//! (The reader adds the live-ACP tier between column and persona and the config
//! file tier at the bottom; the launch projection has neither — a spawn reads
//! neither a running session nor the on-disk harness file.)
//!
//! The **tier-reading** native key is the runtime's real `thinking_env_var`
//! (`None` for Claude/Codex — those have no native key, so the column is the
//! sole authority and a user-supplied `BUZZ_ACP_EFFORT_LEVEL` is transport, not
//! a tier). The **emission** key ([`EffortLaunch::key`]) is
//! `thinking_env_var.unwrap_or(BUZZ_ACP_EFFORT_LEVEL)`: Goose emits
//! `GOOSE_THINKING_EFFORT`, buzz-agent emits `BUZZ_AGENT_THINKING_EFFORT`,
//! Claude/Codex/keyless-ACP and any unknown/custom runtime emit the retained
//! ACP-startup sentinel `BUZZ_ACP_EFFORT_LEVEL`.
//!
//! [`EffortLaunch::suppress`] lists every known native/legacy effort key plus
//! the sentinel; every consumer strips them all first, then emits at most the
//! one `key`. This is what guarantees a launched process, a remote payload, and
//! a restart snapshot can never carry two effort authorities.

use std::collections::BTreeMap;

use super::LEGACY_THINKING_EFFORT_KEY;
use crate::managed_agents::custom_harnesses::HarnessDefinition;
use crate::managed_agents::discovery::KnownAcpRuntime;
use crate::managed_agents::types::{AgentDefinition, ManagedAgentRecord};

/// The retained ACP-startup transport key. Claude, Codex, keyless ACP adapters,
/// and any unknown/custom runtime route the effective effort through this key
/// (the harness reads it into `PoolStartup.startup_effort`). It is *transport*,
/// never a value-authority tier: a user-supplied entry is suppressed and
/// overwritten by the projected effective value.
pub(crate) const ACP_STARTUP_EFFORT_KEY: &str = "BUZZ_ACP_EFFORT_LEVEL";

/// The resolved launch effort for one runtime: the single fact every spawn
/// path (local, remote, snapshot) consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EffortLaunch {
    /// The final effective effort value, normalized for contract runtimes and
    /// raw for contract-less ones, resolved over ALL tiers (column + env).
    /// `None` when no tier supplies a value the destination can express.
    pub value: Option<String>,
    /// The destination env key the value is emitted under.
    pub key: &'static str,
    /// Every effort key to strip from the launch env before emitting `key`.
    /// Always includes the sentinel and all known native/legacy effort keys, so
    /// no foreign or transport effort key can shadow the projected authority.
    pub suppress: Vec<&'static str>,
}

impl EffortLaunch {
    /// Apply the projection to a launch env map: strip every `suppress` key,
    /// then emit `key = value` when a value is present. After this call the map
    /// holds at most one effort key (`key`), carrying the effective value.
    pub(crate) fn apply(&self, env: &mut BTreeMap<String, String>) {
        for k in &self.suppress {
            env.remove(*k);
        }
        if let Some(ref v) = self.value {
            env.insert(self.key.to_string(), v.clone());
        }
    }
}

/// Resolve the single harness-agnostic effort authority and apply it to a fully
/// layered launch `env`: strip every known/legacy/transport effort key, then
/// emit exactly the one destination key holding the effective value. Called by
/// the descriptor resolver AFTER the full layer stack, so the launch env, the
/// remote deploy payload, and the restart snapshot all carry one effort key and
/// one value — no double authority, no foreign key, no launch/badge disagreement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_launch_effort(
    env: &mut BTreeMap<String, String>,
    record: &ManagedAgentRecord,
    runtime: Option<&KnownAcpRuntime>,
    personas: &[AgentDefinition],
    global_env: &BTreeMap<String, String>,
    harness_def: Option<&HarnessDefinition>,
    baked_env: &BTreeMap<String, String>,
) {
    effort_launch_projection(
        record,
        runtime,
        personas,
        record.persona_id.as_deref(),
        global_env,
        harness_def,
        baked_env,
    )
    .apply(env);
}

/// Resolve one effort tier's value, applying within-tier legacy aliasing and
/// normalization. Returns the canonical (or raw, contract-less) value, or
/// `None` when no usable candidate exists.
///
/// Lookup (per tier, independent of other tiers):
///   1. Native key — normalized; invalid → skip as absent.
///   2. Legacy key (`BUZZ_AGENT_THINKING_EFFORT`) — only when the native key
///      differs from it AND `allow_legacy_alias` is set AND the value
///      normalizes. Invalid legacy is skipped so the next tier can supply one.
pub(crate) fn effort_tier_alias(
    map: &BTreeMap<String, String>,
    native_key: &str,
    norm: impl Fn(&str) -> Option<String>,
    allow_legacy_alias: bool,
) -> Option<String> {
    if let Some(raw) = map.get(native_key) {
        if let Some(canonical) = norm(raw) {
            return Some(canonical);
        }
    }
    if allow_legacy_alias && native_key != LEGACY_THINKING_EFFORT_KEY {
        if let Some(raw) = map.get(LEGACY_THINKING_EFFORT_KEY) {
            if let Some(canonical) = norm(raw) {
                return Some(canonical);
            }
        }
    }
    None
}

/// The destination env key the effective effort is emitted under for `runtime`:
/// the runtime's native `thinking_env_var`, else the ACP-startup sentinel
/// (Claude, Codex, keyless ACP adapters, and unknown/custom runtimes).
pub(crate) fn effort_dest_key(runtime: Option<&KnownAcpRuntime>) -> &'static str {
    runtime
        .and_then(|r| r.thinking_env_var)
        .unwrap_or(ACP_STARTUP_EFFORT_KEY)
}

/// Every effort key to strip before emitting the single destination key: all
/// known native effort keys, the legacy alias, and the ACP-startup sentinel.
/// Stripping the full set guarantees no foreign or transport effort key can
/// shadow the projected authority.
pub(crate) fn effort_suppress_keys() -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = super::all_known_effort_keys().collect();
    if !keys.contains(&ACP_STARTUP_EFFORT_KEY) {
        keys.push(ACP_STARTUP_EFFORT_KEY);
    }
    if !keys.contains(&LEGACY_THINKING_EFFORT_KEY) {
        keys.push(LEGACY_THINKING_EFFORT_KEY);
    }
    keys
}

/// Build the single effective-effort projection for a launch.
///
/// `global_env`, `persona_id`+`personas`, `harness_def`, and `baked_env` supply
/// the same per-tier inputs the layered spawn env is built from; the projection
/// re-reads them so an invalid high-tier value skips as absent and a lower tier
/// can win (which a merged last-wins env map cannot express).
pub(crate) fn effort_launch_projection(
    record: &ManagedAgentRecord,
    runtime: Option<&KnownAcpRuntime>,
    personas: &[AgentDefinition],
    persona_id: Option<&str>,
    global_env: &BTreeMap<String, String>,
    harness_def: Option<&HarnessDefinition>,
    baked_env: &BTreeMap<String, String>,
) -> EffortLaunch {
    let key = effort_dest_key(runtime);
    let suppress = effort_suppress_keys();

    // Normalizer: contract runtimes canonicalize (invalid → skip); contract-less
    // runtimes pass raw (any present value is valid for their per-model catalog).
    let contract = runtime.and_then(|r| r.effort_normalization);
    let norm = |raw: &str| -> Option<String> {
        match contract {
            Some(c) => c.normalize_str(raw),
            None => Some(raw.to_string()),
        }
    };

    // Tier-reading native key: the runtime's REAL native key. `None` (Claude,
    // Codex, unknown/custom) means there are no env-tier authorities — the
    // sentinel in user env is transport only — so the column is the sole source.
    let native_key = runtime.and_then(|r| r.thinking_env_var);

    let value = resolve_effective_effort(
        record,
        native_key,
        &norm,
        personas,
        persona_id,
        global_env,
        harness_def,
        baked_env,
    );

    EffortLaunch {
        value,
        key,
        suppress,
    }
}

/// Resolve the effective effort value in CLEAR authority order (launch tiers).
#[allow(clippy::too_many_arguments)]
fn resolve_effective_effort(
    record: &ManagedAgentRecord,
    native_key: Option<&str>,
    norm: &impl Fn(&str) -> Option<String>,
    personas: &[AgentDefinition],
    persona_id: Option<&str>,
    global_env: &BTreeMap<String, String>,
    harness_def: Option<&HarnessDefinition>,
    baked_env: &BTreeMap<String, String>,
) -> Option<String> {
    use crate::managed_agents::env_vars::{is_reserved_env_key, live_persona_env, merged_user_env};

    // Sanitize env tiers exactly as the layered spawn env does (reserved/
    // malformed/NUL filtering), so the resolved authority matches what launches.
    let record_env = merged_user_env(&BTreeMap::new(), &record.env_vars);

    // 1. record native — only for runtimes with a real native key.
    if let Some(nk) = native_key {
        if let Some(raw) = record_env.get(nk) {
            if let Some(v) = norm(raw) {
                return Some(v);
            }
        }
    }
    // 2. canonical column — normalized (raw passthrough for contract-less).
    if let Some(raw) = record.effort_level.as_deref() {
        if let Some(v) = norm(raw) {
            return Some(v);
        }
    }
    // 3. record legacy alias — only when the native key differs from it.
    if let Some(nk) = native_key {
        if nk != LEGACY_THINKING_EFFORT_KEY {
            if let Some(raw) = record_env.get(LEGACY_THINKING_EFFORT_KEY) {
                if let Some(v) = norm(raw) {
                    return Some(v);
                }
            }
        }
    }
    // Env tiers below require a native key to read.
    let nk = native_key?;

    // 4. persona (native, then legacy) — sanitized like the layered spawn env.
    let persona_env = merged_user_env(&BTreeMap::new(), &live_persona_env(personas, persona_id));
    if let Some(v) = effort_tier_alias(&persona_env, nk, norm, true) {
        return Some(v);
    }
    // 5. global (native only).
    let global = merged_user_env(&BTreeMap::new(), global_env);
    if let Some(v) = effort_tier_alias(&global, nk, norm, false) {
        return Some(v);
    }
    // 6. definition (native only) — author-controlled; reserved keys stripped.
    if let Some(def) = harness_def {
        let def_env: BTreeMap<String, String> = def
            .env
            .iter()
            .filter(|(k, _)| !is_reserved_env_key(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Some(v) = effort_tier_alias(&def_env, nk, norm, false) {
            return Some(v);
        }
    }
    // 7. baked build floor (native only).
    if let Some(raw) = baked_env.get(nk) {
        if let Some(v) = norm(raw) {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
#[path = "effort_tests.rs"]
mod tests;
