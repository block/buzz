export const teamCatalogCopy = {
  chooseFromCatalog: "Choose from catalog",
  dialogTitle: "Team Catalog",
  dialogDescription: "Browse teams shared to this relay.",
  emptyCatalogTitle: "No teams are being shared",
  emptyCatalogDescription: "Shared teams will appear here.",
  addAction: "Add team",
  addedAction: "Added to my teams",
  addingAction: "Adding…",
  shareTitle: "Share to catalog",
  shareDescription:
    "Anyone in this community can find and add a copy of this team. Both the team instructions and every member’s instructions are shared as plaintext. Memories and secrets aren’t included.",
  invalidMemberSingular:
    "1 member in this team could not be verified and cannot be added.",
  invalidMemberPlural: (count: number) =>
    `${count} members in this team could not be verified and cannot be added.`,
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
  return `"${teamName}" has been queued for removal from the community catalog because it can no longer be projected: ${reason}`;
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
      ? `Sharing ${teamName} is queued. It will appear after the relay accepts the update.`
      : `Removing ${teamName} is queued. It may remain discoverable until the relay accepts the update.`;
  }
  return shared
    ? `Published ${teamName} to the community catalog.`
    : `${teamName} is no longer discoverable in the community catalog.`;
}
