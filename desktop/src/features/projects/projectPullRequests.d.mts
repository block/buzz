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
  commit: string | null;
  anchor: ProjectPullRequestCommentAnchor | null;
  inlineCommentStatus: "current" | "outdated" | null;
  isInlineComment: boolean;
  isApproval: boolean;
  isChangeRequest: boolean;
  isReviewRequest: boolean;
  isTrustedReviewDecision: boolean;
  isTrustedReviewRequest: boolean;
  reviewDecision: "approved" | "changes-requested" | null;
  reviewDecisionStatus: "current" | "historical" | null;
  reviewerPubkeys: string[];
};

export type ProjectPullRequestCommentAnchor = {
  line: number;
  path: string;
  side: "old" | "new";
};

export type ProjectPullRequestApproval = {
  id: string;
  author: string;
  createdAt: number;
  commit: string;
  reviewDecision: "approved";
};

export type ProjectPullRequestChangeRequest = {
  id: string;
  author: string;
  createdAt: number;
  commit: string;
  reviewDecision: "changes-requested";
};

export const PR_REVIEW_REQUEST_LABEL: string;
export const PR_APPROVAL_LABEL: string;
export const PR_CHANGES_REQUESTED_LABEL: string;
export const PR_INLINE_COMMENT_LABEL: string;

export function normalizeProjectPullRequestCommentAnchor(
  anchor:
    | {
        line?: unknown;
        path?: unknown;
        side?: unknown;
      }
    | null
    | undefined,
): ProjectPullRequestCommentAnchor | null;

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
  /** Latest current-commit approval per reviewer, oldest first. */
  approvals: ProjectPullRequestApproval[];
  /** Latest current-commit change request per reviewer, oldest first. */
  changeRequests: ProjectPullRequestChangeRequest[];
  status: "Open" | "Merged" | "Closed" | "Draft";
  statusEventId: string | null;
  statusCreatedAt: number | null;
  branchName: string | null;
  targetBranch: string | null;
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
export function nextProjectPullRequestReviewCreatedAt(
  pullRequest: Pick<ProjectPullRequest, "approvals" | "changeRequests">,
  now: number,
): number;
export function projectPullRequestCommentTimelineKind(
  comment: Pick<
    ProjectPullRequestComment,
    | "isTrustedReviewDecision"
    | "isTrustedReviewRequest"
    | "reviewDecision"
    | "reviewDecisionStatus"
  >,
): "approved" | "changes-requested" | "review-request" | null;
export function projectPullRequestReviewSummary(
  pullRequest: Pick<
    ProjectPullRequest,
    "approvals" | "changeRequests" | "reviewers" | "status"
  >,
): {
  approvalCount: number;
  changeRequestCount: number;
  detail: string | null;
  showState: boolean;
  state: string;
};
export function projectPullRequestEffectiveReviewDecision(
  pullRequest: Pick<ProjectPullRequest, "approvals" | "changeRequests">,
  comment: Pick<ProjectPullRequestComment, "id">,
): "approved" | "changes-requested" | null;
export function projectPullRequestEventsToPullRequests(
  pullRequestEvents: RelayEvent[],
  updateEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
  statusEvents?: RelayEvent[],
): ProjectPullRequest[];
