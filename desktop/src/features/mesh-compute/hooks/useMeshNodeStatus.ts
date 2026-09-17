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
  update: (status: MeshNodeStatus) => void;
} {
  const [status, setStatus] = React.useState<MeshNodeStatus | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const requestSequence = React.useRef(0);
  const fetchOnce = React.useCallback(() => {
    const sequence = ++requestSequence.current;
    void (async () => {
      try {
        const value = await meshNodeStatus();
        if (sequence === requestSequence.current) {
          setStatus(value);
          setError(null);
        }
      } catch (err) {
        if (sequence === requestSequence.current) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
  }, []);

  const update = React.useCallback((value: MeshNodeStatus) => {
    // Invalidate any slower request started before this authoritative command
    // result so it cannot restore the preceding lifecycle state.
    requestSequence.current += 1;
    setStatus(value);
    setError(null);
  }, []);

  React.useEffect(() => {
    fetchOnce();
  }, [fetchOnce]);

  // Fast poll while in a transitioning state; slow poll while steady or off.
  React.useEffect(() => {
    const transitioning =
      status?.state === "starting" || status?.state === "stopping";
    // Keep reconciling aggressively while a start command is still waiting on
    // model readiness. The command can take minutes and its eventual response
    // is not the only signal that the runtime crossed to running.
    const interval = transitioning ? 500 : 4000;
    const handle = window.setInterval(() => {
      fetchOnce();
    }, interval);
    return () => window.clearInterval(handle);
  }, [status?.state, fetchOnce]);

  return { status, error, refresh: fetchOnce, update };
}
