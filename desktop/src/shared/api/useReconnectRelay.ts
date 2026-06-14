/**
 * Light reconnect hook — clears the relay client's terminal flag and refetches
 * all queries without tearing down the workspace.
 *
 * Deliberately uses `relayClient.preconnect()` + `queryClient.invalidateQueries()`
 * rather than the full `reconnectWorkspace()` path, which unmounts the entire
 * React tree and clears drafts. The goal here is a transparent re-handshake
 * when WARP VPN comes back online; the user should not lose their in-progress
 * compose state.
 */

import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { relayClient } from "@/shared/api/relayClient";

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
      await relayClient.preconnect();
      await queryClient.invalidateQueries();
      // No success toast — the banner auto-hides once the connection state
      // transitions back to "connected", which is the user-visible confirmation.
      return true;
    } catch (err) {
      toast.error("Reconnect failed — check your VPN or network.");
      console.error("[useReconnectRelay] reconnect failed:", err);
      return false;
    } finally {
      inFlightRef.current = false;
      setIsPending(false);
    }
  }, [queryClient]);

  return { reconnect, isPending };
}
