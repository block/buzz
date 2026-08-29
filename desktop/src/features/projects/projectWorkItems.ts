import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import {
  fetchAssignmentOperationEvents,
  mergeEventsById,
} from "./assignmentOperationFetch";
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

type RepositoryReference = {
  repoAddress: string;
};

type ProjectReference = {
  repositories: RepositoryReference[];
};

type ProjectRepository<TProject extends ProjectReference> =
  TProject["repositories"][number];

/** Optional event groups that can fail without discarding root work items. */
export type ProjectWorkItemSection =
  | "assignments"
  | "comments"
  | "pull-request-updates"
  | "statuses";

/** Aggregate work items plus any optional event groups that failed to load. */
export type ProjectsWorkItemsResult<TProject extends ProjectReference> = {
  issues: {
    items: Array<{
      project: TProject;
      repository: ProjectRepository<TProject>;
      issue: ProjectIssue;
    }>;
    failedSections: ProjectWorkItemSection[];
  };
  pullRequests: {
    items: Array<{
      project: TProject;
      repository: ProjectRepository<TProject>;
      pullRequest: ProjectPullRequest;
    }>;
    failedSections: ProjectWorkItemSection[];
  };
};

/** Includes every repository-bearing read model, including repository-only ones. */
export function projectsWithWorkItemRepositories<
  TProject extends ProjectReference,
>(projects: readonly TProject[]): TProject[] {
  return projects.filter((project) => project.repositories.length > 0);
}

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

type FetchEventsInput = Parameters<(typeof relayClient)["fetchEvents"]>[0];

/** Loads aggregate issue and pull-request data with bounded relay fan-out. */
export async function fetchProjectsWorkItems<TProject extends ProjectReference>(
  projects: TProject[],
  fetchEvents: (
    filter: FetchEventsInput,
  ) => Promise<RelayEvent[]> = relayClient.fetchEvents.bind(relayClient),
  signal?: AbortSignal,
): Promise<ProjectsWorkItemsResult<TProject>> {
  const repoAddresses = [
    ...new Set(
      projects.flatMap((project) =>
        project.repositories.map((repository) => repository.repoAddress),
      ),
    ),
  ];
  const rootPromise = fetchEvents({
    kinds: [KIND_GIT_ISSUE, KIND_GIT_PULL_REQUEST],
    "#a": repoAddresses,
    limit: 2_000,
  });
  const [rootResult, updateResult, commentResult, statusResult, assignResult] =
    await Promise.allSettled([
      rootPromise,
      fetchEvents({
        kinds: [KIND_GIT_PR_UPDATE],
        "#a": repoAddresses,
        limit: 2_000,
      }),
      fetchEvents({
        kinds: [KIND_TEXT_NOTE],
        "#a": repoAddresses,
        limit: 2_000,
      }),
      fetchEvents({
        kinds: [
          KIND_GIT_STATUS_OPEN,
          KIND_GIT_STATUS_MERGED,
          KIND_GIT_STATUS_CLOSED,
          KIND_GIT_STATUS_DRAFT,
        ],
        "#a": repoAddresses,
        limit: 2_000,
      }),
      // Assignment state must reduce over the complete operation history —
      // the 2,000-comment window above is shared across every loaded repo
      // and can evict older assignment operations. Keyed by issue id (`#e`)
      // because that is the only tag constraint the relay applies before its
      // SQL LIMIT; see fetchAssignmentOperationEvents.
      rootPromise.then((rootEvents) =>
        fetchAssignmentOperationEvents(
          rootEvents
            .filter((event) => event.kind === KIND_GIT_ISSUE)
            .map((event) => event.id),
          fetchEvents,
          signal,
        ),
      ),
    ]);

  // The five eager queries above are single bounded REQs the relay client
  // cannot abort mid-flight; only the assignment pagination is abort-aware.
  // What cancellation CAN save here is the reduce work below and caching a
  // result for a surface the user already left.
  signal?.throwIfAborted();

  if (rootResult.status === "rejected") {
    throw rootResult.reason instanceof Error
      ? rootResult.reason
      : new Error("Could not load project tasks and reviews.");
  }

  const updateEvents =
    updateResult.status === "fulfilled" ? updateResult.value : [];
  const commentEvents = mergeEventsById(
    commentResult.status === "fulfilled" ? commentResult.value : [],
    assignResult.status === "fulfilled" ? assignResult.value : [],
  );
  const statusEvents =
    statusResult.status === "fulfilled" ? statusResult.value : [];
  const rootsByRepo = groupByRepoAddress(rootResult.value);
  const updatesByRepo = groupByRepoAddress(updateEvents);
  const commentsByRepo = groupByRepoAddress(commentEvents);
  const statusesByRepo = groupByRepoAddress(statusEvents);

  const pullRequests = projects
    .flatMap((project) =>
      project.repositories.flatMap((repository) =>
        projectPullRequestEventsToPullRequests(
          (rootsByRepo.get(repository.repoAddress) ?? []).filter(
            (event) => event.kind === KIND_GIT_PULL_REQUEST,
          ),
          updatesByRepo.get(repository.repoAddress) ?? [],
          commentsByRepo.get(repository.repoAddress) ?? [],
          statusesByRepo.get(repository.repoAddress) ?? [],
        ).map((pullRequest) => ({ project, pullRequest, repository })),
      ),
    )
    // Deduplicate by (repoAddress, pull-request id): a repository in N projects
    // must produce exactly one aggregate row per pull request (NIP-MP §Multiple
    // membership). First occurrence wins; that project's navigation context is
    // kept for the row.
    .filter(
      (() => {
        const seen = new Set<string>();
        return (item: {
          repository: RepositoryReference;
          pullRequest: ProjectPullRequest;
        }) => {
          const key = `${item.repository.repoAddress}:${item.pullRequest.id}`;
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        };
      })(),
    )
    .sort(
      (left, right) => right.pullRequest.updatedAt - left.pullRequest.updatedAt,
    );
  const issues = projects
    .flatMap((project) =>
      project.repositories.flatMap((repository) =>
        projectIssueEventsToIssues(
          (rootsByRepo.get(repository.repoAddress) ?? []).filter(
            (event) => event.kind === KIND_GIT_ISSUE,
          ),
          statusesByRepo.get(repository.repoAddress) ?? [],
          commentsByRepo.get(repository.repoAddress) ?? [],
        ).map((issue) => ({ issue, project, repository })),
      ),
    )
    // Deduplicate by (repoAddress, issue id).
    .filter(
      (() => {
        const seen = new Set<string>();
        return (item: {
          repository: RepositoryReference;
          issue: ProjectIssue;
        }) => {
          const key = `${item.repository.repoAddress}:${item.issue.id}`;
          if (seen.has(key)) return false;
          seen.add(key);
          return true;
        };
      })(),
    )
    .sort((left, right) => right.issue.updatedAt - left.issue.updatedAt);
  const sharedFailedSections: ProjectWorkItemSection[] = [];
  if (assignResult.status === "rejected") {
    sharedFailedSections.push("assignments");
  }
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
