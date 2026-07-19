import type { Project, ProjectRepoSnapshot } from "@/features/projects/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Tooltip for the push/pull sync buttons, e.g. "Pull 2 remote commits". */
export function pushPullTitle(
  verb: "Push" | "Pull",
  count: number | undefined,
  side: "local" | "remote",
) {
  if (!count) return `${verb} ${side} commits`;
  return `${verb} ${count} ${side} ${count === 1 ? "commit" : "commits"}`;
}

/** Returns the normalized owner and contributor pubkeys for a project. */
export function projectPeople(project: Project) {
  return [
    ...new Set(
      [project.owner, ...project.contributors]
        .filter(Boolean)
        .map(normalizePubkey),
    ),
  ];
}

/** Reports whether a repository snapshot contains any displayable content. */
export function snapshotHasContent(
  snapshot: ProjectRepoSnapshot | null | undefined,
) {
  return Boolean(
    snapshot &&
      (snapshot.latestCommit ||
        snapshot.commits.length > 0 ||
        snapshot.files.length > 0 ||
        snapshot.contributors.length > 0),
  );
}
