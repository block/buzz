import * as React from "react";

import { meshServingUsage } from "@/shared/api/tauriMesh";
import type { MeshServingUsage } from "@/shared/api/tauriMesh";

/**
 * Polls host-side serving usage while this machine is sharing compute.
 *
 * Only polls when `enabled` (the card passes `isSharing`) so a machine that
 * isn't serving does no work. Cadence is a plain 4s — usage is informational,
 * not a lifecycle transition, so it doesn't need the adaptive stepping that
 * `useMeshNodeStatus` uses. Returns `null` until the first successful fetch.
 */
export function useMeshServingUsage(enabled: boolean): MeshServingUsage | null {
  const [usage, setUsage] = React.useState<MeshServingUsage | null>(null);

  React.useEffect(() => {
    if (!enabled) {
      setUsage(null);
      return;
    }
    let cancelled = false;
    const fetchOnce = () => {
      (async () => {
        try {
          const value = await meshServingUsage();
          if (!cancelled) setUsage(value);
        } catch {
          // Usage is best-effort; a failed poll leaves the last value in place
          // rather than flapping the indicator.
        }
      })();
    };
    fetchOnce();
    const handle = window.setInterval(fetchOnce, 4000);
    return () => {
      cancelled = true;
      window.clearInterval(handle);
    };
  }, [enabled]);

  return usage;
}
