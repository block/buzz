import * as React from "react";

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

  return {
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
): UserProfileLookup {
  const merged = { ...profiles };
  for (const agent of [...relayAgents, ...managedAgents]) {
    const key = normalizePubkey(agent.pubkey);
    if (!merged[key]?.displayName) {
      merged[key] = {
        ...merged[key],
        displayName: agent.name,
        avatarUrl: merged[key]?.avatarUrl ?? null,
        nip05Handle: merged[key]?.nip05Handle ?? null,
      };
    }
  }
  return merged;
}
