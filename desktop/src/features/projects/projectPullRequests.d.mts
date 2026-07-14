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
  isApproval: boolean;
  isReviewRequest: boolean;
  reviewerPubkeys: string[];
};

export type ProjectPullRequestApproval = {
  id: string;
  author: string;
  createdAt: number;
};

export const PR_REVIEW_REQUEST_LABEL: string;
export const PR_APPROVAL_LABEL: string;

export type ProjectPullRequest = {
  id: string;
  title: string;
  content: string;
  author: string;
  createdAt: number;
  repoAddress: string | null;
  labels: string[];
  recipients: string[];
  /** Requested reviewers (root `p` tags + trusted review-request comments). */
  reviewers: string[];
  /** Latest approval per reviewer, oldest first. */
  approvals: ProjectPullRequestApproval[];
  status: "Open" | "Merged" | "Closed" | "Draft";
  statusEventId: string | null;
  statusCreatedAt: number | null;
  branchName: string | null;
  initialCommit: string | null;
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
export function nextProjectPullRequestStatusCreatedAt(
  pullRequest: Pick<ProjectPullRequest, "statusCreatedAt">,
  now: number,
): number;
export function projectPullRequestEventsToPullRequests(
  pullRequestEvents: RelayEvent[],
  updateEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
  statusEvents?: RelayEvent[],
): ProjectPullRequest[];
