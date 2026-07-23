import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type {
  AgentPersona,
  AgentTeam,
  ChannelRole,
  UserSearchResult,
} from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";

export type TeamMentionMember = {
  displayName: string;
  kind: "identity" | "persona";
  personaId?: string;
  pubkey?: string;
};

export type CategoryMentionId = "agents" | "people";

export type MentionCandidate = {
  kind: "identity" | "persona" | "team" | "category";
  pubkey?: string;
  personaId?: string;
  teamId?: string;
  categoryId?: CategoryMentionId;
  teamMembers?: TeamMentionMember[];
  displayName: string | null;
  avatarUrl?: string | null;
  isMember: boolean;
  role?: ChannelRole | null;
  personaName?: string | null;
  secondaryLabel?: string | null;
  ownerPubkey?: string | null;
  isAgent: boolean;
  isManagedAgent?: boolean;
  isGlobalSearchResult?: boolean;
};

export function mentionCandidateLabel(candidate: MentionCandidate) {
  return (
    candidate.displayName ??
    (candidate.pubkey ? truncatePubkey(candidate.pubkey) : "agent")
  );
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

    const mentionNames = new Set<string>();
    for (const member of teamMembers) {
      const mentionName = member.displayName.trim().toLowerCase();
      if (mentionNames.has(mentionName)) return [];
      mentionNames.add(mentionName);
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
 * Build the virtual `@agents` / `@people` autocomplete entries from the
 * channel's current membership. Selecting one unfurls into individual
 * mentions of every matching member — plain `@Name` text and standard
 * per-recipient tags, so recipients and relays see ordinary mentions.
 *
 * Mirrors team-mention safety rules: a category is omitted entirely when
 * two of its members share a display name (the mention map is keyed by
 * name, so a collision would silently drop one of them). Members without
 * a display name cannot be mentioned by name and are skipped. `@people`
 * excludes the current user — you don't need to notify yourself.
 */
export function buildCategoryMentionCandidates(
  candidates: readonly MentionCandidate[],
  currentPubkey?: string | null,
): MentionCandidate[] {
  const groups: Array<{
    categoryId: CategoryMentionId;
    matches: (candidate: MentionCandidate) => boolean;
  }> = [
    {
      categoryId: "agents",
      matches: (candidate) => candidate.isAgent === true,
    },
    {
      categoryId: "people",
      matches: (candidate) =>
        candidate.isAgent !== true &&
        (!currentPubkey || candidate.pubkey !== currentPubkey),
    },
  ];

  return groups.flatMap(({ categoryId, matches }) => {
    const members: TeamMentionMember[] = [];
    const mentionNames = new Set<string>();

    for (const candidate of candidates) {
      if (candidate.kind !== "identity") continue;
      if (!candidate.isMember || !candidate.pubkey) continue;
      if (!matches(candidate)) continue;

      const displayName = candidate.displayName?.trim();
      if (!displayName) continue;

      const mentionName = displayName.toLowerCase();
      if (mentionNames.has(mentionName)) return [];
      mentionNames.add(mentionName);

      members.push({
        displayName,
        kind: "identity",
        personaId: candidate.personaId,
        pubkey: candidate.pubkey,
      });
    }

    if (members.length === 0) return [];

    return [
      {
        kind: "category" as const,
        categoryId,
        teamMembers: members,
        displayName: categoryId,
        isMember: false,
        isAgent: categoryId === "agents",
      },
    ];
  });
}

export function formatCategoryMention(members: readonly TeamMentionMember[]) {
  return `${members.map((member) => `@${member.displayName}`).join(" ")} `;
}

/**
 * Team and category suggestions share the unfurl path: both expand into the
 * individually tracked mentions carried in `teamMembers`.
 */
export function groupMentionMembers(suggestion: {
  kind?: "identity" | "persona" | "team" | "category";
  teamMembers?: TeamMentionMember[];
}): TeamMentionMember[] | null {
  return (suggestion.kind === "team" || suggestion.kind === "category") &&
    suggestion.teamMembers
    ? suggestion.teamMembers
    : null;
}

export function formatGroupMention(
  suggestion: {
    kind?: "identity" | "persona" | "team" | "category";
    displayName: string;
  },
  members: readonly TeamMentionMember[],
) {
  return suggestion.kind === "category"
    ? formatCategoryMention(members)
    : formatTeamMention(suggestion.displayName, members);
}

export function formatSearchUserDisplayName(user: UserSearchResult) {
  return user.displayName?.trim() || user.nip05Handle?.trim() || null;
}

export function formatSearchUserSecondaryLabel(user: UserSearchResult) {
  const displayName = user.displayName?.trim();
  const nip05Handle = user.nip05Handle?.trim();
  if (displayName && nip05Handle) {
    return nip05Handle;
  }
  return null;
}
