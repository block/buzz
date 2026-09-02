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
use crate::managed_agents::discovery::{EffortNormalization, KnownAcpRuntime};
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
    /// When no tier resolved a `value`, preserve a value the launch env already
    /// carries under `key` (collapsing every case variant to the canonical
    /// spelling). Set only for unknown/custom runtimes, where the ACP sentinel
    /// is user pass-through transport that must survive a spawn — not a foreign
    /// key to drop. Known runtimes leave it `false`: a bare destination-key
    /// value with no resolved authority is invalid/foreign and is dropped.
    pub preserve_passthrough: bool,
}

impl EffortLaunch {
    /// Apply the projection to a launch env map: strip every `suppress` key,
    /// then emit `key = value` when a value is present. After this call the map
    /// holds at most one effort key (`key`), carrying the effective value.
    ///
    /// Suppression is ASCII-case-insensitive: Windows `Command` case-folds env
    /// names, so a hand-set `goose_thinking_effort` would otherwise evade an
    /// exact-case strip and shadow the projected authority.
    ///
    /// When `preserve_passthrough` is set and no tier resolved a value, a value
    /// already present under `key` (in any case) is carried forward and
    /// re-emitted canonically. Multiple case spellings can survive the
    /// case-sensitive layer merge (e.g. a lower-tier `BUZZ_ACP_EFFORT_LEVEL`
    /// plus a higher-tier `buzz_acp_effort_level`); the carry selects the LAST
    /// case-insensitive match in `BTreeMap` iteration order, which is exactly
    /// the value Rust's Windows `Command` writer produces — it sets each spelling
    /// in iteration order into a case-folded env map, so the last set wins. This
    /// keeps an unknown/custom runtime's hand-set sentinel alive, preserves the
    /// value the child would actually receive, and guarantees one canonical
    /// spelling downstream.
    pub(crate) fn apply(&self, env: &mut BTreeMap<String, String>) {
        let carried = (self.value.is_none() && self.preserve_passthrough)
            .then(|| {
                env.iter()
                    .rev()
                    .find(|(k, _)| k.eq_ignore_ascii_case(self.key))
                    .map(|(_, v)| v.clone())
            })
            .flatten();
        env.retain(|k, _| {
            !self
                .suppress
                .iter()
                .any(|suppressed| k.eq_ignore_ascii_case(suppressed))
        });
        if let Some(v) = self.value.as_ref().or(carried.as_ref()) {
            env.insert(self.key.to_string(), v.clone());
        }
    }
}

/// Look up `key` in `map` case-insensitively (ASCII), selecting the LAST
/// case-insensitive match in `BTreeMap` iteration order. Effort key resolution
/// must match Windows `Command` env semantics: `Command` writes each spelling
/// in iteration order into a case-folded env map, so the last-set spelling wins
/// and is the value the child actually receives. Preferring an exact match
/// instead would pick a different case variant than the child gets — e.g.
/// `GOOSE_THINKING_EFFORT=low` plus `goose_thinking_effort=high` would resolve
/// to `low` while the child runs `high`. This mirrors `EffortLaunch::apply`'s
/// `.rev().find` carry so the tier reader, the passthrough carry, and the child
/// all agree on one value.
pub(crate) fn get_ci<'a>(map: &'a BTreeMap<String, String>, key: &str) -> Option<&'a String> {
    map.iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
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
    if let Some(raw) = get_ci(map, native_key) {
        if let Some(canonical) = norm(raw) {
            return Some(canonical);
        }
    }
    if allow_legacy_alias && native_key != LEGACY_THINKING_EFFORT_KEY {
        if let Some(raw) = get_ci(map, LEGACY_THINKING_EFFORT_KEY) {
            if let Some(canonical) = norm(raw) {
                return Some(canonical);
            }
        }
    }
    None
}

/// Normalize/validate an effort candidate for a runtime's destination
/// vocabulary. The single value gate shared by the launch projection and the
/// reader, so the panel and the next spawn never disagree on a value's validity.
///
/// - `contract` present (Goose): canonicalize through the alias table; invalid
///   → `None` (skip as absent).
/// - `contract` absent but `accepted` present (buzz-agent): validation-only —
///   accept a value case-insensitively iff the destination parser would
///   (`parse_thinking_effort`), emit it lowercased; a foreign canonical (e.g.
///   Goose `off`) is rejected so it is never emitted as
///   `BUZZ_AGENT_THINKING_EFFORT=off`, which crashes the child at config init.
/// - both absent (Claude/Codex, unknown/custom): raw passthrough — the value
///   rides `BUZZ_ACP_EFFORT_LEVEL` to an adapter that accepts any string.
pub(crate) fn normalize_effort(
    contract: Option<&EffortNormalization>,
    accepted: Option<&[&str]>,
    raw: &str,
) -> Option<String> {
    match contract {
        Some(c) => c.normalize_str(raw),
        None => match accepted {
            Some(values) => {
                let lower = raw.trim().to_ascii_lowercase();
                values.iter().any(|v| *v == lower).then_some(lower)
            }
            None => Some(raw.to_string()),
        },
    }
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

/// Strip every known effort key from a [`std::process::Command`] before the
/// descriptor overlay is written.
///
/// Only used in tests to verify tombstone assertions on individual keys.
/// Production stripping runs inside `apply_effort_launch_to_command`
/// (the loop over `launch.suppress`) which is exercised by the
/// production-sequence tests.
#[cfg(test)]
pub(crate) fn strip_effort_keys_from_command(cmd: &mut std::process::Command) {
    for key in effort_suppress_keys() {
        cmd.env_remove(key);
        // Belt-and-suspenders for Unix inherited env with non-canonical casing
        // (e.g. a shell export of `goose_thinking_effort`). Our own cmd.env()
        // calls always use UPPER_SNAKE_CASE; only ambient inherited keys can
        // arrive in non-standard case on Unix.
        let lower = key.to_ascii_lowercase();
        if lower != key {
            cmd.env_remove(&lower);
        }
    }
}

/// Strip effort keys and emit the projected effort value to a
/// [`std::process::Command`].
///
/// This is the production command-boundary seam: call after
/// `build_buzz_agent_provider_defaults` (which writes raw baked env) and
/// before the `descriptor.env` loop (which overlays the projected key).
/// Extracting both steps into one call lets tests exercise the full
/// baked-write → strip → emit sequence and inspect the child's effective
/// environment, making the test fail if either step is removed or misordered
/// in production.
///
/// Strip policy follows `launch.suppress`: for known runtimes that is the full
/// effort vocabulary; for unknown/custom runtimes it is only the ACP sentinel,
/// leaving foreign effort keys (e.g. a wrapper's own `GOOSE_THINKING_EFFORT`)
/// untouched. Each key is stripped in canonical and lowercase form so ambient
/// inherited env with non-canonical casing is swept on Unix.
///
/// When `launch.preserve_passthrough` is set and `launch.value` is `None`
/// (unknown runtime, no authoritative column), the suppress set is skipped
/// entirely: the inherited process env carries the user's hand-set sentinel,
/// and stripping it here without a re-emit would silently drop it. Known
/// runtimes always have a resolved `value` or do not set `preserve_passthrough`.
pub(crate) fn apply_effort_launch_to_command(
    cmd: &mut std::process::Command,
    launch: &EffortLaunch,
) {
    // For unknown/custom runtimes with no resolved value the suppress set is
    // only the ACP sentinel, and stripping it without re-emitting would destroy
    // the user's ambient pass-through config. Skip the strip entirely and let
    // the inherited env carry it through unchanged.
    // MUTATION: removing this guard strips the sentinel and breaks
    // `production_sequence_custom_inherited_acp_sentinel_survives`.
    if launch.preserve_passthrough && launch.value.is_none() {
        return;
    }
    for key in &launch.suppress {
        cmd.env_remove(key);
        let lower = key.to_ascii_lowercase();
        if lower.as_str() != *key {
            cmd.env_remove(&lower);
        }
    }
    if let Some(ref value) = launch.value {
        cmd.env(launch.key, value);
    }
}

/// The effort keys the restart snapshot must strip from its captured launch env
/// so effort keeps exactly ONE representation (`effort_level`), mirroring what
/// [`effort_launch_projection`] actually suppressed for `runtime`:
///
/// - **known runtime** — the full suppress set. The projection already swept
///   every effort key to the single destination key, so this removes only that
///   destination key (a no-op on the already-swept siblings).
/// - **unknown/custom runtime** — only the ACP-startup sentinel. The projection
///   suppresses just the sentinel here (reconciling every case variant to the
///   canonical spelling — external review, Carl P2), leaving every other
///   effort-looking key (e.g. a hand-rolled `GOOSE_THINKING_EFFORT`) untouched
///   as ordinary env. Those must remain in `env` so an edit to them diffs the
///   snapshot normally; only the sentinel — the key the projection emits and
///   `effective_effort` reads into `effort_level` — is removed.
pub(crate) fn snapshot_suppress_keys(runtime: Option<&KnownAcpRuntime>) -> Vec<&'static str> {
    if runtime.is_some() {
        effort_suppress_keys()
    } else {
        vec![effort_dest_key(runtime)]
    }
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

    // Suppress the full effort vocabulary for KNOWN runtimes. For an
    // unknown/custom runtime (external review #2) we keep every foreign
    // effort-looking key as pass-through — a hand-rolled `GOOSE_THINKING_EFFORT`
    // on a custom Goose wrapper must reach the child untouched — EXCEPT our own
    // ACP-startup sentinel, which we always reconcile to a single canonical
    // spelling (external review, Carl P2): the projection emits the sentinel, so
    // a user-set case variant (e.g. `buzz_acp_effort_level`) is never intentional
    // config, and leaving one to shadow the emitted `BUZZ_ACP_EFFORT_LEVEL` on
    // Windows (where `Command` case-folds env names) would hand the child a
    // different value than the snapshot reads. Stripping the sentinel here and
    // re-emitting canonically guarantees at most ONE sentinel spelling downstream,
    // so the child, the restart snapshot, and the badge cannot disagree on case.
    let suppress = if runtime.is_some() {
        effort_suppress_keys()
    } else {
        vec![ACP_STARTUP_EFFORT_KEY]
    };
    // When no tier resolves a value, an unknown runtime still preserves a
    // hand-set sentinel the user routed to the child (the retained pass-through
    // from external review #2) — carried forward and re-emitted canonically by
    // `apply`. Known runtimes never preserve a bare dest-key value: it is either
    // the projection's own emission or a foreign key, both handled by `value`.
    let preserve_passthrough = runtime.is_none();

    // Value gate: Goose canonicalizes through its alias contract; buzz-agent
    // validates against its accepted set (invalid → skip, so a foreign
    // canonical like Goose `off` is never emitted where the destination parser
    // rejects it); Claude/Codex and unknown/custom pass raw over the sentinel.
    let contract = runtime.and_then(|r| r.effort_normalization);
    let accepted = runtime.and_then(|r| r.effort_accepted_values);
    let norm = |raw: &str| -> Option<String> { normalize_effort(contract, accepted, raw) };

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
        preserve_passthrough,
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
        if let Some(raw) = get_ci(&record_env, nk) {
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
            if let Some(raw) = get_ci(&record_env, LEGACY_THINKING_EFFORT_KEY) {
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
    if let Some(raw) = get_ci(baked_env, nk) {
        if let Some(v) = norm(raw) {
            return Some(v);
        }
    }
    None
}

/// Combined spawn seam: baked-env write + effort strip + emit.
///
/// Called by `apply_effort_to_spawn_command` in `runtime.rs` (production path)
/// and by `effort_cmd_tests` (test seam). Deleting `build_buzz_agent_provider_defaults`
/// or `apply_effort_launch_to_command` inside turns the production-sequence tests RED.
/// Deleting the outer `apply_effort_to_spawn_command` call from `spawn_agent_child`
/// is a compile error — `spawn_with_effort_proof` consumes the returned `EffortApplied`
/// token, so removing the binding leaves `effort` undefined at the spawn site.
pub(crate) fn apply_spawn_effort_env(
    cmd: &mut std::process::Command,
    record: &ManagedAgentRecord,
    runtime: Option<&KnownAcpRuntime>,
    personas: &[AgentDefinition],
    persona_id: Option<&str>,
    global_env: &BTreeMap<String, String>,
    baked_env: &BTreeMap<String, String>,
) {
    crate::managed_agents::agent_env::build_buzz_agent_provider_defaults(cmd);
    let launch = effort_launch_projection(
        record, runtime, personas, persona_id, global_env, None, baked_env,
    );
    apply_effort_launch_to_command(cmd, &launch);
}

#[cfg(test)]
#[path = "effort_tests.rs"]
mod tests;
