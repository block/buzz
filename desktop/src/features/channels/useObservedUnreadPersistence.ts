import * as React from "react";
import {
  flushObservedUnreadWrite,
  pruneObservedUnreadByMarkers,
  readObservedUnreadFromStorage,
  scheduleObservedUnreadWrite,
  deriveLatestByChannel,
  clearObservedUnreadStorage,
  type ObservedUnreadRefs,
} from "@/features/channels/observedUnreadStorage";
import { activityScopeKey } from "@/features/channels/threadActivityStorage";
import type { ObservedUnreadEvent } from "@/features/channels/unreadChannelCounts";

export type ObservedUnreadPersistence = {
  /** Scope key loaded into the refs ("" until identity is known). */
  scopeLoadedRef: React.MutableRefObject<string>;
  /** Current scope derived from normalized pubkey + relay. */
  currentScope: string;
  /**
   * Returns true when the identity-reset effect has committed and the observed
   * refs hold data for the current scope. Always reads the ref at call time.
   * Use as a scope guard before projecting or mutating the observed refs.
   */
  isScopeLoaded: () => boolean;
  /**
   * Call after recording a new observed event to schedule a debounced write.
   * @param scope - the currentScope value captured this render
   */
  schedule: (scope: string) => void;
  /** Remove a single channel from the persisted cache (clearObserved path). */
  removeChannel: (channelId: string) => void;
  /** Clear the entire persisted cache (mark-all-read path). */
  clearAll: () => void;
};

/**
 * Additional options for useObservedUnreadPersistence.
 */
export type UseObservedUnreadPersistenceOptions = {
  /**
   * Called when a marker-prune pass removes at least one event from the
   * in-memory refs. The hook itself cannot bump the parent's version counter;
   * pass a stable callback (e.g. useEvent-wrapped bumpLatestVersion) here so
   * the UI re-renders when stale observed events are swept out.
   */
  onPruned?: () => void;
};

/**
 * Hook that manages the observed-unread localStorage persistence layer for
 * useUnreadChannels. Owns the scope ref, debounce timer, pagehide flush, and
 * marker-prune effect.
 *
 * Returns a stable API object; callers read `scopeLoadedRef` and `currentScope`
 * to guard writes, and call `schedule`/`removeChannel`/`clearAll` on mutations.
 */
export function useObservedUnreadPersistence(
  normalizedPubkey: string | null,
  normalizedRelayUrl: string,
  isReadStateReady: boolean,
  readStateVersion: number,
  getEffectiveTimestamp: (channelId: string) => number | null,
  getOwnTimestamp: (contextId: string) => number | null,
  observedUnreadEventsByChannelRef: React.MutableRefObject<
    Map<string, Map<string, ObservedUnreadEvent>>
  >,
  latestByChannelRef: React.MutableRefObject<Map<string, number>>,
  options: UseObservedUnreadPersistenceOptions = {},
): ObservedUnreadPersistence {
  const currentScope = activityScopeKey(normalizedPubkey, normalizedRelayUrl);

  const { onPruned } = options;

  const scopeLoadedRef = React.useRef<string>("");
  const timerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  const persistRefs = React.useRef<ObservedUnreadRefs>({
    eventsRef: observedUnreadEventsByChannelRef,
    scopeLoadedRef,
    timerRef,
  });
  persistRefs.current.eventsRef = observedUnreadEventsByChannelRef;

  // pagehide: synchronously flush any pending debounce before the webview
  // unloads. Cmd+R reloads within 500ms of teardown; without this flush the
  // last observed event would be lost within the debounce window.
  React.useEffect(() => {
    const refs = persistRefs.current;
    const onPageHide = () => flushObservedUnreadWrite(refs);
    window.addEventListener("pagehide", onPageHide);
    return () => window.removeEventListener("pagehide", onPageHide);
  }, []);

  // Hydrate refs from storage whenever identity/relay changes. Flush the OLD
  // scope first so no event is lost before we clobber the refs.
  // biome-ignore lint/correctness/useExhaustiveDependencies: normalizedRelayUrl is intentional reset signal alongside normalizedPubkey
  React.useEffect(() => {
    flushObservedUnreadWrite(persistRefs.current);

    observedUnreadEventsByChannelRef.current = new Map();
    latestByChannelRef.current = new Map();

    if (normalizedPubkey && normalizedRelayUrl) {
      const stored = readObservedUnreadFromStorage(
        normalizedPubkey,
        normalizedRelayUrl,
      );
      if (stored && stored.size > 0) {
        observedUnreadEventsByChannelRef.current = stored;
        latestByChannelRef.current = deriveLatestByChannel(stored);
      }
    }
    scopeLoadedRef.current = activityScopeKey(
      normalizedPubkey,
      normalizedRelayUrl,
    );

    // On unmount (or before next effect run), flush the current scope so any
    // in-flight debounce is persisted before refs are clobbered.
    return () => {
      flushObservedUnreadWrite(persistRefs.current);
    };
  }, [normalizedPubkey, normalizedRelayUrl]);

  // Marker prune: whenever read state advances, drop events now covered by
  // their channel/thread/msg marker, persist if anything changed.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion + isReadStateReady are intentional prune triggers
  React.useEffect(() => {
    if (!isReadStateReady) return;
    const scope = scopeLoadedRef.current;
    if (!scope || scope !== currentScope) return;

    const changed = pruneObservedUnreadByMarkers(
      observedUnreadEventsByChannelRef.current,
      latestByChannelRef.current,
      getEffectiveTimestamp,
      getOwnTimestamp,
    );
    if (changed) {
      scheduleObservedUnreadWrite(scope, persistRefs.current);
      onPruned?.();
    }
  }, [readStateVersion, isReadStateReady]);

  const schedule = React.useCallback(
    (scope: string) => scheduleObservedUnreadWrite(scope, persistRefs.current),
    [],
  );

  const removeChannel = React.useCallback(
    (channelId: string) => {
      // Reject if the loaded scope has drifted — a stale callback during A→B
      // transitions must not cancel B's pending snapshot or corrupt B's refs.
      if (scopeLoadedRef.current !== currentScope) return;
      // Delete from both in-memory refs so the projection no longer sees this
      // channel. Then replace any pending snapshot with a new snapshot of the
      // current full map — never cancel-without-replacement, which would lose
      // unsaved sibling-channel events on the next reload.
      observedUnreadEventsByChannelRef.current.delete(channelId);
      latestByChannelRef.current.delete(channelId);
      scheduleObservedUnreadWrite(currentScope, persistRefs.current);
    },
    [currentScope, observedUnreadEventsByChannelRef, latestByChannelRef],
  );

  const clearAll = React.useCallback(() => {
    // Reject if the loaded scope has drifted — a stale callback must not
    // cancel the new scope's pending snapshot or clear the wrong bucket.
    if (scopeLoadedRef.current !== currentScope) return;
    // Cancel any pending snapshot and clear both in-memory refs before touching
    // storage — the parent no longer resets the refs directly, so this is the
    // single transactional clear path for mark-all-read.
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    observedUnreadEventsByChannelRef.current = new Map();
    latestByChannelRef.current = new Map();
    clearObservedUnreadStorage(normalizedPubkey ?? "", normalizedRelayUrl);
  }, [
    currentScope,
    normalizedPubkey,
    normalizedRelayUrl,
    observedUnreadEventsByChannelRef,
    latestByChannelRef,
  ]);

  // isScopeLoaded reads the ref at call time — always fresh, never a stale
  // snapshot from a closed-over useMemo value.
  const isScopeLoaded = React.useCallback(
    () => scopeLoadedRef.current === currentScope,
    [currentScope],
  );

  // Stable API object: only reconstructed when scope or stable callbacks change.
  // This prevents useCallback deps in the parent from seeing a new object each
  // render, which would restart catch-up REQs on every unrelated re-render.
  return React.useMemo(
    () => ({
      scopeLoadedRef,
      currentScope,
      isScopeLoaded,
      schedule,
      removeChannel,
      clearAll,
    }),
    [currentScope, isScopeLoaded, schedule, removeChannel, clearAll],
  );
}
