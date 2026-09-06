import { useMutation } from "@tanstack/react-query";

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import {
  KIND_GIT_STATUS_MERGED,
  KIND_GIT_STATUS_OPEN,
} from "@/shared/constants/kinds";
import type { Repository as Project } from "./hooks";
import { useProjectIssueWriteInvalidation } from "./issueAssignments";
import {
  MYBUZZ_WORKFLOW_STATUS_LABEL,
  MYBUZZ_WORKFLOW_STATUS_OWNER,
  MYBUZZ_WORKFLOW_STATUS_STATES,
  nextProjectIssueStatusCreatedAt,
  type ProjectIssue,
} from "./projectIssues.mjs";

export type ProjectIssueHumanVerdict = "accepted" | "rejected";

export const MAX_REJECTION_REASON_LENGTH = 500;

const MYBUZZ_REASON_REQUIRED_STATES = new Set([
  "triage",
  "backlog",
  "ready-for-test",
]);

export type MyBuzzWorkflowStatusState =
  (typeof MYBUZZ_WORKFLOW_STATUS_STATES)[number];

function hasControlCharacters(value: string): boolean {
  return [...value].some((character) => {
    const code = character.charCodeAt(0);
    return code <= 31 || code === 127;
  });
}

export function buildProjectIssueVerdict({
  issue,
  project,
  reason,
  verdict,
}: {
  issue: Pick<ProjectIssue, "author" | "id" | "currentReview">;
  project: Pick<Project, "owner" | "repoAddress">;
  reason?: string;
  verdict: ProjectIssueHumanVerdict;
}): { content: string; kind: number; tags: string[][] } {
  const reviewId = issue.currentReview?.id;
  if (!reviewId) {
    throw new Error("A current review is required.");
  }
  if (issue.currentReview?.verdict) {
    throw new Error("A verdict was already sent for the current review.");
  }
  const content = reason?.trim() ?? "";
  if (
    verdict === "rejected" &&
    (!content ||
      content.length > MAX_REJECTION_REASON_LENGTH ||
      hasControlCharacters(content))
  ) {
    throw new Error(
      `A concrete, control-character-free rejection reason of at most ${MAX_REJECTION_REASON_LENGTH} characters is required.`,
    );
  }
  return {
    kind:
      verdict === "accepted" ? KIND_GIT_STATUS_MERGED : KIND_GIT_STATUS_OPEN,
    content: verdict === "rejected" ? content : "",
    tags: [
      ["e", issue.id, "", "root"],
      ["a", project.repoAddress],
      ...[
        ...new Set([project.owner.toLowerCase(), issue.author.toLowerCase()]),
      ].map((recipient) => ["p", recipient]),
      ["t", "human-verdict"],
      ["verdict", verdict],
      ["review", reviewId],
    ],
  };
}

export function canSubmitProjectIssueVerdict(
  issue: Pick<ProjectIssue, "currentReview">,
  viewer: string | null,
): boolean {
  return Boolean(
    viewer &&
      !issue.currentReview?.verdict &&
      issue.currentReview?.authorizedHumanPubkeys.some(
        (pubkey) => pubkey === viewer.toLowerCase(),
      ),
  );
}

export function canWriteMyBuzzWorkflowStatus(viewer: string | null): boolean {
  return viewer?.toLowerCase() === MYBUZZ_WORKFLOW_STATUS_OWNER;
}

export function buildMyBuzzWorkflowStatus({
  issue,
  project,
  reason,
  state,
}: {
  issue: Pick<ProjectIssue, "id" | "statusCreatedAt">;
  project: Pick<Project, "repoAddress">;
  reason?: string;
  state: MyBuzzWorkflowStatusState;
}): { content: string; kind: number; tags: string[][] } {
  if (!MYBUZZ_WORKFLOW_STATUS_STATES.includes(state)) {
    throw new Error("Unknown MyBuzz workflow status.");
  }
  const normalizedReason = reason?.trim() ?? "";
  if (
    (MYBUZZ_REASON_REQUIRED_STATES.has(state) && !normalizedReason) ||
    (normalizedReason &&
      (normalizedReason !== reason || hasControlCharacters(normalizedReason)))
  ) {
    throw new Error(`A printable reason is required for ${state}.`);
  }
  return {
    content: `Status changed to ${state}.`,
    kind: 1,
    tags: [
      ["e", issue.id, "", "root"],
      ["a", project.repoAddress],
      ["t", MYBUZZ_WORKFLOW_STATUS_LABEL],
      ["workflow", "mybuzz-status-v1"],
      ["state", state],
      ...(normalizedReason ? [["reason", normalizedReason]] : []),
    ],
  };
}

async function updateProjectIssueStatus({
  issue,
  project,
  reason,
  state,
}: {
  issue: ProjectIssue;
  project: Project;
  reason?: string;
  state: MyBuzzWorkflowStatusState;
}): Promise<void> {
  const createdAt = nextProjectIssueStatusCreatedAt(
    issue,
    Math.floor(Date.now() / 1_000),
  );
  const workflowStatus = buildMyBuzzWorkflowStatus({
    issue,
    project,
    reason,
    state,
  });
  const event = await signRelayEvent({
    ...workflowStatus,
    createdAt,
  });

  await relayClient.publishEvent(
    event,
    "Timed out updating MyBuzz workflow status.",
    "Failed to update MyBuzz workflow status.",
  );
}

async function submitProjectIssueVerdict({
  issue,
  project,
  reason,
  verdict,
}: {
  issue: ProjectIssue;
  project: Project;
  reason?: string;
  verdict: ProjectIssueHumanVerdict;
}): Promise<void> {
  const verdictEvent = buildProjectIssueVerdict({
    issue,
    project,
    reason,
    verdict,
  });
  const event = await signRelayEvent({
    ...verdictEvent,
    createdAt: nextProjectIssueStatusCreatedAt(
      issue,
      Math.floor(Date.now() / 1_000),
    ),
  });
  await relayClient.publishEvent(
    event,
    "Timed out sending human verdict.",
    "Failed to send human verdict.",
  );
}

export function useSubmitProjectIssueVerdictMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectIssueWriteInvalidation(project);
  return useMutation({
    mutationFn: (input: {
      issue: ProjectIssue;
      reason?: string;
      verdict: ProjectIssueHumanVerdict;
    }) => {
      if (!project) throw new Error("No project selected.");
      return submitProjectIssueVerdict({ ...input, project });
    },
    onSuccess: invalidate,
  });
}

export function useUpdateProjectIssueStatusMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectIssueWriteInvalidation(project);

  return useMutation({
    mutationFn: (input: {
      issue: ProjectIssue;
      reason?: string;
      state: MyBuzzWorkflowStatusState;
    }) => {
      if (!project) throw new Error("No project selected.");
      return updateProjectIssueStatus({ ...input, project });
    },
    onSuccess: invalidate,
  });
}
