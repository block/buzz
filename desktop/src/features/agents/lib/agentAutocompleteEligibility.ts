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

/**
 * Whether an autocomplete candidate that is an agent identity may be shown.
 *
 * Agents are hidden unless we can reach them, so a mention never silently
 * fails. Two things make an agent reachable by the current user:
 *
 * 1. It is in this machine's managed-agent list (we can spawn it on demand).
 * 2. It is a member of this channel **and** the current user owns it. #1243
 *    set out to "scope mention/add autocomplete to reachable identities" and
 *    described an eligible agent as "my managed/owned agent", but only the
 *    managed list was ever consulted. Membership supplies the reachability
 *    evidence that ownership alone does not: the agent is in the channel and
 *    answering, so it is running somewhere. Ownership then supplies the
 *    permission, and it does not depend on which machine hosts the process —
 *    an agent left on the default `respond_to: owner-only` accepts its owner
 *    by definition, so hiding it from that owner is backwards.
 *
 * Deliberately NOT extended to non-member owned agents: a profile-only agent
 * we own has no evidence of running anywhere, and #1243 hides it on purpose.
 *
 * `currentPubkey` is optional so existing callers keep their behaviour; pass
 * it to enable the ownership branch.
 */
export function isAgentIdentityInManagedList(
  candidate: {
    isAgent?: boolean;
    isMember?: boolean;
    pubkey: string;
    ownerPubkey?: string | null;
  },
  managedAgentPubkeys: ReadonlySet<string>,
  currentPubkey?: string | null,
) {
  if (candidate.isAgent !== true) {
    return true;
  }
  if (managedAgentPubkeys.has(normalizePubkey(candidate.pubkey))) {
    return true;
  }
  return (
    candidate.isMember === true &&
    isOwnedByCurrentUser(candidate.ownerPubkey, currentPubkey)
  );
}

function isOwnedByCurrentUser(
  ownerPubkey: string | null | undefined,
  currentPubkey: string | null | undefined,
) {
  if (!ownerPubkey || !currentPubkey) {
    return false;
  }
  return normalizePubkey(ownerPubkey) === normalizePubkey(currentPubkey);
}

export function shouldHideAgentFromMentions({
  isAgent,
  isMember,
  pubkey,
  ownerPubkey,
  currentPubkey,
  mentionableAgentPubkeys,
  directoryAgentPubkeys,
}: {
  isAgent: boolean;
  isMember: boolean;
  pubkey: string;
  ownerPubkey?: string | null;
  currentPubkey?: string | null;
  mentionableAgentPubkeys: ReadonlySet<string>;
  directoryAgentPubkeys: ReadonlySet<string>;
}) {
  if (!isAgent) return false;
  const normalized = normalizePubkey(pubkey);
  // Invocable => always show.
  if (mentionableAgentPubkeys.has(normalized)) return false;
  // Non-member, non-invocable => hide (preserves prior behavior).
  if (!isMember) return true;
  // A member we own => invocable, wherever its process runs.
  // `mentionableAgentPubkeys` only admits relay agents whose `respond_to` is
  // `anyone` or an allowlist naming us, so an agent left on the default
  // `owner-only` never lands there — yet `owner-only` is precisely the mode
  // that always accepts its owner. Without this, the directory check below
  // reads that agent's kind:10100 entry as an explicit not-invocable signal
  // and hides it from the one person guaranteed to be able to invoke it.
  if (isOwnedByCurrentUser(ownerPubkey, currentPubkey)) return false;
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

/**
 * The single eligibility decision for an @-mention autocomplete candidate.
 *
 * The two predicates above encode one policy but are ordered: the managed-list
 * gate runs first and can reject a candidate before the invocability gate ever
 * applies its "invocable => show" rule. Composing them here keeps that order
 * explicit and in one place, so a caller cannot chain them the other way round
 * or apply only half the policy.
 */
export function isAgentMentionEligible({
  candidate,
  currentPubkey,
  directoryAgentPubkeys,
  managedAgentPubkeys,
  mentionableAgentPubkeys,
}: {
  candidate: {
    isAgent?: boolean;
    isMember?: boolean;
    pubkey: string;
    ownerPubkey?: string | null;
  };
  currentPubkey?: string | null;
  directoryAgentPubkeys: ReadonlySet<string>;
  managedAgentPubkeys: ReadonlySet<string>;
  mentionableAgentPubkeys: ReadonlySet<string>;
}) {
  if (candidate.isAgent !== true) {
    return true;
  }
  if (
    !isAgentIdentityInManagedList(candidate, managedAgentPubkeys, currentPubkey)
  ) {
    return false;
  }
  return !shouldHideAgentFromMentions({
    currentPubkey,
    directoryAgentPubkeys,
    isAgent: true,
    isMember: candidate.isMember === true,
    mentionableAgentPubkeys,
    ownerPubkey: candidate.ownerPubkey,
    pubkey: candidate.pubkey,
  });
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
