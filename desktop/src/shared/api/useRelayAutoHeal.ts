import * as React from "react";

import { useQueryClient } from "@tanstack/react-query";

import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";

const AUTO_HEAL_MIN_INTERVAL_MS = 15_000;

/**
 * Auto-heal: when the connection recovers from a degraded state, invalidate
 * all queries so errored queries (e.g. messages, which don't poll) refetch
 * automatically without requiring a manual reconnect action.
 *
 * Rate-limited to prevent a flappy connection (e.g. VPN toggling) from
 * firing an unfiltered invalidation — ~20-40 requests across active queries
 * with retry:1 — every time the relay briefly recovers.
 */
export function useRelayAutoHeal(): void {
  const queryClient = useQueryClient();
  const connectionState = useRelayConnection();
  const prevConnectionStateRef = React.useRef(connectionState);
  const lastAutoHealAtRef = React.useRef(0);

  React.useEffect(() => {
    const prev = prevConnectionStateRef.current;
    prevConnectionStateRef.current = connectionState;
    if (isRelayConnectionDegraded(prev) && connectionState === "connected") {
      const now = Date.now();
      if (now - lastAutoHealAtRef.current < AUTO_HEAL_MIN_INTERVAL_MS) {
        return;
      }
      lastAutoHealAtRef.current = now;
      void queryClient.invalidateQueries();
    }
  }, [connectionState, queryClient]);
}
