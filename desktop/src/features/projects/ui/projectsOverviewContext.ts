import type {
  Project,
  ProjectActivitySummary,
  ProjectIssue,
  ProjectPullRequest,
} from "@/features/projects/hooks";
import { projectContributorActivityCounts } from "@/features/projects/lib/projectContributorMatching";
import { uniqueProjectRelatedChannelCount } from "@/features/projects/lib/projectRelatedChannels";
import type { ProjectsFilter } from "@/features/projects/lib/projectsViewHelpers";
import { normalizePubkey } from "@/shared/lib/pubkey";

import {
  projectReviewActivity,
  projectTaskActivity,
} from "./projectRightPanelContext";
import { projectsSectionTitle } from "./projectsSectionMeta";

export type ProjectsOverviewSection =
  | "projects"
  | "repositories"
  | "channels"
  | "prs"
  | "issues";

export type OverviewContextStatIcon =
  | "projects"
  | "repositories"
  | "channels"
  | "tasks"
  | "reviews"
  | "completed"
  | "merged";

export type OverviewContextAction = {
  kind: "project" | "issue" | "pullRequest";
  label: string;
  testId: string;
} | null;

export type OverviewContextStat = {
  count: number;
  icon: OverviewContextStatIcon;
  label: string;
  section: ProjectsOverviewSection;
};

export type OverviewContextPresentation = {
  action: OverviewContextAction;
  activityByDay: Record<string, number> | null;
  detailsTitle: string;
  people: string[];
  stats: OverviewContextStat[];
  title: string;
};

type OverviewContextInput = {
  filter: ProjectsFilter;
  issues: ProjectIssue[];
  projects: Project[];
  pullRequests: ProjectPullRequest[];
  summaries?: Record<string, ProjectActivitySummary>;
};

function summaryWorkItemCounts(
  projects: Project[],
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  return projects.reduce(
    (stats, project) => {
      const summary = summaries?.[project.id];
      return {
        issues: stats.issues + (summary?.issueCount ?? 0),
        prs: stats.prs + (summary?.prCount ?? 0),
      };
    },
    { issues: 0, prs: 0 },
  );
}

function repositoryCount(projects: Project[]) {
  return projects.reduce(
    (count, project) => count + project.repositories.length,
    0,
  );
}

function resolvedTaskActivity(
  issues: ProjectIssue[],
  summaryIssueCount: number,
) {
  if (issues.length > 0) return projectTaskActivity(issues);
  return {
    active: summaryIssueCount,
    completed: 0,
    total: summaryIssueCount,
  };
}

function resolvedReviewActivity(
  pullRequests: ProjectPullRequest[],
  summaryPrCount: number,
) {
  if (pullRequests.length > 0) return projectReviewActivity(pullRequests);
  return {
    merged: 0,
    open: summaryPrCount,
    total: summaryPrCount,
  };
}

function createProjectAction(): OverviewContextAction {
  return {
    kind: "project",
    label: "Create project",
    testId: "projects-overview-create-project",
  };
}

function uniquePubkeys(pubkeys: Array<string | null | undefined>) {
  return [
    ...new Set(
      pubkeys
        .filter(
          (pubkey): pubkey is string =>
            typeof pubkey === "string" && pubkey.trim().length > 0,
        )
        .map((pubkey) => normalizePubkey(pubkey)),
    ),
  ];
}

function summaryActivityByDay(
  projects: Project[],
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  const merged: Record<string, number> = {};
  for (const project of projects) {
    const byDay = summaries?.[project.id]?.activityByDay;
    if (!byDay) continue;
    for (const [day, count] of Object.entries(byDay)) {
      merged[day] = (merged[day] ?? 0) + count;
    }
  }
  return merged;
}

function latestCommitAuthors(
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  if (!summaries) return [];
  return uniquePubkeys(
    Object.values(summaries)
      .map((summary) => summary.latestCommit?.author)
      .filter((author): author is string => Boolean(author)),
  );
}

function workspaceActorCounts(
  issues: ProjectIssue[],
  pullRequests: ProjectPullRequest[],
) {
  return projectContributorActivityCounts({
    owner: "",
    contributors: [],
    issues: issues
      .filter((issue) => issue.author)
      .map((issue) => ({
        author: issue.author,
        comments: issue.comments ?? [],
      })),
    pullRequests: pullRequests
      .filter((pullRequest) => pullRequest.author)
      .map((pullRequest) => ({
        author: pullRequest.author,
        approvals: pullRequest.approvals ?? [],
        comments: pullRequest.comments ?? [],
        commit: pullRequest.commit,
        initialCommit: pullRequest.initialCommit,
        updates: pullRequest.updates ?? [],
      })),
  });
}

function peopleWithKind(
  counts: ReturnType<typeof workspaceActorCounts>,
  kind: "any" | "commits" | "reviews" | "tasks",
) {
  return Object.entries(counts)
    .filter(([, activity]) => {
      if (kind === "tasks") return activity.tasks > 0;
      if (kind === "reviews") return activity.reviews > 0;
      if (kind === "commits") return activity.commits > 0;
      return activity.tasks + activity.reviews + activity.commits > 0;
    })
    .map(([pubkey]) => pubkey);
}

function projectOwners(projects: Project[]) {
  return uniquePubkeys(
    projects.flatMap((project) => [
      project.owner,
      ...project.repositories.map((repository) => repository.owner),
    ]),
  );
}

function overviewContextPeople({
  filter,
  issues,
  projects,
  pullRequests,
  summaries,
}: OverviewContextInput) {
  const counts = workspaceActorCounts(issues, pullRequests);
  const commitAuthors = uniquePubkeys([
    ...peopleWithKind(counts, "commits"),
    ...latestCommitAuthors(summaries),
  ]);

  if (filter === "issues") return peopleWithKind(counts, "tasks");
  if (filter === "prs") return peopleWithKind(counts, "reviews");
  if (filter === "repositories") return commitAuthors;
  if (filter === "channels") return [];
  if (filter === "all") {
    return uniquePubkeys([...peopleWithKind(counts, "any"), ...commitAuthors]);
  }
  return projectOwners(projects);
}

function overviewContextActivityByDay({
  filter,
  projects,
  summaries,
}: OverviewContextInput): Record<string, number> | null {
  if (filter === "all" || filter === "projects" || filter === "repositories") {
    return summaryActivityByDay(projects, summaries);
  }
  return null;
}

/** Title, people, activity, and stats for the Projects overview context pod. */
export function projectsOverviewContext(
  input: OverviewContextInput,
): OverviewContextPresentation {
  const { filter, issues, projects, pullRequests, summaries } = input;
  const summaryCounts = summaryWorkItemCounts(projects, summaries);
  const tasks = resolvedTaskActivity(issues, summaryCounts.issues);
  const reviews = resolvedReviewActivity(pullRequests, summaryCounts.prs);
  const channelCount = uniqueProjectRelatedChannelCount(projects);
  const repositories = repositoryCount(projects);
  const people = overviewContextPeople(input);
  const activityByDay = overviewContextActivityByDay(input);

  if (filter === "repositories") {
    return {
      action: null,
      activityByDay,
      detailsTitle: "Repository activity",
      people,
      stats: [
        {
          count: repositories,
          icon: "repositories",
          label: "Repositories",
          section: "repositories",
        },
        {
          count: tasks.active,
          icon: "tasks",
          label: "Active tasks",
          section: "issues",
        },
        {
          count: reviews.open,
          icon: "reviews",
          label: "Open reviews",
          section: "prs",
        },
      ],
      title: "Repositories",
    };
  }

  if (filter === "channels") {
    return {
      action: null,
      activityByDay,
      detailsTitle: "Details",
      people,
      stats: [
        {
          count: channelCount,
          icon: "channels",
          label: "Channels",
          section: "channels",
        },
        {
          count: projects.length,
          icon: "projects",
          label: "Projects",
          section: "projects",
        },
        {
          count: repositories,
          icon: "repositories",
          label: "Repositories",
          section: "repositories",
        },
      ],
      title: "Channels",
    };
  }

  if (filter === "issues") {
    return {
      action: {
        kind: "issue",
        label: "Create task",
        testId: "projects-overview-create-issue",
      },
      activityByDay,
      detailsTitle: "Details",
      people,
      stats: [
        {
          count: tasks.total,
          icon: "tasks",
          label: "Tasks",
          section: "issues",
        },
        {
          count: tasks.active,
          icon: "tasks",
          label: "Active",
          section: "issues",
        },
        {
          count: tasks.completed,
          icon: "completed",
          label: "Completed",
          section: "issues",
        },
      ],
      title: "Tasks",
    };
  }

  if (filter === "prs") {
    return {
      action: {
        kind: "pullRequest",
        label: "Create review",
        testId: "projects-overview-create-pull-request",
      },
      activityByDay,
      detailsTitle: "Review activity",
      people,
      stats: [
        {
          count: reviews.total,
          icon: "reviews",
          label: "Reviews",
          section: "prs",
        },
        {
          count: reviews.open,
          icon: "reviews",
          label: "Open",
          section: "prs",
        },
        {
          count: reviews.merged,
          icon: "merged",
          label: "Merged",
          section: "prs",
        },
      ],
      title: "Reviews",
    };
  }

  if (filter === "all") {
    return {
      action: createProjectAction(),
      activityByDay,
      detailsTitle: "Details",
      people,
      stats: [
        {
          count: projects.length,
          icon: "projects",
          label: "Projects",
          section: "projects",
        },
        {
          count: repositories,
          icon: "repositories",
          label: "Repositories",
          section: "repositories",
        },
        {
          count: channelCount,
          icon: "channels",
          label: "Channels",
          section: "channels",
        },
        {
          count: tasks.total,
          icon: "tasks",
          label: "Tasks",
          section: "issues",
        },
        {
          count: reviews.total,
          icon: "reviews",
          label: "Reviews",
          section: "prs",
        },
      ],
      title: "Projects",
    };
  }

  return {
    action: createProjectAction(),
    activityByDay,
    detailsTitle: "Details",
    people,
    stats: [
      {
        count: projects.length,
        icon: "projects",
        label: "Projects",
        section: "projects",
      },
      {
        count: repositories,
        icon: "repositories",
        label: "Repositories",
        section: "repositories",
      },
      {
        count: channelCount,
        icon: "channels",
        label: "Channels",
        section: "channels",
      },
    ],
    title: projectsSectionTitle(filter),
  };
}
