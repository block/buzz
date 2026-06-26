import * as React from "react";

import { meshAvailability } from "@/shared/api/tauriMesh";
import type { MeshAvailability } from "@/shared/api/tauriMesh";

/**
 * Polls `mesh_availability` at a slow cadence — this drives the Settings
 * client-mode tile AND the Create-Agent "Relay mesh" flow gate. Both consumers
 * tolerate up-to-5-second staleness; we re-fetch on focus to keep transitions
 * (member added/removed, serve node started/stopped) feeling live.
 *
 * Returns `null` until the first successful fetch; consumers render a neutral
 * loading state during that window rather than guessing at false defaults.
 */
export function useMeshAvailability(): {
  availability: MeshAvailability | null;
  error: string | null;
  refresh: () => void;
} {
  const [availability, setAvailability] =
    React.useState<MeshAvailability | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  // Single fetch routine; the latest one wins via the `cancelled` flag captured
  // at call time. Returning the cleanup lets useEffect / setInterval invalidate
  // in-flight fetches when they unmount or super-cede each other.
  const fetchOnce = React.useCallback(() => {
    let cancelled = false;
    (async () => {
      try {
        const value = await meshAvailability();
        if (!cancelled) {
          setAvailability(value);
          setError(null);
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  React.useEffect(() => fetchOnce(), [fetchOnce]);

  // Slow polling — availability changes when members are admitted/removed or
  // serve nodes come up/down, which is human-scale, not sub-second.
  React.useEffect(() => {
    const handle = window.setInterval(() => {
      fetchOnce();
    }, 5000);
    return () => window.clearInterval(handle);
  }, [fetchOnce]);

  // Refresh when window regains focus — common case is user toggled
  // share-compute on a peer machine and tabbed back.
  React.useEffect(() => {
    const onFocus = () => {
      fetchOnce();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [fetchOnce]);

  return { availability, error, refresh: fetchOnce };
}
