import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { getCachedRelayOrigin } from "@/shared/lib/mediaUrl";
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
  KIND_DELETION,
  KIND_GIT_ISSUE,
  KIND_GIT_PATCH,
  KIND_GIT_PR_UPDATE,
  KIND_GIT_PULL_REQUEST,
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
  KIND_REPO_ANNOUNCEMENT,
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
import { effectiveCloneUrls } from "./lib/projectCloneUrl";
import type { ProjectIssue } from "./projectIssues.mjs";
import { projectIssueEventsToIssues } from "./projectIssues.mjs";
import type {
  ProjectPullRequest,
  ProjectPullRequestCommentAnchor,
} from "./projectPullRequests.mjs";
import {
  normalizeProjectPullRequestCommentAnchor,
  PR_INLINE_COMMENT_LABEL,
  projectPullRequestEventsToPullRequests,
} from "./projectPullRequests.mjs";
import { fetchProjectsWorkItems } from "./projectWorkItems";

export type {
  ProjectIssue,
  ProjectPullRequest,
  ProjectPullRequestCommentAnchor,
};

const HIDDEN_PROJECT_CARDS_KEY = "buzz.projects.hidden-cards.v1";

export type Project = {
  id: string;
  dtag: string;
  name: string;
  description: string;
  cloneUrls: string[];
  webUrl: string | null;
  owner: string;
  contributors: string[];
  createdAt: number;
  projectChannelId: string | null;
  status: string;
  defaultBranch: string;
  repoAddress: string;
};

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
  pullRequest: ProjectPullRequest;
};

export type ProjectIssueListItem = {
  project: Project;
  issue: ProjectIssue;
};

function getTag(event: RelayEvent, name: string): string | undefined {
  const value = event.tags.find((t) => t[0] === name)?.[1];
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function getAllTags(event: RelayEvent, name: string): string[] {
  return event.tags
    .filter((t) => t[0] === name && typeof t[1] === "string" && t[1].length > 0)
    .map((t) => t[1]);
}

function getCloneUrls(event: RelayEvent): string[] {
  const tag = event.tags.find((t) => t[0] === "clone");
  return tag ? tag.slice(1) : [];
}

function projectCoordinate(project: Pick<Project, "owner" | "dtag">): string {
  return `${KIND_REPO_ANNOUNCEMENT}:${project.owner}:${project.dtag}`;
}

function readHiddenProjectCards(): string[] {
  if (typeof window === "undefined") {
    return [];
  }

  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(HIDDEN_PROJECT_CARDS_KEY) ?? "[]",
    );
    return Array.isArray(parsed)
      ? parsed.filter((item): item is string => typeof item === "string")
      : [];
  } catch {
    return [];
  }
}

function isHiddenLocally(project: Project): boolean {
  return readHiddenProjectCards().includes(projectCoordinate(project));
}

function isDeletedByA(project: Project, deletionEvents: RelayEvent[]): boolean {
  const coordinate = projectCoordinate(project);
  // NIP-09: a deletion is only valid when signed by the author of the
  // referenced event — otherwise anyone could hide someone else's project.
  return deletionEvents.some(
    (event) =>
      event.pubkey.toLowerCase() === project.owner.toLowerCase() &&
      event.tags.some((tag) => tag[0] === "a" && tag[1] === coordinate),
  );
}

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
): Project {
  const d = getTag(event, "d") ?? event.id;
  const name = getTag(event, "name") || d;
  const description = getTag(event, "description") || event.content || "";
  const cloneUrls = effectiveCloneUrls(
    getCloneUrls(event),
    relayOrigin,
    event.pubkey,
    d,
  );
  const webUrl = getTag(event, "web") ?? null;
  const setupUsers = getAllTags(event, "auth");
  const contributors = [...new Set([...getAllTags(event, "p"), ...setupUsers])];
  // `h`/`project-channel`, `status`, and `default-branch` are NOT part of
  // NIP-34 — they are read-side tolerance for extension tags no code writes
  // today (the write path that emitted them was removed). If a write path is
  // reintroduced it must go through the buzz-sdk repo-announcement builder;
  // the canonical NIP-34 source for the default branch is the kind:30618
  // state event's HEAD ref, not a 30617 tag.
  const projectChannelId =
    getTag(event, "h") ?? getTag(event, "project-channel") ?? null;

  return {
    id: `${event.pubkey}:${d}`,
    dtag: d,
    name,
    description,
    cloneUrls,
    webUrl,
    owner: event.pubkey,
    contributors,
    createdAt: event.created_at,
    projectChannelId,
    status: getTag(event, "status") ?? "active",
    defaultBranch: getTag(event, "default-branch") ?? "main",
    repoAddress: projectCoordinate({ owner: event.pubkey, dtag: d }),
  };
}

function dedup(events: RelayEvent[]): RelayEvent[] {
  const best = new Map<string, RelayEvent>();

  for (const e of events) {
    const d = getTag(e, "d") ?? "";
    const key = `${e.pubkey}:${e.kind}:${d}`;
    const prev = best.get(key);

    if (!prev || e.created_at > prev.created_at) {
      best.set(key, e);
    }
  }

  return [...best.values()];
}

export async function fetchProjects(): Promise<Project[]> {
  const [events, deletionEvents] = await Promise.all([
    relayClient.fetchEvents({
      kinds: [KIND_REPO_ANNOUNCEMENT],
      limit: 200,
    }),
    relayClient.fetchEvents({
      kinds: [KIND_DELETION],
      limit: 500,
    }),
  ]);

  return dedup(events)
    .map((event) => eventToProject(event, getCachedRelayOrigin()))
    .filter(
      (project) =>
        !isHiddenLocally(project) && !isDeletedByA(project, deletionEvents),
    )
    .sort((a, b) => b.createdAt - a.createdAt);
}

/**
 * Splits a project route ID into its owner pubkey and dtag. The canonical
 * form is `<owner-pubkey>:<dtag>` (matching `Project.id`) — NIP-34 repo
 * identity is the full `30617:<owner>:<dtag>` coordinate, and two owners can
 * both publish the same dtag (forks). Bare-dtag IDs from legacy links are
 * still resolved, ambiguously, to whichever owner the relay returns first.
 */
function parseProjectRouteId(projectId: string): {
  owner: string | null;
  dtag: string;
} {
  const owner = projectId.slice(0, 64);
  if (projectId[64] === ":" && /^[0-9a-fA-F]{64}$/.test(owner)) {
    return { owner: owner.toLowerCase(), dtag: projectId.slice(65) };
  }
  return { owner: null, dtag: projectId };
}

async function fetchProject(projectId: string): Promise<Project | null> {
  const { owner, dtag } = parseProjectRouteId(projectId);
  const events = await relayClient.fetchEvents({
    kinds: [KIND_REPO_ANNOUNCEMENT],
    ...(owner ? { authors: [owner] } : {}),
    "#d": [dtag],
    limit: 10,
  });

  const deduped = dedup(events).filter(
    (event) => !owner || event.pubkey.toLowerCase() === owner,
  );
  const project =
    deduped.length > 0
      ? eventToProject(deduped[0], getCachedRelayOrigin())
      : null;
  if (!project) {
    return null;
  }

  const deletionEvents = await relayClient.fetchEvents({
    kinds: [KIND_DELETION],
    authors: [project.owner],
    "#a": [project.repoAddress],
    limit: 10,
  });

  if (isDeletedByA(project, deletionEvents)) return null;
  const repoState = await fetchRepoState(project);
  return repoState?.head
    ? { ...project, defaultBranch: repoState.head }
    : project;
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

async function fetchRepoState(project: Project): Promise<RepoState | null> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_REPO_STATE],
    authors: [project.owner],
    "#d": [project.dtag],
    limit: 1,
  });

  return events.length > 0 ? eventToRepoState(events[0]) : null;
}

async function fetchProjectIssues(project: Project): Promise<ProjectIssue[]> {
  const [issueEvents, statusEvents, commentEvents] = await Promise.all([
    relayClient.fetchEvents({
      kinds: [KIND_GIT_ISSUE],
      "#a": [project.repoAddress],
      limit: 200,
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
    relayClient.fetchEvents({
      kinds: [KIND_TEXT_NOTE],
      "#a": [project.repoAddress],
      limit: 500,
    }),
  ]);

  return projectIssueEventsToIssues(issueEvents, statusEvents, commentEvents);
}

async function fetchProjectPullRequests(
  project: Project,
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
  mediaTags,
  mentionPubkeys = [],
  project,
  pullRequest,
}: {
  anchor?: ProjectPullRequestCommentAnchor;
  content: string;
  mediaTags?: string[][];
  mentionPubkeys?: string[];
  project: Project;
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
  if (normalizedAnchor && !pullRequest.commit) {
    throw new Error("Pull request commit is required for inline comments.");
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
    ...(mediaTags ?? []),
  ];

  const event = await signRelayEvent({
    kind: KIND_TEXT_NOTE,
    content: body,
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out posting pull request comment.",
    "Failed to post pull request comment.",
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
  project: Project;
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

  const event = await signRelayEvent({
    kind: KIND_TEXT_NOTE,
    content: body,
    tags,
  });

  await relayClient.publishEvent(
    event,
    "Timed out posting issue comment.",
    "Failed to post issue comment.",
  );
}

async function fetchProjectRepoSnapshot(
  project: Project,
  branchName?: string | null,
  pullRequest?: ProjectPullRequest | null,
): Promise<ProjectRepoSnapshot | null> {
  const cloneUrl = pullRequest?.cloneUrls[0] ?? project.cloneUrls[0];
  if (!cloneUrl) return null;

  return getProjectRepoSnapshot({
    cloneUrl,
    defaultBranch: branchName ?? project.defaultBranch,
    baseBranch: project.defaultBranch,
    targetCommit: pullRequest?.commit ?? null,
    targetRef: pullRequest ? `refs/nostr/${pullRequest.id}` : null,
  });
}

async function fetchProjectRepoDiff(
  project: Project,
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
  project: Project,
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
  project: Project,
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

async function fetchProjectActivitySummaries(
  projects: Project[],
): Promise<Record<string, ProjectActivitySummary>> {
  if (projects.length === 0) return {};

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
    "#a": projects.map((project) => project.repoAddress),
    limit: 1_000,
  });

  return summarizeProjectActivityEvents(events, projects) as Record<
    string,
    ProjectActivitySummary
  >;
}

async function deleteProject(project: Project): Promise<void> {
  const identity = await getIdentity();
  if (identity.pubkey.toLowerCase() !== project.owner.toLowerCase()) {
    throw new Error("Only branch owners can delete branches.");
  }

  const event = await signRelayEvent({
    kind: KIND_DELETION,
    content: `Delete project ${project.name}`,
    tags: [["a", project.repoAddress]],
  });

  await relayClient.publishEvent(
    event,
    "Timed out deleting project.",
    "Failed to delete project.",
  );
}

export const projectsQueryKey = ["projects"] as const;

export function useProjectsQuery() {
  return useQuery({
    queryKey: projectsQueryKey,
    queryFn: fetchProjects,
    staleTime: 60_000,
  });
}

export function useProjectQuery(projectId: string) {
  return useQuery({
    queryKey: ["project", projectId],
    queryFn: () => fetchProject(projectId),
    staleTime: 60_000,
  });
}

export function useRepoStateQuery(project: Project | null | undefined) {
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
  project: Project | null | undefined,
  branchName?: string | null,
  pullRequest?: ProjectPullRequest | null,
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(project?.cloneUrls[0]),
    queryKey: [
      "project",
      project?.id ?? "none",
      "repo-snapshot",
      selectedBranch ?? "default",
      pullRequest?.id ?? "none",
      pullRequest?.commit ?? "none",
    ],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchProjectRepoSnapshot(project, selectedBranch, pullRequest);
    },
    staleTime: 30_000,
    retry: 1,
  });
}

export function useProjectRepoDiffQuery(
  project: Project | null | undefined,
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
  project: Project | null | undefined,
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
  project: Project | null | undefined,
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
    staleTime: 10_000,
    retry: 1,
  });
}

export function useProjectIssuesQuery(project: Project | null | undefined) {
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
  project: Project | null | undefined,
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
    queryKey: ["projects", "work-items", projects.map((project) => project.id)],
    queryFn: () => fetchProjectsWorkItems(projects),
    staleTime: 30_000,
  });
}

export function useCreateProjectIssueCommentMutation(
  project: Project | null | undefined,
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
  project: Project | null | undefined,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      anchor,
      content,
      mediaTags,
      mentionPubkeys,
      pullRequest,
    }: {
      anchor?: ProjectPullRequestCommentAnchor;
      content: string;
      mediaTags?: string[][];
      mentionPubkeys?: string[];
      pullRequest: ProjectPullRequest;
    }) => {
      if (!project) throw new Error("No project selected.");
      return createProjectPullRequestComment({
        anchor,
        content,
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
    () => projects.map((project) => project.repoAddress).sort(),
    [projects],
  );

  return useQuery({
    enabled: repoAddresses.length > 0,
    queryKey: ["projects", "activity-summaries", repoAddresses],
    queryFn: () => fetchProjectActivitySummaries(projects),
    staleTime: 30_000,
  });
}

export function useDeleteProjectMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: deleteProject,
    onSuccess: (_data, project) => {
      queryClient.setQueryData<Project[]>(projectsQueryKey, (current = []) =>
        current.filter((item) => item.id !== project.id),
      );
      queryClient.setQueryData(["project", project.id], null);
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
      void queryClient.invalidateQueries({
        queryKey: ["project", project.id],
      });
    },
  });
}
