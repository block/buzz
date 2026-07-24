import {
  Check,
  GitPullRequest,
  GitPullRequestDraft,
  MoreHorizontal,
  RotateCcw,
  X,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import type { Project, ProjectPullRequest } from "@/features/projects/hooks";
import { nextProjectPullRequestReviewCreatedAt } from "@/features/projects/projectPullRequests.mjs";
import {
  canReviewProjectPullRequest,
  useApproveProjectPullRequestMutation,
  useUpdateProjectPullRequestStatusMutation,
} from "@/features/projects/pullRequestReviews";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Textarea } from "@/shared/ui/textarea";
import {
  MergePullRequestButton,
  type OpenMergeRecoveryTerminal,
} from "./MergePullRequestButton";

/** Available pull-request actions rendered beside the latest review activity. */
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
  const [approveDialogOpen, setApproveDialogOpen] = React.useState(false);
  const [approvalSummary, setApprovalSummary] = React.useState("");
  const reviewDecisionInFlightRef = React.useRef(false);
  const lastReviewDecisionCreatedAtRef = React.useRef(0);

  const viewerPubkey = identityQuery.data?.pubkey ?? null;
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const isAuthor = viewer === normalizePubkey(pullRequest.author);
  const isOwner = viewer === normalizePubkey(project.owner);
  const isManagedAgentOwner = useIsManagedAgent(project.owner) === true;
  const canChangeStatus =
    Boolean(viewer) && (isAuthor || isOwner || isManagedAgentOwner);
  const hasApproved = Boolean(
    viewer &&
      pullRequest.approvals.some(
        (approval) => normalizePubkey(approval.author) === viewer,
      ),
  );
  const canReview = canReviewProjectPullRequest(project, pullRequest, viewer);
  const canApprove = canReview && !hasApproved;
  const canMerge =
    (isOwner || isManagedAgentOwner) &&
    pullRequest.status === "Open" &&
    Boolean(pullRequest.branchName && pullRequest.commit);

  const handleStatusChange = React.useCallback(
    async (status: "open" | "draft" | "closed") => {
      try {
        await updatePullRequestStatus({
          pullRequest,
          signAsManagedOwner: isManagedAgentOwner && !isOwner,
          status,
        });
        toast.success(
          status === "draft"
            ? "Converted to draft."
            : status === "closed"
              ? "Pull request closed."
              : pullRequest.status === "Closed"
                ? "Pull request reopened."
                : "Marked as ready for review.",
        );
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to update status.",
        );
      }
    },
    [isManagedAgentOwner, isOwner, pullRequest, updatePullRequestStatus],
  );

  const runReviewDecision = React.useCallback(
    async (
      mutate: (input: {
        content?: string;
        createdAt: number;
        pullRequest: ProjectPullRequest;
      }) => Promise<unknown>,
      successMessage: string,
      fallbackErrorMessage: string,
      content?: string,
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
        await mutate({ content, createdAt, pullRequest });
        toast.success(successMessage);
        return true;
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : fallbackErrorMessage,
        );
        return false;
      } finally {
        reviewDecisionInFlightRef.current = false;
      }
    },
    [pullRequest],
  );

  const handleApprove = React.useCallback(async () => {
    const approved = await runReviewDecision(
      approvePullRequest,
      "Pull request approved.",
      "Failed to approve.",
      approvalSummary,
    );
    if (approved) {
      setApproveDialogOpen(false);
      setApprovalSummary("");
    }
  }, [approvalSummary, approvePullRequest, runReviewDecision]);

  const reviewDecisionPending = isApproving;
  const canMarkReady = canChangeStatus && pullRequest.status === "Draft";
  const canConvertToDraft = canChangeStatus && pullRequest.status === "Open";
  const canClose =
    canChangeStatus &&
    (pullRequest.status === "Open" || pullRequest.status === "Draft");
  const canReopen = canChangeStatus && pullRequest.status === "Closed";
  const hasOverflowAction = canConvertToDraft || canClose;
  const hasAvailableAction =
    canApprove ||
    canMerge ||
    canMarkReady ||
    canConvertToDraft ||
    canClose ||
    canReopen;

  if (!hasAvailableAction) return null;

  return (
    <>
      <div className="pull-request-action-timeline flex min-w-0 flex-1 items-start gap-2">
        <span
          aria-hidden="true"
          className="mt-3.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-background text-muted-foreground ring-1 ring-border/70"
        >
          <GitPullRequest className="h-3 w-3" />
        </span>
        <div
          className="flex min-w-0 flex-1 flex-wrap items-center justify-start gap-2 rounded-lg border border-border/60 bg-muted/15 p-2"
          data-testid="project-pull-request-actions"
        >
          {canApprove ? (
            <Button
              className="h-8 gap-1.5 bg-green-600 px-3.5 text-white shadow-sm hover:bg-green-700"
              disabled={reviewDecisionPending}
              onClick={() => setApproveDialogOpen(true)}
              size="xs"
              type="button"
            >
              <Check className="h-3.5 w-3.5" />
              Approve
            </Button>
          ) : null}
          {canMerge ? (
            <MergePullRequestButton
              onOpenTerminal={onOpenTerminal}
              project={project}
              pullRequest={pullRequest}
            />
          ) : null}
          {canMarkReady ? (
            <Button
              className="h-8 gap-1.5 px-3"
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
          {canReopen ? (
            <Button
              className="h-8 gap-1.5 px-3"
              disabled={isUpdatingStatus}
              onClick={() => {
                void handleStatusChange("open");
              }}
              size="xs"
              type="button"
              variant="secondary"
            >
              <RotateCcw className="h-3.5 w-3.5" />
              Reopen pull request
            </Button>
          ) : null}
          {hasOverflowAction ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  aria-label="More pull request actions"
                  className="ml-auto h-8 w-8"
                  disabled={isUpdatingStatus}
                  size="icon-xs"
                  type="button"
                  variant="ghost"
                >
                  <MoreHorizontal className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {canConvertToDraft ? (
                  <DropdownMenuItem
                    onSelect={() => {
                      void handleStatusChange("draft");
                    }}
                  >
                    <GitPullRequestDraft className="h-4 w-4" />
                    Convert to draft
                  </DropdownMenuItem>
                ) : null}
                {canClose ? (
                  <DropdownMenuItem
                    className="text-destructive focus:text-destructive"
                    onSelect={() => {
                      void handleStatusChange("closed");
                    }}
                  >
                    <X className="h-4 w-4" />
                    Close pull request
                  </DropdownMenuItem>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
        </div>
      </div>
      <Dialog
        onOpenChange={(open) => {
          if (!open && isApproving) return;
          setApproveDialogOpen(open);
          if (!open) setApprovalSummary("");
        }}
        open={approveDialogOpen}
      >
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Approve pull request</DialogTitle>
            <DialogDescription>
              Add an optional summary for the author and other reviewers.
            </DialogDescription>
          </DialogHeader>
          <Textarea
            aria-label="Approval summary"
            disabled={isApproving}
            onChange={(event) => setApprovalSummary(event.target.value)}
            placeholder="What looks good?"
            value={approvalSummary}
          />
          <DialogFooter>
            <Button
              disabled={isApproving}
              onClick={() => setApproveDialogOpen(false)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              className="bg-green-600 text-white hover:bg-green-700"
              disabled={isApproving}
              onClick={() => {
                void handleApprove();
              }}
              type="button"
            >
              <Check className="h-4 w-4" />
              Approve
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
