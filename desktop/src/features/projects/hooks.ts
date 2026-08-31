import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { getRelaySelf } from "@/features/moderation/lib/relaySelf";
import { signRelayEvent } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import {
  getProjectLocalRepoDiff,
  getProjectRepoDiff,
  getProjectLocalRepoSnapshot,
  getProjectRepoSnapshot,
  listProjectLocalRepositories,
} from "@/shared/api/projectGit";
import {
  KIND_GIT_ISSUE,
  KIND_GIT_PATCH,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_REPO_STATE,
  KIND_TEXT_NOTE,
} from "@/shared/constants/kinds";
import type {
  ProjectLocalRepository,
  ProjectLocalRepoSnapshot,
  ProjectRepoDiff,
  ProjectRepoPushResult,
  ProjectRepoContributor,
  ProjectRepoFile,
  ProjectRepoSnapshot,
  ProjectRepoSyncStatus,
  RelayEvent,
} from "@/shared/api/types";
import { summarizeProjectActivityEvents } from "./projectActivity.mjs";
import {
  fetchAssignmentOperationEvents,
  mergeEventsById,
} from "./assignmentOperationFetch";
import type { ProjectIssue } from "./projectIssues.mjs";
import {
  nextProjectIssueCommentCreatedAt,
  projectIssueEventsToIssues,
} from "./projectIssues.mjs";
import type {
  ProjectPullRequest,
  ProjectPullRequestCommentAnchor,
} from "./projectPullRequests.mjs";
import {
  nextProjectPullRequestReviewCreatedAt,
  normalizeProjectPullRequestCommentAnchor,
  PR_CHANGES_REQUESTED_LABEL,
  PR_INLINE_COMMENT_LABEL,
  projectPullRequestEventsToPullRequests,
} from "./projectPullRequests.mjs";
import { fetchProjectsWorkItems } from "./projectWorkItems";
import {
  projectDeletionMutationOptions,
  projectsQueryKey,
} from "./projectDeletionMutation";
import {
  eventToRepository,
  type Project,
  type Repository,
} from "./projectModels";
import { fetchProjectHomeForChannel, fetchProjects } from "./projectFetch";
import {
  markProjectCollectionAuthoritative,
  persistProjectSnapshot,
  PROJECT_QUERY_STRUCTURAL_SHARING,
} from "./projectSnapshot";
import { projectMatchesRouteId } from "./projectRoutes";

export { fetchProjects } from "./projectFetch";
export { projectsQueryKey };

export type {
  Project,
  ProjectIssue,
  ProjectPullRequest,
  ProjectPullRequestCommentAnchor,
  Repository,
};
export type ProjectPullRequestCommentDecision = "request-changes";

export type RepoState = {
  branches: Array<{ name: string; commit: string }>;
  tags: Array<{ name: string; commit: string }>;
  head: string | null;
  updatedAt: number;
};

export type ProjectActivitySummary = {
  repoAddress: string;
  issueCount: number;
  prCount: number;
  commitCount: number;
  activityCount: number;
  updatedAt: number;
  participantPubkeys: string[];
  latestCommit: {
    author: string | null;
    commit: string;
    createdAt: number;
    title: string;
  } | null;
  /** Activity event counts bucketed by local-time day key ("YYYY-MM-DD"). */
  activityByDay: Record<string, number>;
};

export type {
  ProjectLocalRepository,
  ProjectLocalRepoSnapshot,
  ProjectRepoDiff,
  ProjectRepoPushResult,
  ProjectRepoContributor,
  ProjectRepoFile,
  ProjectRepoSnapshot,
  ProjectRepoSyncStatus,
};

export type ProjectPullRequestListItem = {
  project: Project;
  repository: Repository;
  pullRequest: ProjectPullRequest;
};

export type ProjectIssueListItem = {
  project: Project;
  repository: Repository;
  issue: ProjectIssue;
};

/**
 * Converts a kind:30617 repo announcement into a `Project`.
 *
 * `relayOrigin` is the resolved relay HTTP origin (from `getCachedRelayOrigin`)
 * used to synthesize a canonical clone URL when the announcement omits an
 * explicit `clone` tag. Callers outside the relay-connected app (e.g. unit
 * tests) may omit it, in which case no default is derived.
 */
export function eventToProject(
  event: RelayEvent,
  relayOrigin?: string | null,
): Repository {
  const repository = eventToRepository(event, relayOrigin);
  if (!repository) {
    throw new Error("Invalid repository announcement.");
  }
  return repository;
}

function eventToRepoState(event: RelayEvent): RepoState {
  const branches: RepoState["branches"] = [];
  const tags: RepoState["tags"] = [];
  let head: string | null = null;

  for (const tag of event.tags) {
    const [name, value] = tag;
    if (!name || !value) continue;

    if (name.startsWith("refs/heads/")) {
      branches.push({ name: name.slice("refs/heads/".length), commit: value });
    } else if (name.startsWith("refs/tags/")) {
      tags.push({ name: name.slice("refs/tags/".length), commit: value });
    } else if (name === "HEAD") {
      head = value.replace(/^ref:\s*/, "").replace(/^refs\/heads\//, "");
    }
  }

  return {
    branches,
    tags,
    head,
    updatedAt: event.created_at,
  };
}
/** Load the trusted relay state used to resolve a repository's live refs. */
export async function fetchRepoState(
  project: Repository,
): Promise<RepoState | null> {
  const relaySelf = await getRelaySelf();
  const trustedAuthors = [
    ...new Set(
      [project.owner, relaySelf].filter((value): value is string =>
        Boolean(value),
      ),
    ),
  ];
  const events = await relayClient.fetchEvents({
    kinds: [KIND_REPO_STATE],
    authors: trustedAuthors,
    "#d": [project.dtag],
    limit: 1,
  });

  return events.length > 0 ? eventToRepoState(events[0]) : null;
}

async function fetchProjectIssues(
  project: Repository,
): Promise<ProjectIssue[]> {
  const issuePromise = relayClient.fetchEvents({
    kinds: [KIND_GIT_ISSUE],
    "#a": [project.repoAddress],
    limit: 200,
  });
  const [issueEvents, statusEvents, commentEvents, assignmentEvents] =
    await Promise.all([
      issuePromise,
      relayClient.fetchEvents({
        kinds: [
          KIND_GIT_STATUS_OPEN,
          KIND_GIT_STATUS_MERGED,
          KIND_GIT_STATUS_CLOSED,
          KIND_GIT_STATUS_DRAFT,
        ],
        "#a": [project.repoAddress],
        limit: 500,
      }),
      relayClient.fetchEvents({
        kinds: [KIND_TEXT_NOTE],
        "#a": [project.repoAddress],
        limit: 500,
      }),
      // Assignment state must reduce over the complete operation history, not
      // whatever survives the bounded comment window above. Keyed by issue id
      // (`#e`) because that is the only tag constraint the relay applies
      // before its SQL LIMIT — see fetchAssignmentOperationEvents.
      issuePromise.then((events) =>
        fetchAssignmentOperationEvents(events.map((event) => event.id)),
      ),
    ]);

  return projectIssueEventsToIssues(
    issueEvents,
    statusEvents,
    mergeEventsById(commentEvents, assignmentEvents),
  );
}

async function fetchProjectPullRequests(
  project: Repository,
): Promise<ProjectPullRequest[]> {
  const [pullRequestEvents, updateEvents, commentEvents, statusEvents] =
    await Promise.all([
      relayClient.fetchEvents({
        kinds: [KIND_GIT_PULL_REQUEST],
        "#a": [project.repoAddress],
        limit: 200,
      }),
      relayClient.fetchEvents({
        kinds: [KIND_GIT_PR_UPDATE],
        "#a": [project.repoAddress],
        limit: 500,
      }),
      relayClient.fetchEvents({
        kinds: [KIND_TEXT_NOTE],
        "#a": [project.repoAddress],
        limit: 500,
      }),
      relayClient.fetchEvents({
        kinds: [
          KIND_GIT_STATUS_OPEN,
          KIND_GIT_STATUS_MERGED,
          KIND_GIT_STATUS_CLOSED,
          KIND_GIT_STATUS_DRAFT,
        ],
        "#a": [project.repoAddress],
        limit: 500,
      }),
    ]);

  return projectPullRequestEventsToPullRequests(
    pullRequestEvents,
    updateEvents,
    commentEvents,
    statusEvents,
  );
}

// Issue/PR comments are published as kind:1 text notes because the relay
// does not register NIP-22 kind 1111 (current NIP-34 reply convention).
// Pulse feeds filter these out via the repo-address `a` tag (see
// features/pulse/lib/projectComments.ts). If the relay ever allowlists
// 1111, migrate these to NIP-22 comments and drop that filter.
async function createProjectPullRequestComment({
  anchor,
  content,
  decision,
  mediaTags,
  mentionPubkeys = [],
  project,
  pullRequest,
}: {
  anchor?: ProjectPullRequestCommentAnchor;
  content: string;
  decision?: ProjectPullRequestCommentDecision;
  mediaTags?: string[][];
  mentionPubkeys?: string[];
  project: Repository;
  pullRequest: ProjectPullRequest;
}): Promise<void> {
  const body = content.trim();
  if (!body) {
    throw new Error("Comment cannot be empty.");
  }
  const normalizedAnchor = anchor
    ? normalizeProjectPullRequestCommentAnchor(anchor)
    : null;
  if (anchor && !normalizedAnchor) {
    throw new Error("Comment location is invalid.");
  }
  if ((normalizedAnchor || decision) && !pullRequest.commit) {
    throw new Error("A review commit is required for review comments.");
  }

  const recipients = new Set([
    project.owner.toLowerCase(),
    pullRequest.author.toLowerCase(),
    ...pullRequest.recipients.map((recipient) => recipient.toLowerCase()),
    ...mentionPubkeys.map((pubkey) => pubkey.toLowerCase()),
  ]);
  const tags = [
    ["e", pullRequest.id, "", "root"],
    ["a", project.repoAddress],
    ...[...recipients].map((recipient) => ["p", recipient]),
    ...(normalizedAnchor
      ? [
          ["t", PR_INLINE_COMMENT_LABEL],
          ["c", pullRequest.commit as string],
          ["file", normalizedAnchor.path],
          ["side", normalizedAnchor.side],
          ["line", String(normalizedAnchor.line)],
        ]
      : []),
    ...(decision
      ? [
          ["t", PR_CHANGES_REQUESTED_LABEL],
          ...(!normalizedAnchor ? [["c", pullRequest.commit as string]] : []),
        ]
      : []),
    ...(mediaTags ?? []),
  ];

  const event = await signRelayEvent({
    kind: KIND_TEXT_NOTE,
    content: body,
    ...(decision
      ? {
          createdAt: nextProjectPullRequestReviewCreatedAt(
            pullRequest,
            Math.floor(Date.now() / 1_000),
          ),
        }
      : {}),
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out posting review comment.",
    "Failed to post review comment.",
  );
}

async function createProjectIssueComment({
  content,
  mediaTags,
  mentionPubkeys = [],
  issue,
  project,
}: {
  content: string;
  mediaTags?: string[][];
  mentionPubkeys?: string[];
  issue: ProjectIssue;
  project: Repository;
}): Promise<void> {
  const body = content.trim();
  if (!body) {
    throw new Error("Comment cannot be empty.");
  }

  const recipients = new Set([
    project.owner.toLowerCase(),
    issue.author.toLowerCase(),
    ...issue.recipients.map((recipient) => recipient.toLowerCase()),
    ...mentionPubkeys.map((pubkey) => pubkey.toLowerCase()),
  ]);
  const tags = [
    ["e", issue.id, "", "root"],
    ["a", project.repoAddress],
    ...[...recipients].map((recipient) => ["p", recipient]),
    ...(mediaTags ?? []),
  ];
  const identity = await getIdentity();

  const event = await signRelayEvent({
    kind: KIND_TEXT_NOTE,
    content: body,
    createdAt: nextProjectIssueCommentCreatedAt(
      issue,
      Math.floor(Date.now() / 1_000),
      identity.pubkey,
    ),
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out posting task comment.",
    "Failed to post task comment.",
  );
}

async function fetchProjectRepoSnapshot(
  project: Repository,
  branchName?: string | null,
  pullRequest?: ProjectPullRequest | null,
  tag?: { name: string; commit: string } | null,
): Promise<ProjectRepoSnapshot | null> {
  const cloneUrl = pullRequest?.cloneUrls[0] ?? project.cloneUrls[0];
  if (!cloneUrl) return null;

  return getProjectRepoSnapshot({
    cloneUrl,
    defaultBranch: branchName ?? project.defaultBranch,
    baseBranch: project.defaultBranch,
    targetCommit: tag?.commit ?? pullRequest?.commit ?? null,
    targetRef: tag
      ? `refs/tags/${tag.name}`
      : pullRequest
        ? `refs/nostr/${pullRequest.id}`
        : null,
  });
}

async function fetchProjectRepoDiff(
  project: Repository,
  branchName?: string | null,
  pullRequest?: ProjectPullRequest | null,
): Promise<ProjectRepoDiff | null> {
  const cloneUrl = pullRequest?.cloneUrls[0] ?? project.cloneUrls[0];
  if (!cloneUrl) return null;

  return getProjectRepoDiff({
    cloneUrl,
    defaultBranch: branchName ?? project.defaultBranch,
    baseBranch: project.defaultBranch,
    targetCommit: pullRequest?.commit ?? null,
    targetRef: pullRequest ? `refs/nostr/${pullRequest.id}` : null,
  });
}

async function fetchProjectLocalRepoDiff(
  project: Repository,
  reposDir?: string | null,
  branchName?: string | null,
  pullRequest?: ProjectPullRequest | null,
): Promise<ProjectRepoDiff | null> {
  return getProjectLocalRepoDiff({
    reposDir,
    projectDtag: project.dtag,
    cloneUrl: project.cloneUrls[0] ?? null,
    defaultBranch: branchName ?? project.defaultBranch,
    baseBranch: project.defaultBranch,
    baseCommit:
      pullRequest?.initialCommit &&
      pullRequest.initialCommit !== pullRequest.commit
        ? pullRequest.initialCommit
        : null,
    targetCommit: pullRequest?.commit ?? null,
  });
}

async function fetchProjectLocalRepoSnapshot(
  project: Repository,
  reposDir?: string | null,
  branchName?: string | null,
): Promise<ProjectLocalRepoSnapshot | null> {
  return getProjectLocalRepoSnapshot({
    reposDir,
    projectDtag: project.dtag,
    cloneUrl: project.cloneUrls[0] ?? null,
    defaultBranch: branchName ?? project.defaultBranch,
    baseBranch: project.defaultBranch,
  });
}

/** Loads commit, pull-request, and issue activity keyed by repository address. */
export async function fetchRepositoryActivitySummaries(
  repositories: Repository[],
): Promise<Record<string, ProjectActivitySummary>> {
  if (repositories.length === 0) return {};
  const events = await relayClient.fetchEvents({
    kinds: [
      KIND_GIT_ISSUE,
      KIND_GIT_STATUS_OPEN,
      KIND_GIT_STATUS_MERGED,
      KIND_GIT_STATUS_CLOSED,
      KIND_GIT_STATUS_DRAFT,
      KIND_GIT_PATCH,
      KIND_GIT_PULL_REQUEST,
      KIND_GIT_PR_UPDATE,
    ],
    "#a": repositories.map((repository) => repository.repoAddress),
    limit: 1_000,
  });

  return summarizeProjectActivityEvents(events, repositories) as Record<
    string,
    ProjectActivitySummary
  >;
}

async function fetchProjectActivitySummaries(
  projects: Project[],
): Promise<Record<string, ProjectActivitySummary>> {
  if (projects.length === 0) return {};

  const repositories = [
    ...new Map(
      projects
        .flatMap((project) => project.repositories)
        .map((repository) => [repository.repoAddress, repository]),
    ).values(),
  ];
  const summariesByRepository =
    await fetchRepositoryActivitySummaries(repositories);
  return Object.fromEntries(
    projects.map((project) => {
      const summaries = project.repositories.map(
        (repository) => summariesByRepository[repository.repoAddress],
      );
      const latestCommit =
        summaries
          .map((summary) => summary?.latestCommit)
          .filter(
            (
              commit,
            ): commit is NonNullable<ProjectActivitySummary["latestCommit"]> =>
              Boolean(commit),
          )
          .sort((left, right) => right.createdAt - left.createdAt)[0] ?? null;
      const activityByDay: Record<string, number> = {};
      for (const summary of summaries) {
        for (const [day, count] of Object.entries(
          summary?.activityByDay ?? {},
        )) {
          activityByDay[day] = (activityByDay[day] ?? 0) + count;
        }
      }
      return [
        project.id,
        {
          repoAddress: project.projectAddress,
          issueCount: summaries.reduce(
            (count, summary) => count + (summary?.issueCount ?? 0),
            0,
          ),
          prCount: summaries.reduce(
            (count, summary) => count + (summary?.prCount ?? 0),
            0,
          ),
          commitCount: summaries.reduce(
            (count, summary) => count + (summary?.commitCount ?? 0),
            0,
          ),
          activityCount: summaries.reduce(
            (count, summary) => count + (summary?.activityCount ?? 0),
            0,
          ),
          updatedAt: Math.max(
            0,
            ...summaries.map((summary) => summary?.updatedAt ?? 0),
          ),
          participantPubkeys: [
            ...new Set(
              summaries.flatMap((summary) => summary?.participantPubkeys ?? []),
            ),
          ],
          latestCommit,
          activityByDay,
        } satisfies ProjectActivitySummary,
      ];
    }),
  );
}

/**
 * Freshness windows for the Projects surface. Every local write path
 * invalidates its keys explicitly (issue/PR mutations, project creation,
 * repo sync), so short windows bought nothing for own-actions and charged a
 * full relay fan-out — project enumeration is an exhaustive paginated scan,
 * work items are five 2,000-event queries — on nearly every Projects
 * re-entry. Remote actors' changes surface within the window. gcTime keeps
 * the enumeration cached across visits so re-entering Projects paints from
 * cache instead of blocking on the scan.
 */
export const PROJECTS_STALE_TIME_MS = 5 * 60_000;
// Window-guarded like react-query's own server default (Infinity): an
// explicit finite gcTime schedules a real, non-unref'd timeout per cache
// entry, which keeps node test processes alive for the full 30 minutes.
export const PROJECTS_GC_TIME_MS =
  typeof window === "undefined" ? Number.POSITIVE_INFINITY : 30 * 60_000;
export const PROJECT_WORK_ITEMS_STALE_TIME_MS = 2 * 60_000;
export const PROJECT_ACTIVITY_STALE_TIME_MS = 2 * 60_000;
export const PROJECT_LOCAL_REPOS_STALE_TIME_MS = 2 * 60_000;

export function useProjectsQuery(enabled = true) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: projectsQueryKey,
    queryFn: async ({ signal }) => {
      const projects = await fetchProjects(undefined, signal);
      markProjectCollectionAuthoritative(queryClient);
      persistProjectSnapshot(queryClient, projects);
      return projects;
    },
    staleTime: PROJECTS_STALE_TIME_MS,
    gcTime: PROJECTS_GC_TIME_MS,
    structuralSharing: PROJECT_QUERY_STRUCTURAL_SHARING,
    enabled,
  });
}

export function useProjectHomeForChannelQuery(
  channelId: string,
  enabled = true,
) {
  return useQuery({
    queryKey: ["projects", "home-channel", channelId],
    queryFn: ({ signal }) => fetchProjectHomeForChannel(channelId, signal),
    staleTime: PROJECTS_STALE_TIME_MS,
    gcTime: PROJECTS_GC_TIME_MS,
    enabled: enabled && channelId.length > 0,
  });
}

export function useProjectQuery(projectId: string) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: projectsQueryKey,
    queryFn: async ({ signal }) => {
      const projects = await fetchProjects(undefined, signal);
      markProjectCollectionAuthoritative(queryClient);
      persistProjectSnapshot(queryClient, projects);
      return projects;
    },
    select: (projects) =>
      projects.find((project) => projectMatchesRouteId(project, projectId)) ??
      null,
    staleTime: PROJECTS_STALE_TIME_MS,
    gcTime: PROJECTS_GC_TIME_MS,
    structuralSharing: PROJECT_QUERY_STRUCTURAL_SHARING,
  });
}

export function useRepoStateQuery(project: Repository | null | undefined) {
  return useQuery({
    enabled: Boolean(project),
    queryKey: ["project", project?.id ?? "none", "repo-state"],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchRepoState(project);
    },
    staleTime: 30_000,
  });
}

export function useProjectRepoSnapshotQuery(
  project: Repository | null | undefined,
  branchName?: string | null,
  pullRequest?: ProjectPullRequest | null,
  tag?: { name: string; commit: string } | null,
  enabled = true,
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(enabled && project?.cloneUrls[0]),
    queryKey: [
      "project",
      project?.id ?? "none",
      "repo-snapshot",
      selectedBranch ?? "default",
      pullRequest?.id ?? "none",
      pullRequest?.commit ?? "none",
      tag?.name ?? "no-tag",
      tag?.commit ?? "no-tag-commit",
    ],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchProjectRepoSnapshot(
        project,
        selectedBranch,
        pullRequest,
        tag,
      );
    },
    staleTime: 30_000,
    retry: 1,
  });
}

export function useProjectRepoDiffQuery(
  project: Repository | null | undefined,
  branchName?: string | null,
  pullRequest?: ProjectPullRequest | null,
  enabled = true,
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(enabled && project?.cloneUrls[0] && pullRequest),
    queryKey: [
      "project",
      project?.id ?? "none",
      "repo-diff",
      selectedBranch ?? "default",
      pullRequest?.id ?? "none",
      pullRequest?.commit ?? "none",
    ],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchProjectRepoDiff(project, selectedBranch, pullRequest);
    },
    staleTime: 30_000,
    retry: 1,
  });
}

export function useProjectLocalRepoDiffQuery(
  project: Repository | null | undefined,
  reposDir?: string | null,
  branchName?: string | null,
  pullRequest?: ProjectPullRequest | null,
  enabled = true,
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(enabled && project),
    queryKey: [
      "project",
      project?.id ?? "none",
      "local-repo-diff",
      reposDir ?? "default",
      selectedBranch ?? "default",
      pullRequest?.initialCommit ?? "none",
      pullRequest?.commit ?? "none",
    ],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchProjectLocalRepoDiff(
        project,
        reposDir,
        selectedBranch,
        pullRequest,
      );
    },
    staleTime: 30_000,
    retry: 1,
  });
}

export function useProjectLocalRepoSnapshotQuery(
  project: Repository | null | undefined,
  reposDir?: string | null,
  branchName?: string | null,
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(project),
    queryKey: [
      "project",
      project?.id ?? "none",
      "local-repo-snapshot",
      reposDir ?? "default",
      selectedBranch ?? "default",
    ],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchProjectLocalRepoSnapshot(project, reposDir, selectedBranch);
    },
    staleTime: 10_000,
    retry: 1,
  });
}

export function useProjectLocalRepositoriesQuery(reposDir?: string | null) {
  return useQuery({
    queryKey: ["projects", "local-repositories", reposDir ?? "default"],
    queryFn: () => listProjectLocalRepositories({ reposDir }),
    // Filesystem scan; repo sync/clone flows invalidate this key on change.
    staleTime: PROJECT_LOCAL_REPOS_STALE_TIME_MS,
    retry: 1,
  });
}

export function useProjectIssuesQuery(project: Repository | null | undefined) {
  return useQuery({
    enabled: Boolean(project),
    queryKey: ["project", project?.id ?? "none", "issues"],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchProjectIssues(project);
    },
    staleTime: 30_000,
  });
}

export function useProjectPullRequestsQuery(
  project: Repository | null | undefined,
) {
  return useQuery({
    enabled: Boolean(project),
    queryKey: ["project", project?.id ?? "none", "pull-requests"],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchProjectPullRequests(project);
    },
    staleTime: 30_000,
  });
}

/** Loads cross-project issues and pull requests with partial-failure metadata. */
export function useProjectsWorkItemsQuery(projects: Project[]) {
  return useQuery({
    enabled: projects.length > 0,
    queryKey: [
      "projects",
      "work-items",
      projects.map((project) => project.id),
      // Repo attach/detach changes the fan-out inputs without changing
      // project ids; keying on addresses too prevents a pre-attach result
      // from serving as fresh for the whole staleTime window.
      projects
        .flatMap((project) =>
          project.repositories.map((repository) => repository.repoAddress),
        )
        .sort(),
    ],
    queryFn: ({ signal }) =>
      fetchProjectsWorkItems(projects, undefined, signal),
    staleTime: PROJECT_WORK_ITEMS_STALE_TIME_MS,
  });
}

export function useCreateProjectIssueCommentMutation(
  project: Repository | null | undefined,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      content,
      mediaTags,
      mentionPubkeys,
      issue,
    }: {
      content: string;
      mediaTags?: string[][];
      mentionPubkeys?: string[];
      issue: ProjectIssue;
    }) => {
      if (!project) throw new Error("No project selected.");
      return createProjectIssueComment({
        content,
        mediaTags,
        mentionPubkeys,
        issue,
        project,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["project", project?.id ?? "none", "issues"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["projects", "work-items"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["projects", "activity-summaries"],
      });
    },
  });
}

export function useCreateProjectPullRequestCommentMutation(
  project: Repository | null | undefined,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      anchor,
      content,
      decision,
      mediaTags,
      mentionPubkeys,
      pullRequest,
    }: {
      anchor?: ProjectPullRequestCommentAnchor;
      content: string;
      decision?: ProjectPullRequestCommentDecision;
      mediaTags?: string[][];
      mentionPubkeys?: string[];
      pullRequest: ProjectPullRequest;
    }) => {
      if (!project) throw new Error("No project selected.");
      return createProjectPullRequestComment({
        anchor,
        content,
        decision,
        mediaTags,
        mentionPubkeys,
        project,
        pullRequest,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["project", project?.id ?? "none", "pull-requests"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["projects", "work-items"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["projects", "activity-summaries"],
      });
    },
  });
}

export function useProjectActivitySummariesQuery(projects: Project[]) {
  const repoAddresses = React.useMemo(
    () =>
      projects
        .flatMap((project) =>
          project.repositories.map((repository) => repository.repoAddress),
        )
        .sort(),
    [projects],
  );

  return useQuery({
    enabled: repoAddresses.length > 0,
    queryKey: ["projects", "activity-summaries", repoAddresses],
    queryFn: () => fetchProjectActivitySummaries(projects),
    staleTime: PROJECT_ACTIVITY_STALE_TIME_MS,
  });
}

export function useDeleteProjectMutation() {
  const queryClient = useQueryClient();

  return useMutation(projectDeletionMutationOptions(queryClient));
}
