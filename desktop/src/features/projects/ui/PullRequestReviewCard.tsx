import { Check, GitPullRequest, GitPullRequestDraft, X } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import type { Project, ProjectPullRequest } from "@/features/projects/hooks";
import {
  nextProjectPullRequestReviewCreatedAt,
  projectPullRequestReviewSummary,
} from "@/features/projects/projectPullRequests.mjs";
import {
  useApproveProjectPullRequestMutation,
  useRequestProjectPullRequestChangesMutation,
  useUpdateProjectPullRequestStatusMutation,
} from "@/features/projects/pullRequestReviews";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  MergePullRequestButton,
  type OpenMergeRecoveryTerminal,
} from "./MergePullRequestButton";

/** GitHub-style review state and actions rendered in the conversation flow. */
export function PullRequestReviewCard({
  onOpenTerminal,
  project,
  pullRequest,
}: {
  onOpenTerminal?: OpenMergeRecoveryTerminal;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const identityQuery = useIdentityQuery();
  const { isPending: isUpdatingStatus, mutateAsync: updatePullRequestStatus } =
    useUpdateProjectPullRequestStatusMutation(project);
  const { isPending: isApproving, mutateAsync: approvePullRequest } =
    useApproveProjectPullRequestMutation(project);
  const {
    isPending: isRequestingChanges,
    mutateAsync: requestPullRequestChanges,
  } = useRequestProjectPullRequestChangesMutation(project);
  const reviewDecisionInFlightRef = React.useRef(false);
  const lastReviewDecisionCreatedAtRef = React.useRef(0);

  const viewerPubkey = identityQuery.data?.pubkey ?? null;
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const isAuthor = viewer === normalizePubkey(pullRequest.author);
  const isOwner = viewer === normalizePubkey(project.owner);
  const isRequestedReviewer = Boolean(
    viewer &&
      pullRequest.reviewers.some(
        (reviewer) => normalizePubkey(reviewer) === viewer,
      ),
  );
  const isManagedAgentOwner = useIsManagedAgent(project.owner) === true;
  const canChangeStatus = Boolean(viewer) && (isAuthor || isOwner);
  const hasApproved = Boolean(
    viewer &&
      pullRequest.approvals.some(
        (approval) => normalizePubkey(approval.author) === viewer,
      ),
  );
  const hasRequestedChanges = Boolean(
    viewer &&
      pullRequest.changeRequests.some(
        (request) => normalizePubkey(request.author) === viewer,
      ),
  );
  const canReview =
    Boolean(viewer) &&
    !isAuthor &&
    (isOwner || isRequestedReviewer) &&
    Boolean(pullRequest.commit) &&
    (pullRequest.status === "Open" || pullRequest.status === "Draft");
  const canApprove = canReview && !hasApproved;
  const canRequestChanges = canReview && !hasRequestedChanges;
  const canMerge =
    (isOwner || isManagedAgentOwner) &&
    pullRequest.status === "Open" &&
    Boolean(pullRequest.branchName && pullRequest.commit);

  const handleStatusChange = React.useCallback(
    async (status: "open" | "draft") => {
      try {
        await updatePullRequestStatus({ pullRequest, status });
        toast.success(
          status === "draft"
            ? "Converted to draft."
            : "Marked as ready for review.",
        );
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to update status.",
        );
      }
    },
    [pullRequest, updatePullRequestStatus],
  );

  const runReviewDecision = React.useCallback(
    async (
      mutate: (input: {
        createdAt: number;
        pullRequest: ProjectPullRequest;
      }) => Promise<unknown>,
      successMessage: string,
      fallbackErrorMessage: string,
    ) => {
      if (reviewDecisionInFlightRef.current) return;
      reviewDecisionInFlightRef.current = true;
      const createdAt = Math.max(
        nextProjectPullRequestReviewCreatedAt(
          pullRequest,
          Math.floor(Date.now() / 1_000),
        ),
        lastReviewDecisionCreatedAtRef.current + 1,
      );
      lastReviewDecisionCreatedAtRef.current = createdAt;

      try {
        await mutate({ createdAt, pullRequest });
        toast.success(successMessage);
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : fallbackErrorMessage,
        );
      } finally {
        reviewDecisionInFlightRef.current = false;
      }
    },
    [pullRequest],
  );

  const handleApprove = React.useCallback(async () => {
    await runReviewDecision(
      approvePullRequest,
      "Pull request approved.",
      "Failed to approve.",
    );
  }, [approvePullRequest, runReviewDecision]);

  const handleRequestChanges = React.useCallback(async () => {
    await runReviewDecision(
      requestPullRequestChanges,
      "Changes requested.",
      "Failed to request changes.",
    );
  }, [requestPullRequestChanges, runReviewDecision]);

  const {
    approvalCount,
    changeRequestCount,
    detail: reviewStateDetail,
    showState: showReviewState,
    state: reviewState,
  } = projectPullRequestReviewSummary(pullRequest);
  const reviewDecisionPending = isApproving || isRequestingChanges;
  const isDraft = pullRequest.status === "Draft";
  const showActions =
    hasApproved ||
    hasRequestedChanges ||
    canApprove ||
    canRequestChanges ||
    canMerge ||
    (canChangeStatus && isDraft);
  const showDraftControl = canChangeStatus && pullRequest.status === "Open";

  if (
    approvalCount + changeRequestCount > 0 &&
    !showActions &&
    !showDraftControl
  ) {
    return null;
  }

  return (
    <div className="space-y-2.5 pt-3">
      <div className="min-w-0 space-y-2.5 rounded-xl bg-muted/40 px-3 py-2.5">
        {showReviewState ? (
          <div className="flex min-w-0 items-start gap-2">
            {isDraft ? (
              <GitPullRequestDraft className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            ) : (
              <GitPullRequest className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            )}
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-foreground">
                {reviewState}
              </p>
              {reviewStateDetail ? (
                <p className="text-xs text-muted-foreground">
                  {reviewStateDetail}
                </p>
              ) : null}
            </div>
          </div>
        ) : null}
        {showActions ? (
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            {hasApproved ? (
              <span className="inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border border-green-600/40 px-3.5 text-xs font-medium text-green-600 dark:text-green-500">
                <Check className="h-3.5 w-3.5" />
                Approved
              </span>
            ) : null}
            {hasRequestedChanges ? (
              <span className="inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border border-destructive/40 px-3.5 text-xs font-medium text-destructive">
                <X className="h-3.5 w-3.5" />
                Changes requested
              </span>
            ) : null}
            {canApprove ? (
              <Button
                className="h-8 gap-1.5 bg-green-600 px-3.5 text-white shadow-sm hover:bg-green-700"
                disabled={reviewDecisionPending}
                onClick={() => {
                  void handleApprove();
                }}
                size="xs"
                type="button"
              >
                <Check className="h-3.5 w-3.5" />
                Approve
              </Button>
            ) : null}
            {canRequestChanges ? (
              <Button
                className="h-8 gap-1.5 px-3.5 text-destructive hover:text-destructive"
                disabled={reviewDecisionPending}
                onClick={() => {
                  void handleRequestChanges();
                }}
                size="xs"
                type="button"
                variant="outline"
              >
                <X className="h-3.5 w-3.5" />
                Request changes
              </Button>
            ) : null}
            {canMerge ? (
              <MergePullRequestButton
                onOpenTerminal={onOpenTerminal}
                project={project}
                pullRequest={pullRequest}
              />
            ) : null}
            {canChangeStatus && isDraft ? (
              <Button
                className="h-7 gap-1.5 px-3"
                disabled={isUpdatingStatus}
                onClick={() => {
                  void handleStatusChange("open");
                }}
                size="xs"
                type="button"
                variant="secondary"
              >
                <GitPullRequest className="h-3.5 w-3.5" />
                Ready for review
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
      {showDraftControl ? (
        <p className="px-1 text-xs text-muted-foreground">
          Still in progress?{" "}
          <button
            className="font-medium underline-offset-2 hover:text-foreground hover:underline disabled:opacity-50"
            disabled={isUpdatingStatus}
            onClick={() => {
              void handleStatusChange("draft");
            }}
            type="button"
          >
            Convert to draft
          </button>
        </p>
      ) : null}
    </div>
  );
}
