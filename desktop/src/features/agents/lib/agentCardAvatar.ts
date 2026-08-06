/**
 * Resolve the avatar for a running agent card.
 *
 * The card opens the concrete agent pubkey's profile, so that profile's kind:0
 * picture is authoritative. The linked definition remains a fallback while the
 * profile is missing or has no picture.
 */
export function resolveAgentCardAvatarUrl(
  profileAvatarUrl: string | null | undefined,
  personaAvatarUrl: string | null | undefined,
): string | null {
  for (const candidate of [profileAvatarUrl, personaAvatarUrl]) {
    const trimmed = candidate?.trim();
    if (trimmed) return trimmed;
  }
  return null;
}
