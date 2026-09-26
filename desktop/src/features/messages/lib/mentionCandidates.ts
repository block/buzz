import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import { overlayCommunityBotDisplayName } from "@/features/community-bots/lib/displayName";
import type { CommunityBot } from "@/features/community-bots/lib/types";
import type {
  AgentPersona,
  AgentTeam,
  ChannelMember,
  ChannelRole,
  UserProfileSummary,
  UserSearchResult,
} from "@/shared/api/types";
import { normalizePubkey, truncateNpub } from "@/shared/lib/pubkey";

export function formatSearchUserDisplayName(user: UserSearchResult) {
  return user.displayName?.trim() || user.nip05Handle?.trim() || null;
}

export function formatSearchUserSecondaryLabel(user: UserSearchResult) {
  const displayName = user.displayName?.trim();
  const nip05Handle = user.nip05Handle?.trim();
  return displayName && nip05Handle ? nip05Handle : null;
}

export function appendUniqueName(current: string[], name: string): string[] {
  return current.some(
    (candidate) => candidate.toLowerCase() === name.toLowerCase(),
  )
    ? current
    : [...current, name];
}

export type TeamMentionMember = {
  displayName: string;
  kind: "identity" | "persona";
  personaId?: string;
  pubkey?: string;
};

export type MentionCandidate = {
  kind: "identity" | "persona" | "team";
  pubkey?: string;
  personaId?: string;
  teamId?: string;
  teamMembers?: TeamMentionMember[];
  displayName: string | null;
  avatarUrl?: string | null;
  isMember: boolean;
  role?: ChannelRole | null;
  personaName?: string | null;
  secondaryLabel?: string | null;
  ownerPubkey?: string | null;
  isAgent: boolean;
  isActiveAgent?: boolean;
  isManagedAgent?: boolean;
  isGlobalSearchResult?: boolean;
};

export function mentionCandidateLabel(candidate: MentionCandidate) {
  return (
    candidate.displayName ??
    (candidate.pubkey ? truncateNpub(candidate.pubkey) : "agent")
  );
}

/**
 * Mention chip label for a current room member. Reuses the members-sidebar
 * community-bots catalog overlay, then member / kind-0 / NIP-05 names, then
 * the truncated npub so unnamed members still appear in autocomplete.
 */
export function resolveMentionMemberDisplayName(input: {
  pubkey: string;
  memberDisplayName?: string | null;
  agentName?: string | null;
  profileDisplayName?: string | null;
  profileNip05?: string | null;
  communityBots?: ReadonlyArray<CommunityBot>;
}): string {
  const catalogOrMember = overlayCommunityBotDisplayName(
    input.memberDisplayName,
    input.pubkey,
    input.communityBots,
  );
  if (catalogOrMember) {
    return catalogOrMember;
  }

  const next =
    input.agentName?.trim() ||
    overlayCommunityBotDisplayName(
      input.profileDisplayName,
      input.pubkey,
      input.communityBots,
    ) ||
    input.profileNip05?.trim() ||
    null;
  return next || truncateNpub(input.pubkey);
}

/** Build the identity candidate for one current channel member. */
export function buildChannelMemberMentionCandidate(input: {
  member: Pick<ChannelMember, "pubkey" | "displayName" | "isAgent" | "role">;
  profile?: Pick<
    UserProfileSummary,
    "avatarUrl" | "displayName" | "isAgent" | "nip05Handle" | "ownerPubkey"
  > | null;
  agentName?: string | null;
  isDirectoryAgent?: boolean;
  managedAgentPersonaId?: string;
  linkedPersonaId?: string;
  personaName?: string | null;
  communityBots?: ReadonlyArray<CommunityBot>;
}): MentionCandidate & { pubkey: string } {
  const pubkey = normalizePubkey(input.member.pubkey);
  const profile = input.profile ?? null;
  return {
    kind: "identity",
    pubkey,
    displayName: resolveMentionMemberDisplayName({
      pubkey,
      memberDisplayName: input.member.displayName,
      agentName: input.agentName,
      profileDisplayName: profile?.displayName,
      profileNip05: profile?.nip05Handle,
      communityBots: input.communityBots,
    }),
    avatarUrl: profile?.avatarUrl ?? null,
    isMember: true,
    personaId: input.managedAgentPersonaId ?? input.linkedPersonaId,
    isAgent:
      input.member.isAgent === true ||
      profile?.isAgent === true ||
      input.member.role === "bot" ||
      input.isDirectoryAgent === true,
    ownerPubkey: profile?.ownerPubkey ?? null,
    personaName: input.personaName ?? null,
    role: input.member.role,
    secondaryLabel:
      profile?.displayName?.trim() && profile?.nip05Handle?.trim()
        ? profile.nip05Handle
        : null,
  };
}

export function globalSearchIdentityKey(candidate: MentionCandidate) {
  if (
    !candidate.isGlobalSearchResult ||
    candidate.isMember ||
    candidate.isAgent
  ) {
    return null;
  }

  const label = candidate.displayName?.trim().toLowerCase();
  if (!label) return null;

  const secondaryLabel = candidate.secondaryLabel?.trim().toLowerCase() ?? "";
  return `global-person:${label}:${secondaryLabel}`;
}

function findTeamMemberTarget(
  persona: AgentPersona,
  candidates: readonly MentionCandidate[],
): TeamMentionMember | null {
  const linked = candidates
    .filter(
      (candidate) =>
        candidate.kind !== "team" && candidate.personaId === persona.id,
    )
    .sort((left, right) => {
      const rank = (candidate: MentionCandidate) => {
        if (candidate.kind === "identity" && candidate.isMember) return 0;
        if (candidate.kind === "identity" && candidate.isManagedAgent) return 1;
        if (candidate.kind === "identity") return 2;
        return 3;
      };
      return rank(left) - rank(right);
    })[0];

  if (linked) {
    return {
      displayName: linked.displayName?.trim() || persona.displayName,
      kind: linked.kind === "identity" ? "identity" : "persona",
      personaId: linked.personaId,
      pubkey: linked.pubkey,
    };
  }

  return persona.isActive
    ? {
        displayName: persona.displayName,
        kind: "persona",
        personaId: persona.id,
      }
    : null;
}

/** Build autocomplete entries for editable, locally owned teams. */
export function buildTeamMentionCandidates(
  teams: readonly AgentTeam[],
  personas: AgentPersona[],
  candidates: readonly MentionCandidate[],
): MentionCandidate[] {
  return teams.flatMap((team) => {
    if (team.isBuiltin || !team.name.trim()) return [];

    const resolution = resolveTeamPersonas(team, personas);
    if (!resolution.isUsable) return [];

    const teamMembers = resolution.resolvedPersonas
      .map((persona) => findTeamMemberTarget(persona, candidates))
      .filter((member): member is TeamMentionMember => member !== null);
    if (teamMembers.length !== resolution.resolvedPersonas.length) return [];

    const mentionNames = new Map<string, TeamMentionMember>();
    for (const member of teamMembers) {
      const mentionName = member.displayName.trim().toLowerCase();
      const previous = mentionNames.get(mentionName);
      // Exact-key members can reserve distinct labels at selection time. A
      // persona without a key cannot yet be disambiguated that way.
      if (previous && (!previous.pubkey || !member.pubkey)) return [];
      mentionNames.set(mentionName, member);
    }

    return [
      {
        kind: "team" as const,
        teamId: team.id,
        teamMembers,
        displayName: team.name.trim(),
        isMember: false,
        isAgent: true,
      },
    ];
  });
}

export function formatTeamMention(
  teamName: string,
  members: readonly TeamMentionMember[],
) {
  return `${teamName}(${members.map((member) => `@${member.displayName}`).join(" ")}) `;
}

/**
 * Mention discovery gate. Archive always wins — even for current-room members
 * and catalog `bot` identities that PR #16 keeps mentionable otherwise.
 * Catalog overlay must not resurrect an archived pubkey.
 */
export function shouldAcceptMentionIdentityCandidate(input: {
  isArchived: boolean;
  isMember: boolean;
  hideAgent: boolean;
}): boolean {
  if (input.isArchived) {
    return false;
  }
  if (input.isMember) {
    return true;
  }
  return !input.hideAgent;
}
