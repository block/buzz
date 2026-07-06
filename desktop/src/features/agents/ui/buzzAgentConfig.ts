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

/**
 * Returns true when the given runtime id is buzz-agent, which is the only
 * runtime that supports the tier-1 model-tuning knobs above.
 */
export function isBuzzAgentRuntime(runtimeId: string): boolean {
  return runtimeId === "buzz-agent";
}
