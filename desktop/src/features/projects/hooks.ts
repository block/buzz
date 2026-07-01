import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { getIdentity, signRelayEvent } from "@/shared/api/tauri";
import {
  getProjectRepoSyncStatus,
  getProjectLocalRepoSnapshot,
  getProjectRepoSnapshot,
  listProjectLocalRepositories,
  pushProjectLocalRepository,
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
  ProjectRepoPushResult,
  ProjectRepoContributor,
  ProjectRepoFile,
  ProjectRepoSnapshot,
  ProjectRepoSyncStatus,
  RelayEvent,
} from "@/shared/api/types";
import { summarizeProjectActivityEvents } from "./projectActivity.mjs";
import type { ProjectIssue } from "./projectIssues.mjs";
import { projectIssueEventsToIssues } from "./projectIssues.mjs";
import type { ProjectPullRequest } from "./projectPullRequests.mjs";
import { projectPullRequestEventsToPullRequests } from "./projectPullRequests.mjs";

export type { ProjectPullRequest };

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
  activityCount: number;
  updatedAt: number;
  participantPubkeys: string[];
};

export type {
  ProjectLocalRepository,
  ProjectLocalRepoSnapshot,
  ProjectRepoPushResult,
  ProjectRepoContributor,
  ProjectRepoFile,
  ProjectRepoSnapshot,
  ProjectRepoSyncStatus,
};

function getTag(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((t) => t[0] === name)?.[1];
}

function getAllTags(event: RelayEvent, name: string): string[] {
  return event.tags.filter((t) => t[0] === name).map((t) => t[1]);
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
  return deletionEvents.some((event) =>
    event.tags.some((tag) => tag[0] === "a" && tag[1] === coordinate),
  );
}

function eventToProject(event: RelayEvent): Project {
  const d = getTag(event, "d") ?? event.id;
  const name = getTag(event, "name") || d;
  const description = getTag(event, "description") || event.content || "";
  const cloneUrls = getCloneUrls(event);
  const webUrl = getTag(event, "web") ?? null;
  const setupUsers = getAllTags(event, "auth");
  const contributors = [...new Set([...getAllTags(event, "p"), ...setupUsers])];
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

async function fetchProjects(): Promise<Project[]> {
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
    .map(eventToProject)
    .filter(
      (project) =>
        !isHiddenLocally(project) && !isDeletedByA(project, deletionEvents),
    )
    .sort((a, b) => b.createdAt - a.createdAt);
}

async function fetchProject(projectId: string): Promise<Project | null> {
  const events = await relayClient.fetchEvents({
    kinds: [KIND_REPO_ANNOUNCEMENT],
    "#d": [projectId],
    limit: 10,
  });

  const deduped = dedup(events);
  const project = deduped.length > 0 ? eventToProject(deduped[0]) : null;
  if (!project) {
    return null;
  }

  const deletionEvents = await relayClient.fetchEvents({
    kinds: [KIND_DELETION],
    "#a": [project.repoAddress],
    limit: 10,
  });

  return isDeletedByA(project, deletionEvents) ? null : project;
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
      head = value.replace(/^ref:\s*/, "");
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
  const [issueEvents, statusEvents] = await Promise.all([
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
  ]);

  return projectIssueEventsToIssues(issueEvents, statusEvents);
}

async function fetchProjectPullRequests(
  project: Project,
): Promise<ProjectPullRequest[]> {
  const [pullRequestEvents, updateEvents, commentEvents] = await Promise.all([
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
  ]);

  return projectPullRequestEventsToPullRequests(
    pullRequestEvents,
    updateEvents,
    commentEvents,
  );
}

async function createProjectPullRequestComment({
  content,
  project,
  pullRequest,
}: {
  content: string;
  project: Project;
  pullRequest: ProjectPullRequest;
}): Promise<void> {
  const body = content.trim();
  if (!body) {
    throw new Error("Comment cannot be empty.");
  }

  const recipients = new Set([
    project.owner.toLowerCase(),
    pullRequest.author.toLowerCase(),
    ...pullRequest.recipients.map((recipient) => recipient.toLowerCase()),
  ]);
  const tags = [
    ["e", pullRequest.id, "", "root"],
    ["a", project.repoAddress],
    ...[...recipients].map((recipient) => ["p", recipient]),
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

async function fetchProjectRepoSnapshot(
  project: Project,
  branchName?: string | null,
): Promise<ProjectRepoSnapshot | null> {
  const cloneUrl = project.cloneUrls[0];
  if (!cloneUrl) return null;

  return getProjectRepoSnapshot({
    cloneUrl,
    defaultBranch: branchName ?? project.defaultBranch,
    baseBranch: project.defaultBranch,
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
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(project?.cloneUrls[0]),
    queryKey: [
      "project",
      project?.id ?? "none",
      "repo-snapshot",
      selectedBranch ?? "default",
    ],
    queryFn: () => {
      if (!project) throw new Error("No project selected.");
      return fetchProjectRepoSnapshot(project, selectedBranch);
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

export function useProjectRepoSyncStatusQuery(
  project: Project | null | undefined,
  reposDir?: string | null,
  branchName?: string | null,
) {
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useQuery({
    enabled: Boolean(project?.cloneUrls[0]),
    queryKey: [
      "project",
      project?.id ?? "none",
      "repo-sync-status",
      reposDir ?? "default",
      selectedBranch ?? "default",
    ],
    queryFn: () => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      return getProjectRepoSyncStatus({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl: project.cloneUrls[0],
        defaultBranch: selectedBranch,
      });
    },
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

export function useCreateProjectPullRequestCommentMutation(
  project: Project | null | undefined,
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      content,
      pullRequest,
    }: {
      content: string;
      pullRequest: ProjectPullRequest;
    }) => {
      if (!project) throw new Error("No project selected.");
      return createProjectPullRequestComment({ content, project, pullRequest });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["project", project?.id ?? "none", "pull-requests"],
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
      queryClient.setQueryData(["project", project.dtag], null);
      void queryClient.invalidateQueries({ queryKey: projectsQueryKey });
      void queryClient.invalidateQueries({
        queryKey: ["project", project.dtag],
      });
    },
  });
}

export function usePushProjectLocalRepositoryMutation(
  project: Project | null | undefined,
  reposDir?: string | null,
  branchName?: string | null,
) {
  const queryClient = useQueryClient();
  const selectedBranch = branchName ?? project?.defaultBranch ?? null;

  return useMutation({
    mutationFn: () => {
      if (!project?.cloneUrls[0]) throw new Error("No project selected.");
      return pushProjectLocalRepository({
        reposDir,
        projectDtag: project.dtag,
        cloneUrl: project.cloneUrls[0],
        defaultBranch: selectedBranch,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["project", project?.id ?? "none"],
      });
      void queryClient.invalidateQueries({ queryKey: ["projects"] });
    },
  });
}
