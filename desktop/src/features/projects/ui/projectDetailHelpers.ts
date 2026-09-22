import type {
  ProjectRepoSnapshot,
  Repository as Project,
} from "@/features/projects/hooks";
import { i18n } from "@/i18n";
import type { EntityLinkTab } from "@/shared/lib/entityLink";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const PROJECT_REPOSITORY_SEARCH_KEYS = [
  "repositoryId",
  "issueId",
  "pullRequestId",
  "commitHash",
  "filePath",
] as const;

/** Breadcrumb label for a workspace tab, or `null` for an unknown tab. */
export function projectTabCrumbLabel(tab: string): string | null {
  if (tab === "files") return i18n.t("projects.tabs.files");
  if (tab === "activity") return i18n.t("projects.tabs.commits");
  if (tab === "issues") return i18n.t("projects.tabs.tasks");
  if (tab === "prs") return i18n.t("projects.tabs.review");
  if (tab === "contributors") return i18n.t("projects.tabs.contributors");
  if (tab === "channels") return i18n.t("projects.tabs.channels");
  return null;
}

export type ProjectDetailScreenProps = {
  commitHash?: string;
  entityNavigationId?: string;
  filePath?: string;
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
  const isPush = verb === "Push";
  const isLocal = side === "local";
  if (!count) {
    if (isPush && isLocal) return i18n.t("projects.sync.push-local-any");
    if (isPush) return i18n.t("projects.sync.push-remote-any");
    if (isLocal) return i18n.t("projects.sync.pull-local-any");
    return i18n.t("projects.sync.pull-remote-any");
  }
  if (isPush && isLocal) return i18n.t("projects.sync.push-local", { count });
  if (isPush) return i18n.t("projects.sync.push-remote", { count });
  if (isLocal) return i18n.t("projects.sync.pull-local", { count });
  return i18n.t("projects.sync.pull-remote", { count });
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
