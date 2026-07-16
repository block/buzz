import { GitMerge } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type { Project, ProjectPullRequest } from "@/features/projects/hooks";
import {
  useMergeProjectPullRequestMutation,
  usePublishProjectPullRequestMergedMutation,
} from "@/features/projects/pullRequestMutations";
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

export function MergePullRequestButton({
  project,
  pullRequest,
}: {
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);
  const [unpublishedMergeCommit, setUnpublishedMergeCommit] = React.useState<
    string | null
  >(null);
  const mergeMutation = useMergeProjectPullRequestMutation(project);
  const publishMergedMutation =
    usePublishProjectPullRequestMergedMutation(project);

  const handleMerge = React.useCallback(async () => {
    try {
      const result = await mergeMutation.mutateAsync({ pullRequest });
      if (result.statusPublicationError) {
        setUnpublishedMergeCommit(result.mergeCommit);
        toast.warning(result.message, {
          description: result.statusPublicationError,
        });
      } else {
        setUnpublishedMergeCommit(null);
        toast.success(result.message);
      }
      setConfirmOpen(false);
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to merge pull request.",
      );
    }
  }, [mergeMutation, pullRequest]);

  const handlePublishMergedStatus = React.useCallback(async () => {
    if (!unpublishedMergeCommit) return;
    try {
      await publishMergedMutation.mutateAsync({
        mergeCommit: unpublishedMergeCommit,
        pullRequest,
      });
      setUnpublishedMergeCommit(null);
      toast.success("Published merged pull request status.");
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to publish merged pull request status.",
      );
    }
  }, [publishMergedMutation, pullRequest, unpublishedMergeCommit]);

  return (
    <AlertDialog onOpenChange={setConfirmOpen} open={confirmOpen}>
      <Button
        className="h-8 gap-1.5 bg-purple-600 px-3.5 text-white shadow-sm hover:bg-purple-700"
        disabled={mergeMutation.isPending || publishMergedMutation.isPending}
        onClick={() => {
          if (unpublishedMergeCommit) {
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
          : unpublishedMergeCommit
            ? "Publish merged status"
            : "Merge"}
      </Button>
      <AlertDialogContent data-testid="merge-pull-request-confirm">
        <AlertDialogHeader>
          <AlertDialogTitle>Merge pull request?</AlertDialogTitle>
          <AlertDialogDescription>
            Merge {pullRequest.branchName} into {project.defaultBranch} and push
            the result to the repository. The remote will reject the operation
            if the branch changed or conflicts.
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
  );
}
