import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { shouldTriggerResumeReconnect } from "@/shared/api/relayResumeTriggerPolicy";
import { scheduleAfterForegroundReady } from "@/shared/lib/foregroundReady";

type ScheduleForegroundWork = (callback: () => void) => () => void;

export function installRelayResumeTriggers(
  attempt: () => void,
  schedule: ScheduleForegroundWork = scheduleAfterForegroundReady,
): () => void {
  let cancelPendingResume: (() => void) | null = null;
  const scheduleAttempt = () => {
    if (cancelPendingResume) return;
    cancelPendingResume = schedule(() => {
      cancelPendingResume = null;
      attempt();
    });
  };
  const onVisibilityChange = () => {
    if (document.visibilityState === "visible") scheduleAttempt();
    else {
      cancelPendingResume?.();
      cancelPendingResume = null;
    }
  };

  window.addEventListener("online", scheduleAttempt);
  window.addEventListener("focus", scheduleAttempt);
  document.addEventListener("visibilitychange", onVisibilityChange);

  return () => {
    cancelPendingResume?.();
    window.removeEventListener("online", scheduleAttempt);
    window.removeEventListener("focus", scheduleAttempt);
    document.removeEventListener("visibilitychange", onVisibilityChange);
  };
}

/**
 * Event-driven reconnect triggers: network `online`, window `focus`, and
 * visibility→visible coalesce a foreground-ready `resumeReconnect()` attempt
 * when the relay session is degraded (reconnecting/stalled), rate-limited by
 * `RESUME_TRIGGER_MIN_INTERVAL_MS`.
 *
 * Rationale (CMD+R gap audit G1): without these, recovery rides solely on
 * the backoff timer, which WKWebView throttles while the window is occluded
 * or the system was asleep. The moment a user focuses the window to hit
 * CMD+R *is* a focus event — this fires the reconnect first.
 *
 * Uses `resumeReconnect()`, NOT `preconnect()`: resume events bypass the
 * pending backoff timer but must preserve the terminal latch and the AUTH
 * rejection streak. Otherwise a focus/online event arriving during repeated
 * AUTH rejection would reset the consecutive-rejection cap and the session
 * could retry indefinitely instead of surfacing `disconnected`. The
 * terminal state stays user-owned via the reconnect card.
 */
export function useRelayResumeTriggers(): void {
  React.useEffect(() => {
    let lastAttemptAt = -Infinity;

    const attempt = () => {
      const now = Date.now();
      if (
        !shouldTriggerResumeReconnect({
          connectionState: relayClient.getConnectionState(),
          lastAttemptAt,
          now,
        })
      ) {
        return;
      }
      lastAttemptAt = now;
      // resumeReconnect() clears any pending backoff timer and connects now,
      // preserving terminal/AUTH-streak state. Failures re-arm the normal
      // backoff loop; nothing to handle here.
      void relayClient.resumeReconnect().catch(() => {});
    };

    return installRelayResumeTriggers(attempt);
  }, []);
}
