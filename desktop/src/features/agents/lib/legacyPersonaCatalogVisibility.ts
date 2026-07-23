const LEGACY_PERSONA_CATALOG_VISIBILITY_STORAGE_KEY =
  "buzz-persona-catalog-visibility-v1";

/**
 * Removes the retired custom-persona catalog preference so it cannot resurface
 * agents after the visibility control has been removed.
 */
export function clearLegacyPersonaCatalogVisibility(
  storage?: Pick<Storage, "removeItem"> | null,
) {
  let targetStorage = storage;
  if (targetStorage === undefined) {
    if (typeof window === "undefined") return;

    try {
      targetStorage = window.localStorage;
    } catch {
      return;
    }
  }
  if (!targetStorage) return;

  try {
    targetStorage.removeItem(LEGACY_PERSONA_CATALOG_VISIBILITY_STORAGE_KEY);
  } catch {
    // Catalog cleanup is best-effort and should not block the agents view.
  }
}
