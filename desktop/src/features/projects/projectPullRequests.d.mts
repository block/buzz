import type { RelayEvent } from "@/shared/api/types";

export type ProjectPullRequest = {
  id: string;
  title: string;
  content: string;
  author: string;
  createdAt: number;
  repoAddress: string | null;
  labels: string[];
  recipients: string[];
  branchName: string | null;
  commit: string | null;
  cloneUrls: string[];
  updateCount: number;
  updatedAt: number;
};

export function eventToProjectPullRequest(
  pullRequest: RelayEvent,
  updateEvents?: RelayEvent[],
): ProjectPullRequest;
export function projectPullRequestEventsToPullRequests(
  pullRequestEvents: RelayEvent[],
  updateEvents?: RelayEvent[],
): ProjectPullRequest[];
