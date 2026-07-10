/**
 * Source-of-truth constants for buzz-agent model-tuning configuration knobs.
 *
 * Values must stay in sync with `crates/buzz-agent/src/config.rs`
 * `parse_thinking_effort` — that function is the authoritative list.
 */

/** Env var key for the thinking/effort level sent to the LLM. */
export const BUZZ_AGENT_THINKING_EFFORT = "BUZZ_AGENT_THINKING_EFFORT";

/** Env var key for the maximum output token count per turn. */
export const BUZZ_AGENT_MAX_OUTPUT_TOKENS = "BUZZ_AGENT_MAX_OUTPUT_TOKENS";

/** Env var key for the context window token limit. */
export const BUZZ_AGENT_MAX_CONTEXT_TOKENS = "BUZZ_AGENT_MAX_CONTEXT_TOKENS";

/** Env var key for the maximum number of LLM/tool rounds per turn. */
export const BUZZ_AGENT_MAX_ROUNDS = "BUZZ_AGENT_MAX_ROUNDS";

/**
 * Ordered set of valid thinking-effort values accepted by buzz-agent.
 * Mirrors `parse_thinking_effort` in `crates/buzz-agent/src/config.rs`.
 */
export const BUZZ_AGENT_THINKING_EFFORT_VALUES = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
] as const;

export type ThinkingEffortValue =
  (typeof BUZZ_AGENT_THINKING_EFFORT_VALUES)[number];

// ---------------------------------------------------------------------------
// Provider-aware effort configuration
// ---------------------------------------------------------------------------

/**
 * Describes which thinking-effort values are valid for a given provider+model,
 * and which value is the provider's semantic default.
 *
 * `defaultValue = null` means the provider/model's default is to omit the
 * thinking configuration entirely (i.e. "Inherit" is the natural default).
 * This applies to Anthropic manual-budget models where the effort level maps
 * to a budget_tokens count — there is no "default effort level" in the API.
 *
 * Mirrors the model-family tables in `crates/buzz-agent/src/config.rs`
 * (`openai_efforts_for_model`, `is_manual_budget_model`,
 * `is_adaptive_thinking_model`, `clamp_adaptive_effort`). Keep in sync.
 */
export type ProviderEffortConfig = {
  validValues: ReadonlyArray<ThinkingEffortValue>;
  /** Provider/model's semantic default, or `null` when Inherit is the default. */
  defaultValue: ThinkingEffortValue | null;
};

const ALL_VALUES = BUZZ_AGENT_THINKING_EFFORT_VALUES;

/**
 * Returns the valid thinking-effort values and semantic default for the
 * given provider and optional model string.
 *
 * Model matching mirrors the Rust backend:
 * - Anthropic: strip `databricks-` prefix, then test `is_manual_budget_model`
 *   / `is_adaptive_thinking_model` / `clamp_adaptive_effort` family checks.
 * - OpenAI: strip `databricks-` prefix, then test `openai_efforts_for_model`
 *   family checks (boundary-aware: -pro before -5.x, digit/letter boundary).
 * - DatabricksV2: strip prefix and route by model family.
 * - Unknown/empty: all 7 values, default medium.
 */
export function getProviderEffortConfig(
  providerId: string,
  model?: string,
): ProviderEffortConfig {
  const provider = providerId.toLowerCase();
  // Strip Databricks gateway prefix before model-family matching.
  const rawModel = (model ?? "").trim();
  const strippedModel = rawModel.startsWith("databricks-")
    ? rawModel.slice("databricks-".length)
    : rawModel.startsWith("goose-")
      ? rawModel.slice("goose-".length)
      : rawModel;
  const m = strippedModel.toLowerCase();

  if (provider === "anthropic") {
    return anthropicConfig(m);
  }
  if (provider === "openai") {
    return openaiConfig(m);
  }
  if (provider === "databricks_v2") {
    // Route by model family: claude* → Anthropic tables, gpt-5* → OpenAI tables.
    // Non-Claude concrete models (e.g. llama-3) go through MlflowChatCompletions,
    // which applies normalize_effort_for_openai_route → clamps max to xhigh.
    // Route them through openaiConfig to exclude max. Only blank/unknown model
    // uses the all-7 fallback (can't know the route without a concrete model).
    if (m.startsWith("claude-")) {
      return anthropicConfig(m);
    }
    if (gpt5FamilyModel(m)) {
      return openaiConfig(m);
    }
    if (m.length > 0) {
      // Concrete non-Claude, non-GPT model → MLflow path clamps max → xhigh.
      return openaiConfig(m);
    }
    // Blank model — route unknown, show all 7.
    return { validValues: ALL_VALUES, defaultValue: "medium" };
  }
  if (provider === "databricks") {
    // databricks v1 uses OpenAI Chat Completions wire format.
    return openaiConfig(m);
  }
  // openai-compat, unknown, empty — all values, default medium.
  return { validValues: ALL_VALUES, defaultValue: "medium" };
}

// ---------------------------------------------------------------------------
// Anthropic family tables
// ---------------------------------------------------------------------------

function anthropicConfig(m: string): ProviderEffortConfig {
  // Manual-budget models: claude-3* and claude-opus-4-5.
  // These use budget_tokens — there is no "default effort level" in the API.
  if (m.startsWith("claude-3") || m === "claude-opus-4-5") {
    return {
      validValues: ["low", "medium", "high"],
      defaultValue: null,
    };
  }
  // Adaptive models that support xhigh: opus-4-7+, sonnet-5.x, fable-5, mythos-5.
  // mirrors clamp_adaptive_effort supports_xhigh check.
  if (
    m.startsWith("claude-opus-4-7") ||
    m.startsWith("claude-opus-4-8") ||
    m.startsWith("claude-sonnet-5") ||
    m.startsWith("claude-fable-5") ||
    m.startsWith("claude-mythos-5")
  ) {
    return {
      validValues: ["low", "medium", "high", "xhigh", "max"],
      defaultValue: "high",
    };
  }
  // Adaptive models that do NOT support xhigh: opus-4-6, sonnet-4-6, mythos-preview.
  if (
    m.startsWith("claude-opus-4-6") ||
    m.startsWith("claude-sonnet-4-6") ||
    m.startsWith("claude-mythos-preview")
  ) {
    return {
      validValues: ["low", "medium", "high", "max"],
      defaultValue: "high",
    };
  }
  // Unknown Anthropic model — assume adaptive with full support.
  return {
    validValues: ["low", "medium", "high", "xhigh", "max"],
    defaultValue: "high",
  };
}

// ---------------------------------------------------------------------------
// OpenAI family tables — mirrors openai_efforts_for_model in config.rs
// ---------------------------------------------------------------------------

/**
 * Returns true if `m` contains a GPT-5 family token at a word boundary
 * (not immediately followed by a digit or letter). Mirrors
 * `gpt5_token_matches` / `gpt5_base_matches` in config.rs.
 */
function gpt5TokenMatches(m: string, token: string): boolean {
  let start = 0;
  while (true) {
    const idx = m.indexOf(token, start);
    if (idx === -1) return false;
    const afterIdx = idx + token.length;
    const afterChar = afterIdx < m.length ? m[afterIdx] : "";
    // Boundary: end-of-string or a `-` separator (not a digit or letter).
    if (afterChar === "" || afterChar === "-") return true;
    start = afterIdx;
  }
}

/** Like gpt5TokenMatches but also rejects short -<1-3 digit> suffixes (e.g. -5, -10). */
function gpt5BaseMatches(m: string, token: string): boolean {
  let start = 0;
  while (true) {
    const idx = m.indexOf(token, start);
    if (idx === -1) return false;
    const afterIdx = idx + token.length;
    const suffix = m.slice(afterIdx);
    if (suffix === "") return true;
    if (!suffix.startsWith("-")) {
      start = afterIdx;
      continue;
    }
    // Has a `-` suffix — check if it looks like a 1-3 digit version number.
    const dashRest = suffix.slice(1);
    if (/^\d{1,3}(?:[^a-z\d]|$)/i.test(dashRest)) {
      start = afterIdx;
      continue;
    }
    return true;
  }
}

/** Returns true if the model string belongs to any GPT-5 family. */
function gpt5FamilyModel(m: string): boolean {
  return (
    gpt5TokenMatches(m, "gpt-5-pro") ||
    gpt5TokenMatches(m, "gpt5-pro") ||
    gpt5TokenMatches(m, "gpt-5.5") ||
    gpt5TokenMatches(m, "gpt5.5") ||
    gpt5TokenMatches(m, "gpt-5.4") ||
    gpt5TokenMatches(m, "gpt5.4") ||
    gpt5TokenMatches(m, "gpt-5.1") ||
    gpt5TokenMatches(m, "gpt5.1") ||
    gpt5BaseMatches(m, "gpt-5") ||
    gpt5BaseMatches(m, "gpt5")
  );
}

function openaiConfig(m: string): ProviderEffortConfig {
  // Check -pro before versioned suffixes (gpt-5-pro contains "gpt-5").
  if (gpt5TokenMatches(m, "gpt-5-pro") || gpt5TokenMatches(m, "gpt5-pro")) {
    return { validValues: ["high"], defaultValue: "high" };
  }
  if (
    gpt5TokenMatches(m, "gpt-5.5") ||
    gpt5TokenMatches(m, "gpt5.5") ||
    gpt5TokenMatches(m, "gpt-5-5") ||
    gpt5TokenMatches(m, "gpt5-5") ||
    gpt5TokenMatches(m, "gpt-5.4") ||
    gpt5TokenMatches(m, "gpt5.4") ||
    gpt5TokenMatches(m, "gpt-5-4") ||
    gpt5TokenMatches(m, "gpt5-4")
  ) {
    return {
      validValues: ["none", "low", "medium", "high", "xhigh"],
      defaultValue: "medium",
    };
  }
  if (
    gpt5TokenMatches(m, "gpt-5.1") ||
    gpt5TokenMatches(m, "gpt5.1") ||
    gpt5TokenMatches(m, "gpt-5-1") ||
    gpt5TokenMatches(m, "gpt5-1")
  ) {
    return {
      validValues: ["none", "low", "medium", "high"],
      defaultValue: "none",
    };
  }
  if (gpt5BaseMatches(m, "gpt-5") || gpt5BaseMatches(m, "gpt5")) {
    return {
      validValues: ["minimal", "low", "medium", "high"],
      defaultValue: "medium",
    };
  }
  // Unknown OpenAI model — show all except max (OpenAI doesn't accept max).
  return {
    validValues: ["none", "minimal", "low", "medium", "high", "xhigh"],
    defaultValue: "medium",
  };
}

/**
 * Returns true when the given runtime id is buzz-agent, which is the only
 * runtime that supports the tier-1 model-tuning knobs above.
 */
export function isBuzzAgentRuntime(runtimeId: string): boolean {
  return runtimeId === "buzz-agent";
}
