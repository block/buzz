import { useMutation, useQueryClient } from "@tanstack/react-query";

import {
  managedAgentsQueryKey,
  relayAgentsQueryKey,
} from "@/features/agents/hooks";
import { applyManagedAgentBackendChange } from "@/features/agents/lib/changeManagedAgentBackend";
import type { ManagedAgent } from "@/shared/api/types";

/**
 * Backend swap for an existing agent (edit dialog "Run on"). Wraps the whole
 * orchestration — teardown transition plus, for execution-node targets, the
 * authoritative deploy — so pending state covers the deploy leg too.
 *
 * Lives outside `hooks.ts` only for the file-size guard; it shares that
 * module's query keys and cache conventions.
 */
export function useChangeManagedAgentBackendMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (input: Parameters<typeof applyManagedAgentBackendChange>[0]) =>
      applyManagedAgentBackendChange(input),
    onSuccess: (result) => {
      if (result.cancelled) return;
      queryClient.setQueryData<ManagedAgent[]>(
        managedAgentsQueryKey,
        (current) => {
          if (!current) return current;
          return current.map((agent) =>
            agent.pubkey === result.agent.pubkey ? result.agent : agent,
          );
        },
      );
    },
    onSettled: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
        queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
      ]);
    },
  });
}
