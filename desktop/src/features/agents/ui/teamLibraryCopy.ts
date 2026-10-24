// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
export const teamCatalogCopy = {
  get chooseFromCatalog() {
    return uiT("Choose from catalog");
  },
  get dialogTitle() {
    return uiT("Team Catalog");
  },
  get dialogDescription() {
    return uiT("Browse teams shared to this relay.");
  },
  get emptyCatalogTitle() {
    return uiT("No teams are being shared");
  },
  get emptyCatalogDescription() {
    return uiT("Shared teams will appear here.");
  },
  get addAction() {
    return uiT("Add team");
  },
  get addedAction() {
    return uiT("Added to my teams");
  },
  get addingAction() {
    return uiT("Adding…");
  },
  get shareTitle() {
    return uiT("Share to catalog");
  },
  get shareDescription() {
    return uiT(
      "Anyone in this community can find and add a copy of this team. Both the team instructions and every member’s instructions are shared as plaintext. Memories and secrets aren’t included.",
    );
  },
} as const;

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
  return uiT(
    '"{0}" has been queued for removal from the community catalog because it can no longer be projected: {1}',
    { p0: teamName, p1: reason },
  );
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
      ? uiT(
          "Sharing {0} is queued. It will appear after the relay accepts the update.",
          { p0: teamName },
        )
      : uiT(
          "Removing {0} is queued. It may remain discoverable until the relay accepts the update.",
          { p0: teamName },
        );
  }
  return shared
    ? uiT("Published {0} to the community catalog.", { p0: teamName })
    : uiT("{0} is no longer discoverable in the community catalog.", {
        p0: teamName,
      });
}
