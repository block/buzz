import * as React from "react";

import {
  ensureRelayObserverSubscription,
  getAgentObserverSnapshot,
  getAgentTranscript,
  ingestArchivedObserverEvents,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import {
  listSaveSubscriptions,
  readArchivedObserverEventsForChannel,
  readUnindexedObserverRows,
  indexObserverChannelId,
} from "@/shared/api/tauriArchive";
import { decryptObserverEvent } from "@/shared/api/tauriObserver";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { TranscriptItem } from "./agentSessionTypes";
import type { RelayEvent } from "@/shared/api/types";

// Stable subscribe reference shared by all useSyncExternalStore hooks.
// subscribeAgentObserverStore already has a fixed identity, so this thin
// wrapper satisfies React's requirement without per-hook useCallback.
const subscribeToStore = (onStoreChange: () => void) =>
  subscribeAgentObserverStore(onStoreChange);

export function useObserverEvents(
  enabled: boolean,
  agentPubkey?: string | null,
) {
  const getSnapshot = React.useCallback(
    () => getAgentObserverSnapshot(agentPubkey, enabled),
    [agentPubkey, enabled],
  );

  const snapshot = React.useSyncExternalStore(subscribeToStore, getSnapshot);

  React.useEffect(() => {
    if (enabled && agentPubkey) {
      void ensureRelayObserverSubscription();
    }
  }, [enabled, agentPubkey]);

  return snapshot;
}

export function useAgentTranscript(
  enabled: boolean,
  agentPubkey?: string | null,
): TranscriptItem[] {
  const getSnapshot = React.useCallback(
    () => getAgentTranscript(agentPubkey, enabled),
    [agentPubkey, enabled],
  );

  return React.useSyncExternalStore(subscribeToStore, getSnapshot);
}

const ARCHIVED_EVENTS_PAGE_SIZE = 50;

/**
 * Load-older-on-scroll for archived observer frames, scoped to a single channel.
 *
 * Reads from `observer_channel_index` (via `readArchivedObserverEventsForChannel`)
 * so only frames attributable to this channel are loaded — cross-channel
 * contamination is impossible. Frames with null/decrypt-failed channelId are
 * excluded at the Rust level (Will's (a) ruling).
 *
 * On first mount, runs a one-shot idempotent backfill: decrypts all
 * not-yet-indexed `owner_p` kind 24200 rows and writes their (id, channelId)
 * pairs into the index, so existing archived history is available immediately
 * without requiring the user to scroll through every page.
 *
 * Degrades cleanly when no `owner_p` subscription exists or when `channelId`
 * is null (returns `hasOlderArchived: false` without making any archive calls).
 */
export function useLoadArchivedObserverEvents(
  enabled: boolean,
  channelId: string | null,
) {
  const identityQuery = useIdentityQuery();
  const identityPubkey = identityQuery.data?.pubkey ?? null;

  // Whether the current identity has an owner_p save subscription.
  const [hasSubscription, setHasSubscription] = React.useState<boolean | null>(
    null,
  );
  const [hasOlderArchived, setHasOlderArchived] = React.useState(true);
  const isFetchingRef = React.useRef(false);
  // Backfill state: "pending" → "running" → "done".
  // fetchOlderArchived awaits backfillPromiseRef before reading the index so
  // the first scroll-trigger never races the write path and incorrectly marks
  // the channel exhausted before backfill has completed.
  const backfillStatusRef = React.useRef<"pending" | "running" | "done">(
    "pending",
  );
  const backfillPromiseRef = React.useRef<Promise<void> | null>(null);
  const backfillResolveRef = React.useRef<(() => void) | null>(null);
  // Expose a promise that resolves when backfill is done.  Created eagerly so
  // fetchOlderArchived can await it before the effect that starts backfill fires.
  if (!backfillPromiseRef.current) {
    backfillPromiseRef.current = new Promise<void>((resolve) => {
      backfillResolveRef.current = resolve;
    });
  }
  // Compound keyset cursor: tracks both `created_at` and `id` of the oldest
  // event seen so far.  Mirrors the SQL `ORDER BY created_at DESC, id DESC` so
  // same-second siblings are never skipped at a page boundary.
  const cursorRef = React.useRef<{ createdAt: number; id: string } | null>(
    null,
  );

  // Check for an owner_p subscription once per identity.
  React.useEffect(() => {
    if (!enabled || !identityPubkey) {
      return;
    }
    let cancelled = false;
    listSaveSubscriptions()
      .then((subs) => {
        if (cancelled) {
          return;
        }
        const hasSub = subs.some(
          (s) => s.scopeType === "owner_p" && s.scopeValue === identityPubkey,
        );
        setHasSubscription(hasSub);
        if (!hasSub) {
          setHasOlderArchived(false);
          // No subscription → backfill will never run; resolve the promise
          // immediately so fetchOlderArchived doesn't await indefinitely.
          backfillStatusRef.current = "done";
          backfillResolveRef.current?.();
        }
      })
      .catch(() => {
        if (!cancelled) {
          setHasSubscription(false);
          setHasOlderArchived(false);
          backfillStatusRef.current = "done";
          backfillResolveRef.current?.();
        }
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, identityPubkey]);

  // One-shot idempotent backfill: attempt to decrypt all not-yet-processed
  // owner_p kind 24200 rows and write their (id, channelId?) into
  // observer_channel_index. A status row is written for EVERY processed event —
  // null/failed channelId rows get channel_id=null, so re-runs skip them.
  // Runs once per mount when the subscription is confirmed; gated by
  // backfillStatusRef so fetchOlderArchived can await completion.
  React.useEffect(() => {
    if (
      !enabled ||
      !hasSubscription ||
      backfillStatusRef.current !== "pending"
    ) {
      return;
    }
    backfillStatusRef.current = "running";
    const promise = (async () => {
      try {
        const rows = await readUnindexedObserverRows();

        const toIndex: Array<{
          eventId: string;
          channelId: string | null;
          createdAt: number;
        }> = [];

        for (const row of rows) {
          let parsed: RelayEvent;
          try {
            parsed = JSON.parse(row.rawJson) as RelayEvent;
          } catch {
            // Malformed JSON: write a null status row so we skip on re-run.
            toIndex.push({
              eventId: row.id,
              channelId: null,
              createdAt: row.createdAt,
            });
            continue;
          }
          try {
            const decoded = (await decryptObserverEvent(parsed)) as {
              channelId?: string | null;
            };
            // Write a status row for every event — non-null channelId is
            // attributable; null/undefined channelId writes channel_id=null so
            // the frame is marked processed and excluded from scoped views.
            toIndex.push({
              eventId: row.id,
              channelId: decoded?.channelId ?? null,
              createdAt: row.createdAt,
            });
          } catch {
            // Decrypt failure → write null status row (processed, unscoped).
            toIndex.push({
              eventId: row.id,
              channelId: null,
              createdAt: row.createdAt,
            });
          }
        }

        if (toIndex.length > 0) {
          await indexObserverChannelId(toIndex);
        }
      } catch (error) {
        console.error(
          "[useLoadArchivedObserverEvents] backfill failed:",
          error,
        );
      } finally {
        backfillStatusRef.current = "done";
        backfillResolveRef.current?.();
      }
    })();
    backfillPromiseRef.current = promise;
  }, [enabled, hasSubscription]);

  const fetchOlderArchived = React.useCallback(async () => {
    if (
      !enabled ||
      !identityPubkey ||
      !hasSubscription ||
      !channelId ||
      isFetchingRef.current ||
      !hasOlderArchived
    ) {
      return;
    }

    // Await backfill completion before reading the channel index. This
    // guarantees the index is populated before the first paginated read, so
    // a scroll-trigger that fires before backfill writes can't return 0 rows
    // and falsely mark the channel exhausted.
    if (backfillPromiseRef.current) {
      await backfillPromiseRef.current;
    }

    // Re-check after awaiting: hasOlderArchived might have been set false
    // while we were waiting (e.g. subscription check failed).
    if (!hasOlderArchived) {
      return;
    }

    isFetchingRef.current = true;
    try {
      const before = cursorRef.current ?? undefined;
      const events = await readArchivedObserverEventsForChannel(channelId, {
        before: before ?? null,
        limit: ARCHIVED_EVENTS_PAGE_SIZE,
      });

      if (events.length > 0) {
        // Cursor = the last row in newest-first order = the oldest event on
        // this page.  Capture both created_at and id to mirror the compound
        // sort key so same-second siblings are not skipped on the next page.
        const oldestEvent = events[events.length - 1];
        cursorRef.current = {
          createdAt: oldestEvent.created_at,
          id: oldestEvent.id,
        };
        await ingestArchivedObserverEvents(events);
      }

      // A short page means the archive is exhausted for this channel.
      if (events.length < ARCHIVED_EVENTS_PAGE_SIZE) {
        setHasOlderArchived(false);
      }
    } catch (error) {
      console.error("[useLoadArchivedObserverEvents] fetch failed:", error);
    } finally {
      isFetchingRef.current = false;
    }
  }, [enabled, identityPubkey, hasSubscription, channelId, hasOlderArchived]);

  return { fetchOlderArchived, hasOlderArchived };
}
