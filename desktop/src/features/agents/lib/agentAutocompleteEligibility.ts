import type { Channel, ManagedAgent, RelayAgent } from "@/shared/api/types";
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

export function getMentionableAgentPubkeys({
  currentPubkey,
  managedAgentPubkeys,
  relayAgents,
  sharedChannelIds,
}: {
  currentPubkey?: string | null;
  managedAgentPubkeys: Iterable<string>;
  relayAgents: readonly RelayAgent[] | undefined;
  sharedChannelIds: ReadonlySet<string>;
}) {
  const pubkeys = new Set(
    [...managedAgentPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );

  for (const agent of relayAgents ?? []) {
    if (relayAgentIsSharedWithUser(agent, sharedChannelIds, currentPubkey)) {
      pubkeys.add(normalizePubkey(agent.pubkey));
    }
  }

  return pubkeys;
}

export function isActiveManagedAgent(agent: Pick<ManagedAgent, "status">) {
  return agent.status === "running" || agent.status === "deployed";
}

export function getActiveManagedAgentMetadata(
  agents:
    | readonly Pick<ManagedAgent, "pubkey" | "personaId" | "status">[]
    | undefined,
) {
  const activeAgents = (agents ?? []).filter(isActiveManagedAgent);
  return {
    personaIds: new Set(
      activeAgents
        .map((agent) => agent.personaId)
        .filter((personaId): personaId is string => Boolean(personaId)),
    ),
    pubkeys: new Set(
      activeAgents.map((agent) => normalizePubkey(agent.pubkey)),
    ),
  };
}

export function getPreferredAgentPubkeys({
  managedAgents,
  relayAgents,
}: {
  managedAgents: readonly Pick<ManagedAgent, "pubkey" | "status">[] | undefined;
  relayAgents: readonly Pick<RelayAgent, "pubkey" | "status">[] | undefined;
}) {
  return new Set([
    ...(managedAgents ?? [])
      .filter(isActiveManagedAgent)
      .map((agent) => normalizePubkey(agent.pubkey)),
    ...(relayAgents ?? [])
      .filter((agent) => agent.status !== "offline")
      .map((agent) => normalizePubkey(agent.pubkey)),
  ]);
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

type AgentAutocompleteCandidate = {
  pubkey?: string;
  displayName?: string | null;
  ownerPubkey?: string | null;
  isAgent?: boolean;
  isManagedAgent?: boolean;
  isMember?: boolean;
  kind?: "identity" | "persona" | "team";
  personaId?: string | null;
};

function normalizeLabel(label: string | null | undefined) {
  return label?.trim().toLowerCase() || null;
}

function agentIdentityKey<T extends AgentAutocompleteCandidate>(
  candidate: T,
  currentPubkey: string | null | undefined,
  getLabel: (candidate: T) => string | null | undefined,
) {
  if (candidate.isAgent !== true) {
    return null;
  }

  if (candidate.personaId) {
    return candidate.kind === "persona"
      ? `persona-template:${candidate.personaId}`
      : `persona:${candidate.personaId}`;
  }

  const label = normalizeLabel(getLabel(candidate));
  if (!label) {
    return null;
  }

  const ownerPubkey = candidate.ownerPubkey
    ? normalizePubkey(candidate.ownerPubkey)
    : null;
  if (ownerPubkey) {
    if (currentPubkey && ownerPubkey === normalizePubkey(currentPubkey)) {
      return `local:name:${label}`;
    }
    return `owner:${ownerPubkey}:name:${label}`;
  }

  return null;
}

function agentCandidateRank<T extends AgentAutocompleteCandidate>(
  candidate: T,
  currentPubkey: string | null | undefined,
  preferredPubkeys: ReadonlySet<string>,
) {
  const pubkey = candidate.pubkey ? normalizePubkey(candidate.pubkey) : null;
  const ownerPubkey = candidate.ownerPubkey
    ? normalizePubkey(candidate.ownerPubkey)
    : null;
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;

  return [
    pubkey && preferredPubkeys.has(pubkey) ? 0 : 1,
    candidate.isMember === true ? 0 : 1,
    candidate.isManagedAgent === true ? 0 : 1,
    candidate.personaId ? 0 : 1,
    ownerPubkey && ownerPubkey === normalizedCurrentPubkey ? 0 : 1,
  ];
}

function isPreferredAgentCandidate<T extends AgentAutocompleteCandidate>(
  next: T,
  current: T,
  currentPubkey: string | null | undefined,
  preferredPubkeys: ReadonlySet<string>,
) {
  const nextRank = agentCandidateRank(next, currentPubkey, preferredPubkeys);
  const currentRank = agentCandidateRank(
    current,
    currentPubkey,
    preferredPubkeys,
  );

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
    currentPubkey,
    getLabel,
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
    const key = agentIdentityKey(candidate, currentPubkey, getLabel);
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
        currentPubkey,
        preferredPubkeys,
      )
    ) {
      output[currentIndex] = candidate;
    }
  }

  return output;
}
