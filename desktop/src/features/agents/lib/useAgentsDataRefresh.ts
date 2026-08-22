import { listen } from "@tauri-apps/api/event";
import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import {
  managedAgentsQueryKey,
  personasQueryKey,
  teamsQueryKey,
} from "@/features/agents/hooks";
import { managedAgentRuntimesQueryKey } from "@/features/agents/managedAgentRuntimeHooks";

export const LOCAL_AGENT_DATA_QUERY_KEYS = [
  personasQueryKey,
  teamsQueryKey,
  managedAgentsQueryKey,
] as const;

// Trailing-coalesce local agent-store bursts into one cache refresh. The relay
// directory is deliberately excluded: local persona/team/agent reconciliation
// cannot change remote directory records, and rebuilding that directory is a
// relay-wide operation. Remote data keeps its focused poll and is revalidated
// directly before an agent mention is sent.
const COALESCE_MS = 200;

export function useAgentsDataRefresh(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;

    const unlistenRuntime = listen("managed-agent-runtime-status", () => {
      void queryClient.invalidateQueries({
        queryKey: managedAgentRuntimesQueryKey,
      });
      void queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey });
    });

    const unlisten = listen("agents-data-changed", () => {
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => {
        for (const queryKey of LOCAL_AGENT_DATA_QUERY_KEYS) {
          void queryClient.invalidateQueries({ queryKey });
        }
      }, COALESCE_MS);
    });

    return () => {
      if (timer !== undefined) clearTimeout(timer);
      void unlisten.then((fn) => fn());
      void unlistenRuntime.then((fn) => fn());
    };
  }, [queryClient]);
}
