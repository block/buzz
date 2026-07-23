import { AlertTriangle, Copy, GitMerge, SquareTerminal } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type { Project, ProjectPullRequest } from "@/features/projects/hooks";
import { projectPullRequestConflictCommands } from "@/features/projects/projectPullRequestConflictRecovery";
import {
  useMergeProjectPullRequestMutation,
  usePublishProjectPullRequestMergedMutation,
} from "@/features/projects/pullRequestMutations";
import {
  ProjectPullRequestMergeError,
  type ProjectPullRequestMergeRecovery,
} from "@/shared/api/projectGit";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";

export type OpenMergeRecoveryTerminal = (input: {
  expectedCommit: string;
  sourceBranch: string;
  sourceCloneUrl: string;
  targetBranch: string;
}) => Promise<{ recoveryRef: string; targetRef: string }>;

export function MergePullRequestButton({
  onOpenTerminal,
  project,
  pullRequest,
}: {
  onOpenTerminal?: OpenMergeRecoveryTerminal;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const [isPreparingRecovery, setIsPreparingRecovery] = React.useState(false);
  const [conflictRecoveryState, setConflictRecoveryState] = React.useState<{
    pullRequestId: string;
    recovery: ProjectPullRequestMergeRecovery;
  } | null>(null);
  const [unpublishedStatusState, setUnpublishedStatusState] = React.useState<{
    event: string;
    pullRequestId: string;
  } | null>(null);
  const [preparedRecoveryState, setPreparedRecoveryState] = React.useState<{
    pullRequestId: string;
    recoveryRef: string;
    targetRef: string;
  } | null>(null);
  const mergeMutation = useMergeProjectPullRequestMutation(project);
  const publishMergedMutation =
    usePublishProjectPullRequestMergedMutation(project);
  const targetBranch = pullRequest.targetBranch ?? project.defaultBranch;
  const conflictRecovery =
    conflictRecoveryState?.pullRequestId === pullRequest.id
      ? conflictRecoveryState.recovery
      : null;
  const unpublishedStatusEvent =
    unpublishedStatusState?.pullRequestId === pullRequest.id
      ? unpublishedStatusState.event
      : null;
  const preparedRecovery =
    preparedRecoveryState?.pullRequestId === pullRequest.id
      ? preparedRecoveryState
      : null;

  const handleMerge = React.useCallback(async () => {
    try {
      const result = await mergeMutation.mutateAsync({ pullRequest });
      if (result.statusPublicationError) {
        setUnpublishedStatusState({
          event: result.statusEvent,
          pullRequestId: pullRequest.id,
        });
        toast.warning(result.message, {
          description: result.statusPublicationError,
        });
      } else {
        setUnpublishedStatusState(null);
        toast.success(result.message);
      }
      setConflictRecoveryState(null);
      setPreparedRecoveryState(null);
      setConfirmOpen(false);
    } catch (error) {
      if (
        error instanceof ProjectPullRequestMergeError &&
        error.code === "merge_conflict" &&
        error.recovery
      ) {
        setConflictRecoveryState({
          pullRequestId: pullRequest.id,
          recovery: error.recovery,
        });
        setPreparedRecoveryState(null);
        setConfirmOpen(false);
      }
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to merge pull request.",
      );
    }
  }, [mergeMutation, pullRequest]);

  const recoveryCommands =
    conflictRecovery && preparedRecovery
      ? projectPullRequestConflictCommands({
          recoveryRef: preparedRecovery.recoveryRef,
          targetBranch: conflictRecovery.targetBranch,
          targetRef: preparedRecovery.targetRef,
        })
      : [];

  const handleOpenRecoveryTerminal = React.useCallback(async () => {
    const sourceCloneUrl = pullRequest.cloneUrls[0] ?? project.cloneUrls[0];
    if (!conflictRecovery || !pullRequest.commit || !sourceCloneUrl) return;
    setIsPreparingRecovery(true);
    try {
      const result = await onOpenTerminal?.({
        expectedCommit: pullRequest.commit,
        sourceBranch: conflictRecovery.sourceBranch,
        sourceCloneUrl,
        targetBranch: conflictRecovery.targetBranch,
      });
      if (!result) return;
      setPreparedRecoveryState({
        pullRequestId: pullRequest.id,
        recoveryRef: result.recoveryRef,
        targetRef: result.targetRef,
      });
      toast.success("Recovery commit fetched and terminal opened.");
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to prepare merge recovery.",
      );
    } finally {
      setIsPreparingRecovery(false);
    }
  }, [
    conflictRecovery,
    onOpenTerminal,
    project.cloneUrls,
    pullRequest.cloneUrls,
    pullRequest.commit,
    pullRequest.id,
  ]);

  const handlePublishMergedStatus = React.useCallback(async () => {
    if (!unpublishedStatusEvent) return;
    try {
      await publishMergedMutation.mutateAsync({
        statusEvent: unpublishedStatusEvent,
      });
      setUnpublishedStatusState(null);
      toast.success("Published merged pull request status.");
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to publish merged pull request status.",
      );
    }
  }, [publishMergedMutation, unpublishedStatusEvent]);

  return (
    <div className="contents">
      <AlertDialog onOpenChange={setConfirmOpen} open={confirmOpen}>
        <Button
          className="h-8 gap-1.5 bg-purple-600 px-3.5 text-white shadow-sm hover:bg-purple-700"
          disabled={mergeMutation.isPending || publishMergedMutation.isPending}
          onClick={() => {
            if (unpublishedStatusEvent) {
              void handlePublishMergedStatus();
            } else {
              setConfirmOpen(true);
            }
          }}
          size="xs"
          type="button"
        >
          <GitMerge className="h-3.5 w-3.5" />
          {publishMergedMutation.isPending
            ? "Publishing…"
            : unpublishedStatusEvent
              ? "Publish merged status"
              : "Merge"}
        </Button>
        <AlertDialogContent data-testid="merge-pull-request-confirm">
          <AlertDialogHeader>
            <AlertDialogTitle>Merge pull request?</AlertDialogTitle>
            <AlertDialogDescription>
              Merge {pullRequest.branchName} into {targetBranch} and push the
              result to the repository. The remote will reject the operation if
              the branch changed or conflicts.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={mergeMutation.isPending}>
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                data-testid="merge-pull-request-confirm-button"
                disabled={mergeMutation.isPending}
                onClick={(event) => {
                  event.preventDefault();
                  void handleMerge();
                }}
                type="button"
              >
                {mergeMutation.isPending ? "Merging…" : "Merge pull request"}
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      {conflictRecovery ? (
        <div
          className="w-full basis-full space-y-2 rounded-lg border border-amber-500/35 bg-amber-500/10 p-3"
          data-testid="merge-conflict-recovery"
        >
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-foreground">
                Resolve conflicts in your local checkout
              </p>
              <p className="text-xs text-muted-foreground">
                Prepare the local checkout, then switch to{" "}
                {conflictRecovery.targetBranch} with the commands shown. After
                resolving and committing, push the target branch and retry the
                merge.
              </p>
            </div>
          </div>
          {preparedRecovery ? (
            <pre className="overflow-x-auto rounded-md bg-background/80 p-2 font-mono text-xs text-foreground">
              {recoveryCommands.join("\n")}
            </pre>
          ) : (
            <p className="rounded-md bg-background/80 p-2 text-xs text-muted-foreground">
              Resolve in Terminal securely fetches the target and pull request
              commits before showing copyable commands.
            </p>
          )}
          <div className="flex flex-wrap gap-2">
            <Button
              disabled={!onOpenTerminal || isPreparingRecovery}
              onClick={() => void handleOpenRecoveryTerminal()}
              size="xs"
              type="button"
              variant="outline"
            >
              <SquareTerminal className="h-3.5 w-3.5" />
              {isPreparingRecovery ? "Preparing…" : "Resolve in Terminal"}
            </Button>
            <Button
              disabled={!preparedRecovery}
              onClick={() =>
                copyTextToClipboard(
                  recoveryCommands.join("\n"),
                  "Recovery commands copied",
                )
              }
              size="xs"
              type="button"
              variant="ghost"
            >
              <Copy className="h-3.5 w-3.5" />
              Copy commands
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
