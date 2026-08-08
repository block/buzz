import type { MeshCatalogEntry } from "@/shared/api/tauriMesh";

/**
 * Pick the first usable model to prefill Share compute when the member has not
 * chosen one yet. Prefer the curated/recommended path the UI shows above the
 * fold, but never auto-select entries marked too large for this machine.
 */
export function defaultShareModelFromCatalog(
  entries: MeshCatalogEntry[],
): string | null {
  const usable = entries.filter((entry) => entry.fit !== "too_large");
  return (
    usable.find((entry) => entry.recommended)?.name ??
    usable.find((entry) => entry.curated)?.name ??
    usable[0]?.name ??
    null
  );
}
