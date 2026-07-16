import type { MeshModelCatalog } from "@/shared/api/tauriMesh";

/**
 * Is `ref` one of the curated shared/split models?
 *
 * Shared models are layer-packages (`meshllm/…-layers`) that are too big for
 * one machine and run split across several members. Serving one does nothing
 * until enough members join; the mesh then auto-splits it across the group.
 *
 * Detection is an exact-name match against the catalog's `shared` list — that
 * is what drives the cohort-aware status copy ("waiting for members" vs a
 * plain solo "starting").
 */
export function isSharedModelRef(
  ref: string,
  catalog: MeshModelCatalog | null,
): boolean {
  if (!catalog) return false;
  const trimmed = ref.trim();
  if (trimmed.length === 0) return false;
  return catalog.shared.some((m) => m.name === trimmed);
}

/**
 * Short, human display name for a shared layer-package ref, e.g.
 * `meshllm/Qwen3-235B-A22B-UD-Q4_K_XL-layers` → `Qwen3-235B-A22B-UD-Q4_K_XL`.
 */
export function sharedModelShortName(ref: string): string {
  return ref.replace(/^meshllm\//, "").replace(/-layers$/, "");
}
