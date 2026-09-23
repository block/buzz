import * as React from "react";

import { meshSnapshot } from "@/shared/api/tauriMesh";
import type { MeshSnapshot } from "@/shared/api/tauriMesh";

/**
 * Poll the community shared-compute snapshot.
 *
 * Deliberately slow compared to `useMeshNodeStatus` (750ms/4000ms): that hook
 * watches a LOCAL lifecycle that must feel responsive, while this one reads
 * RELAY state that only changes as fast as the 45s member heartbeat, with a
 * 120s freshness window. Polling faster would just add relay queries without
 * adding information.
 *
 * An empty mesh is not an error — the snapshot carries its own `reason`. `error`
 * here means the query itself failed (offline relay, stub build).
 */
const SNAPSHOT_POLL_INTERVAL_MS = 30_000;

export function useMeshSnapshot(options?: { refreshKey?: number }): {
  snapshot: MeshSnapshot | null;
  error: string | null;
  refresh: () => void;
} {
  const [snapshot, setSnapshot] = React.useState<MeshSnapshot | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const refreshKey = options?.refreshKey ?? 0;

  // Guards against a slow in-flight response overwriting a newer one.
  const requestSeq = React.useRef(0);

  const fetchOnce = React.useCallback(() => {
    const seq = ++requestSeq.current;
    (async () => {
      try {
        const value = await meshSnapshot();
        if (seq === requestSeq.current) {
          setSnapshot(value);
          setError(null);
        }
      } catch (err) {
        if (seq === requestSeq.current) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
  }, []);

  // Re-fetch immediately when the local node transitions (refreshKey), so
  // turning sharing on updates the community view without waiting for a poll.
  // biome-ignore lint/correctness/useExhaustiveDependencies: refreshKey is an intentional trigger, not a value read in the body — bumping it re-queries the community snapshot right after a local node transition instead of waiting out the 30s poll
  React.useEffect(() => {
    fetchOnce();
  }, [fetchOnce, refreshKey]);

  React.useEffect(() => {
    const handle = window.setInterval(fetchOnce, SNAPSHOT_POLL_INTERVAL_MS);
    return () => window.clearInterval(handle);
  }, [fetchOnce]);

  return { snapshot, error, refresh: fetchOnce };
}
