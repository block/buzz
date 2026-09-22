import { Loader2 } from "lucide-react";
import * as React from "react";

import {
  normalizeProjectBranchName,
  projectBranchNameError,
} from "@/features/projects/lib/projectBranches";
import { projectBranchErrorMessage } from "@/features/projects/lib/projectBranchErrors";
import { useTranslation } from "@/i18n";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";

export function CreateProjectBranchDialog({
  existingBranches,
  onCreate,
  onOpenChange,
  open,
  pending,
  sourceBranch,
  sourceCommit,
}: {
  existingBranches: string[];
  onCreate: (branch: string) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  pending: boolean;
  sourceBranch: string;
  sourceCommit: string | null;
}) {
  const { t } = useTranslation();
  const [branchName, setBranchName] = React.useState("");
  const [submitError, setSubmitError] = React.useState<string | null>(null);
  const validationError = projectBranchNameError(branchName, existingBranches);

  React.useEffect(() => {
    if (!open) return;
    setBranchName("");
    setSubmitError(null);
  }, [open]);

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const branch = normalizeProjectBranchName(branchName);
    if (!branch || validationError || !sourceCommit) return;
    setSubmitError(null);
    try {
      await onCreate(branch);
      onOpenChange(false);
    } catch (error) {
      setSubmitError(
        projectBranchErrorMessage(error, "Failed to create branch."),
      );
    }
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!pending) onOpenChange(nextOpen);
      }}
      open={open}
    >
      <DialogContent data-testid="project-create-branch-dialog">
        <form className="space-y-5" onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>
              {t("projects.branch-dialogs.create-branch")}
            </DialogTitle>
            <DialogDescription>
              {sourceCommit
                ? t("projects.branch-dialogs.create-description-commit", {
                    branch: sourceBranch,
                    commit: sourceCommit.slice(0, 7),
                  })
                : t("projects.branch-dialogs.create-description", {
                    branch: sourceBranch,
                  })}
            </DialogDescription>
          </DialogHeader>
          <label
            className="block space-y-2 text-sm font-medium"
            htmlFor="project-create-branch-name"
          >
            <span>{t("projects.branch-dialogs.branch-name")}</span>
            <Input
              autoFocus
              data-testid="project-create-branch-name"
              disabled={pending}
              id="project-create-branch-name"
              onChange={(event) => setBranchName(event.target.value)}
              placeholder="feature/my-change"
              value={branchName}
            />
          </label>
          {branchName && validationError ? (
            <p className="text-sm text-destructive">{validationError}</p>
          ) : null}
          {!sourceCommit ? (
            <p className="text-sm text-destructive">
              {t("projects.branch-dialogs.refresh-hint")}
            </p>
          ) : null}
          {submitError ? (
            <p className="text-sm text-destructive">{submitError}</p>
          ) : null}
          <DialogFooter>
            <Button
              disabled={pending}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="outline"
            >
              {t("projects.card.cancel")}
            </Button>
            <Button
              data-testid="project-create-branch-submit"
              disabled={pending || Boolean(validationError) || !sourceCommit}
              type="submit"
            >
              {pending ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {pending
                ? t("projects.shared.creating")
                : t("projects.branch-dialogs.create-branch")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export function DeleteProjectBranchDialog({
  branch,
  onDelete,
  onOpenChange,
  open,
  pending,
}: {
  branch: string;
  onDelete: () => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  pending: boolean;
}) {
  const { t } = useTranslation();
  const [submitError, setSubmitError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (open) setSubmitError(null);
  }, [open]);

  async function handleDelete() {
    setSubmitError(null);
    try {
      await onDelete();
      onOpenChange(false);
    } catch (error) {
      setSubmitError(
        projectBranchErrorMessage(error, "Failed to delete branch."),
      );
    }
  }

  return (
    <AlertDialog
      onOpenChange={(nextOpen) => {
        if (!pending) onOpenChange(nextOpen);
      }}
      open={open}
    >
      <AlertDialogContent data-testid="project-delete-branch-dialog">
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("projects.branch-dialogs.delete-question")}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t("projects.branch-dialogs.delete-description", { branch })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {submitError ? (
          <p className="text-sm text-destructive">{submitError}</p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={pending} type="button" variant="outline">
              {t("projects.card.cancel")}
            </Button>
          </AlertDialogCancel>
          <Button
            data-testid="project-delete-branch-submit"
            disabled={pending}
            onClick={() => void handleDelete()}
            type="button"
            variant="destructive"
          >
            {pending ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
            {pending
              ? t("projects.shared.deleting")
              : t("projects.branch-dialogs.delete-branch")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
