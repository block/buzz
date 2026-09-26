// Channel-level provider for per-thread activity pills.
//
// Computes thread groups ONCE per channel (frames → turns → thread-root
// resolution) and exposes a root-id lookup, so each thread summary row in
// the timeline can render an activity pill without running its own store
// subscription or archive hydration. Pills consume this context directly —
// the timeline rows themselves never subscribe, so observer-frame ticks
// re-render only the pills, not the message list.
//
// Mounted by ChannelPane around the timeline. Surfaces without the provider
// (thread panel, profile) get the empty default and render no pills.

import * as React from "react";

import { useFeatureEnabled } from "@/shared/features";
import { useChannelWorkingAgentPubkeys } from "../agentWorkingSignal";
import { useLoadArchivedObserverEvents } from "../ui/useObserverEvents";
import type { ThreadGroup } from "./activityThreads";
import { useFramesByAgent } from "./useMultiAgentActivity";
import { useThreadScope } from "./useThreadScope";

export interface ThreadActivityAgent {
  pubkey: string;
  name: string;
}

export interface ChannelThreadActivityValue {
  /** Resolved thread groups keyed by thread-root event id. */
  groupsByRootId: ReadonlyMap<string, ThreadGroup>;
  /**
   * Agents with observer frames in THIS channel (pill labels, map input) —
   * pre-filtered so agent chips never offer an agent with nothing to show.
   */
  agents: readonly ThreadActivityAgent[];
  channelId: string | null;
  /** Channel-level working signal; intersect with a group's open turns. */
  workingPubkeys: readonly string[];
}

const EMPTY_VALUE: ChannelThreadActivityValue = {
  groupsByRootId: new Map(),
  agents: [],
  channelId: null,
  workingPubkeys: [],
};

const ChannelThreadActivityContext =
  React.createContext<ChannelThreadActivityValue>(EMPTY_VALUE);

export function useChannelThreadActivity(): ChannelThreadActivityValue {
  return React.useContext(ChannelThreadActivityContext);
}

export function ChannelThreadActivityProvider({
  activityAgents,
  channelId,
  children,
}: {
  activityAgents: readonly ThreadActivityAgent[];
  channelId: string | null;
  children: React.ReactNode;
}) {
  const enabled = useFeatureEnabled("agentActivityPanel");
  if (!enabled || !channelId) {
    return (
      <ChannelThreadActivityContext.Provider value={EMPTY_VALUE}>
        {children}
      </ChannelThreadActivityContext.Provider>
    );
  }
  return (
    <ChannelThreadActivityProviderBody
      activityAgents={activityAgents}
      channelId={channelId}
    >
      {children}
    </ChannelThreadActivityProviderBody>
  );
}

function ChannelThreadActivityProviderBody({
  activityAgents,
  channelId,
  children,
}: {
  activityAgents: readonly ThreadActivityAgent[];
  channelId: string;
  children: React.ReactNode;
}) {
  // Hydrate this channel's archived observer frames into the store so thread
  // activity pills can render from both live and saved observer windows. Cheap
  // when the thread panel already hydrated: pages dedup on (seq, timestamp).
  useLoadArchivedObserverEvents(true, channelId);

  const framesByAgent = useFramesByAgent(activityAgents, channelId);
  const { groups } = useThreadScope(framesByAgent);
  const workingPubkeys = useChannelWorkingAgentPubkeys(channelId);

  const value = React.useMemo<ChannelThreadActivityValue>(() => {
    const groupsByRootId = new Map<string, ThreadGroup>();
    for (const group of groups.threads) {
      if (group.rootId) groupsByRootId.set(group.rootId, group);
    }
    return {
      groupsByRootId,
      // framesByAgent only contains agents with frames in this channel; use it
      // to keep pills scoped to agents that have channel-local activity.
      agents: activityAgents.filter((agent) => framesByAgent.has(agent.pubkey)),
      channelId,
      workingPubkeys,
    };
  }, [groups, activityAgents, framesByAgent, channelId, workingPubkeys]);

  return (
    <ChannelThreadActivityContext.Provider value={value}>
      {children}
    </ChannelThreadActivityContext.Provider>
  );
}
