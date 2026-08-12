import * as React from "react";

import { millisecondsUntilVerifiedIdentityExpiry } from "@/shared/lib/verifiedIdentity";

const MAX_TIMEOUT_MS = 2_147_483_647;

/**
 * Force a render at the earliest assertion cutoff. Callers then re-run the
 * local-clock sanitizer, even when React Query is serving an offline cache.
 */
export function useVerifiedIdentityExpiryRevision(
  expirations: ReadonlyArray<number | null | undefined>,
): number {
  const nowMs = Date.now();
  let nextDelayMs: number | null = null;
  for (const expiresAt of expirations) {
    const delayMs = millisecondsUntilVerifiedIdentityExpiry(expiresAt, nowMs);
    if (
      delayMs !== null &&
      delayMs > 0 &&
      (nextDelayMs === null || delayMs < nextDelayMs)
    ) {
      nextDelayMs = delayMs;
    }
  }

  const [revision, setRevision] = React.useState(0);
  React.useEffect(() => {
    // Re-arm after a timer tick even if a wall-clock adjustment happens to
    // produce the same remaining delay as the previous render.
    void revision;
    if (nextDelayMs === null) {
      return;
    }

    const timeout = setTimeout(
      () => setRevision((current) => current + 1),
      Math.min(nextDelayMs + 1, MAX_TIMEOUT_MS),
    );
    return () => clearTimeout(timeout);
  }, [nextDelayMs, revision]);

  return revision;
}
