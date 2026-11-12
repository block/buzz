/**
 * Source-of-truth constants for buzz-agent model-tuning configuration knobs.
 *
 * The thinking-effort value list and the provider/model → effort projection are
 * both derived from the shared capability manifest via the interpreter in
 * `./modelCapabilities`; this module owns only the buzz-agent env-var keys and
 * the runtime-id guard. Mirrors the `config.rs` ⇄ `model_capabilities.rs` seam
 * in `crates/buzz-agent`, where effort resolution is delegated to the manifest.
 * (The interpreter owns the value list rather than the reverse, because it uses
 * the values at module-load for zod — the acyclic direction.)
 */
import {
  THINKING_EFFORT_VALUES,
  type ThinkingEffortValue,
  resolveModelCapabilities,
} from "./modelCapabilities";

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
 * Re-exported from the manifest interpreter, which owns the canonical list
 * (mirrors `parse_thinking_effort` in `crates/buzz-agent/src/config.rs`).
 */
export const BUZZ_AGENT_THINKING_EFFORT_VALUES = THINKING_EFFORT_VALUES;

export type { ThinkingEffortValue };

/**
 * Describes which thinking-effort values are valid for a given provider+model,
 * and which value is the provider's semantic default.
 *
 * `defaultValue = null` means the provider/model's default is to omit the
 * thinking configuration entirely (i.e. "Inherit" is the natural default).
 * This applies to Anthropic manual-budget models, whose effort maps to a
 * budget_tokens count — there is no "default effort level" in the API.
 */
export type ProviderEffortConfig = {
  validValues: ReadonlyArray<ThinkingEffortValue>;
  /** Provider/model's semantic default, or `null` when Inherit is the default. */
  defaultValue: ThinkingEffortValue | null;
};

/**
 * Returns the valid thinking-effort values and semantic default for the given
 * provider and optional model, projected from the shared capability manifest.
 *
 * A thin projection over `resolveModelCapabilities`: `validValues` is the
 * resolved `supportedEfforts` axis and `defaultValue` is `defaultEffort`.
 * Provider canonicalization (alias map) and endpoint-prefix stripping happen
 * inside the resolver, so callers pass raw provider/model strings.
 */
export function getProviderEffortConfig(
  providerId: string,
  model?: string,
): ProviderEffortConfig {
  const cap = resolveModelCapabilities(providerId, model ?? "");
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
