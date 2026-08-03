/**
 * Source-of-truth constants for buzz-agent model-tuning configuration knobs.
 *
 * `getProviderEffortConfig()` is generated-backed (thin wrapper over
 * `resolveModelCapabilities()`).
 */
import { canonicalizeProvider } from "../lib/formatAgentModelLabel.ts";
import { resolveModelCapabilities } from "./modelCapabilities.ts";

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
 */
export type ProviderEffortConfig = {
  validValues: ReadonlyArray<ThinkingEffortValue>;
  /** Provider/model's semantic default, or `null` when Inherit is the default. */
  defaultValue: ThinkingEffortValue | null;
};

/**
 * Returns the valid thinking-effort values and semantic default for the
 * given provider and optional model string, resolved from the generated
 * model-capabilities manifest (modelCapabilities.ts).
 *
 * This is the production entry point for all UI consumers. Resolution order:
 *   1. Provider-qualified raw exact lookup
 *   2. Provider-scoped family rules on normalized alias
 *   3. Per-provider fallback
 *
 * The manifest's `supportedEfforts` maps to `validValues`; `defaultEffort`
 * (which may be null for manual-budget models — "Inherit" is the natural
 * default) maps to `defaultValue`.
 */
export function getProviderEffortConfig(
  providerId: string,
  model?: string,
): ProviderEffortConfig {
  const cap = resolveModelCapabilities(
    canonicalizeProvider(providerId),
    model ?? "",
  );
  return {
    validValues: cap.supportedEfforts,
    defaultValue: cap.defaultEffort,
  };
}

/**
 * Returns true when the given runtime id is buzz-agent, which is the only
 * runtime that supports the tier-1 model-tuning knobs above.
 */
export function isBuzzAgentRuntime(runtimeId: string): boolean {
  return runtimeId === "buzz-agent";
}
