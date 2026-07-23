import { useMutation, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { signProjectPullRequestReviewRequest } from "@/shared/api/projectGit";
import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import {
  KIND_GIT_STATUS_CLOSED,
  KIND_GIT_STATUS_DRAFT,
  KIND_GIT_STATUS_OPEN,
  KIND_TEXT_NOTE,
} from "@/shared/constants/kinds";
import type { Project } from "./hooks";
import {
  nextProjectPullRequestStatusCreatedAt,
  type ProjectPullRequest,
} from "./projectPullRequests.mjs";
import {
  PR_APPROVAL_LABEL,
  PR_CHANGES_REQUESTED_LABEL,
  PR_REVIEW_REQUEST_LABEL,
} from "./projectPullRequests.mjs";

/** NIP-34 lifecycle states the desktop can publish for a PR. Merged (1631)
 * is intentionally excluded — merges happen through git, not this UI. */
export type ProjectPullRequestLifecycleStatus = "open" | "draft" | "closed";

const PR_STATUS_KIND_BY_LIFECYCLE: Record<
  ProjectPullRequestLifecycleStatus,
  number
> = {
  open: KIND_GIT_STATUS_OPEN,
  draft: KIND_GIT_STATUS_DRAFT,
  closed: KIND_GIT_STATUS_CLOSED,
};

// Same shape as `buzz pr status` (buzz-sdk build_git_status): root `e` tag,
// repo `a` tag, and `p` tags for the repo owner + PR author. Only the PR
// author or repo owner are trusted for status changes (allowedActorsForRoot).
async function updateProjectPullRequestStatus({
  project,
  pullRequest,
  status,
}: {
  project: Project;
  pullRequest: ProjectPullRequest;
  status: ProjectPullRequestLifecycleStatus;
}): Promise<void> {
  const recipients = new Set([
    project.owner.toLowerCase(),
    pullRequest.author.toLowerCase(),
  ]);
  const event = await signRelayEvent({
    kind: PR_STATUS_KIND_BY_LIFECYCLE[status],
    content: "",
    createdAt: nextProjectPullRequestStatusCreatedAt(
      pullRequest,
      Math.floor(Date.now() / 1_000),
    ),
    tags: [
      ["e", pullRequest.id, "", "root"],
      ["a", project.repoAddress],
      ...[...recipients].map((recipient) => ["p", recipient]),
    ],
  });

  await relayClient.publishEvent(
    event,
    "Timed out updating pull request status.",
    "Failed to update pull request status.",
  );
}

// Review requests and approvals are labeled kind:1 comments (see
// projectPullRequests.mjs) — NIP-34 has no dedicated review kinds, and the
// relay does not register kind 1111. `p` tags on a review request are the
// requested reviewers; parsing only trusts requests signed by the PR author
// or repo owner.
async function requestProjectPullRequestReview({
  project,
  pullRequest,
  reviewers,
  reviewerLabel,
  signAsManagedOwner,
}: {
  project: Project;
  pullRequest: ProjectPullRequest;
  reviewers: string[];
  reviewerLabel: string;
  signAsManagedOwner: boolean;
}): Promise<void> {
  if (reviewers.length === 0) {
    throw new Error("Select at least one reviewer.");
  }
  const reviewerPubkeys = [
    ...new Set(reviewers.map((pubkey) => pubkey.toLowerCase())),
  ];
  if (signAsManagedOwner) {
    await signProjectPullRequestReviewRequest({
      targetOwner: project.owner,
      repoAddress: project.repoAddress,
      pullRequestId: pullRequest.id,
      reviewers: reviewerPubkeys,
      reviewerLabel,
    });
    return;
  }
  const event = await signRelayEvent({
    kind: KIND_TEXT_NOTE,
    content: `Requested a review from ${reviewerLabel}`,
    tags: [
      ["e", pullRequest.id, "", "root"],
      ["a", project.repoAddress],
      ...reviewerPubkeys.map((pubkey) => ["p", pubkey]),
      ["t", PR_REVIEW_REQUEST_LABEL],
    ],
  });

  await relayClient.publishEvent(
    event,
    "Timed out requesting review.",
    "Failed to request review.",
  );
}

type ProjectPullRequestReviewDecision = "approve" | "request-changes";

const REVIEW_DECISION_DETAILS: Record<
  ProjectPullRequestReviewDecision,
  {
    content: string;
    errorMessage: string;
    label: string;
    timeoutMessage: string;
  }
> = {
  approve: {
    content: "Approved these changes",
    errorMessage: "Failed to approve pull request.",
    label: PR_APPROVAL_LABEL,
    timeoutMessage: "Timed out approving pull request.",
  },
  "request-changes": {
    content: "Requested changes",
    errorMessage: "Failed to request changes.",
    label: PR_CHANGES_REQUESTED_LABEL,
    timeoutMessage: "Timed out requesting changes.",
  },
};

async function submitProjectPullRequestReview({
  createdAt,
  decision,
  project,
  pullRequest,
}: {
  createdAt: number;
  decision: ProjectPullRequestReviewDecision;
  project: Project;
  pullRequest: ProjectPullRequest;
}): Promise<void> {
  if (!pullRequest.commit) {
    throw new Error("The pull request has no commit to review.");
  }
  const details = REVIEW_DECISION_DETAILS[decision];
  const recipients = new Set([
    project.owner.toLowerCase(),
    pullRequest.author.toLowerCase(),
  ]);
  const event = await signRelayEvent({
    kind: KIND_TEXT_NOTE,
    content: details.content,
    createdAt,
    tags: [
      ["e", pullRequest.id, "", "root"],
      ["a", project.repoAddress],
      ...[...recipients].map((recipient) => ["p", recipient]),
      ["t", details.label],
      ["c", pullRequest.commit],
    ],
  });

  await relayClient.publishEvent(
    event,
    details.timeoutMessage,
    details.errorMessage,
  );
}

export function useProjectPullRequestWriteInvalidation(
  project: Project | null | undefined,
) {
  const queryClient = useQueryClient();
  return React.useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: ["project", project?.id ?? "none", "pull-requests"],
    });
    void queryClient.invalidateQueries({
      queryKey: ["projects", "work-items"],
    });
    void queryClient.invalidateQueries({
      queryKey: ["projects", "activity-summaries"],
    });
  }, [project?.id, queryClient]);
}

export function useUpdateProjectPullRequestStatusMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectPullRequestWriteInvalidation(project);

  return useMutation({
    mutationFn: ({
      pullRequest,
      status,
    }: {
      pullRequest: ProjectPullRequest;
      status: ProjectPullRequestLifecycleStatus;
    }) => {
      if (!project) throw new Error("No project selected.");
      return updateProjectPullRequestStatus({ project, pullRequest, status });
    },
    onSuccess: invalidate,
  });
}

export function useRequestProjectPullRequestReviewMutation(
  project: Project | null | undefined,
) {
  const invalidate = useProjectPullRequestWriteInvalidation(project);

  return useMutation({
    mutationFn: ({
      pullRequest,
      reviewers,
      reviewerLabel,
      signAsManagedOwner,
    }: {
      pullRequest: ProjectPullRequest;
      reviewers: string[];
      reviewerLabel: string;
      signAsManagedOwner: boolean;
    }) => {
      if (!project) throw new Error("No project selected.");
      return requestProjectPullRequestReview({
        project,
        pullRequest,
        reviewers,
        reviewerLabel,
        signAsManagedOwner,
      });
    },
    onSuccess: invalidate,
  });
}

function useProjectPullRequestReviewDecisionMutation(
  project: Project | null | undefined,
  decision: ProjectPullRequestReviewDecision,
) {
  const invalidate = useProjectPullRequestWriteInvalidation(project);

  return useMutation({
    mutationFn: ({
      createdAt,
      pullRequest,
    }: {
      createdAt: number;
      pullRequest: ProjectPullRequest;
    }) => {
      if (!project) throw new Error("No project selected.");
      return submitProjectPullRequestReview({
        createdAt,
        decision,
        project,
        pullRequest,
      });
    },
    onSuccess: invalidate,
  });
}

export function useApproveProjectPullRequestMutation(
  project: Project | null | undefined,
) {
  return useProjectPullRequestReviewDecisionMutation(project, "approve");
}

export function useRequestProjectPullRequestChangesMutation(
  project: Project | null | undefined,
) {
  return useProjectPullRequestReviewDecisionMutation(
    project,
    "request-changes",
  );
}
