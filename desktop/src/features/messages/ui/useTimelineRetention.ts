import * as React from "react";
import type { VListHandle } from "virtua";
import { nextRetainedTimelineKeys } from "./timelineRetention";

export function useTimelineRetention(
  keys: readonly string[],
  listRef: React.RefObject<VListHandle | null>,
  isPrepend: boolean,
) {
  const [retainedKeys, setRetainedKeys] = React.useState<ReadonlySet<string>>(
    () => new Set(keys),
  );
  const evictionNotBeforeRef = React.useRef(0);
  const refreshTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
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

  React.useEffect(
    () => () => {
      if (refreshTimerRef.current !== null) {
        clearTimeout(refreshTimerRef.current);
      }
    },
    [],
  );

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
