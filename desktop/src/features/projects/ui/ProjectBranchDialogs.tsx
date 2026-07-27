import { Loader2 } from "lucide-react";
import * as React from "react";

import {
  normalizeProjectBranchName,
  projectBranchNameError,
} from "@/features/projects/lib/projectBranches";
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

function errorMessage(error: unknown, fallback: string) {
  if (!(error instanceof Error)) return fallback;
  if (error.message.includes("no channel binding")) {
    return "This repository is owned by another identity and is not linked to a project channel.";
  }
  return error.message;
}

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
      setSubmitError(errorMessage(error, "Failed to create branch."));
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
            <DialogTitle>Create branch</DialogTitle>
            <DialogDescription>
              Create a remote branch from{" "}
              <span className="font-mono text-foreground">{sourceBranch}</span>
              {sourceCommit ? ` at ${sourceCommit.slice(0, 7)}` : ""}.
            </DialogDescription>
          </DialogHeader>
          <label
            className="block space-y-2 text-sm font-medium"
            htmlFor="project-create-branch-name"
          >
            <span>Branch name</span>
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
              Refresh the repository before creating a branch.
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
              Cancel
            </Button>
            <Button
              data-testid="project-create-branch-submit"
              disabled={pending || Boolean(validationError) || !sourceCommit}
              type="submit"
            >
              {pending ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {pending ? "Creating…" : "Create branch"}
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
      setSubmitError(errorMessage(error, "Failed to delete branch."));
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
          <AlertDialogTitle>Delete branch?</AlertDialogTitle>
          <AlertDialogDescription>
            Delete the remote branch{" "}
            <span className="font-mono text-foreground">{branch}</span>. This
            cannot be undone and may be rejected by repository protection rules.
          </AlertDialogDescription>
        </AlertDialogHeader>
        {submitError ? (
          <p className="text-sm text-destructive">{submitError}</p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={pending} type="button" variant="outline">
              Cancel
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
            {pending ? "Deleting…" : "Delete branch"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
