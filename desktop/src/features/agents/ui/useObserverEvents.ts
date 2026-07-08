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
import {
  createArchivePagingState,
  applyChannelReset,
} from "./archivePagingState";
export type { ArchivePagingState } from "./archivePagingState";

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

  // All mutable paging state lives in one stable ref. createArchivePagingState
  // initialises the backfill promise eagerly so fetchOlderArchived can await it
  // before the backfill effect fires. applyChannelReset resets cursor/exhaustion/
  // fetchLock when channelId changes; backfill state is untouched (identity-level).
  const pagingStateRef = React.useRef(createArchivePagingState());
  const ps = pagingStateRef.current;

  // React state mirrors the fields callers observe so re-renders fire on change.
  const [hasSubscription, setHasSubscription] = React.useState<boolean | null>(
    ps.hasSubscription,
  );
  const [hasOlderArchived, setHasOlderArchived] = React.useState(
    ps.hasOlderArchived,
  );

  // Reset per-channel paging state when channelId changes. Backfill state is
  // identity-level (not per-channel) and must NOT be reset here — the backfill
  // index covers all channels and only needs to run once per identity mount.
  // Only the cursor, exhaustion flag, and fetching lock are channel-scoped.
  // biome-ignore lint/correctness/useExhaustiveDependencies: channelId is the intentional reset key; ps is a stable ref excluded from deps by convention; setHasOlderArchived is a stable React state setter
  React.useEffect(() => {
    applyChannelReset(ps);
    setHasOlderArchived(true);
  }, [channelId]);

  // Check for an owner_p subscription once per identity.
  // biome-ignore lint/correctness/useExhaustiveDependencies: ps is a stable ref excluded from deps by convention; setHasSubscription/setHasOlderArchived are stable React state setters
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
        ps.hasSubscription = hasSub;
        if (!hasSub) {
          setHasOlderArchived(false);
          ps.hasOlderArchived = false;
          // No subscription → backfill will never run; resolve the promise
          // immediately so fetchOlderArchived doesn't await indefinitely.
          ps.backfillStatus = "done";
          ps.backfillResolve?.();
        }
      })
      .catch(() => {
        if (!cancelled) {
          setHasSubscription(false);
          ps.hasSubscription = false;
          setHasOlderArchived(false);
          ps.hasOlderArchived = false;
          ps.backfillStatus = "done";
          ps.backfillResolve?.();
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
  // ps.backfillStatus so fetchOlderArchived can await completion.
  // biome-ignore lint/correctness/useExhaustiveDependencies: ps is a stable ref excluded from deps by convention
  React.useEffect(() => {
    if (!enabled || !hasSubscription || ps.backfillStatus !== "pending") {
      return;
    }
    ps.backfillStatus = "running";
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
        ps.backfillStatus = "done";
        ps.backfillResolve?.();
      }
    })();
    ps.backfillPromise = promise;
  }, [enabled, hasSubscription]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: ps is a stable ref; ps.isFetching/ps.cursor/ps.backfillPromise/ps.hasOlderArchived are read via the stable ref object, not reactive values
  const fetchOlderArchived = React.useCallback(async () => {
    if (
      !enabled ||
      !identityPubkey ||
      !hasSubscription ||
      !channelId ||
      ps.isFetching ||
      !hasOlderArchived
    ) {
      return;
    }

    // Await backfill completion before reading the channel index. This
    // guarantees the index is populated before the first paginated read, so
    // a scroll-trigger that fires before backfill writes can't return 0 rows
    // and falsely mark the channel exhausted.
    if (ps.backfillPromise) {
      await ps.backfillPromise;
    }

    // Re-check after awaiting: hasOlderArchived might have been set false
    // while we were waiting (e.g. subscription check failed).
    if (!hasOlderArchived) {
      return;
    }

    ps.isFetching = true;
    try {
      const before = ps.cursor ?? undefined;
      const events = await readArchivedObserverEventsForChannel(channelId, {
        before: before ?? null,
        limit: ARCHIVED_EVENTS_PAGE_SIZE,
      });

      if (events.length > 0) {
        // Cursor = the last row in newest-first order = the oldest event on
        // this page.  Capture both created_at and id to mirror the compound
        // sort key so same-second siblings are not skipped on the next page.
        const oldestEvent = events[events.length - 1];
        ps.cursor = {
          createdAt: oldestEvent.created_at,
          id: oldestEvent.id,
        };
        await ingestArchivedObserverEvents(events);
      }

      // A short page means the archive is exhausted for this channel.
      if (events.length < ARCHIVED_EVENTS_PAGE_SIZE) {
        setHasOlderArchived(false);
        ps.hasOlderArchived = false;
      }
    } catch (error) {
      console.error("[useLoadArchivedObserverEvents] fetch failed:", error);
    } finally {
      ps.isFetching = false;
    }
  }, [enabled, identityPubkey, hasSubscription, channelId, hasOlderArchived]);

  return { fetchOlderArchived, hasOlderArchived };
}
