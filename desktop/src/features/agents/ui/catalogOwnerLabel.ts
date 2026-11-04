// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
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
    summary?.displayName?.trim() ||
    summary?.name?.trim() ||
    uiT("Community member")
  );
}
