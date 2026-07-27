/**
 * Select the first non-empty avatar source for an agent-management card.
 *
 * Callers provide sources in presentation-priority order. The final source is
 * normally the runtime-catalog fallback projected by the backend, so stopped
 * legacy agents still render their runtime logo without overwriting stored or
 * relay-published custom avatars.
 */
export function resolveAgentCardAvatarUrl(
  candidates: Array<string | null | undefined>,
  supersededRuntimeAvatarUrls: readonly string[] = [],
): string | null {
  const superseded = new Set(
    supersededRuntimeAvatarUrls.map((candidate) => candidate.trim()),
  );
  for (const candidate of candidates) {
    const trimmed = candidate?.trim();
    if (trimmed && !superseded.has(trimmed)) return trimmed;
  }
  return null;
}
