import * as React from "react";

import { meshNodeStatus } from "@/shared/api/tauriMesh";
import type { MeshNodeStatus } from "@/shared/api/tauriMesh";

/**
 * Polls `mesh_node_status` faster than availability — lifecycle transitions
 * (off → starting → running, or running → failed) need to render quickly so
 * the Share-compute card doesn't show a frozen "Starting…" for minutes.
 *
 * The poll interval steps up while transitioning and steps down when steady.
 * That avoids hammering the runtime once a node is just "running ok."
 *
 * Returns `null` until first successful fetch.
 */
export function useMeshNodeStatus(): {
  status: MeshNodeStatus | null;
  error: string | null;
  refresh: () => void;
} {
  const [status, setStatus] = React.useState<MeshNodeStatus | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const requestSeq = React.useRef(0);

  const fetchOnce = React.useCallback(() => {
    const seq = ++requestSeq.current;
    (async () => {
      try {
        const value = await meshNodeStatus();
        if (seq === requestSeq.current) {
          setStatus(value);
          setError(null);
        }
      } catch (err) {
        if (seq === requestSeq.current) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
  }, []);

  React.useEffect(() => {
    fetchOnce();
  }, [fetchOnce]);

  // Fast poll while in a transitioning state; slow poll while steady or off.
  React.useEffect(() => {
    const transitioning =
      status?.state === "starting" || status?.state === "stopping";
    const interval = transitioning ? 750 : 4000;
    const handle = window.setInterval(() => {
      fetchOnce();
    }, interval);
    return () => window.clearInterval(handle);
  }, [status?.state, fetchOnce]);

  return { status, error, refresh: fetchOnce };
}
