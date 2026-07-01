import type { RelayEvent } from "@/shared/api/types";

export type ProjectPullRequestUpdate = {
  id: string;
  content: string;
  author: string;
  createdAt: number;
  commit: string | null;
  cloneUrls: string[];
};

export type ProjectPullRequestComment = {
  id: string;
  content: string;
  author: string;
  createdAt: number;
};

export type ProjectPullRequest = {
  id: string;
  title: string;
  content: string;
  author: string;
  createdAt: number;
  repoAddress: string | null;
  labels: string[];
  recipients: string[];
  status: "Open" | "Merged" | "Closed" | "Draft";
  statusEventId: string | null;
  branchName: string | null;
  commit: string | null;
  cloneUrls: string[];
  updateCount: number;
  updatedAt: number;
  updates: ProjectPullRequestUpdate[];
  comments: ProjectPullRequestComment[];
};

export function eventToProjectPullRequest(
  pullRequest: RelayEvent,
  updateEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
  statusEvents?: RelayEvent[],
): ProjectPullRequest;
export function projectPullRequestEventsToPullRequests(
  pullRequestEvents: RelayEvent[],
  updateEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
  statusEvents?: RelayEvent[],
): ProjectPullRequest[];
