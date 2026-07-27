const FALLBACK_AGENT_DESCRIPTOR = "No description yet";

function collapseWhitespace(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

/**
 * Resolve the persistent one-line copy shown on agent cards.
 *
 * Legacy definitions do not have an explicit description, so their first
 * instruction sentence is used until the user saves a dedicated one-liner.
 */
export function resolveAgentDescriptor(
  description: string | null | undefined,
  systemPrompt: string | null | undefined,
): string {
  const explicit = collapseWhitespace(description ?? "");
  if (explicit) return explicit;

  const prompt = collapseWhitespace(systemPrompt ?? "");
  if (!prompt) return FALLBACK_AGENT_DESCRIPTOR;

  const firstSentenceEnd = prompt.search(/[.!?](?:\s|$)/);
  return firstSentenceEnd >= 0 ? prompt.slice(0, firstSentenceEnd + 1) : prompt;
}
