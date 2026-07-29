import type { PersonaSharePublicationResult } from "@/shared/api/tauriPersonas";

/**
 * The confirmation shown after a persona edit is saved.
 *
 * `publicationStatus` is null when the edit did not promise publication, so
 * the copy stays silent about the catalog. When it did, the copy must
 * distinguish a relay-accepted publish from a queued one — a "published"
 * message for an edit still sitting in the outbox is the promise the
 * "Save and publish" button was making falsely.
 */
export function personaSaveNotice(
  displayName: string,
  publicationStatus: PersonaSharePublicationResult["publicationStatus"] | null,
): string {
  switch (publicationStatus) {
    case "published":
      return `Updated ${displayName} and published it to the community catalog.`;
    case "queued":
      return `Updated ${displayName}. Publishing to the community catalog is queued and will appear after the relay accepts the update.`;
    default:
      return `Updated ${displayName}.`;
  }
}
