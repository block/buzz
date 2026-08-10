import { FolderOpen, NotebookText, TriangleAlert } from "lucide-react";

import { Button } from "@/shared/ui/button";

/**
 * Shown when no vault is selected, or when the selected one can't be opened.
 *
 * The same picker is reachable from Settings → Documents; this exists so a
 * first-time user never has to go looking for it.
 */
export function VaultEmptyState({
  errorMessage,
  onChooseVault,
}: {
  errorMessage?: string;
  onChooseVault: () => void;
}) {
  return (
    <div
      className="flex min-h-0 flex-1 items-center justify-center p-8"
      data-testid="documents-empty-state"
    >
      <div className="flex max-w-md flex-col items-center text-center">
        <div className="mb-4 rounded-2xl border border-border/60 bg-card/40 p-4">
          <NotebookText className="h-8 w-8 text-muted-foreground" />
        </div>

        <h2 className="text-base font-medium">Choose a vault folder</h2>
        <p className="mt-2 text-sm text-muted-foreground">
          Documents reads and writes plain markdown files in a folder on this
          computer. Point it at an existing Obsidian vault, or any folder of
          <span className="whitespace-nowrap"> .md </span>
          files.
        </p>

        {errorMessage ? (
          <p
            className="mt-4 flex items-start gap-2 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-left text-sm text-destructive"
            data-testid="documents-vault-error"
          >
            <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
            <span>{errorMessage}</span>
          </p>
        ) : null}

        <Button
          className="mt-5"
          data-testid="documents-choose-vault"
          onClick={onChooseVault}
          type="button"
        >
          <FolderOpen className="h-4 w-4" />
          Choose folder
        </Button>
      </div>
    </div>
  );
}
