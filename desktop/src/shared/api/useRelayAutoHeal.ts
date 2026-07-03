import * as React from "react";

import { useQueryClient } from "@tanstack/react-query";

import type { ConnectionState } from "@/shared/api/relayClientShared";
import {
  isRelayConnectionDegraded,
  useRelayConnection,
} from "@/shared/api/useRelayConnection";

export const AUTO_HEAL_MIN_INTERVAL_MS = 15_000;

/**
 * Tracks degraded→connected transitions and fires `onHeal` at most once per
 * `minIntervalMs`. When a transition is suppressed by the rate limiter, a
 * deferred heal is scheduled for the remaining window so the *last* recovery
 * always wins — stale query errors do not persist after reconnect.
 *
 * Injectable deps make this testable without React or DOM.
 */
export class RelayAutoHealScheduler {
  private lastHealAt = -Infinity;
  private deferredId: number | null = null;
  private readonly onHeal: () => void;
  private readonly minIntervalMs: number;
  private readonly setTimeoutFn: (fn: () => void, ms: number) => number;
  private readonly clearTimeoutFn: (id: number) => void;
  private readonly nowFn: () => number;

  constructor(
    onHeal: () => void,
    minIntervalMs: number,
    setTimeoutFn: (fn: () => void, ms: number) => number,
    clearTimeoutFn: (id: number) => void,
    nowFn: () => number = () => Date.now(),
  ) {
    this.onHeal = onHeal;
    this.minIntervalMs = minIntervalMs;
    this.setTimeoutFn = setTimeoutFn;
    this.clearTimeoutFn = clearTimeoutFn;
    this.nowFn = nowFn;
  }

  /**
   * Call on every connection-state change. Fires `onHeal` (immediately or
   * deferred) when the transition is degraded→connected.
   */
  onTransition(prev: ConnectionState, next: ConnectionState): void {
    if (!(isRelayConnectionDegraded(prev) && next === "connected")) return;

    // A new recovery supersedes any pending deferred heal.
    if (this.deferredId !== null) {
      this.clearTimeoutFn(this.deferredId);
      this.deferredId = null;
    }

    const now = this.nowFn();
    const elapsed = now - this.lastHealAt;

    if (elapsed < this.minIntervalMs) {
      // Rate-limited — schedule for when the window expires.
      const remaining = this.minIntervalMs - elapsed;
      this.deferredId = this.setTimeoutFn(() => {
        this.deferredId = null;
        this.lastHealAt = this.nowFn();
        this.onHeal();
      }, remaining);
    } else {
      this.lastHealAt = now;
      this.onHeal();
    }
  }

  /** Cancel any pending deferred heal (call on unmount). */
  dispose(): void {
    if (this.deferredId !== null) {
      this.clearTimeoutFn(this.deferredId);
      this.deferredId = null;
    }
  }
}

/**
 * Auto-heal: when the connection recovers from a degraded state, invalidate
 * all queries so errored queries (e.g. messages, which don't poll) refetch
 * automatically without requiring a manual reconnect action.
 *
 * Rate-limited to prevent a flappy connection (e.g. VPN toggling) from
 * firing an unfiltered invalidation — ~20-40 requests across active queries
 * with retry:1 — every time the relay briefly recovers.
 *
 * When a recovery is suppressed by the rate limiter (an earlier flap consumed
 * the budget), a deferred heal is scheduled for the remaining window so the
 * *last* recovery always wins and stale query errors do not persist.
 */
export function useRelayAutoHeal(): void {
  const queryClient = useQueryClient();
  const connectionState = useRelayConnection();
  const prevConnectionStateRef = React.useRef(connectionState);
  const schedulerRef = React.useRef<RelayAutoHealScheduler | null>(null);

  if (schedulerRef.current === null) {
    schedulerRef.current = new RelayAutoHealScheduler(
      () => void queryClient.invalidateQueries(),
      AUTO_HEAL_MIN_INTERVAL_MS,
      window.setTimeout.bind(window),
      window.clearTimeout.bind(window),
    );
  }

  React.useEffect(() => {
    return () => {
      schedulerRef.current?.dispose();
    };
  }, []);

  React.useEffect(() => {
    const prev = prevConnectionStateRef.current;
    prevConnectionStateRef.current = connectionState;
    schedulerRef.current?.onTransition(prev, connectionState);
  }, [connectionState]);
}
