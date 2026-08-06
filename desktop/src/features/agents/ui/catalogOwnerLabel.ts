/**
 * Derives the "Added by" label for a catalog entry from a resolved profile
 * summary. Prefers `displayName`, falls back to `name`, then to the default
 * "Community member" string when both are absent, null, or whitespace-only.
 */
export function resolveCatalogOwnerLabel(
  summary:
    | { displayName?: string | null; name?: string | null }
    | null
    | undefined,
): string {
  return (
    summary?.displayName?.trim() || summary?.name?.trim() || "Community member"
  );
}
