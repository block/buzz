import * as React from "react";

import { meshFeatureEnabled } from "@/shared/api/tauriMesh";

/**
 * Resolves whether Share Compute was compiled into this desktop binary.
 * `null` while the probe is in flight.
 */
export function useMeshFeatureEnabled(): boolean | null {
  const [enabled, setEnabled] = React.useState<boolean | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const value = await meshFeatureEnabled();
        if (!cancelled) setEnabled(value);
      } catch {
        // Older sidecars without the probe still surface stub errors on status.
        if (!cancelled) setEnabled(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return enabled;
}
