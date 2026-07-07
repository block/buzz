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
  readArchivedEvents,
} from "@/shared/api/tauriArchive";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { TranscriptItem } from "./agentSessionTypes";

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
 * Load-older-on-scroll for archived observer frames.
 *
 * Checks whether an `owner_p` save subscription exists for the current
 * identity. If one does, exposes `fetchOlderArchived` and `hasOlderArchived`
 * for wiring into a sentinel-based scroll loader.
 *
 * Degrades cleanly when no subscription exists (returns `hasOlderArchived:
 * false` without making any archive calls).
 */
export function useLoadArchivedObserverEvents(enabled: boolean) {
  const identityQuery = useIdentityQuery();
  const identityPubkey = identityQuery.data?.pubkey ?? null;

  // Whether the current identity has an owner_p save subscription.
  const [hasSubscription, setHasSubscription] = React.useState<boolean | null>(
    null,
  );
  const [hasOlderArchived, setHasOlderArchived] = React.useState(true);
  const isFetchingRef = React.useRef(false);
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
        }
      })
      .catch(() => {
        if (!cancelled) {
          setHasSubscription(false);
          setHasOlderArchived(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, identityPubkey]);

  const fetchOlderArchived = React.useCallback(async () => {
    if (
      !enabled ||
      !identityPubkey ||
      !hasSubscription ||
      isFetchingRef.current ||
      !hasOlderArchived
    ) {
      return;
    }

    isFetchingRef.current = true;
    try {
      const before = cursorRef.current ?? undefined;
      const events = await readArchivedEvents("owner_p", identityPubkey, {
        kinds: [24200],
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

      // A short page means the archive is exhausted.
      if (events.length < ARCHIVED_EVENTS_PAGE_SIZE) {
        setHasOlderArchived(false);
      }
    } catch (error) {
      console.error("[useLoadArchivedObserverEvents] fetch failed:", error);
    } finally {
      isFetchingRef.current = false;
    }
  }, [enabled, identityPubkey, hasSubscription, hasOlderArchived]);

  return { fetchOlderArchived, hasOlderArchived };
}
