import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type {
  AgentPersona,
  AgentTeam,
  ChannelRole,
  UserSearchResult,
} from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";

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

export type MentionGroupMember = {
  displayName: string;
  kind: "identity" | "persona";
  personaId?: string;
  pubkey?: string;
};

export type TeamMentionMember = MentionGroupMember;

export type MentionCandidate = {
  kind: "identity" | "persona" | "team" | "channel";
  pubkey?: string;
  personaId?: string;
  teamId?: string;
  teamMembers?: TeamMentionMember[];
  channelMembers?: MentionGroupMember[];
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

/** Build the safe client-side expansion for the current channel's members. */
export function buildChannelMentionCandidate(
  candidates: readonly MentionCandidate[],
  currentPubkey: string | null,
): MentionCandidate | null {
  const channelMembers = candidates.flatMap((candidate) => {
    if (
      candidate.kind !== "identity" ||
      !candidate.isMember ||
      !candidate.pubkey ||
      candidate.pubkey === currentPubkey
    ) {
      return [];
    }

    const displayName = candidate.displayName?.trim();
    return displayName
      ? [{ displayName, kind: "identity" as const, pubkey: candidate.pubkey }]
      : [];
  });
  const names = new Set<string>();
  if (
    channelMembers.length === 0 ||
    channelMembers.some((member) => {
      const normalized = member.displayName.toLowerCase();
      if (names.has(normalized)) return true;
      names.add(normalized);
      return false;
    })
  ) {
    return null;
  }

  return {
    kind: "channel",
    channelMembers,
    displayName: "everyone",
    isAgent: false,
    isMember: false,
  };
}

export function formatChannelMention(members: readonly MentionGroupMember[]) {
  return `${members.map((member) => `@${member.displayName}`).join(" ")} `;
}
