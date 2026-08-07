import * as React from "react";

import { meshExperimentalCatalog } from "@/shared/api/tauriMesh";
import type { MeshExperimentalCatalog } from "@/shared/api/tauriMesh";

/**
 * Load the split-capable model catalog.
 *
 * Fetched once rather than polled: the community catalog is a published dataset
 * that changes when someone packages a model, not live state. mesh-llm caches it
 * on disk for 24h, so re-asking on a timer would add nothing.
 *
 * `undefined` while in flight and `null` on failure are deliberately distinct —
 * the experimental tier has to tell "still looking" from "couldn't read the
 * catalog" from "the catalog genuinely has no packages", and all three would
 * otherwise render as an unexplained empty list.
 *
 * Requires no running node: you have to choose what to share before you start
 * sharing it.
 */
export function useMeshExperimentalCatalog(enabled: boolean): {
  catalog: MeshExperimentalCatalog | null | undefined;
  error: string | null;
} {
  const [catalog, setCatalog] = React.useState<
    MeshExperimentalCatalog | null | undefined
  >(undefined);
  const [error, setError] = React.useState<string | null>(null);
  const loaded = React.useRef(false);

  React.useEffect(() => {
    // Only when the section is actually opened: the first call can download the
    // catalog, which is not work to do for a screen nobody expanded.
    if (!enabled || loaded.current) return;
    loaded.current = true;
    let cancelled = false;
    void (async () => {
      try {
        const next = await meshExperimentalCatalog();
        if (!cancelled) {
          setCatalog(next);
          setError(null);
        }
      } catch (err) {
        if (!cancelled) {
          // null, not an empty catalog: "could not read" must not masquerade as
          // "nothing published".
          setCatalog(null);
          setError(err instanceof Error ? err.message : String(err));
          // Allow a retry on reopen — a transient network failure should not
          // permanently empty the section.
          loaded.current = false;
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [enabled]);

  return { catalog, error };
}
