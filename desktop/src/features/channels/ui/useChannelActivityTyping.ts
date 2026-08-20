import * as React from "react";

import { reportChannelBotTyping } from "@/features/agents/agentWorkingSignal";
import { combineObserverIngestionAgents } from "@/features/agents/useAgentObserverIngestion";
import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  Channel,
  ChannelMember,
  ManagedAgent,
  RelayAgent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  buildChannelAgentSessionCandidates,
  getChannelAgentSessionAgents,
} from "./useChannelAgentSessions";

/**
 * Key of bot typing pubkeys that may mark the *channel* as working. Only
 * channel-scoped entries (`threadHeadId === null`) count — thread-only typing
 * must not light channel-level surfaces (composer bar, sidebar, profile);
 * thread surfaces apply their own `threadHeadId` filter.
 *
 * When `ownedPubkeys` is supplied, entries are additionally narrowed to agents
 * the current identity owns (normalized pubkeys). The working timer/badge is a
 * "my agents are busy" affordance, so another member's agent typing in a shared
 * channel must not light it — only the owner sees their own agents' activity.
 * Omitting `ownedPubkeys` keeps every channel-scoped entry (used by surfaces
 * that legitimately show all agents, e.g. the in-channel "typing…" row).
 */
export function channelScopedBotTypingPubkeyKey(
  entries: readonly Pick<TypingIndicatorEntry, "pubkey" | "threadHeadId">[],
  ownedPubkeys?: ReadonlySet<string>,
): string {
  return entries
    .filter(
      (entry) =>
        entry.threadHeadId === null &&
        (ownedPubkeys === undefined ||
          ownedPubkeys.has(normalizePubkey(entry.pubkey))),
    )
    .map((entry) => entry.pubkey.toLowerCase())
    .sort()
    .join(",");
}

export function useChannelActivityTyping({
  activeChannel,
  activeChannelId,
  channelMembers,
  currentPubkey,
  managedAgents,
  openThreadHeadId,
  relayAgents,
  typingEntries,
}: {
  activeChannel: Channel | null;
  activeChannelId: string | null;
  channelMembers?: ChannelMember[];
  currentPubkey: string | null | undefined;
  managedAgents: ManagedAgent[];
  openThreadHeadId: string | null;
  relayAgents: RelayAgent[];
  typingEntries: TypingIndicatorEntry[];
}) {
  const agentCandidates = React.useMemo(
    () =>
      buildChannelAgentSessionCandidates({
        channelMembers,
        managedAgents,
        relayAgents,
      }),
    [channelMembers, managedAgents, relayAgents],
  );
  const channelAgentSessionAgents = React.useMemo(
    () =>
      getChannelAgentSessionAgents({
        activeChannel,
        activeChannelId,
        agents: agentCandidates,
        channelMembers,
      }),
    [activeChannel, activeChannelId, agentCandidates, channelMembers],
  );
  const channelAgentPubkeys = React.useMemo(
    () =>
      new Set(
        channelAgentSessionAgents.map((agent) => normalizePubkey(agent.pubkey)),
      ),
    [channelAgentSessionAgents],
  );
  const threadTypingPubkeys = React.useMemo(
    () =>
      typingEntries
        .filter(
          (entry) =>
            entry.threadHeadId === openThreadHeadId &&
            !channelAgentPubkeys.has(normalizePubkey(entry.pubkey)),
        )
        .map((entry) => entry.pubkey),
    [channelAgentPubkeys, openThreadHeadId, typingEntries],
  );
  const { botTypingEntries, humanTypingPubkeys } = React.useMemo<{
    botTypingEntries: TypingIndicatorEntry[];
    humanTypingPubkeys: string[];
  }>(() => {
    const botTypingEntries: TypingIndicatorEntry[] = [];
    const humanTypingPubkeys: string[] = [];
    for (const entry of typingEntries) {
      if (channelAgentPubkeys.has(normalizePubkey(entry.pubkey))) {
        botTypingEntries.push(entry);
      } else if (entry.threadHeadId === null) {
        humanTypingPubkeys.push(entry.pubkey);
      }
    }
    return { botTypingEntries, humanTypingPubkeys };
  }, [channelAgentPubkeys, typingEntries]);

  // Agents the current identity owns: locally managed agents plus relay agents
  // whose declared owner (NIP-OA) is me. Reuses the exact owned-set rule the
  // observer-turn ingestion applies (combineObserverIngestionAgents), so both
  // feeds of the working signal agree on "my agents". The relay-agent owner
  // lookup shares React Query's cache with the observer ingestion's identical
  // query, so this adds no extra fetch.
  const relayAgentPubkeys = React.useMemo(
    () => relayAgents.map((agent) => agent.pubkey),
    [relayAgents],
  );
  const ownerProfilesQuery = useUsersBatchQuery(relayAgentPubkeys, {
    enabled: Boolean(currentPubkey) && relayAgentPubkeys.length > 0,
  });
  const ownerProfiles = ownerProfilesQuery.data?.profiles;
  const ownedAgentPubkeys = React.useMemo(() => {
    const ownerByPubkey = new Map<string, string>();
    for (const [pubkey, summary] of Object.entries(ownerProfiles ?? {})) {
      if (summary.ownerPubkey) {
        ownerByPubkey.set(
          normalizePubkey(pubkey),
          normalizePubkey(summary.ownerPubkey),
        );
      }
    }
    return new Set(
      combineObserverIngestionAgents(
        managedAgents,
        relayAgentPubkeys,
        ownerByPubkey,
        currentPubkey,
      ).map((agent) => normalizePubkey(agent.pubkey)),
    );
  }, [managedAgents, relayAgentPubkeys, ownerProfiles, currentPubkey]);

  // Mirror bot typing into the unified working signal so surfaces that read
  // agentWorkingSignal (sidebar badges, activity panel, composer bar) get the
  // typing fallback. Entries follow the typing TTL because this effect
  // re-reports whenever botTypingEntries changes. Thread-only typing is
  // excluded, and — unlike the in-channel typing row — only agents *I own* are
  // mirrored, so another member's agent never lights my working timer (see
  // channelScopedBotTypingPubkeyKey).
  const botTypingPubkeyKey = channelScopedBotTypingPubkeyKey(
    botTypingEntries,
    ownedAgentPubkeys,
  );
  React.useEffect(() => {
    if (!activeChannelId) {
      return;
    }
    reportChannelBotTyping(
      activeChannelId,
      botTypingPubkeyKey ? botTypingPubkeyKey.split(",") : [],
    );
    return () => {
      reportChannelBotTyping(activeChannelId, []);
    };
  }, [activeChannelId, botTypingPubkeyKey]);

  return {
    agentSessionCandidates: agentCandidates,
    botTypingEntries,
    channelAgentSessionAgents,
    humanTypingPubkeys,
    threadTypingPubkeys,
  };
}

export function mergeAgentNamesIntoProfiles(
  profiles: UserProfileLookup,
  managedAgents: ManagedAgent[],
  relayAgents: RelayAgent[],
  currentPubkey?: string | null,
): UserProfileLookup {
  const merged = { ...profiles };
  for (const agent of relayAgents) {
    const key = normalizePubkey(agent.pubkey);
    merged[key] = {
      ...merged[key],
      displayName: merged[key]?.displayName || agent.name,
      avatarUrl: merged[key]?.avatarUrl ?? null,
      nip05Handle: merged[key]?.nip05Handle ?? null,
      isAgent: true,
    };
  }
  for (const agent of managedAgents) {
    const key = normalizePubkey(agent.pubkey);
    merged[key] = {
      ...merged[key],
      displayName: merged[key]?.displayName || agent.name,
      avatarUrl: merged[key]?.avatarUrl ?? agent.avatarUrl,
      nip05Handle: merged[key]?.nip05Handle ?? null,
      ownerPubkey: merged[key]?.ownerPubkey ?? currentPubkey ?? null,
      isAgent: true,
    };
  }
  return merged;
}

/**
 * Fold channel-member agent flags (`role === "bot"` or `isAgent`) into a
 * profile lookup as `isAgent: true` entries — the same pattern
 * `mergeAgentNamesIntoProfiles` applies to managed/relay agents, extended to
 * the membership signal. Per-pubkey `profiles[pk]?.isAgent` checks
 * (MessageRow's agent predicate) then see member-only bots — agents known
 * through channel membership alone, with no profile flag and no
 * managed/relay/feed presence — without a separate agent-set prop.
 *
 * Returns the input lookup unchanged (same reference) when no member carries
 * an agent flag.
 */
export function mergeMemberAgentFlagsIntoProfiles(
  profiles: UserProfileLookup,
  channelMembers:
    | readonly Pick<ChannelMember, "pubkey" | "role" | "isAgent">[]
    | undefined,
): UserProfileLookup {
  const agentMembers = (channelMembers ?? []).filter(
    (member) => member.role === "bot" || member.isAgent,
  );
  if (agentMembers.length === 0) {
    return profiles;
  }
  const merged = { ...profiles };
  for (const member of agentMembers) {
    const key = normalizePubkey(member.pubkey);
    merged[key] = {
      ...merged[key],
      displayName: merged[key]?.displayName ?? null,
      avatarUrl: merged[key]?.avatarUrl ?? null,
      nip05Handle: merged[key]?.nip05Handle ?? null,
      ownerPubkey: merged[key]?.ownerPubkey ?? null,
      isAgent: true,
    };
  }
  return merged;
}
