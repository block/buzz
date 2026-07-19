import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  listManagedAgentRuntimes,
  restartManagedAgentRuntime,
  startManagedAgentRuntime,
  stopManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";

export const managedAgentRuntimesQueryKey = ["managed-agent-runtimes"] as const;

export function useManagedAgentRuntimesQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: managedAgentRuntimesQueryKey,
    queryFn: listManagedAgentRuntimes,
  });
}

export function useManagedAgentRuntimeAction() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      action,
      pubkey,
      relayUrl,
    }: {
      action: "start" | "stop" | "restart";
      pubkey: string;
      relayUrl: string;
    }) => {
      if (action === "stop") return stopManagedAgentRuntime(pubkey, relayUrl);
      if (action === "restart") {
        return restartManagedAgentRuntime(pubkey, relayUrl);
      }
      return startManagedAgentRuntime(pubkey, relayUrl);
    },
    onSuccess: (runtime) => {
      queryClient.setQueryData<ManagedAgentRuntimeStatus[]>(
        managedAgentRuntimesQueryKey,
        (current = []) => {
          const index = current.findIndex(
            (candidate) =>
              candidate.pubkey === runtime.pubkey &&
              candidate.relayUrl === runtime.relayUrl,
          );
          if (index === -1) return [...current, runtime];
          return current.map((candidate, candidateIndex) =>
            candidateIndex === index ? runtime : candidate,
          );
        },
      );
    },
  });
}
