import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  cacheReconciledManagedAgentRuntimes,
  managedAgentRuntimesQueryKey,
} from "@/features/agents/managedAgentRuntimeHooks";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";
import { reconcileManagedAgentRuntimes } from "@/shared/api/tauriManagedAgents";

export function useManagedAgentRuntimeReconciliation(
  communities: readonly { relayUrl: string }[],
): void {
  const queryClient = useQueryClient();
  const reconciled = React.useRef(false);

  React.useEffect(() => {
    if (reconciled.current) return;
    reconciled.current = true;

    const baseline = queryClient.getQueryData<ManagedAgentRuntimeStatus[]>(
      managedAgentRuntimesQueryKey,
    );
    void reconcileManagedAgentRuntimes(communities)
      .then((runtimes) => {
        cacheReconciledManagedAgentRuntimes(queryClient, baseline, runtimes);
      })
      .catch((error) => {
        console.warn(
          "[managed-agent-runtimes] startup reconcile failed:",
          error,
        );
      });
  }, [communities, queryClient]);
}
