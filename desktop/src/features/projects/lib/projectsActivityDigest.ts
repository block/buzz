import type {
  Project,
  ProjectActivitySummary,
  ProjectIssueListItem,
  ProjectPullRequestListItem,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";

const WEEK_SECONDS = 7 * 24 * 60 * 60;

export type ProjectsActivityDigest = {
  highlights: string[];
  prefix: string;
  suffix: string;
};

function plural(count: number, singular: string, pluralForm = `${singular}s`) {
  return `${count} ${count === 1 ? singular : pluralForm}`;
}

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
    commitCount > 0 ? `${plural(commitCount, "new commit")}` : null,
    taskCount > 0 ? `${plural(taskCount, "task")} opened` : null,
    reviewCount > 0 ? `${plural(reviewCount, "review")} opened` : null,
  ].filter((value): value is string => value !== null);

  if (highlights.length > 0) {
    highlights.push(`${plural(activeProjectIds.size, "active project")}`);
    return {
      highlights,
      prefix: "This week:",
      suffix: ".",
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
      plural(projects.length, "project"),
      plural(totals.tasks, "task"),
      plural(totals.reviews, "review"),
    ],
    prefix: "Currently tracking",
    suffix: ".",
  };
}
