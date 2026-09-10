import { useEffect, useMemo, useState } from "react";
import { runtime } from "@/shared/runtime/client";
import type { ObserverEvent } from "@/shared/runtime/observerEvent";
import { reduceActivity } from "./activityProjection";
import type { AgentTurn } from "./types";

/**
 * The one app-scoped ingestion point for owner-private observer telemetry.
 *
 * Mount this once above conversation placements. Consumers receive a read model
 * filtered to their current channel rather than starting their own observer
 * subscription.
 */
export function useAgentActivity() {
  const [turnsByKey, setTurnsByKey] = useState<Map<string, AgentTurn>>(
    () => new Map(),
  );

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;

    void runtime
      .observe((event: ObserverEvent) => {
        if (!disposed)
          setTurnsByKey((current) => reduceActivity(current, event));
      })
      .then((cleanup) => {
        if (disposed) cleanup();
        else stop = cleanup;
      });

    return () => {
      disposed = true;
      stop?.();
    };
  }, []);

  const turns = useMemo(() => [...turnsByKey.values()], [turnsByKey]);

  return {
    /** Reads activity for a conversation; it never owns the observer stream. */
    forChannel(channelId: string) {
      return turns.filter((turn) => turn.channelId === channelId);
    },
  };
}
