import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  isRelayConnectionDegraded,
  type ConnectionState,
} from "@/shared/api/relayClientShared";

export { isRelayConnectionDegraded };

/**
 * Subscribe to the relay singleton's connection state with a debounce on
 * "transient" transitions.
 *
 * Why debounce? In normal use the socket can flap to `reconnecting` for a
 * second or two between events (initial AUTH, brief network blips) — we don't
 * want a red "connection lost" banner painting itself for every blink. We
 * only surface non-healthy states once they've persisted past
 * `degradedAfterMs` (default 2 seconds). `connected` / `idle` are reported
 * immediately so the UI clears the warning the moment things recover.
 */
export function useRelayConnection(options?: {
  /** Min ms a non-healthy state must persist before being reported. */
  degradedAfterMs?: number;
}): ConnectionState {
  const degradedAfterMs = options?.degradedAfterMs ?? 2_000;
  const [state, setState] = React.useState<ConnectionState>(() =>
    relayClient.getConnectionState(),
  );

  React.useEffect(() => {
    let pendingTimer: number | null = null;

    const clearPending = () => {
      if (pendingTimer !== null) {
        window.clearTimeout(pendingTimer);
        pendingTimer = null;
      }
    };

    const unsubscribe = relayClient.subscribeToConnectionState((next) => {
      clearPending();

      if (next === "connected" || next === "idle" || next === "disconnected") {
        // Healthy or terminal — report immediately.
        setState(next);
        return;
      }

      // Transient degraded states — wait before showing the user a warning.
      pendingTimer = window.setTimeout(() => {
        pendingTimer = null;
        setState(next);
      }, degradedAfterMs);
    });

    return () => {
      clearPending();
      unsubscribe();
    };
  }, [degradedAfterMs]);

  return state;
}
