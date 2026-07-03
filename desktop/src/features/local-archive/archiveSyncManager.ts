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
    await this.resubscribeAll();
    this.offSubscriptionChange = this.deps.onSubscriptionChange(() => {
      void this.resubscribeAll();
    });
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

  private async resubscribeAll(): Promise<void> {
    if (this.destroyed) return;

    let subs: SaveSubscription[];
    try {
      subs = await this.deps.listSaveSubscriptions();
    } catch (err) {
      console.warn("[archiveSyncManager] list_save_subscriptions failed:", err);
      return;
    }

    if (this.destroyed) return;

    // Full keys (scope+kinds) we want after reload.
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
    for (const sub of subs) {
      const key = subKey(sub.scopeType, sub.scopeValue, sub.kinds);
      if (this.active.has(key)) continue;

      const scopeType = sub.scopeType;
      const scopeValue = sub.scopeValue;
      const filter = buildFilter(sub);

      // Await the subscribe call so we only mark the key active after it
      // succeeds. On failure the key is left absent so a future resubscribeAll
      // (triggered by a config change or app restart) will retry it.
      // JS is single-threaded, so no re-entrancy between the await and the
      // active.set below — a concurrent resubscribeAll would have already
      // returned or will run only after this microtask yields.
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

      if (this.destroyed) {
        // Manager was destroyed while we were awaiting — tear down immediately.
        void dispose();
        continue;
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
