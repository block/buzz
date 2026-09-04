import {
  coalesceAgentAutocompleteCandidates,
  coalesceAutocompleteCandidatesByKey,
  shouldHideAgentFromMentions,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  AgentPersona,
  ChannelMember,
  ManagedAgent,
  RelayAgent,
  UserSearchResult,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  formatSearchUserDisplayName,
  formatSearchUserSecondaryLabel,
  globalSearchIdentityKey,
  type MentionCandidate,
  mentionCandidateLabel,
} from "./mentionCandidates";

/** Directories and rosters the mention picker merges into one candidate list. */
export type BuildMentionCandidatesInput = {
  activeAgentPubkeys: ReadonlySet<string>;
  activePersonaById: ReadonlyMap<string, AgentPersona>;
  /** Already narrowed to `isActive` personas. */
  activePersonas: readonly AgentPersona[];
  canSearchGlobalUsers: boolean;
  currentPubkey: string | null;
  isArchived: (pubkey: string) => boolean;
  managedAgentDirectoryReady: boolean;
  managedAgentNamesByPubkey: ReadonlyMap<string, string>;
  managedAgentPersonaIds: ReadonlySet<string>;
  managedAgentPersonaIdsByPubkey: ReadonlyMap<string, string>;
  managedAgents: readonly ManagedAgent[] | undefined;
  memberPubkeys: ReadonlySet<string>;
  members: readonly ChannelMember[] | undefined;
  mentionChannelId: string | null;
  mentionableAgentPubkeys: ReadonlySet<string>;
  personaNameByPubkey: ReadonlyMap<string, string>;
  profiles: UserProfileLookup | undefined;
  relayAgentDirectoryReady: boolean;
  relayAgentNamesByPubkey: ReadonlyMap<string, string>;
  relayAgents: readonly RelayAgent[] | undefined;
  userSearchResults: readonly UserSearchResult[];
};

/**
 * Merge the channel roster, agent directories, global people search, and
 * standalone personas into the deduplicated candidate list the mention
 * autocomplete ranks. Archived identities and agents the viewer may not
 * mention are dropped; identities appearing in several sources are coalesced
 * into a single entry that keeps the richest field from each.
 */
export function buildMentionCandidates({
  activeAgentPubkeys,
  activePersonaById,
  activePersonas,
  canSearchGlobalUsers,
  currentPubkey,
  isArchived,
  managedAgentDirectoryReady,
  managedAgentNamesByPubkey,
  managedAgentPersonaIds,
  managedAgentPersonaIdsByPubkey,
  managedAgents,
  memberPubkeys,
  members,
  mentionChannelId,
  mentionableAgentPubkeys,
  personaNameByPubkey,
  profiles,
  relayAgentDirectoryReady,
  relayAgentNamesByPubkey,
  relayAgents,
  userSearchResults,
}: BuildMentionCandidatesInput): MentionCandidate[] {
  const candidatesByPubkey = new Map<string, MentionCandidate>();
  const addCandidate = (candidate: MentionCandidate & { pubkey: string }) => {
    const pubkey = normalizePubkey(candidate.pubkey);
    if (isArchived(pubkey)) {
      return;
    }
    if (
      shouldHideAgentFromMentions({
        isAgent: candidate.isAgent === true,
        pubkey,
        mentionableAgentPubkeys,
        directoryReady:
          candidate.isManagedAgent === true
            ? managedAgentDirectoryReady
            : relayAgentDirectoryReady,
      })
    ) {
      return;
    }
    const current = candidatesByPubkey.get(pubkey);
    if (!current) {
      candidatesByPubkey.set(pubkey, { ...candidate, pubkey });
      return;
    }
    candidatesByPubkey.set(pubkey, {
      ...current,
      avatarUrl: current.avatarUrl ?? candidate.avatarUrl ?? null,
      displayName:
        current.isAgent && !candidate.isAgent
          ? current.displayName
          : candidate.isAgent && !current.isAgent
            ? (candidate.displayName ?? current.displayName)
            : (current.displayName ?? candidate.displayName),
      isAgent: current.isAgent || candidate.isAgent,
      isActiveAgent: current.isActiveAgent || candidate.isActiveAgent,
      isMember: current.isMember || candidate.isMember,
      personaId: current.personaId ?? candidate.personaId,
      personaName: current.personaName ?? candidate.personaName ?? null,
      role: current.role ?? candidate.role ?? null,
      secondaryLabel:
        current.secondaryLabel ?? candidate.secondaryLabel ?? null,
      ownerPubkey:
        current.ownerPubkey ??
        candidate.ownerPubkey ??
        (candidate.isAgent && candidate.pubkey
          ? profiles?.[pubkey]?.ownerPubkey
          : null) ??
        null,
      isManagedAgent: current.isManagedAgent || candidate.isManagedAgent,
    });
  };
  for (const member of members ?? []) {
    const pubkey = normalizePubkey(member.pubkey);
    const linkedPersonaId = activePersonaById.has(pubkey) ? pubkey : undefined;
    const agentName =
      managedAgentNamesByPubkey.get(pubkey) ??
      relayAgentNamesByPubkey.get(pubkey) ??
      null;
    const profile = profiles?.[pubkey] ?? null;
    addCandidate({
      kind: "identity",
      pubkey,
      displayName:
        member.displayName?.trim() ||
        agentName ||
        profile?.displayName?.trim() ||
        profile?.nip05Handle?.trim() ||
        null,
      avatarUrl: profile?.avatarUrl ?? null,
      isMember: true,
      personaId: managedAgentPersonaIdsByPubkey.get(pubkey) ?? linkedPersonaId,
      isAgent:
        member.isAgent === true ||
        profile?.isAgent === true ||
        member.role === "bot" ||
        managedAgentNamesByPubkey.has(pubkey) ||
        relayAgentNamesByPubkey.has(pubkey),
      isActiveAgent: activeAgentPubkeys.has(pubkey),
      isManagedAgent: managedAgentNamesByPubkey.has(pubkey),
      ownerPubkey: profile?.ownerPubkey ?? null,
      personaName: personaNameByPubkey.get(pubkey) ?? null,
      role: member.role,
      secondaryLabel:
        profile?.displayName?.trim() && profile?.nip05Handle?.trim()
          ? profile.nip05Handle
          : null,
    });
  }
  for (const agent of relayAgents ?? []) {
    const pubkey = normalizePubkey(agent.pubkey);
    addCandidate({
      kind: "identity",
      pubkey,
      displayName: agent.name,
      // Prefer the active channel's signed roster. The relay-agent directory
      // is filtered by access policy, so its channel ids can legitimately omit
      // a room where this identity is already a member.
      isMember:
        memberPubkeys.has(pubkey) ||
        (mentionChannelId !== null &&
          agent.channelIds.includes(mentionChannelId)),
      personaId:
        managedAgentPersonaIdsByPubkey.get(pubkey) ??
        (activePersonaById.has(pubkey) ? pubkey : undefined),
      ownerPubkey: agent.ownerPubkey,
      isAgent: true,
      isActiveAgent: agent.status === "online" || agent.status === "away",
    });
  }
  for (const agent of managedAgents ?? []) {
    const pubkey = normalizePubkey(agent.pubkey);
    addCandidate({
      kind: "identity",
      pubkey,
      displayName: agent.name,
      isMember: memberPubkeys.has(pubkey),
      isAgent: true,
      isActiveAgent: agent.status === "running" || agent.status === "deployed",
      isManagedAgent: true,
      personaId: agent.personaId ?? undefined,
      personaName:
        personaNameByPubkey.get(normalizePubkey(agent.pubkey)) ?? null,
      ownerPubkey: currentPubkey,
    });
  }
  if (canSearchGlobalUsers) {
    for (const user of userSearchResults) {
      const pubkey = normalizePubkey(user.pubkey);
      addCandidate({
        kind: "identity",
        pubkey,
        displayName: formatSearchUserDisplayName(user),
        avatarUrl: user.avatarUrl ?? null,
        personaId:
          managedAgentPersonaIdsByPubkey.get(pubkey) ??
          (activePersonaById.has(pubkey) ? pubkey : undefined),
        isMember: false,
        isAgent:
          user.isAgent ||
          managedAgentNamesByPubkey.has(pubkey) ||
          relayAgentNamesByPubkey.has(pubkey),
        personaName: personaNameByPubkey.get(pubkey) ?? null,
        secondaryLabel: formatSearchUserSecondaryLabel(user),
        ownerPubkey: user.ownerPubkey ?? null,
        isGlobalSearchResult: true,
        isManagedAgent: managedAgentNamesByPubkey.has(pubkey),
      });
    }
  }
  const personaCandidates: MentionCandidate[] = activePersonas
    .filter((persona) => !managedAgentPersonaIds.has(persona.id))
    .map((persona) => ({
      kind: "persona" as const,
      personaId: persona.id,
      displayName: persona.displayName,
      avatarUrl: persona.avatarUrl,
      isMember: false,
      isAgent: true,
    }))
    .filter((candidate) => candidate.displayName.trim().length > 0);
  return coalesceAgentAutocompleteCandidates(
    coalesceAutocompleteCandidatesByKey(
      [...candidatesByPubkey.values(), ...personaCandidates],
      globalSearchIdentityKey,
    ),
    {
      currentPubkey,
      getLabel: mentionCandidateLabel,
      preferredPubkeys: memberPubkeys,
    },
  );
}
