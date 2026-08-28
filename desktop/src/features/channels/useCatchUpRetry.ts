import * as React from "react";

const RETRY_DELAYS_MS = [1_000, 5_000, 30_000] as const;

export function useCatchUpRetry(scope: string, isScopeLoaded: () => boolean) {
  const attemptsRef = React.useRef(new Map<string, number>());
  const timerRef = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const [retryVersion, bumpRetryVersion] = React.useReducer(
    (value: number) => value + 1,
    0,
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: scope is the intentional reset signal
  React.useEffect(() => {
    attemptsRef.current = new Map();
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, [scope]);

  const clearAttempt = React.useCallback((channelId: string) => {
    attemptsRef.current.delete(channelId);
  }, []);

  const schedule = React.useCallback(
    (channelIds: Iterable<string>) => {
      let nextDelay: number | null = null;
      for (const channelId of new Set(channelIds)) {
        const attempt = attemptsRef.current.get(channelId) ?? 0;
        const delay = RETRY_DELAYS_MS[attempt];
        if (delay === undefined) continue;
        attemptsRef.current.set(channelId, attempt + 1);
        nextDelay = nextDelay === null ? delay : Math.min(nextDelay, delay);
      }
      if (nextDelay === null || timerRef.current !== null) return;
      const retryScope = scope;
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        if (scope !== retryScope || !isScopeLoaded()) return;
        bumpRetryVersion();
      }, nextDelay);
    },
    [isScopeLoaded, scope],
  );

  return { retryVersion, clearAttempt, schedule };
}
