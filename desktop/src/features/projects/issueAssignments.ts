import { useMutation, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  signProjectIssueAssignment,
  signProjectIssueUnassignment,
} from "@/shared/api/projectGit";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { KIND_TEXT_NOTE } from "@/shared/constants/kinds";
import type { Repository as Project } from "./hooks";
import {
  ISSUE_ASSIGNMENT_LABEL,
  ISSUE_UNASSIGNMENT_LABEL,
  nextProjectIssueCommentCreatedAt,
  type ProjectIssue,
} from "./projectIssues.mjs";

function nextAssignmentOperationCreatedAt(
  issue: ProjectIssue,
  project: Project,
  signAsManagedOwner: boolean,
  signerPubkey: string,
) {
  const signer = signAsManagedOwner ? project.owner : signerPubkey;
  return nextProjectIssueCommentCreatedAt(
    issue,
    Math.floor(Date.now() / 1_000),
    signer,
  );
}

function normalizedAssigneeLabel(label: string) {
  const normalized = label.trim();
  if (normalized.length === 0 || Array.from(normalized).length > 128) {
    throw new Error("Assignee label must be between 1 and 128 characters.");
  }
  return normalized;
}

type IssueAssignmentOperation = "assign" | "unassign";
type IssueAssignmentMutationInput = {
  assignees: string[];
  assigneeLabel: string;
  issue: ProjectIssue;
  signerPubkey: string;
  signAsManagedOwner: boolean;
};

async function writeProjectIssueAssignment({
  assignees,
  assigneeLabel,
  issue,
  operation,
  project,
  signerPubkey,
  signAsManagedOwner,
}: IssueAssignmentMutationInput & {
  operation: IssueAssignmentOperation;
  project: Project;
}): Promise<void> {
  if (assignees.length === 0) {
    throw new Error("Select at least one assignee.");
  }
  const normalizedLabel = normalizedAssigneeLabel(assigneeLabel);
  const assigneePubkeys = [
    ...new Set(assignees.map((pubkey) => pubkey.toLowerCase())),
  ];
  const createdAt = nextAssignmentOperationCreatedAt(
    issue,
    project,
    signAsManagedOwner,
    signerPubkey,
  );
  const isAssignment = operation === "assign";
  const content = isAssignment
    ? `Assigned this task to ${normalizedLabel}`
    : `Unassigned ${normalizedLabel} from this task`;
  const label = isAssignment
    ? ISSUE_ASSIGNMENT_LABEL
    : ISSUE_UNASSIGNMENT_LABEL;
  if (signAsManagedOwner) {
    const signManagedOperation = isAssignment
      ? signProjectIssueAssignment
      : signProjectIssueUnassignment;
    await signManagedOperation({
      targetOwner: project.owner,
      repoAddress: project.repoAddress,
      issueId: issue.id,
      assignees: assigneePubkeys,
      assigneeLabel: normalizedLabel,
      createdAt,
    });
    return;
  }
  const normalizedSigner = signerPubkey.toLowerCase();
  const prior =
    assigneePubkeys.length === 1 && assigneePubkeys[0] === normalizedSigner
      ? issue.assigneeOperationHeads[normalizedSigner]
      : undefined;
  const event = await signRelayEvent({
    kind: KIND_TEXT_NOTE,
    content,
    createdAt,
    tags: [
      ["e", issue.id, "", "root"],
      ["a", project.repoAddress],
      ...assigneePubkeys.map((pubkey) => ["p", pubkey]),
      ["t", label],
      ...(prior ? [["prior", prior]] : []),
    ],
  });

  await relayClient.publishEvent(
    event,
    `Timed out ${operation}ing task.`,
    `Failed to ${operation} task.`,
  );
}

export function useProjectIssueWriteInvalidation(
  project: Project | null | undefined,
) {
  const queryClient = useQueryClient();
  return React.useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: ["project", project?.id ?? "none", "issues"],
    });
    void queryClient.invalidateQueries({
      queryKey: ["projects", "work-items"],
    });
    void queryClient.invalidateQueries({
      queryKey: ["projects", "activity-summaries"],
    });
  }, [project?.id, queryClient]);
}

function useProjectIssueAssignmentMutation(
  project: Project | null | undefined,
  operation: IssueAssignmentOperation,
) {
  const invalidate = useProjectIssueWriteInvalidation(project);

  return useMutation({
    mutationFn: (input: IssueAssignmentMutationInput) => {
      if (!project) throw new Error("No project selected.");
      return writeProjectIssueAssignment({
        ...input,
        operation,
        project,
      });
    },
    onSuccess: invalidate,
  });
}

export function useAssignProjectIssueMutation(
  project: Project | null | undefined,
) {
  return useProjectIssueAssignmentMutation(project, "assign");
}

export function useUnassignProjectIssueMutation(
  project: Project | null | undefined,
) {
  return useProjectIssueAssignmentMutation(project, "unassign");
}
