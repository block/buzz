import * as React from "react";

import { computeAutoScanDelay } from "@/features/triage/lib/autoScan";

/**
 * Rescans when new relevant messages arrive, debounced so a burst of activity
 * costs one scan rather than one per message.
 *
 * The trigger rides signals the shell already maintains (the home feed and live
 * thread activity), so it adds no relay traffic of its own — only the scan's
 * own catch-up queries, which the debounce keeps infrequent.
 */
export function useTriageAutoScan({
  enabled,
  isScanning,
  onScan,
  pendingCount,
}: {
  enabled: boolean;
  isScanning: boolean;
  onScan: () => void;
  /** Arrivals the current triage results do not cover yet. */
  pendingCount: number;
}) {
  const firstPendingAtRef = React.useRef<number | null>(null);
  const runScan = React.useEffectEvent(() => {
    onScan();
  });

  React.useEffect(() => {
    if (!enabled || pendingCount === 0) {
      firstPendingAtRef.current = null;
      return;
    }
    // Wait for the in-flight scan; its result re-runs this effect.
    if (isScanning) {
      return;
    }

    const now = Date.now();
    if (firstPendingAtRef.current === null) {
      firstPendingAtRef.current = now;
    }

    const delay = computeAutoScanDelay({
      pendingCount,
      waitedMs: now - firstPendingAtRef.current,
    });
    if (delay === null) {
      return;
    }

    // Each new arrival changes pendingCount, which re-arms this timer; the
    // shrinking budget in computeAutoScanDelay stops a busy channel from
    // deferring the scan forever.
    const timer = window.setTimeout(() => {
      firstPendingAtRef.current = null;
      runScan();
    }, delay);

    return () => {
      window.clearTimeout(timer);
    };
  }, [enabled, isScanning, pendingCount]);
}
