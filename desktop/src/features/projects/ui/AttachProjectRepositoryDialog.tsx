import { FolderGit2 } from "lucide-react";
import * as React from "react";

import type { Project, Repository } from "@/features/projects/hooks";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

export function AttachProjectRepositoryDialog({
  isAttaching,
  onAttach,
  onOpenChange,
  open,
  project,
  repositories,
}: {
  isAttaching: boolean;
  onAttach: (repository: Repository) => Promise<void>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  project: Project;
  repositories: Repository[];
}) {
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (open) setErrorMessage(null);
  }, [open]);

  async function handleAttach(repository: Repository) {
    setErrorMessage(null);
    try {
      await onAttach(repository);
      onOpenChange(false);
    } catch (error) {
      setErrorMessage(
        error instanceof Error ? error.message : "Failed to attach repository.",
      );
    }
  }

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isAttaching) return;
        onOpenChange(nextOpen);
      }}
      open={open}
    >
      <ChooserDialogContent
        className="max-w-lg"
        contentClassName="max-h-96 overflow-y-auto pt-3"
        data-testid="attach-project-repository-dialog"
        description={`Choose an existing repository to add to ${project.name}.`}
        title="Add existing repository"
      >
        <div className="space-y-2">
          {repositories.length === 0 ? (
            <p className="py-6 text-center text-sm text-muted-foreground">
              Every available repository is already in this project.
            </p>
          ) : (
            repositories.map((repository) => (
              <Button
                className="h-auto w-full justify-start gap-3 px-3 py-2.5 text-left"
                data-testid={`attach-existing-repository-${repository.dtag}`}
                disabled={isAttaching}
                key={repository.repoAddress}
                onClick={() => void handleAttach(repository)}
                type="button"
                variant="outline"
              >
                <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0">
                  <span className="block truncate font-medium">
                    {repository.name}
                  </span>
                  <span className="block truncate text-xs font-normal text-muted-foreground">
                    {repository.dtag}
                  </span>
                </span>
              </Button>
            ))
          )}
          {errorMessage ? (
            <p className="text-sm text-destructive">{errorMessage}</p>
          ) : null}
        </div>
      </ChooserDialogContent>
    </Dialog>
  );
}
