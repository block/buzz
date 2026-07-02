import type { RelayEvent } from "@/shared/api/types";

export type ProjectIssueStatus =
  | "Triage"
  | "Backlog"
  | "In Progress"
  | "In Review"
  | "Done"
  | "Closed";

export type ProjectIssueComment = {
  id: string;
  content: string;
  author: string;
  createdAt: number;
};

export type ProjectIssue = {
  id: string;
  title: string;
  content: string;
  author: string;
  createdAt: number;
  repoAddress: string | null;
  labels: string[];
  recipients: string[];
  status: ProjectIssueStatus;
  statusEventId: string | null;
  updatedAt: number;
  comments: ProjectIssueComment[];
};

export const PROJECT_ISSUE_STATUS: {
  TRIAGE: "Triage";
  BACKLOG: "Backlog";
  IN_PROGRESS: "In Progress";
  IN_REVIEW: "In Review";
  DONE: "Done";
  CLOSED: "Closed";
};

export function getTag(event: RelayEvent, name: string): string | undefined;
export function getAllTags(event: RelayEvent, name: string): string[];
export function eventToProjectIssue(
  issue: RelayEvent,
  statusEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
): ProjectIssue;
export function projectIssueEventsToIssues(
  issueEvents: RelayEvent[],
  statusEvents?: RelayEvent[],
  commentEvents?: RelayEvent[],
): ProjectIssue[];
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
