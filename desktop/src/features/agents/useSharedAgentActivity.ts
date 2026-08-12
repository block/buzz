import * as React from "react";

import {
  mergeSharedAgentActivities,
  type SharedAgentActivity,
} from "@/features/agents/sharedAgentActivity";
import { subscribeToAgentActivitySummaries } from "@/shared/api/agentActivitySummaryRelay";

export type SharedAgentActivityConnection =
  | "idle"
  | "connecting"
  | "live"
  | "closed"
  | "error";

export function useSharedAgentActivity(input: {
  enabled: boolean;
  agentPubkey: string;
  channelId: string | null;
}) {
  const [activities, setActivities] = React.useState<SharedAgentActivity[]>([]);
  const [connection, setConnection] =
    React.useState<SharedAgentActivityConnection>("idle");

  React.useEffect(() => {
    setActivities([]);
    if (!input.enabled || !input.channelId) {
      setConnection("idle");
      return;
    }

    let disposed = false;
    let unsubscribe: (() => Promise<void>) | null = null;
    const seenEventIds = new Set<string>();
    const seenOrder: string[] = [];
    setConnection("connecting");

    void subscribeToAgentActivitySummaries({
      agentPubkey: input.agentPubkey,
      channelId: input.channelId,
      onReady: () => {
        if (!disposed) setConnection("live");
      },
      onTerminalClosed: () => {
        if (disposed) return;
        seenEventIds.clear();
        seenOrder.length = 0;
        setActivities([]);
        setConnection("closed");
      },
      onEvent: ({ eventId, frame }) => {
        if (disposed || seenEventIds.has(eventId)) return;
        seenEventIds.add(eventId);
        seenOrder.push(eventId);
        while (seenOrder.length > 500) {
          const oldest = seenOrder.shift();
          if (oldest) seenEventIds.delete(oldest);
        }
        setActivities((current) =>
          mergeSharedAgentActivities(current, frame.activities),
        );
      },
    })
      .then((cleanup) => {
        if (disposed) void cleanup();
        else unsubscribe = cleanup;
      })
      .catch(() => {
        if (!disposed) setConnection("error");
      });

    return () => {
      disposed = true;
      if (unsubscribe) void unsubscribe();
    };
  }, [input.agentPubkey, input.channelId, input.enabled]);

  return { activities, connection };
}
