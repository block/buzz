export type CustomEmojiCatalogStatus = {
  /** The latest fetch failed. With `hasCatalog` the last good list still shows. */
  refreshFailed: boolean;
  /** A catalog has loaded at least once. */
  hasCatalog: boolean;
  /** The relay returned the full set limit, so some sets may be missing. */
  setLimitReached: boolean;
  /** Entries left out of the palette by the entry cap. */
  droppedEntries: number;
};

type CatalogReport = {
  setLimitReached: boolean;
  droppedEntries: number;
};

/** Derive the status from the palette query result and the last catalog report. */
export function customEmojiCatalogStatus(
  query: { isError: boolean; data: unknown },
  report: CatalogReport | undefined,
): CustomEmojiCatalogStatus {
  return {
    refreshFailed: query.isError,
    hasCatalog: query.data !== undefined,
    setLimitReached: report?.setLimitReached ?? false,
    droppedEntries: report?.droppedEntries ?? 0,
  };
}

/** Settings-card notice for the status, or null when the catalog is healthy and complete. */
export function customEmojiCatalogNotice(
  status: CustomEmojiCatalogStatus,
): string | null {
  if (status.refreshFailed) {
    return status.hasCatalog
      ? "Could not refresh community emoji. Showing the last loaded list."
      : "Could not load community emoji.";
  }
  if (status.droppedEntries > 0) {
    return `Community emoji list is too large. ${status.droppedEntries} emoji are hidden.`;
  }
  if (status.setLimitReached) return "Community emoji list may be incomplete.";
  return null;
}
