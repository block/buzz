import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { shouldTriggerResumeReconnect } from "@/shared/api/relayResumeTriggerPolicy";

/**
 * Event-driven reconnect triggers: network `online`, window `focus`, and
 * visibility→visible each attempt an immediate `preconnect()` when the relay
 * session is degraded (reconnecting/stalled), rate-limited by
 * `RESUME_TRIGGER_MIN_INTERVAL_MS`.
 *
 * Rationale (CMD+R gap audit G1): without these, recovery rides solely on
 * the backoff timer, which WKWebView throttles while the window is occluded
 * or the system was asleep. The moment a user focuses the window to hit
 * CMD+R *is* a focus event — this fires the reconnect first. Deliberately
 * does not trigger on `disconnected` (terminal latch): that path requires
 * explicit user re-engagement via the reconnect card.
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
      // preconnect() clears any pending backoff timer and connects now.
      // Failures re-arm the normal backoff loop; nothing to handle here.
      void relayClient.preconnect().catch(() => {});
    };

    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") attempt();
    };

    window.addEventListener("online", attempt);
    window.addEventListener("focus", attempt);
    document.addEventListener("visibilitychange", onVisibilityChange);

    return () => {
      window.removeEventListener("online", attempt);
      window.removeEventListener("focus", attempt);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, []);
}
