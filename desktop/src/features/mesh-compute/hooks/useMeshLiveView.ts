import * as React from "react";

import { type MeshLiveView, meshLiveView } from "@/shared/api/tauriMesh";

/**
 * Poll this machine's live gossip view of the mesh.
 *
 * Faster than the relay snapshot's 30s cadence because this is local IPC
 * reading an in-process status payload — no relay round trip — and peers join
 * and leave on a human timescale that a 30s poll makes look broken.
 *
 * Only polls while a runtime is expected to exist: with no node there is
 * nothing to report, and hammering a command that always returns
 * `connected:false` is waste.
 */
const POLL_MS = 5000;

export function useMeshLiveView(enabled: boolean): {
  view: MeshLiveView | null;
  refresh: () => void;
} {
  const [view, setView] = React.useState<MeshLiveView | null>(null);
  const [nonce, setNonce] = React.useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: nonce is an intentional re-run trigger for refresh(), never a value read in the body — bumping it re-polls the live view immediately after a local node transition
  React.useEffect(() => {
    if (!enabled) {
      // Drop the stale view rather than leaving the last peers on screen after
      // the node stops — they are no longer reachable.
      setView(null);
      return;
    }
    let cancelled = false;
    const tick = async () => {
      try {
        const next = await meshLiveView();
        if (!cancelled) {
          setView(next);
        }
      } catch {
        // A transient IPC failure is not evidence the mesh is empty; keep the
        // previous view rather than blanking the topology.
      }
    };
    void tick();
    const timer = setInterval(() => void tick(), POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [enabled, nonce]);

  return {
    view,
    refresh: React.useCallback(() => setNonce((value) => value + 1), []),
  };
}
