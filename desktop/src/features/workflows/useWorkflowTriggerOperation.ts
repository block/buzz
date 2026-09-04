import type { TriggerWorkflowResponse } from "@/shared/api/types";
import { toast } from "sonner";
import { useSyncExternalStore } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { workflowTriggerOperations as operations } from "./triggerOperations";

export function useWorkflowTriggerOperation(workflowId: string) {
  const { activeCommunity } = useCommunities();
  const identity = useIdentityQuery().data;
  const queryClient = useQueryClient();
  const scope = {
    expectedRelayUrl: activeCommunity?.relayUrl ?? "",
    expectedSignerPubkey: identity?.pubkey ?? "",
  };
  const key = operations.key(workflowId, scope);
  const state = useSyncExternalStore(operations.subscribe, () =>
    operations.state(key),
  );
  const run = async (newRun = false) => {
    let result: TriggerWorkflowResponse;
    try {
      result = await operations.run(workflowId, scope, newRun);
    } catch (error) {
      // Capacity refusal creates no operation; it still needs an accessible
      // error (normal failures live in the shared inline state).
      if (operations.state(key).status === "idle") toast.error(String(error));
      throw error;
    }
    void queryClient.invalidateQueries({
      queryKey: ["workflow-runs", workflowId],
    });
    return result;
  };
  return {
    ...state,
    run,
    ready: Boolean(scope.expectedRelayUrl && scope.expectedSignerPubkey),
  };
}
