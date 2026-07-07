import * as React from "react";

import { reportChannelBotTyping } from "@/features/agents/agentWorkingSignal";
import type { TypingIndicatorEntry } from "@/features/messages/useChannelTyping";
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
 */
export function channelScopedBotTypingPubkeyKey(
  entries: readonly Pick<TypingIndicatorEntry, "pubkey" | "threadHeadId">[],
): string {
  return entries
    .filter((entry) => entry.threadHeadId === null)
    .map((entry) => entry.pubkey.toLowerCase())
    .sort()
    .join(",");
}

export function useChannelActivityTyping({
  activeChannel,
  activeChannelId,
  channelMembers,
  managedAgents,
  openThreadHeadId,
  relayAgents,
  typingEntries,
}: {
  activeChannel: Channel | null;
  activeChannelId: string | null;
  channelMembers?: ChannelMember[];
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

  // Mirror bot typing into the unified working signal so surfaces that read
  // agentWorkingSignal (sidebar badges, activity panel, composer bar) get the
  // typing fallback. Entries follow the typing TTL because this effect
  // re-reports whenever botTypingEntries changes. Thread-only typing is
  // excluded — see channelScopedBotTypingPubkeyKey.
  const botTypingPubkeyKey = channelScopedBotTypingPubkeyKey(botTypingEntries);
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
