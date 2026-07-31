import { KIND_REPO_ANNOUNCEMENT } from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";

export function projectRepoCoordinate(
  project: Pick<{ owner: string; dtag: string }, "owner" | "dtag">,
): string {
  return `${KIND_REPO_ANNOUNCEMENT}:${project.owner}:${project.dtag}`;
}

/**
 * True when a kind:5 deletion should hide this announcement.
 *
 * NIP-09: a deletion is only valid when signed by the author of the referenced
 * event, and only applies to events that existed when the deletion was
 * published. A newer replaceable announcement at the same coordinate is live.
 */
export function isProjectHiddenByDeletion(
  project: Pick<{ owner: string; dtag: string; createdAt: number }, "owner" | "dtag" | "createdAt">,
  deletionEvents: RelayEvent[],
): boolean {
  const coordinate = projectRepoCoordinate(project);
  return deletionEvents.some(
    (event) =>
      event.created_at >= project.createdAt &&
      event.pubkey.toLowerCase() === project.owner.toLowerCase() &&
      event.tags.some((tag) => tag[0] === "a" && tag[1] === coordinate),
  );
}
