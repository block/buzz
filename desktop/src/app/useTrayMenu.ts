import * as React from "react";
import { isTauri, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  getActiveTurnsForAgent,
  useActiveAgentTurnsByChannel,
} from "@/features/agents/activeAgentTurnsStore";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { useNow } from "@/shared/lib/useNow";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import type { Channel } from "@/shared/api/types";
import {
  keepOpenableTrayActivities,
  type TrayAgentActivity,
} from "@/app/trayActivities";
import {
  subscribeToTrayActions,
  type TrayAction,
} from "@/app/trayActionConsumer";

const MAX_RECENT_TRAY_ACTIVITIES = 5;

/**
 * Keeps Buzz's native tray menu synchronized with active agent turns and
 * forwards its navigation actions into the React app.
 */
export function useTrayMenu({
  channels,
  goChannel,
  openCreateChannel,
}: {
  channels: Channel[];
  goChannel: (channelId: string) => Promise<unknown>;
  openCreateChannel: () => void;
}): void {
  const activeTurns = useActiveAgentTurnsByChannel();
  const now = useNow(1000);
  const managedAgents = useManagedAgentsQuery().data;
  const relayAgents = useRelayAgentsQuery().data;
  const previousActivitiesRef = React.useRef(
    new Map<string, TrayAgentActivity>(),
  );
  const [recentActivities, setRecentActivities] = React.useState<
    TrayAgentActivity[]
  >([]);

  const channelIds = React.useMemo(
    () => new Set(channels.map((channel) => channel.id)),
    [channels],
  );
  const activities = React.useMemo<TrayAgentActivity[]>(() => {
    const channelNames = new Map(
      channels.map((channel) => [channel.id, channel.name]),
    );
    const agentNames = new Map<string, string>();
    for (const agent of [...(managedAgents ?? []), ...(relayAgents ?? [])]) {
      agentNames.set(normalizePubkey(agent.pubkey), agent.name);
    }

    const currentActivities = activeTurns.flatMap((channelTurn) =>
      channelTurn.agentPubkeys.map((pubkey) => {
        const agentTurn = getActiveTurnsForAgent(pubkey).find(
          (turn) => turn.channelId === channelTurn.channelId,
        );

        return {
          activityId: `${channelTurn.channelId}:${normalizePubkey(pubkey)}`,
          agentName:
            agentNames.get(normalizePubkey(pubkey)) ??
            `Agent ${truncatePubkey(pubkey)}`,
          channelId: channelTurn.channelId,
          channelName:
            channelNames.get(channelTurn.channelId) ?? "Unknown channel",
          elapsed: formatElapsed(
            now - (agentTurn?.anchorAt ?? channelTurn.anchorAt),
          ),
        };
      }),
    );
    return keepOpenableTrayActivities(currentActivities, channelIds);
  }, [activeTurns, channelIds, channels, managedAgents, now, relayAgents]);

  React.useEffect(() => {
    const currentActivities = new Map(
      activities.map((activity) => [activity.activityId, activity]),
    );
    const completedActivities = [...previousActivitiesRef.current.entries()]
      .filter(([activityId]) => !currentActivities.has(activityId))
      .map(([, activity]) => ({
        ...activity,
        activityId: `recent:${activity.activityId}:${Date.now()}`,
      }));

    if (completedActivities.length > 0) {
      setRecentActivities((current) =>
        [...completedActivities, ...current].slice(
          0,
          MAX_RECENT_TRAY_ACTIVITIES,
        ),
      );
    }
    previousActivitiesRef.current = currentActivities;
  }, [activities]);

  const openableRecentActivities = React.useMemo(
    () => keepOpenableTrayActivities(recentActivities, channelIds),
    [channelIds, recentActivities],
  );

  React.useEffect(() => {
    if (!isTauri()) return;
    void invoke("update_tray_agent_activity", {
      activities,
      recentActivities: openableRecentActivities,
    }).catch((error) => {
      console.error("Failed to update the macOS tray menu", error);
    });
  }, [activities, openableRecentActivities]);

  React.useEffect(() => {
    if (!isTauri()) return;

    return subscribeToTrayActions({
      addFocusListener: (listener) => {
        // Native queues before restoring the window, so focus is the durable
        // wake-up if WebKit dropped the earlier event while Buzz was hidden.
        window.addEventListener("focus", listener);
        return () => window.removeEventListener("focus", listener);
      },
      listenForAvailable: (listener) =>
        listen("tray-action-available", listener),
      takePendingActions: () => invoke<TrayAction[]>("take_tray_actions"),
      requeueActions: (actions) => invoke("requeue_tray_actions", { actions }),
      handleAction: (action) => {
        if (action.kind === "newChannel") {
          openCreateChannel();
        } else {
          void goChannel(action.channelId);
        }
      },
    });
  }, [goChannel, openCreateChannel]);
}
