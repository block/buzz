import * as React from "react";
import {
  activityScopeKey,
  flushThreadActivityWrite,
  readActivityFromStorage,
  removeLegacyThreadActivityKey,
  scheduleThreadActivityWrite,
  type ThreadActivityItem,
  type ThreadActivityRefs,
} from "@/features/channels/threadActivityStorage";

export type ThreadActivityPersistence = {
  /** Scope key loaded into the buffer ("" until identity is known). */
  scopeLoadedRef: React.MutableRefObject<string>;
  /** Current scope derived from normalized pubkey + relay. */
  currentScope: string;
  /**
   * True only when the hydration effect has committed for the current scope
   * AND that scope is non-empty. Reads the ref at call time so it is never a
   * stale snapshot. Use as the write/merge guard: an empty scope must never
   * pass, or a writer could fire before the first valid scope is seeded.
   */
  isScopeLoaded: () => boolean;
  /** Arm a coalesced write after mutating the buffer for `scope`. */
  schedule: (scope: string) => void;
};

/**
 * Manages the thread-activity localStorage persistence layer for
 * useUnreadChannels: owns the loaded-scope ref, the coalescing timer, the
 * pagehide/visibility flush, and hydration on identity/relay change. The buffer
 * itself (`itemsRef`) is owned by the parent and merged in place by its writers;
 * this hook only decides when the buffer is durably persisted.
 *
 * Sibling of useObservedUnreadPersistence — same scope-fence and flush shape,
 * minus marker-prune/removeChannel/clearAll, which thread activity has no
 * analog for.
 */
export function useThreadActivityPersistence(
  normalizedPubkey: string | null,
  normalizedRelayUrl: string,
  itemsRef: React.MutableRefObject<ThreadActivityItem[]>,
): ThreadActivityPersistence {
  const currentScope = activityScopeKey(normalizedPubkey, normalizedRelayUrl);

  const scopeLoadedRef = React.useRef<string>("");
  const timerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  const persistRefs = React.useRef<ThreadActivityRefs>({
    itemsRef,
    scopeLoadedRef,
    timerRef,
  });
  persistRefs.current.itemsRef = itemsRef;

  // pagehide + visibilitychange→hidden: synchronously persist any pending write
  // before the webview unloads or is backgrounded. Cmd+R and #5588's idle
  // reload both tear the webview down within the coalescing window; without
  // these flushes the last burst of replies would be lost.
  React.useEffect(() => {
    const refs = persistRefs.current;
    const flush = () => flushThreadActivityWrite(refs);
    const onVisibility = () => {
      if (document.visibilityState === "hidden") flush();
    };
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      window.removeEventListener("pagehide", flush);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);

  // Hydrate the buffer whenever identity/relay changes. Flush the OLD scope
  // first so an in-flight coalesced write lands under the old key before the
  // buffer is clobbered, then drop the orphaned legacy key for this pubkey.
  // biome-ignore lint/correctness/useExhaustiveDependencies: normalizedRelayUrl is an intentional reset signal alongside normalizedPubkey
  React.useEffect(() => {
    flushThreadActivityWrite(persistRefs.current);

    if (normalizedPubkey && normalizedRelayUrl) {
      removeLegacyThreadActivityKey(normalizedPubkey);
      itemsRef.current = readActivityFromStorage(
        normalizedPubkey,
        normalizedRelayUrl,
      );
    } else {
      itemsRef.current = [];
    }
    scopeLoadedRef.current = currentScope;

    // Flush the current scope on unmount / before the next run so a pending
    // write is never dropped when refs are clobbered.
    return () => {
      flushThreadActivityWrite(persistRefs.current);
    };
  }, [normalizedPubkey, normalizedRelayUrl]);

  const schedule = React.useCallback(
    (scope: string) => scheduleThreadActivityWrite(scope, persistRefs.current),
    [],
  );

  const isScopeLoaded = React.useCallback(
    () => currentScope !== "" && scopeLoadedRef.current === currentScope,
    [currentScope],
  );

  return React.useMemo(
    () => ({ scopeLoadedRef, currentScope, isScopeLoaded, schedule }),
    [currentScope, isScopeLoaded, schedule],
  );
}
