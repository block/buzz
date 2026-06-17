/**
 * Light reconnect hook — clears the relay client's terminal flag and refetches
 * all queries without tearing down the workspace.
 *
 * Deliberately uses `relayClient.preconnect()` + `queryClient.invalidateQueries()`
 * rather than the full `reconnectWorkspace()` path, which unmounts the entire
 * React tree and clears drafts. The goal here is a transparent re-handshake
 * when the transport comes back online; the user should not lose their in-progress
 * compose state.
 */

import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

import { relayClient } from "@/shared/api/relayClient";

const RECONNECT_HOOK_TIMEOUT_MS = 20_000;
const RELAY_PRECONNECT_TIMEOUT_MS = 15_000;

function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  let timeoutId: number | null = null;
  const timeout = new Promise<never>((_, reject) => {
    timeoutId = window.setTimeout(() => {
      reject(new Error(`${label} timed out after ${timeoutMs}ms`));
    }, timeoutMs);
  });

  return Promise.race([promise, timeout]).finally(() => {
    if (timeoutId !== null) {
      window.clearTimeout(timeoutId);
    }
  });
}

export function useReconnectRelay(): {
  reconnect: () => Promise<boolean>;
  isPending: boolean;
} {
  const queryClient = useQueryClient();
  const [isPending, setIsPending] = React.useState(false);
  // Ref-based guard prevents a second fire if the user clicks while the first
  // preconnect is still in flight — stale closure over `isPending` state would
  // allow a double-trigger between the click and the next render.
  const inFlightRef = React.useRef(false);

  const reconnect = React.useCallback(async () => {
    if (inFlightRef.current) return false;
    inFlightRef.current = true;
    setIsPending(true);
    try {
      // Run the transport-layer reconnect hook configured by internal builds.
      // No-op in OSS builds. Non-fatal — transport failure shouldn't block relay reconnect.
      try {
        await withTimeout(
          invoke("relay_reconnect_hook"),
          RECONNECT_HOOK_TIMEOUT_MS,
          "reconnect hook",
        );
      } catch (err) {
        console.warn("[useReconnectRelay] reconnect hook failed:", err);
      }

      await withTimeout(
        relayClient.preconnect(),
        RELAY_PRECONNECT_TIMEOUT_MS,
        "relay preconnect",
      );
      // Let callers render the recovered/connected state before refetching the
      // sidebar data. The refetch can briefly swap the sidebar into loading UI.
      window.setTimeout(() => {
        void queryClient.invalidateQueries().catch((error) => {
          console.error(
            "[useReconnectRelay] failed to refresh queries after reconnect:",
            error,
          );
        });
      }, 0);
      // No success toast — the banner auto-hides once the connection state
      // transitions back to "connected", which is the user-visible confirmation.
      return true;
    } catch (err) {
      toast.error("Reconnect failed — check your network.");
      console.error("[useReconnectRelay] reconnect failed:", err);
      return false;
    } finally {
      inFlightRef.current = false;
      setIsPending(false);
    }
  }, [queryClient]);

  return { reconnect, isPending };
}
