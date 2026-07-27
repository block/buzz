import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_TEXT_NOTE,
} from "@/shared/constants/kinds";
import {
  getTag,
  type ProjectIssue,
  projectIssueEventsToIssues,
} from "./projectIssues.mjs";
import {
  type ProjectPullRequest,
  projectPullRequestEventsToPullRequests,
} from "./projectPullRequests.mjs";

type ProjectReference = {
  repoAddress: string;
};

/** Optional event groups that can fail without discarding root work items. */
export type ProjectWorkItemSection =
  | "comments"
  | "pull-request-updates"
  | "statuses";

/** Aggregate work items plus any optional event groups that failed to load. */
export type ProjectsWorkItemsResult<TProject extends ProjectReference> = {
  issues: {
    items: Array<{ project: TProject; issue: ProjectIssue }>;
    failedSections: ProjectWorkItemSection[];
  };
  pullRequests: {
    items: Array<{ project: TProject; pullRequest: ProjectPullRequest }>;
    failedSections: ProjectWorkItemSection[];
  };
};

function groupByRepoAddress(events: RelayEvent[]): Map<string, RelayEvent[]> {
  const grouped = new Map<string, RelayEvent[]>();
  for (const event of events) {
    const repoAddress = getTag(event, "a");
    if (!repoAddress) continue;
    const projectEvents = grouped.get(repoAddress) ?? [];
    projectEvents.push(event);
    grouped.set(repoAddress, projectEvents);
  }
  return grouped;
}

/** Loads aggregate issue and pull-request data with bounded relay fan-out. */
export async function fetchProjectsWorkItems<TProject extends ProjectReference>(
  projects: TProject[],
): Promise<ProjectsWorkItemsResult<TProject>> {
  const repoAddresses = [
    ...new Set(projects.map((project) => project.repoAddress)),
  ];
  const [rootResult, updateResult, commentResult, statusResult] =
    await Promise.allSettled([
      relayClient.fetchEvents({
        kinds: [KIND_GIT_ISSUE, KIND_GIT_PULL_REQUEST],
        "#a": repoAddresses,
        limit: 2_000,
      }),
      relayClient.fetchEvents({
        kinds: [KIND_GIT_PR_UPDATE],
        "#a": repoAddresses,
        limit: 2_000,
      }),
      relayClient.fetchEvents({
        kinds: [KIND_TEXT_NOTE],
        "#a": repoAddresses,
        limit: 2_000,
      }),
      relayClient.fetchEvents({
        kinds: [
          KIND_GIT_STATUS_OPEN,
          KIND_GIT_STATUS_MERGED,
          KIND_GIT_STATUS_CLOSED,
          KIND_GIT_STATUS_DRAFT,
        ],
        "#a": repoAddresses,
        limit: 2_000,
      }),
    ]);

  if (rootResult.status === "rejected") {
    throw rootResult.reason instanceof Error
      ? rootResult.reason
      : new Error("Could not load project issues and pull requests.");
  }

  const updateEvents =
    updateResult.status === "fulfilled" ? updateResult.value : [];
  const commentEvents =
    commentResult.status === "fulfilled" ? commentResult.value : [];
  const statusEvents =
    statusResult.status === "fulfilled" ? statusResult.value : [];
  const rootsByRepo = groupByRepoAddress(rootResult.value);
  const updatesByRepo = groupByRepoAddress(updateEvents);
  const commentsByRepo = groupByRepoAddress(commentEvents);
  const statusesByRepo = groupByRepoAddress(statusEvents);

  const pullRequests = projects
    .flatMap((project) =>
      projectPullRequestEventsToPullRequests(
        (rootsByRepo.get(project.repoAddress) ?? []).filter(
          (event) => event.kind === KIND_GIT_PULL_REQUEST,
        ),
        updatesByRepo.get(project.repoAddress) ?? [],
        commentsByRepo.get(project.repoAddress) ?? [],
        statusesByRepo.get(project.repoAddress) ?? [],
      ).map((pullRequest) => ({ project, pullRequest })),
    )
    .sort(
      (left, right) => right.pullRequest.updatedAt - left.pullRequest.updatedAt,
    );
  const issues = projects
    .flatMap((project) =>
      projectIssueEventsToIssues(
        (rootsByRepo.get(project.repoAddress) ?? []).filter(
          (event) => event.kind === KIND_GIT_ISSUE,
        ),
        statusesByRepo.get(project.repoAddress) ?? [],
        commentsByRepo.get(project.repoAddress) ?? [],
      ).map((issue) => ({ project, issue })),
    )
    .sort((left, right) => right.issue.updatedAt - left.issue.updatedAt);
  const sharedFailedSections: ProjectWorkItemSection[] = [];
  if (commentResult.status === "rejected") {
    sharedFailedSections.push("comments");
  }
  if (statusResult.status === "rejected") {
    sharedFailedSections.push("statuses");
  }
  const pullRequestFailedSections = [...sharedFailedSections];
  if (updateResult.status === "rejected") {
    pullRequestFailedSections.unshift("pull-request-updates");
  }

  return {
    issues: {
      items: issues,
      failedSections: sharedFailedSections,
    },
    pullRequests: {
      items: pullRequests,
      failedSections: pullRequestFailedSections,
    },
  };
}
