import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelReferences } from "@/features/channels/openChannelDirectory";
import { useAgentSession } from "@/shared/context/AgentSessionContext";
import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { getAgentWorkingState } from "./agentWorkingSignal";
import { useRelayAgentsQuery } from "./hooks";

const INACCESSIBLE_ACTIVITY_MESSAGE =
  "This agent is active in a channel you haven't joined, so its activity can't be opened from here.";

/**
 * Can the viewer actually open this channel? Joined channels always;
 * open-visibility channels are readable without joining. Channels missing
 * from the viewer's channel list (e.g. private rooms they aren't in) are
 * not navigable destinations.
 */
export function isChannelOpenable(
  channel: Pick<Channel, "isMember" | "visibility"> | undefined,
): boolean {
  return (
    channel !== undefined && (channel.isMember || channel.visibility === "open")
  );
}

/**
 * Pick the channel to land in when opening an agent's activity from a
 * non-channel route: the agent's first working channel the viewer can open,
 * else the agent's first member channel the viewer can open, else null.
 */
export function resolveOpenableActivityChannelId({
  agentChannelIds,
  openableChannelIds,
  workingChannelIds,
}: {
  agentChannelIds: readonly string[];
  openableChannelIds: ReadonlySet<string>;
  workingChannelIds: readonly string[];
}): string | null {
  for (const channelId of workingChannelIds) {
    if (openableChannelIds.has(channelId)) {
      return channelId;
    }
  }
  for (const channelId of agentChannelIds) {
    if (openableChannelIds.has(channelId)) {
      return channelId;
    }
  }
  return null;
}

/**
 * Universal ingress for opening an agent's activity pane.
 *
 * Inside a channel screen the AgentSessionContext handler opens the pane in
 * place. Everywhere else (agents page, home profile panel, popovers reached
 * from non-channel routes) there is no provider, so we navigate to a channel
 * with the `agentSession` search param instead — preferring a channel the
 * agent is currently working in (unified working signal), then falling back
 * to the first channel the agent is a member of.
 *
 * Navigation only ever targets channels the viewer can actually open
 * (joined, or open visibility). Owner-global ingestion means the working
 * signal can report activity in rooms the viewer can't access; deep-linking
 * there would land on a screen they can't read. In that case we surface a
 * safe warning instead of navigating — no channel content, no trap-door.
 *
 * This replaces the old behavior where "View activity log" silently
 * disappeared on routes without an AgentSessionProvider.
 */
export function useOpenAgentActivity() {
  const { onOpenAgentSession } = useAgentSession();
  const { goChannel } = useAppNavigation();
  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgents = relayAgentsQuery.data;
  // Agent metadata and the working-signal snapshot are both finite id sources.
  // Resolve them by id, never by scanning the all-open directory, so an agent
  // can link to a readable open channel the viewer has not browsed this session.
  const activityChannelIds = React.useMemo(
    () => [
      ...(relayAgents ?? []).flatMap((agent) => agent.channelIds),
      ...(relayAgents ?? []).flatMap((agent) =>
        getAgentWorkingState(agent.pubkey).channels.map(
          (working) => working.channelId,
        ),
      ),
    ],
    [relayAgents],
  );
  const { channelsById, isReady: areChannelsReady } =
    useChannelReferences(activityChannelIds);

  const findOpenableChannel = React.useCallback(
    (channelId: string): boolean =>
      isChannelOpenable(channelsById.get(channelId)),
    [channelsById],
  );

  const resolveChannelId = React.useCallback(
    (pubkey: string): string | null => {
      const key = normalizePubkey(pubkey);
      const relayAgent = relayAgents?.find(
        (agent) => normalizePubkey(agent.pubkey) === key,
      );
      const openableChannelIds = new Set(
        [...channelsById.values()]
          .filter((channel) => isChannelOpenable(channel))
          .map((channel) => channel.id),
      );
      return resolveOpenableActivityChannelId({
        agentChannelIds: relayAgent?.channelIds ?? [],
        openableChannelIds,
        // Deliberately an unsubscribed snapshot: this callback runs on click
        // (and in canOpenAgentActivity), not in render, so we don't need to
        // recompute when working state changes — its deps are only
        // [channelsById, relayAgents]. Worst case the preferred working-channel
        // target lags a just-changed signal; the member-channel fallback in
        // resolveOpenableActivityChannelId keeps the destination valid.
        workingChannelIds: getAgentWorkingState(pubkey).channels.map(
          (working) => working.channelId,
        ),
      });
    },
    [channelsById, relayAgents],
  );

  const canOpenAgentActivity = React.useCallback(
    (pubkey: string | null | undefined): boolean => {
      if (!pubkey) {
        return false;
      }
      if (onOpenAgentSession) {
        return true;
      }
      // While channels are still loading, resolveChannelId can only see an
      // empty openable set and would report a transient false. Stay
      // optimistic until channels resolve so "View activity log" doesn't
      // flicker in on cold start; openAgentActivity still guards the actual
      // navigation.
      if (!areChannelsReady) {
        return true;
      }
      return resolveChannelId(pubkey) !== null;
    },
    [areChannelsReady, onOpenAgentSession, resolveChannelId],
  );

  const openAgentActivity = React.useCallback(
    (pubkey: string, options?: { channelId?: string | null }): boolean => {
      // An explicit channel target (e.g. clicking a "Working in #channel"
      // badge) navigates so the pane opens scoped to that channel — but only
      // when the viewer can actually open that channel. Scoping the pane to
      // an inaccessible room (in place or via navigation) would expose that
      // room's activity content, so we warn and stop instead.
      if (options?.channelId) {
        if (!onOpenAgentSession) {
          if (!findOpenableChannel(options.channelId)) {
            toast.warning(INACCESSIBLE_ACTIVITY_MESSAGE);
            return false;
          }
          void goChannel(options.channelId, { agentSession: pubkey });
          return true;
        }
        // A channel-scoped AgentSessionProvider belongs to the channel view
        // already authorized by its route. Do not reject its own current
        // channel while the member/reference query is still settling.
        onOpenAgentSession(pubkey, options.channelId);
        return true;
      }
      if (onOpenAgentSession) {
        onOpenAgentSession(pubkey);
        return true;
      }
      const channelId = resolveChannelId(pubkey);
      if (channelId) {
        void goChannel(channelId, { agentSession: pubkey });
        return true;
      }
      // The agent may be working somewhere, just nowhere the viewer can open.
      // Say so plainly rather than failing silently — without leaking which
      // room, or navigating into it.
      if (getAgentWorkingState(pubkey).channels.length > 0) {
        toast.warning(INACCESSIBLE_ACTIVITY_MESSAGE);
      }
      return false;
    },
    [findOpenableChannel, goChannel, onOpenAgentSession, resolveChannelId],
  );

  return { canOpenAgentActivity, openAgentActivity };
}
