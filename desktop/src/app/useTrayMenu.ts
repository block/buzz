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
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { UserProfileSummary } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { useNow } from "@/shared/lib/useNow";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import type { Channel } from "@/shared/api/types";

type TrayAgentActivity = {
  activityId: string;
  agentName: string;
  channelId: string;
  channelName: string;
  elapsed: string;
};

type TrayAgentActivityState = TrayAgentActivity & {
  agentPubkey: string;
};

type TrayAction =
  | { kind: "newChannel" }
  | { kind: "openChannel"; channelId: string };

const MAX_RECENT_TRAY_ACTIVITIES = 5;

export function resolveTrayAgentName({
  knownAgentName,
  profile,
  pubkey,
}: {
  knownAgentName?: string;
  profile?: Pick<UserProfileSummary, "displayName" | "name">;
  pubkey: string;
}): string {
  return (
    profile?.displayName?.trim() ||
    profile?.name?.trim() ||
    knownAgentName?.trim() ||
    `Agent ${truncatePubkey(pubkey)}`
  );
}

export function resolveTrayActivities({
  activities,
  knownAgentNames,
  profiles,
}: {
  activities: TrayAgentActivityState[];
  knownAgentNames: Map<string, string>;
  profiles?: UserProfileLookup;
}): TrayAgentActivity[] {
  return activities.map(({ agentPubkey, ...activity }) => ({
    ...activity,
    agentName: resolveTrayAgentName({
      knownAgentName: knownAgentNames.get(normalizePubkey(agentPubkey)),
      profile: profiles?.[normalizePubkey(agentPubkey)],
      pubkey: agentPubkey,
    }),
  }));
}

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
    new Map<string, TrayAgentActivityState>(),
  );
  const [recentActivities, setRecentActivities] = React.useState<
    TrayAgentActivityState[]
  >([]);
  const activityAgentPubkeys = React.useMemo(
    () => [
      ...new Set(
        [
          ...activeTurns.flatMap((turn) => turn.agentPubkeys),
          ...recentActivities.map((activity) => activity.agentPubkey),
        ].map((pubkey) => normalizePubkey(pubkey)),
      ),
    ],
    [activeTurns, recentActivities],
  );
  const profiles = useUsersBatchQuery(activityAgentPubkeys, {
    enabled: activityAgentPubkeys.length > 0,
  }).data?.profiles;
  const knownAgentNames = React.useMemo(
    () =>
      new Map(
        [...(managedAgents ?? []), ...(relayAgents ?? [])].map((agent) => [
          normalizePubkey(agent.pubkey),
          agent.name,
        ]),
      ),
    [managedAgents, relayAgents],
  );

  const activities = React.useMemo<TrayAgentActivityState[]>(() => {
    const channelNames = new Map(
      channels.map((channel) => [channel.id, channel.name]),
    );

    return activeTurns.flatMap((channelTurn) =>
      channelTurn.agentPubkeys.map((pubkey) => {
        const agentTurn = getActiveTurnsForAgent(pubkey).find(
          (turn) => turn.channelId === channelTurn.channelId,
        );

        return {
          activityId: `${channelTurn.channelId}:${normalizePubkey(pubkey)}`,
          agentPubkey: pubkey,
          agentName: resolveTrayAgentName({
            knownAgentName: knownAgentNames.get(normalizePubkey(pubkey)),
            profile: profiles?.[normalizePubkey(pubkey)],
            pubkey,
          }),
          channelId: channelTurn.channelId,
          channelName:
            channelNames.get(channelTurn.channelId) ?? "Unknown channel",
          elapsed: formatElapsed(
            now - (agentTurn?.anchorAt ?? channelTurn.anchorAt),
          ),
        };
      }),
    );
  }, [activeTurns, channels, knownAgentNames, now, profiles]);

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

  React.useEffect(() => {
    if (!isTauri()) return;
    void invoke("update_tray_agent_activity", {
      activities: resolveTrayActivities({
        activities,
        knownAgentNames,
        profiles,
      }),
      recentActivities: resolveTrayActivities({
        activities: recentActivities,
        knownAgentNames,
        profiles,
      }),
    }).catch((error) => {
      console.error("Failed to update the macOS tray menu", error);
    });
  }, [activities, knownAgentNames, profiles, recentActivities]);

  React.useEffect(() => {
    if (!isTauri()) return;

    let disposed = false;
    let unlisten: (() => void) | undefined;

    const handlePendingActions = async () => {
      if (disposed) return;
      const actions = await invoke<TrayAction[]>("take_tray_actions");
      if (disposed) {
        if (actions.length > 0) {
          await invoke("requeue_tray_actions", { actions });
        }
        return;
      }
      for (const action of actions) {
        if (action.kind === "newChannel") {
          openCreateChannel();
        } else {
          void goChannel(action.channelId);
        }
      }
    };

    void (async () => {
      const nextUnlisten = await listen("tray-action-available", () => {
        void handlePendingActions();
      });
      if (disposed) {
        nextUnlisten();
        return;
      }
      unlisten = nextUnlisten;
      await handlePendingActions();
    })();

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [goChannel, openCreateChannel]);
}
