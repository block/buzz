import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { managedAgentRuntimesQueryKey } from "@/features/agents/managedAgentRuntimeHooks";
import { reconcileManagedAgentRuntimes } from "@/shared/api/tauriManagedAgents";

export function useManagedAgentRuntimeReconciliation(
  communities: readonly { relayUrl: string }[],
): void {
  const queryClient = useQueryClient();
  const reconciled = React.useRef(false);

  React.useEffect(() => {
    if (reconciled.current) return;
    reconciled.current = true;

    void reconcileManagedAgentRuntimes(communities)
      .then((runtimes) => {
        queryClient.setQueryData(managedAgentRuntimesQueryKey, runtimes);
      })
      .catch((error) => {
        console.warn(
          "[managed-agent-runtimes] startup reconcile failed:",
          error,
        );
      });
  }, [communities, queryClient]);
}
