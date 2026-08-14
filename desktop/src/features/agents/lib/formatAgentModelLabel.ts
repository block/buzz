/**
 * Returns a human-readable model label for an agent or persona, falling back to
 * "Auto" when no model is set (empty or whitespace-only).
 */
export function formatAgentModelLabel(model: string | null | undefined) {
  const trimmed = model?.trim();
  return trimmed && trimmed.length > 0 ? trimmed : "Auto";
}
