import { AlertTriangle, Copy, GitMerge, SquareTerminal } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type {
  ProjectPullRequest,
  Repository as Project,
} from "@/features/projects/hooks";
import { projectPullRequestConflictCommands } from "@/features/projects/projectPullRequestConflictRecovery";
import {
  useMergeProjectPullRequestMutation,
  usePublishProjectPullRequestMergedMutation,
} from "@/features/projects/pullRequestMutations";
import { useTranslation } from "@/i18n";
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
  const { t } = useTranslation();
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
          : t("projects.merge-review-button.merge-failed"),
      );
    }
  }, [mergeMutation, pullRequest, t]);

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
      toast.success(t("projects.merge-review-button.recovery-opened"));
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t("projects.merge-review-button.prepare-failed"),
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
    t,
  ]);

  const handlePublishMergedStatus = React.useCallback(async () => {
    if (!unpublishedStatusEvent) return;
    try {
      await publishMergedMutation.mutateAsync({
        statusEvent: unpublishedStatusEvent,
      });
      setUnpublishedStatusState(null);
      toast.success(t("projects.merge-review-button.published"));
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : t("projects.merge-review-button.publish-failed"),
      );
    }
  }, [publishMergedMutation, t, unpublishedStatusEvent]);

  return (
    <div className="contents">
      <AlertDialog onOpenChange={setConfirmOpen} open={confirmOpen}>
        <Button
          className="h-8 gap-1.5 px-3.5"
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
            ? t("projects.merge-review-button.publishing")
            : unpublishedStatusEvent
              ? t("projects.merge-review-button.publish-merged-status")
              : t("projects.merge-review-button.merge")}
        </Button>
        <AlertDialogContent data-testid="merge-pull-request-confirm">
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("projects.merge-review-button.confirm-title")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("projects.merge-review-button.confirm-description", {
                source: pullRequest.branchName,
                target: targetBranch,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={mergeMutation.isPending}>
              {t("projects.card.cancel")}
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
                {mergeMutation.isPending
                  ? t("projects.merge-review-button.merging")
                  : t("projects.merge-review-button.merge-review")}
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
                {t("projects.merge-review-button.resolve-title")}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("projects.merge-review-button.resolve-steps", {
                  branch: conflictRecovery.targetBranch,
                })}
              </p>
            </div>
          </div>
          {preparedRecovery ? (
            <pre className="overflow-x-auto rounded-md bg-background/80 p-2 font-mono text-xs text-foreground">
              {recoveryCommands.join("\n")}
            </pre>
          ) : (
            <p className="rounded-md bg-background/80 p-2 text-xs text-muted-foreground">
              {t("projects.merge-review-button.resolve-hint")}
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
              {isPreparingRecovery
                ? t("projects.merge-review-button.preparing")
                : t("projects.merge-review-button.resolve-in-terminal")}
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
              {t("projects.merge-review-button.copy-commands")}
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
