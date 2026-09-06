import type { RelayEvent } from "@/shared/api/types";

export type ProjectIssueStatus =
  | "Triage"
  | "Backlog"
  | "In Development"
  | "Implemented"
  | "Code-QS"
  | "To Be Published"
  | "Ready for Test"
  | "Done";

export type MyBuzzWorkflowStatus = {
  eventId: string;
  state:
    | "triage"
    | "backlog"
    | "in-development"
    | "implemented"
    | "code-qs"
    | "to-be-published"
    | "ready-for-test";
  reason: string | null;
  createdAt: number;
};

export type ProjectIssueActivity = {
  id: string;
  createdAt: number;
  text: string;
};

export type ProjectIssueComment = {
  id: string;
  content: string;
  tags: string[][];
  author: string;
  createdAt: number;
  recipients: string[];
  actionRequired: boolean;
};

export type ProjectIssueReviewConfirmation = {
  eventId: string;
  createdAt: number;
  kanbanStatus: "done" | "ready" | "todo";
};

export type ProjectIssueReviewVerdict = {
  eventId: string;
  kind: "accepted" | "rejected";
  actorPubkey: string;
  createdAt: number;
  reason: string | null;
  confirmation: ProjectIssueReviewConfirmation | null;
};

export type ProjectIssueReview = {
  id: string;
  target: string;
  evidence: string;
  test: string;
  limitations: string;
  authorizedHumanPubkeys: string[];
  verdict: ProjectIssueReviewVerdict | null;
};

export type ProjectIssueReviewAuthority = {
  coordinatorPubkeys: string[];
  humanPubkeys: string[];
};

export type ProjectIssue = {
  id: string;
  title: string;
  content: string;
  tags: string[][];
  author: string;
  createdAt: number;
  repoAddress: string | null;
  channelId: string | null;
  originAgentName: string | null;
  labels: string[];
  recipients: string[];
  assignees: string[];
  assigneeOperationHeads: Record<string, string>;
  status: ProjectIssueStatus;
  workflowStatus: MyBuzzWorkflowStatus | null;
  activity: ProjectIssueActivity[];
  statusEventId: string | null;
  statusCreatedAt: number | null;
  updatedAt: number;
  currentReview: ProjectIssueReview | null;
  comments: ProjectIssueComment[];
};

export const ISSUE_ASSIGNMENT_LABEL: "assignment";
export const ISSUE_UNASSIGNMENT_LABEL: "unassignment";
export const ISSUE_ACTION_REQUIRED_LABEL: "action-required";
export const MYBUZZ_WORKFLOW_STATUS_LABEL: "mybuzz-workflow-status";
export const MYBUZZ_WORKFLOW_STATUS_WORKFLOW: "mybuzz-status-v1";
export const MYBUZZ_WORKFLOW_STATUS_OWNER: string;
export const MYBUZZ_WORKFLOW_STATUS_STATES: string[];
export const MYBUZZ_WORKFLOW_STATUS_LABELS: Record<string, ProjectIssueStatus>;
export function isHumanDirectedIssueComment(body: string): boolean;

export const PROJECT_ISSUE_STATUS: {
  TRIAGE: "Triage";
  BACKLOG: "Backlog";
  IN_PROGRESS: "In Progress";
  APPROVED: "Approved";
  IN_REVIEW: "In Review";
  DONE: "Done";
  CLOSED: "Closed";
};

export function getTag(event: RelayEvent, name: string): string | undefined;
export function getAllTags(event: RelayEvent, name: string): string[];
export function getImetaTags(event: RelayEvent): string[][];
export function eventToProjectIssue(
  issue: RelayEvent,
  statusEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
  additionalStatusActors?: string[],
  reviewAuthority?: ProjectIssueReviewAuthority,
): ProjectIssue;
export function projectIssueEventsToIssues(
  issueEvents: RelayEvent[],
  statusEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
  additionalStatusActors?: string[],
  reviewAuthority?: ProjectIssueReviewAuthority,
): ProjectIssue[];
export function nextProjectIssueStatusCreatedAt(
  issue: ProjectIssue,
  now: number,
): number;
export function nextProjectIssueCommentCreatedAt(
  issue: ProjectIssue,
  now: number,
  author: string,
): number;
export function buildGitIssueTags(input: {
  repoAddress: string;
  repoOwner: string;
  title: string;
  labels?: string[];
}): string[][];
export function buildGitStatusTags(input: {
  issueId: string;
  repoAddress?: string | null;
  repoOwner?: string | null;
}): string[][];
