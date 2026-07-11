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

  React.useLayoutEffect(() => {
    if (isPrepend) evictionNotBeforeRef.current = performance.now() + 3_000;
  }, [isPrepend]);

  const retainedIndices = React.useMemo(
    () => keys.flatMap((key, index) => (retainedKeys.has(key) ? [index] : [])),
    [keys, retainedKeys],
  );
  const onScrollEnd = React.useCallback(() => {
    const list = listRef.current;
    if (
      !list ||
      keys.length === 0 ||
      performance.now() < evictionNotBeforeRef.current
    ) {
      return;
    }
    setRetainedKeys((previous) =>
      nextRetainedTimelineKeys(keys, previous, list),
    );
  }, [keys, listRef]);

  return { retainedIndices, onScrollEnd };
}
