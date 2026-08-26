import { resolveTeamPersonas } from "@/features/agents/lib/teamPersonas";
import type {
  AgentPersona,
  AgentTeam,
  ChannelRole,
  ChannelType,
  UserSearchResult,
} from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";
import {
  MENTION_SCOPE_CHANNEL,
  MENTION_SCOPE_HERE,
} from "./globalMentions.mjs";

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
  kind: "identity" | "persona" | "team" | "scope";
  pubkey?: string;
  personaId?: string;
  teamId?: string;
  teamMembers?: TeamMentionMember[];
  /** For a `"scope"` candidate: the global-mention scope it inserts. */
  scope?: "channel" | "here";
  /** For a `"scope"` candidate: the one-line explanation shown beneath it. */
  description?: string;
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

/**
 * The `@channel` / `@here` autocomplete entries.
 *
 * These are synthetic — they carry no pubkey or persona. Selecting one just
 * inserts the literal `@channel ` / `@here ` text; the send path detects that
 * text (see `detectMentionScope` in `hooks.ts`) and attaches the
 * `mention-scope` tag. `displayName` is the word inserted after the `@`.
 */
export const GLOBAL_MENTION_SCOPES: ReadonlyArray<{
  scope: "channel" | "here";
  displayName: string;
  description: string;
}> = [
  {
    scope: MENTION_SCOPE_CHANNEL,
    displayName: "channel",
    description: "Notify everyone in this channel",
  },
  {
    scope: MENTION_SCOPE_HERE,
    displayName: "here",
    description: "Notify members who are online",
  },
];

/**
 * Build the `@channel` / `@here` candidates for a channel.
 *
 * Global mentions only make sense in a channel, never a DM, so this returns
 * nothing outside a channel. Membership/permission is deliberately not gated:
 * the app lets everyone use these scopes.
 */
export function buildGlobalMentionScopeCandidates(
  channelType: ChannelType | null | undefined,
): MentionCandidate[] {
  if (channelType == null || channelType === "dm") {
    return [];
  }
  return GLOBAL_MENTION_SCOPES.map((entry) => ({
    kind: "scope" as const,
    scope: entry.scope,
    displayName: entry.displayName,
    description: entry.description,
    isMember: false,
    isAgent: false,
  }));
}

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
