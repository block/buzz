import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";

/**
 * Agents page: refetch an agent's profile the moment its kind:0 changes (a rename), so cards that show the profile name update
 * live on desktop and web instead of after the 60 s profile staleTime.
 */
export function useAgentProfileLive(pubkeys: readonly string[]) {
  const queryClient = useQueryClient();
  const key = [...new Set(pubkeys.map((p) => p.toLowerCase()))].sort().join(",");
  React.useEffect(() => {
    const authors = key ? key.split(",") : [];
    if (authors.length === 0) return;
    let disposed = false;
    let dispose: (() => Promise<void>) | null = null;
    const refresh = (ev?: { pubkey?: string }) => {
      const pk = ev?.pubkey?.toLowerCase();
      void queryClient.invalidateQueries({
        predicate: (q) => q.queryKey[0] === "user-profile" && (pk ? q.queryKey[1] === pk : authors.includes(String(q.queryKey[1]))),
      });
    };
    void relayClient
      .subscribeLive({ kinds: [0], authors, limit: 0 }, (ev) => refresh(ev as { pubkey?: string }))
      .then((unsubscribe) => {
        if (disposed) void unsubscribe();
        else dispose = unsubscribe;
      })
      .catch(() => {
        /* best effort: the profile still refreshes on its normal schedule */
      });
    const off = relayClient.subscribeToReconnects(() => refresh());
    return () => {
      disposed = true;
      void dispose?.();
      off();
    };
  }, [key, queryClient]);
}
