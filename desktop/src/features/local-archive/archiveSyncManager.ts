import { relayClient as defaultRelayClient } from "@/shared/api/relayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import {
  archiveEvents as defaultArchiveEvents,
  listSaveSubscriptions as defaultListSaveSubscriptions,
  onSubscriptionChange as defaultOnSubscriptionChange,
  type SaveSubscription,
  type ScopeType,
} from "@/shared/api/tauriArchive";

// ── Constants ─────────────────────────────────────────────────────────────────

const FLUSH_BATCH_SIZE = 25;
const FLUSH_IDLE_MS = 2_000;

// ── Types ─────────────────────────────────────────────────────────────────────

/** Dependency injection interface — production uses module singletons; tests inject fakes. */
export interface ArchiveSyncDeps {
  relayClient: {
    subscribeLive: (
      filter: RelaySubscriptionFilter,
      onEvent: (event: RelayEvent) => void,
    ) => Promise<() => Promise<void>>;
  };
  listSaveSubscriptions: () => Promise<SaveSubscription[]>;
  archiveEvents: (
    candidates: Array<{
      rawEventJson: string;
      matchedScope: { scopeType: ScopeType; scopeValue: string };
    }>,
  ) => Promise<unknown>;
  onSubscriptionChange: (listener: () => void) => () => void;
  flushBatchSize?: number;
  flushIdleMs?: number;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function buildFilter(sub: SaveSubscription): RelaySubscriptionFilter {
  const base = { kinds: sub.kinds, limit: 0 } as const;
  switch (sub.scopeType) {
    case "channel_h":
      return { ...base, "#h": [sub.scopeValue] };
    case "owner_p":
      return { ...base, "#p": [sub.scopeValue] };
    case "referenced_e":
      return { ...base, "#e": [sub.scopeValue] };
  }
}

/** Stable key encoding scope + kinds — ensures kinds changes trigger resubscribe. */
function subKey(
  scopeType: ScopeType,
  scopeValue: string,
  kinds: number[],
): string {
  const sortedKinds = [...kinds].sort((a, b) => a - b).join(",");
  return `${scopeType}:${scopeValue}:${sortedKinds}`;
}

/** Scope-only key used to find and tear down a stale sub when kinds change. */
function scopeKey(scopeType: ScopeType, scopeValue: string): string {
  return `${scopeType}:${scopeValue}`;
}

// ── ArchiveSyncManager ────────────────────────────────────────────────────────

/**
 * Always-on manager that opens one live relay subscription per saved archive
 * config and forwards matched events to `archive_events` in debounced batches.
 *
 * Lifecycle: created once at app-shell mount (see `useArchiveSync`), destroyed
 * on workspace switch. Resubscribes automatically when subscriptions change
 * via the module-level notifier in `tauriArchive.ts`.
 *
 * Accepts optional `deps` for testing — production callers pass nothing.
 */
export class ArchiveSyncManager {
  private readonly deps: Required<
    Omit<ArchiveSyncDeps, "flushBatchSize" | "flushIdleMs">
  >;
  private readonly flushBatchSize: number;
  private readonly flushIdleMs: number;

  // full subKey (scope+kinds) → unsub
  private active = new Map<string, () => Promise<void>>();
  // Single-flight reload state — exactly one doResubscribe body runs at a time.
  // Any reload request arriving while one is running sets reloadPending so that
  // the loop runs one additional full pass after the current one finishes.
  // This structural guarantee makes concurrent list/subscribe awaits impossible,
  // eliminating all interleaving defects without per-boundary guards.
  private reloading = false;
  private reloadPending = false;
  private buffer: Array<{
    rawEventJson: string;
    matchedScope: { scopeType: ScopeType; scopeValue: string };
  }> = [];
  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private destroyed = false;
  private offSubscriptionChange: (() => void) | null = null;

  constructor(deps?: ArchiveSyncDeps) {
    this.deps = {
      relayClient: deps?.relayClient ?? defaultRelayClient,
      listSaveSubscriptions:
        deps?.listSaveSubscriptions ?? defaultListSaveSubscriptions,
      archiveEvents: deps?.archiveEvents ?? defaultArchiveEvents,
      onSubscriptionChange:
        deps?.onSubscriptionChange ?? defaultOnSubscriptionChange,
    };
    this.flushBatchSize = deps?.flushBatchSize ?? FLUSH_BATCH_SIZE;
    this.flushIdleMs = deps?.flushIdleMs ?? FLUSH_IDLE_MS;
  }

  async start(): Promise<void> {
    // Register the change listener before the initial load so that any
    // subscription change arriving while the first pass is running sets
    // reloadPending and gets picked up by the coalescing loop.
    this.offSubscriptionChange = this.deps.onSubscriptionChange(() => {
      this.resubscribeAll();
    });
    await this.runReloadLoop();
  }

  destroy(): void {
    this.destroyed = true;
    if (this.flushTimer !== null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    this.offSubscriptionChange?.();
    this.offSubscriptionChange = null;
    // Flush any buffered events before tearing down.
    if (this.buffer.length > 0) {
      const toFlush = this.buffer.splice(0);
      void this.deps.archiveEvents(toFlush).catch((err: unknown) => {
        console.warn("[archiveSyncManager] flush on destroy failed:", err);
      });
    }
    for (const [, unsub] of this.active) {
      void unsub();
    }
    this.active.clear();
  }

  /**
   * Request a reload of all save subscriptions.
   *
   * If no reload is currently running, starts one immediately via
   * `runReloadLoop`. If one is already running, sets `reloadPending` so
   * the loop runs exactly one additional full pass after the current pass
   * completes — no matter how many changes arrive mid-run, they coalesce
   * into a single follow-up pass.
   */
  private resubscribeAll(): void {
    if (this.reloading) {
      this.reloadPending = true;
      return;
    }
    void this.runReloadLoop();
  }

  /**
   * Runs `doResubscribe` in a loop, draining any reload requests that
   * arrive while a pass is in flight. The loop terminates once no reload
   * is pending and the manager has not been destroyed.
   *
   * Awaited directly by `start()` for the initial load.
   */
  private async runReloadLoop(): Promise<void> {
    this.reloading = true;
    try {
      do {
        this.reloadPending = false;
        await this.doResubscribe();
      } while (this.reloadPending && !this.destroyed);
    } finally {
      this.reloading = false;
    }
  }

  /**
   * One full reload pass: fetch the current subscription list, tear down
   * removed or kind-changed subscriptions, and open new ones.
   *
   * Only one instance of this method ever runs at a time (enforced by
   * `runReloadLoop`). That single-flight guarantee makes concurrent
   * list/subscribe awaits structurally impossible.
   */
  private async doResubscribe(): Promise<void> {
    if (this.destroyed) return;

    let subs: SaveSubscription[];
    try {
      subs = await this.deps.listSaveSubscriptions();
    } catch (err) {
      console.warn("[archiveSyncManager] list_save_subscriptions failed:", err);
      return;
    }

    if (this.destroyed) return;

    // Full keys (scope+kinds) for the current subscription list.
    const wanted = new Set(
      subs.map((s) => subKey(s.scopeType, s.scopeValue, s.kinds)),
    );

    // Tear down subscriptions that are no longer needed or whose kinds changed.
    // A stale entry whose scope is still present but with different kinds will
    // have a different full key and be absent from `wanted`, so it gets torn
    // down here and recreated below with the new filter.
    for (const [key, unsub] of this.active) {
      if (!wanted.has(key)) {
        void unsub();
        this.active.delete(key);
      }
    }

    // Open new subscriptions for any full key not already active.
    // No concurrency guards are needed here: single-flight serialization
    // ensures this loop body is the only async path running, so nothing
    // can add a duplicate entry to `active` between iterations.
    for (const sub of subs) {
      if (this.destroyed) return;

      const key = subKey(sub.scopeType, sub.scopeValue, sub.kinds);
      if (this.active.has(key)) continue;

      const scopeType = sub.scopeType;
      const scopeValue = sub.scopeValue;
      const filter = buildFilter(sub);

      let dispose: (() => Promise<void>) | undefined;
      try {
        dispose = await this.deps.relayClient.subscribeLive(
          filter,
          (event: RelayEvent) => {
            this.enqueue(event, scopeType, scopeValue);
          },
        );
      } catch (err) {
        console.warn(
          `[archiveSyncManager] subscribeLive failed for ${scopeKey(scopeType, scopeValue)}:`,
          err,
        );
        // Do NOT add key to active — next resubscribeAll will retry.
        continue;
      }

      // A destroy() call may have arrived while subscribeLive was pending.
      // Tear down the just-opened subscription and bail out.
      if (this.destroyed) {
        void dispose();
        return;
      }

      this.active.set(key, dispose);
    }
  }

  private enqueue(
    event: RelayEvent,
    scopeType: ScopeType,
    scopeValue: string,
  ): void {
    if (this.destroyed) return;
    this.buffer.push({
      rawEventJson: JSON.stringify(event),
      matchedScope: { scopeType, scopeValue },
    });
    if (this.buffer.length >= this.flushBatchSize) {
      this.flush();
    } else {
      this.scheduleFlush();
    }
  }

  private scheduleFlush(): void {
    if (this.flushTimer !== null) return;
    this.flushTimer = setTimeout(() => {
      this.flushTimer = null;
      this.flush();
    }, this.flushIdleMs);
  }

  private flush(): void {
    if (this.flushTimer !== null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    if (this.buffer.length === 0) return;
    const batch = this.buffer.splice(0);
    void this.deps.archiveEvents(batch).catch((err: unknown) => {
      console.warn("[archiveSyncManager] archive_events failed:", err);
    });
  }
}

// ── React hook ────────────────────────────────────────────────────────────────

import * as React from "react";

/**
 * Starts the ArchiveSyncManager at app-shell mount and tears it down when the
 * component unmounts (workspace switch). No return value needed — the manager
 * runs entirely in the background.
 */
export function useArchiveSync(): void {
  React.useEffect(() => {
    const manager = new ArchiveSyncManager();
    void manager.start();
    return () => {
      manager.destroy();
    };
  }, []);
}
