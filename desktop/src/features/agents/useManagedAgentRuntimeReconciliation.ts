import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { reconcileRetryDelayMs } from "@/features/agents/managedAgentReconciliationPlan";
import {
  cacheReconciledManagedAgentRuntimes,
  managedAgentRuntimesQueryKey,
} from "@/features/agents/managedAgentRuntimeHooks";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";
import { reconcileManagedAgentRuntimes } from "@/shared/api/tauriManagedAgents";

/**
 * Bootstrap a lazy harness pair for every auto-start local agent in the active
 * workspace, with retry on failure.
 *
 * Under the active-scope-only runtime policy the backend derives the sole
 * target relay from the captured active scope — no community list is passed.
 * Reconciliation runs once on mount; if it fails it is retried with a capped
 * backoff (5s / 30s / 2m). Once it succeeds, no timer is left running.
 *
 * The `activeCommunityKey` parameter is a stable key that changes whenever the
 * active workspace changes (e.g. `"${communityId}-${reinitKey}"`). A workspace
 * switch unmounts/remounts the effect, resetting the reconcile state and
 * re-running for the new scope.
 */
export function useManagedAgentRuntimeReconciliation(
  activeCommunityKey: string,
): void {
  const queryClient = useQueryClient();
  const failureCountRef = React.useRef(0);
  const retryTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  // activeCommunityKey changes on workspace switch, resetting the effect.
  // biome-ignore lint/correctness/useExhaustiveDependencies: activeCommunityKey is an intentional trigger dependency — the effect must re-run on workspace switch to reset reconcile state for the new scope.
  React.useEffect(() => {
    let cancelled = false;
    failureCountRef.current = 0;

    const clearRetryTimer = () => {
      if (retryTimerRef.current !== null) {
        clearTimeout(retryTimerRef.current);
        retryTimerRef.current = null;
      }
    };

    const scheduleRetry = () => {
      const nextCount = failureCountRef.current + 1;
      failureCountRef.current = nextCount;
      const delay = reconcileRetryDelayMs(nextCount);
      clearRetryTimer();
      if (delay === null) return; // retry cap exhausted
      retryTimerRef.current = setTimeout(() => {
        retryTimerRef.current = null;
        if (!cancelled) runReconcile();
      }, delay);
    };

    const runReconcile = () => {
      const baseline = queryClient.getQueryData<ManagedAgentRuntimeStatus[]>(
        managedAgentRuntimesQueryKey,
      );
      void reconcileManagedAgentRuntimes()
        .then((runtimes) => {
          if (!cancelled) {
            cacheReconciledManagedAgentRuntimes(
              queryClient,
              baseline,
              runtimes,
            );
          }
        })
        .catch((error) => {
          console.warn("[managed-agent-runtimes] reconcile failed:", error);
          if (!cancelled) scheduleRetry();
        });
    };

    runReconcile();

    return () => {
      cancelled = true;
      clearRetryTimer();
    };
  }, [activeCommunityKey, queryClient]);
}
