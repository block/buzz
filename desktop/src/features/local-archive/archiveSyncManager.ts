import { relayClient } from "@/shared/api/relayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import {
  archiveEvents,
  listSaveSubscriptions,
  onSubscriptionChange,
  type SaveSubscription,
  type ScopeType,
} from "@/shared/api/tauriArchive";

// ── Constants ─────────────────────────────────────────────────────────────────

const FLUSH_BATCH_SIZE = 25;
const FLUSH_IDLE_MS = 2_000;

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

function subKey(scopeType: ScopeType, scopeValue: string): string {
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
 */
export class ArchiveSyncManager {
  private unsubs = new Map<string, () => Promise<void>>();
  private buffer: Array<{
    rawEventJson: string;
    matchedScope: { scopeType: ScopeType; scopeValue: string };
  }> = [];
  private flushTimer: ReturnType<typeof setTimeout> | null = null;
  private destroyed = false;
  private offSubscriptionChange: (() => void) | null = null;

  async start(): Promise<void> {
    await this.resubscribeAll();
    this.offSubscriptionChange = onSubscriptionChange(() => {
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
      void archiveEvents(toFlush).catch((err: unknown) => {
        console.warn("[archiveSyncManager] flush on destroy failed:", err);
      });
    }
    for (const [, unsub] of this.unsubs) {
      void unsub();
    }
    this.unsubs.clear();
  }

  private async resubscribeAll(): Promise<void> {
    if (this.destroyed) return;

    let subs: SaveSubscription[];
    try {
      subs = await listSaveSubscriptions();
    } catch (err) {
      console.warn("[archiveSyncManager] list_save_subscriptions failed:", err);
      return;
    }

    if (this.destroyed) return;

    // Keys we want after reload.
    const wanted = new Set(subs.map((s) => subKey(s.scopeType, s.scopeValue)));

    // Tear down subscriptions that are no longer needed.
    for (const [key, unsub] of this.unsubs) {
      if (!wanted.has(key)) {
        void unsub();
        this.unsubs.delete(key);
      }
    }

    // Open new subscriptions for any that aren't already running.
    for (const sub of subs) {
      const key = subKey(sub.scopeType, sub.scopeValue);
      if (this.unsubs.has(key)) continue;

      const scopeType = sub.scopeType;
      const scopeValue = sub.scopeValue;
      const filter = buildFilter(sub);

      let unsub: (() => Promise<void>) | null = null;
      let cancelled = false;

      void relayClient
        .subscribeLive(filter, (event: RelayEvent) => {
          this.enqueue(event, scopeType, scopeValue);
        })
        .then((dispose) => {
          if (cancelled) {
            void dispose();
          } else {
            unsub = dispose;
          }
        })
        .catch((err: unknown) => {
          console.warn(
            `[archiveSyncManager] subscribeLive failed for ${key}:`,
            err,
          );
        });

      // Store a teardown handle that works whether the promise resolved yet.
      this.unsubs.set(key, async () => {
        cancelled = true;
        if (unsub) await unsub();
      });
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
    if (this.buffer.length >= FLUSH_BATCH_SIZE) {
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
    }, FLUSH_IDLE_MS);
  }

  private flush(): void {
    if (this.flushTimer !== null) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    if (this.buffer.length === 0) return;
    const batch = this.buffer.splice(0);
    void archiveEvents(batch).catch((err: unknown) => {
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
