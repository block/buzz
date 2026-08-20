import type {
  ProjectRepoSnapshot,
  Repository as Project,
} from "@/features/projects/hooks";
import type { EntityLinkTab } from "@/shared/lib/entityLink";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const PROJECT_TAB_CRUMB_LABELS: Record<string, string> = {
  files: "Files",
  activity: "Commits",
  issues: "Tasks",
  prs: "Review",
  contributors: "Contributors",
  channels: "Channels",
};

export const PROJECT_REPOSITORY_SEARCH_KEYS = [
  "repositoryId",
  "issueId",
  "pullRequestId",
  "commitHash",
] as const;

export type ProjectDetailScreenProps = {
  commitHash?: string;
  entityNavigationId?: string;
  projectId: string;
  pullRequestId?: string;
  issueId?: string;
  repositoryId?: string;
  /** Workspace tab requested by a share link (link vocabulary). */
  tab?: EntityLinkTab;
};

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
