import type { Channel, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function getSharedChannelIds(channels: readonly Channel[] | undefined) {
  return new Set(
    (channels ?? [])
      .filter((channel) => channel.isMember && channel.archivedAt === null)
      .map((channel) => channel.id),
  );
}

export function relayAgentIsSharedWithUser(
  agent: Pick<RelayAgent, "channelIds" | "respondTo" | "respondToAllowlist">,
  sharedChannelIds: ReadonlySet<string>,
  currentPubkey?: string | null,
) {
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;

  if (agent.respondTo === "allowlist" && normalizedCurrentPubkey) {
    return agent.respondToAllowlist
      .map((pubkey) => normalizePubkey(pubkey))
      .includes(normalizedCurrentPubkey);
  }

  return (
    agent.respondTo === "anyone" &&
    agent.channelIds.some((channelId) => sharedChannelIds.has(channelId))
  );
}

export function relayAgentCanRespondInChannel(
  agent: Pick<RelayAgent, "channelIds" | "respondTo" | "respondToAllowlist">,
  channelId: string,
  currentPubkey?: string | null,
) {
  return (
    agent.channelIds.includes(channelId) &&
    relayAgentIsSharedWithUser(agent, new Set([channelId]), currentPubkey)
  );
}

export type AgentEligibilityScope =
  | { type: "community" }
  | { type: "channel"; channelId: string }
  | { type: "managed-only" };

/**
 * Returns agent identities that are both present in the active channel and
 * cryptographically owned by the viewer. The caller must supply ownerPubkey
 * values verified from NIP-OA profile events; display names and local persona
 * cards are never ownership evidence.
 *
 * This set may override normal relay response policy only for channel mention
 * routing after the relay directory has loaded successfully. It must not be
 * used to seed community search or direct messages.
 */
export function getChannelOwnedAgentPubkeys({
  channelMembers,
  currentPubkey,
  profiles,
}: {
  channelMembers:
    | readonly {
        pubkey: string;
        role?: string | null;
        isAgent?: boolean;
      }[]
    | undefined;
  currentPubkey?: string | null;
  profiles:
    | Readonly<
        Record<string, { ownerPubkey?: string | null; isAgent?: boolean }>
      >
    | undefined;
}) {
  const ownedAgentPubkeys = new Set<string>();
  if (!currentPubkey) return ownedAgentPubkeys;

  const normalizedCurrentPubkey = normalizePubkey(currentPubkey);
  for (const member of channelMembers ?? []) {
    const pubkey = normalizePubkey(member.pubkey);
    const profile = profiles?.[pubkey];
    const isAgent =
      member.role === "bot" ||
      member.isAgent === true ||
      profile?.isAgent === true;
    if (
      isAgent &&
      profile?.ownerPubkey &&
      normalizePubkey(profile.ownerPubkey) === normalizedCurrentPubkey
    ) {
      ownedAgentPubkeys.add(pubkey);
    }
  }

  return ownedAgentPubkeys;
}

function relayAgentIsHeartbeatOnly(agent: Pick<RelayAgent, "respondTo">) {
  return agent.respondTo === "nobody";
}

export function getMentionableAgentPubkeys({
  channelOwnedAgentPubkeys,
  currentPubkey,
  eligibilityScope,
  managedAgentPubkeys,
  relayAgents,
  sharedChannelIds,
}: {
  channelOwnedAgentPubkeys?: Iterable<string>;
  currentPubkey?: string | null;
  eligibilityScope: AgentEligibilityScope;
  managedAgentPubkeys: Iterable<string>;
  relayAgents: readonly RelayAgent[] | undefined;
  sharedChannelIds: ReadonlySet<string>;
}) {
  const pubkeys = new Set(
    [...managedAgentPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );

  // A missing directory is not equivalent to an empty one: until a successful
  // response proves there is no `nobody` entry, remote-owner admission must
  // fail closed. Locally managed identities remain available above.
  if (eligibilityScope.type === "channel" && relayAgents !== undefined) {
    const heartbeatOnlyPubkeys = new Set(
      (relayAgents ?? [])
        .filter(relayAgentIsHeartbeatOnly)
        .map((agent) => normalizePubkey(agent.pubkey)),
    );
    for (const pubkey of channelOwnedAgentPubkeys ?? []) {
      const normalized = normalizePubkey(pubkey);
      if (!heartbeatOnlyPubkeys.has(normalized)) {
        pubkeys.add(normalized);
      }
    }
  }

  for (const agent of relayAgents ?? []) {
    const isAllowed =
      eligibilityScope.type === "managed-only"
        ? false
        : eligibilityScope.type === "community"
          ? relayAgentIsSharedWithUser(agent, sharedChannelIds, currentPubkey)
          : relayAgentCanRespondInChannel(
              agent,
              eligibilityScope.channelId,
              currentPubkey,
            );
    if (isAllowed) {
      pubkeys.add(normalizePubkey(agent.pubkey));
    }
  }

  return pubkeys;
}

export function isAgentIdentityInAllowedList(
  candidate: { isAgent?: boolean; pubkey: string },
  allowedAgentPubkeys: ReadonlySet<string>,
) {
  return (
    candidate.isAgent !== true ||
    allowedAgentPubkeys.has(normalizePubkey(candidate.pubkey))
  );
}

export function shouldHideAgentFromMentions({
  isAgent,
  isMember,
  pubkey,
  mentionableAgentPubkeys,
  directoryAgentPubkeys,
}: {
  isAgent: boolean;
  isMember: boolean;
  pubkey: string;
  mentionableAgentPubkeys: ReadonlySet<string>;
  directoryAgentPubkeys: ReadonlySet<string>;
}) {
  if (!isAgent) return false;
  const normalized = normalizePubkey(pubkey);
  // Invocable => always show.
  if (mentionableAgentPubkeys.has(normalized)) return false;
  // Non-member, non-invocable => hide (preserves prior behavior).
  if (!isMember) return true;
  // Member (Option B): hide only when we have an explicit not-invocable
  // signal — a relay directory (kind:10100) entry that excludes us.
  // Unknown invocability (not in directory) => show.
  //
  // NOTE: this assumes `directoryAgentPubkeys` and `mentionableAgentPubkeys`
  // share the same source query (`relayAgentsQuery.data`), so directory
  // presence without membership in `mentionableAgentPubkeys` is a real
  // explicit-exclusion signal. If a future change sources the directory set
  // from a different query, an agent that's directory-present but whose
  // mentionability is still loading could be hidden prematurely — keep the
  // two sets derived from the same query.
  return directoryAgentPubkeys.has(normalized);
}

export function isAgentMentionChannelType(type?: string | null) {
  return type === "stream" || type === "forum";
}

export function uniqueAutocompleteLabels(
  candidates: readonly AgentAutocompleteCandidate[],
) {
  const unique = new Map<string, string>();
  for (const candidate of candidates) {
    for (const label of [
      candidate.displayName,
      candidate.personaName,
      candidate.secondaryLabel,
    ]) {
      const trimmed = label?.trim();
      if (trimmed && !unique.has(trimmed.toLowerCase())) {
        unique.set(trimmed.toLowerCase(), trimmed);
      }
    }
  }
  return [...unique.values()];
}

export function filterCachedAgentSuggestions<
  T extends {
    isAgent?: boolean;
    pubkey?: string;
  },
>(
  suggestions: readonly T[],
  currentCandidates: readonly AgentAutocompleteCandidate[],
) {
  const admittedAgentPubkeys = new Set(
    currentCandidates.flatMap((candidate) =>
      candidate.isAgent && candidate.pubkey
        ? [normalizePubkey(candidate.pubkey)]
        : [],
    ),
  );
  return suggestions.filter(
    (suggestion) =>
      !suggestion.isAgent ||
      !suggestion.pubkey ||
      admittedAgentPubkeys.has(normalizePubkey(suggestion.pubkey)),
  );
}

type AgentAutocompleteCandidate = {
  pubkey?: string;
  displayName?: string | null;
  personaName?: string | null;
  secondaryLabel?: string | null;
  ownerPubkey?: string | null;
  isAgent?: boolean;
  isManagedAgent?: boolean;
  isMember?: boolean;
  personaId?: string | null;
};

function agentIdentityKey<T extends AgentAutocompleteCandidate>(candidate: T) {
  if (candidate.isAgent !== true || !candidate.pubkey) {
    return null;
  }

  // Pubkeys—not persona metadata or a display name—are agent identities.
  // A persona may be installed more than once, and an owner may intentionally
  // create multiple same-named agents. Collapsing either case makes one agent
  // impossible to choose from autocomplete.
  return `pubkey:${normalizePubkey(candidate.pubkey)}`;
}

function agentCandidateRank<T extends AgentAutocompleteCandidate>(
  candidate: T,
  preferredPubkeys: ReadonlySet<string>,
) {
  const pubkey = candidate.pubkey ? normalizePubkey(candidate.pubkey) : null;

  return [
    candidate.isMember === true ? 0 : 1,
    pubkey && preferredPubkeys.has(pubkey) ? 0 : 1,
    candidate.isManagedAgent === true ? 0 : 1,
    candidate.personaId ? 0 : 1,
  ];
}

function isPreferredAgentCandidate<T extends AgentAutocompleteCandidate>(
  next: T,
  current: T,
  preferredPubkeys: ReadonlySet<string>,
) {
  const nextRank = agentCandidateRank(next, preferredPubkeys);
  const currentRank = agentCandidateRank(current, preferredPubkeys);

  for (let index = 0; index < nextRank.length; index++) {
    if (nextRank[index] !== currentRank[index]) {
      return nextRank[index] < currentRank[index];
    }
  }

  return false;
}

export function coalesceAutocompleteCandidatesByKey<T>(
  candidates: readonly T[],
  getKey: (candidate: T) => string | null,
) {
  const output: T[] = [];
  const indexesByKey = new Map<string, number>();

  for (const candidate of candidates) {
    const key = getKey(candidate);
    if (!key) {
      output.push(candidate);
      continue;
    }

    if (!indexesByKey.has(key)) {
      indexesByKey.set(key, output.length);
      output.push(candidate);
    }
  }

  return output;
}

export function coalesceAgentAutocompleteCandidates<
  T extends AgentAutocompleteCandidate,
>(
  candidates: readonly T[],
  {
    currentPubkey: _currentPubkey,
    getLabel: _getLabel,
    preferredPubkeys = new Set(),
  }: {
    currentPubkey?: string | null;
    getLabel: (candidate: T) => string | null | undefined;
    preferredPubkeys?: ReadonlySet<string>;
  },
) {
  const output: T[] = [];
  const indexesByKey = new Map<string, number>();

  for (const candidate of candidates) {
    const key = agentIdentityKey(candidate);
    if (!key) {
      output.push(candidate);
      continue;
    }

    const currentIndex = indexesByKey.get(key);
    if (currentIndex === undefined) {
      indexesByKey.set(key, output.length);
      output.push(candidate);
      continue;
    }

    if (
      isPreferredAgentCandidate(
        candidate,
        output[currentIndex],
        preferredPubkeys,
      )
    ) {
      output[currentIndex] = candidate;
    }
  }

  return output;
}
