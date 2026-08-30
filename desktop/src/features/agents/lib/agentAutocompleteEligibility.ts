import type { Channel, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function isAgentDirectoryReady({
  data,
  error,
}: {
  data: unknown;
  error: unknown;
}) {
  // A successful cached directory remains suitable for autocomplete during a
  // refetch. Sending still re-fetches and fails closed at its authorization
  // boundary, so suggestions are hints rather than permission to send.
  return data !== undefined && error === null;
}

export function getSharedChannelIds(channels: readonly Channel[] | undefined) {
  return new Set(
    (channels ?? [])
      .filter((channel) => channel.isMember && channel.archivedAt === null)
      .map((channel) => channel.id),
  );
}

export function relayAgentIsSharedWithUser(
  agent: Pick<
    RelayAgent,
    "channelIds" | "ownerPubkey" | "respondTo" | "respondToAllowlist"
  >,
  sharedChannelIds: ReadonlySet<string>,
  currentPubkey?: string | null,
) {
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;

  if (
    agent.respondTo === "owner-only" &&
    normalizedCurrentPubkey &&
    agent.ownerPubkey
  ) {
    return normalizePubkey(agent.ownerPubkey) === normalizedCurrentPubkey;
  }

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
  agent: Pick<
    RelayAgent,
    "channelIds" | "ownerPubkey" | "respondTo" | "respondToAllowlist"
  >,
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

export function getMentionableAgentPubkeys({
  currentPubkey,
  eligibilityScope,
  managedAgentPubkeys,
  relayAgents,
  sharedChannelIds,
}: {
  currentPubkey?: string | null;
  eligibilityScope: AgentEligibilityScope;
  managedAgentPubkeys: Iterable<string>;
  relayAgents: readonly RelayAgent[] | undefined;
  sharedChannelIds: ReadonlySet<string>;
}) {
  const pubkeys = new Set(
    [...managedAgentPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );

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

export type AgentMentionAdmission = "allow" | "deny" | "unknown";

export function getAgentMentionAdmission({
  isAgent,
  pubkey,
  mentionableAgentPubkeys,
  directoryReady,
}: {
  isAgent: boolean;
  pubkey: string;
  mentionableAgentPubkeys: ReadonlySet<string>;
  directoryReady: boolean;
}): AgentMentionAdmission {
  if (!isAgent) return "allow";
  if (!directoryReady) return "unknown";

  return mentionableAgentPubkeys.has(normalizePubkey(pubkey))
    ? "allow"
    : "deny";
}

export function shouldHideAgentFromMentions({
  isAgent,
  pubkey,
  mentionableAgentPubkeys,
  directoryReady = true,
}: {
  isAgent: boolean;
  pubkey: string;
  mentionableAgentPubkeys: ReadonlySet<string>;
  directoryReady?: boolean;
}) {
  return (
    getAgentMentionAdmission({
      isAgent,
      pubkey,
      mentionableAgentPubkeys,
      directoryReady,
    }) !== "allow"
  );
}

export function getAgentIdentityPubkeys({
  managedAgentPubkeys,
  relayAgents,
  members,
  profileIsAgent,
}: {
  managedAgentPubkeys: ReadonlySet<string>;
  relayAgents: readonly { pubkey: string }[];
  members: readonly {
    pubkey: string;
    isAgent?: boolean;
    role?: string | null;
  }[];
  profileIsAgent: (pubkey: string) => boolean;
}) {
  return new Set([
    ...managedAgentPubkeys,
    ...relayAgents.map(({ pubkey }) => normalizePubkey(pubkey)),
    ...members
      .filter(
        (member) =>
          member.isAgent === true ||
          member.role === "bot" ||
          profileIsAgent(normalizePubkey(member.pubkey)),
      )
      .map(({ pubkey }) => normalizePubkey(pubkey)),
  ]);
}

export function getAdmittedAgentPubkeys(
  candidates: readonly { pubkey?: string; isAgent?: boolean }[],
) {
  return new Set(
    candidates.flatMap((candidate) =>
      candidate.isAgent && candidate.pubkey
        ? [normalizePubkey(candidate.pubkey)]
        : [],
    ),
  );
}

export function rememberSelectedAgentPubkeys(
  target: Set<string>,
  selected: readonly { pubkey?: string; isAgent?: boolean }[],
  selectionIsAgent: boolean,
) {
  for (const candidate of selected) {
    if (candidate.pubkey && (selectionIsAgent || candidate.isAgent === true)) {
      target.add(normalizePubkey(candidate.pubkey));
    }
  }
}

export function filterAdmittedMentionPubkeys(
  pubkeys: readonly string[],
  agentIdentityPubkeys: ReadonlySet<string>,
  admittedAgentPubkeys: ReadonlySet<string>,
) {
  return pubkeys.filter((pubkey) => {
    const normalized = normalizePubkey(pubkey);
    return (
      !agentIdentityPubkeys.has(normalized) ||
      admittedAgentPubkeys.has(normalized)
    );
  });
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
