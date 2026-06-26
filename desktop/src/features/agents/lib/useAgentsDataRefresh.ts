import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import {
  managedAgentsQueryKey,
  personasQueryKey,
  relayAgentsQueryKey,
  teamsQueryKey,
} from "@/features/agents/hooks";

// Trailing-coalesce window: a backfill burst (up to 500 inbound events fed
// one-by-one through reconcile) fires one `agents-data-changed` per event.
// Collapsing them into a single invalidate after the burst settles keeps the
// refetch off React Query's implicit in-flight dedup and avoids redundant
// disk-read IPC.
const COALESCE_MS = 200;

// Invalidate the live Agents-tab queries when the backend signals that inbound
// relay events changed the on-disk agents data. Mounted once at the app root
// with empty deps — invalidation is global and has no reason to be
// pubkey-scoped, so it must NOT live inside the pubkey-keyed `usePersonaSync`
// (re-registering per identity switch would leak a listener each time).
export function useAgentsDataRefresh(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;

    const unlisten = listen("agents-data-changed", () => {
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => {
        void queryClient.invalidateQueries({ queryKey: personasQueryKey });
        void queryClient.invalidateQueries({ queryKey: teamsQueryKey });
        void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
        void queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey });
      }, COALESCE_MS);
    });

    return () => {
      if (timer !== undefined) clearTimeout(timer);
      void unlisten.then((fn) => fn());
    };
  }, [queryClient]);
}
