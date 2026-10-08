import * as React from "react";
import type { VListHandle } from "virtua";
import { nextRetainedTimelineKeys } from "./timelineRetention";

const INITIAL_RETAINED_TAIL_SIZE = 100;

export function useTimelineRetention(
  keys: readonly string[],
  listRef: React.RefObject<VListHandle | null>,
  isPrepend: boolean,
) {
  // Retain only a bounded visual tail on the first render. The timeline opens
  // at newest, so this gives Virtua stable rows for initial bottom positioning
  // without turning `keepMounted` into an all-history mount.
  const [retainedKeys, setRetainedKeys] = React.useState<ReadonlySet<string>>(
    () => new Set(keys.slice(-INITIAL_RETAINED_TAIL_SIZE)),
  );
  const evictionNotBeforeRef = React.useRef(0);
  const refreshTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const initialRefreshFrameRef = React.useRef<number | null>(null);
  const keysRef = React.useRef(keys);
  keysRef.current = keys;

  const refreshRetainedKeys = React.useCallback(() => {
    const remainingGuardMs = evictionNotBeforeRef.current - performance.now();
    if (remainingGuardMs > 0) {
      refreshTimerRef.current = setTimeout(
        refreshRetainedKeys,
        remainingGuardMs,
      );
      return;
    }

    refreshTimerRef.current = null;
    const currentKeys = keysRef.current;
    const list = listRef.current;
    if (!list || currentKeys.length === 0) return;
    setRetainedKeys((previous) =>
      nextRetainedTimelineKeys(currentKeys, previous, list),
    );
  }, [listRef]);

  React.useLayoutEffect(() => {
    if (isPrepend) evictionNotBeforeRef.current = performance.now() + 3_000;
  }, [isPrepend]);

  React.useEffect(() => {
    // `onScrollEnd` is not guaranteed for Virtua's initial programmatic
    // positioning. Wait until the first painted frame so the initial render
    // still gives Virtua only the bounded tail, then seed from its measured
    // viewport instead of retaining all history.
    initialRefreshFrameRef.current = requestAnimationFrame(() => {
      initialRefreshFrameRef.current = null;
      refreshRetainedKeys();
    });
    return () => {
      if (initialRefreshFrameRef.current !== null) {
        cancelAnimationFrame(initialRefreshFrameRef.current);
      }
      if (refreshTimerRef.current !== null) {
        clearTimeout(refreshTimerRef.current);
      }
    };
  }, [refreshRetainedKeys]);

  const retainedIndices = React.useMemo(
    () => keys.flatMap((key, index) => (retainedKeys.has(key) ? [index] : [])),
    [keys, retainedKeys],
  );
  const onScrollEnd = React.useCallback(() => {
    if (refreshTimerRef.current !== null) {
      clearTimeout(refreshTimerRef.current);
      refreshTimerRef.current = null;
    }
    refreshRetainedKeys();
  }, [refreshRetainedKeys]);

  return { retainedIndices, onScrollEnd };
}
