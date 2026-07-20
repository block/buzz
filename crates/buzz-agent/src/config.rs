use std::time::Duration;

pub const PROTOCOL_VERSION: u32 = 2;

/// Reasoning/thinking effort level for providers that support it.
///
/// Set via `BUZZ_AGENT_THINKING_EFFORT` (`none|minimal|low|medium|high|xhigh|max`).
/// When unset the provider's default behaviour is preserved — no thinking
/// config is sent in the request body.
///
/// Provider support (doc-verified, July 2025):
/// - **Anthropic adaptive**: `low|medium|high|xhigh|max` (model-dependent; see `anthropic_thinking_config`).
///   `none`/`minimal` are not Anthropic values — rejected at startup.
/// - **Anthropic manual budget** (claude-3*, opus-4-5): `low|medium|high`; `xhigh`/`max` clamp to high budget.
/// - **OpenAI Responses / Chat Completions**: effort support is model-dependent and normalized at
///   request time; `max` is valid for documented max-supporting families such as GPT-5.6.
/// - **Databricks**: routed by model family (Claude → Anthropic mapping, GPT-5 → Responses, MLflow → Chat).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThinkingEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl ThinkingEffort {
    /// Map level to an Anthropic `budget_tokens` value for legacy Claude 3.x / Opus 4.5 models.
    /// `XHigh` and `Max` clamp to the high budget value; the answer-room reserve of 1024 tokens
    /// is applied separately in `anthropic_thinking_config`.
    pub fn anthropic_budget_tokens(self) -> u32 {
        match self {
            ThinkingEffort::Low => 1_024,
            ThinkingEffort::Medium => 8_192,
            ThinkingEffort::High | ThinkingEffort::XHigh | ThinkingEffort::Max => 32_768,
            // None/Minimal are not valid for Anthropic (rejected at startup); treat as zero
            // defensively so a misconfigured call doesn't accidentally enable thinking.
            ThinkingEffort::None | ThinkingEffort::Minimal => 0,
        }
    }

    /// Map level to an OpenAI `reasoning.effort` / `reasoning_effort` string.
    pub fn openai_effort_str(self) -> &'static str {
        match self {
            ThinkingEffort::None => "none",
            ThinkingEffort::Minimal => "minimal",
            ThinkingEffort::Low => "low",
            ThinkingEffort::Medium => "medium",
            ThinkingEffort::High => "high",
            ThinkingEffort::XHigh => "xhigh",
            ThinkingEffort::Max => "max",
        }
    }

    /// Map level to an Anthropic `output_config.effort` string.
    /// Returns the level string if supported, or the highest supported level for the model.
    /// Caller must apply model-level clamping via `clamp_for_anthropic_adaptive`.
    pub fn anthropic_effort_str(self) -> &'static str {
        match self {
            ThinkingEffort::Low => "low",
            ThinkingEffort::Medium => "medium",
            ThinkingEffort::High => "high",
            ThinkingEffort::XHigh => "xhigh",
            ThinkingEffort::Max => "max",
            // None/Minimal are rejected at startup for Anthropic; defensive fallback.
            ThinkingEffort::None | ThinkingEffort::Minimal => "low",
        }
    }
}

/// Strip any endpoint-naming prefix from a model name so the family classifiers
/// (`is_manual_budget_model`, `is_adaptive_thinking_model`, etc.) can match on the canonical
/// `claude-*` form regardless of how the model is stored in the Databricks catalog.
///
/// Rather than maintaining an allowlist of known prefixes, this function finds the first
/// occurrence of a known model-family token (`claude-`, `gpt-`) and drops everything before
/// it. This handles any endpoint naming convention without needing to enumerate prefixes.
///
/// Examples:
/// - `databricks-claude-fable-5`  → `claude-fable-5`
/// - `goose-claude-fable-5`       → `claude-fable-5`
/// - `team-x-claude-opus-4-7`     → `claude-opus-4-7`
/// - `goose-gpt-5.5`              → `gpt-5.5`
/// - `llama-3`                    → `llama-3` (no family token, returned unchanged)
///
/// If no family token is present the name is returned unchanged.
fn strip_catalog_prefix(model: &str) -> &str {
    const FAMILY_TOKENS: &[&str] = &["claude-", "gpt-"];
    let lower = model.to_ascii_lowercase();
    let first_idx = FAMILY_TOKENS.iter().filter_map(|tok| lower.find(tok)).min();
    match first_idx {
        Some(idx) => &model[idx..],
        None => model,
    }
}

/// Build the Anthropic thinking/effort request fields for the given model and effort level.
///
/// API shape selection (per Anthropic extended-thinking support table,
/// https://platform.claude.com/docs/en/build-with-claude/extended-thinking, July 2025):
///
/// **Adaptive families** — `thinking: {type:"adaptive"}` + `output_config: {effort}`.
/// These models use adaptive thinking; `thinking:{type:"adaptive"}` is required to enable
/// thinking — without it requests run without thinking even when `output_config.effort` is set.
/// Doc-verified (extended-thinking table): Opus 4.8, Opus 4.7, Opus 4.6, Sonnet 5.x, Sonnet 4.6.
/// Matched by explicit version strings (no wildcard over version numbers).
///
/// **Manual-budget families** — `thinking: {type:"enabled", budget_tokens}`.
/// `budget_tokens` is clamped to `min(level_budget, max_output_tokens - 1024)` to preserve
/// at least 1024 answer tokens. If the result is < 1024 (i.e., `max_output_tokens <= 2047`),
/// thinking is omitted entirely with a `warn!`.
/// Doc-verified: claude-3* (legacy), claude-opus-4-5 (effort page: "uses manual thinking").
///
/// **Everything else** — omit both fields. This includes unknown/future `claude-*` names
/// not yet in the support table. Safer to omit than to guess an unverified shape.
///
/// The Databricks `databricks-` and other endpoint-naming prefixes are stripped before
/// matching so that `databricks-claude-opus-4-7`, `goose-claude-fable-5`, and
/// `team-x-claude-opus-4-7` all route to the correct bucket. See `strip_catalog_prefix`.
///
/// Returns `(thinking_field, output_config_field)` where each is `None` if not applicable.
pub fn anthropic_thinking_config(
    effective_model: &str,
    effort: ThinkingEffort,
    max_output_tokens: u32,
) -> (Option<serde_json::Value>, Option<serde_json::Value>) {
    use serde_json::json;
    // Normalise the model name for matching: strip any endpoint-naming prefix
    // (e.g. "databricks-claude-opus-4-7" → "claude-opus-4-7",
    //       "goose-claude-fable-5"        → "claude-fable-5",
    //       "team-x-claude-opus-4-7"      → "claude-opus-4-7").
    let model = strip_catalog_prefix(effective_model);

    if is_manual_budget_model(model) {
        // Manual-budget shape: budget_tokens must be strictly < max_tokens AND must leave
        // at least MIN_ANSWER_TOKENS (1024) for the visible answer. The Anthropic API
        // requires budget_tokens < max_tokens AND budget_tokens >= 1024.
        //
        // Clamp: budget = min(level_budget, max_output_tokens - MIN_ANSWER_TOKENS).
        // If result < MIN_ANSWER_TOKENS, thinking would starve the answer — omit thinking
        // entirely and warn instead of emitting an invalid or answer-starving budget.
        const MIN_ANSWER_TOKENS: u32 = 1024;
        let level_budget = effort.anthropic_budget_tokens();
        let headroom = max_output_tokens.saturating_sub(MIN_ANSWER_TOKENS);
        let budget = level_budget.min(headroom);
        if budget < MIN_ANSWER_TOKENS {
            tracing::warn!(
                max_output_tokens,
                level_budget,
                headroom,
                "BUZZ_AGENT_THINKING_EFFORT: max_output_tokens too small to fit thinking budget + answer headroom; omitting thinking fields"
            );
            return (None, None);
        }
        (
            Some(json!({ "type": "enabled", "budget_tokens": budget })),
            None,
        )
    } else if is_adaptive_thinking_model(model) {
        // Adaptive families: thinking must be explicitly enabled via type:"adaptive".
        // output_config.effort controls the depth. Both fields are required together.
        // Apply per-model effort clamping: if the requested level exceeds the model's
        // doc-verified maximum, clamp down to the highest supported level with a warning.
        let clamped = clamp_adaptive_effort(model, effort);
        (
            Some(json!({ "type": "adaptive" })),
            Some(json!({ "effort": clamped.anthropic_effort_str() })),
        )
    } else {
        // Unrecognised or unverified model name — omit both fields rather than guess.
        // This includes unknown future claude-* names not yet in the support table.
        (None, None)
    }
}

/// Returns true for adaptive Anthropic models that support the `xhigh` effort level.
///
/// Used by both `clamp_adaptive_effort` (request-time) and `anthropic_efforts_for_model`
/// (UI capability table) to keep xhigh-support classification in a single place.
///
/// `model` must already have catalog prefixes stripped (via `strip_catalog_prefix`).
fn anthropic_model_supports_xhigh(model: &str) -> bool {
    model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-8")
        || model.starts_with("claude-sonnet-5")
        || model.starts_with("claude-fable-5")
        || model.starts_with("claude-mythos-5")
}

/// Clamp the requested effort level to the highest doc-verified level for the given adaptive model.
///
/// Doc-verified availability (Anthropic effort page, July 2025):
/// - `max`: Opus 4.8, 4.7, 4.6; Sonnet 5.x, 4.6; Fable 5; Mythos 5; Mythos Preview
/// - `xhigh`: Opus 4.8, 4.7; Sonnet 5.x; Fable 5; Mythos 5
///   (NOT Opus 4.6, Sonnet 4.6, or Mythos Preview)
/// - `low|medium|high`: all adaptive families
///
/// If the requested level is not available for the model, clamps down to the highest
/// supported level below the requested one, and logs a warning. This is dynamic (not
/// startup-time) because `session/set_model` can change the model after startup.
///
/// `model` must already have catalog prefixes stripped (via `strip_catalog_prefix`).
pub fn clamp_adaptive_effort(model: &str, effort: ThinkingEffort) -> ThinkingEffort {
    // Models that support all levels including xhigh (and max).
    let supports_xhigh = anthropic_model_supports_xhigh(model);

    let clamped = if supports_xhigh {
        effort // all levels pass through
    } else if effort == ThinkingEffort::XHigh {
        // xhigh not available for this model; clamp to high (the highest supported below xhigh).
        ThinkingEffort::High
    } else {
        effort // low/medium/high/max all pass through for the other adaptive families
    };

    if clamped != effort {
        tracing::warn!(
            model,
            requested = effort.openai_effort_str(),
            clamped = clamped.openai_effort_str(),
            "BUZZ_AGENT_THINKING_EFFORT is not available for this model; clamping to highest supported level"
        );
    }
    clamped
}

/// Returns true if `lower_model` contains `token` as a bounded family segment — i.e., the
/// token is immediately followed by end-of-string or a `-` separator (not a digit or letter).
///
/// This prevents:
/// - `gpt-5.1` from matching `gpt-5.10` (digit follows the `1`)
/// - `gpt-5-1` from matching `gpt-5-1106` (digit follows the `1`)
/// - `gpt-5-4` from matching `gpt-5-4o` (letter follows the `4`)
///
/// Gateway prefixes (`databricks-`) and date/build suffixes (`-2025-04-01`) are allowed
/// because they start with `-` which is the only permitted boundary character.
fn gpt5_token_matches(lower_model: &str, token: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = lower_model[start..].find(token) {
        let abs = start + pos;
        let after = abs + token.len();
        // The character immediately after the token must be end-of-string or '-'.
        // Any alphanumeric character (digit OR letter) means this is a longer token, not
        // the family we're looking for.
        let safe_suffix = lower_model[after..].chars().next().is_none_or(|c| c == '-');
        if safe_suffix {
            return true;
        }
        start = abs + 1;
    }
    false
}

/// Like `gpt5_token_matches` but additionally rejects short version-like numeric suffixes —
/// used for the base `gpt-5` / `gpt5` token to avoid false-matching unrecognized versions.
///
/// After a `-` separator:
/// - `-<non-digit>…` e.g. `-pro` → **accepted** (capability suffix, no digits)
/// - `digit_run == 1-3` AND the char right after the digits is a **letter** e.g. `-4o` →
///   **accepted** (real variant shape: digit + letter)
/// - `digit_run == 1-3` AND the char after the digits is end-of-string, `-`, `.`, or other
///   separator e.g. `-10`, `-10-preview` → **rejected** (version-like suffix)
/// - `digit_run >= 4` regardless of what follows e.g. `-1106`, `-1106-preview`, `-0514` →
///   **accepted** (date/build segment)
fn gpt5_base_matches(lower_model: &str, token: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = lower_model[start..].find(token) {
        let abs = start + pos;
        let after = abs + token.len();
        let rest = &lower_model[after..];
        let safe_suffix = if rest.is_empty() {
            // End of string — clean boundary.
            true
        } else if let Some(tail) = rest.strip_prefix('-') {
            // Count leading digits in the suffix component.
            let digit_run: usize = tail.chars().take_while(|c| c.is_ascii_digit()).count();
            if digit_run == 0 {
                // No leading digit (e.g. '-pro'): capability suffix → accepted.
                true
            } else if digit_run >= 4 {
                // 4+ digit run (e.g. '-1106', '-1106-preview', '-0514'): date/build → accepted.
                true
            } else {
                // 1-3 digit run: accepted only if the char right after the digits is a letter
                // (real variant shape like '-4o'). Separator/EOS after short digits is
                // version-like (e.g. '-10', '-10-preview') → rejected.
                tail[digit_run..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
            }
        } else {
            // Dot, letter, or other non-hyphen character directly after token → not base.
            false
        };
        if safe_suffix {
            return true;
        }
        start = abs + 1;
    }
    false
}

/// Returns the set of `reasoning.effort` values supported by a given OpenAI model family.
///
/// Doc-verified availability (OpenAI model pages, July 2025):
///
/// | Model        | Supported effort values                   |
/// |-------------|-------------------------------------------|
/// | gpt-5-pro   | `high` only                               |
/// | gpt-5.6     | `none, low, medium, high, xhigh, max`     |
/// | gpt-5.5     | `none, low, medium, high, xhigh`          |
/// | gpt-5.4     | `none, low, medium, high, xhigh`          |
/// | gpt-5.1     | `none, low, medium, high`                 |
/// | gpt-5 (base)| `minimal, low, medium, high`              |
/// | unknown     | not doc-verified — `max` clamps to `xhigh` |
///
/// Note the `none` vs `minimal` split: `gpt-5` (base) supports `minimal` but not `none`;
/// `gpt-5.1`/`gpt-5.4`/`gpt-5.5`/`gpt-5.6` support `none` but not `minimal`. These are matched via
/// nearest-supported fallback in `normalize_effort_for_openai_route`.
///
/// Match order: `-pro` variant checked before versioned strings to prevent `gpt-5-pro` from
/// falling into the `gpt-5` base bucket (substring "gpt-5" is shared).
///
/// `model` is a raw model name (may include Databricks gateway prefixes or date suffixes).
/// Unknown models return `None` — callers pass through values except `max`, which clamps to
/// `xhigh` until support is confirmed.
/// Versioned tokens use `gpt5_token_matches` (end-of-string or `-` boundary, blocking digit
/// and letter continuations). The base token uses `gpt5_base_matches`, which additionally
/// rejects short `-<1-3 digit>` suffixes that look like two-digit version numbers.
fn openai_efforts_for_model(model: &str) -> Option<&'static [ThinkingEffort]> {
    // Effort ordered from lowest to highest for each family.
    const GPT5_PRO: &[ThinkingEffort] = &[ThinkingEffort::High];
    const GPT5_6: &[ThinkingEffort] = &[
        ThinkingEffort::None,
        ThinkingEffort::Low,
        ThinkingEffort::Medium,
        ThinkingEffort::High,
        ThinkingEffort::XHigh,
        ThinkingEffort::Max,
    ];
    const GPT5_5_AND_5_4: &[ThinkingEffort] = &[
        ThinkingEffort::None,
        ThinkingEffort::Low,
        ThinkingEffort::Medium,
        ThinkingEffort::High,
        ThinkingEffort::XHigh,
    ];
    const GPT5_1: &[ThinkingEffort] = &[
        ThinkingEffort::None,
        ThinkingEffort::Low,
        ThinkingEffort::Medium,
        ThinkingEffort::High,
    ];
    const GPT5_BASE: &[ThinkingEffort] = &[
        ThinkingEffort::Minimal,
        ThinkingEffort::Low,
        ThinkingEffort::Medium,
        ThinkingEffort::High,
    ];

    let lower = model.to_ascii_lowercase();
    // Check gpt-5-pro before gpt-5.5 / gpt-5.4 etc. to avoid the `-pro` name
    // matching the base "gpt-5" prefix first.
    if gpt5_token_matches(&lower, "gpt-5-pro") || gpt5_token_matches(&lower, "gpt5-pro") {
        Some(GPT5_PRO)
    } else if gpt5_token_matches(&lower, "gpt-5.6")
        || gpt5_token_matches(&lower, "gpt5.6")
        || gpt5_token_matches(&lower, "gpt-5-6")
        || gpt5_token_matches(&lower, "gpt5-6")
    {
        Some(GPT5_6)
    } else if gpt5_token_matches(&lower, "gpt-5.5")
        || gpt5_token_matches(&lower, "gpt5.5")
        || gpt5_token_matches(&lower, "gpt-5-5")
        || gpt5_token_matches(&lower, "gpt5-5")
        || gpt5_token_matches(&lower, "gpt-5.4")
        || gpt5_token_matches(&lower, "gpt5.4")
        || gpt5_token_matches(&lower, "gpt-5-4")
        || gpt5_token_matches(&lower, "gpt5-4")
    {
        // gpt-5.5 and gpt-5.4 share the same effort availability table.
        Some(GPT5_5_AND_5_4)
    } else if gpt5_token_matches(&lower, "gpt-5.1")
        || gpt5_token_matches(&lower, "gpt5.1")
        || gpt5_token_matches(&lower, "gpt-5-1")
        || gpt5_token_matches(&lower, "gpt5-1")
    {
        Some(GPT5_1)
    } else if gpt5_base_matches(&lower, "gpt-5") || gpt5_base_matches(&lower, "gpt5") {
        // Base gpt-5 (no version suffix matching any of the above).
        Some(GPT5_BASE)
    } else {
        // Unknown model — not doc-verified; server validates.
        None
    }
}

/// Returns the effort capability set for a given Anthropic model.
///
/// This is the single production source of truth for Anthropic family routing.
/// Both `anthropic_thinking_config` (request-time) and the effort-table UI
/// (`valid_effort_values_for_provider_model`, via its Anthropic branch) must
/// derive their behaviour from this helper so the two stay in sync.
///
/// Returns `(valid_values, default)` where:
/// - `valid_values` is the static slice of `ThinkingEffort` values accepted
///   by this model family's effort dropdown.
/// - `default` is `None` for manual-budget models (no semantic default —
///   user must choose) or `Some(High)` for adaptive families.
///
/// `model` must already have catalog prefixes stripped (via `strip_catalog_prefix`).
pub fn anthropic_efforts_for_model(
    model: &str,
) -> (&'static [ThinkingEffort], Option<ThinkingEffort>) {
    const MANUAL: &[ThinkingEffort] = &[
        ThinkingEffort::Low,
        ThinkingEffort::Medium,
        ThinkingEffort::High,
    ];
    const ADAPTIVE_XHIGH: &[ThinkingEffort] = &[
        ThinkingEffort::Low,
        ThinkingEffort::Medium,
        ThinkingEffort::High,
        ThinkingEffort::XHigh,
        ThinkingEffort::Max,
    ];
    const ADAPTIVE_NO_XHIGH: &[ThinkingEffort] = &[
        ThinkingEffort::Low,
        ThinkingEffort::Medium,
        ThinkingEffort::High,
        ThinkingEffort::Max,
    ];

    if is_manual_budget_model(model) {
        return (MANUAL, None);
    }
    if is_adaptive_thinking_model(model) {
        // Reuse `anthropic_model_supports_xhigh` (the single source of truth
        // shared with `clamp_adaptive_effort`) — no side-effects, no duplication.
        if anthropic_model_supports_xhigh(model) {
            return (ADAPTIVE_XHIGH, Some(ThinkingEffort::High));
        } else {
            return (ADAPTIVE_NO_XHIGH, Some(ThinkingEffort::High));
        }
    }
    // Unknown Anthropic model — assume full adaptive (xhigh-capable) as a safe default.
    (ADAPTIVE_XHIGH, Some(ThinkingEffort::High))
}

/// Resolve the nearest supported effort level for a given OpenAI model.
///
/// When the requested effort is not in the model's supported set, falls back to the
/// nearest supported level using this preference order:
///
/// - `none` ↔ `minimal` are each other's first fallback (the none/minimal split across
///   model families means the "closest analogue" is the other form before jumping to `low`).
/// - Above that pair: upward clamp first, then downward (prefer more thinking over less).
/// - `xhigh` falls back to `high` when not supported (no model skips from `high` to `xhigh`).
/// - `max` passes through for model families whose table includes it; otherwise it resolves to
///   the nearest supported level.
///
/// Logs a `warn!` on every substitution.
fn resolve_openai_effort(
    model: &str,
    requested: ThinkingEffort,
    supported: &[ThinkingEffort],
) -> ThinkingEffort {
    if supported.contains(&requested) {
        return requested;
    }

    // Build a candidate list ordered by preference: the "other" form of none/minimal first,
    // then the levels sorted nearest to requested (ascending distance).
    let candidates: Vec<ThinkingEffort> = {
        // none ↔ minimal are each other's first fallback.
        let peer = match requested {
            ThinkingEffort::None => Some(ThinkingEffort::Minimal),
            ThinkingEffort::Minimal => Some(ThinkingEffort::None),
            _ => None,
        };
        // All supported values sorted by distance (abs diff in ordinal), upward ties win.
        let mut by_dist: Vec<ThinkingEffort> = supported.to_vec();
        by_dist.sort_by_key(|&e| {
            let dist = (e as i32 - requested as i32).unsigned_abs();
            // Prefer upward (e > requested) to break ties between equidistant values.
            let up = if e >= requested { 0u32 } else { 1 };
            (dist, up)
        });
        // Peer first, then by distance.
        let mut result = Vec::new();
        if let Some(p) = peer {
            if supported.contains(&p) {
                result.push(p);
            }
        }
        for e in by_dist {
            if !result.contains(&e) {
                result.push(e);
            }
        }
        result
    };

    let resolved = candidates
        .into_iter()
        .next()
        .expect("supported is non-empty");

    tracing::warn!(
        %model,
        requested = requested.openai_effort_str(),
        resolved = resolved.openai_effort_str(),
        "BUZZ_AGENT_THINKING_EFFORT={} is not supported by this OpenAI model; using nearest supported level",
        requested.openai_effort_str(),
    );
    resolved
}

/// Normalize the effort value for an OpenAI-shaped request body (Chat Completions or Responses).
///
/// Per-model effort availability is applied for doc-verified OpenAI model families. A requested
/// level not in the model's supported set is substituted with the nearest supported level (see
/// `resolve_openai_effort` for preference order). For unknown/unverified models, `max` is clamped
/// to `xhigh` because its support cannot be confirmed; all other values pass through unchanged.
///
/// Applies to pure-OpenAI request paths AND DBv2 OpenAI-shaped routes.
///
/// Doc-verified model table (July 2025):
/// - `gpt-5-pro`: `high` only
/// - `gpt-5.6`: `none, low, medium, high, xhigh, max`
/// - `gpt-5.5`, `gpt-5.4`: `none, low, medium, high, xhigh`
/// - `gpt-5.1`: `none, low, medium, high`
/// - `gpt-5` (base): `minimal, low, medium, high`
/// - unknown: `max` clamps to `xhigh`; other values pass through
pub fn normalize_effort_for_openai_route(effort: ThinkingEffort, model: &str) -> ThinkingEffort {
    match openai_efforts_for_model(model) {
        Some(supported) => resolve_openai_effort(model, effort, supported),
        None if effort == ThinkingEffort::Max => {
            tracing::warn!(
                requested = "max",
                resolved = "xhigh",
                "BUZZ_AGENT_THINKING_EFFORT=max not confirmed for unknown OpenAI model; clamping to xhigh"
            );
            ThinkingEffort::XHigh
        }
        None => effort,
    }
}

/// Normalize the effort value for an Anthropic-shaped request body (Messages API).
///
/// Anthropic-shaped bodies (`anthropic_body`) do not have a `none` or `minimal` concept —
/// the thinking block is either present (with a level) or absent. When `none` or `minimal`
/// is configured, we omit the thinking fields entirely and log a warning (omission = provider
/// default; default-on/always-on adaptive models may still think). This handles `DatabricksV2`
/// sessions where the route can switch from GPT to Claude via `session/set_model` after startup.
///
/// Returns `None` to signal "omit thinking fields", or the original effort if it is a valid
/// Anthropic level.
pub fn normalize_effort_for_anthropic_route(effort: ThinkingEffort) -> Option<ThinkingEffort> {
    match effort {
        ThinkingEffort::None | ThinkingEffort::Minimal => {
            tracing::warn!(
                requested = effort.openai_effort_str(),
                "BUZZ_AGENT_THINKING_EFFORT={} is not expressible as an Anthropic thinking level; \
                 omitting thinking fields (provider default; default-on/always-on adaptive models may still think)",
                effort.openai_effort_str()
            );
            None
        }
        other => Some(other),
    }
}

/// Returns true for Claude model families that use manual thinking budgets (doc-verified, July 2025).
///
/// Source: https://platform.claude.com/docs/en/build-with-claude/extended-thinking (support table)
/// - claude-3*: legacy manual budget (all Claude 3.x variants).
/// - claude-opus-4-5: effort page states "uses manual thinking, where effort works alongside
///   the thinking token budget" — manual bucket, not adaptive.
///
/// `model` must already have catalog prefixes stripped (via `strip_catalog_prefix`).
fn is_manual_budget_model(model: &str) -> bool {
    model.starts_with("claude-3") || model == "claude-opus-4-5"
}

/// Returns true for Claude model families that use adaptive thinking (doc-verified, July 2025).
///
/// Sources: https://platform.claude.com/docs/en/build-with-claude/extended-thinking (support table)
///          https://platform.claude.com/docs/en/build-with-claude/effort (effort page)
///
/// Adaptive thinking models (always-on or default-on):
///   Opus 4.8, Opus 4.7, Opus 4.6, Sonnet 5.x, Sonnet 4.6,
///   Fable 5 (always-on), Mythos 5 (always-on), Mythos Preview (default-on).
///
/// Note: Opus 4.5 is NOT in this bucket — it uses manual budget (see `is_manual_budget_model`).
/// No prefix wildcards over version numbers; each entry is doc-verified explicitly.
///
/// `model` must already have catalog prefixes stripped (via `strip_catalog_prefix`).
fn is_adaptive_thinking_model(model: &str) -> bool {
    // Exact version strings for Opus 4.x adaptive models (4.6, 4.7, 4.8).
    // Opus 4.5 is excluded — manual budget only.
    model.starts_with("claude-opus-4-6")
        || model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-8")
        // Sonnet 5.x (any patch/date suffix after "claude-sonnet-5").
        || model.starts_with("claude-sonnet-5")
        // Sonnet 4.6 exactly (not Sonnet 4.5 or earlier — not in the adaptive table).
        || model.starts_with("claude-sonnet-4-6")
        // Fable 5 and Mythos 5 (always-on adaptive thinking, July 2025).
        || model.starts_with("claude-fable-5")
        || model.starts_with("claude-mythos-5")
        // Mythos Preview (default-on adaptive thinking, July 2025).
        // Note: xhigh is NOT available on Mythos Preview — clamp_adaptive_effort handles this.
        || model.starts_with("claude-mythos-preview")
}

/// Parse `BUZZ_AGENT_THINKING_EFFORT`. Pure (env-free) for testability.
pub fn parse_thinking_effort(raw: Option<&str>) -> Result<Option<ThinkingEffort>, String> {
    match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") => Ok(None),
        Some("none") => Ok(Some(ThinkingEffort::None)),
        Some("minimal") => Ok(Some(ThinkingEffort::Minimal)),
        Some("low") => Ok(Some(ThinkingEffort::Low)),
        Some("medium") => Ok(Some(ThinkingEffort::Medium)),
        Some("high") => Ok(Some(ThinkingEffort::High)),
        Some("xhigh") => Ok(Some(ThinkingEffort::XHigh)),
        Some("max") => Ok(Some(ThinkingEffort::Max)),
        Some(other) => Err(format!(
            "config: BUZZ_AGENT_THINKING_EFFORT={other} not supported (use none|minimal|low|medium|high|xhigh|max)"
        )),
    }
}

pub const MAX_PROMPT_BYTES: usize = 1024 * 1024;
pub const MAX_SYSTEM_PROMPT_BYTES: usize = 512 * 1024;
/// Total per-result byte ceiling (text + images). Sized for image-bearing
/// results — view_image can legitimately return multi-MiB base64 payloads.
/// Text is governed by the much smaller `BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES`.
pub const MAX_TOOL_RESULT_BYTES: usize = 8 * 1024 * 1024;
/// Default cap on the *text* portion of a single tool result. Oversized text
/// is middle-elided before it enters history; without this, one fat `cat`
/// burns the context window and forces a lossy handoff. 50 KiB matches the
/// shell-output caps in sprout-dev-mcp, goose, and pi; codex defaults to
/// 10 KB. Tunable via `BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES`.
pub const DEFAULT_TOOL_RESULT_TEXT_BYTES: usize = 50 * 1024;
pub const MAX_TOOL_CALLS_PER_TURN: usize = 64;

pub const HANDOFF_MAX_OUTPUT_TOKENS: u32 = 8192;

pub const HANDOFF_ORIGINAL_TASK_MAX_BYTES: usize = 16 * 1024;

pub const HANDOFF_MAX_TOOL_NAMES: usize = 20;

const DEFAULT_SYSTEM_PROMPT: &str =
    "You are buzz-agent. Use the provided tools to act. Tool calls are your only output.";

/// Default base URL for `Provider::Gemini`: Google's OpenAI-compatible surface.
/// Chat Completions and `/models` live under this prefix. Overridable via
/// `GEMINI_BASE_URL`. Native `generateContent` is intentionally not used.
pub(crate) const GEMINI_DEFAULT_BASE_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/openai";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Provider {
    Anthropic,
    OpenAi,
    /// Google Gemini via its OpenAI-compatible surface. Reuses the OpenAI Chat
    /// Completions body builder/parser and a static `GEMINI_API_KEY` bearer;
    /// only the env var names and default base URL differ from `OpenAi`.
    Gemini,
    /// Databricks model serving. Routes to `{base_url}/serving-endpoints/{model}/invocations`
    /// with a dynamically-acquired bearer (OAuth 2.0 PKCE, or static `DATABRICKS_TOKEN`).
    /// Wire format is OpenAI-chat-compatible — reuses the same body builder and parser.
    Databricks,
    /// Databricks AI Gateway v2. Routes by model family through the gateway's
    /// OpenAI Responses, Anthropic Messages, or MLflow Chat Completions paths.
    DatabricksV2,
}

/// Which OpenAI-family HTTP API to call. Set via `OPENAI_COMPAT_API`
/// (`auto|chat|responses`); ignored when `provider = Anthropic`. `Auto`
/// picks Responses for `*.openai.com`, Chat Completions otherwise, and
/// permits a one-shot chat→responses upgrade on a "use /v1/responses"
/// provider error.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenAiApi {
    Chat,
    Responses,
    Auto,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: Provider,
    pub system_prompt: String,
    pub max_rounds: u32,
    pub max_output_tokens: u32,
    pub llm_timeout: Duration,
    pub tool_timeout: Duration,
    pub mcp_init_timeout: Duration,
    pub mcp_max_restart_attempts: u32,
    pub mcp_restart_base_ms: u64,
    pub mcp_restart_max_ms: u64,
    pub max_sessions: usize,
    pub max_line_bytes: usize,
    pub max_history_bytes: usize,
    /// Per-tool-result cap on text content. Oversized text is middle-elided
    /// (head + tail kept) before entering history. Images are exempt — they
    /// are bounded by [`MAX_TOOL_RESULT_BYTES`] and accounted separately.
    /// Set via `BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES`.
    pub max_tool_result_text_bytes: usize,
    /// Provider context window in tokens used to gate handoff. The handoff
    /// fires when the previous request's (cache-summed) input tokens cross the
    /// handoff threshold for this budget, before the next request can exceed
    /// the window and 400. Default 200_000 — matching Claude 4.x windows;
    /// operators lower/raise it for other models. Set via
    /// `BUZZ_AGENT_MAX_CONTEXT_TOKENS`.
    pub max_context_tokens: u64,
    pub max_handoffs: usize,
    pub max_parallel_tools: usize,
    pub hook_timeout: Duration,
    /// Maximum `_Stop` rejections per prompt. Default 3. Set to 0 to
    /// disable `_Stop` hooks entirely (agent always honors end_turn).
    pub stop_max_rejections: u32,
    /// Hook server allowlist. See [`HookServers`] for variant semantics.
    /// Default (env unset/empty) is `None` — hooks are off unless the
    /// operator explicitly opts in.
    pub hook_servers: HookServers,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub anthropic_api_version: String,
    /// OpenAI endpoint selection. See [`OpenAiApi`].
    pub openai_api: OpenAiApi,
    pub hints_enabled: bool,
    /// Thinking/reasoning effort level. `None` = use provider default (no
    /// thinking config sent). Set via `BUZZ_AGENT_THINKING_EFFORT`.
    pub thinking_effort: Option<ThinkingEffort>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let databricks_host = env("DATABRICKS_HOST");
        let databricks_model = env("DATABRICKS_MODEL");
        let provider = resolve_provider(
            env("BUZZ_AGENT_PROVIDER").as_deref(),
            env("ANTHROPIC_API_KEY").as_deref(),
            env("OPENAI_COMPAT_API_KEY").as_deref(),
            env("GEMINI_API_KEY").as_deref(),
        )?;

        // Universal model override — takes priority over provider-specific model
        // env vars (ANTHROPIC_MODEL, OPENAI_COMPAT_MODEL, DATABRICKS_MODEL) when
        // present. Set by the desktop from the persona/record to express explicit
        // user intent; provider-specific vars serve as defaults for CLI/standalone use.
        let buzz_agent_model = env("BUZZ_AGENT_MODEL");

        // OPENAI_COMPAT_API is only read when provider=openai, so a stray
        // bad value can't break an Anthropic-only deployment.
        //
        // Databricks borrows api_key as the *optional* `DATABRICKS_TOKEN` escape
        // hatch — empty means "use OAuth PKCE." Legacy Databricks encodes the
        // model in the URL path; Databricks v2 keeps it in the request body.
        let (api_key, model, base_url, openai_api) = match provider {
            Provider::Anthropic => (
                req("ANTHROPIC_API_KEY")?,
                resolve_model(
                    buzz_agent_model.as_deref(),
                    env("ANTHROPIC_MODEL").as_deref(),
                )
                .ok_or_else(|| "config: ANTHROPIC_MODEL required".to_string())?,
                env_or("ANTHROPIC_BASE_URL", "https://api.anthropic.com"),
                OpenAiApi::Auto, // unused for Anthropic
            ),
            Provider::OpenAi => (
                req("OPENAI_COMPAT_API_KEY")?,
                resolve_model(
                    buzz_agent_model.as_deref(),
                    env("OPENAI_COMPAT_MODEL").as_deref(),
                )
                .ok_or_else(|| "config: OPENAI_COMPAT_MODEL required".to_string())?,
                env_or("OPENAI_COMPAT_BASE_URL", "https://api.openai.com/v1"),
                parse_openai_api(env("OPENAI_COMPAT_API").as_deref())?,
            ),
            // Gemini's OpenAI-compatible surface is Chat Completions only, so the
            // API shape is pinned to Chat rather than Auto (there is no /responses
            // endpoint to upgrade to on generativelanguage.googleapis.com).
            Provider::Gemini => (
                req("GEMINI_API_KEY")?,
                resolve_model(buzz_agent_model.as_deref(), env("GEMINI_MODEL").as_deref())
                    .ok_or_else(|| "config: GEMINI_MODEL required".to_string())?,
                env_or("GEMINI_BASE_URL", GEMINI_DEFAULT_BASE_URL),
                OpenAiApi::Chat,
            ),
            Provider::Databricks | Provider::DatabricksV2 => (
                env("DATABRICKS_TOKEN").unwrap_or_default(),
                resolve_model(buzz_agent_model.as_deref(), databricks_model.as_deref())
                    .ok_or_else(|| "config: DATABRICKS_MODEL required".to_string())?,
                databricks_host.ok_or_else(|| "config: DATABRICKS_HOST required".to_string())?,
                OpenAiApi::Chat, // only read by OpenAI/legacy Databricks dispatch
            ),
        };
        let system_prompt = match (env("BUZZ_AGENT_SYSTEM_PROMPT"), env("BUZZ_AGENT_SYSTEM_PROMPT_FILE")) {
            (Some(_), Some(_)) => return Err(
                "config: BUZZ_AGENT_SYSTEM_PROMPT and BUZZ_AGENT_SYSTEM_PROMPT_FILE are mutually exclusive".into()),
            (Some(s), _) => s,
            (_, Some(p)) => std::fs::read_to_string(&p).map_err(|e| format!("config: read {p}: {e}"))?,
            _ => DEFAULT_SYSTEM_PROMPT.to_owned(),
        };
        let cfg = Config {
            provider,
            system_prompt,
            api_key,
            model,
            base_url,
            anthropic_api_version: env_or("ANTHROPIC_API_VERSION", "2023-06-01"),
            openai_api,
            max_rounds: parse_env("BUZZ_AGENT_MAX_ROUNDS", 0)?,
            max_output_tokens: parse_env("BUZZ_AGENT_MAX_OUTPUT_TOKENS", 32_768)?,
            llm_timeout: Duration::from_secs(parse_env("BUZZ_AGENT_LLM_TIMEOUT_SECS", 240)?),
            tool_timeout: Duration::from_secs(parse_env("BUZZ_AGENT_TOOL_TIMEOUT_SECS", 660)?),
            mcp_init_timeout: Duration::from_secs(parse_env(
                "BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS",
                30,
            )?),
            mcp_max_restart_attempts: parse_env("BUZZ_AGENT_MCP_RESTART_MAX_ATTEMPTS", 3u32)?,
            mcp_restart_base_ms: parse_env("BUZZ_AGENT_MCP_RESTART_BASE_MS", 500u64)?,
            mcp_restart_max_ms: parse_env("BUZZ_AGENT_MCP_RESTART_MAX_MS", 30_000u64)?,
            max_sessions: parse_env("BUZZ_AGENT_MAX_SESSIONS", usize::MAX)?,
            max_line_bytes: parse_env("BUZZ_AGENT_MAX_LINE_BYTES", 4 * 1024 * 1024)?,
            max_history_bytes: parse_env("BUZZ_AGENT_MAX_HISTORY_BYTES", 16 * 1024 * 1024)?,
            max_tool_result_text_bytes: parse_env(
                "BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES",
                DEFAULT_TOOL_RESULT_TEXT_BYTES,
            )?,
            max_context_tokens: parse_env("BUZZ_AGENT_MAX_CONTEXT_TOKENS", 200_000u64)?,
            max_handoffs: parse_env("BUZZ_AGENT_MAX_HANDOFFS", 10)?,
            max_parallel_tools: parse_env("BUZZ_AGENT_MAX_PARALLEL_TOOLS", 8usize)?,
            hook_timeout: Duration::from_millis(parse_env("BUZZ_AGENT_HOOK_TIMEOUT_MS", 2500u64)?),
            stop_max_rejections: parse_env("BUZZ_AGENT_STOP_MAX_REJECTIONS", 3u32)?,
            hook_servers: parse_hook_servers_env("MCP_HOOK_SERVERS"),
            hints_enabled: parse_env("BUZZ_AGENT_NO_HINTS", 0u8)? == 0,
            thinking_effort: parse_thinking_effort(env("BUZZ_AGENT_THINKING_EFFORT").as_deref())?,
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Construct a minimal `Config` for model-catalog discovery.
    ///
    /// Only the fields used by [`build_token_source`](crate::llm::build_token_source)
    /// and the catalog HTTP helpers are meaningful; all others are set to
    /// inert defaults. Never call `from_env` for discovery — it requires
    /// `DATABRICKS_MODEL` and other fields that are irrelevant here.
    pub fn for_discovery(provider: Provider, api_key: String, base_url: String) -> Self {
        Self {
            provider,
            api_key,
            base_url,
            model: String::new(),
            system_prompt: String::new(),
            anthropic_api_version: "2023-06-01".into(),
            openai_api: OpenAiApi::Chat,
            max_rounds: 0,
            max_output_tokens: 1,
            llm_timeout: Duration::from_secs(30),
            tool_timeout: Duration::from_secs(30),
            mcp_init_timeout: Duration::from_secs(30),
            mcp_max_restart_attempts: 0,
            mcp_restart_base_ms: 0,
            mcp_restart_max_ms: 0,
            max_sessions: 1,
            max_line_bytes: 4 * 1024 * 1024,
            max_history_bytes: 16 * 1024 * 1024,
            max_tool_result_text_bytes: 50 * 1024,
            max_context_tokens: 200_001,
            max_handoffs: 0,
            max_parallel_tools: 1,
            hook_timeout: Duration::from_secs(1),
            stop_max_rejections: 0,
            hook_servers: HookServers::None,
            hints_enabled: false,
            thinking_effort: None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        const MIN_HISTORY_BYTES: usize = 4096;
        const MIN_LINE_BYTES: usize = 1024;
        const MIN_TOOL_RESULT_TEXT_BYTES: usize = 1024;
        const MIN_TIMEOUT: Duration = Duration::from_secs(1);

        if self.max_output_tokens < 1 {
            return Err("config: BUZZ_AGENT_MAX_OUTPUT_TOKENS must be >= 1".into());
        }
        if self.max_context_tokens <= u64::from(self.max_output_tokens) {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_CONTEXT_TOKENS ({}) must be > BUZZ_AGENT_MAX_OUTPUT_TOKENS ({}) — the context window must leave room for the response",
                self.max_context_tokens, self.max_output_tokens
            ));
        }
        if self.max_history_bytes < MIN_HISTORY_BYTES {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_HISTORY_BYTES must be >= {MIN_HISTORY_BYTES}"
            ));
        }
        if self.max_history_bytes < MAX_PROMPT_BYTES {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_HISTORY_BYTES ({}) must be >= MAX_PROMPT_BYTES ({MAX_PROMPT_BYTES})",
                self.max_history_bytes
            ));
        }
        if self.max_line_bytes < MIN_LINE_BYTES {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_LINE_BYTES must be >= {MIN_LINE_BYTES}"
            ));
        }
        if self.max_tool_result_text_bytes < MIN_TOOL_RESULT_TEXT_BYTES
            || self.max_tool_result_text_bytes > MAX_TOOL_RESULT_BYTES
        {
            return Err(format!(
                "config: BUZZ_AGENT_MAX_TOOL_RESULT_TEXT_BYTES must be in {MIN_TOOL_RESULT_TEXT_BYTES}..={MAX_TOOL_RESULT_BYTES}"
            ));
        }
        if self.llm_timeout < MIN_TIMEOUT {
            return Err("config: BUZZ_AGENT_LLM_TIMEOUT_SECS must be >= 1".into());
        }
        if self.tool_timeout < MIN_TIMEOUT {
            return Err("config: BUZZ_AGENT_TOOL_TIMEOUT_SECS must be >= 1".into());
        }
        if self.mcp_init_timeout < MIN_TIMEOUT {
            return Err("config: BUZZ_AGENT_MCP_INIT_TIMEOUT_SECS must be >= 1".into());
        }
        if self.max_parallel_tools < 1 {
            return Err("config: BUZZ_AGENT_MAX_PARALLEL_TOOLS must be >= 1".into());
        }
        if self.mcp_max_restart_attempts < 1 {
            return Err("config: BUZZ_AGENT_MCP_RESTART_MAX_ATTEMPTS must be >= 1".into());
        }
        if self.mcp_restart_base_ms < 1 {
            return Err("config: BUZZ_AGENT_MCP_RESTART_BASE_MS must be >= 1".into());
        }
        if self.mcp_restart_max_ms < self.mcp_restart_base_ms {
            return Err(
                "config: BUZZ_AGENT_MCP_RESTART_MAX_MS must be >= BUZZ_AGENT_MCP_RESTART_BASE_MS"
                    .into(),
            );
        }
        // Provider-level effort validation (fail-fast, clear error).
        // `none`/`minimal` are not Anthropic values — rejected at startup.
        //
        // OpenAI, Databricks, and DatabricksV2 defer effort validation to request-time routing:
        // availability is model-dependent, and `session/set_model` can change the effective model
        // after startup. `normalize_effort_for_openai_route` / `normalize_effort_for_anthropic_route`
        // apply route-aware normalization in `llm.rs` when building each request.
        if let Some(effort) = self.thinking_effort {
            let is_pure_anthropic = matches!(self.provider, Provider::Anthropic);
            if is_pure_anthropic && matches!(effort, ThinkingEffort::None | ThinkingEffort::Minimal)
            {
                return Err(format!(
                    "config: BUZZ_AGENT_THINKING_EFFORT={} is not valid for Anthropic providers \
                     (allowed: low|medium|high|xhigh|max)",
                    effort.openai_effort_str()
                ));
            }
        }
        Ok(())
    }
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok()
}

fn env_or(k: &str, d: &str) -> String {
    env(k).unwrap_or_else(|| d.into())
}

fn req(k: &str) -> Result<String, String> {
    env(k).ok_or_else(|| format!("config: {k} required"))
}

/// Returns the first present value. `explicit_override` (BUZZ_AGENT_MODEL,
/// set by the desktop from the persona/record) wins over `provider_default`
/// (provider-specific env var that may be inherited from the shell).
/// Returns `None` when both are absent so the caller can supply a
/// provider-specific error message.
fn resolve_model(
    explicit_override: Option<&str>,
    provider_default: Option<&str>,
) -> Option<String> {
    explicit_override.or(provider_default).map(str::to_owned)
}

fn present_nonempty(v: Option<&str>) -> bool {
    v.map(str::trim).is_some_and(|s| !s.is_empty())
}

fn resolve_provider(
    requested: Option<&str>,
    anthropic_key: Option<&str>,
    openai_key: Option<&str>,
    gemini_key: Option<&str>,
) -> Result<Provider, String> {
    match requested.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => {
            let normalized = raw.to_ascii_lowercase();
            match normalized.as_str() {
                "anthropic" if present_nonempty(anthropic_key) => Ok(Provider::Anthropic),
                "anthropic" => Err(
                    "config: ANTHROPIC_API_KEY required".into(),
                ),
                "openai" | "openai-compat" if present_nonempty(openai_key) => Ok(Provider::OpenAi),
                "openai" | "openai-compat" => Err(
                    "config: OPENAI_COMPAT_API_KEY required".into(),
                ),
                "gemini" if present_nonempty(gemini_key) => Ok(Provider::Gemini),
                "gemini" => Err(
                    "config: GEMINI_API_KEY required".into(),
                ),
                "databricks" => Ok(Provider::Databricks),
                "databricks_v2" | "databricks-v2" => Ok(Provider::DatabricksV2),
                _ => Err(format!(
                    "config: BUZZ_AGENT_PROVIDER={raw} not supported"
                )),
            }
        }
        None => Err(
            "config: BUZZ_AGENT_PROVIDER is required — set it to your provider (e.g. anthropic, openai, gemini, databricks)".into(),
        ),
    }
}

/// Parse `OPENAI_COMPAT_API`. Pure (env-free) for testability; the
/// caller hands in the raw value.
fn parse_openai_api(raw: Option<&str>) -> Result<OpenAiApi, String> {
    match raw.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "chat" | "chat-completions" | "chat_completions" => Ok(OpenAiApi::Chat),
        "responses" => Ok(OpenAiApi::Responses),
        "auto" | "" => Ok(OpenAiApi::Auto),
        other => Err(format!(
            "config: OPENAI_COMPAT_API={other} not supported (use auto|chat|responses)"
        )),
    }
}

/// `true` when `base_url` is an official OpenAI host. Hosts on
/// `*.openai.com` get Responses under `Auto`; everything else (vLLM,
/// Ollama, OpenRouter, Block Gateway, …) gets Chat Completions.
/// Lookalike-safe: `api.openai.com.evil.example` returns `false`.
pub fn is_openai_host(base_url: &str) -> bool {
    let rest = match base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))
    {
        Some(r) => r,
        None => return false,
    };
    let host = &rest[..rest.find(['/', ':']).unwrap_or(rest.len())];
    host == "api.openai.com" || host.ends_with(".openai.com")
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> Result<T, String>
where
    T::Err: std::fmt::Display,
{
    env(key)
        .map(|v| v.parse().map_err(|e| format!("config: {key}: {e}")))
        .unwrap_or(Ok(default))
}

/// Hook-server allowlist parsed from a comma-separated env var.
///   - unset / empty / whitespace-only → `None` (no hooks enabled)
///   - `*`                              → `All` (every server eligible)
///   - `a,b,c`                          → `Only(["a","b","c"])`
#[derive(Debug, Clone)]
pub enum HookServers {
    None,
    All,
    Only(Vec<String>),
}

impl HookServers {
    /// Returns true iff `name` may receive hook calls.
    pub fn allows(&self, name: &str) -> bool {
        match self {
            HookServers::None => false,
            HookServers::All => true,
            HookServers::Only(v) => v.iter().any(|s| s == name),
        }
    }

    /// True if no hooks should ever fire — used to short-circuit dispatch.
    pub fn is_disabled(&self) -> bool {
        matches!(self, HookServers::None)
    }
}

fn parse_hook_servers_env(key: &str) -> HookServers {
    parse_hook_servers(env(key).as_deref())
}

/// Pure parser exposed for unit tests. `None` (env unset) and `Some("")`
/// (env set but empty) both yield `HookServers::None`.
fn parse_hook_servers(raw: Option<&str>) -> HookServers {
    let raw = match raw {
        Some(v) => v,
        None => return HookServers::None,
    };
    let names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        return HookServers::None;
    }
    // `*` is the wildcard — only honored when it's the sole entry. A mixed
    // value like "*,foo" falls through to `Only(["*","foo"])`; "*" is not a
    // legal MCP server name (it can't pass `valid_name`), so it never matches
    // an actual server. This avoids silently widening scope on typos.
    if names.len() == 1 && names[0] == "*" {
        return HookServers::All;
    }
    HookServers::Only(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_servers_unset_is_none() {
        assert!(matches!(parse_hook_servers(None), HookServers::None));
    }

    #[test]
    fn hook_servers_empty_string_is_none() {
        assert!(matches!(parse_hook_servers(Some("")), HookServers::None));
    }

    #[test]
    fn hook_servers_whitespace_only_is_none() {
        assert!(matches!(
            parse_hook_servers(Some("   ,, ,")),
            HookServers::None
        ));
    }

    #[test]
    fn hook_servers_star_is_all() {
        assert!(matches!(parse_hook_servers(Some("*")), HookServers::All));
    }

    #[test]
    fn hook_servers_star_with_whitespace_is_all() {
        assert!(matches!(
            parse_hook_servers(Some("  *  ")),
            HookServers::All
        ));
    }

    #[test]
    fn hook_servers_named_list() {
        match parse_hook_servers(Some("foo,bar")) {
            HookServers::Only(v) => assert_eq!(v, vec!["foo".to_owned(), "bar".to_owned()]),
            other => panic!("expected Only, got {other:?}"),
        }
    }

    #[test]
    fn hook_servers_trims_entries() {
        match parse_hook_servers(Some(" foo , bar , ")) {
            HookServers::Only(v) => assert_eq!(v, vec!["foo".to_owned(), "bar".to_owned()]),
            other => panic!("expected Only, got {other:?}"),
        }
    }

    #[test]
    fn hook_servers_star_mixed_is_literal() {
        // `*,foo` is NOT a wildcard — it's a literal Only(["*","foo"]).
        // No real server can be named `*`, so this never matches anything.
        match parse_hook_servers(Some("*,foo")) {
            HookServers::Only(v) => assert_eq!(v, vec!["*".to_owned(), "foo".to_owned()]),
            other => panic!("expected Only, got {other:?}"),
        }
    }

    #[test]
    fn hook_servers_allows_matches_named_only() {
        let hs = parse_hook_servers(Some("foo,bar"));
        assert!(hs.allows("foo"));
        assert!(hs.allows("bar"));
        assert!(!hs.allows("baz"));
    }

    #[test]
    fn hook_servers_allows_matches_all() {
        assert!(parse_hook_servers(Some("*")).allows("anything"));
    }

    #[test]
    fn hook_servers_allows_blocks_when_none() {
        assert!(!parse_hook_servers(None).allows("foo"));
    }

    #[test]
    fn hook_servers_star_mixed_does_not_match_real_server() {
        let hs = parse_hook_servers(Some("*,foo"));
        // The literal "*" entry exists in Only, but no real server can
        // be named "*" (rejected by the MCP server name validator).
        assert!(hs.allows("foo"));
        assert!(!hs.allows("bar"));
        // Allowed strictly only as a literal match — defense-in-depth
        // expectation for callers.
        assert!(hs.allows("*"));
    }

    #[test]
    fn parse_openai_api_values() {
        use OpenAiApi::*;
        for (raw, want) in [
            (None, Ok(Auto)),
            (Some("auto"), Ok(Auto)),
            (Some("  AUTO  "), Ok(Auto)),
            (Some(""), Ok(Auto)),
            (Some("chat"), Ok(Chat)),
            (Some("chat-completions"), Ok(Chat)),
            (Some("Responses"), Ok(Responses)),
        ] {
            assert_eq!(parse_openai_api(raw), want, "raw={raw:?}");
        }
        let err = parse_openai_api(Some("nope")).unwrap_err();
        assert!(err.contains("OPENAI_COMPAT_API=nope"), "{err}");
    }

    #[test]
    fn resolve_provider_keeps_requested_provider_when_token_present() {
        assert_eq!(
            resolve_provider(Some("anthropic"), Some("sk-ant"), None, None).unwrap(),
            Provider::Anthropic
        );
        assert_eq!(
            resolve_provider(Some("openai"), None, Some("sk-openai"), None).unwrap(),
            Provider::OpenAi
        );
        assert_eq!(
            resolve_provider(Some("gemini"), None, None, Some("AIza-key")).unwrap(),
            Provider::Gemini
        );
        // Case-insensitive, like the other providers.
        assert_eq!(
            resolve_provider(Some("Gemini"), None, None, Some("AIza-key")).unwrap(),
            Provider::Gemini
        );
    }

    #[test]
    fn resolve_provider_errors_when_requested_provider_key_missing() {
        // No fallback — missing key returns an error regardless of Databricks availability.
        let err = resolve_provider(Some("anthropic"), None, None, None).unwrap_err();
        assert!(err.contains("ANTHROPIC_API_KEY required"), "{err}");

        let err = resolve_provider(Some("openai-compat"), None, Some("   "), None).unwrap_err();
        assert!(err.contains("OPENAI_COMPAT_API_KEY required"), "{err}");

        // Whitespace-only Gemini key is treated as absent.
        let err = resolve_provider(Some("gemini"), None, None, Some("   ")).unwrap_err();
        assert!(err.contains("GEMINI_API_KEY required"), "{err}");
    }

    #[test]
    fn resolve_provider_errors_when_provider_env_absent() {
        // No implicit inference — absent BUZZ_AGENT_PROVIDER is an error.
        let err = resolve_provider(None, None, None, None).unwrap_err();
        assert!(err.contains("BUZZ_AGENT_PROVIDER is required"), "{err}");
    }

    #[test]
    fn resolve_provider_requires_databricks_host_and_model_for_fallback() {
        // Renamed: verify the explicit databricks provider path works correctly.
        // When BUZZ_AGENT_PROVIDER=databricks, resolve_provider succeeds regardless
        // of DATABRICKS_HOST/MODEL (those are validated later in from_env()).
        assert_eq!(
            resolve_provider(Some("databricks"), None, None, None).unwrap(),
            Provider::Databricks
        );
        // Missing key for other providers still errors — no Databricks fallback.
        let err = resolve_provider(Some("openai"), None, None, None).unwrap_err();
        assert!(err.contains("OPENAI_COMPAT_API_KEY required"), "{err}");
        let err = resolve_provider(None, None, None, None).unwrap_err();
        assert!(err.contains("BUZZ_AGENT_PROVIDER is required"), "{err}");
    }

    #[test]
    fn resolve_provider_unsupported_error_preserves_user_casing() {
        let err = resolve_provider(Some("OpenAIish"), None, None, None).unwrap_err();
        assert!(err.contains("BUZZ_AGENT_PROVIDER=OpenAIish"));
    }

    #[test]
    fn is_openai_host_matrix() {
        // Lookalike-safe: `api.openai.com.evil.example` and malformed URLs
        // are treated as non-OpenAI (which falls back to Chat Completions).
        for (url, want) in [
            ("https://api.openai.com/v1", true),
            ("https://api.openai.com", true),
            ("http://eu.api.openai.com/v1", true),
            ("http://localhost:11434/v1", false),
            ("https://openrouter.ai/api/v1", false),
            ("https://gateway.block.example/v1", false),
            ("https://api.openai.com.evil.example/v1", false),
            ("not a url", false),
        ] {
            assert_eq!(is_openai_host(url), want, "url={url}");
        }
    }

    #[test]
    fn resolve_model_prefers_explicit_override() {
        let result = resolve_model(Some("override-model"), Some("provider-model"));
        assert_eq!(result.as_deref(), Some("override-model"));
    }

    #[test]
    fn resolve_model_falls_back_to_provider_default() {
        let result = resolve_model(None, Some("provider-model"));
        assert_eq!(result.as_deref(), Some("provider-model"));
    }

    #[test]
    fn resolve_model_returns_none_when_both_absent() {
        let result = resolve_model(None, None);
        assert!(result.is_none());
    }

    #[test]
    fn gemini_default_base_url_is_openai_compat_surface() {
        // The default base must point at Google's OpenAI-compatible surface
        // (v1beta/openai), not the native generateContent API. from_env() feeds
        // this const to env_or("GEMINI_BASE_URL", …).
        assert_eq!(
            GEMINI_DEFAULT_BASE_URL,
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
        // Sanity: the compat surface must not be treated as a first-party OpenAI
        // host, or Auto-mode dispatch would try to upgrade it to /responses.
        assert!(!is_openai_host(GEMINI_DEFAULT_BASE_URL));
    }

    #[test]
    fn gemini_model_resolution_prefers_buzz_agent_model_override() {
        // from_env() resolves Gemini's model as
        // resolve_model(BUZZ_AGENT_MODEL, GEMINI_MODEL): the universal override wins.
        assert_eq!(
            resolve_model(Some("gemini-2.5-pro"), Some("gemini-1.5-flash")).as_deref(),
            Some("gemini-2.5-pro"),
            "BUZZ_AGENT_MODEL must override GEMINI_MODEL"
        );
        // Falls back to GEMINI_MODEL when no universal override is set.
        assert_eq!(
            resolve_model(None, Some("gemini-1.5-flash")).as_deref(),
            Some("gemini-1.5-flash"),
            "GEMINI_MODEL must be used when BUZZ_AGENT_MODEL is unset"
        );
    }

    #[test]
    fn parse_thinking_effort_round_trips_all_values() {
        for (raw, expected) in [
            ("none", ThinkingEffort::None),
            ("minimal", ThinkingEffort::Minimal),
            ("low", ThinkingEffort::Low),
            ("medium", ThinkingEffort::Medium),
            ("high", ThinkingEffort::High),
            ("xhigh", ThinkingEffort::XHigh),
            ("max", ThinkingEffort::Max),
        ] {
            assert_eq!(
                parse_thinking_effort(Some(raw)).unwrap(),
                Some(expected),
                "raw={raw:?}"
            );
        }
    }

    #[test]
    fn parse_thinking_effort_none_and_empty_yield_none() {
        assert_eq!(parse_thinking_effort(None).unwrap(), None);
        assert_eq!(parse_thinking_effort(Some("")).unwrap(), None);
        assert_eq!(parse_thinking_effort(Some("   ")).unwrap(), None);
    }

    #[test]
    fn parse_thinking_effort_is_case_insensitive() {
        assert_eq!(
            parse_thinking_effort(Some("HIGH")).unwrap(),
            Some(ThinkingEffort::High)
        );
        assert_eq!(
            parse_thinking_effort(Some("  Medium  ")).unwrap(),
            Some(ThinkingEffort::Medium)
        );
    }

    #[test]
    fn parse_thinking_effort_rejects_unknown_value() {
        let err = parse_thinking_effort(Some("extreme")).unwrap_err();
        assert!(err.contains("BUZZ_AGENT_THINKING_EFFORT=extreme"), "{err}");
        assert!(
            err.contains("none|minimal|low|medium|high|xhigh|max"),
            "{err}"
        );
    }

    #[test]
    fn thinking_effort_anthropic_budget_tokens_mapping() {
        assert_eq!(ThinkingEffort::Low.anthropic_budget_tokens(), 1_024);
        assert_eq!(ThinkingEffort::Medium.anthropic_budget_tokens(), 8_192);
        assert_eq!(ThinkingEffort::High.anthropic_budget_tokens(), 32_768);
        // XHigh and Max clamp to the high budget value for manual-budget models.
        assert_eq!(ThinkingEffort::XHigh.anthropic_budget_tokens(), 32_768);
        assert_eq!(ThinkingEffort::Max.anthropic_budget_tokens(), 32_768);
        // None/Minimal are rejected at startup for Anthropic; defensive zero.
        assert_eq!(ThinkingEffort::None.anthropic_budget_tokens(), 0);
        assert_eq!(ThinkingEffort::Minimal.anthropic_budget_tokens(), 0);
    }

    #[test]
    fn thinking_effort_openai_effort_str_mapping() {
        assert_eq!(ThinkingEffort::None.openai_effort_str(), "none");
        assert_eq!(ThinkingEffort::Minimal.openai_effort_str(), "minimal");
        assert_eq!(ThinkingEffort::Low.openai_effort_str(), "low");
        assert_eq!(ThinkingEffort::Medium.openai_effort_str(), "medium");
        assert_eq!(ThinkingEffort::High.openai_effort_str(), "high");
        assert_eq!(ThinkingEffort::XHigh.openai_effort_str(), "xhigh");
        assert_eq!(ThinkingEffort::Max.openai_effort_str(), "max");
    }

    #[test]
    fn thinking_effort_anthropic_effort_str_mapping() {
        assert_eq!(ThinkingEffort::Low.anthropic_effort_str(), "low");
        assert_eq!(ThinkingEffort::Medium.anthropic_effort_str(), "medium");
        assert_eq!(ThinkingEffort::High.anthropic_effort_str(), "high");
        assert_eq!(ThinkingEffort::XHigh.anthropic_effort_str(), "xhigh");
        assert_eq!(ThinkingEffort::Max.anthropic_effort_str(), "max");
        // Defensive fallback for invalid Anthropic values (caught at startup validation).
        assert_eq!(ThinkingEffort::None.anthropic_effort_str(), "low");
        assert_eq!(ThinkingEffort::Minimal.anthropic_effort_str(), "low");
    }

    #[test]
    fn thinking_effort_ord_ordering() {
        // PartialOrd/Ord must reflect the ordered hierarchy.
        assert!(ThinkingEffort::None < ThinkingEffort::Minimal);
        assert!(ThinkingEffort::Minimal < ThinkingEffort::Low);
        assert!(ThinkingEffort::Low < ThinkingEffort::Medium);
        assert!(ThinkingEffort::Medium < ThinkingEffort::High);
        assert!(ThinkingEffort::High < ThinkingEffort::XHigh);
        assert!(ThinkingEffort::XHigh < ThinkingEffort::Max);
    }

    // ---- anthropic_thinking_config helper — per-family tests ----

    #[test]
    fn anthropic_thinking_config_claude3_emits_budget_tokens() {
        // Claude 3.x → `thinking.budget_tokens`; clamped to min(level_budget, max_output - 1024).
        // max_output_tokens = 4096: headroom = 4096 - 1024 = 3072; High budget (32768) → 3072.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-3-7-sonnet-20250219", ThinkingEffort::High, 4096);
        let t = thinking.expect("thinking field must be present for claude-3");
        assert_eq!(t["type"], "enabled");
        assert_eq!(t["budget_tokens"], 3072); // capped: min(32768, 4096-1024)
        assert!(
            output_config.is_none(),
            "output_config must be absent for claude-3"
        );
    }

    #[test]
    fn anthropic_thinking_config_claude3_omits_thinking_when_max_output_too_small() {
        // max_output_tokens = 2047: headroom = 2047 - 1024 = 1023 < 1024 → omit thinking.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-3-7-sonnet-20250219", ThinkingEffort::High, 2047);
        assert!(
            thinking.is_none(),
            "thinking must be omitted when max_output_tokens - 1024 < 1024 (budget would starve answer)"
        );
        assert!(output_config.is_none());
    }

    #[test]
    fn anthropic_thinking_config_claude3_emits_thinking_at_boundary_2048() {
        // max_output_tokens = 2048: headroom = 2048 - 1024 = 1024 ≥ 1024 → emit budget = 1024.
        let (thinking, _) =
            anthropic_thinking_config("claude-3-7-sonnet-20250219", ThinkingEffort::High, 2048);
        let t = thinking.expect("thinking must be present when max_output_tokens = 2048");
        assert_eq!(t["budget_tokens"], 1024); // min(32768, 2048-1024) = 1024
    }

    #[test]
    fn anthropic_thinking_config_claude3_budget_uncapped_when_fits() {
        // High budget fits comfortably under a large max_output_tokens.
        let (thinking, _) =
            anthropic_thinking_config("claude-3-7-sonnet-20250219", ThinkingEffort::High, 65_536);
        let t = thinking.unwrap();
        assert_eq!(t["budget_tokens"], 32_768);
    }

    #[test]
    fn anthropic_thinking_config_opus_4_8_emits_adaptive_and_effort() {
        // Opus 4.8 — adaptive family. Requires thinking:{type:"adaptive"} to enable thinking.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-opus-4-8", ThinkingEffort::High, 32_768);
        let t = thinking.expect("thinking must be present for claude-opus-4-8");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-opus-4-8");
        assert_eq!(oc["effort"], "high");
    }

    #[test]
    fn anthropic_thinking_config_opus_4_7_emits_adaptive_and_effort() {
        // Opus 4.7 — adaptive family.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-opus-4-7", ThinkingEffort::Medium, 32_768);
        let t = thinking.expect("thinking must be present for claude-opus-4-7");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-opus-4-7");
        assert_eq!(oc["effort"], "medium");
    }

    #[test]
    fn anthropic_thinking_config_sonnet_5_emits_adaptive_and_effort() {
        // Sonnet 5 — adaptive family.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-sonnet-5-20250901", ThinkingEffort::Low, 32_768);
        let t = thinking.expect("thinking must be present for claude-sonnet-5");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-sonnet-5");
        assert_eq!(oc["effort"], "low");
    }

    #[test]
    fn anthropic_thinking_config_sonnet_4_6_emits_adaptive_and_effort() {
        // Sonnet 4.6 — adaptive family. Docs explicitly list "Combine effort with adaptive thinking."
        let (thinking, output_config) =
            anthropic_thinking_config("claude-sonnet-4-6", ThinkingEffort::High, 32_768);
        let t = thinking.expect("thinking must be present for claude-sonnet-4-6");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-sonnet-4-6");
        assert_eq!(oc["effort"], "high");
    }

    #[test]
    fn anthropic_thinking_config_opus_4_5_emits_manual_budget() {
        // Opus 4.5 — manual budget (NOT adaptive; effort page: "uses manual thinking").
        let (thinking, output_config) =
            anthropic_thinking_config("claude-opus-4-5", ThinkingEffort::High, 65_536);
        let t = thinking.expect("thinking must be present for claude-opus-4-5");
        assert_eq!(t["type"], "enabled");
        assert_eq!(t["budget_tokens"], 32_768); // High budget fits under 65536
        assert!(
            output_config.is_none(),
            "output_config must be absent for claude-opus-4-5 (manual budget)"
        );
    }

    #[test]
    fn anthropic_thinking_config_opus_4_5_budget_capped() {
        // Opus 4.5 manual budget is clamped to min(level_budget, max_output_tokens - 1024).
        // max_output_tokens = 4096: headroom = 4096 - 1024 = 3072; High budget (32768) → 3072.
        let (thinking, _) =
            anthropic_thinking_config("claude-opus-4-5", ThinkingEffort::High, 4096);
        let t = thinking.unwrap();
        assert_eq!(t["budget_tokens"], 3072); // min(32768, 4096-1024)
    }

    #[test]
    fn anthropic_thinking_config_opus_4_5_omits_thinking_when_max_output_1025() {
        // max_output_tokens = 1025: headroom = 1025 - 1024 = 1 < 1024 → omit thinking.
        let (thinking, _) =
            anthropic_thinking_config("claude-opus-4-5", ThinkingEffort::High, 1025);
        assert!(
            thinking.is_none(),
            "thinking must be omitted when max_output_tokens - 1024 < 1024"
        );
    }

    #[test]
    fn anthropic_thinking_config_manual_budget_low_emits_1024_when_fits() {
        // Low budget (1024 tokens) exactly fits when max_output_tokens = 2048.
        // headroom = 2048 - 1024 = 1024; min(1024, 1024) = 1024 ≥ 1024 → emit.
        let (thinking, _) =
            anthropic_thinking_config("claude-3-7-sonnet-20250219", ThinkingEffort::Low, 2048);
        let t = thinking.expect("Low budget (1024) must be emitted when max_output_tokens = 2048");
        assert_eq!(t["budget_tokens"], 1024);
    }

    #[test]
    fn anthropic_thinking_config_unknown_claude_omits_both_fields() {
        // An unknown/future "claude-*" name that is not in the allowlist → omit both fields.
        // This prevents sending an unverified shape to an unrecognized model.
        // Includes Opus 4.9 (future version), which is NOT in the doc-verified adaptive list.
        for model in &[
            "claude-haiku-4-5",
            "claude-sonnet-4-5",
            "claude-unknown-9-1",
            "claude-future-model",
            "claude-opus-4-9",
        ] {
            let (thinking, output_config) =
                anthropic_thinking_config(model, ThinkingEffort::High, 32_768);
            assert!(
                thinking.is_none(),
                "thinking must be absent for unverified claude model: {model}"
            );
            assert!(
                output_config.is_none(),
                "output_config must be absent for unverified claude model: {model}"
            );
        }
    }

    #[test]
    fn anthropic_thinking_config_non_claude_omits_both_fields() {
        // Non-Anthropic model names (gpt-5, llama, etc.) → omit both fields.
        let (thinking, output_config) =
            anthropic_thinking_config("gpt-4o-mini", ThinkingEffort::High, 32_768);
        assert!(
            thinking.is_none(),
            "thinking must be absent for non-claude model"
        );
        assert!(
            output_config.is_none(),
            "output_config must be absent for non-claude model"
        );
    }

    #[test]
    fn anthropic_thinking_config_databricks_prefix_stripped_for_claude3() {
        // Databricks gateway prefixes like "databricks-claude-3-..." must be stripped.
        let (thinking, output_config) =
            anthropic_thinking_config("databricks-claude-3-5-sonnet", ThinkingEffort::Low, 8_192);
        let t = thinking.expect("thinking must be present after stripping databricks- prefix");
        assert_eq!(t["type"], "enabled");
        assert!(output_config.is_none());
    }

    #[test]
    fn anthropic_thinking_config_databricks_prefix_stripped_for_opus_4_7() {
        // Databricks gateway prefix stripping applies to adaptive Claude families too.
        let (thinking, output_config) =
            anthropic_thinking_config("databricks-claude-opus-4-7", ThinkingEffort::High, 32_768);
        let t = thinking
            .expect("thinking:{type:adaptive} must be present for databricks-claude-opus-4-7");
        assert_eq!(t["type"], "adaptive");
        let oc =
            output_config.expect("output_config must be present for databricks-claude-opus-4-7");
        assert_eq!(oc["effort"], "high");
    }

    #[test]
    fn anthropic_thinking_config_databricks_prefix_stripped_for_opus_4_8() {
        // Databricks gateway prefix stripping applies to Opus 4.8 too.
        let (thinking, output_config) =
            anthropic_thinking_config("databricks-claude-opus-4-8", ThinkingEffort::Medium, 32_768);
        let t = thinking
            .expect("thinking:{type:adaptive} must be present for databricks-claude-opus-4-8");
        assert_eq!(t["type"], "adaptive");
        let oc =
            output_config.expect("output_config must be present for databricks-claude-opus-4-8");
        assert_eq!(oc["effort"], "medium");
    }

    #[test]
    fn anthropic_thinking_config_goose_prefix_stripped_for_fable_5() {
        // "goose-" catalog prefix must be stripped so goose-claude-fable-5 routes to
        // the adaptive + xhigh/max bucket, not the "unknown model → (None, None)" path.
        let (thinking, output_config) =
            anthropic_thinking_config("goose-claude-fable-5", ThinkingEffort::Max, 32_768);
        let t =
            thinking.expect("thinking:{type:adaptive} must be present for goose-claude-fable-5");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for goose-claude-fable-5");
        assert_eq!(oc["effort"], "max");
    }

    #[test]
    fn anthropic_thinking_config_goose_prefix_stripped_for_sonnet_5() {
        // Adaptive xhigh model via goose- prefix.
        let (thinking, output_config) =
            anthropic_thinking_config("goose-claude-sonnet-5", ThinkingEffort::XHigh, 32_768);
        let t =
            thinking.expect("thinking:{type:adaptive} must be present for goose-claude-sonnet-5");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for goose-claude-sonnet-5");
        assert_eq!(oc["effort"], "xhigh");
    }

    #[test]
    fn anthropic_thinking_config_arbitrary_prefix_stripped_for_opus_4_7() {
        // team-x-claude-opus-4-7: first claude- token at index 7 → strips "team-x-"
        // Verifies the arbitrary-prefix normalization reaches anthropic_thinking_config
        // end-to-end: UI exposes max as valid, and runtime must honor it.
        let (thinking, output_config) =
            anthropic_thinking_config("team-x-claude-opus-4-7", ThinkingEffort::Max, 32_768);
        let t =
            thinking.expect("thinking:{type:adaptive} must be present for team-x-claude-opus-4-7");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for team-x-claude-opus-4-7");
        assert_eq!(oc["effort"], "max");
    }

    // ---- clamp_adaptive_effort — per-model clamping tests ----

    #[test]
    fn clamp_adaptive_effort_xhigh_passes_through_for_opus_4_7() {
        // Opus 4.7 supports xhigh — no clamping.
        assert_eq!(
            clamp_adaptive_effort("claude-opus-4-7", ThinkingEffort::XHigh),
            ThinkingEffort::XHigh
        );
    }

    #[test]
    fn clamp_adaptive_effort_xhigh_passes_through_for_opus_4_8() {
        // Opus 4.8 supports xhigh — no clamping.
        assert_eq!(
            clamp_adaptive_effort("claude-opus-4-8", ThinkingEffort::XHigh),
            ThinkingEffort::XHigh
        );
    }

    #[test]
    fn clamp_adaptive_effort_xhigh_passes_through_for_sonnet_5() {
        // Sonnet 5 supports xhigh — no clamping.
        assert_eq!(
            clamp_adaptive_effort("claude-sonnet-5-20250901", ThinkingEffort::XHigh),
            ThinkingEffort::XHigh
        );
    }

    #[test]
    fn clamp_adaptive_effort_xhigh_clamped_to_high_for_opus_4_6() {
        // Opus 4.6 does NOT support xhigh (only low/medium/high/max) — clamp to high.
        assert_eq!(
            clamp_adaptive_effort("claude-opus-4-6", ThinkingEffort::XHigh),
            ThinkingEffort::High
        );
    }

    #[test]
    fn clamp_adaptive_effort_xhigh_clamped_to_high_for_sonnet_4_6() {
        // Sonnet 4.6 does NOT support xhigh — clamp to high.
        assert_eq!(
            clamp_adaptive_effort("claude-sonnet-4-6", ThinkingEffort::XHigh),
            ThinkingEffort::High
        );
    }

    #[test]
    fn clamp_adaptive_effort_max_passes_through_for_opus_4_6() {
        // Opus 4.6 supports max — no clamping.
        assert_eq!(
            clamp_adaptive_effort("claude-opus-4-6", ThinkingEffort::Max),
            ThinkingEffort::Max
        );
    }

    #[test]
    fn clamp_adaptive_effort_max_passes_through_for_opus_4_7() {
        // Opus 4.7 supports max — no clamping.
        assert_eq!(
            clamp_adaptive_effort("claude-opus-4-7", ThinkingEffort::Max),
            ThinkingEffort::Max
        );
    }

    #[test]
    fn clamp_adaptive_effort_max_passes_through_for_opus_4_8() {
        // Opus 4.8 supports max — no clamping.
        assert_eq!(
            clamp_adaptive_effort("claude-opus-4-8", ThinkingEffort::Max),
            ThinkingEffort::Max
        );
    }

    #[test]
    fn clamp_adaptive_effort_low_medium_high_never_clamped() {
        // low/medium/high pass through for all adaptive models.
        for model in &[
            "claude-opus-4-6",
            "claude-opus-4-7",
            "claude-opus-4-8",
            "claude-sonnet-5-20250901",
            "claude-sonnet-4-6",
        ] {
            for effort in [
                ThinkingEffort::Low,
                ThinkingEffort::Medium,
                ThinkingEffort::High,
            ] {
                assert_eq!(
                    clamp_adaptive_effort(model, effort),
                    effort,
                    "model={model} effort={effort:?}"
                );
            }
        }
    }

    // ---- anthropic_thinking_config — xhigh/max body-shape assertions ----

    #[test]
    fn anthropic_thinking_config_opus_4_8_xhigh_emits_xhigh_effort() {
        // Opus 4.8 supports xhigh; output_config.effort must be "xhigh".
        let (thinking, output_config) =
            anthropic_thinking_config("claude-opus-4-8", ThinkingEffort::XHigh, 32_768);
        let t = thinking.expect("thinking must be present for claude-opus-4-8");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-opus-4-8");
        assert_eq!(oc["effort"], "xhigh");
    }

    #[test]
    fn anthropic_thinking_config_opus_4_8_max_emits_max_effort() {
        // Opus 4.8 supports max; output_config.effort must be "max".
        let (thinking, output_config) =
            anthropic_thinking_config("claude-opus-4-8", ThinkingEffort::Max, 32_768);
        let t = thinking.expect("thinking must be present for claude-opus-4-8");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-opus-4-8");
        assert_eq!(oc["effort"], "max");
    }

    #[test]
    fn anthropic_thinking_config_opus_4_7_xhigh_emits_xhigh_effort() {
        // Opus 4.7 supports xhigh.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-opus-4-7", ThinkingEffort::XHigh, 32_768);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.unwrap();
        assert_eq!(oc["effort"], "xhigh");
    }

    #[test]
    fn anthropic_thinking_config_opus_4_6_xhigh_clamps_to_high() {
        // Opus 4.6 does NOT support xhigh → clamp to high.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-opus-4-6", ThinkingEffort::XHigh, 32_768);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.unwrap();
        assert_eq!(
            oc["effort"], "high",
            "xhigh must clamp to high for claude-opus-4-6"
        );
    }

    #[test]
    fn anthropic_thinking_config_opus_4_6_max_passes_through() {
        // Opus 4.6 supports max — passes through without clamping.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-opus-4-6", ThinkingEffort::Max, 32_768);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.unwrap();
        assert_eq!(oc["effort"], "max");
    }

    #[test]
    fn anthropic_thinking_config_manual_bucket_xhigh_clamps_to_high_budget() {
        // Manual-budget models (claude-3*, opus-4-5): xhigh clamps to high budget (32_768).
        for model in &["claude-3-7-sonnet-20250219", "claude-opus-4-5"] {
            let (thinking, output_config) =
                anthropic_thinking_config(model, ThinkingEffort::XHigh, 65_536);
            let t = thinking.expect("thinking must be present");
            assert_eq!(t["type"], "enabled");
            assert_eq!(
                t["budget_tokens"], 32_768,
                "xhigh must clamp to high budget for manual model {model}"
            );
            assert!(output_config.is_none());
        }
    }

    #[test]
    fn anthropic_thinking_config_manual_bucket_max_clamps_to_high_budget() {
        // Manual-budget models: max also clamps to high budget (32_768).
        let (thinking, _) =
            anthropic_thinking_config("claude-opus-4-5", ThinkingEffort::Max, 65_536);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "enabled");
        assert_eq!(t["budget_tokens"], 32_768);
    }

    // ---- provider-level validation tests ----

    /// Build a minimal Config with the given provider and thinking_effort, bypassing from_env().
    /// Uses `Config::for_discovery` as a base and patches the fields we care about.
    fn make_config_for_validation(
        provider: Provider,
        thinking_effort: Option<ThinkingEffort>,
    ) -> Config {
        let mut cfg = Config::for_discovery(provider, "key".into(), "https://example.com".into());
        cfg.model = "some-model".into();
        cfg.thinking_effort = thinking_effort;
        // for_discovery sets max_output_tokens=1 and max_context_tokens=200_001 which satisfies
        // the context > output constraint. Adjust to something valid for further checks.
        cfg.max_output_tokens = 1024;
        cfg.max_context_tokens = 200_000 + 1024;
        // Restore mandatory positive values that for_discovery zeroes out.
        cfg.mcp_max_restart_attempts = 1;
        cfg.mcp_restart_base_ms = 1;
        cfg.mcp_restart_max_ms = 1;
        cfg.max_parallel_tools = 1;
        cfg.llm_timeout = Duration::from_secs(1);
        cfg.tool_timeout = Duration::from_secs(1);
        cfg.mcp_init_timeout = Duration::from_secs(1);
        cfg
    }

    #[test]
    fn validate_rejects_none_effort_for_anthropic() {
        let cfg = make_config_for_validation(Provider::Anthropic, Some(ThinkingEffort::None));
        let err = cfg.validate().unwrap_err();
        assert!(
            err.contains("BUZZ_AGENT_THINKING_EFFORT=none"),
            "error must name the value: {err}"
        );
        assert!(
            err.contains("not valid for Anthropic"),
            "error must name the provider: {err}"
        );
        assert!(
            err.contains("low|medium|high|xhigh|max"),
            "error must name allowed values: {err}"
        );
    }

    #[test]
    fn validate_rejects_minimal_effort_for_anthropic() {
        let cfg = make_config_for_validation(Provider::Anthropic, Some(ThinkingEffort::Minimal));
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("BUZZ_AGENT_THINKING_EFFORT=minimal"), "{err}");
        assert!(err.contains("not valid for Anthropic"), "{err}");
    }

    #[test]
    fn validate_accepts_all_efforts_for_databricks_v2() {
        // DatabricksV2 dispatches across Anthropic/OpenAI/MLflow routes at request build time.
        // No effort value is invalid for all three routes — startup rejects none.
        for effort in [
            ThinkingEffort::None,
            ThinkingEffort::Minimal,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ] {
            let cfg = make_config_for_validation(Provider::DatabricksV2, Some(effort));
            assert!(
                cfg.validate().is_ok(),
                "DatabricksV2 must accept {effort:?} at startup (route-aware normalization at request build)"
            );
        }
    }

    #[test]
    fn validate_accepts_all_efforts_for_openai() {
        // OpenAI effort support is model-dependent and normalized at request build time.
        for effort in [
            ThinkingEffort::None,
            ThinkingEffort::Minimal,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ] {
            let cfg = make_config_for_validation(Provider::OpenAi, Some(effort));
            assert!(
                cfg.validate().is_ok(),
                "OpenAI must accept {effort:?} at startup (route-aware normalization at request build)"
            );
        }
    }

    #[test]
    fn validate_accepts_all_efforts_for_gemini() {
        // Gemini rides the OpenAI-shaped Chat path, so effort validation must
        // behave exactly like OpenAI: none/minimal are NOT rejected at startup
        // (only pure Anthropic rejects them); availability is normalized at
        // request-build time via normalize_effort_for_openai_route.
        for effort in [
            ThinkingEffort::None,
            ThinkingEffort::Minimal,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ] {
            let cfg = make_config_for_validation(Provider::Gemini, Some(effort));
            assert!(
                cfg.validate().is_ok(),
                "Gemini must accept {effort:?} at startup, consistent with the OpenAI Chat path"
            );
        }
    }

    #[test]
    fn validate_accepts_all_efforts_for_databricks() {
        // Legacy Databricks effort support is model-dependent and normalized at request build time.
        for effort in [
            ThinkingEffort::None,
            ThinkingEffort::Minimal,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ] {
            let cfg = make_config_for_validation(Provider::Databricks, Some(effort));
            assert!(
                cfg.validate().is_ok(),
                "Databricks must accept {effort:?} at startup (route-aware normalization at request build)"
            );
        }
    }

    #[test]
    fn validate_accepts_xhigh_for_anthropic() {
        // xhigh is valid for Anthropic providers — model-level clamping is dynamic.
        let cfg = make_config_for_validation(Provider::Anthropic, Some(ThinkingEffort::XHigh));
        assert!(
            cfg.validate().is_ok(),
            "xhigh must be accepted at startup for Anthropic"
        );
    }

    #[test]
    fn validate_accepts_max_for_anthropic() {
        // max is valid for Anthropic providers.
        let cfg = make_config_for_validation(Provider::Anthropic, Some(ThinkingEffort::Max));
        assert!(cfg.validate().is_ok(), "max must be accepted for Anthropic");
    }

    #[test]
    fn validate_accepts_xhigh_for_openai() {
        // xhigh is valid for OpenAI providers (server-validated per-model).
        let cfg = make_config_for_validation(Provider::OpenAi, Some(ThinkingEffort::XHigh));
        assert!(cfg.validate().is_ok(), "xhigh must be accepted for OpenAI");
    }

    #[test]
    fn validate_accepts_none_and_minimal_for_openai() {
        // none/minimal are valid OpenAI effort values.
        let cfg_none = make_config_for_validation(Provider::OpenAi, Some(ThinkingEffort::None));
        assert!(
            cfg_none.validate().is_ok(),
            "none must be accepted for OpenAI"
        );
        let cfg_minimal =
            make_config_for_validation(Provider::OpenAi, Some(ThinkingEffort::Minimal));
        assert!(
            cfg_minimal.validate().is_ok(),
            "minimal must be accepted for OpenAI"
        );
    }

    // ---- normalize_effort_for_openai_route ----

    #[test]
    fn normalize_openai_route_clamps_max_to_xhigh() {
        // Use an unknown model so only the max→xhigh clamp fires, not per-model logic.
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::Max, "llama-4"),
            ThinkingEffort::XHigh
        );
    }

    #[test]
    fn normalize_openai_route_passes_through_all_other_values_for_unknown_model() {
        // Unknown/unverified models pass through unchanged (server-validated).
        for effort in [
            ThinkingEffort::None,
            ThinkingEffort::Minimal,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
        ] {
            assert_eq!(
                normalize_effort_for_openai_route(effort, "unknown-future-model"),
                effort,
                "normalize_effort_for_openai_route must pass through {effort:?} for unknown model"
            );
        }
    }

    // ---- normalize_effort_for_anthropic_route ----

    #[test]
    fn normalize_anthropic_route_none_yields_none() {
        assert_eq!(
            normalize_effort_for_anthropic_route(ThinkingEffort::None),
            None,
            "none must yield None (omit thinking fields)"
        );
    }

    #[test]
    fn normalize_anthropic_route_minimal_yields_none() {
        assert_eq!(
            normalize_effort_for_anthropic_route(ThinkingEffort::Minimal),
            None,
            "minimal must yield None (omit thinking fields)"
        );
    }

    #[test]
    fn normalize_anthropic_route_passes_through_valid_values() {
        for effort in [
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ] {
            assert_eq!(
                normalize_effort_for_anthropic_route(effort),
                Some(effort),
                "normalize_effort_for_anthropic_route must pass through {effort:?}"
            );
        }
    }

    // ---- F2: Fable 5 / Mythos 5 / Mythos Preview adaptive thinking ----

    #[test]
    fn anthropic_thinking_config_fable_5_emits_adaptive_and_effort() {
        // Fable 5 — always-on adaptive thinking.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-fable-5", ThinkingEffort::High, 32_768);
        let t = thinking.expect("thinking must be present for claude-fable-5");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-fable-5");
        assert_eq!(oc["effort"], "high");
    }

    #[test]
    fn anthropic_thinking_config_mythos_5_emits_adaptive_and_effort() {
        // Mythos 5 — always-on adaptive thinking.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-mythos-5", ThinkingEffort::Medium, 32_768);
        let t = thinking.expect("thinking must be present for claude-mythos-5");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-mythos-5");
        assert_eq!(oc["effort"], "medium");
    }

    #[test]
    fn anthropic_thinking_config_mythos_preview_emits_adaptive_and_effort() {
        // Mythos Preview — default-on adaptive thinking.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-mythos-preview", ThinkingEffort::Low, 32_768);
        let t = thinking.expect("thinking must be present for claude-mythos-preview");
        assert_eq!(t["type"], "adaptive");
        let oc = output_config.expect("output_config must be present for claude-mythos-preview");
        assert_eq!(oc["effort"], "low");
    }

    #[test]
    fn clamp_adaptive_effort_xhigh_passes_through_for_fable_5() {
        // Fable 5 supports xhigh.
        assert_eq!(
            clamp_adaptive_effort("claude-fable-5", ThinkingEffort::XHigh),
            ThinkingEffort::XHigh
        );
    }

    #[test]
    fn clamp_adaptive_effort_xhigh_passes_through_for_mythos_5() {
        // Mythos 5 supports xhigh.
        assert_eq!(
            clamp_adaptive_effort("claude-mythos-5", ThinkingEffort::XHigh),
            ThinkingEffort::XHigh
        );
    }

    #[test]
    fn clamp_adaptive_effort_xhigh_clamped_to_high_for_mythos_preview() {
        // Mythos Preview does NOT support xhigh — clamp to high.
        assert_eq!(
            clamp_adaptive_effort("claude-mythos-preview", ThinkingEffort::XHigh),
            ThinkingEffort::High
        );
    }

    #[test]
    fn clamp_adaptive_effort_max_passes_through_for_fable_5() {
        // Fable 5 supports max.
        assert_eq!(
            clamp_adaptive_effort("claude-fable-5", ThinkingEffort::Max),
            ThinkingEffort::Max
        );
    }

    #[test]
    fn clamp_adaptive_effort_max_passes_through_for_mythos_5() {
        // Mythos 5 supports max.
        assert_eq!(
            clamp_adaptive_effort("claude-mythos-5", ThinkingEffort::Max),
            ThinkingEffort::Max
        );
    }

    #[test]
    fn clamp_adaptive_effort_max_passes_through_for_mythos_preview() {
        // Mythos Preview supports max.
        assert_eq!(
            clamp_adaptive_effort("claude-mythos-preview", ThinkingEffort::Max),
            ThinkingEffort::Max
        );
    }

    #[test]
    fn anthropic_thinking_config_fable_5_xhigh_emits_xhigh() {
        let (thinking, output_config) =
            anthropic_thinking_config("claude-fable-5", ThinkingEffort::XHigh, 32_768);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "adaptive");
        assert_eq!(output_config.unwrap()["effort"], "xhigh");
    }

    #[test]
    fn anthropic_thinking_config_mythos_5_xhigh_emits_xhigh() {
        let (thinking, output_config) =
            anthropic_thinking_config("claude-mythos-5", ThinkingEffort::XHigh, 32_768);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "adaptive");
        assert_eq!(output_config.unwrap()["effort"], "xhigh");
    }

    #[test]
    fn anthropic_thinking_config_mythos_preview_xhigh_clamps_to_high() {
        // Mythos Preview does NOT support xhigh → clamp to high.
        let (thinking, output_config) =
            anthropic_thinking_config("claude-mythos-preview", ThinkingEffort::XHigh, 32_768);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "adaptive");
        assert_eq!(
            output_config.unwrap()["effort"],
            "high",
            "xhigh must clamp to high for claude-mythos-preview"
        );
    }

    #[test]
    fn anthropic_thinking_config_fable_5_max_passes_through() {
        let (thinking, output_config) =
            anthropic_thinking_config("claude-fable-5", ThinkingEffort::Max, 32_768);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "adaptive");
        assert_eq!(output_config.unwrap()["effort"], "max");
    }

    #[test]
    fn anthropic_thinking_config_mythos_preview_max_passes_through() {
        let (thinking, output_config) =
            anthropic_thinking_config("claude-mythos-preview", ThinkingEffort::Max, 32_768);
        let t = thinking.unwrap();
        assert_eq!(t["type"], "adaptive");
        assert_eq!(output_config.unwrap()["effort"], "max");
    }

    // ---- openai_efforts_for_model / normalize_effort_for_openai_route per-model table ----

    #[test]
    fn openai_efforts_for_model_gpt5_pro_high_only() {
        // gpt-5-pro: high only — any other value must be substituted.
        let supported = openai_efforts_for_model("gpt-5-pro").expect("gpt-5-pro must be in table");
        assert_eq!(
            supported,
            &[ThinkingEffort::High],
            "gpt-5-pro supports only high"
        );
    }

    #[test]
    fn openai_efforts_for_model_gpt5_6_includes_max() {
        let expected: &[ThinkingEffort] = &[
            ThinkingEffort::None,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
            ThinkingEffort::Max,
        ];

        for model in ["gpt-5.6", "gpt-5.6-sol", "gpt-5-6-sol", "goose-gpt-5-6-sol"] {
            assert_eq!(
                openai_efforts_for_model(model),
                Some(expected),
                "{model} must match the gpt-5.6 effort table"
            );
        }
    }

    #[test]
    fn openai_efforts_for_model_gpt5_5_includes_xhigh() {
        let supported = openai_efforts_for_model("gpt-5.5").expect("gpt-5.5 must be in table");
        assert!(
            supported.contains(&ThinkingEffort::XHigh),
            "gpt-5.5 must support xhigh"
        );
        assert!(
            supported.contains(&ThinkingEffort::None),
            "gpt-5.5 must support none"
        );
    }

    #[test]
    fn openai_efforts_for_model_gpt5_1_excludes_xhigh_and_minimal() {
        let supported = openai_efforts_for_model("gpt-5.1").expect("gpt-5.1 must be in table");
        assert!(
            !supported.contains(&ThinkingEffort::XHigh),
            "gpt-5.1 must NOT support xhigh"
        );
        assert!(
            !supported.contains(&ThinkingEffort::Minimal),
            "gpt-5.1 must NOT support minimal"
        );
        assert!(
            supported.contains(&ThinkingEffort::None),
            "gpt-5.1 must support none"
        );
    }

    #[test]
    fn openai_efforts_for_model_gpt5_base_excludes_none_includes_minimal() {
        let supported = openai_efforts_for_model("gpt-5").expect("gpt-5 base must be in table");
        assert!(
            !supported.contains(&ThinkingEffort::None),
            "gpt-5 base must NOT support none"
        );
        assert!(
            supported.contains(&ThinkingEffort::Minimal),
            "gpt-5 base must support minimal"
        );
    }

    #[test]
    fn openai_efforts_for_model_unknown_returns_none() {
        // Unknown models are not doc-verified — caller treats as server-validated pass-through.
        assert!(openai_efforts_for_model("llama-4").is_none());
        assert!(openai_efforts_for_model("claude-opus-4-8").is_none());
        assert!(openai_efforts_for_model("gpt-4o").is_none());
    }

    // ---- Boundary-safe matching: version digits must not false-match longer versions ----

    #[test]
    fn openai_efforts_for_model_boundary_dated_base_ids_are_not_versioned() {
        // gpt-5-1106: the "-1" is not version 5.1 — it's a date segment on the base model.
        // Must fall through to base table, not gpt-5.1.
        let result = openai_efforts_for_model("gpt-5-1106");
        let base = openai_efforts_for_model("gpt-5").unwrap();
        assert_eq!(
            result,
            Some(base),
            "gpt-5-1106 must match base table (not gpt-5.1): got {result:?}"
        );
        // Crucially, must NOT support None (that's a gpt-5.1 property, not base).
        assert!(
            !result.unwrap().contains(&ThinkingEffort::None),
            "gpt-5-1106 must NOT support none — base table only has minimal"
        );
    }

    #[test]
    fn openai_efforts_for_model_boundary_gpt5_4o_is_base_not_5_4() {
        // gpt-5-4o: the "-4" could false-match the gpt-5.4 family, but "4o" is a
        // capability suffix on the base gpt-5 model, not version 5.4.
        // Must fall through to base table.
        let result = openai_efforts_for_model("gpt-5-4o");
        let base = openai_efforts_for_model("gpt-5").unwrap();
        assert_eq!(
            result,
            Some(base),
            "gpt-5-4o must match base table (not gpt-5.4): got {result:?}"
        );
        // Crucially, must NOT support XHigh (that's a gpt-5.4 property, not base).
        assert!(
            !result.unwrap().contains(&ThinkingEffort::XHigh),
            "gpt-5-4o must NOT support xhigh — that's a gpt-5.4 property and would 400"
        );
    }

    #[test]
    fn openai_efforts_for_model_boundary_multi_digit_versions_pass_through() {
        // Dotted two-digit versions (gpt-5.10, gpt5.10, gpt-5.50) must not match any known
        // single-digit family — the digit boundary check on dotted tokens blocks them.
        // These return None (server-validated pass-through).
        assert!(
            openai_efforts_for_model("gpt-5.10").is_none(),
            "gpt-5.10 must pass through (unknown future model)"
        );
        assert!(
            openai_efforts_for_model("gpt5.10").is_none(),
            "gpt5.10 must pass through (unknown future model)"
        );
        assert!(
            openai_efforts_for_model("gpt-5.50").is_none(),
            "gpt-5.50 must pass through (not gpt-5.5)"
        );
        // Dash two-digit versions (gpt-5-10, databricks-gpt-5-10) look like short numeric
        // version segments and must also pass through as unknown — not bucketed as base.
        assert!(
            openai_efforts_for_model("gpt-5-10").is_none(),
            "gpt-5-10 must pass through (short numeric suffix = potential unrecognized version)"
        );
        assert!(
            openai_efforts_for_model("databricks-gpt-5-10").is_none(),
            "databricks-gpt-5-10 must pass through (short numeric suffix)"
        );
        // Short numeric suffix + textual continuation (e.g. a hypothetical 'gpt-5.10-preview')
        // must also pass through — the digit count (1-3) determines version-like, regardless of
        // what follows.
        assert!(
            openai_efforts_for_model("gpt-5-10-preview").is_none(),
            "gpt-5-10-preview must pass through (short numeric version suffix with text tail)"
        );
        assert!(
            openai_efforts_for_model("databricks-gpt-5-10-preview").is_none(),
            "databricks-gpt-5-10-preview must pass through (short numeric version suffix with text tail)"
        );
    }

    #[test]
    fn openai_efforts_for_model_boundary_date_segment_with_suffix_is_base() {
        // 4+ digit date segment followed by a textual suffix must still resolve to the base
        // table — the date length (>=4) determines it's a build/date, not a version number.
        let result = openai_efforts_for_model("gpt-5-1106-preview");
        assert!(
            result.is_some(),
            "gpt-5-1106-preview must match base table (4-digit date segment)"
        );
        let supported = result.unwrap();
        assert!(
            supported.contains(&ThinkingEffort::Minimal),
            "gpt-5-1106-preview (base) must support minimal"
        );
        assert!(
            !supported.contains(&ThinkingEffort::None),
            "gpt-5-1106-preview (base) must NOT support none"
        );
        assert!(
            !supported.contains(&ThinkingEffort::XHigh),
            "gpt-5-1106-preview (base) must NOT support xhigh"
        );
    }

    #[test]
    fn openai_efforts_for_model_boundary_databricks_prefixed_still_matches() {
        // Databricks-prefixed names (gateway forwarding) must still resolve to the right table.
        let result = openai_efforts_for_model("databricks-gpt-5-5");
        assert_eq!(
            result,
            openai_efforts_for_model("gpt-5.5"),
            "databricks-gpt-5-5 must match gpt-5.5 family table"
        );
    }

    #[test]
    fn openai_efforts_for_model_boundary_date_suffixed_still_matches() {
        // Date-suffixed names (e.g. gpt-5.1-2025-04-01) must still resolve to the right family.
        let result = openai_efforts_for_model("gpt-5.1-2025-04-01");
        assert_eq!(
            result,
            openai_efforts_for_model("gpt-5.1"),
            "gpt-5.1-2025-04-01 must match gpt-5.1 family table"
        );
    }

    #[test]
    fn openai_efforts_for_model_pro_before_base_gpt5() {
        // gpt-5-pro must match the -pro table, not the base gpt-5 table.
        let pro = openai_efforts_for_model("gpt-5-pro").unwrap();
        let base = openai_efforts_for_model("gpt-5").unwrap();
        assert_ne!(
            pro, base,
            "gpt-5-pro and gpt-5 base must hit different table entries"
        );
        assert_eq!(pro, &[ThinkingEffort::High]);
    }

    #[test]
    fn normalize_openai_route_gpt5_pro_high_passes_through() {
        // gpt-5-pro: high is the only supported value → high passes through unchanged.
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::High, "gpt-5-pro"),
            ThinkingEffort::High
        );
    }

    #[test]
    fn normalize_openai_route_gpt5_pro_anything_but_high_becomes_high() {
        // gpt-5-pro: any effort other than high must resolve to high.
        for effort in [
            ThinkingEffort::None,
            ThinkingEffort::Minimal,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::XHigh,
        ] {
            assert_eq!(
                normalize_effort_for_openai_route(effort, "gpt-5-pro"),
                ThinkingEffort::High,
                "gpt-5-pro: {effort:?} must resolve to high"
            );
        }
    }

    #[test]
    fn normalize_openai_route_gpt5_base_none_becomes_minimal() {
        // gpt-5 base supports minimal but not none. none → minimal (peer fallback).
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::None, "gpt-5"),
            ThinkingEffort::Minimal,
            "gpt-5 base: none must fall back to minimal (peer)"
        );
    }

    #[test]
    fn normalize_openai_route_passes_max_through_for_gpt5_6() {
        for model in ["gpt-5.6", "gpt-5-6-sol"] {
            assert_eq!(
                normalize_effort_for_openai_route(ThinkingEffort::Max, model),
                ThinkingEffort::Max,
                "{model} must preserve max"
            );
        }
    }

    #[test]
    fn normalize_openai_route_gpt5_5_max_becomes_xhigh() {
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::Max, "gpt-5.5"),
            ThinkingEffort::XHigh,
            "gpt-5.5 must clamp max to xhigh"
        );
    }

    #[test]
    fn normalize_openai_route_gpt5_5_minimal_becomes_none() {
        // gpt-5.5 supports none but not minimal. minimal → none (peer fallback).
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::Minimal, "gpt-5.5"),
            ThinkingEffort::None,
            "gpt-5.5: minimal must fall back to none (peer)"
        );
    }

    #[test]
    fn normalize_openai_route_gpt5_1_xhigh_becomes_high() {
        // gpt-5.1 does not support xhigh → nearest supported below xhigh is high.
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::XHigh, "gpt-5.1"),
            ThinkingEffort::High,
            "gpt-5.1: xhigh must resolve to high"
        );
    }

    #[test]
    fn normalize_openai_route_gpt5_4_xhigh_passes_through() {
        // gpt-5.4 supports xhigh → pass through unchanged.
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::XHigh, "gpt-5.4"),
            ThinkingEffort::XHigh
        );
    }

    #[test]
    fn normalize_openai_route_gpt5_5_xhigh_passes_through() {
        // gpt-5.5 supports xhigh → pass through unchanged.
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::XHigh, "gpt-5.5"),
            ThinkingEffort::XHigh
        );
    }

    #[test]
    fn normalize_openai_route_gpt5_dash_suffix_variants_match_correctly() {
        // Databricks-prefixed or date-suffixed names must still hit the right family.
        // "gpt-5.5" and "gpt-5-5" are treated identically; ditto for other families.
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::XHigh, "gpt-5-5"),
            ThinkingEffort::XHigh,
            "gpt-5-5 (dash) must match gpt-5.5 table"
        );
        assert_eq!(
            normalize_effort_for_openai_route(ThinkingEffort::None, "gpt-5-1"),
            ThinkingEffort::None,
            "gpt-5-1 (dash) must match gpt-5.1 table"
        );
    }

    #[test]
    fn normalize_openai_route_unknown_model_passthrough() {
        // Unknown models: all values pass through without substitution (server-validated).
        for effort in [
            ThinkingEffort::None,
            ThinkingEffort::Minimal,
            ThinkingEffort::Low,
            ThinkingEffort::Medium,
            ThinkingEffort::High,
            ThinkingEffort::XHigh,
        ] {
            assert_eq!(
                normalize_effort_for_openai_route(effort, "llama-4"),
                effort,
                "unknown model: {effort:?} must pass through unchanged"
            );
        }
    }

    // ---- effort-table fixture sync guard ----------------------------------------
    //
    // Loads `effortTable.fixture.json` (the single source of truth shared with
    // the TS test in `buzzAgentConfig.test.mjs`) and verifies that this Rust
    // implementation produces the same valid-effort-value sets and default values
    // as the TS `getProviderEffortConfig` function.
    //
    // Drift (a new model family added to one side but not the other) fails CI here
    // before it can silently diverge in production.
    // ─────────────────────────────────────────────────────────────────────────────

    /// Compute the valid effort values for a provider/model pair, mirroring
    /// `getProviderEffortConfig` in `buzzAgentConfig.ts`.
    ///
    /// Returns `(valid_values, default_value)` where `default_value` is `None`
    /// for Anthropic manual-budget models (TS `defaultValue: null`), otherwise
    /// `Some("medium")` or `Some("high")`.
    fn valid_effort_values_for_provider_model(
        provider: &str,
        model: &str,
    ) -> (Vec<&'static str>, Option<&'static str>) {
        const ALL_7: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];
        const ALL_EXCEPT_MAX: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh"];
        const GPT5_PRO: &[&str] = &["high"];
        const GPT5_1: &[&str] = &["none", "low", "medium", "high"];

        let p = provider.to_ascii_lowercase();
        // Strip arbitrary endpoint-naming prefix before model matching, mirroring TS and
        // strip_catalog_prefix: find the first known family token (claude-, gpt-) and
        // drop everything before it. Handles any catalog naming convention.
        let raw_model = model.trim();
        let lower_raw = raw_model.to_ascii_lowercase();
        const FAMILY_TOKENS: &[&str] = &["claude-", "gpt-"];
        let first_idx = FAMILY_TOKENS
            .iter()
            .filter_map(|tok| lower_raw.find(tok))
            .min();
        let stripped = match first_idx {
            Some(idx) => &raw_model[idx..],
            None => raw_model,
        };
        let m = stripped.to_ascii_lowercase();

        // Thin adapter: converts production helper output to the string-based
        // return type used by this function.
        fn anthropic_result(m: &str) -> (Vec<&'static str>, Option<&'static str>) {
            let (values, default) = anthropic_efforts_for_model(m);
            let strs: Vec<&'static str> = values.iter().map(|e| e.openai_effort_str()).collect();
            (strs, default.map(|e| e.openai_effort_str()))
        }

        fn openai_result(m: &str) -> (Vec<&'static str>, Option<&'static str>) {
            if let Some(values) = openai_efforts_for_model(m) {
                let strs: Vec<&'static str> =
                    values.iter().map(|e| e.openai_effort_str()).collect();
                // Determine default from the family.
                let default_val = if strs == GPT5_PRO {
                    Some("high")
                } else if strs == GPT5_1 {
                    Some("none")
                } else {
                    Some("medium")
                };
                (strs, default_val)
            } else {
                // Unknown model → all-except-max, default medium.
                (ALL_EXCEPT_MAX.to_vec(), Some("medium"))
            }
        }

        if p == "anthropic" {
            return anthropic_result(&m);
        }
        if p == "openai" {
            return openai_result(&m);
        }
        if p == "databricks_v2" {
            if m.starts_with("claude-") {
                return anthropic_result(&m);
            }
            // gpt-5 family check mirrors gpt5FamilyModel in TS.
            let is_gpt5 = gpt5_token_matches(&m, "gpt-5-pro")
                || gpt5_token_matches(&m, "gpt5-pro")
                || gpt5_token_matches(&m, "gpt-5.6")
                || gpt5_token_matches(&m, "gpt5.6")
                || gpt5_token_matches(&m, "gpt-5-6")
                || gpt5_token_matches(&m, "gpt5-6")
                || gpt5_token_matches(&m, "gpt-5.5")
                || gpt5_token_matches(&m, "gpt5.5")
                || gpt5_token_matches(&m, "gpt-5.4")
                || gpt5_token_matches(&m, "gpt5.4")
                || gpt5_token_matches(&m, "gpt-5.1")
                || gpt5_token_matches(&m, "gpt5.1")
                || gpt5_base_matches(&m, "gpt-5")
                || gpt5_base_matches(&m, "gpt5");
            if is_gpt5 {
                return openai_result(&m);
            }
            if !m.is_empty() {
                // Concrete non-claude, non-gpt5: MLflow path → all-except-max.
                return openai_result(&m);
            }
            // Blank model: route unknown, all-7.
            return (ALL_7.to_vec(), Some("medium"));
        }
        if p == "databricks" {
            return openai_result(&m);
        }
        // openai-compat, unknown, empty → all-7, default medium.
        (ALL_7.to_vec(), Some("medium"))
    }

    #[derive(serde::Deserialize)]
    struct FixtureEntry {
        note: Option<String>,
        provider: String,
        model: String,
        #[serde(rename = "validValues")]
        valid_values: Vec<String>,
        #[serde(rename = "defaultValue")]
        default_value: Option<String>,
    }

    #[test]
    fn effort_table_fixture_matches_rust_implementation() {
        let fixture_json =
            include_str!("../../../desktop/src/features/agents/ui/effortTable.fixture.json");
        let entries: Vec<FixtureEntry> =
            serde_json::from_str(fixture_json).expect("fixture must be valid JSON");

        assert!(
            !entries.is_empty(),
            "fixture must contain at least one entry"
        );

        for entry in &entries {
            let label = entry.note.as_deref().unwrap_or(entry.model.as_str());
            let (valid_values, default_value) =
                valid_effort_values_for_provider_model(&entry.provider, &entry.model);

            let expected: Vec<&str> = entry.valid_values.iter().map(String::as_str).collect();
            assert_eq!(
                valid_values, expected,
                "validValues mismatch for fixture entry \"{label}\" \
                 (provider={}, model={}): Rust side has {valid_values:?}, \
                 fixture expects {expected:?}",
                entry.provider, entry.model,
            );

            let expected_default: Option<&str> = entry.default_value.as_deref();
            assert_eq!(
                default_value, expected_default,
                "defaultValue mismatch for fixture entry \"{label}\" \
                 (provider={}, model={}): Rust side has {default_value:?}, \
                 fixture expects {expected_default:?}",
                entry.provider, entry.model,
            );
        }
    }
}
