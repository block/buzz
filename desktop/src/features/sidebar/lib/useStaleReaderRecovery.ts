import React from "react";

import type { FetchResult } from "./sidebarSyncWatermark.ts";

/**
 * Shared bounded-backoff retry effect for all four sidebar sync hooks.
 *
 * Polls the relay at 5 → 10 → 30 → 60 s (steady at 60 s) so a stale view
 * recovers without an edit-kick or reconnect.  Fires immediately on
 * `visibilitychange` to "visible" so returning from background converges
 * quickly.  Single-flight: skips ticks while a fetch is in progress.
 *
 * **Pending-first guard (apply-time, not response-time):**
 * `hasPending()` and the effect lifetime are re-evaluated inside the React
 * state updater, after `fetch()` resolves but before `applyRemote` runs.
 * This closes the window where a local edit updater queued before the remote
 * response could be executed after it: React may execute queued updaters in
 * the same batch, so checking only at response time is insufficient.
 *
 * **Mutation revision guard:**
 * A revision counter is snapshotted at the start of each `fetch()` call and
 * rechecked inside the updater.  If a local mutation landed while the request
 * was in flight the response is discarded — no old relay read can roll back a
 * successfully published local edit.
 *
 * Both guards apply inside `makeUpdater(data)`, the lane-supplied function
 * that wraps `applyRemote` and `setStore`.  The checks happen before
 * `applyRemote`'s returned updater runs, so `cancelPending*Publish` inside
 * `applyRemote` is never reached on a fenced path.
 *
 * Residual: a pending edit whose publish permanently fails and is never
 * resolved by a reconnect will defer recovery indefinitely.  Main today has
 * zero recovery in that state, so this is not a regression.
 *
 * @param enabled         Must be true for the effect to activate
 *                        (e.g. `!!pubkey && !!relayUrl`).
 * @param fetch           Calls the manager's `fetchRemote*()` for this lane.
 * @param hasPending      Returns true when a local unpublished edit is in
 *                        flight.  Called inside the state updater.
 * @param getRevision     Returns the current local-mutation revision counter.
 *                        Called at fetch start (snapshot) and inside the
 *                        updater (recheck).
 * @param makeUpdater     Given the fetched data, returns a React state updater
 *                        that applies `applyRemote(data)` to the previous
 *                        store.  Called only when both guards pass.
 * @param setStore        The hook's React state setter; called with the fenced
 *                        updater produced by `makeUpdater`.
 * @param pubkey          Identity key — forces the recovery effect to restart
 *                        when the active pubkey changes.  Pass whenever the
 *                        `makeUpdater` closure does not directly capture pubkey
 *                        (e.g. if `applyRemote` only depends on pubkey but the
 *                        outer `makeUpdater` wraps a stable ref).
 * @param relayUrl        Relay key — forces the recovery effect to restart when
 *                        the relay changes.  Required for stars/mutes whose
 *                        `applyRemote` does not capture relayUrl; without it a
 *                        still-mounted hook switching relay A→B would keep
 *                        applying A's stale fetch after the switch.
 */
export function useStaleReaderRecovery<T, S>({
  enabled,
  fetch,
  hasPending,
  getRevision,
  makeUpdater,
  setStore,
  pubkey,
  relayUrl,
}: {
  enabled: boolean;
  fetch: () => Promise<FetchResult<T> | undefined> | undefined;
  hasPending: () => boolean;
  getRevision: () => number;
  makeUpdater: (data: T) => (prev: S) => S;
  setStore: React.Dispatch<React.SetStateAction<S>>;
  /** Passed to bind the effect lifetime to identity; see `@param pubkey`. */
  pubkey?: string | undefined;
  /** Passed to bind the effect lifetime to relay; see `@param relayUrl`. */
  relayUrl?: string | undefined;
}): void {
  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey and relayUrl are intentional — they bind the recovery effect lifetime to identity and relay so a still-mounted hook switching relay A→B cancels the old effect (stars/mutes applyRemote does not capture relayUrl in its deps)
  React.useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let inFlight = false;
    const BACKOFF_STEPS = [5_000, 10_000, 30_000, 60_000];
    let stepIndex = 0;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const tick = async () => {
      if (cancelled || inFlight) return;
      inFlight = true;
      // Snapshot the local-mutation revision before the async fetch so we can
      // detect a mutation that landed while the request was in flight.
      const revisionAtFetch = getRevision();
      try {
        // A read started while an edit is pending can return the pre-edit
        // blob after the edit publishes and pending clears; skip it.
        const result = hasPending() ? undefined : await fetch();
        if (!cancelled && result?.status === "found") {
          const remoteUpdater = makeUpdater(result.data);
          // Apply inside a state updater so hasPending() and the cancelled /
          // revision guards are evaluated at execution time, not at response
          // time.  This closes the React-batch window where a local edit
          // updater could be queued before this remote updater but execute
          // after it.
          setStore((prev) => {
            if (cancelled) return prev;
            if (hasPending()) return prev;
            if (getRevision() !== revisionAtFetch) return prev;
            return remoteUpdater(prev);
          });
        }
      } finally {
        inFlight = false;
      }
      if (!cancelled) {
        const delay = BACKOFF_STEPS[
          Math.min(stepIndex, BACKOFF_STEPS.length - 1)
        ] as number;
        stepIndex = Math.min(stepIndex + 1, BACKOFF_STEPS.length - 1);
        timer = setTimeout(() => {
          void tick();
        }, delay);
      }
    };

    const onVisible = () => {
      if (document.visibilityState !== "visible") return;
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
      stepIndex = 0;
      void tick();
    };

    document.addEventListener("visibilitychange", onVisible);
    // Kick off the first tick immediately so bootstrap failures are recovered
    // without waiting a full backoff interval.
    void tick();

    return () => {
      cancelled = true;
      document.removeEventListener("visibilitychange", onVisible);
      if (timer !== null) clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    enabled,
    fetch,
    hasPending,
    getRevision,
    makeUpdater,
    setStore,
    pubkey,
    relayUrl,
  ]);
}
