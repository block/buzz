import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  classifyReconcileResult,
  connectionTargetCommunityRelays,
  pendingReconcileRelays,
  reconcileRetryDelayMs,
} from "@/features/agents/managedAgentReconciliationPlan";
import {
  cacheReconciledManagedAgentRuntimes,
  managedAgentRuntimesQueryKey,
} from "@/features/agents/managedAgentRuntimeHooks";
import { connectionTargetUrl } from "@/features/agents/managedAgentRuntimeStatus";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";
import { reconcileManagedAgentRuntimes } from "@/shared/api/tauriManagedAgents";

/**
 * Bootstrap a lazy harness pair for every auto-start local agent in every
 * configured community, incrementally and with retry.
 *
 * Reconciliation bookkeeping is keyed by connection target: each configured relay is
 * reconciled once it appears (so adding a community mid-session spawns pairs
 * there without needing the add flow to also switch communities), and a relay
 * whose reconcile fails is retried with a capped backoff (5s / 30s / 2m) rather
 * than left un-spawned until the next switch or relaunch. Relays that reconcile
 * cleanly are never re-hit; once nothing is outstanding, no timer is left
 * running.
 */
export function useManagedAgentRuntimeReconciliation(
  communities: readonly { relayUrl: string }[],
): void {
  const queryClient = useQueryClient();
  // Connection targets that have reconciled cleanly — never re-hit.
  const reconciledRef = React.useRef<Set<string>>(new Set());
  // Connection targets with a reconcile call in flight — not re-dispatched.
  const inFlightRef = React.useRef<Set<string>>(new Set());
  // Consecutive failures per connection target, driving the retry backoff.
  const failuresRef = React.useRef<Map<string, number>>(new Map());
  const retryTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  React.useEffect(() => {
    let cancelled = false;

    const clearRetryTimer = () => {
      if (retryTimerRef.current !== null) {
        clearTimeout(retryTimerRef.current);
        retryTimerRef.current = null;
      }
    };

    const scheduleRetry = (failed: readonly string[]) => {
      // One shared timer fires at the soonest per-relay backoff; every failing
      // relay is retried together (reconcile is idempotent), so re-hitting a
      // longer-backoff relay early is harmless.
      let soonest: number | null = null;
      for (const relay of failed) {
        const nextCount = (failuresRef.current.get(relay) ?? 0) + 1;
        failuresRef.current.set(relay, nextCount);
        const delay = reconcileRetryDelayMs(nextCount);
        if (delay !== null && (soonest === null || delay < soonest)) {
          soonest = delay;
        }
      }
      clearRetryTimer();
      if (soonest === null) return; // all failing relays hit the retry cap
      retryTimerRef.current = setTimeout(() => {
        retryTimerRef.current = null;
        if (!cancelled) runReconcile();
      }, soonest);
    };

    const runReconcile = () => {
      const targetToRequested = connectionTargetCommunityRelays(
        communities,
        connectionTargetUrl,
      );
      // Forget bookkeeping for relays that are no longer configured so the sets
      // stay bounded and re-adding a removed community reconciles it afresh.
      for (const done of [...reconciledRef.current]) {
        if (!targetToRequested.has(done)) reconciledRef.current.delete(done);
      }
      for (const failing of [...failuresRef.current.keys()]) {
        if (!targetToRequested.has(failing)) {
          failuresRef.current.delete(failing);
        }
      }

      const pending = pendingReconcileRelays(
        targetToRequested,
        reconciledRef.current,
        inFlightRef.current,
      );
      if (pending.length === 0) {
        clearRetryTimer();
        return;
      }

      for (const relay of pending) inFlightRef.current.add(relay);
      const targets = pending.map((relay) => ({
        relayUrl: targetToRequested.get(relay) as string,
      }));
      const baseline = queryClient.getQueryData<ManagedAgentRuntimeStatus[]>(
        managedAgentRuntimesQueryKey,
      );

      void reconcileManagedAgentRuntimes(targets)
        .then((runtimes) => {
          cacheReconciledManagedAgentRuntimes(queryClient, baseline, runtimes);
          return classifyReconcileResult(
            pending,
            runtimes,
            connectionTargetUrl,
          );
        })
        .catch((error) => {
          console.warn("[managed-agent-runtimes] reconcile failed:", error);
          return classifyReconcileResult(pending, null, connectionTargetUrl);
        })
        .then(({ succeeded, failed }) => {
          for (const relay of pending) inFlightRef.current.delete(relay);
          for (const relay of succeeded) {
            reconciledRef.current.add(relay);
            failuresRef.current.delete(relay);
          }
          if (!cancelled && failed.length > 0) scheduleRetry(failed);
        });
    };

    runReconcile();

    return () => {
      cancelled = true;
      clearRetryTimer();
    };
  }, [communities, queryClient]);
}
