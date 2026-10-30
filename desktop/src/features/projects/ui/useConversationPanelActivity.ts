import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { selectThreadComposerBotTypingPubkeys } from "@/features/channels/ui/ChannelPane.helpers";
import {
  buildChannelAgentSessionCandidates,
  getChannelAgentSessionAgents,
} from "@/features/channels/ui/useChannelAgentSessions";
import { useChannelTyping } from "@/features/messages/useChannelTyping";
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import type {
  Channel,
  ChannelMember,
  ManagedAgent,
  RelayAgent,
  RelayEvent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

const NO_MANAGED_AGENTS: ManagedAgent[] = [];
const NO_RELAY_AGENTS: RelayAgent[] = [];

/**
 * Who is typing or working in a thread shown outside its channel, split the
 * way the channel's thread panel splits it (useChannelActivityTyping): people
 * get the typing line, the channel's agents get the composer's activity chip.
 * Unlike the channel hook this reports nothing to the shared working signal;
 * the channel screen owns that.
 */
export function useConversationPanelActivity({
  activeChannel,
  channelMembers,
  currentPubkey,
  events,
  rootId,
}: {
  activeChannel: Channel | null;
  channelMembers: ChannelMember[] | undefined;
  currentPubkey: string | undefined;
  events: readonly RelayEvent[];
  rootId: string | null;
}) {
  const relaySelfPubkey = useRelaySelfQuery(activeChannel !== null).data;
  // The newest event clears its author's typing line, as in the channel.
  const latestEvent = React.useMemo(
    () =>
      events.reduce<RelayEvent | null>(
        (latest, event) =>
          !latest || event.created_at > latest.created_at ? event : latest,
        null,
      ),
    [events],
  );
  const typingEntries = useChannelTyping(
    activeChannel,
    currentPubkey,
    latestEvent,
    relaySelfPubkey,
  );
  const managedAgents = useManagedAgentsQuery().data ?? NO_MANAGED_AGENTS;
  const relayAgents = useRelayAgentsQuery().data ?? NO_RELAY_AGENTS;
  const agents = React.useMemo(
    () =>
      getChannelAgentSessionAgents({
        activeChannel,
        activeChannelId: activeChannel?.id ?? null,
        agents: buildChannelAgentSessionCandidates({
          channelMembers,
          managedAgents,
          relayAgents,
        }),
        channelMembers,
      }),
    [activeChannel, channelMembers, managedAgents, relayAgents],
  );
  const { threadTypingPubkeys, workingAgentPubkeys } = React.useMemo(() => {
    const agentPubkeys = new Set(
      agents.map((agent) => normalizePubkey(agent.pubkey)),
    );
    const isAgent = (pubkey: string) =>
      agentPubkeys.has(normalizePubkey(pubkey));
    return {
      threadTypingPubkeys: typingEntries
        .filter(
          (entry) => entry.threadHeadId === rootId && !isAgent(entry.pubkey),
        )
        .map((entry) => entry.pubkey),
      workingAgentPubkeys: selectThreadComposerBotTypingPubkeys(
        typingEntries.filter((entry) => isAgent(entry.pubkey)),
        rootId,
      ),
    };
  }, [agents, rootId, typingEntries]);

  return { agents, threadTypingPubkeys, workingAgentPubkeys };
}
