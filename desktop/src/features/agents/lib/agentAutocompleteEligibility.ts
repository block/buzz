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

export function isAgentIdentityInManagedList(
  candidate: { isAgent?: boolean; pubkey: string },
  managedAgentPubkeys: ReadonlySet<string>,
) {
  return (
    candidate.isAgent !== true ||
    managedAgentPubkeys.has(normalizePubkey(candidate.pubkey))
  );
}

/** The respond-to declaration from an agent's kind:10100 directory entry. */
export type RelayAgentMentionPolicy = Pick<
  RelayAgent,
  "respondTo" | "respondToAllowlist"
>;

/**
 * Hide an agent-classified mention candidate unless there is evidence the
 * mention will be received and answered.
 *
 * Offering a mention asserts "this agent will receive the message and its
 * policy admits responding to you". That is decided from the same proofs
 * other agent surfaces already trust, never from the local store alone:
 *
 * 1. A local managed record — this desktop runs the agent, so it can vouch
 *    directly (the native path, unchanged).
 * 2. Channel membership (relay-authoritative — candidates originate from the
 *    member list) plus the agent's own kind:10100 respond-to declaration:
 *    `anyone`, an allowlist naming the current user, or `owner-only` when
 *    the current user IS the agent's owner. Ownership uses
 *    `candidate.ownerPubkey` — the NIP-OA-verified value the "managed by"
 *    surface renders — so external agents owned by the current user are
 *    mentionable exactly like managed ones.
 *
 * Liveness is deliberately not required: buzz-acp replays missed mentions on
 * reconnect (`since` filter), and stopped managed agents are offered today.
 * An agent-classified candidate with no local record and no directory
 * declaration stays hidden — with no proof it will answer, a mention chip
 * would fire into the void.
 */
export function shouldHideAgentFromMentions({
  candidate,
  currentPubkey,
  managedAgentPubkeys,
  relayAgentPolicies,
}: {
  candidate: {
    isAgent?: boolean;
    isMember?: boolean;
    ownerPubkey?: string | null;
    pubkey: string;
  };
  currentPubkey: string | null | undefined;
  managedAgentPubkeys: ReadonlySet<string>;
  relayAgentPolicies: ReadonlyMap<string, RelayAgentMentionPolicy>;
}) {
  if (candidate.isAgent !== true) return false;
  const pubkey = normalizePubkey(candidate.pubkey);
  if (managedAgentPubkeys.has(pubkey)) return false;
  // Offering a non-member external agent would need the add-via-mention flow
  // to honor the agent's `channel_add_policy` declaration — a follow-up, not
  // this change. Members only for now.
  if (candidate.isMember !== true) return true;
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const ownerPubkey = candidate.ownerPubkey
    ? normalizePubkey(candidate.ownerPubkey)
    : null;
  const policy = relayAgentPolicies.get(pubkey);
  if (!policy) {
    // No declaration: offer only to the verified owner. Sound because the
    // harness's inbound author gate admits the owner under every respond-to
    // mode, so an owner's mention is never a void chip — and it covers the
    // owner-on-another-device reports (#2349, #3277) without requiring a
    // directory entry that nothing publishes for native agents yet. For
    // anyone else there is still no proof the agent will answer. Stale
    // same-name identities an owner might now see fold into the live one
    // via the same-label/same-owner coalescing (and NIP-IA archived
    // identities are peeled before candidates are built).
    return (
      normalizedCurrentPubkey === null ||
      ownerPubkey === null ||
      ownerPubkey !== normalizedCurrentPubkey
    );
  }
  switch (policy.respondTo) {
    case "anyone":
      return false;
    case "allowlist":
      return (
        normalizedCurrentPubkey === null ||
        !policy.respondToAllowlist
          .map((entry) => normalizePubkey(entry))
          .includes(normalizedCurrentPubkey)
      );
    case "owner-only":
      return (
        normalizedCurrentPubkey === null ||
        ownerPubkey === null ||
        ownerPubkey !== normalizedCurrentPubkey
      );
    default:
      return true;
  }
}

type AgentAutocompleteCandidate = {
  pubkey?: string;
  displayName?: string | null;
  ownerPubkey?: string | null;
  isAgent?: boolean;
  isManagedAgent?: boolean;
  isMember?: boolean;
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
    return `persona:${candidate.personaId}`;
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
    candidate.isMember === true ? 0 : 1,
    pubkey && preferredPubkeys.has(pubkey) ? 0 : 1,
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
