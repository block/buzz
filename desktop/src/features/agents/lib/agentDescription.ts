import type { AgentPersona } from "@/shared/api/types";

/** Hard cap on a public agent description, mirroring the Rust validator. */
export const MAX_AGENT_DESCRIPTION_CHARS = 280;

/**
 * The description to display for a persona: the authored `description`,
 * trimmed, when non-empty; otherwise `null`.
 *
 * Rust twin: `effective_agent_description` in
 * `managed_agents/agent_description.rs`, which resolves the same value on
 * the kind:0 `about` publish path — keep both in sync.
 */
export function effectiveAgentDescription(
  persona: Partial<Pick<AgentPersona, "description">>,
): string | null {
  const authored = persona.description?.trim() ?? "";
  return authored.length > 0 ? authored : null;
}
