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

/**
 * A linked agent's profile is authoritative even when the definition already
 * supplies a fallback. Avatar-dependent actions must wait for that profile
 * query so they cannot snapshot the fallback before the profile resolves.
 */
export function isAgentCardAvatarLoading(
  hasLinkedAgent: boolean,
  isProfilePending: boolean,
): boolean {
  return hasLinkedAgent && isProfilePending;
}
