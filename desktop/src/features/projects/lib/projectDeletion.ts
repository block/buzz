/**
 * NIP-09 address deletion for NIP-34 repo announcements (kind:30617).
 *
 * Desktop loads bare kind:5 history and filters projects client-side. A delete
 * targets the replaceable address `30617:<owner>:<dtag>`. After the owner
 * deletes and later re-announces the same d-tag, the new announcement must
 * surface again — an older tombstone must not hide it forever.
 */

export type ProjectDeletionTarget = {
  owner: string;
  dtag: string;
  /** Announcement `created_at` (unix seconds). */
  createdAt: number;
};

export type ProjectDeletionEvent = {
  pubkey: string;
  created_at: number;
  tags: string[][];
};

/** Canonical NIP-33/34 address for a repo announcement. */
export function projectRepoAddress(
  kind: number,
  project: Pick<ProjectDeletionTarget, "owner" | "dtag">,
): string {
  // Owner pubkeys are hex; normalize so address matches are case-stable.
  return `${kind}:${project.owner.toLowerCase()}:${project.dtag}`;
}

/**
 * Returns true when an owner-signed kind:5 with `a` = the project coordinate
 * is at least as new as the current announcement.
 *
 * - Foreign pubkeys cannot hide someone else's project.
 * - A deletion older than the current announcement does not apply (resurrect
 *   after delete + re-announce with the same d-tag).
 * - Equal timestamps treat the deletion as winning (fail closed on same-second races).
 */
export function isProjectDeletedByAddress(
  kind: number,
  project: ProjectDeletionTarget,
  deletionEvents: ProjectDeletionEvent[],
): boolean {
  const coordinate = projectRepoAddress(kind, project);
  const owner = project.owner.toLowerCase();

  return deletionEvents.some(
    (event) =>
      event.pubkey.toLowerCase() === owner &&
      event.created_at >= project.createdAt &&
      event.tags.some((tag) => tag[0] === "a" && tag[1] === coordinate),
  );
}
