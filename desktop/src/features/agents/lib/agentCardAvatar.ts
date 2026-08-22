import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  ManagedAgent,
} from "@/shared/api/types";

export function resolveCurrentRuntimeAvatarUrl(
  agent: Pick<
    ManagedAgent,
    "agentCommand" | "agentCommandOverride" | "runtime"
  >,
  persona: Pick<AgentPersona, "runtime">,
  runtimes: readonly AcpRuntimeCatalogEntry[],
): string | null {
  const effectiveCommand = agent.agentCommand.trim();
  const inheritedRuntimeId =
    agent.agentCommandOverride == null
      ? agent.runtime?.trim() || persona.runtime?.trim()
      : null;
  const runtime =
    runtimes.find((candidate) => candidate.id.trim() === inheritedRuntimeId) ??
    runtimes.find((candidate) =>
      [candidate.command, candidate.binaryPath].some(
        (value) => value?.trim() === effectiveCommand,
      ),
    );

  return runtime?.avatarUrl.trim() || null;
}

/**
 * Resolve the avatar for a running agent card.
 *
 * The card opens the concrete agent pubkey's profile, so that profile's kind:0
 * picture is authoritative. The linked definition remains a fallback while the
 * profile is missing or has no picture. A catalog-recognized stock avatar may
 * be replaced for display when the agent now uses a different runtime; unknown
 * and custom URLs remain authoritative.
 */
export function resolveAgentCardAvatarUrl(
  profileAvatarUrl: string | null | undefined,
  personaAvatarUrl: string | null | undefined,
  currentRuntimeAvatarUrl?: string | null,
  catalogStockAvatarUrls?: ReadonlySet<string> | null,
): string | null {
  const currentRuntimeAvatar = currentRuntimeAvatarUrl?.trim() || null;
  const shouldReplaceStockAvatar =
    currentRuntimeAvatarUrl !== undefined && catalogStockAvatarUrls != null;
  const resolveCandidate = (candidate: string | null | undefined) => {
    const trimmed = candidate?.trim();
    if (!trimmed) return null;

    const isCatalogStockAvatar = catalogStockAvatarUrls?.has(trimmed) ?? false;
    return isCatalogStockAvatar && shouldReplaceStockAvatar
      ? currentRuntimeAvatar
      : trimmed;
  };

  for (const candidate of [profileAvatarUrl, personaAvatarUrl]) {
    const resolved = resolveCandidate(candidate);
    if (resolved) return resolved;
  }

  return shouldReplaceStockAvatar ? currentRuntimeAvatar : null;
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
