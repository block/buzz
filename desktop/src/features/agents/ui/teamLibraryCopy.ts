import { i18n } from "@/i18n";

/**
 * Copy for the team catalog / "My teams" surfaces. Resolved through `i18n.t` at
 * call time, so callers read the strings while rendering rather than at import.
 */
export function teamCatalogCopy() {
  return {
    chooseFromCatalog: i18n.t("agents.team-catalog.choose-from-catalog"),
    dialogTitle: i18n.t("agents.team-catalog.title"),
    dialogDescription: i18n.t("agents.team-catalog.description"),
    emptyCatalogTitle: i18n.t("agents.team-catalog.empty-title"),
    emptyCatalogDescription: i18n.t("agents.team-catalog.empty-description"),
    addAction: i18n.t("agents.team-catalog.add-team"),
    addedAction: i18n.t("agents.team-catalog.added-team"),
    addingAction: i18n.t("agents.team-catalog.adding-team"),
    shareTitle: i18n.t("agents.team-catalog.share-title"),
    shareDescription: i18n.t("agents.team-catalog.share-description"),
  };
}

/**
 * The warning notice shown when the backend automatically queues a retraction
 * for a shared team that can no longer be projected.
 *
 * "Queued" is accurate — the tombstone has been enqueued for the flush loop
 * but the relay head may still be discoverable until the flush succeeds.
 * Using "queued for removal" rather than "was removed" avoids a false claim
 * that the catalog has already changed.
 */
export function teamAutoRetractedNotice(
  teamName: string,
  reason: string,
): string {
  return i18n.t("agents.team-catalog.auto-retracted", { teamName, reason });
}

/**
 * The result message for a share toggle.
 *
 * `queued` is not a failure: the head is durably enqueued and the flush loop
 * will publish it, so the copy promises eventual visibility rather than
 * claiming the catalog already changed.
 */
export function teamShareNotice(
  teamName: string,
  shared: boolean,
  publicationStatus: "published" | "queued",
): string {
  if (publicationStatus === "queued") {
    return shared
      ? i18n.t("agents.team-catalog.share-queued", { teamName })
      : i18n.t("agents.team-catalog.unshare-queued", { teamName });
  }
  return shared
    ? i18n.t("agents.team-catalog.published", { teamName })
    : i18n.t("agents.team-catalog.unshared", { teamName });
}
