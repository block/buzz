import type {
  Project,
  ProjectActivitySummary,
  ProjectIssueListItem,
  ProjectPullRequestListItem,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import { i18n } from "@/i18n";

const WEEK_SECONDS = 7 * 24 * 60 * 60;

export type ProjectsActivityDigest = {
  highlights: string[];
  prefix: string;
  suffix: string;
};

/** Builds a short, deterministic sentence from the currently loaded activity. */
export function buildProjectsActivityDigest({
  issues,
  nowSeconds,
  projects,
  pullRequests,
  snapshots,
  summaries,
}: {
  issues: ProjectIssueListItem[];
  nowSeconds: number;
  projects: Project[];
  pullRequests: ProjectPullRequestListItem[];
  snapshots?: Record<string, ProjectRepoSnapshot>;
  summaries?: Record<string, ProjectActivitySummary>;
}): ProjectsActivityDigest {
  const since = nowSeconds - WEEK_SECONDS;
  const activeProjectIds = new Set<string>();
  let commitCount = 0;
  for (const [projectId, snapshot] of Object.entries(snapshots ?? {})) {
    const recent = snapshot.commits.filter(
      (commit) => commit.timestamp >= since,
    ).length;
    commitCount += recent;
    if (recent > 0) activeProjectIds.add(projectId);
  }
  const taskCount = issues.filter(({ issue, project }) => {
    const recent = issue.createdAt >= since;
    if (recent) activeProjectIds.add(project.id);
    return recent;
  }).length;
  const reviewCount = pullRequests.filter(({ project, pullRequest }) => {
    const recent = pullRequest.createdAt >= since;
    if (recent) activeProjectIds.add(project.id);
    return recent;
  }).length;
  const highlights = [
    commitCount > 0
      ? i18n.t("projects.digest.new-commits", { count: commitCount })
      : null,
    taskCount > 0
      ? i18n.t("projects.digest.tasks-opened", { count: taskCount })
      : null,
    reviewCount > 0
      ? i18n.t("projects.digest.reviews-opened", { count: reviewCount })
      : null,
  ].filter((value): value is string => value !== null);

  if (highlights.length > 0) {
    highlights.push(
      i18n.t("projects.digest.active-projects", {
        count: activeProjectIds.size,
      }),
    );
    return {
      highlights,
      prefix: i18n.t("projects.digest.prefix-week"),
      suffix: i18n.t("projects.digest.suffix"),
    };
  }

  const totals = Object.values(summaries ?? {}).reduce(
    (result, summary) => ({
      reviews: result.reviews + summary.prCount,
      tasks: result.tasks + summary.issueCount,
    }),
    { reviews: 0, tasks: 0 },
  );
  return {
    highlights: [
      i18n.t("projects.digest.projects", { count: projects.length }),
      i18n.t("projects.digest.tasks", { count: totals.tasks }),
      i18n.t("projects.digest.reviews", { count: totals.reviews }),
    ],
    prefix: i18n.t("projects.digest.prefix-currently-tracking"),
    suffix: i18n.t("projects.digest.suffix"),
  };
}
