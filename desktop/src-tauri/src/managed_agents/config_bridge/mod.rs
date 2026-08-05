mod buzz_agent;
mod claude;
mod codex;
mod goose;
pub(crate) mod reader;
mod schema_walker;
pub(crate) mod types;

pub(crate) use types::*;

/// The legacy effort env key written by pre-migration saves.
///
/// Harnesses whose native `thinking_env_var` differs from this constant
/// (Goose uses `GOOSE_THINKING_EFFORT`, Claude uses `CLAUDE_CODE_EFFORT_LEVEL`)
/// need the alias resolver below to translate old saves.
/// buzz-agent's native key equals this constant, so no aliasing applies there.
pub(crate) const LEGACY_THINKING_EFFORT_KEY: &str = "BUZZ_AGENT_THINKING_EFFORT";

/// Return all known native thinking-effort env keys across all runtimes.
///
/// Derived from `KNOWN_ACP_RUNTIMES::thinking_env_var` so that adding a new
/// runtime automatically participates in foreign-key stripping without a
/// separate constant to update.
///
/// Callers that need the slice for iterating (e.g. foreign-key stripping in
/// `apply_effort_bridge`) should call this function rather than maintaining
/// a parallel constant.
pub(crate) fn all_known_effort_keys() -> impl Iterator<Item = &'static str> {
    crate::managed_agents::discovery::KNOWN_ACP_RUNTIMES
        .iter()
        .filter_map(|rt| rt.thinking_env_var)
}

/// Resolve the thinking-effort value for a single env-var tier map, with
/// within-tier legacy aliasing and normalization.
///
/// Returns the **canonical** value (normalized via `norm`) for the tier, or
/// `None` when no usable candidate exists.
///
/// Lookup order (applied independently per tier, not globally):
///   1. Native key (`native_key`) — value normalized; invalid values skip as absent.
///   2. Legacy key (`BUZZ_AGENT_THINKING_EFFORT`) — honoured only when:
///      (a) `native_key` differs from the legacy key (i.e. non-buzz-agent runtime), AND
///      (b) `allow_legacy_alias` is true (record and persona tiers only), AND
///      (c) the value normalizes to a canonical form.
///      An invalid legacy value is skipped so the next tier can supply a candidate.
///
/// The `norm` function normalizes a raw value to canonical form; `None` = invalid.
///
/// ## Per-tier `allow_legacy_alias` policy (plan v3)
///
/// | Tier        | `allow_legacy_alias` | Rationale                                        |
/// |-------------|----------------------|--------------------------------------------------|
/// | record      | `true`               | Record-level legacy key migrated at save         |
/// | persona     | `true`               | Persona-level legacy key migrated at save        |
/// | global      | `false`              | Global legacy excluded end-to-end (Delta 2/5)    |
/// | definition  | `false`              | Definition env is author-controlled; legacy alias|
/// |             |                      | would silently conflate foreign effort           |
/// | baked       | `false`              | Build floor; only native key is authoritative    |
pub(crate) fn effort_tier_alias(
    map: &std::collections::BTreeMap<String, String>,
    native_key: &str,
    norm: impl Fn(&str) -> Option<String>,
    allow_legacy_alias: bool,
) -> Option<String> {
    // Native key first — normalize the value; invalid → skip.
    if let Some(raw) = map.get(native_key) {
        if let Some(canonical) = norm(raw) {
            return Some(canonical);
        }
        // Invalid native value: skip-as-absent, fall through to legacy.
    }
    // Legacy alias — only when keys differ and this tier permits legacy consumption.
    if allow_legacy_alias && native_key != LEGACY_THINKING_EFFORT_KEY {
        if let Some(raw) = map.get(LEGACY_THINKING_EFFORT_KEY) {
            if let Some(canonical) = norm(raw) {
                return Some(canonical);
            }
            // Invalid legacy value for this harness: skip, fall through to next tier.
        }
    }
    None
}

/// Read the goose harness config file (`~/.config/goose/config.yaml`).
///
/// Used by readiness evaluation to silence requirements that are already
/// satisfied in the file config layer — the harness reads this file at startup
/// so env vars we would otherwise require are not needed from Buzz.
pub(crate) fn read_goose_file_config() -> Option<RuntimeFileConfig> {
    goose::read_config_file()
}

/// Apply the spawn-side legacy effort bridge to an already-merged effective env.
///
/// For runtimes with a static effort vocabulary (`effort_normalization` is `Some`):
/// 1. Walk per-tier sanitized maps in tier-first precedence order and resolve the
///    canonical effort value.
/// 2. Strip all foreign known effort keys from `env` (runtime-scoped invariant).
/// 3. Remove any raw (possibly invalid/alias-form) entry for the native key.
/// 4. Insert the canonical value under the native key (if any tier resolved one).
///
/// Tier order (spawn; ACP and file tiers absent):
///   record native → record legacy → persona native → persona legacy
///   → global native → definition native → baked native
///
/// Global and definition legacy are excluded (plan v3 Delta 2).
/// Baked tier is native-only: build-floor values are already canonical;
/// applying the legacy alias there would silently consume a foreign key.
#[allow(clippy::too_many_arguments)] // baked tier is a required 8th param; grouping into a struct is premature
pub(crate) fn apply_effort_bridge(
    env: &mut std::collections::BTreeMap<String, String>,
    runtime: Option<&crate::managed_agents::discovery::KnownAcpRuntime>,
    record_env: &std::collections::BTreeMap<String, String>,
    personas: &[crate::managed_agents::types::AgentDefinition],
    persona_id: Option<&str>,
    global_env: &std::collections::BTreeMap<String, String>,
    harness_def: Option<&crate::managed_agents::custom_harnesses::HarnessDefinition>,
    baked_env: &std::collections::BTreeMap<String, String>,
) {
    use std::collections::BTreeMap;

    let rt = match runtime {
        Some(rt) => rt,
        None => return,
    };
    // Strip foreign known effort keys for any runtime that has a native effort key,
    // regardless of whether it has an effort_normalization contract.
    // This ensures GOOSE_THINKING_EFFORT is absent from buzz-agent descriptors and vice versa.
    // Derived from runtime declarations — adding a new runtime automatically participates.
    if let Some(native_key) = &rt.thinking_env_var {
        for key in all_known_effort_keys() {
            if key != *native_key {
                env.remove(key);
            }
        }
    }
    // Effort tier resolution and alias normalization require effort_normalization.
    let (norm, native_key) = match (&rt.effort_normalization, &rt.thinking_env_var) {
        (Some(n), Some(k)) => (n, k),
        _ => return,
    };
    let norm_fn = |raw: &str| norm.normalize_str(raw);
    let mue = crate::managed_agents::env_vars::merged_user_env;
    let is_reserved = crate::managed_agents::env_vars::is_reserved_env_key;
    let live_persona_env = crate::managed_agents::env_vars::live_persona_env;

    let s_record = mue(&BTreeMap::new(), record_env);
    let s_persona = mue(&BTreeMap::new(), &live_persona_env(personas, persona_id));
    let s_global = mue(&BTreeMap::new(), global_env);
    let s_def: BTreeMap<String, String> = harness_def
        .map(|d| {
            d.env
                .iter()
                .filter(|(k, _)| !is_reserved(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();

    // Baked tier: native-key only (no legacy alias — build floor is already
    // canonical; allowing legacy here would silently conflate a foreign effort key).
    // Only the native key is extracted to avoid carrying arbitrary baked keys.
    let baked_native: BTreeMap<String, String> = baked_env
        .get(*native_key)
        .map(|v| [(native_key.to_string(), v.clone())].into_iter().collect())
        .unwrap_or_default();

    // Tier precedence (highest → lowest):
    //   record (legacy allowed) → persona (legacy allowed) → global (no legacy)
    //   → definition (no legacy) → baked (no legacy)
    let canonical = None
        .or_else(|| effort_tier_alias(&s_record, native_key, norm_fn, true))
        .or_else(|| effort_tier_alias(&s_persona, native_key, norm_fn, true))
        .or_else(|| effort_tier_alias(&s_global, native_key, norm_fn, false))
        .or_else(|| effort_tier_alias(&s_def, native_key, norm_fn, false))
        .or_else(|| effort_tier_alias(&baked_native, native_key, norm_fn, false));

    // Remove raw native key (may be alias-form or invalid); canonical re-inserted below.
    env.remove(*native_key);
    if let Some(value) = canonical {
        env.insert(native_key.to_string(), value);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// Goose runtime from the catalog — has static effort vocabulary and
    /// native key `GOOSE_THINKING_EFFORT`.
    fn goose_rt() -> &'static crate::managed_agents::discovery::KnownAcpRuntime {
        crate::managed_agents::discovery::known_acp_runtime_exact("goose")
            .expect("goose must be in catalog")
    }

    /// Claude Code runtime from the catalog — has static effort vocabulary and
    /// native key `CLAUDE_CODE_EFFORT_LEVEL`.
    fn claude_rt() -> &'static crate::managed_agents::discovery::KnownAcpRuntime {
        crate::managed_agents::discovery::known_acp_runtime_exact("claude")
            .expect("claude must be in catalog")
    }

    fn empty_personas() -> Vec<crate::managed_agents::types::AgentDefinition> {
        Vec::new()
    }

    fn env_with(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // ── baked tier tests ──────────────────────────────────────────────────────

    /// Baked `GOOSE_THINKING_EFFORT=high` with no higher-precedence tier
    /// → canonical `high` survives in the effective env.
    #[test]
    fn baked_high_value_spawns_as_effort() {
        let baked = env_with(&[("GOOSE_THINKING_EFFORT", "high")]);
        let record = BTreeMap::new();
        let global = BTreeMap::new();
        let mut env = baked.clone(); // spawn starts with baked floor

        apply_effort_bridge(
            &mut env,
            Some(goose_rt()),
            &record,
            &empty_personas(),
            None,
            &global,
            None,
            &baked,
        );

        assert_eq!(
            env.get("GOOSE_THINKING_EFFORT").map(String::as_str),
            Some("high"),
            "baked valid native value must survive to launch"
        );
    }

    /// Baked `GOOSE_THINKING_EFFORT=xhigh` → normalized to canonical `max`.
    #[test]
    fn baked_xhigh_normalizes_to_max() {
        let baked = env_with(&[("GOOSE_THINKING_EFFORT", "xhigh")]);
        let mut env = baked.clone();

        apply_effort_bridge(
            &mut env,
            Some(goose_rt()),
            &BTreeMap::new(),
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &baked,
        );

        assert_eq!(
            env.get("GOOSE_THINKING_EFFORT").map(String::as_str),
            Some("max"),
            "baked xhigh alias must normalize to canonical max"
        );
    }

    /// Baked `GOOSE_THINKING_EFFORT=minimal` (invalid for Goose) → key absent from env.
    #[test]
    fn baked_invalid_minimal_skipped() {
        let baked = env_with(&[("GOOSE_THINKING_EFFORT", "minimal")]);
        let mut env = baked.clone();

        apply_effort_bridge(
            &mut env,
            Some(goose_rt()),
            &BTreeMap::new(),
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &baked,
        );

        assert!(
            !env.contains_key("GOOSE_THINKING_EFFORT"),
            "baked invalid value must be skipped (key absent from launch env)"
        );
    }

    /// Baked env contains `BUZZ_AGENT_THINKING_EFFORT=high` (legacy key for Goose).
    /// The baked tier is native-key-only — the legacy key must NOT be aliased.
    #[test]
    fn baked_legacy_key_not_aliased_in_baked_tier() {
        // The baked env has only the legacy key — no Goose-native key.
        let baked = env_with(&[("BUZZ_AGENT_THINKING_EFFORT", "high")]);
        let mut env = baked.clone();

        apply_effort_bridge(
            &mut env,
            Some(goose_rt()),
            &BTreeMap::new(),
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &baked,
        );

        // Legacy key should be stripped (foreign to Goose) and native key absent.
        assert!(
            !env.contains_key("GOOSE_THINKING_EFFORT"),
            "baked legacy key must not produce a native effort value (no aliasing in baked tier)"
        );
        // The legacy foreign key is also stripped by the foreign-key sweep.
        assert!(
            !env.contains_key("BUZZ_AGENT_THINKING_EFFORT"),
            "foreign effort key must be stripped from Goose's env"
        );
    }

    /// Record-level effort beats baked — record `max` wins over baked `high`.
    #[test]
    fn baked_beaten_by_record() {
        let baked = env_with(&[("GOOSE_THINKING_EFFORT", "high")]);
        let record = env_with(&[("GOOSE_THINKING_EFFORT", "max")]);
        let mut env = {
            let mut e = baked.clone();
            for (k, v) in &record {
                e.insert(k.clone(), v.clone());
            }
            e
        };

        apply_effort_bridge(
            &mut env,
            Some(goose_rt()),
            &record,
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &baked,
        );

        assert_eq!(
            env.get("GOOSE_THINKING_EFFORT").map(String::as_str),
            Some("max"),
            "record-level effort must beat baked floor"
        );
    }

    // ── definition tier alias policy ──────────────────────────────────────────

    fn def_with_env(
        pairs: &[(&str, &str)],
    ) -> crate::managed_agents::custom_harnesses::HarnessDefinition {
        crate::managed_agents::custom_harnesses::HarnessDefinition {
            id: "test-def".to_string(),
            label: "Test Definition".to_string(),
            command: "goose".to_string(),
            args: Vec::new(),
            env: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            install_instructions_url: String::new(),
            install_hint: String::new(),
        }
    }

    /// Definition env containing only the legacy key `BUZZ_AGENT_THINKING_EFFORT=high`
    /// for a Goose agent → the bridge must NOT alias it; native key remains absent.
    /// Mirrors reader.rs: definition tier uses `allow_legacy_alias=false`.
    #[test]
    fn definition_legacy_key_excluded_spawn() {
        let harness_def = def_with_env(&[("BUZZ_AGENT_THINKING_EFFORT", "high")]);
        let mut env = env_with(&[("BUZZ_AGENT_THINKING_EFFORT", "high")]);

        apply_effort_bridge(
            &mut env,
            Some(goose_rt()),
            &BTreeMap::new(),
            &empty_personas(),
            None,
            &BTreeMap::new(),
            Some(&harness_def),
            &BTreeMap::new(),
        );

        assert!(
            !env.contains_key("GOOSE_THINKING_EFFORT"),
            "definition-tier legacy key must NOT be aliased to the native effort key"
        );
        // Legacy key is also stripped by the foreign-key sweep.
        assert!(
            !env.contains_key("BUZZ_AGENT_THINKING_EFFORT"),
            "foreign effort key must be stripped from Goose's env"
        );
    }

    /// Definition env with the native key `GOOSE_THINKING_EFFORT=medium`
    /// → the bridge accepts it (native key at definition tier is fine).
    #[test]
    fn definition_native_key_accepted_spawn() {
        let harness_def = def_with_env(&[("GOOSE_THINKING_EFFORT", "medium")]);
        let mut env = env_with(&[("GOOSE_THINKING_EFFORT", "medium")]);

        apply_effort_bridge(
            &mut env,
            Some(goose_rt()),
            &BTreeMap::new(),
            &empty_personas(),
            None,
            &BTreeMap::new(),
            Some(&harness_def),
            &BTreeMap::new(),
        );

        assert_eq!(
            env.get("GOOSE_THINKING_EFFORT").map(String::as_str),
            Some("medium"),
            "definition native key must be accepted and reinserted as canonical"
        );
    }

    // ── Claude Code effort bridge ─────────────────────────────────────────────

    /// Claude `CLAUDE_CODE_EFFORT_LEVEL=high` with no higher tier
    /// → canonical `high` survives in the effective env.
    #[test]
    fn claude_native_effort_spawns_correctly() {
        let record = env_with(&[("CLAUDE_CODE_EFFORT_LEVEL", "high")]);
        let mut env = record.clone();

        apply_effort_bridge(
            &mut env,
            Some(claude_rt()),
            &record,
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
        );

        assert_eq!(
            env.get("CLAUDE_CODE_EFFORT_LEVEL").map(String::as_str),
            Some("high"),
            "Claude native effort must survive to launch"
        );
    }

    /// Claude has no aliases — an unrecognised value is invalid
    /// for Claude Code's vocabulary and must be skipped as absent.
    #[test]
    fn claude_invalid_value_skipped_as_absent() {
        // An invalid value (not in Claude's 5-value vocabulary) is skipped as absent.
        // Note: unlike Goose where xhigh aliases to max, Claude's vocabulary is
        // low|medium|high|xhigh|max — xhigh is canonical, so test with a truly
        // invalid string.
        let record = env_with(&[("CLAUDE_CODE_EFFORT_LEVEL", "invalid_val")]);
        let mut env = record.clone();

        apply_effort_bridge(
            &mut env,
            Some(claude_rt()),
            &record,
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
        );

        assert!(
            !env.contains_key("CLAUDE_CODE_EFFORT_LEVEL"),
            "invalid Claude effort must be skipped (key absent from launch env)"
        );
    }

    /// Claude foreign-key stripping: `GOOSE_THINKING_EFFORT` and
    /// `BUZZ_AGENT_THINKING_EFFORT` must be absent from a Claude agent's env.
    #[test]
    fn claude_strips_foreign_effort_keys() {
        let mut env = env_with(&[
            ("GOOSE_THINKING_EFFORT", "high"),
            ("BUZZ_AGENT_THINKING_EFFORT", "medium"),
            ("CLAUDE_CODE_EFFORT_LEVEL", "low"),
        ]);

        apply_effort_bridge(
            &mut env,
            Some(claude_rt()),
            &env_with(&[("CLAUDE_CODE_EFFORT_LEVEL", "low")]),
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
        );

        assert!(
            !env.contains_key("GOOSE_THINKING_EFFORT"),
            "Goose key must be stripped from Claude env"
        );
        assert!(
            !env.contains_key("BUZZ_AGENT_THINKING_EFFORT"),
            "Buzz legacy key must be stripped from Claude env"
        );
        assert_eq!(
            env.get("CLAUDE_CODE_EFFORT_LEVEL").map(String::as_str),
            Some("low"),
            "Claude native effort must survive"
        );
    }

    /// Claude legacy alias at record tier: `BUZZ_AGENT_THINKING_EFFORT` with a
    /// value valid in Claude's vocabulary IS aliased at record/persona tier
    /// (plan v3 Delta 2: legacy alias applies to record and persona tiers for
    /// non-buzz-agent harnesses). Invalid values are not aliased.
    #[test]
    fn claude_legacy_key_aliased_at_record_tier_when_valid() {
        // record contains only the legacy key with a value valid in Claude's vocab
        let record = env_with(&[("BUZZ_AGENT_THINKING_EFFORT", "high")]);
        let mut env = record.clone();

        apply_effort_bridge(
            &mut env,
            Some(claude_rt()),
            &record,
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
        );

        // Legacy key is aliased to Claude's native key at record tier (valid value).
        assert_eq!(
            env.get("CLAUDE_CODE_EFFORT_LEVEL").map(String::as_str),
            Some("high"),
            "valid legacy key at record tier must be aliased to Claude native key"
        );
        assert!(
            !env.contains_key("BUZZ_AGENT_THINKING_EFFORT"),
            "legacy key must be stripped (foreign key sweep)"
        );
    }

    /// Legacy value that is valid for Goose but NOT for Claude (`off`) must NOT
    /// be aliased — normalization via Claude's vocabulary rejects it.
    #[test]
    fn claude_legacy_key_not_aliased_when_value_invalid_for_claude() {
        // "off" is valid for Goose but not in Claude's 5-value vocabulary.
        let record = env_with(&[("BUZZ_AGENT_THINKING_EFFORT", "off")]);
        let mut env = record.clone();

        apply_effort_bridge(
            &mut env,
            Some(claude_rt()),
            &record,
            &empty_personas(),
            None,
            &BTreeMap::new(),
            None,
            &BTreeMap::new(),
        );

        assert!(
            !env.contains_key("CLAUDE_CODE_EFFORT_LEVEL"),
            "invalid-for-Claude legacy value must not produce a native effort key"
        );
        assert!(
            !env.contains_key("BUZZ_AGENT_THINKING_EFFORT"),
            "legacy key must be stripped as a foreign key regardless"
        );
    }
}
