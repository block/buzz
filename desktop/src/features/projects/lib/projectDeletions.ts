import type { Project } from "@/features/projects/hooks";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_REPO_ANNOUNCEMENT } from "@/shared/constants/kinds";

/**
 * The NIP-34 repo address (`30617:<owner>:<dtag>`) — the coordinate a kind:5
 * deletion targets with an `a` tag, and the identity two forks of the same
 * dtag are distinguished by.
 */
export function projectCoordinate(
  project: Pick<Project, "owner" | "dtag">,
): string {
  return `${KIND_REPO_ANNOUNCEMENT}:${project.owner}:${project.dtag}`;
}

/**
 * Whether a kind:5 tombstone in `deletionEvents` hides this repo announcement.
 *
 * A repo announcement is addressable (kind:30617), so its coordinate outlives
 * any single event at it: deleting a repo and announcing the same dtag again
 * is a legitimate recovery path, and the re-announcement is live. NIP-09
 * scopes an `a`-tag deletion to versions "up to the `created_at` timestamp of
 * the deletion request event" — so a tombstone only hides announcements at or
 * older than itself, never a newer one published after it.
 *
 * Without that timestamp bound a single deletion hid the coordinate forever:
 * the relay kept serving the newer announcement and git clone/push worked,
 * but the card never rendered again for anyone (#3760).
 */
export function isDeletedByA(
  project: Pick<Project, "owner" | "dtag" | "createdAt">,
  deletionEvents: RelayEvent[],
): boolean {
  const coordinate = projectCoordinate(project);

  return deletionEvents.some(
    (event) =>
      // NIP-09: a deletion is only valid when signed by the author of the
      // referenced event — otherwise anyone could hide someone else's project.
      event.pubkey.toLowerCase() === project.owner.toLowerCase() &&
      event.created_at >= project.createdAt &&
      event.tags.some((tag) => tag[0] === "a" && tag[1] === coordinate),
  );
}
